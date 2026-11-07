// Helper roota — uruchamiany przez GUI jako:
//   pkexec <binarka> helper
// Konfiguracja instalacji przychodzi JSON-em na stdin (hasła nie dotykają
// dysku ani wiersza poleceń), postęp wychodzi protokołem liniowym na stdout.

mod cmd;
mod install;
mod partition;

use crate::config::{InstallConfig, Mode};
use cmd::{done, fail, log, progress};
use std::io::Read;

/// Narzędzia hosta wymagane zanim czegokolwiek dotkniemy.
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
            "brak wymaganych narzędzi na hoście: {}",
            missing.join(", ")
        ))
    }
}

fn run_install(cfg: &InstallConfig) -> Result<(), String> {
    let total: u32 = match cfg.mode {
        Mode::Live => 10,
        Mode::Foreign => 12,
    };
    let mut step = 0u32;
    let mut next = |text: &str| {
        step += 1;
        progress(step, total, text);
    };

    next("Sprawdzanie środowiska");
    if !nix_is_root() {
        return Err("helper nie działa jako root — uruchom przez pkexec".into());
    }
    check_tools(cfg)?;

    if cfg.mode == Mode::Foreign {
        next("Pobieranie obrazu bazowego Chimera Linux");
        let tarball = install::foreign_download()?;
        next("Rozpakowywanie obrazu do /tmp/chimera-linux");
        install::foreign_extract(&tarball)?;
    }

    next("Partycjonowanie dysku");
    let parts = partition::apply(cfg)?;
    log(&format!(
        "partycja root: {}, ESP: {}",
        parts.root,
        parts.esp.as_deref().unwrap_or("(brak — BIOS)")
    ));

    next("Tworzenie systemów plików");
    install::make_filesystems(&parts)?;

    next("Tworzenie subwolumenów btrfs (układ openSUSE)");
    install::make_subvolumes(&parts)?;

    next("Montowanie systemu docelowego");
    let target = install::mount_target(cfg, &parts)?;

    next("Instalacja systemu bazowego (chimera-bootstrap)");
    let res = (|| -> Result<(), String> {
        install::bootstrap(cfg)?;

        next("Konfiguracja systemu (fstab, użytkownicy, strefa czasowa)");
        install::setup_chroot_binds(&target)?;
        let uuids = install::read_uuids(&parts)?;
        install::write_fstab(&target, cfg, &uuids)?;
        install::install_packages(cfg, &target)?;
        install::configure_system(cfg, &target)?;

        next("Konfiguracja snappera (jak w openSUSE)");
        install::configure_snapper(&target)?;

        next("Instalacja GRUB-a i initramfs");
        install::configure_grub(cfg, &target)?;
        Ok(())
    })();

    next("Finalizacja (odmontowywanie)");
    install::finalize(cfg, &target);
    res
}

/// `std::process::id()` zwraca PID, nie UID — faktyczną kontrolę roota
/// robimy przez odczyt /proc/self/status (bez crate'a libc).
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

pub fn main() -> i32 {
    // pkexec czyści środowisko; narzędzia wołamy po nazwie, więc PATH musi być pełny.
    std::env::set_var(
        "PATH",
        "/usr/sbin:/usr/bin:/sbin:/bin:/usr/local/sbin:/usr/local/bin",
    );
    std::env::set_var("LC_ALL", "C");

    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        fail(&format!("nie można odczytać konfiguracji ze stdin: {e}"));
        return 1;
    }
    let cfg: InstallConfig = match serde_json::from_str(&raw) {
        Ok(c) => c,
        Err(e) => {
            fail(&format!("niepoprawna konfiguracja instalacji: {e}"));
            return 1;
        }
    };
    drop(raw);

    match run_install(&cfg) {
        Ok(()) => {
            log("instalacja zakończona pomyślnie");
            done();
            0
        }
        Err(e) => {
            fail(&e);
            1
        }
    }
}
