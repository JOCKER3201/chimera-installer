// Partycjonowanie w helperze roota: weryfikacja planu z GUI na świeżym
// skanie dysku, budowa skryptu sfdisk i odnalezienie nowych partycji.

use super::cmd::{log, run, run_capture, run_soft, run_stdin};
use crate::config::{
    InstallConfig, Strategy, BIOSBOOT_BYTES, BIOSBOOT_GUID, ESP_BYTES, ESP_GUID, SECTOR,
};
use crate::disks;

/// Wynik partycjonowania: urządzenia, na których stanie system.
pub struct TargetParts {
    pub root: String,
    /// ESP do zamontowania pod /boot/efi (istniejąca albo świeżo utworzona).
    pub esp: Option<String>,
    /// Czy ESP wymaga mkfs.vfat (tylko świeżo utworzona).
    pub esp_format: bool,
}

const LINUX_GUID: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";

fn find_disk(cfg: &InstallConfig) -> Result<disks::DiskInfo, String> {
    let all = disks::scan()?;
    all.into_iter()
        .find(|d| d.path == cfg.disk)
        .ok_or_else(|| format!("dysk {} nie istnieje", cfg.disk))
}

/// Odmontowuje wszystko, co jest zamontowane z partycji danego dysku
/// (tylko przy wymazywaniu całego dysku).
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

/// Po sfdisk odnajduje partycję po sektorze początkowym.
fn part_by_start(disk_path: &str, start: u64) -> Result<String, String> {
    let all = disks::scan()?;
    let disk = all
        .iter()
        .find(|d| d.path == disk_path)
        .ok_or_else(|| format!("dysk {disk_path} zniknął po partycjonowaniu"))?;
    disk.parts
        .iter()
        .find(|p| p.start == start)
        .map(|p| p.path.clone())
        .ok_or_else(|| {
            format!("nie znaleziono nowej partycji zaczynającej się w sektorze {start}")
        })
}

pub fn apply(cfg: &InstallConfig) -> Result<TargetParts, String> {
    let disk = find_disk(cfg)?;
    if disk.busy_system {
        return Err(format!(
            "dysk {} zawiera zamontowany system — przerwano",
            cfg.disk
        ));
    }

    match cfg.strategy {
        Strategy::Whole => apply_whole(cfg, &disk),
        Strategy::Free { start, sectors } => apply_free(cfg, &disk, start, sectors),
    }
}

/// Cały dysk: nowa tablica GPT. UEFI: ESP 1 GiB + root.
/// BIOS: partycja "BIOS boot" 1 MiB + root.
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
            return Err(format!("urządzenie {dev} nie pojawiło się po partycjonowaniu"));
        }
    }
    Ok(TargetParts { root, esp, esp_format: cfg.uefi })
}

/// Wolne miejsce: dopisanie partycji bez naruszania istniejących.
fn apply_free(
    cfg: &InstallConfig,
    disk: &disks::DiskInfo,
    start: u64,
    sectors: u64,
) -> Result<TargetParts, String> {
    // Świeża weryfikacja: luka z planu musi nadal istnieć w niezmienionym miejscu.
    let gap = disks::largest_gap(disk)
        .ok_or("na dysku nie ma już wolnego miejsca — układ zmienił się od skanowania")?;
    if gap.start != start || gap.sectors < sectors {
        return Err(format!(
            "układ dysku zmienił się od skanowania (oczekiwano luki {}+{}, jest {}+{}) — przerwano",
            start, sectors, gap.start, gap.sectors
        ));
    }
    if cfg.pttype.as_deref() == Some("dos") && disk.parts.len() >= 4 {
        return Err("tablica MBR ma już 4 partycje główne — brak wolnego slotu".into());
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
        // Zamontowane sąsiednie partycje potrafią zablokować ponowny odczyt
        // tablicy; wtedy dopisujemy bez niego i odświeżamy partprobe.
        log(&format!("sfdisk --append nie powiódł się ({e}), ponawiam z --no-reread"));
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
        // Tylko kontrola istnienia; GRUB sam znajdzie partycję BIOS boot.
        part_by_start(&cfg.disk, s)?;
    }

    // Świeże partycje mogą zawierać stare sygnatury z czasów, gdy obszar
    // należał do innej partycji.
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

/// UUID systemu plików (blkid).
pub fn fs_uuid(dev: &str) -> Result<String, String> {
    let out = run_capture(&["blkid", "-s", "UUID", "-o", "value", dev])?;
    let uuid = out.trim().to_string();
    if uuid.is_empty() {
        return Err(format!("blkid nie zwrócił UUID dla {dev}"));
    }
    Ok(uuid)
}
