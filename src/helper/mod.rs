// The root helper — launched by the GUI as:
//   pkexec <binary> helper
// The installation configuration arrives as JSON on stdin (passwords never
// touch the disk or the command line); progress goes out over a line
// protocol on stdout.

mod cmd;
mod install;
mod nvidia;
mod partition;

use crate::config::{InstallConfig, Mode};
use cmd::{done, fail, log, progress};
use std::io::Read;

/// Host tools required before we touch anything.
fn required_tools(cfg: &InstallConfig) -> Vec<&'static str> {
    let mut t = vec![
        "lsblk", "sfdisk", "wipefs", "blkid", "mount", "umount", "chroot", "mkfs.btrfs",
        "mkfs.vfat", "btrfs", "tar", "sha256sum", "udevadm", "chmod", "date", "uname",
    ];
    if cfg.mode == Mode::Live {
        t.push("chimera-bootstrap");
    }
    t
}

fn check_tools(cfg: &InstallConfig) -> Result<(), String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let missing: Vec<&str> = required_tools(cfg)
        .into_iter()
        .filter(|tool| {
            !path
                .split(':')
                .any(|dir| std::path::Path::new(dir).join(tool).exists())
        })
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "required tools missing on the host: {}",
            missing.join(", ")
        ))
    }
}

fn run_install(cfg: &InstallConfig) -> Result<(), String> {
    let mut total: u32 = match cfg.mode {
        Mode::Live => 11,
        Mode::Foreign => 13,
    };
    if nvidia::wants_proprietary(cfg) {
        // Separate steps for the build (before partitioning) and for the
        // installation into the target.
        total += 2;
    }
    let mut step = 0u32;
    let mut next = |text: &str| {
        step += 1;
        progress(step, total, text);
    };

    next("Checking the environment");
    if !nix_is_root() {
        return Err("the helper is not running as root — launch it via pkexec".into());
    }
    check_tools(cfg)?;

    // The experimental NVIDIA driver is built AT THE VERY BEGINNING, before
    // we touch the disk — a build failure (the option is a draft) then does
    // not leave the user with a wiped disk.
    if nvidia::wants_proprietary(cfg) {
        next("Building the proprietary NVIDIA driver (cbuild)");
        nvidia::build(cfg)?;
    }

    if cfg.mode == Mode::Foreign {
        next("Downloading the Chimera Linux base image");
        let tarball = install::foreign_download()?;
        next("Unpacking the image to /tmp/chimera-linux");
        install::foreign_extract(&tarball)?;
    }

    next("Partitioning the disk");
    let parts = partition::apply(cfg)?;
    log(&format!(
        "root partition: {}, ESP: {}",
        parts.root,
        parts.esp.as_deref().unwrap_or("(none — BIOS)")
    ));

    next("Creating filesystems");
    install::make_filesystems(&parts)?;

    next("Creating btrfs subvolumes (CachyOS layout)");
    install::make_subvolumes(&parts)?;

    next("Mounting the target system (FABRIC snapshot)");
    let target = install::mount_target(cfg, &parts, install::FABRIC_SUBVOL)?;

    next("Installing the base system (chimera-bootstrap)");
    let res = (|| -> Result<(), String> {
        install::bootstrap(cfg)?;

        next("Configuring the system (fstab, users, dinit services)");
        install::setup_chroot_binds(&target)?;
        let uuids = install::read_uuids(&parts)?;
        install::write_fstab(&target, cfg, &uuids)?;
        install::install_packages(cfg, &target)?;
        install::configure_gpu(cfg, &target)?;
        if nvidia::wants_proprietary(cfg) {
            next("Installing the proprietary NVIDIA driver into the system");
            nvidia::install(cfg, &target)?;
        }
        install::configure_system(cfg, &target)?;
        install::enable_services(cfg, &target)?;

        next("Configuring snapper (openSUSE-style)");
        install::configure_snapper(&target)?;

        next("Initramfs and boot files (inside the FABRIC snapshot)");
        install::configure_grub_files(&target)?;

        // The system is complete inside the FABRIC snapshot — clone it to
        // the permanent root subvolume @, install GRUB only there (the
        // prefix in the ESP must point at @, not at the snapshot), and
        // finally lock FABRIC.
        next("Cloning FABRIC to @ and installing GRUB");
        install::fabric_to_root(cfg, &parts, &target)?;
        install::install_grub(cfg, &target)?;
        install::lock_fabric(&target)?;
        Ok(())
    })();

    next("Finalizing (unmounting)");
    install::finalize(cfg, &target);
    res
}

/// `std::process::id()` returns the PID, not the UID — the actual root check
/// is done by reading /proc/self/status (no libc crate dependency).
fn nix_is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(|u| u == "0"))
        })
        .unwrap_or(false)
}

fn set_helper_env() {
    // pkexec clears the environment; tools are invoked by name, so PATH
    // must be complete.
    std::env::set_var(
        "PATH",
        "/usr/sbin:/usr/bin:/sbin:/bin:/usr/local/sbin:/usr/local/bin",
    );
    std::env::set_var("LC_ALL", "C");
}

pub fn main() -> i32 {
    set_helper_env();

    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        fail(&format!("cannot read the configuration from stdin: {e}"));
        return 1;
    }
    let cfg: InstallConfig = match serde_json::from_str(&raw) {
        Ok(c) => c,
        Err(e) => {
            fail(&format!("invalid installation configuration: {e}"));
            return 1;
        }
    };
    drop(raw);

    match run_install(&cfg) {
        Ok(()) => {
            log("installation finished successfully");
            done();
            0
        }
        Err(e) => {
            fail(&e);
            1
        }
    }
}
