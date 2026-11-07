// Wykrywanie kart graficznych po stronie GUI (bez roota) z /sys/bus/pci
// oraz katalog otwartych sterowników z jądra do wyboru per karta.
// Zamknięte sterowniki świadomie poza zakresem (w repozytoriach Chimery
// i tak ich nie ma); dla NVIDII wyłącznie nouveau + NVK.

use std::fs;

#[derive(Clone)]
pub struct DriverOption {
    /// Nazwa modułu jądra (pusty = nic nie konfigurujemy).
    pub module: &'static str,
    pub label: &'static str,
}

#[derive(Clone)]
pub struct GpuInfo {
    /// Adres PCI, np. 0000:01:00.0.
    pub slot: String,
    /// Identyfikatory hex bez prefiksu 0x, np. 10de / 2684.
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
                label: "nouveau + NVK (otwarty, z jądra)",
            }],
        ),
        "1002" => (
            "AMD",
            vec![
                DriverOption { module: "amdgpu", label: "amdgpu (otwarty, nowsze karty)" },
                DriverOption { module: "radeon", label: "radeon (otwarty, starsze karty)" },
            ],
        ),
        "8086" => (
            "Intel",
            vec![
                DriverOption { module: "i915", label: "i915 (otwarty)" },
                DriverOption { module: "xe", label: "xe (otwarty, najnowsze generacje)" },
            ],
        ),
        "15ad" => (
            "VMware",
            vec![DriverOption { module: "vmwgfx", label: "vmwgfx (otwarty, z jądra)" }],
        ),
        "1af4" => (
            "VirtIO",
            vec![DriverOption { module: "virtio-gpu", label: "virtio-gpu (otwarty, z jądra)" }],
        ),
        "1234" | "1b36" => (
            "QEMU",
            vec![DriverOption { module: "bochs", label: "bochs (otwarty, z jądra)" }],
        ),
        _ => (
            "Nieznany producent",
            vec![DriverOption { module: "", label: "ogólny KMS z jądra (bez zmian)" }],
        ),
    }
}

/// Karty graficzne: urządzenia PCI o klasie 0x03xxxx.
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
