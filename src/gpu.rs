// Graphics card detection on the GUI side (no root) from /sys/bus/pci,
// plus a catalog of in-kernel open drivers to choose from per card.
// Proprietary drivers are deliberately out of scope (Chimera's repositories
// do not carry them anyway); for NVIDIA only nouveau + NVK.

use std::fs;

#[derive(Clone)]
pub struct DriverOption {
    /// Kernel module name (empty = configure nothing).
    pub module: &'static str,
    pub label: &'static str,
}

#[derive(Clone)]
pub struct GpuInfo {
    /// PCI address, e.g. 0000:01:00.0.
    pub slot: String,
    /// Hex identifiers without the 0x prefix, e.g. 10de / 2684.
    pub vendor: String,
    pub device: String,
    pub vendor_name: &'static str,
    pub options: Vec<DriverOption>,
}

fn read_hex(path: &std::path::Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    Some(s.trim().trim_start_matches("0x").to_lowercase())
}

fn options_for(vendor: &str) -> (&'static str, Vec<DriverOption>) {
    match vendor {
        "10de" => (
            "NVIDIA",
            vec![DriverOption {
                module: "nouveau",
                label: "nouveau + NVK (open, in-kernel)",
            }],
        ),
        "1002" => (
            "AMD",
            vec![
                DriverOption { module: "amdgpu", label: "amdgpu (open, newer cards)" },
                DriverOption { module: "radeon", label: "radeon (open, older cards)" },
            ],
        ),
        "8086" => (
            "Intel",
            vec![
                DriverOption { module: "i915", label: "i915 (open)" },
                DriverOption { module: "xe", label: "xe (open, latest generations)" },
            ],
        ),
        "15ad" => (
            "VMware",
            vec![DriverOption { module: "vmwgfx", label: "vmwgfx (open, in-kernel)" }],
        ),
        "1af4" => (
            "VirtIO",
            vec![DriverOption { module: "virtio-gpu", label: "virtio-gpu (open, in-kernel)" }],
        ),
        "1234" | "1b36" => (
            "QEMU",
            vec![DriverOption { module: "bochs", label: "bochs (open, in-kernel)" }],
        ),
        _ => (
            "Unknown vendor",
            vec![DriverOption { module: "", label: "generic in-kernel KMS (no changes)" }],
        ),
    }
}

/// Graphics cards: PCI devices of class 0x03xxxx.
pub fn scan() -> Vec<GpuInfo> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") else {
        return found;
    };
    for e in entries.flatten() {
        let p = e.path();
        let Some(class) = read_hex(&p.join("class")) else { continue };
        if !class.starts_with("03") {
            continue;
        }
        let Some(vendor) = read_hex(&p.join("vendor")) else { continue };
        let device = read_hex(&p.join("device")).unwrap_or_default();
        let (vendor_name, options) = options_for(&vendor);
        found.push(GpuInfo {
            slot: e.file_name().to_string_lossy().to_string(),
            vendor,
            device,
            vendor_name,
            options,
        });
    }
    found.sort_by(|a, b| a.slot.cmp(&b.slot));
    found
}
