// Właściwa instalacja: btrfs w układzie CachyOS (stały subvol=@), bootstrap
// (live/chroot), konfiguracja systemu docelowego, snapper, GRUB i rollback.

use super::cmd::{
    log, run, run_capture, run_in, run_in_capture_stdin, run_in_redacted, run_in_soft, run_soft,
};
use super::partition::{fs_uuid, TargetParts};
use crate::config::{Desktop, Edition, InstallConfig, Mode, Source};
use std::fs;
use std::path::{Path, PathBuf};

/// Katalog roboczy trybu "z innego systemu" (wymaganie: /tmp/chimera-linux).
pub const STAGE_DIR: &str = "/tmp/chimera-linux";
/// Pobrane obrazy trzymamy obok, żeby ponowna próba nie pobierała od nowa.
const CACHE_DIR: &str = "/tmp/chimera-linux-cache";
const REPO_URL: &str = "https://repo.chimera-linux.org/live/latest/";

/// Punkt montowania systemu docelowego. W trybie live zgodnie z dokumentacją
/// Chimery (/media/root); w trybie obcym wewnątrz rozpakowanego stage'a,
/// żeby chimera-bootstrap widział go jako /media/root po chroocie.
pub fn target_mount(cfg: &InstallConfig) -> PathBuf {
    match cfg.mode {
        Mode::Live => PathBuf::from("/media/root"),
        Mode::Foreign => Path::new(STAGE_DIR).join("media/root"),
    }
}

/// Płaskie, nazwane subwolumeny top-level w stylu CachyOS. / to STAŁY
/// subvol=@ — rollback podmienia ZAWARTOŚĆ @ (mv @ → kopia, snapshot → @),
/// więc działa na waniliowym GRUB (bez btrfs set-default, bez łatek).
/// /boot zostaje WEWNĄTRZ @, żeby rollback obejmował kernel/initrd.
/// Para: (nazwa subwolumenu top-level, punkt montowania względem /).
const SUBVOLS: &[(&str, &str)] = &[
    ("@home", "home"),
    ("@root", "root"),
    ("@srv", "srv"),
    ("@cache", "var/cache"),
    ("@tmp", "var/tmp"),
    ("@log", "var/log"),
];

/// Subwolumen migawek — MUSI być osobny (poza @), żeby przetrwał podmianę
/// @ przy rollbacku; inaczej migawki zniknęłyby razem ze starym @.
const SNAPSHOTS_SUBVOL: &str = "@snapshots";

/// Migawka fabryczna FABRIC (nr 1) — subwolumen PIERWOTNY: system instaluje
/// się wprost do niej, a @ powstaje na końcu instalacji jako jej migawka
/// (`btrfs subvolume snapshot FABRIC → @`).
pub const FABRIC_SUBVOL: &str = "@snapshots/1/snapshot";

const TOP_MOUNT: &str = "/run/chimera-installer/top";

// ---------------------------------------------------------------------------
// Krok: systemy plików i subwolumeny
// ---------------------------------------------------------------------------

pub fn make_filesystems(parts: &TargetParts) -> Result<(), String> {
    if parts.esp_format {
        if let Some(esp) = &parts.esp {
            run(&["mkfs.vfat", "-F", "32", esp])?;
        }
    }
    run(&["mkfs.btrfs", "-f", &parts.root])?;
    Ok(())
}

pub fn make_subvolumes(parts: &TargetParts) -> Result<(), String> {
    fs::create_dir_all(TOP_MOUNT).map_err(|e| format!("mkdir {TOP_MOUNT}: {e}"))?;
    run(&["mount", "-o", "subvolid=5", &parts.root, TOP_MOUNT])?;
    let res = (|| -> Result<(), String> {
        let top = Path::new(TOP_MOUNT);
        let at = |s: &str| top.join(s).to_string_lossy().to_string();

        // @snapshots — osobny magazyn migawek. Celem instalacji jest migawka
        // fabryczna FABRIC (@snapshots/1/snapshot); stały subwolumen roota @
        // powstanie na końcu instalacji jako jej migawka (fabric_to_root).
        run(&["btrfs", "subvolume", "create", &at(SNAPSHOTS_SUBVOL)])?;
        fs::create_dir_all(top.join(SNAPSHOTS_SUBVOL).join("1"))
            .map_err(|e| format!("mkdir {SNAPSHOTS_SUBVOL}/1: {e}"))?;
        run(&["btrfs", "subvolume", "create", &at(FABRIC_SUBVOL)])?;

        for (sv, _) in SUBVOLS {
            run(&["btrfs", "subvolume", "create", &at(sv)])?;
        }
        // Dużo drobnych zapisów — bez kopiowania przy zapisie (jak CachyOS).
        run_soft(&["chattr", "+C", &at("@log")]);
        run_soft(&["chattr", "+C", &at("@cache")]);
        Ok(())
    })();
    run_soft(&["umount", TOP_MOUNT]);
    res
}

/// Montuje układ docelowy z `root_subvol` jako /. Podczas instalacji jest to
/// migawka FABRIC, po jej sklonowaniu do @ (fabric_to_root) — stały subvol=@,
/// dokładnie tak, jak zamontuje go GRUB i fstab.
pub fn mount_target(
    cfg: &InstallConfig,
    parts: &TargetParts,
    root_subvol: &str,
) -> Result<PathBuf, String> {
    let target = target_mount(cfg);
    let t = target.to_string_lossy().to_string();
    fs::create_dir_all(&target).map_err(|e| format!("mkdir {t}: {e}"))?;
    run(&["mount", "-o", &format!("subvol={root_subvol}"), &parts.root, &t])?;
    run(&["chmod", "755", &t])?;

    // Osobny magazyn migawek najpierw, potem pozostałe subwolumeny.
    let mut mounts: Vec<(String, String)> =
        vec![(SNAPSHOTS_SUBVOL.to_string(), ".snapshots".to_string())];
    for (sv, mp) in SUBVOLS {
        mounts.push((sv.to_string(), mp.to_string()));
    }
    for (subvol, rel) in mounts {
        let mp = target.join(&rel);
        fs::create_dir_all(&mp).map_err(|e| format!("mkdir {}: {e}", mp.display()))?;
        run(&[
            "mount",
            "-o",
            &format!("subvol={subvol}"),
            &parts.root,
            &mp.to_string_lossy(),
        ])?;
    }
    if let Some(esp) = &parts.esp {
        let mp = target.join("boot/efi");
        fs::create_dir_all(&mp).map_err(|e| format!("mkdir {}: {e}", mp.display()))?;
        run(&["mount", esp, &mp.to_string_lossy()])?;
    }
    Ok(target)
}

// ---------------------------------------------------------------------------
// Tryb "z innego systemu": pobranie obrazu i przygotowanie chroota
// ---------------------------------------------------------------------------

fn downloader() -> Result<&'static str, String> {
    for c in ["curl", "wget"] {
        if std::process::Command::new(c)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Ok(c);
        }
    }
    Err("brak curl ani wget — nie mogę pobrać obrazu".into())
}

fn fetch_text(tool: &str, url: &str) -> Result<String, String> {
    match tool {
        "curl" => run_capture(&["curl", "-fsSL", url]),
        _ => run_capture(&["wget", "-qO-", url]),
    }
}

fn fetch_file(tool: &str, url: &str, dest: &str) -> Result<(), String> {
    match tool {
        "curl" => run(&["curl", "-fL", "--progress-bar", "-o", dest, url]),
        _ => run(&["wget", "-O", dest, url]),
    }
}

/// Wyszukuje w listingu katalogu najnowszy plik
/// chimera-linux-<arch>-ROOTFS-<data>-bootstrap.tar.gz (bez crate'a regex).
fn find_tarball_name(listing: &str, arch: &str) -> Option<String> {
    let prefix = format!("chimera-linux-{arch}-ROOTFS-");
    let suffix = "-bootstrap.tar.gz";
    let mut best: Option<String> = None;
    let mut rest = listing;
    while let Some(pos) = rest.find(&prefix) {
        let tail = &rest[pos..];
        let date: String = tail[prefix.len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let candidate = format!("{prefix}{date}{suffix}");
        if !date.is_empty() && tail[prefix.len() + date.len()..].starts_with(suffix) {
            // Nazwy zawierają datę RRRRMMDD, więc porównanie leksykalne wystarcza.
            if best.as_ref().is_none_or(|b| candidate > *b) {
                best = Some(candidate);
            }
        }
        rest = &rest[pos + prefix.len()..];
    }
    best
}

fn sha256_of(path: &str) -> Result<String, String> {
    let out = run_capture(&["sha256sum", path])?;
    Ok(out.split_whitespace().next().unwrap_or_default().to_string())
}

/// Pobiera (z weryfikacją sha256) obraz bootstrap do cache'a.
pub fn foreign_download() -> Result<String, String> {
    let tool = downloader()?;
    let arch = run_capture(&["uname", "-m"])?.trim().to_string();
    log(&format!("szukam obrazu bootstrap dla architektury {arch}"));
    let listing = fetch_text(tool, REPO_URL)?;
    let name = find_tarball_name(&listing, &arch)
        .ok_or_else(|| format!("w {REPO_URL} nie znaleziono obrazu ROOTFS bootstrap dla {arch}"))?;
    log(&format!("najnowszy obraz: {name}"));

    let sums = fetch_text(tool, &format!("{REPO_URL}sha256sums.txt"))?;
    let expected = sums
        .lines()
        .find(|l| l.contains(&name))
        .and_then(|l| l.split_whitespace().next())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("brak sumy kontrolnej dla {name} w sha256sums.txt"))?;

    fs::create_dir_all(CACHE_DIR).map_err(|e| format!("mkdir {CACHE_DIR}: {e}"))?;
    let dest = format!("{CACHE_DIR}/{name}");
    if Path::new(&dest).exists() && sha256_of(&dest)? == expected {
        log("obraz już pobrany, suma kontrolna zgodna — pomijam pobieranie");
        return Ok(dest);
    }
    fetch_file(tool, &format!("{REPO_URL}{name}"), &dest)?;
    let got = sha256_of(&dest)?;
    if got != expected {
        let _ = fs::remove_file(&dest);
        return Err(format!(
            "suma SHA256 obrazu nie zgadza się (oczekiwano {expected}, jest {got}) — przerwano"
        ));
    }
    log("suma SHA256 zgodna");
    Ok(dest)
}

/// Odmontowuje wszystko pod `dir` (najgłębsze najpierw) na podstawie /proc/mounts.
fn umount_under(dir: &str) {
    let Ok(mounts) = fs::read_to_string("/proc/self/mounts") else {
        return;
    };
    let mut points: Vec<String> = mounts
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter(|mp| *mp == dir || mp.starts_with(&format!("{dir}/")))
        .map(|s| s.replace("\\040", " "))
        .collect();
    points.sort_by_key(|b| std::cmp::Reverse(b.len()));
    for mp in points {
        run_soft(&["umount", "-R", &mp]);
    }
}

/// Rozpakowuje obraz do /tmp/chimera-linux (po wyczyszczeniu pozostałości).
pub fn foreign_extract(tarball: &str) -> Result<(), String> {
    umount_under(STAGE_DIR);
    if Path::new(STAGE_DIR).exists() {
        // Bezpiecznik: nie usuwamy katalogu, pod którym coś nadal jest zamontowane.
        let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        if mounts
            .lines()
            .filter_map(|l| l.split_whitespace().nth(1))
            .any(|mp| mp == STAGE_DIR || mp.starts_with(&format!("{STAGE_DIR}/")))
        {
            return Err(format!(
                "pod {STAGE_DIR} wciąż są aktywne montowania — odmontuj je i spróbuj ponownie"
            ));
        }
        fs::remove_dir_all(STAGE_DIR).map_err(|e| format!("czyszczenie {STAGE_DIR}: {e}"))?;
    }
    fs::create_dir_all(STAGE_DIR).map_err(|e| format!("mkdir {STAGE_DIR}: {e}"))?;
    // GNU tar: zachowujemy też xattrs (np. file capabilities); gdy tar ich
    // nie zna (bsdtar), ponawiamy bez tych opcji.
    if run(&[
        "tar", "-xpf", tarball, "--numeric-owner", "--xattrs", "--xattrs-include=*", "-C",
        STAGE_DIR,
    ])
    .is_err()
    {
        log("tar nie przyjął opcji xattrs — rozpakowuję bez nich");
        run(&["tar", "-xpf", tarball, "--numeric-owner", "-C", STAGE_DIR])?;
    }
    Ok(())
}

/// Montowania wymagane do działania chroota (dev/proc/sys + resolv.conf).
pub fn setup_chroot_binds(root: &Path) -> Result<(), String> {
    let r = |s: &str| root.join(s).to_string_lossy().to_string();
    for d in ["dev", "proc", "sys", "tmp", "etc"] {
        fs::create_dir_all(root.join(d)).map_err(|e| format!("mkdir {}/{d}: {e}", root.display()))?;
    }
    run(&["mount", "-t", "proc", "proc", &r("proc")])?;
    for fsname in ["dev", "sys"] {
        run(&["mount", "--rbind", &format!("/{fsname}"), &r(fsname)])?;
        run(&["mount", "--make-rslave", &r(fsname)])?;
    }
    // DNS w chroocie: kopia działającej konfiguracji hosta.
    let resolv = root.join("etc/resolv.conf");
    let _ = fs::remove_file(&resolv);
    if let Ok(content) = fs::read_to_string("/etc/resolv.conf") {
        fs::write(&resolv, content).map_err(|e| format!("zapis resolv.conf: {e}"))?;
    }
    Ok(())
}

pub fn teardown_chroot_binds(root: &Path) {
    for d in ["proc", "dev", "sys"] {
        run_soft(&["umount", "-R", &root.join(d).to_string_lossy()]);
    }
}

// ---------------------------------------------------------------------------
// Bootstrap systemu bazowego
// ---------------------------------------------------------------------------

pub fn bootstrap(cfg: &InstallConfig) -> Result<(), String> {
    match cfg.mode {
        Mode::Live => {
            // -f: cel jest celowo „niepusty” — ma już zamontowany układ
            // subwolumenów (punkty montowania /home, /.snapshots itd.).
            match cfg.source {
                Source::Local => run(&["chimera-bootstrap", "-f", "-l", "/media/root"])
                    .map_err(|e| {
                        format!(
                            "{e}; instalacja lokalna wymaga repozytorium na nośniku — \
                             spróbuj instalacji z sieci"
                        )
                    }),
                Source::Network => run(&["chimera-bootstrap", "-f", "/media/root"]),
            }
        }
        Mode::Foreign => {
            let stage = Path::new(STAGE_DIR);
            setup_chroot_binds(stage)?;
            // Obraz bootstrap to tylko apk + chimerautils — narzędzia
            // instalacyjne trzeba najpierw dociągnąć z repozytorium.
            run_in(stage, &["apk", "update"])?;
            run_in(stage, &["apk", "add", "chimera-install-scripts"])?;
            // Zostawiamy montowania dev/proc/sys stage'a — target jest pod nim
            // i zaraz będziemy chrootować się do targetu bezpośrednio z hosta.
            // -f jak w trybie live: zamontowany układ subwolumenów to nie błąd.
            run_in(stage, &["chimera-bootstrap", "-f", "/media/root"])
        }
    }
}

// ---------------------------------------------------------------------------
// Konfiguracja systemu docelowego (wspólna dla obu trybów)
// ---------------------------------------------------------------------------

pub struct Uuids {
    pub root: String,
    pub esp: Option<String>,
}

pub fn read_uuids(parts: &TargetParts) -> Result<Uuids, String> {
    Ok(Uuids {
        root: fs_uuid(&parts.root)?,
        esp: match &parts.esp {
            Some(e) => Some(fs_uuid(e)?),
            None => None,
        },
    })
}

/// fstab w stylu CachyOS: / jako STAŁY subvol=@ (to on sprawia, że rollback
/// przez podmianę @ działa na waniliowym GRUB), /.snapshots jako osobny
/// @snapshots, pozostałe subwolumeny jawnie, ESP na /boot/efi.
pub fn write_fstab(target: &Path, cfg: &InstallConfig, uuids: &Uuids) -> Result<(), String> {
    let mut fstab = String::from("# /etc/fstab — wygenerowane przez chimera-installer\n");
    fstab.push_str(&format!("UUID={} / btrfs defaults,subvol=@ 0 0\n", uuids.root));
    fstab.push_str(&format!(
        "UUID={} /.snapshots btrfs subvol={SNAPSHOTS_SUBVOL} 0 0\n",
        uuids.root
    ));
    for (sv, mp) in SUBVOLS {
        fstab.push_str(&format!("UUID={} /{mp} btrfs subvol={sv} 0 0\n", uuids.root));
    }
    if cfg.uefi {
        if let Some(esp) = &uuids.esp {
            fstab.push_str(&format!("UUID={esp} /boot/efi vfat utf8 0 2\n"));
        }
    }
    fs::write(target.join("etc/fstab"), fstab).map_err(|e| format!("zapis fstab: {e}"))
}

/// Pakiety pulpitu ponad base-full (zweryfikowane symulacją apk -s w cports).
/// Wariant base wyklucza aplikacje (`!…`), ale dokłada terminal; full bierze
/// pełny metapakiet. gdm/sddm i networkmanager w obu (menedżer logowania +
/// sieć). Wykluczenia `!pkg` apk honoruje w jednej transakcji z resztą.
fn desktop_packages(d: Desktop, edition: Edition) -> &'static [&'static str] {
    match (d, edition) {
        (Desktop::Base, _) => &[],
        (Desktop::Gnome, Edition::Full) => &["gnome", "gdm", "networkmanager"],
        (Desktop::Gnome, Edition::Base) => {
            &["gnome", "!gnome-apps", "gnome-console", "gdm", "networkmanager"]
        }
        (Desktop::Kde, Edition::Full) => &["plasma-desktop", "sddm", "networkmanager"],
        (Desktop::Kde, Edition::Base) => &[
            "plasma-desktop",
            "!plasma-desktop-apps-meta",
            "!plasma-desktop-multimedia-meta",
            "konsole",
            "sddm",
            "networkmanager",
        ],
    }
}

/// Mikrokod CPU wg producenta — base-full go nie zawiera.
fn cpu_ucode_package() -> Option<&'static str> {
    let info = fs::read_to_string("/proc/cpuinfo").ok()?;
    if info.contains("GenuineIntel") {
        Some("ucode-intel")
    } else if info.contains("AuthenticAMD") {
        Some("ucode-amd")
    } else {
        None
    }
}

fn is_local_live(cfg: &InstallConfig) -> bool {
    cfg.mode == Mode::Live && cfg.source == Source::Local
}

pub fn install_packages(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    // Aktualizacja indeksów może się nie udać offline — to nie powód do przerwania.
    run_in_soft(target, &["apk", "update"]);
    run_in_soft(target, &["apk", "upgrade", "--available"]);
    if is_local_live(cfg) {
        // base-live trafia do celu wyłącznie przy kopii z nośnika (-l).
        run_in_soft(
            target,
            &["sh", "-c", "apk info -e base-live >/dev/null 2>&1 && apk del base-live || true"],
        );
    }

    // Snapper jest w repozytorium „user” cports — świeży system ma tylko
    // „main”, więc najpierw dokładamy wpis repozytorium i odświeżamy indeks.
    run_in(target, &["apk", "add", "chimera-repo-user"])?;
    run_in_soft(target, &["apk", "update"]);

    // openssl — hashowanie haseł (zob. set_password);
    // dinit-chimera-udev — bez niego instalacja sieciowa zostaje z
    // dinit-chimera-device-none (brak udevd); pakiet sam włącza swoją usługę.
    let mut pkgs: Vec<&str> = vec![
        "apk",
        "add",
        "btrfs-progs",
        "snapper",
        "openssl",
        "dinit-chimera-udev",
        // Watcher migawek → automatyczne update-grub (chimera-grub-watch).
        "inotify-tools",
    ];
    // Przy kopii z nośnika (-l) jądro przychodzi z systemu live;
    // w instalacji sieciowej base-full jądra nie zawiera.
    if !is_local_live(cfg) {
        pkgs.push("linux-stable");
    }
    if cfg.enable_ssh {
        pkgs.push("openssh");
    }
    pkgs.extend_from_slice(desktop_packages(cfg.desktop, cfg.edition));
    if cfg.uefi {
        pkgs.extend_from_slice(&["grub-x86_64-efi", "efibootmgr"]);
    } else {
        pkgs.push("grub-i386-pc");
    }
    run_in(target, &pkgs)?;

    // Miękko: brak mikrokodu nie blokuje instalacji (np. lokalnie bez sieci);
    // initramfs przegeneruje się później w kroku GRUB-a.
    if let Some(ucode) = cpu_ucode_package() {
        run_in_soft(target, &["apk", "add", ucode]);
    }
    Ok(())
}

/// Firmware i wczesny KMS dla sterowników wybranych na stronie GPU.
/// Wszystko to otwarte sterowniki z jądra — instalujemy tylko bloby
/// firmware (podpakiety firmware-linux) i dopisujemy moduły do initramfs;
/// update-initramfs uruchamia się później, w kroku GRUB-a.
pub fn configure_gpu(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    if cfg.gpus.is_empty() {
        return Ok(());
    }
    let mut firmware: std::collections::BTreeSet<&'static str> =
        std::collections::BTreeSet::new();
    let mut modules: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for g in &cfg.gpus {
        // Pusty wybór = nic nie konfigurujemy; wariant zastrzeżony NVIDIA
        // ma własny krok (firmware GSP i moduły idą z jego pakietów).
        if g.driver.is_empty() || g.driver.starts_with("nvidia-") {
            continue;
        }
        log(&format!(
            "GPU {}:{} -> sterownik {} (otwarty, z jądra)",
            g.vendor, g.device, g.driver
        ));
        firmware.extend(crate::config::driver_firmware(&g.driver));
        modules.insert(g.driver.clone());
    }

    if !firmware.is_empty() {
        // Miękko: firmware bywa niedostępny offline; system wstanie,
        // a brak zostanie wyraźnie w dzienniku.
        let mut args: Vec<&str> = vec!["apk", "add"];
        args.extend(firmware.iter().copied());
        run_in_soft(target, &args);
    }

    if !modules.is_empty() {
        // Wczesny KMS: moduły w initramfs (format pliku: jeden moduł na linię).
        let path = target.join("etc/initramfs-tools/modules");
        fs::create_dir_all(path.parent().unwrap())
            .map_err(|e| format!("mkdir initramfs-tools: {e}"))?;
        let mut content = fs::read_to_string(&path).unwrap_or_default();
        for m in &modules {
            if !content.lines().any(|l| l.trim() == m) {
                content.push_str(m);
                content.push('\n');
                log(&format!("wczesny KMS: moduł {m} dopisany do initramfs"));
            }
        }
        fs::write(&path, content).map_err(|e| format!("zapis modules initramfs: {e}"))?;
    }
    Ok(())
}

/// Usługi dinit wymagające RĘCZNEGO włączenia (ustalenia floty):
/// dbus/elogind/chrony/turnstiled/udevd/pipewire są włączane domyślnie
/// przez podpakiety *-dinit-links i NIE wolno ich dublować.
/// Jeden demon sieci naraz: dhcpcd dla base, NetworkManager dla pulpitów.
fn services_to_enable(cfg: &InstallConfig) -> Vec<&'static str> {
    // UWAGA: snapperd NIE trafia tu — to backend D-Bus uruchamiany na żądanie
    // (org.opensuse.Snapper.service). W boot.d startowałby przed dbus i padał
    // z DBus::ErrorException (signal 6). CLI snappera i tak go aktywuje.
    let mut v = vec!["syslog-ng"];
    match cfg.desktop {
        Desktop::Base => v.push("dhcpcd"),
        Desktop::Gnome => v.extend_from_slice(&["networkmanager", "gdm"]),
        Desktop::Kde => v.extend_from_slice(&["networkmanager", "sddm"]),
    }
    v
}

/// Odpowiednik `dinitctl enable` działający w chroocie (dinitctl wymaga
/// żywego demona): dowiązanie w /etc/dinit.d/boot.d/ wskazujące na plik
/// usługi. Cel MUSI być niezwisający — dinit przy skanowaniu waits-for.d
/// pomija wpisy, których nie da się dostatować (sprawdzone na żywym
/// systemie: `../nazwa` bez /etc/dinit.d/nazwa NIE jest ładowane,
/// bezwzględne /usr/lib/dinit.d/nazwa działa).
pub fn enable_services(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    let mut wanted: std::collections::BTreeSet<String> = services_to_enable(cfg)
        .into_iter()
        .map(str::to_string)
        .collect();

    // Tryb live: stan włączeń przenosi się z nośnika 1:1 — wszystko, co jest
    // w /etc/dinit.d/boot.d działającego live (SDDM, NetworkManager itd.),
    // ma być włączone także w systemie docelowym. Usługi czysto live'owe
    // odfiltruje bezpiecznik niżej (po `apk del base-live` nie mają już
    // plików usług w celu).
    if cfg.mode == Mode::Live {
        if let Ok(entries) = fs::read_dir("/etc/dinit.d/boot.d") {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.starts_with('.') {
                    wanted.insert(name);
                }
            }
        }
    }
    // Dokładnie jeden demon sieci; NetworkManager ma pierwszeństwo nad dhcpcd.
    if wanted.contains("networkmanager") {
        wanted.remove("dhcpcd");
    }
    // sshd sterowany wyłącznie opcją ze strony „System": domyślnie wyłączony,
    // także gdy nośnik live miał go włączonego (usuwamy go z lustra boot.d).
    if cfg.enable_ssh {
        wanted.insert("sshd".to_string());
    } else {
        wanted.remove("sshd");
    }

    let bootd = target.join("etc/dinit.d/boot.d");
    fs::create_dir_all(&bootd).map_err(|e| format!("mkdir boot.d: {e}"))?;
    for name in &wanted {
        let name = name.as_str();
        if !target.join("usr/lib/dinit.d").join(name).exists() {
            log(&format!(
                "OSTRZEŻENIE: brak pliku usługi {name} w /usr/lib/dinit.d — pomijam włączenie"
            ));
            continue;
        }
        if target.join("usr/lib/dinit.d/boot.d").join(name).exists() {
            log(&format!("usługa {name} jest już włączona domyślnie (dinit-links) — pomijam"));
            continue;
        }
        let link = bootd.join(name);
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(format!("/usr/lib/dinit.d/{name}"), &link)
            .map_err(|e| format!("dowiązanie usługi {name}: {e}"))?;
        log(&format!("włączono usługę dinit: {name}"));
    }
    Ok(())
}

pub fn configure_system(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    fs::write(target.join("etc/hostname"), format!("{}\n", cfg.hostname))
        .map_err(|e| format!("zapis /etc/hostname: {e}"))?;

    let tz = format!("/usr/share/zoneinfo/{}", cfg.timezone);
    if target.join(tz.trim_start_matches('/')).exists() {
        run_in(target, &["ln", "-sf", &tz, "/etc/localtime"])?;
    } else {
        log(&format!("OSTRZEŻENIE: strefa {tz} nie istnieje w systemie docelowym — pomijam"));
    }

    configure_locale_keyboard(cfg, target)?;
    write_machine_id(target)?;

    run_in(target, &["useradd", "-m", "-G", "wheel", &cfg.username])?;
    set_password(target, &cfg.username, &cfg.user_password)?;
    match &cfg.root_password {
        Some(pass) => set_password(target, "root", pass)?,
        None => log("konto roota pozostaje zablokowane (administracja: doas przez grupę wheel)"),
    }

    // Wygoda: `sudo` jako dowiązanie do `doas` (Chimera nie ma sudo).
    let sudo = target.join("usr/bin/sudo");
    let _ = fs::remove_file(&sudo);
    if let Err(e) = std::os::unix::fs::symlink("doas", &sudo) {
        log(&format!("OSTRZEŻENIE: nie udało się utworzyć /usr/bin/sudo → doas: {e}"));
    } else {
        log("utworzono dowiązanie /usr/bin/sudo → doas");
    }
    Ok(())
}

/// /etc/machine-id tworzy normalnie dopiero pierwszy rozruch (early-machine-id),
/// ale migawka FABRIC powstaje wcześniej — bootowana read-only nie ma już jak
/// go dorobić (transient fallback dinit-chimera ubija kaskada po seedrng,
/// który na ro pada), a bez niego nie wstaje stos sesji graficznej. Generujemy
/// go więc przy instalacji; pierwszy rozruch zastaje niepusty plik i nie robi nic.
fn write_machine_id(target: &Path) -> Result<(), String> {
    let mid = target.join("etc/machine-id");
    if fs::metadata(&mid).map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(());
    }
    // Spójność z identyfikatorem D-Bus: trigger apk pakietu dbus zdążył już
    // zrobić `dbus-uuidgen --ensure`, więc jeśli jest, przejmujemy jego wartość.
    let dbus_id = target.join("var/lib/dbus/machine-id");
    let id = match fs::read_to_string(&dbus_id) {
        Ok(s) if s.trim().len() == 32 => s.trim().to_string(),
        _ => {
            let mut buf = [0u8; 16];
            let mut f = fs::File::open("/dev/urandom")
                .map_err(|e| format!("otwarcie /dev/urandom: {e}"))?;
            std::io::Read::read_exact(&mut f, &mut buf)
                .map_err(|e| format!("odczyt /dev/urandom: {e}"))?;
            buf.iter().map(|b| format!("{b:02x}")).collect()
        }
    };
    fs::write(&mid, format!("{id}\n")).map_err(|e| format!("zapis /etc/machine-id: {e}"))?;
    log("wygenerowano /etc/machine-id (normalnie powstaje dopiero przy pierwszym rozruchu)");
    Ok(())
}

/// Język i układ klawiatury — tak, żeby obowiązywały też w konsoli tty.
/// Język: LANG w /etc/locale.conf (czyta go profile.d/locale.sh przy każdym
/// logowaniu). Klawiatura: /etc/default/keyboard stosowany przy boocie przez
/// usługę early-keyboard.target (setupcon). Dla pulpitów używamy układu XKB
/// (console-setup-xkb/ckbcomp przekłada go na konsolę) i dokładamy
/// /etc/X11/xorg.conf.d/00-keyboard.conf dla sesji graficznej; dla systemu
/// bazowego bez X — konsolowej mapy KMAP (loadkeys).
fn configure_locale_keyboard(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    fs::write(target.join("etc/locale.conf"), format!("LANG={}\n", cfg.locale))
        .map_err(|e| format!("zapis /etc/locale.conf: {e}"))?;

    let kbd_path = target.join("etc/default/keyboard");
    fs::create_dir_all(kbd_path.parent().unwrap())
        .map_err(|e| format!("mkdir /etc/default: {e}"))?;

    if cfg.desktop == Desktop::Base {
        // Bez środowiska graficznego: konsolowa mapa klawiszy.
        fs::write(&kbd_path, format!("KMAP={}\n", cfg.kb_layout))
            .map_err(|e| format!("zapis /etc/default/keyboard: {e}"))?;
    } else {
        let kbd = format!(
            "XKBMODEL=pc105\nXKBLAYOUT={}\nXKBVARIANT={}\nXKBOPTIONS=\n",
            cfg.kb_layout, cfg.kb_variant
        );
        fs::write(&kbd_path, kbd).map_err(|e| format!("zapis /etc/default/keyboard: {e}"))?;

        let xorg = format!(
            "Section \"InputClass\"\n    Identifier \"system-keyboard\"\n    \
             MatchIsKeyboard \"on\"\n    Option \"XkbLayout\" \"{}\"\n    \
             Option \"XkbModel\" \"pc105\"\n    Option \"XkbVariant\" \"{}\"\nEndSection\n",
            cfg.kb_layout, cfg.kb_variant
        );
        let xdir = target.join("etc/X11/xorg.conf.d");
        fs::create_dir_all(&xdir).map_err(|e| format!("mkdir xorg.conf.d: {e}"))?;
        fs::write(xdir.join("00-keyboard.conf"), xorg)
            .map_err(|e| format!("zapis 00-keyboard.conf: {e}"))?;
    }
    log(&format!(
        "język: {}, klawiatura: {} {}",
        cfg.locale, cfg.kb_layout, cfg.kb_variant
    ));
    Ok(())
}

/// Ustawia hasło użytkownika w systemie docelowym.
///
/// chpasswd z shadow na Chimerze kończy się kodem 0 NIE zapisując hasła
/// (brak stosu PAM dla chpasswd — wpis zostaje zablokowany „!”), więc hash
/// SHA-512-crypt liczymy openssl-em (hasło idzie przez stdin) i wpisujemy
/// przez `usermod -p`, a na końcu sprawdzamy, że wpis naprawdę wylądował.
fn set_password(target: &Path, user: &str, pass: &str) -> Result<(), String> {
    let hash = run_in_capture_stdin(
        target,
        &["openssl", "passwd", "-6", "-stdin"],
        &format!("{pass}\n"),
    )?;
    let hash = hash.trim().to_string();
    if !hash.starts_with("$6$") {
        return Err(format!(
            "openssl nie zwrócił hasha SHA-512 dla użytkownika {user}"
        ));
    }
    run_in_redacted(target, &["usermod", "-p", &hash, user], 2)?;
    run_in(
        target,
        &[
            "sh",
            "-c",
            &format!("grep -q '^{user}:\\$6\\$' /etc/shadow"),
        ],
    )
    .map_err(|_| format!("hasło użytkownika {user} nie zostało zapisane w /etc/shadow"))
}

// ---------------------------------------------------------------------------
// Snapper, GRUB i rollback w stylu CachyOS (stały subvol=@)
// ---------------------------------------------------------------------------

const SNAPPER_ROOT_CONF: &str = include_str!("assets/snapper-root.conf");
const GRUB_SNAPSHOTS: &str = include_str!("assets/80_snapshots.sh");
const GRUB_FABRIC: &str = include_str!("assets/12_fabric.sh");
const CHIMERA_ROLLBACK: &str = include_str!("assets/chimera-rollback.sh");
const GRUB_WATCH: &str = include_str!("assets/chimera-grub-watch.sh");
const GRUB_WATCH_SVC: &str = include_str!("assets/chimera-grub-watch.dinit");

pub fn configure_snapper(target: &Path) -> Result<(), String> {
    let confdir = target.join("etc/snapper/configs");
    fs::create_dir_all(&confdir).map_err(|e| format!("mkdir configs snappera: {e}"))?;
    fs::write(confdir.join("root"), SNAPPER_ROOT_CONF)
        .map_err(|e| format!("zapis konfiguracji snappera: {e}"))?;

    // Rejestr konfiguracji (odpowiednik sysconfig w openSUSE): SNAPPER_CONFIGS="root".
    let reg = target.join("etc/snapper/snapper");
    let mut content = fs::read_to_string(&reg).unwrap_or_default();
    if content.lines().any(|l| l.trim_start().starts_with("SNAPPER_CONFIGS=")) {
        content = content
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("SNAPPER_CONFIGS=") {
                    "SNAPPER_CONFIGS=\"root\"".to_string()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
    } else {
        content.push_str("SNAPPER_CONFIGS=\"root\"\n");
    }
    fs::write(&reg, content).map_err(|e| format!("zapis rejestru snappera: {e}"))?;
    // Metadane migawki wyjściowej (FABRIC) zapisuje fabric_to_root (snapper
    // CLI w chroocie padłby — snapperd wymaga działającego D-Bus).
    Ok(())
}

/// Zwieńczenie modelu „system instalowany w migawce FABRIC": zapisuje jej
/// metadane snappera, odmontowuje układ, klonuje FABRIC do stałego
/// subwolumenu roota (`btrfs subvolume snapshot FABRIC → @`, zapisywalnie)
/// i montuje układ ponownie z @ jako /. Ścieżka targetu się nie zmienia.
/// FABRIC pozostaje zapisywalnym subwolumenem — migawka read-only nie
/// wstawała do pulpitu (bez zapisu padały seedrng, machine-id i menedżer
/// logowania; zweryfikowane na żywo). Metadane: puste `cleanup`, więc
/// automatyczne czyszczenie snappera nie tyka migawki; info.xml piszemy
/// sami, bo snapper CLI w chroocie padłby bez D-Bus.
pub fn fabric_to_root(
    cfg: &InstallConfig,
    parts: &TargetParts,
    target: &Path,
) -> Result<(), String> {
    let date = run_capture(&["date", "-u", "+%F %T"])?.trim().to_string();
    let info = format!(
        "<?xml version=\"1.0\"?>\n<snapshot>\n  <type>single</type>\n  <num>1</num>\n  \
         <date>{date}</date>\n  <description>FABRIC</description>\n  \
         <userdata>\n    <key>important</key>\n    <value>yes</value>\n  </userdata>\n</snapshot>\n"
    );
    fs::write(target.join(".snapshots/1/info.xml"), info)
        .map_err(|e| format!("zapis info.xml migawki fabrycznej: {e}"))?;

    // Przemontowanie FABRIC → @. Odmontowanie musi się powieść w całości,
    // inaczej @ wylądowałby na zajętym punkcie montowania.
    teardown_chroot_binds(target);
    run(&["umount", "-R", &target.to_string_lossy()])?;

    fs::create_dir_all(TOP_MOUNT).map_err(|e| format!("mkdir {TOP_MOUNT}: {e}"))?;
    run(&["mount", "-o", "subvolid=5", &parts.root, TOP_MOUNT])?;
    let top = Path::new(TOP_MOUNT);
    let res = run(&[
        "btrfs",
        "subvolume",
        "snapshot",
        &top.join(FABRIC_SUBVOL).to_string_lossy(),
        &top.join("@").to_string_lossy(),
    ]);
    run_soft(&["umount", TOP_MOUNT]);
    res?;

    mount_target(cfg, parts, "@")?;
    setup_chroot_binds(target)?;
    log("sklonowano migawkę fabryczną FABRIC do stałego subwolumenu roota @");
    Ok(())
}

/// Blokada migawki fabrycznej — ostatni ruch instalacji: kopiuje świeżo
/// wygenerowany /boot/grub z @ do FABRIC (po rollbacku do stanu fabrycznego
/// @ musi mieć grub.cfg i moduły — GRUB czyta je z /@/boot/grub) i nakłada
/// chattr +i na katalog migawki. Immutable na rodzicu blokuje
/// `snapper delete 1`/`btrfs subvolume delete`, dopóki ktoś świadomie nie
/// zrobi `doas chattr -i /.snapshots/1` — root może to cofnąć, więc to
/// utrudnienie, nie absolutna blokada. chattr miękko: brak wsparcia nie
/// przerywa instalacji.
pub fn lock_fabric(target: &Path) -> Result<(), String> {
    run(&[
        "cp",
        "-a",
        &target.join("boot/grub").to_string_lossy(),
        &target.join(".snapshots/1/snapshot/boot").to_string_lossy(),
    ])?;
    run_soft(&["chattr", "+i", &target.join(".snapshots/1").to_string_lossy()]);
    log("zablokowano migawkę fabryczną „FABRIC” (nr 1; zdjęcie blokady: doas chattr -i /.snapshots/1)");
    Ok(())
}

/// Pliki rozruchowe i initramfs — wykonywane jeszcze W MIGAWCE FABRIC,
/// żeby stan fabryczny zawierał komplet (initramfs, generatory grub.d,
/// narzędzia rollbacku). Sam grub-install/update-grub robi install_grub —
/// dopiero po przełączeniu na @, inaczej prefix GRUB-a w ESP wskazywałby
/// migawkę i update-grub głównego systemu nie miałby żadnego skutku.
pub fn configure_grub_files(target: &Path) -> Result<(), String> {
    // Narzędzie rollbacku z CLI (podmiana subwolumenu @, model CachyOS)
    // — z prawem do uruchamiania (755: wykonywalny dla wszystkich, zapis roota).
    let bindir = target.join("usr/bin");
    fs::create_dir_all(&bindir).map_err(|e| format!("mkdir /usr/bin: {e}"))?;
    let rollback = bindir.join("chimera-rollback");
    fs::write(&rollback, CHIMERA_ROLLBACK)
        .map_err(|e| format!("zapis chimera-rollback: {e}"))?;
    run(&["chmod", "0755", &rollback.to_string_lossy()])?;

    // Watcher automatycznie odświeżający menu GRUB po zmianie migawek
    // (odpowiednik grub-btrfs.path z CachyOS). Skrypt + usługa dinit.
    let watch = bindir.join("chimera-grub-watch");
    fs::write(&watch, GRUB_WATCH).map_err(|e| format!("zapis chimera-grub-watch: {e}"))?;
    run(&["chmod", "0755", &watch.to_string_lossy()])?;
    let svc = target.join("usr/lib/dinit.d/chimera-grub-watch");
    fs::create_dir_all(svc.parent().unwrap()).map_err(|e| format!("mkdir dinit.d: {e}"))?;
    fs::write(&svc, GRUB_WATCH_SVC).map_err(|e| format!("zapis usługi watchera: {e}"))?;
    // Włączenie usługi dowiązaniem bezwzględnym w boot.d (jak enable_services).
    let bootd = target.join("etc/dinit.d/boot.d");
    fs::create_dir_all(&bootd).map_err(|e| format!("mkdir boot.d: {e}"))?;
    let link = bootd.join("chimera-grub-watch");
    let _ = fs::remove_file(&link);
    std::os::unix::fs::symlink("/usr/lib/dinit.d/chimera-grub-watch", &link)
        .map_err(|e| format!("dowiązanie usługi watchera: {e}"))?;

    // Generator wpisów GRUB dla bootowania migawek read-only.
    // Domyślny wpis GRUB montuje stały subvol=@ (grub-mkconfig doda
    // rootflags=subvol=@ sam) — po rollbacku podmieniającym @ wstaje
    // przywrócona zawartość bez regeneracji grub.cfg.
    let gdir = target.join("etc/grub.d");
    fs::create_dir_all(&gdir).map_err(|e| format!("mkdir grub.d: {e}"))?;
    let script = gdir.join("80_snapshots");
    fs::write(&script, GRUB_SNAPSHOTS).map_err(|e| format!("zapis 80_snapshots: {e}"))?;
    run(&["chmod", "755", &script.to_string_lossy()])?;
    // Stały wpis migawki fabrycznej (obecny w każdej migawce, bo w @).
    let fabric = gdir.join("12_fabric");
    fs::write(&fabric, GRUB_FABRIC).map_err(|e| format!("zapis 12_fabric: {e}"))?;
    run(&["chmod", "755", &fabric.to_string_lossy()])?;

    run_in(target, &["update-initramfs", "-c", "-k", "all"])
}

/// Instalacja GRUB-a do ESP/MBR i generacja grub.cfg — wykonywana w @
/// (po fabric_to_root), żeby wbudowany prefix i wpis główny wskazywały @.
pub fn install_grub(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    if cfg.uefi {
        let args = [
            "grub-install",
            "--target=x86_64-efi",
            "--efi-directory=/boot/efi",
            "--bootloader-id=chimera",
        ];
        if let Err(e) = run_in(target, &args) {
            // Brak dostępu do zmiennych EFI (częste przy instalacji z chroota
            // innego systemu) — wariant zapasowy do \EFI\BOOT.
            log(&format!(
                "grub-install z wpisem NVRAM nie powiódł się ({e}); instaluję wariant wymienny"
            ));
            run_in(
                target,
                &[
                    "grub-install",
                    "--target=x86_64-efi",
                    "--efi-directory=/boot/efi",
                    "--bootloader-id=chimera",
                    "--removable",
                    "--no-nvram",
                ],
            )?;
        }
    } else {
        run_in(target, &["grub-install", "--target=i386-pc", &cfg.disk])?;
    }
    run_in(target, &["update-grub"])
}

// ---------------------------------------------------------------------------
// Finalizacja
// ---------------------------------------------------------------------------

pub fn finalize(cfg: &InstallConfig, target: &Path) {
    teardown_chroot_binds(target);
    run_soft(&["umount", "-R", &target.to_string_lossy()]);
    if cfg.mode == Mode::Foreign {
        teardown_chroot_binds(Path::new(STAGE_DIR));
    }
    run_soft(&["sync"]);
}
