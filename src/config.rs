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

/// Źródło pakietów przy instalacji z nośnika live.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Repozytorium lokalne nośnika (chimera-bootstrap -l); tylko base-full.
    Local,
    /// Repozytorium sieciowe; umożliwia wybór wariantu systemu.
    Network,
}

/// Wariant instalowanego systemu (tylko źródło sieciowe).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Desktop {
    /// Sam system bazowy (konsola).
    Base,
    Gnome,
    Kde,
}

/// Wariant środowiska graficznego (tylko dla GNOME/KDE).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edition {
    /// Bez zbędnych aplikacji, z samym terminalem.
    Base,
    /// Pełne środowisko ze wszystkimi aplikacjami.
    Full,
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

fn default_edition() -> Edition {
    Edition::Full
}

/// Wybrany sterownik dla wykrytej karty graficznej.
#[derive(Clone, Serialize, Deserialize)]
pub struct GpuChoice {
    /// Identyfikatory PCI hex, np. 10de / 2684 (do opisu w logu).
    pub vendor: String,
    pub device: String,
    /// Nazwa modułu jądra ("" = nic nie konfigurujemy).
    pub driver: String,
}

/// Podpakiety firmware-linux wymagane przez dany sterownik.
pub fn driver_firmware(driver: &str) -> &'static [&'static str] {
    match driver {
        // nouveau na kartach Turing+ wymaga blobów GSP.
        "nouveau" => &["firmware-linux-nvidia", "firmware-linux-nvidia-gsp"],
        "amdgpu" => &["firmware-linux-amdgpu"],
        "radeon" => &["firmware-linux-radeon"],
        "i915" => &["firmware-linux-i915"],
        // xe korzysta też z plików z katalogu i915 (podział w firmware-linux).
        "xe" => &["firmware-linux-xe", "firmware-linux-i915"],
        _ => &[],
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    pub mode: Mode,
    /// W trybie z innego systemu zawsze Network.
    pub source: Source,
    pub desktop: Desktop,
    /// Wariant pulpitu (ignorowany dla Desktop::Base).
    #[serde(default = "default_edition")]
    pub edition: Edition,
    /// Sterowniki GPU wybrane na stronie wykrywania.
    #[serde(default)]
    pub gpus: Vec<GpuChoice>,
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
    /// Język systemu, np. "pl_PL.UTF-8"; zapisywany jako LANG w /etc/locale.conf.
    #[serde(default = "default_locale")]
    pub locale: String,
    /// Kod układu klawiatury xkb, np. "pl".
    #[serde(default = "default_kb_layout")]
    pub kb_layout: String,
    /// Wariant układu xkb (pusty = domyślny).
    #[serde(default)]
    pub kb_variant: String,
    /// Czy włączyć serwer SSH (sshd) — domyślnie wyłączony.
    #[serde(default)]
    pub enable_ssh: bool,
}

fn default_locale() -> String {
    "en_US.UTF-8".to_string()
}

fn default_kb_layout() -> String {
    "us".to_string()
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
