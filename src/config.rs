// Wspólna konfiguracja instalacji przekazywana z GUI do helpera roota
// (serializowana jako JSON przez stdin procesu pkexec).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Instalacja z uruchomionego nośnika live Chimery (chimera-bootstrap -l).
    Live,
    /// Instalacja z innego systemu: pobranie obrazu bootstrap, rozpakowanie
    /// do /tmp/chimera-linux i instalacja przez chroot.
    Foreign,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Strategy {
    /// Cały dysk zostaje wymazany i podzielony od nowa.
    Whole,
    /// Instalacja w nieprzydzielonym obszarze; istniejące partycje zostają.
    /// Wartości w sektorach 512-bajtowych.
    Free { start: u64, sectors: u64 },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    pub mode: Mode,
    /// Ścieżka dysku docelowego, np. /dev/sda albo /dev/nvme0n1.
    pub disk: String,
    pub strategy: Strategy,
    /// Czy host wystartował w trybie UEFI (decyduje o wariancie GRUB-a).
    pub uefi: bool,
    /// Typ istniejącej tablicy partycji ("gpt"/"dos") albo None dla pustego dysku.
    pub pttype: Option<String>,
    /// Istniejąca partycja ESP do ponownego użycia (tylko strategia Free).
    pub esp_reuse: Option<String>,
    /// Czy na dysku GPT istnieje już partycja "BIOS boot" (tylko strategia Free).
    pub biosboot_present: bool,
    pub hostname: String,
    pub username: String,
    pub user_password: String,
    /// None = konto roota pozostaje zablokowane (administracja przez doas/wheel).
    pub root_password: Option<String>,
    /// Np. "Europe/Warsaw"; dowiązanie /etc/localtime w systemie docelowym.
    pub timezone: String,
}

/// Minimalny rozmiar partycji root przyjmowany przez prosty tryb instalatora.
pub const MIN_ROOT_BYTES: u64 = 15 * 1024 * 1024 * 1024;
/// Rozmiar nowo tworzonej partycji ESP (zalecenie z dokumentacji Chimery: 1 GiB).
pub const ESP_BYTES: u64 = 1024 * 1024 * 1024;
/// Rozmiar partycji "BIOS boot" dla GRUB-a na GPT bez UEFI.
pub const BIOSBOOT_BYTES: u64 = 1024 * 1024;

pub const SECTOR: u64 = 512;

/// GUID partycji ESP na GPT / typ 0xef na MBR.
pub const ESP_GUID: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
/// GUID partycji "BIOS boot" na GPT.
pub const BIOSBOOT_GUID: &str = "21686148-6449-6e6f-744e-656564454649";

pub fn format_gib(bytes: u64) -> String {
    let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{:.1} GiB", gib).replace('.', ",")
}
