// The GPU detection and open in-kernel driver choice page —
// between the mode choice and the package source / system variant.

use super::{desktop_page, source_page, wizard_page, App};
use crate::config::{GpuChoice, Mode, Source};
use crate::gpu;
use adw::prelude::*;
use std::rc::Rc;


/// Experimental proprietary NVIDIA drivers built from the cports PR
/// (#6531) — available only in live mode (built with cbuild on the running
/// medium), requires network regardless of the package source.
const NVIDIA_EXPERIMENTAL: &[gpu::DriverOption] = &[
    gpu::DriverOption {
        module: "nvidia-620",
        label: "Proprietary NVIDIA 620 — Turing and newer (EXPERIMENTAL)",
    },
    gpu::DriverOption {
        module: "nvidia-580",
        label: "Proprietary NVIDIA 580 — Maxwell/Pascal (EXPERIMENTAL)",
    },
];

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    let live = app.draft.borrow().mode == Some(Mode::Live);
    let gpus = gpu::scan();
    // The card + its full option list + the row, read when moving on.
    let mut rows: Vec<(gpu::GpuInfo, Vec<gpu::DriverOption>, adw::ComboRow)> = Vec::new();

    if gpus.is_empty() {
        list.append(
            &adw::ActionRow::builder()
                .title("No graphics cards detected")
                .subtitle("The installation will continue without driver configuration.")
                .build(),
        );
    }
    for info in gpus {
        let mut options = info.options.clone();
        if live && info.vendor == "10de" {
            options.extend_from_slice(NVIDIA_EXPERIMENTAL);
        }
        let labels: Vec<&str> = options.iter().map(|o| o.label).collect();
        let row = adw::ComboRow::builder()
            .title(format!("{} — {}:{}", info.vendor_name, info.vendor, info.device))
            .subtitle(info.slot.clone())
            .model(&gtk::StringList::new(&labels))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name("video-display-symbolic"));
        list.append(&row);
        rows.push((info, options, row));
    }

    let next = gtk::Button::builder()
        .label("Next")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    fn proceed(app: &Rc<App>, chosen: Vec<GpuChoice>) {
        let mode = {
            let mut d = app.draft.borrow_mut();
            d.gpus = chosen;
            d.mode
        };
        match mode {
            Some(Mode::Foreign) => {
                app.draft.borrow_mut().source = Some(Source::Network);
                app.nav.push(&desktop_page::build(app));
            }
            _ => app.nav.push(&source_page::build(app)),
        }
    }

    {
        let app = app.clone();
        next.connect_clicked(move |_| {
            let chosen: Vec<GpuChoice> = rows
                .iter()
                .map(|(info, options, row)| {
                    let idx = (row.selected() as usize).min(options.len() - 1);
                    GpuChoice {
                        vendor: info.vendor.clone(),
                        device: info.device.clone(),
                        driver: options[idx].module.to_string(),
                    }
                })
                .collect();

            if chosen.iter().any(|g| g.driver.starts_with("nvidia-")) {
                // Explicit consent to the experimental variant before moving on.
                let dialog = adw::AlertDialog::new(
                    Some("Experimental driver"),
                    Some(
                        "The proprietary NVIDIA driver will be BUILT from the open cports PR \
                         (working directory /tmp/musl-nvidia) at the very beginning of the \
                         installation — before the disk is wiped, so a potential failure will \
                         not touch your data. The build downloads about 1.5–2 GB and takes \
                         around fifteen minutes. The variant is unofficial and may not work \
                         with every card.",
                    ),
                );
                dialog.add_responses(&[("cancel", "Cancel"), ("go", "I understand, continue")]);
                dialog.set_response_appearance("go", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");
                let app2 = app.clone();
                dialog.connect_response(None, move |_, resp| {
                    if resp == "go" {
                        proceed(&app2, chosen.clone());
                    }
                });
                dialog.present(Some(&app.win));
            } else {
                proceed(&app, chosen);
            }
        });
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .build();
    content.append(&list);
    content.append(&next);

    let status = adw::StatusPage::builder()
        .icon_name("video-display-symbolic")
        .title("Graphics card")
        .description(
            "Detected cards and the choice of open in-kernel Linux drivers. \
             The installer will add the right firmware and enable early KMS.",
        )
        .child(
            &adw::Clamp::builder()
                .maximum_size(560)
                .child(&content)
                .build(),
        )
        .build();

    wizard_page("Graphics card", "gpu", &status)
}
