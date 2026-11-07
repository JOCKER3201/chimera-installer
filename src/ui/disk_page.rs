// Strona wyboru dysku. Proste partycjonowanie:
//  - pusty dysk        -> instalacja na całym dysku,
//  - jest wolne miejsce -> pytanie, czy go użyć (lub wymazać cały dysk),
//  - brak miejsca       -> pytanie o tryb zaawansowany (na razie atrapa).

use super::{advanced_mode_stub, user_page, wizard_page, App, DiskChoice};
use crate::config::{format_gib, Strategy, ESP_BYTES, MIN_ROOT_BYTES};
use crate::disks::{self, DiskInfo};
use adw::prelude::*;
use std::rc::Rc;

fn required_free_bytes(disk: &DiskInfo) -> u64 {
    // Root + miejsce na ESP (gdy UEFI bez istniejącej ESP) + ew. BIOS boot.
    let mut need = MIN_ROOT_BYTES;
    if disks::host_is_uefi() && disks::find_esp(disk).is_none() {
        need += ESP_BYTES;
    }
    need + 2 * 1024 * 1024
}

fn accept(app: &Rc<App>, disk: &DiskInfo, strategy: Strategy) {
    app.draft.borrow_mut().choice = Some(DiskChoice {
        disk: disk.clone(),
        strategy,
        esp_reuse: match strategy {
            // Przy wymazywaniu całego dysku ESP zawsze powstaje od nowa.
            Strategy::Whole => None,
            Strategy::Free { .. } => disks::find_esp(disk),
        },
        biosboot_present: matches!(strategy, Strategy::Free { .. }) && disks::has_biosboot(disk),
    });
    app.nav.push(&user_page::build(app));
}

/// Potwierdzenie wymazania całego, niepustego dysku.
fn confirm_wipe(app: &Rc<App>, disk: &DiskInfo) {
    let dialog = adw::AlertDialog::new(
        Some("Wymazać cały dysk?"),
        Some(&format!(
            "Wszystkie dane na {} ({}) zostaną bezpowrotnie usunięte.",
            disk.path,
            format_gib(disk.size)
        )),
    );
    dialog.add_responses(&[("cancel", "Anuluj"), ("wipe", "Wymaż cały dysk")]);
    dialog.set_response_appearance("wipe", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let app2 = app.clone();
    let disk = disk.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp == "wipe" {
            accept(&app2, &disk, Strategy::Whole);
        }
    });
    dialog.present(Some(&app.win));
}

fn on_disk_activated(app: &Rc<App>, disk: &DiskInfo) {
    if disk.busy_system {
        app.toast("Ten dysk zawiera uruchomiony system — nie można na nim instalować.");
        return;
    }

    // Pusty dysk: bez pytań, cały dysk (nie ma czego stracić).
    if disks::is_empty(disk) {
        accept(app, disk, Strategy::Whole);
        return;
    }

    let need = required_free_bytes(disk);
    let gap = disks::largest_gap(disk).filter(|g| g.bytes() >= need);

    match gap {
        Some(g) => {
            // Jest wolne miejsce — pytamy, czy go użyć.
            let dialog = adw::AlertDialog::new(
                Some("Wykryto wolne miejsce"),
                Some(&format!(
                    "Na dysku {} znajduje się {} nieprzydzielonego miejsca. \
                     Chimera Linux może zostać zainstalowana w tym obszarze bez \
                     naruszania istniejących partycji.",
                    disk.path,
                    format_gib(g.bytes())
                )),
            );
            dialog.add_responses(&[
                ("cancel", "Anuluj"),
                ("wipe", "Wymaż cały dysk"),
                ("free", "Użyj wolnego miejsca"),
            ]);
            dialog.set_response_appearance("free", adw::ResponseAppearance::Suggested);
            dialog.set_response_appearance("wipe", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("free"));
            dialog.set_close_response("cancel");
            let app2 = app.clone();
            let disk = disk.clone();
            dialog.connect_response(None, move |_, resp| match resp {
                "free" => accept(
                    &app2,
                    &disk,
                    Strategy::Free {
                        start: g.start,
                        sectors: g.sectors,
                    },
                ),
                "wipe" => confirm_wipe(&app2, &disk),
                _ => {}
            });
            dialog.present(Some(&app.win));
        }
        None => {
            // Brak miejsca — pytanie o tryb zaawansowany (atrapa).
            let dialog = adw::AlertDialog::new(
                Some("Brak wolnego miejsca"),
                Some(&format!(
                    "Dysk {} nie ma wystarczającego nieprzydzielonego miejsca \
                     (potrzeba co najmniej {}). Czy chcesz przejść do trybu \
                     zaawansowanego, aby ręcznie zmienić układ partycji?",
                    disk.path,
                    format_gib(need)
                )),
            );
            dialog.add_responses(&[
                ("cancel", "Anuluj"),
                ("wipe", "Wymaż cały dysk"),
                ("advanced", "Tryb zaawansowany…"),
            ]);
            dialog.set_response_appearance("wipe", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("advanced"));
            dialog.set_close_response("cancel");
            let app2 = app.clone();
            let disk = disk.clone();
            dialog.connect_response(None, move |_, resp| match resp {
                "advanced" => advanced_mode_stub(&app2),
                "wipe" => confirm_wipe(&app2, &disk),
                _ => {}
            });
            dialog.present(Some(&app.win));
        }
    }
}

fn disk_subtitle(disk: &DiskInfo) -> String {
    let mut bits: Vec<String> = Vec::new();
    bits.push(format_gib(disk.size));
    if let Some(t) = &disk.tran {
        bits.push(t.to_uppercase());
    }
    if disk.busy_system {
        bits.push("zawiera uruchomiony system".into());
    } else if disks::is_empty(disk) {
        bits.push("pusty dysk".into());
    } else {
        bits.push(format!("partycje: {}", disk.parts.len()));
        match disks::largest_gap(disk) {
            Some(g) if g.bytes() >= required_free_bytes(disk) => {
                bits.push(format!("wolne miejsce: {}", format_gib(g.bytes())));
            }
            _ => bits.push("brak wolnego miejsca".into()),
        }
    }
    bits.join(" · ")
}

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    match disks::scan() {
        Ok(found) if !found.is_empty() => {
            for disk in found {
                let title = if disk.model.is_empty() {
                    disk.path.clone()
                } else {
                    format!("{} — {}", disk.path, disk.model)
                };
                let row = adw::ActionRow::builder()
                    .title(title)
                    .subtitle(disk_subtitle(&disk))
                    .activatable(!disk.busy_system)
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name("drive-harddisk-symbolic"));
                if disk.busy_system {
                    row.add_suffix(&gtk::Image::from_icon_name("action-unavailable-symbolic"));
                    row.add_css_class("dim-label");
                } else {
                    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                }
                let app = app.clone();
                row.connect_activated(move |_| on_disk_activated(&app, &disk));
                list.append(&row);
            }
        }
        Ok(_) => {
            list.append(
                &adw::ActionRow::builder()
                    .title("Nie znaleziono żadnych dysków")
                    .build(),
            );
        }
        Err(e) => {
            list.append(
                &adw::ActionRow::builder()
                    .title("Błąd skanowania dysków")
                    .subtitle(e)
                    .build(),
            );
        }
    }

    let status = adw::StatusPage::builder()
        .icon_name("drive-harddisk-system-symbolic")
        .title("Wybór dysku")
        .description(&format!(
            "Wybierz dysk, na którym zostanie zainstalowana Chimera Linux \
             (btrfs z migawkami snappera). Tryb rozruchu: {}.",
            if disks::host_is_uefi() { "UEFI" } else { "BIOS" }
        ))
        .child(
            &adw::Clamp::builder()
                .maximum_size(560)
                .child(&list)
                .build(),
        )
        .build();

    wizard_page("Dysk docelowy", "disk", &status)
}
