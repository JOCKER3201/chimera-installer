// Shared installation configuration passed from the GUI to the root helper
// (serialized as JSON over the pkexec process's stdin).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Installation from a running Chimera live medium (chimera-bootstrap -l).
    Live,
    /// Installation from another system: download the bootstrap image,
    /// unpack it to /tmp/chimera-linux and install via chroot.
    Foreign,
}

/// Package source for an installation from a live medium.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The medium's local repository (chimera-bootstrap -l); base-full only.
    Local,
    /// The network repository; allows choosing the system variant.
    Network,
}

/// Variant of the installed system (network source only).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Desktop {
    /// The base system alone (console).
    Base,
    Gnome,
    Kde,
}

/// Desktop environment edition (GNOME/KDE only).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edition {
    /// No extra applications, terminal only.
    Base,
    /// The full environment with all applications.
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Strategy {
    /// The whole disk is wiped and partitioned from scratch.
    Whole,
    /// Installation into an unallocated area; existing partitions stay.
    /// Values in 512-byte sectors.
    Free { start: u64, sectors: u64 },
}

fn default_edition() -> Edition {
    Edition::Full
}

/// Driver chosen for a detected graphics card.
#[derive(Clone, Serialize, Deserialize)]
pub struct GpuChoice {
    /// Hex PCI identifiers, e.g. 10de / 2684 (for log descriptions).
    pub vendor: String,
    pub device: String,
    /// Kernel module name ("" = configure nothing).
    pub driver: String,
}

/// firmware-linux subpackages required by a given driver.
pub fn driver_firmware(driver: &str) -> &'static [&'static str] {
    match driver {
        // nouveau on Turing+ cards requires the GSP blobs.
        "nouveau" => &["firmware-linux-nvidia", "firmware-linux-nvidia-gsp"],
        "amdgpu" => &["firmware-linux-amdgpu"],
        "radeon" => &["firmware-linux-radeon"],
        "i915" => &["firmware-linux-i915"],
        // xe also uses files from the i915 directory (firmware-linux split).
        "xe" => &["firmware-linux-xe", "firmware-linux-i915"],
        _ => &[],
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    pub mode: Mode,
    /// Always Network in the from-another-system mode.
    pub source: Source,
    pub desktop: Desktop,
    /// Desktop edition (ignored for Desktop::Base).
    #[serde(default = "default_edition")]
    pub edition: Edition,
    /// GPU drivers chosen on the detection page.
    #[serde(default)]
    pub gpus: Vec<GpuChoice>,
    /// Target disk path, e.g. /dev/sda or /dev/nvme0n1.
    pub disk: String,
    pub strategy: Strategy,
    /// Whether the host booted in UEFI mode (decides the GRUB variant).
    pub uefi: bool,
    /// Existing partition table type ("gpt"/"dos"), or None for an empty disk.
    pub pttype: Option<String>,
    /// Existing ESP partition to reuse (Free strategy only).
    pub esp_reuse: Option<String>,
    /// Whether a "BIOS boot" partition already exists on the GPT disk (Free strategy only).
    pub biosboot_present: bool,
    pub hostname: String,
    pub username: String,
    pub user_password: String,
    /// None = the root account stays locked (administration via doas/wheel).
    pub root_password: Option<String>,
    /// E.g. "Europe/Warsaw"; the /etc/localtime symlink in the target system.
    pub timezone: String,
    /// System language, e.g. "pl_PL.UTF-8"; written as LANG to /etc/locale.conf.
    #[serde(default = "default_locale")]
    pub locale: String,
    /// xkb keyboard layout code, e.g. "pl".
    #[serde(default = "default_kb_layout")]
    pub kb_layout: String,
    /// xkb layout variant (empty = default).
    #[serde(default)]
    pub kb_variant: String,
    /// Whether to enable the SSH server (sshd) — disabled by default.
    #[serde(default)]
    pub enable_ssh: bool,
}

fn default_locale() -> String {
    "en_US.UTF-8".to_string()
}

fn default_kb_layout() -> String {
    "us".to_string()
}

/// Minimum root partition size assumed by the installer's simple mode.
pub const MIN_ROOT_BYTES: u64 = 15 * 1024 * 1024 * 1024;
/// Size of a newly created ESP (Chimera docs recommendation: 1 GiB).
pub const ESP_BYTES: u64 = 1024 * 1024 * 1024;
/// Size of the "BIOS boot" partition for GRUB on GPT without UEFI.
pub const BIOSBOOT_BYTES: u64 = 1024 * 1024;

pub const SECTOR: u64 = 512;

/// ESP partition GUID on GPT / type 0xef on MBR.
pub const ESP_GUID: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
/// "BIOS boot" partition GUID on GPT.
pub const BIOSBOOT_GUID: &str = "21686148-6449-6e6f-744e-656564454649";

pub fn format_gib(bytes: u64) -> String {
    let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{:.1} GiB", gib)
}
