// Partitioning in the root helper: verifying the GUI's plan against a fresh
// disk scan, building the sfdisk script and locating the new partitions.

use super::cmd::{log, run, run_capture, run_soft, run_stdin};
use crate::config::{
    InstallConfig, Strategy, BIOSBOOT_BYTES, BIOSBOOT_GUID, ESP_BYTES, ESP_GUID, SECTOR,
};
use crate::disks;

/// Partitioning result: the devices the system will live on.
pub struct TargetParts {
    pub root: String,
    /// ESP to mount under /boot/efi (existing or freshly created).
    pub esp: Option<String>,
    /// Whether the ESP needs mkfs.vfat (freshly created only).
    pub esp_format: bool,
}

const LINUX_GUID: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";

fn find_disk(cfg: &InstallConfig) -> Result<disks::DiskInfo, String> {
    let all = disks::scan()?;
    all.into_iter()
        .find(|d| d.path == cfg.disk)
        .ok_or_else(|| format!("disk {} does not exist", cfg.disk))
}

/// Unmounts everything mounted from the given disk's partitions
/// (only when wiping the whole disk).
fn umount_disk(disk: &disks::DiskInfo) {
    for p in &disk.parts {
        for mp in &p.mountpoints {
            if mp == "[SWAP]" {
                run_soft(&["swapoff", &p.path]);
            } else {
                run_soft(&["umount", "-R", mp]);
            }
        }
    }
}

fn settle() {
    run_soft(&["udevadm", "settle"]);
}

/// After sfdisk, finds a partition by its start sector.
fn part_by_start(disk_path: &str, start: u64) -> Result<String, String> {
    let all = disks::scan()?;
    let disk = all
        .iter()
        .find(|d| d.path == disk_path)
        .ok_or_else(|| format!("disk {disk_path} disappeared after partitioning"))?;
    disk.parts
        .iter()
        .find(|p| p.start == start)
        .map(|p| p.path.clone())
        .ok_or_else(|| {
            format!("no new partition starting at sector {start} was found")
        })
}

pub fn apply(cfg: &InstallConfig) -> Result<TargetParts, String> {
    let disk = find_disk(cfg)?;
    if disk.busy_system {
        return Err(format!(
            "disk {} contains a mounted system — aborted",
            cfg.disk
        ));
    }

    match cfg.strategy {
        Strategy::Whole => apply_whole(cfg, &disk),
        Strategy::Free { start, sectors } => apply_free(cfg, &disk, start, sectors),
    }
}

/// Whole disk: a new GPT table. UEFI: 1 GiB ESP + root.
/// BIOS: a 1 MiB "BIOS boot" partition + root.
fn apply_whole(cfg: &InstallConfig, disk: &disks::DiskInfo) -> Result<TargetParts, String> {
    umount_disk(disk);
    run(&["wipefs", "-a", &cfg.disk])?;

    let script = if cfg.uefi {
        format!(
            "label: gpt\nsize={}, type={}\ntype={}\n",
            ESP_BYTES / SECTOR,
            ESP_GUID,
            LINUX_GUID
        )
    } else {
        format!(
            "label: gpt\nsize={}, type={}\ntype={}\n",
            BIOSBOOT_BYTES / SECTOR,
            BIOSBOOT_GUID,
            LINUX_GUID
        )
    };
    run_stdin(&["sfdisk", &cfg.disk], &script, false)?;
    settle();

    let root = disks::part_dev(&cfg.disk, 2);
    let esp = cfg.uefi.then(|| disks::part_dev(&cfg.disk, 1));
    for dev in esp.iter().chain(std::iter::once(&root)) {
        if !std::path::Path::new(dev).exists() {
            return Err(format!("device {dev} did not appear after partitioning"));
        }
    }
    Ok(TargetParts { root, esp, esp_format: cfg.uefi })
}

/// Free space: appending partitions without touching the existing ones.
fn apply_free(
    cfg: &InstallConfig,
    disk: &disks::DiskInfo,
    start: u64,
    sectors: u64,
) -> Result<TargetParts, String> {
    // Fresh verification: the planned gap must still exist, unmoved.
    let gap = disks::largest_gap(disk)
        .ok_or("the disk has no free space anymore — the layout changed since scanning")?;
    if gap.start != start || gap.sectors < sectors {
        return Err(format!(
            "the disk layout changed since scanning (expected gap {}+{}, found {}+{}) — aborted",
            start, sectors, gap.start, gap.sectors
        ));
    }
    if cfg.pttype.as_deref() == Some("dos") && disk.parts.len() >= 4 {
        return Err("the MBR table already has 4 primary partitions — no free slot".into());
    }

    let is_gpt = cfg.pttype.as_deref() == Some("gpt");
    let mut cursor = start;
    let mut lines: Vec<String> = Vec::new();
    let mut biosboot_start = None;
    let mut esp_start = None;

    if !cfg.uefi && is_gpt && !cfg.biosboot_present {
        lines.push(format!(
            "start={cursor}, size={}, type={}",
            BIOSBOOT_BYTES / SECTOR,
            BIOSBOOT_GUID
        ));
        biosboot_start = Some(cursor);
        cursor += BIOSBOOT_BYTES / SECTOR;
    }
    let need_esp = cfg.uefi && cfg.esp_reuse.is_none();
    if need_esp {
        let t = if is_gpt { ESP_GUID } else { "ef" };
        lines.push(format!("start={cursor}, size={}, type={t}", ESP_BYTES / SECTOR));
        esp_start = Some(cursor);
        cursor += ESP_BYTES / SECTOR;
    }
    let root_sectors = sectors - (cursor - start);
    let root_type = if is_gpt { LINUX_GUID } else { "83" };
    lines.push(format!("start={cursor}, size={root_sectors}, type={root_type}"));
    let root_start = cursor;

    let script = lines.join("\n") + "\n";
    if let Err(e) = run_stdin(&["sfdisk", "--append", &cfg.disk], &script, false) {
        // Mounted neighboring partitions can block the table re-read;
        // in that case append without it and refresh with partprobe.
        log(&format!("sfdisk --append failed ({e}), retrying with --no-reread"));
        run_stdin(&["sfdisk", "--append", "--no-reread", &cfg.disk], &script, false)?;
        run_soft(&["partprobe", &cfg.disk]);
    }
    settle();

    let root = part_by_start(&cfg.disk, root_start)?;
    let esp = if let Some(s) = esp_start {
        Some(part_by_start(&cfg.disk, s)?)
    } else {
        cfg.esp_reuse.clone()
    };
    if let Some(s) = biosboot_start {
        // Existence check only; GRUB finds the BIOS boot partition itself.
        part_by_start(&cfg.disk, s)?;
    }

    // Fresh partitions may contain stale signatures from the days the area
    // belonged to another partition.
    run_soft(&["wipefs", "-a", &root]);
    if esp_start.is_some() {
        if let Some(e) = &esp {
            run_soft(&["wipefs", "-a", e]);
        }
    }

    Ok(TargetParts {
        root,
        esp,
        esp_format: esp_start.is_some(),
    })
}

/// Filesystem UUID (blkid).
pub fn fs_uuid(dev: &str) -> Result<String, String> {
    let out = run_capture(&["blkid", "-s", "UUID", "-o", "value", dev])?;
    let uuid = out.trim().to_string();
    if uuid.is_empty() {
        return Err(format!("blkid returned no UUID for {dev}"));
    }
    Ok(uuid)
}
