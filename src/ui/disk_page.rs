// The disk choice page. Simple partitioning:
//  - empty disk          -> installation on the whole disk,
//  - free space present  -> ask whether to use it (or wipe the whole disk),
//  - no space            -> ask about advanced mode (a stub for now).

use super::{advanced_mode_stub, system_page, wizard_page, App, DiskChoice};
use crate::config::{format_gib, Strategy, ESP_BYTES, MIN_ROOT_BYTES};
use crate::disks::{self, DiskInfo};
use adw::prelude::*;
use std::rc::Rc;

fn required_free_bytes(disk: &DiskInfo) -> u64 {
    // Root + room for the ESP (UEFI without an existing ESP) + possibly BIOS boot.
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
            // When wiping the whole disk the ESP is always created anew.
            Strategy::Whole => None,
            Strategy::Free { .. } => disks::find_esp(disk),
        },
        biosboot_present: matches!(strategy, Strategy::Free { .. }) && disks::has_biosboot(disk),
    });
    app.nav.push(&system_page::build(app));
}

/// Confirmation for wiping a whole, non-empty disk.
fn confirm_wipe(app: &Rc<App>, disk: &DiskInfo) {
    let dialog = adw::AlertDialog::new(
        Some("Wipe the whole disk?"),
        Some(&format!(
            "All data on {} ({}) will be irreversibly erased.",
            disk.path,
            format_gib(disk.size)
        )),
    );
    dialog.add_responses(&[("cancel", "Cancel"), ("wipe", "Wipe the whole disk")]);
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
        app.toast("This disk contains a running system — cannot install onto it.");
        return;
    }

    // Empty disk: no questions, whole disk (nothing to lose).
    if disks::is_empty(disk) {
        accept(app, disk, Strategy::Whole);
        return;
    }

    let need = required_free_bytes(disk);
    let gap = disks::largest_gap(disk).filter(|g| g.bytes() >= need);

    match gap {
        Some(g) => {
            // There is free space — ask whether to use it.
            let dialog = adw::AlertDialog::new(
                Some("Free space detected"),
                Some(&format!(
                    "Disk {} has {} of unallocated space. \
                     Chimera Linux can be installed into that area without \
                     touching the existing partitions.",
                    disk.path,
                    format_gib(g.bytes())
                )),
            );
            dialog.add_responses(&[
                ("cancel", "Cancel"),
                ("wipe", "Wipe the whole disk"),
                ("free", "Use the free space"),
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
            // No space — ask about advanced mode (a stub).
            let dialog = adw::AlertDialog::new(
                Some("No free space"),
                Some(&format!(
                    "Disk {} does not have enough unallocated space \
                     (at least {} is needed). Do you want to switch to \
                     advanced mode to adjust the partition layout manually?",
                    disk.path,
                    format_gib(need)
                )),
            );
            dialog.add_responses(&[
                ("cancel", "Cancel"),
                ("wipe", "Wipe the whole disk"),
                ("advanced", "Advanced mode…"),
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
        bits.push("contains a running system".into());
    } else if disks::is_empty(disk) {
        bits.push("empty disk".into());
    } else {
        bits.push(format!("partitions: {}", disk.parts.len()));
        match disks::largest_gap(disk) {
            Some(g) if g.bytes() >= required_free_bytes(disk) => {
                bits.push(format!("free space: {}", format_gib(g.bytes())));
            }
            _ => bits.push("no free space".into()),
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
                    .title("No disks found")
                    .build(),
            );
        }
        Err(e) => {
            list.append(
                &adw::ActionRow::builder()
                    .title("Disk scan error")
                    .subtitle(e)
                    .build(),
            );
        }
    }

    let status = adw::StatusPage::builder()
        .icon_name("drive-harddisk-system-symbolic")
        .title("Disk choice")
        .description(&format!(
            "Choose the disk Chimera Linux will be installed on \
             (btrfs with snapper snapshots). Boot mode: {}.",
            if disks::host_is_uefi() { "UEFI" } else { "BIOS" }
        ))
        .child(
            &adw::Clamp::builder()
                .maximum_size(560)
                .child(&list)
                .build(),
        )
        .build();

    wizard_page("Target disk", "disk", &status)
}
