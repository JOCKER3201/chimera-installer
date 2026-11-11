// Disk scanning on the GUI side (no root) via `lsblk -J`, plus computing
// the largest unallocated gap from sysfs data.
// The root helper uses the same functions to re-verify the plan.

use crate::config::{BIOSBOOT_GUID, ESP_GUID, SECTOR};
use std::process::Command;

#[derive(Clone, Debug)]
pub struct PartInfo {
    pub path: String,
    /// Partition start in 512 B sectors.
    pub start: u64,
    /// Partition size in bytes.
    pub size: u64,
    pub parttype: Option<String>,
    pub fstype: Option<String>,
    pub mountpoints: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DiskInfo {
    pub path: String,
    pub model: String,
    pub tran: Option<String>,
    /// Disk size in bytes.
    pub size: u64,
    pub pttype: Option<String>,
    pub parts: Vec<PartInfo>,
    /// Whether any partition is mounted as part of the running system.
    pub busy_system: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Gap {
    /// Gap start in 512 B sectors (aligned to 2048).
    pub start: u64,
    /// Gap length in 512 B sectors.
    pub sectors: u64,
}

impl Gap {
    pub fn bytes(&self) -> u64 {
        self.sectors * SECTOR
    }
}

const SYSTEM_MOUNTS: &[&str] = &[
    "/", "/sysroot", "/boot", "/boot/efi", "/home", "/var", "/usr", "/etc", "/nix",
];

/// Prefixes also catch nested mounts (e.g. /var/home on Bazzite,
/// /usr/share/... from overlays) that no exact list would enumerate.
const SYSTEM_PREFIXES: &[&str] = &[
    "/sysroot/", "/boot/", "/home/", "/var/", "/usr/", "/etc/", "/nix/",
    "/run/initramfs", "/run/live", "/run/rootfsbase",
];

fn is_system_mount(mp: &str) -> bool {
    SYSTEM_MOUNTS.contains(&mp) || SYSTEM_PREFIXES.iter().any(|p| mp.starts_with(p))
}

fn s_opt(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(|s| s.to_string())
}

fn u_opt(v: &serde_json::Value, key: &str) -> Option<u64> {
    match v.get(key) {
        Some(serde_json::Value::Number(n)) => n.as_u64(),
        Some(serde_json::Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

pub fn scan() -> Result<Vec<DiskInfo>, String> {
    let out = Command::new("lsblk")
        .args([
            "-b",
            "-J",
            "-o",
            "PATH,NAME,TYPE,SIZE,START,PTTYPE,PARTTYPE,FSTYPE,MODEL,TRAN,RM,RO,MOUNTPOINTS",
        ])
        .output()
        .map_err(|e| format!("cannot run lsblk: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "lsblk exited with an error: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("invalid JSON from lsblk: {e}"))?;
    let devices = json
        .get("blockdevices")
        .and_then(|d| d.as_array())
        .ok_or("no blockdevices key in lsblk output")?;

    let mut disks = Vec::new();
    for dev in devices {
        if s_opt(dev, "type").as_deref() != Some("disk") {
            continue;
        }
        if dev.get("ro").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        let size = u_opt(dev, "size").unwrap_or(0);
        if size == 0 {
            continue;
        }
        let path = match s_opt(dev, "path") {
            Some(p) => p,
            None => continue,
        };
        // Skip virtual devices that lsblk reports as "disk".
        if path.starts_with("/dev/zram") || path.starts_with("/dev/loop") {
            continue;
        }

        let mut parts = Vec::new();
        let mut busy = false;
        if let Some(children) = dev.get("children").and_then(|c| c.as_array()) {
            for ch in children {
                if s_opt(ch, "type").as_deref() != Some("part") {
                    continue;
                }
                let mountpoints: Vec<String> = ch
                    .get("mountpoints")
                    .and_then(|m| m.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                for mp in &mountpoints {
                    if is_system_mount(mp) {
                        busy = true;
                    }
                }
                parts.push(PartInfo {
                    path: s_opt(ch, "path").unwrap_or_default(),
                    start: u_opt(ch, "start").unwrap_or(0),
                    size: u_opt(ch, "size").unwrap_or(0),
                    parttype: s_opt(ch, "parttype").map(|s| s.to_lowercase()),
                    fstype: s_opt(ch, "fstype"),
                    mountpoints,
                });
            }
        }
        parts.sort_by_key(|p| p.start);
        disks.push(DiskInfo {
            path,
            model: s_opt(dev, "model").unwrap_or_default().trim().to_string(),
            tran: s_opt(dev, "tran"),
            size,
            pttype: s_opt(dev, "pttype"),
            parts,
            busy_system: busy,
        });
    }
    Ok(disks)
}

fn align_up(sector: u64, align: u64) -> u64 {
    sector.div_ceil(align) * align
}

/// The largest contiguous unallocated area on the disk.
/// Accounts for 1 MiB alignment and the backup GPT header at the disk's end.
pub fn largest_gap(disk: &DiskInfo) -> Option<Gap> {
    let disk_sectors = disk.size / SECTOR;
    // A 2048-sector reserve at the end covers the backup GPT table (33 sectors).
    let usable_end = disk_sectors.saturating_sub(2048);
    let mut cursor = 2048u64;
    let mut best: Option<Gap> = None;
    let mut consider = |start: u64, end: u64| {
        let start = align_up(start, 2048);
        if end > start {
            let g = Gap { start, sectors: end - start };
            if best.map_or(true, |b| g.sectors > b.sectors) {
                best = Some(g);
            }
        }
    };
    for p in &disk.parts {
        let p_start = p.start;
        let p_end = p.start + p.size / SECTOR;
        if p_start > cursor {
            consider(cursor, p_start.min(usable_end));
        }
        cursor = cursor.max(p_end);
    }
    consider(cursor, usable_end);
    best
}

/// Existing ESP partition (GPT GUID or 0xef on MBR) with a VFAT filesystem.
pub fn find_esp(disk: &DiskInfo) -> Option<String> {
    disk.parts
        .iter()
        .find(|p| {
            matches!(p.parttype.as_deref(), Some(t) if t == ESP_GUID || t == "0xef")
                && matches!(p.fstype.as_deref(), Some("vfat") | None)
        })
        .map(|p| p.path.clone())
}

pub fn has_biosboot(disk: &DiskInfo) -> bool {
    disk.parts
        .iter()
        .any(|p| p.parttype.as_deref() == Some(BIOSBOOT_GUID))
}

/// Whether the disk is empty from the simple installation mode's point of
/// view (no partition table, or a table with no partitions at all).
pub fn is_empty(disk: &DiskInfo) -> bool {
    disk.parts.is_empty()
}

pub fn host_is_uefi() -> bool {
    std::path::Path::new("/sys/firmware/efi").exists()
}

/// Device name of the disk's n-th partition (sda -> sda3, nvme0n1 -> nvme0n1p3).
pub fn part_dev(disk: &str, n: u32) -> String {
    if disk.chars().last().is_some_and(|c| c.is_ascii_digit()) {
        format!("{disk}p{n}")
    } else {
        format!("{disk}{n}")
    }
}
