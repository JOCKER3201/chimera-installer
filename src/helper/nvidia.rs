// EXPERIMENTAL: building and installing the proprietary NVIDIA driver from
// the open cports PR (#6531) — bypassing the binary repository.
//
// Available in live mode only (cbuild must run on a live Chimera medium:
// it requires apk-tools 3, musl, bubblewrap, and must NOT be run inside
// a chroot or as root). Working directory: /tmp/musl-nvidia.
//
// The build happens at the VERY BEGINNING of the installation — before the
// target disk is wiped — so a potential failure (the option is
// experimental) does not leave the user with an erased disk.
// Installing the finished packages into the target happens later, once it
// is mounted.
//
// Sources of these findings: the research workflow around PR #6531
// (q66/nina) + the cbuild docs.

use super::cmd::{log, run, run_as_user, run_capture, run_in, run_soft};
use crate::config::{GpuChoice, InstallConfig};
use std::fs;
use std::path::Path;

const CPORTS_REPO: &str = "https://github.com/chimera-linux/cports";
const PR_BRANCH: &str = "nvidia-driver";
// Note: PR #6531 is a draft and gets force-pushed (rebased) every few
// hours, so any fixed commit pin quickly becomes unreachable on the
// branch. We therefore take the CURRENT branch tip and log its SHA.
pub const WORKDIR: &str = "/tmp/musl-nvidia";

struct Builder {
    uid: u32,
    gid: u32,
    name: String,
    home: String,
}

/// The account cbuild will run under (it refuses to run as root).
/// First the user who launched the installer (PKEXEC_UID), then the first
/// regular account from /etc/passwd.
fn find_builder() -> Result<Builder, String> {
    let passwd = fs::read_to_string("/etc/passwd")
        .map_err(|e| format!("reading /etc/passwd: {e}"))?;
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
        "cbuild cannot run as root, and no regular user account was found \
         on the live medium — launch the installer as a user, not as root"
            .to_string()
    })
}

/// Maps package names ("nvidia620") to cbuild templates ("user/nvidia620").
fn template_of(pkg: &str) -> Option<&'static str> {
    match pkg {
        "nvidia620" => Some("user/nvidia620"),
        "nvidia580" => Some("user/nvidia580"),
        _ => None,
    }
}

/// Driver package names chosen for the detected cards ("nvidia-620" ->
/// "nvidia620"); deduplicated.
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

/// Warns if the working directory sits on tmpfs (the build takes several
/// GB — on a live medium this risks exhausting RAM).
fn warn_if_tmpfs() {
    if let Ok(out) = run_capture(&["findmnt", "-n", "-o", "FSTYPE", "--target", "/tmp"]) {
        if out.trim() == "tmpfs" {
            log(
                "WARNING: /tmp is in memory (tmpfs). Building the NVIDIA driver \
                 takes several GB — with little RAM it may fail (OOM).",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 1: building the packages (background process `helper build-nvidia <pkg...>`)
// ---------------------------------------------------------------------------

/// Builds the driver packages from the PR via cbuild. Called at the start
/// of the installation, before the disk is wiped.
pub fn build(cfg: &InstallConfig) -> Result<(), String> {
    let pkgs = selected_packages(&cfg.gpus);
    let templates: Vec<&str> = pkgs.iter().filter_map(|p| template_of(p)).collect();
    if templates.is_empty() {
        return Ok(());
    }
    let b = find_builder()?;
    log(&format!(
        "building the NVIDIA driver from cports PR #6531 (branch {PR_BRANCH}) as {} (UID {})",
        b.name, b.uid
    ));
    warn_if_tmpfs();

    // Host dependencies for cbuild (ruff separately — build_lint is always invoked).
    run(&["apk", "add", "base-cbuild-host", "ruff"])?;

    // A fresh working directory owned by the building user.
    run_soft(&["rm", "-rf", WORKDIR]);
    run(&["mkdir", "-p", WORKDIR])?;
    run(&["chown", &format!("{}:{}", b.uid, b.gid), WORKDIR])?;

    let as_user = |cmd: &str| run_as_user(b.uid, b.gid, &b.name, &b.home, WORKDIR, cmd);

    // Clone the current tip of the PR branch (a real git clone, single
    // branch to save space), then record the commit that was used in the log.
    as_user(&format!(
        "git clone --branch {PR_BRANCH} --single-branch {CPORTS_REPO} ."
    ))?;
    as_user("git rev-parse HEAD")?;

    // Signing key for the local repo + bootstrap of the build environment.
    as_user("./cbuild keygen")?;
    as_user("./cbuild binary-bootstrap")?;

    // Build the selected templates (pulls musl-nvidia and the rest as needed).
    as_user(&format!("./cbuild pkg {}", templates.join(" ")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 2: installing the built packages into the target system
// ---------------------------------------------------------------------------

/// Installs the previously built packages into the target system and
/// configures the target. Called after the target is mounted (needs
/// /media/root).
pub fn install(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    let pkgs = selected_packages(&cfg.gpus);
    if pkgs.is_empty() {
        return Ok(());
    }
    install_to_target(&pkgs, target)
}

/// Installs the built packages into the target system and configures the target.
fn install_to_target(pkgs: &[String], target: &Path) -> Result<(), String> {
    // Install via `apk --root` from the HOST: the local repo lives in /tmp,
    // invisible inside the chroot. The ckms trigger chroots into the target
    // anyway (the proc/sys/dev binds are active), so the module builds in place.
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
        // The headers MUST match the installed kernel (linux-stable),
        // otherwise ckms cannot build the module.
        "linux-stable-devel".into(),
    ];
    for pkg in pkgs {
        args.push(pkg.clone());
        args.push(format!("{pkg}-ckms"));
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&arg_refs)?;

    // apk's ckms trigger tends to get interrupted (BrokenPipe / a known apk
    // bug) and leaves the module unfinished — so we build it explicitly and
    // synchronously.
    build_modules(target);

    configure_target(target)?;

    if !module_present(target) {
        log(
            "WARNING: no NVIDIA kernel module found despite the explicit build. \
             After booting the system try: doas sh -c 'IGNORE_CC_MISMATCH=1 \
             ckms -k <kernel-version> build nvidia && ckms -k <kernel-version> install nvidia \
             && depmod <kernel-version>'.",
        );
    } else {
        log("NVIDIA kernel module built and installed");
    }
    Ok(())
}

/// Whether any of the target's installed kernels has an nvidia*.ko* module.
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

/// Explicit ckms module build in the chroot for every installed kernel that
/// has headers. ckms's abundant output goes to a file in the target (so it
/// does not cut the stream or flood the log); only its tail reaches the log.
fn build_modules(target: &Path) {
    // IGNORE_CC_MISMATCH: Chimera's kernel may be built with a different
    // compiler than the module; the NVIDIA driver only warns, but without
    // this it can refuse.
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
    log("building the NVIDIA kernel module (ckms) — this will take a few minutes…");
    let _ = run_in(target, &["sh", "-c", &full]);

    // Tail of the build log into the main log (the full one is in cbuild's
    // build.log anyway).
    if let Ok(data) = fs::read_to_string(target.join("ckms-install.log")) {
        let lines: Vec<&str> = data.lines().collect();
        let start = lines.len().saturating_sub(20);
        for l in &lines[start..] {
            log(l);
        }
    }
    let _ = fs::remove_file(target.join("ckms-install.log"));
}

/// Target configuration specific to the proprietary driver.
/// The nouveau blacklist and the GSP firmware come with the packages — we
/// do not duplicate them.
fn configure_target(target: &Path) -> Result<(), String> {
    // Modeset for the default Wayland (gdm/sddm). initramfs-tools pulls
    // /etc/modprobe.d into the initramfs; the file is created BEFORE
    // update-initramfs (the GRUB step comes later). If the driver enables
    // modeset by itself, this entry is harmless.
    let conf = target.join("etc/modprobe.d/nvidia-modeset.conf");
    fs::create_dir_all(conf.parent().unwrap())
        .map_err(|e| format!("mkdir modprobe.d: {e}"))?;
    fs::write(&conf, "options nvidia_drm modeset=1\n")
        .map_err(|e| format!("writing nvidia-modeset.conf: {e}"))?;
    log("set nvidia_drm modeset=1 (Wayland)");
    Ok(())
}
