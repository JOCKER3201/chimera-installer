// Właściwa instalacja: btrfs w układzie openSUSE, bootstrap (live/chroot),
// konfiguracja systemu docelowego, snapper i GRUB.

use super::cmd::{log, run, run_capture, run_in, run_in_soft, run_in_stdin, run_soft};
use super::partition::{fs_uuid, TargetParts};
use crate::config::{InstallConfig, Mode};
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

/// Subwolumeny rozdzielone jak w domyślnej instalacji openSUSE.
/// (/boot/grub zamiast /boot/grub2/<platforma>, bo tak nazywa katalog GRUB
/// w Chimerze; cel jest ten sam — konfiguracja GRUB-a wspólna dla migawek.)
const SUBVOLS: &[&str] = &["home", "opt", "root", "srv", "usr/local", "var", "boot/grub"];

const TOP_MOUNT: &str = "/run/vanilla-installer/top";

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

        run(&["btrfs", "subvolume", "create", &at("@")])?;
        run(&["btrfs", "subvolume", "create", &at("@/.snapshots")])?;
        fs::create_dir_all(top.join("@/.snapshots/1"))
            .map_err(|e| format!("mkdir @/.snapshots/1: {e}"))?;
        run(&["btrfs", "subvolume", "create", &at("@/.snapshots/1/snapshot")])?;

        for sv in SUBVOLS {
            if let Some(parent) = Path::new(sv).parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(top.join("@").join(parent))
                        .map_err(|e| format!("mkdir @/{}: {e}", parent.display()))?;
                }
            }
            run(&["btrfs", "subvolume", "create", &at(&format!("@/{sv}"))])?;
        }
        // Jak w openSUSE: /var bez kopiowania przy zapisie (bazy danych, VM).
        run_soft(&["chattr", "+C", &at("@/var")]);

        // Metadane pierwszej migawki, żeby snapper widział ją jak w openSUSE.
        let date = run_capture(&["date", "-u", "+%F %T"])?.trim().to_string();
        let info = format!(
            "<?xml version=\"1.0\"?>\n<snapshot>\n  <type>single</type>\n  <num>1</num>\n  <date>{date}</date>\n  <description>first root filesystem</description>\n</snapshot>\n"
        );
        fs::write(top.join("@/.snapshots/1/info.xml"), info)
            .map_err(|e| format!("zapis info.xml: {e}"))?;

        // System żyje w migawce nr 1 — ustawiamy ją jako domyślny subwolumen.
        let id = run_capture(&[
            "btrfs",
            "inspect-internal",
            "rootid",
            &at("@/.snapshots/1/snapshot"),
        ])?;
        let id = id.trim().to_string();
        run(&["btrfs", "subvolume", "set-default", &id, TOP_MOUNT])?;
        Ok(())
    })();
    run_soft(&["umount", TOP_MOUNT]);
    res
}

pub fn mount_target(cfg: &InstallConfig, parts: &TargetParts) -> Result<PathBuf, String> {
    let target = target_mount(cfg);
    let t = target.to_string_lossy().to_string();
    fs::create_dir_all(&target).map_err(|e| format!("mkdir {t}: {e}"))?;
    // Bez subvol= — montuje się domyślny subwolumen (@/.snapshots/1/snapshot),
    // dokładnie tak, jak będzie montowany działający system.
    run(&["mount", &parts.root, &t])?;
    run(&["chmod", "755", &t])?;

    let mut mounts: Vec<(String, String)> = vec![(
        "/@/.snapshots".to_string(),
        ".snapshots".to_string(),
    )];
    for sv in SUBVOLS {
        mounts.push((format!("/@/{sv}"), sv.to_string()));
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
            // Najpierw z lokalnego repozytorium nośnika live; gdy go brak,
            // chimera-bootstrap w trybie sieciowym.
            if let Err(e) = run(&["chimera-bootstrap", "-l", "/media/root"]) {
                log(&format!(
                    "instalacja z repozytorium lokalnego nie powiodła się ({e}); próbuję z sieci"
                ));
                run(&["chimera-bootstrap", "/media/root"])?;
            }
            Ok(())
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
            run_in(stage, &["chimera-bootstrap", "/media/root"])
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

/// fstab w stylu openSUSE: / z domyślnego subwolumenu (bez subvol=),
/// pozostałe subwolumeny jawnie, ESP na /boot/efi.
pub fn write_fstab(target: &Path, cfg: &InstallConfig, uuids: &Uuids) -> Result<(), String> {
    let mut fstab = String::from("# /etc/fstab — wygenerowane przez vanilla-installer\n");
    fstab.push_str(&format!("UUID={} / btrfs defaults 0 0\n", uuids.root));
    fstab.push_str(&format!(
        "UUID={} /.snapshots btrfs subvol=/@/.snapshots 0 0\n",
        uuids.root
    ));
    for sv in SUBVOLS {
        fstab.push_str(&format!(
            "UUID={} /{sv} btrfs subvol=/@/{sv} 0 0\n",
            uuids.root
        ));
    }
    if cfg.uefi {
        if let Some(esp) = &uuids.esp {
            fstab.push_str(&format!("UUID={esp} /boot/efi vfat utf8 0 2\n"));
        }
    }
    fs::write(target.join("etc/fstab"), fstab).map_err(|e| format!("zapis fstab: {e}"))
}

pub fn install_packages(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    // Aktualizacja indeksów może się nie udać offline — to nie powód do przerwania.
    run_in_soft(target, &["apk", "update"]);
    run_in_soft(target, &["apk", "upgrade", "--available"]);
    // Zgodnie z dokumentacją: pozostałość po nośniku live wylatuje, jeśli jest.
    run_in_soft(
        target,
        &["sh", "-c", "apk info -e base-live >/dev/null 2>&1 && apk del base-live || true"],
    );

    let mut pkgs: Vec<&str> = vec!["apk", "add", "linux-lts", "btrfs-progs", "snapper"];
    if cfg.uefi {
        pkgs.extend_from_slice(&["grub-x86_64-efi", "efibootmgr"]);
    } else {
        pkgs.push("grub-i386-pc");
    }
    run_in(target, &pkgs)
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

    run_in(target, &["useradd", "-m", "-G", "wheel", &cfg.username])?;
    run_in_stdin(
        target,
        &["chpasswd"],
        &format!("{}:{}\n", cfg.username, cfg.user_password),
        true,
    )?;
    match &cfg.root_password {
        Some(pass) => run_in_stdin(target, &["chpasswd"], &format!("root:{pass}\n"), true)?,
        None => log("konto roota pozostaje zablokowane (administracja: doas przez grupę wheel)"),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Snapper i GRUB w stylu openSUSE
// ---------------------------------------------------------------------------

const SNAPPER_ROOT_CONF: &str = include_str!("assets/snapper-root.conf");
const GRUB_SNAPSHOTS: &str = include_str!("assets/80_snapshots.sh");

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

    // Usługa snapperd (dinit): dowiązanie jak po `dinitctl enable snapper`.
    let bootd = target.join("etc/dinit.d/boot.d");
    fs::create_dir_all(&bootd).map_err(|e| format!("mkdir boot.d: {e}"))?;
    if target.join("usr/lib/dinit.d/snapper").exists() {
        let link = bootd.join("snapper");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink("/usr/lib/dinit.d/snapper", &link)
            .map_err(|e| format!("dowiązanie usługi snapper: {e}"))?;
    } else {
        log("OSTRZEŻENIE: pakiet snappera nie dostarcza usługi dinit — pomijam włączenie");
    }
    Ok(())
}

pub fn configure_grub(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    // /etc/default/grub pochodzi z pakietu Chimery; dokładamy przełącznik
    // znany z openSUSE, od którego zależy generowanie wpisów migawek.
    let def = target.join("etc/default/grub");
    let mut content = fs::read_to_string(&def).unwrap_or_default();
    if !content.contains("SUSE_BTRFS_SNAPSHOT_BOOTING") {
        content.push_str(
            "\n# vanilla-installer: rozruch migawek btrfs (konwencja openSUSE)\nSUSE_BTRFS_SNAPSHOT_BOOTING=\"true\"\n",
        );
        fs::write(&def, content).map_err(|e| format!("zapis /etc/default/grub: {e}"))?;
    }

    let script = target.join("etc/grub.d/80_snapshots");
    fs::create_dir_all(script.parent().unwrap()).map_err(|e| format!("mkdir grub.d: {e}"))?;
    fs::write(&script, GRUB_SNAPSHOTS).map_err(|e| format!("zapis 80_snapshots: {e}"))?;
    run(&["chmod", "755", &script.to_string_lossy()])?;

    run_in(target, &["update-initramfs", "-c", "-k", "all"])?;

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
