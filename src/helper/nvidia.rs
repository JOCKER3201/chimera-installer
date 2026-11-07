// EKSPERYMENTALNE: budowa i instalacja zastrzeżonego sterownika NVIDIA
// z otwartego PR-a cports (#6531) — z pominięciem repozytorium binarnego.
//
// Dostępne wyłącznie w trybie live (cbuild musi działać na żywym nośniku
// Chimery: wymaga apk-tools 3, musl, bubblewrap i NIE wolno go odpalać
// w chroocie ani jako root). Katalog roboczy: /tmp/musl-nvidia.
//
// Budowa odbywa się na SAMYM POCZĄTKU instalacji — jeszcze przed
// wyczyszczeniem dysku docelowego — żeby ewentualna porażka (opcja jest
// eksperymentalna) nie zostawiła użytkownika z wymazanym dyskiem.
// Instalacja gotowych pakietów do celu następuje później, po jego
// zamontowaniu.
//
// Źródła ustaleń: workflow badawczy nad PR #6531 (q66/nina) + docs cbuild.

use super::cmd::{log, run, run_as_user, run_capture, run_in, run_soft};
use crate::config::{GpuChoice, InstallConfig};
use std::fs;
use std::path::Path;

const CPORTS_REPO: &str = "https://github.com/chimera-linux/cports";
const PR_BRANCH: &str = "nvidia-driver";
// Uwaga: PR #6531 jest „draft” i bywa force-pushowany (rebase) co kilka
// godzin, przez co każdy sztywny pin commita szybko przestaje być osiągalny
// w gałęzi. Dlatego bierzemy AKTUALNY tip gałęzi i logujemy jego SHA.
pub const WORKDIR: &str = "/tmp/musl-nvidia";

struct Builder {
    uid: u32,
    gid: u32,
    name: String,
    home: String,
}

/// Konto, z którego uruchomimy cbuild (odmawia pracy jako root).
/// Najpierw użytkownik, który odpalił instalator (PKEXEC_UID), potem
/// pierwsze zwykłe konto z /etc/passwd.
fn find_builder() -> Result<Builder, String> {
    let passwd = fs::read_to_string("/etc/passwd")
        .map_err(|e| format!("odczyt /etc/passwd: {e}"))?;
    let lookup = |want_uid: Option<u32>| -> Option<Builder> {
        for line in passwd.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                continue;
            }
            let uid: u32 = f[2].parse().ok()?;
            let gid: u32 = f[3].parse().ok()?;
            let hit = match want_uid {
                Some(w) => uid == w,
                None => uid >= 1000 && uid < 65000 && f[6] != "/usr/sbin/nologin",
            };
            if hit {
                return Some(Builder {
                    uid,
                    gid,
                    name: f[0].to_string(),
                    home: f[5].to_string(),
                });
            }
        }
        None
    };

    if let Some(uid) = std::env::var("PKEXEC_UID").ok().and_then(|s| s.parse::<u32>().ok()) {
        if uid != 0 {
            if let Some(b) = lookup(Some(uid)) {
                return Ok(b);
            }
        }
    }
    lookup(None).ok_or_else(|| {
        "cbuild nie może działać jako root, a nie znaleziono zwykłego konta użytkownika \
         na nośniku live — uruchom instalator jako użytkownik, nie root"
            .to_string()
    })
}

/// Mapuje nazwy pakietów ("nvidia620") na szablony cbuild ("user/nvidia620").
fn template_of(pkg: &str) -> Option<&'static str> {
    match pkg {
        "nvidia620" => Some("user/nvidia620"),
        "nvidia580" => Some("user/nvidia580"),
        _ => None,
    }
}

/// Nazwy pakietów sterownika wybrane dla wykrytych kart ("nvidia-620" ->
/// "nvidia620"); bez duplikatów.
pub fn selected_packages(gpus: &[GpuChoice]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in gpus {
        let pkg = match g.driver.as_str() {
            "nvidia-620" => "nvidia620",
            "nvidia-580" => "nvidia580",
            _ => continue,
        };
        if !out.iter().any(|p| p == pkg) {
            out.push(pkg.to_string());
        }
    }
    out
}

pub fn wants_proprietary(cfg: &InstallConfig) -> bool {
    cfg.gpus.iter().any(|g| g.driver.starts_with("nvidia-"))
}

/// Ostrzega, jeśli katalog roboczy leży na tmpfs (build to kilka GB —
/// na nośniku live grozi to wyczerpaniem RAM-u).
fn warn_if_tmpfs() {
    if let Ok(out) = run_capture(&["findmnt", "-n", "-o", "FSTYPE", "--target", "/tmp"]) {
        if out.trim() == "tmpfs" {
            log(
                "OSTRZEŻENIE: /tmp jest w pamięci (tmpfs). Budowa sterownika NVIDIA \
                 zajmuje kilka GB — przy małej ilości RAM może się nie powieść (OOM).",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Faza 1: budowa pakietów (proces w tle `helper build-nvidia <pkg...>`)
// ---------------------------------------------------------------------------

/// Buduje pakiety sterownika z PR-a przez cbuild. Wołana na początku
/// instalacji, przed wyczyszczeniem dysku.
pub fn build(cfg: &InstallConfig) -> Result<(), String> {
    let pkgs = selected_packages(&cfg.gpus);
    let templates: Vec<&str> = pkgs.iter().filter_map(|p| template_of(p)).collect();
    if templates.is_empty() {
        return Ok(());
    }
    let b = find_builder()?;
    log(&format!(
        "budowa sterownika NVIDIA z PR cports #6531 (gałąź {PR_BRANCH}) jako {} (UID {})",
        b.name, b.uid
    ));
    warn_if_tmpfs();

    // Zależności hosta do cbuild (ruff osobno — build_lint woła się zawsze).
    run(&["apk", "add", "base-cbuild-host", "ruff"])?;

    // Świeży katalog roboczy należący do użytkownika budującego.
    run_soft(&["rm", "-rf", WORKDIR]);
    run(&["mkdir", "-p", WORKDIR])?;
    run(&["chown", &format!("{}:{}", b.uid, b.gid), WORKDIR])?;

    let as_user = |cmd: &str| run_as_user(b.uid, b.gid, &b.name, &b.home, WORKDIR, cmd);

    // Klon aktualnego tipa gałęzi PR-a (prawdziwy klon gita, pojedyncza
    // gałąź dla oszczędności), a następnie zapis użytego commita do dziennika.
    as_user(&format!(
        "git clone --branch {PR_BRANCH} --single-branch {CPORTS_REPO} ."
    ))?;
    as_user("git rev-parse HEAD")?;

    // Klucz podpisujący lokalne repo + bootstrap środowiska budowy.
    as_user("./cbuild keygen")?;
    as_user("./cbuild binary-bootstrap")?;

    // Budowa wybranych gałęzi (ciągnie też musl-nvidia i resztę zależnie).
    as_user(&format!("./cbuild pkg {}", templates.join(" ")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Faza 2: instalacja zbudowanych pakietów do systemu docelowego
// ---------------------------------------------------------------------------

/// Instaluje zbudowane wcześniej pakiety do systemu docelowego i konfiguruje
/// cel. Wołana po zamontowaniu celu (potrzebuje /media/root).
pub fn install(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    let pkgs = selected_packages(&cfg.gpus);
    if pkgs.is_empty() {
        return Ok(());
    }
    install_to_target(&pkgs, target)
}

/// Instaluje zbudowane pakiety do systemu docelowego i konfiguruje cel.
fn install_to_target(pkgs: &[String], target: &Path) -> Result<(), String> {
    // Instalacja przez `apk --root` z HOSTA: lokalne repo leży w /tmp,
    // niewidocznym w chroocie. Trigger ckms i tak chrootuje się do celu
    // (bindy proc/sys/dev są aktywne), więc moduł zbuduje się na miejscu.
    let root = target.to_string_lossy().to_string();
    let main_repo = format!("{WORKDIR}/packages/main");
    let user_repo = format!("{WORKDIR}/packages/user");
    let mut args: Vec<String> = vec![
        "apk".into(),
        "add".into(),
        "--root".into(),
        root,
        "--allow-untrusted".into(),
        "-X".into(),
        main_repo,
        "-X".into(),
        user_repo,
        // Nagłówki MUSZĄ odpowiadać zainstalowanemu jądru (linux-stable),
        // inaczej ckms nie zbuduje modułu.
        "linux-stable-devel".into(),
    ];
    for pkg in pkgs {
        args.push(pkg.clone());
        args.push(format!("{pkg}-ckms"));
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&arg_refs)?;

    // Trigger ckms w `apk` bywa przerywany (BrokenPipe / znany błąd apk)
    // i nie dokańcza modułu — budujemy go więc jawnie i synchronicznie.
    build_modules(target);

    configure_target(target)?;

    if !module_present(target) {
        log(
            "OSTRZEŻENIE: mimo jawnej budowy nie znaleziono modułu jądra NVIDIA. \
             Po uruchomieniu systemu spróbuj: doas sh -c 'IGNORE_CC_MISMATCH=1 \
             ckms -k <wersja-jądra> build nvidia && ckms -k <wersja-jądra> install nvidia \
             && depmod <wersja-jądra>'.",
        );
    } else {
        log("moduł jądra NVIDIA zbudowany i zainstalowany");
    }
    Ok(())
}

/// Czy w którymś z zainstalowanych jąder celu jest moduł nvidia*.ko*.
fn module_present(target: &Path) -> bool {
    fn walk(dir: &Path) -> bool {
        let Ok(rd) = fs::read_dir(dir) else {
            return false;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if walk(&p) {
                    return true;
                }
            } else if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
                if n.starts_with("nvidia") && n.contains(".ko") {
                    return true;
                }
            }
        }
        false
    }
    walk(&target.join("usr/lib/modules"))
}

/// Jawna budowa modułów ckms w chroocie dla każdego zainstalowanego jądra
/// z nagłówkami. Obfity output ckms idzie do pliku w celu (żeby nie urwać
/// strumienia i nie zalać dziennika); do dziennika trafia tylko jego ogon.
fn build_modules(target: &Path) {
    // IGNORE_CC_MISMATCH: jądro Chimery bywa budowane innym kompilatorem niż
    // moduł; sterownik NVIDIA tylko ostrzega, ale bez tego potrafi odmówić.
    let script = "set +e\n\
        for ini in /usr/src/*/ckms.ini; do [ -e \"$ini\" ] && ckms add \"${ini%/ckms.ini}\" >/dev/null 2>&1; done\n\
        export IGNORE_CC_MISMATCH=1\n\
        for kdir in /usr/lib/modules/*; do\n\
          [ -d \"$kdir/build\" ] || continue\n\
          kv=${kdir##*/}\n\
          ckms -k \"$kv\" plain-status 2>/dev/null | while read m v _rest; do\n\
            [ -n \"$m\" ] || continue\n\
            ckms -k \"$kv\" build \"$m=$v\"\n\
            ckms -k \"$kv\" install \"$m=$v\"\n\
          done\n\
          depmod \"$kv\"\n\
        done\n";
    let full = format!("{{ {script} }} >/ckms-install.log 2>&1");
    log("buduję moduł jądra NVIDIA (ckms) — to potrwa kilka minut…");
    let _ = run_in(target, &["sh", "-c", &full]);

    // Ogon dziennika budowy do głównego logu (pełność i tak w build.log cbuilda).
    if let Ok(data) = fs::read_to_string(target.join("ckms-install.log")) {
        let lines: Vec<&str> = data.lines().collect();
        let start = lines.len().saturating_sub(20);
        for l in &lines[start..] {
            log(l);
        }
    }
    let _ = fs::remove_file(target.join("ckms-install.log"));
}

/// Konfiguracja celu specyficzna dla sterownika zastrzeżonego.
/// Blacklistę nouveau i firmware GSP dostarczają pakiety — nie dublujemy.
fn configure_target(target: &Path) -> Result<(), String> {
    // Modeset dla domyślnego Waylanda (gdm/sddm). initramfs-tools wciąga
    // /etc/modprobe.d do initramfs; plik powstaje PRZED update-initramfs
    // (krok GRUB-a jest później). Jeśli sterownik włącza modeset sam,
    // ten wpis jest nieszkodliwy.
    let conf = target.join("etc/modprobe.d/nvidia-modeset.conf");
    fs::create_dir_all(conf.parent().unwrap())
        .map_err(|e| format!("mkdir modprobe.d: {e}"))?;
    fs::write(&conf, "options nvidia_drm modeset=1\n")
        .map_err(|e| format!("zapis nvidia-modeset.conf: {e}"))?;
    log("ustawiono nvidia_drm modeset=1 (Wayland)");
    Ok(())
}
