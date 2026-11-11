// The installation proper: btrfs in the CachyOS layout (fixed subvol=@),
// bootstrap (live/chroot), target system configuration, snapper, GRUB and
// rollback.

use super::cmd::{
    log, run, run_capture, run_in, run_in_capture_stdin, run_in_redacted, run_in_soft, run_soft,
};
use super::partition::{fs_uuid, TargetParts};
use crate::config::{Desktop, Edition, InstallConfig, Mode, Source};
use std::fs;
use std::path::{Path, PathBuf};

/// Working directory of the from-another-system mode (requirement: /tmp/chimera-linux).
pub const STAGE_DIR: &str = "/tmp/chimera-linux";
/// Downloaded images are kept next to it, so a retry does not download again.
const CACHE_DIR: &str = "/tmp/chimera-linux-cache";
const REPO_URL: &str = "https://repo.chimera-linux.org/live/latest/";

/// Mount point of the target system. In live mode as per the Chimera docs
/// (/media/root); in foreign mode inside the unpacked stage, so that
/// chimera-bootstrap sees it as /media/root after the chroot.
pub fn target_mount(cfg: &InstallConfig) -> PathBuf {
    match cfg.mode {
        Mode::Live => PathBuf::from("/media/root"),
        Mode::Foreign => Path::new(STAGE_DIR).join("media/root"),
    }
}

/// Flat, named top-level subvolumes, CachyOS-style. / is a FIXED subvol=@ —
/// a rollback swaps the CONTENTS of @ (mv @ → copy, snapshot → @), so it
/// works on vanilla GRUB (no btrfs set-default, no patches).
/// /boot stays INSIDE @, so a rollback also covers the kernel/initrd.
/// Pair: (top-level subvolume name, mount point relative to /).
const SUBVOLS: &[(&str, &str)] = &[
    ("@home", "home"),
    ("@root", "root"),
    ("@srv", "srv"),
    ("@cache", "var/cache"),
    ("@tmp", "var/tmp"),
    ("@log", "var/log"),
];

/// The snapshots subvolume — MUST be separate (outside @) so it survives
/// the swap of @ during a rollback; otherwise the snapshots would vanish
/// together with the old @.
const SNAPSHOTS_SUBVOL: &str = "@snapshots";

/// The FABRIC factory snapshot (no. 1) — a PRIMARY subvolume: the system is
/// installed directly into it, and @ is created at the end of the
/// installation as its snapshot (`btrfs subvolume snapshot FABRIC → @`).
pub const FABRIC_SUBVOL: &str = "@snapshots/1/snapshot";

const TOP_MOUNT: &str = "/run/chimera-installer/top";

// ---------------------------------------------------------------------------
// Step: filesystems and subvolumes
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

        // @snapshots — a separate snapshot store. The installation target is
        // the FABRIC factory snapshot (@snapshots/1/snapshot); the permanent
        // root subvolume @ is created at the end of the installation as its
        // snapshot (fabric_to_root).
        run(&["btrfs", "subvolume", "create", &at(SNAPSHOTS_SUBVOL)])?;
        fs::create_dir_all(top.join(SNAPSHOTS_SUBVOL).join("1"))
            .map_err(|e| format!("mkdir {SNAPSHOTS_SUBVOL}/1: {e}"))?;
        run(&["btrfs", "subvolume", "create", &at(FABRIC_SUBVOL)])?;

        for (sv, _) in SUBVOLS {
            run(&["btrfs", "subvolume", "create", &at(sv)])?;
        }
        // Lots of small writes — no copy-on-write (like CachyOS).
        run_soft(&["chattr", "+C", &at("@log")]);
        run_soft(&["chattr", "+C", &at("@cache")]);
        Ok(())
    })();
    run_soft(&["umount", TOP_MOUNT]);
    res
}

/// Mounts the target layout with `root_subvol` as /. During the
/// installation that is the FABRIC snapshot; after it is cloned to @
/// (fabric_to_root) — the fixed subvol=@, exactly as GRUB and fstab will
/// mount it.
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

    // The separate snapshot store first, then the remaining subvolumes.
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
// From-another-system mode: downloading the image and preparing the chroot
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
    Err("neither curl nor wget available — cannot download the image".into())
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

/// Finds the newest chimera-linux-<arch>-ROOTFS-<date>-bootstrap.tar.gz
/// file in the directory listing (without the regex crate).
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
            // The names carry a YYYYMMDD date, so lexical comparison suffices.
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

/// Downloads (with sha256 verification) the bootstrap image into the cache.
pub fn foreign_download() -> Result<String, String> {
    let tool = downloader()?;
    let arch = run_capture(&["uname", "-m"])?.trim().to_string();
    log(&format!("looking for a bootstrap image for architecture {arch}"));
    let listing = fetch_text(tool, REPO_URL)?;
    let name = find_tarball_name(&listing, &arch)
        .ok_or_else(|| format!("no ROOTFS bootstrap image for {arch} found at {REPO_URL}"))?;
    log(&format!("newest image: {name}"));

    let sums = fetch_text(tool, &format!("{REPO_URL}sha256sums.txt"))?;
    let expected = sums
        .lines()
        .find(|l| l.contains(&name))
        .and_then(|l| l.split_whitespace().next())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("no checksum for {name} in sha256sums.txt"))?;

    fs::create_dir_all(CACHE_DIR).map_err(|e| format!("mkdir {CACHE_DIR}: {e}"))?;
    let dest = format!("{CACHE_DIR}/{name}");
    if Path::new(&dest).exists() && sha256_of(&dest)? == expected {
        log("image already downloaded, checksum matches — skipping the download");
        return Ok(dest);
    }
    fetch_file(tool, &format!("{REPO_URL}{name}"), &dest)?;
    let got = sha256_of(&dest)?;
    if got != expected {
        let _ = fs::remove_file(&dest);
        return Err(format!(
            "the image's SHA256 sum does not match (expected {expected}, got {got}) — aborted"
        ));
    }
    log("SHA256 sum matches");
    Ok(dest)
}

/// Unmounts everything under `dir` (deepest first) based on /proc/mounts.
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

/// Unpacks the image to /tmp/chimera-linux (after cleaning up leftovers).
pub fn foreign_extract(tarball: &str) -> Result<(), String> {
    umount_under(STAGE_DIR);
    if Path::new(STAGE_DIR).exists() {
        // Safety check: never remove a directory something is still mounted under.
        let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        if mounts
            .lines()
            .filter_map(|l| l.split_whitespace().nth(1))
            .any(|mp| mp == STAGE_DIR || mp.starts_with(&format!("{STAGE_DIR}/")))
        {
            return Err(format!(
                "there are still active mounts under {STAGE_DIR} — unmount them and try again"
            ));
        }
        fs::remove_dir_all(STAGE_DIR).map_err(|e| format!("cleaning {STAGE_DIR}: {e}"))?;
    }
    fs::create_dir_all(STAGE_DIR).map_err(|e| format!("mkdir {STAGE_DIR}: {e}"))?;
    // GNU tar: also preserve xattrs (e.g. file capabilities); when tar does
    // not know them (bsdtar), retry without those options.
    if run(&[
        "tar", "-xpf", tarball, "--numeric-owner", "--xattrs", "--xattrs-include=*", "-C",
        STAGE_DIR,
    ])
    .is_err()
    {
        log("tar rejected the xattrs options — unpacking without them");
        run(&["tar", "-xpf", tarball, "--numeric-owner", "-C", STAGE_DIR])?;
    }
    Ok(())
}

/// Mounts required for a working chroot (dev/proc/sys + resolv.conf).
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
    // DNS in the chroot: a copy of the host's working configuration.
    let resolv = root.join("etc/resolv.conf");
    let _ = fs::remove_file(&resolv);
    if let Ok(content) = fs::read_to_string("/etc/resolv.conf") {
        fs::write(&resolv, content).map_err(|e| format!("writing resolv.conf: {e}"))?;
    }
    Ok(())
}

pub fn teardown_chroot_binds(root: &Path) {
    for d in ["proc", "dev", "sys"] {
        run_soft(&["umount", "-R", &root.join(d).to_string_lossy()]);
    }
}

// ---------------------------------------------------------------------------
// Base system bootstrap
// ---------------------------------------------------------------------------

pub fn bootstrap(cfg: &InstallConfig) -> Result<(), String> {
    match cfg.mode {
        Mode::Live => {
            // -f: the target is deliberately "non-empty" — it already has
            // the subvolume layout mounted (mount points /home, /.snapshots
            // and so on).
            match cfg.source {
                Source::Local => run(&["chimera-bootstrap", "-f", "-l", "/media/root"])
                    .map_err(|e| {
                        format!(
                            "{e}; a local installation requires a repository on the medium — \
                             try the network installation"
                        )
                    }),
                Source::Network => run(&["chimera-bootstrap", "-f", "/media/root"]),
            }
        }
        Mode::Foreign => {
            let stage = Path::new(STAGE_DIR);
            setup_chroot_binds(stage)?;
            // The bootstrap image is just apk + chimerautils — the
            // installation tools must be fetched from the repository first.
            run_in(stage, &["apk", "update"])?;
            run_in(stage, &["apk", "add", "chimera-install-scripts"])?;
            // Keep the stage's dev/proc/sys mounts — the target sits under
            // it and we are about to chroot into the target directly from
            // the host.
            // -f as in live mode: the mounted subvolume layout is not an error.
            run_in(stage, &["chimera-bootstrap", "-f", "/media/root"])
        }
    }
}

// ---------------------------------------------------------------------------
// Target system configuration (common to both modes)
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

/// CachyOS-style fstab: / as a FIXED subvol=@ (this is what makes the
/// rollback-by-swapping-@ work on vanilla GRUB), /.snapshots as the
/// separate @snapshots, the remaining subvolumes explicitly, the ESP on
/// /boot/efi.
pub fn write_fstab(target: &Path, cfg: &InstallConfig, uuids: &Uuids) -> Result<(), String> {
    let mut fstab = String::from("# /etc/fstab — generated by chimera-installer\n");
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
    fs::write(target.join("etc/fstab"), fstab).map_err(|e| format!("writing fstab: {e}"))
}

/// Desktop packages on top of base-full (verified with an apk -s simulation
/// in cports). The base edition excludes the applications (`!…`) but adds a
/// terminal; full takes the complete metapackage. gdm/sddm and
/// networkmanager in both (login manager + networking). apk honors the
/// `!pkg` exclusions in a single transaction with the rest.
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

/// CPU microcode by vendor — base-full does not include it.
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
    // Index refresh can fail offline — not a reason to abort.
    run_in_soft(target, &["apk", "update"]);
    run_in_soft(target, &["apk", "upgrade", "--available"]);
    if is_local_live(cfg) {
        // base-live ends up in the target only with a copy from the medium (-l).
        run_in_soft(
            target,
            &["sh", "-c", "apk info -e base-live >/dev/null 2>&1 && apk del base-live || true"],
        );
    }

    // Snapper lives in the cports "user" repository — a fresh system has
    // only "main", so first add the repository entry and refresh the index.
    run_in(target, &["apk", "add", "chimera-repo-user"])?;
    run_in_soft(target, &["apk", "update"]);

    // openssl — password hashing (see set_password);
    // dinit-chimera-udev — without it the network installation is left with
    // dinit-chimera-device-none (no udevd); the package enables its own service.
    let mut pkgs: Vec<&str> = vec![
        "apk",
        "add",
        "btrfs-progs",
        "snapper",
        "openssl",
        "dinit-chimera-udev",
        // Snapshot watcher → automatic update-grub (chimera-grub-watch).
        "inotify-tools",
    ];
    // With a copy from the medium (-l) the kernel comes from the live
    // system; in a network installation base-full contains no kernel.
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

    // Soft: missing microcode does not block the installation (e.g. a local
    // install without network); the initramfs regenerates later in the GRUB
    // step.
    if let Some(ucode) = cpu_ucode_package() {
        run_in_soft(target, &["apk", "add", ucode]);
    }
    Ok(())
}

/// Firmware and early KMS for the drivers chosen on the GPU page.
/// These are all open in-kernel drivers — we install only the firmware
/// blobs (firmware-linux subpackages) and add the modules to the initramfs;
/// update-initramfs runs later, in the GRUB step.
pub fn configure_gpu(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    if cfg.gpus.is_empty() {
        return Ok(());
    }
    let mut firmware: std::collections::BTreeSet<&'static str> =
        std::collections::BTreeSet::new();
    let mut modules: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for g in &cfg.gpus {
        // Empty choice = configure nothing; the proprietary NVIDIA variant
        // has its own step (the GSP firmware and modules come with its
        // packages).
        if g.driver.is_empty() || g.driver.starts_with("nvidia-") {
            continue;
        }
        log(&format!(
            "GPU {}:{} -> driver {} (open, in-kernel)",
            g.vendor, g.device, g.driver
        ));
        firmware.extend(crate::config::driver_firmware(&g.driver));
        modules.insert(g.driver.clone());
    }

    if !firmware.is_empty() {
        // Soft: firmware may be unavailable offline; the system will boot,
        // and the omission stays clearly visible in the log.
        let mut args: Vec<&str> = vec!["apk", "add"];
        args.extend(firmware.iter().copied());
        run_in_soft(target, &args);
    }

    if !modules.is_empty() {
        // Early KMS: modules in the initramfs (file format: one module per line).
        let path = target.join("etc/initramfs-tools/modules");
        fs::create_dir_all(path.parent().unwrap())
            .map_err(|e| format!("mkdir initramfs-tools: {e}"))?;
        let mut content = fs::read_to_string(&path).unwrap_or_default();
        for m in &modules {
            if !content.lines().any(|l| l.trim() == m) {
                content.push_str(m);
                content.push('\n');
                log(&format!("early KMS: module {m} added to the initramfs"));
            }
        }
        fs::write(&path, content).map_err(|e| format!("writing initramfs modules: {e}"))?;
    }
    Ok(())
}

/// dinit services that require MANUAL enabling (fleet findings):
/// dbus/elogind/chrony/turnstiled/udevd/pipewire are enabled by default by
/// the *-dinit-links subpackages and MUST NOT be duplicated.
/// One network daemon at a time: dhcpcd for base, NetworkManager for desktops.
fn services_to_enable(cfg: &InstallConfig) -> Vec<&'static str> {
    // NOTE: snapperd does NOT go here — it is a D-Bus backend started on
    // demand (org.opensuse.Snapper.service). In boot.d it would start
    // before dbus and crash with DBus::ErrorException (signal 6). The
    // snapper CLI activates it anyway.
    let mut v = vec!["syslog-ng"];
    match cfg.desktop {
        Desktop::Base => v.push("dhcpcd"),
        Desktop::Gnome => v.extend_from_slice(&["networkmanager", "gdm"]),
        Desktop::Kde => v.extend_from_slice(&["networkmanager", "sddm"]),
    }
    v
}

/// The chroot-safe equivalent of `dinitctl enable` (dinitctl needs a live
/// daemon): a symlink in /etc/dinit.d/boot.d/ pointing at the service file.
/// The target MUST NOT dangle — while scanning waits-for.d, dinit skips
/// entries it cannot stat (verified on a live system: `../name` without
/// /etc/dinit.d/name is NOT loaded, an absolute /usr/lib/dinit.d/name works).
pub fn enable_services(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    let mut wanted: std::collections::BTreeSet<String> = services_to_enable(cfg)
        .into_iter()
        .map(str::to_string)
        .collect();

    // Live mode: the enablement state carries over from the medium 1:1 —
    // everything in /etc/dinit.d/boot.d of the running live system (SDDM,
    // NetworkManager and so on) should be enabled in the target system too.
    // Purely live services are filtered out by the safety check below
    // (after `apk del base-live` they no longer have service files in the
    // target).
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
    // Exactly one network daemon; NetworkManager takes precedence over dhcpcd.
    if wanted.contains("networkmanager") {
        wanted.remove("dhcpcd");
    }
    // sshd is controlled solely by the "System" page option: disabled by
    // default, even when the live medium had it enabled (we remove it from
    // the boot.d mirror).
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
                "WARNING: no service file {name} in /usr/lib/dinit.d — skipping enablement"
            ));
            continue;
        }
        if target.join("usr/lib/dinit.d/boot.d").join(name).exists() {
            log(&format!("service {name} is already enabled by default (dinit-links) — skipping"));
            continue;
        }
        let link = bootd.join(name);
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(format!("/usr/lib/dinit.d/{name}"), &link)
            .map_err(|e| format!("symlinking service {name}: {e}"))?;
        log(&format!("enabled dinit service: {name}"));
    }
    Ok(())
}

pub fn configure_system(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    fs::write(target.join("etc/hostname"), format!("{}\n", cfg.hostname))
        .map_err(|e| format!("writing /etc/hostname: {e}"))?;

    let tz = format!("/usr/share/zoneinfo/{}", cfg.timezone);
    if target.join(tz.trim_start_matches('/')).exists() {
        run_in(target, &["ln", "-sf", &tz, "/etc/localtime"])?;
    } else {
        log(&format!("WARNING: zone {tz} does not exist in the target system — skipping"));
    }

    configure_locale_keyboard(cfg, target)?;
    write_machine_id(target)?;

    run_in(target, &["useradd", "-m", "-G", "wheel", &cfg.username])?;
    set_password(target, &cfg.username, &cfg.user_password)?;
    match &cfg.root_password {
        Some(pass) => set_password(target, "root", pass)?,
        None => log("the root account stays locked (administration: doas via the wheel group)"),
    }

    // Convenience: `sudo` as a symlink to `doas` (Chimera has no sudo).
    let sudo = target.join("usr/bin/sudo");
    let _ = fs::remove_file(&sudo);
    if let Err(e) = std::os::unix::fs::symlink("doas", &sudo) {
        log(&format!("WARNING: failed to create /usr/bin/sudo → doas: {e}"));
    } else {
        log("created the /usr/bin/sudo → doas symlink");
    }
    Ok(())
}

/// /etc/machine-id is normally created only by the first boot
/// (early-machine-id), but the FABRIC snapshot is made earlier — booted
/// read-only it has no way to create it anymore (dinit-chimera's transient
/// fallback is killed by the cascade after seedrng, which fails on ro), and
/// without it the graphical session stack does not come up. So we generate
/// it at installation time; the first boot finds a non-empty file and does
/// nothing.
fn write_machine_id(target: &Path) -> Result<(), String> {
    let mid = target.join("etc/machine-id");
    if fs::metadata(&mid).map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(());
    }
    // Consistency with the D-Bus identifier: the dbus package's apk trigger
    // has already run `dbus-uuidgen --ensure`, so if it exists we adopt its
    // value.
    let dbus_id = target.join("var/lib/dbus/machine-id");
    let id = match fs::read_to_string(&dbus_id) {
        Ok(s) if s.trim().len() == 32 => s.trim().to_string(),
        _ => {
            let mut buf = [0u8; 16];
            let mut f = fs::File::open("/dev/urandom")
                .map_err(|e| format!("opening /dev/urandom: {e}"))?;
            std::io::Read::read_exact(&mut f, &mut buf)
                .map_err(|e| format!("reading /dev/urandom: {e}"))?;
            buf.iter().map(|b| format!("{b:02x}")).collect()
        }
    };
    fs::write(&mid, format!("{id}\n")).map_err(|e| format!("writing /etc/machine-id: {e}"))?;
    log("generated /etc/machine-id (normally created only on the first boot)");
    Ok(())
}

/// Language and keyboard layout — set up so they apply in the tty console
/// as well. Language: LANG in /etc/locale.conf (read by profile.d/locale.sh
/// on every login). Keyboard: /etc/default/keyboard applied at boot by the
/// early-keyboard.target service (setupcon). For desktops we use an XKB
/// layout (console-setup-xkb/ckbcomp translates it for the console) and add
/// /etc/X11/xorg.conf.d/00-keyboard.conf for the graphical session; for the
/// base system without X — a console KMAP keymap (loadkeys).
fn configure_locale_keyboard(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    fs::write(target.join("etc/locale.conf"), format!("LANG={}\n", cfg.locale))
        .map_err(|e| format!("writing /etc/locale.conf: {e}"))?;

    let kbd_path = target.join("etc/default/keyboard");
    fs::create_dir_all(kbd_path.parent().unwrap())
        .map_err(|e| format!("mkdir /etc/default: {e}"))?;

    if cfg.desktop == Desktop::Base {
        // No graphical environment: a console keymap.
        fs::write(&kbd_path, format!("KMAP={}\n", cfg.kb_layout))
            .map_err(|e| format!("writing /etc/default/keyboard: {e}"))?;
    } else {
        let kbd = format!(
            "XKBMODEL=pc105\nXKBLAYOUT={}\nXKBVARIANT={}\nXKBOPTIONS=\n",
            cfg.kb_layout, cfg.kb_variant
        );
        fs::write(&kbd_path, kbd).map_err(|e| format!("writing /etc/default/keyboard: {e}"))?;

        let xorg = format!(
            "Section \"InputClass\"\n    Identifier \"system-keyboard\"\n    \
             MatchIsKeyboard \"on\"\n    Option \"XkbLayout\" \"{}\"\n    \
             Option \"XkbModel\" \"pc105\"\n    Option \"XkbVariant\" \"{}\"\nEndSection\n",
            cfg.kb_layout, cfg.kb_variant
        );
        let xdir = target.join("etc/X11/xorg.conf.d");
        fs::create_dir_all(&xdir).map_err(|e| format!("mkdir xorg.conf.d: {e}"))?;
        fs::write(xdir.join("00-keyboard.conf"), xorg)
            .map_err(|e| format!("writing 00-keyboard.conf: {e}"))?;
    }
    log(&format!(
        "language: {}, keyboard: {} {}",
        cfg.locale, cfg.kb_layout, cfg.kb_variant
    ));
    Ok(())
}

/// Sets a user's password in the target system.
///
/// chpasswd from shadow on Chimera exits with code 0 WITHOUT saving the
/// password (no PAM stack for chpasswd — the entry stays locked with "!"),
/// so we compute a SHA-512-crypt hash with openssl (the password goes over
/// stdin) and write it with `usermod -p`, finally verifying that the entry
/// really landed.
fn set_password(target: &Path, user: &str, pass: &str) -> Result<(), String> {
    let hash = run_in_capture_stdin(
        target,
        &["openssl", "passwd", "-6", "-stdin"],
        &format!("{pass}\n"),
    )?;
    let hash = hash.trim().to_string();
    if !hash.starts_with("$6$") {
        return Err(format!(
            "openssl did not return a SHA-512 hash for user {user}"
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
    .map_err(|_| format!("the password for user {user} was not saved to /etc/shadow"))
}

// ---------------------------------------------------------------------------
// Snapper, GRUB and rollback, CachyOS-style (fixed subvol=@)
// ---------------------------------------------------------------------------

const SNAPPER_ROOT_CONF: &str = include_str!("assets/snapper-root.conf");
const GRUB_SNAPSHOTS: &str = include_str!("assets/80_snapshots.sh");
const GRUB_FABRIC: &str = include_str!("assets/12_fabric.sh");
const CHIMERA_ROLLBACK: &str = include_str!("assets/chimera-rollback.sh");
const GRUB_WATCH: &str = include_str!("assets/chimera-grub-watch.sh");
const GRUB_WATCH_SVC: &str = include_str!("assets/chimera-grub-watch.dinit");

pub fn configure_snapper(target: &Path) -> Result<(), String> {
    let confdir = target.join("etc/snapper/configs");
    fs::create_dir_all(&confdir).map_err(|e| format!("mkdir snapper configs: {e}"))?;
    fs::write(confdir.join("root"), SNAPPER_ROOT_CONF)
        .map_err(|e| format!("writing the snapper configuration: {e}"))?;

    // The configuration registry (openSUSE's sysconfig equivalent): SNAPPER_CONFIGS="root".
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
    fs::write(&reg, content).map_err(|e| format!("writing the snapper registry: {e}"))?;
    // The initial (FABRIC) snapshot's metadata is written by fabric_to_root
    // (the snapper CLI would crash in the chroot — snapperd needs a running
    // D-Bus).
    Ok(())
}

/// The capstone of the "system installed inside the FABRIC snapshot" model:
/// writes its snapper metadata, unmounts the layout, clones FABRIC to the
/// permanent root subvolume (`btrfs subvolume snapshot FABRIC → @`,
/// writable) and remounts the layout with @ as /. The target path does not
/// change. FABRIC remains a writable subvolume — a read-only snapshot did
/// not boot to the desktop (with no writes, seedrng, machine-id and the
/// login manager failed; verified live). Metadata: an empty `cleanup`, so
/// snapper's automatic cleanup leaves the snapshot alone; we write info.xml
/// ourselves because the snapper CLI would crash in the chroot without D-Bus.
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
        .map_err(|e| format!("writing the factory snapshot's info.xml: {e}"))?;

    // Remount FABRIC → @. The unmount must succeed in full, otherwise @
    // would land on a busy mount point.
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
    log("cloned the FABRIC factory snapshot to the permanent root subvolume @");
    Ok(())
}

/// Locking the factory snapshot — the installation's final move: copies the
/// freshly generated /boot/grub from @ into FABRIC (after a rollback to the
/// factory state, @ must have grub.cfg and the modules — GRUB reads them
/// from /@/boot/grub) and applies chattr +i to the snapshot directory.
/// Immutable on the parent blocks `snapper delete 1`/`btrfs subvolume
/// delete` until someone deliberately runs `doas chattr -i /.snapshots/1` —
/// root can undo it, so it is a deterrent, not an absolute lock. chattr is
/// soft: missing support does not abort the installation.
pub fn lock_fabric(target: &Path) -> Result<(), String> {
    run(&[
        "cp",
        "-a",
        &target.join("boot/grub").to_string_lossy(),
        &target.join(".snapshots/1/snapshot/boot").to_string_lossy(),
    ])?;
    run_soft(&["chattr", "+i", &target.join(".snapshots/1").to_string_lossy()]);
    log("locked the \"FABRIC\" factory snapshot (no. 1; to unlock: doas chattr -i /.snapshots/1)");
    Ok(())
}

/// Boot files and initramfs — done while still INSIDE the FABRIC snapshot,
/// so the factory state contains the full set (initramfs, grub.d
/// generators, the rollback tooling). grub-install/update-grub itself is
/// done by install_grub — only after switching to @, otherwise GRUB's
/// prefix in the ESP would point at the snapshot and the main system's
/// update-grub would have no effect.
pub fn configure_grub_files(target: &Path) -> Result<(), String> {
    // The CLI rollback tool (swapping the @ subvolume, the CachyOS model) —
    // executable (755: runnable by everyone, writable by root).
    let bindir = target.join("usr/bin");
    fs::create_dir_all(&bindir).map_err(|e| format!("mkdir /usr/bin: {e}"))?;
    let rollback = bindir.join("chimera-rollback");
    fs::write(&rollback, CHIMERA_ROLLBACK)
        .map_err(|e| format!("writing chimera-rollback: {e}"))?;
    run(&["chmod", "0755", &rollback.to_string_lossy()])?;

    // The watcher that automatically refreshes the GRUB menu when snapshots
    // change (the grub-btrfs.path equivalent from CachyOS). Script + dinit
    // service.
    let watch = bindir.join("chimera-grub-watch");
    fs::write(&watch, GRUB_WATCH).map_err(|e| format!("writing chimera-grub-watch: {e}"))?;
    run(&["chmod", "0755", &watch.to_string_lossy()])?;
    let svc = target.join("usr/lib/dinit.d/chimera-grub-watch");
    fs::create_dir_all(svc.parent().unwrap()).map_err(|e| format!("mkdir dinit.d: {e}"))?;
    fs::write(&svc, GRUB_WATCH_SVC).map_err(|e| format!("writing the watcher service: {e}"))?;
    // Enable the service with an absolute symlink in boot.d (like enable_services).
    let bootd = target.join("etc/dinit.d/boot.d");
    fs::create_dir_all(&bootd).map_err(|e| format!("mkdir boot.d: {e}"))?;
    let link = bootd.join("chimera-grub-watch");
    let _ = fs::remove_file(&link);
    std::os::unix::fs::symlink("/usr/lib/dinit.d/chimera-grub-watch", &link)
        .map_err(|e| format!("symlinking the watcher service: {e}"))?;

    // The GRUB entry generator for booting snapshots read-only.
    // GRUB's default entry mounts the fixed subvol=@ (grub-mkconfig adds
    // rootflags=subvol=@ itself) — after a rollback that swaps @, the
    // restored contents boot with no grub.cfg regeneration.
    let gdir = target.join("etc/grub.d");
    fs::create_dir_all(&gdir).map_err(|e| format!("mkdir grub.d: {e}"))?;
    let script = gdir.join("80_snapshots");
    fs::write(&script, GRUB_SNAPSHOTS).map_err(|e| format!("writing 80_snapshots: {e}"))?;
    run(&["chmod", "755", &script.to_string_lossy()])?;
    // The permanent factory snapshot entry (present in every snapshot, since it is in @).
    let fabric = gdir.join("12_fabric");
    fs::write(&fabric, GRUB_FABRIC).map_err(|e| format!("writing 12_fabric: {e}"))?;
    run(&["chmod", "755", &fabric.to_string_lossy()])?;

    run_in(target, &["update-initramfs", "-c", "-k", "all"])
}

/// Installing GRUB to the ESP/MBR and generating grub.cfg — done in @
/// (after fabric_to_root), so the embedded prefix and the main entry point
/// at @.
pub fn install_grub(cfg: &InstallConfig, target: &Path) -> Result<(), String> {
    if cfg.uefi {
        let args = [
            "grub-install",
            "--target=x86_64-efi",
            "--efi-directory=/boot/efi",
            "--bootloader-id=chimera",
        ];
        if let Err(e) = run_in(target, &args) {
            // No access to the EFI variables (common when installing from
            // another system's chroot) — fall back to the \EFI\BOOT variant.
            log(&format!(
                "grub-install with an NVRAM entry failed ({e}); installing the removable variant"
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
// Finalization
// ---------------------------------------------------------------------------

pub fn finalize(cfg: &InstallConfig, target: &Path) {
    teardown_chroot_binds(target);
    run_soft(&["umount", "-R", &target.to_string_lossy()]);
    if cfg.mode == Mode::Foreign {
        teardown_chroot_binds(Path::new(STAGE_DIR));
    }
    run_soft(&["sync"]);
}
