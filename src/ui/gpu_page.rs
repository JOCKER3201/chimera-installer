// Strona wykrywania GPU i wyboru otwartych sterowników z jądra —
// między wyborem trybu a źródłem pakietów / wariantem systemu.

use super::{desktop_page, source_page, wizard_page, App};
use crate::config::{GpuChoice, Mode, Source};
use crate::gpu;
use adw::prelude::*;
use std::rc::Rc;

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    let gpus = gpu::scan();
    // Pary (karta, wiersz z wyborem) do odczytu przy przejściu dalej.
    let mut rows: Vec<(gpu::GpuInfo, adw::ComboRow)> = Vec::new();

    if gpus.is_empty() {
        list.append(
            &adw::ActionRow::builder()
                .title("Nie wykryto kart graficznych")
                .subtitle("Instalacja będzie kontynuowana bez konfiguracji sterowników.")
                .build(),
        );
    }
    for info in gpus {
        let labels: Vec<&str> = info.options.iter().map(|o| o.label).collect();
        let row = adw::ComboRow::builder()
            .title(format!("{} — {}:{}", info.vendor_name, info.vendor, info.device))
            .subtitle(info.slot.clone())
            .model(&gtk::StringList::new(&labels))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name("video-display-symbolic"));
        list.append(&row);
        rows.push((info, row));
    }

    let next = gtk::Button::builder()
        .label("Dalej")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    {
        let app = app.clone();
        next.connect_clicked(move |_| {
            let chosen: Vec<GpuChoice> = rows
                .iter()
                .map(|(info, row)| {
                    let idx = (row.selected() as usize).min(info.options.len() - 1);
                    GpuChoice {
                        vendor: info.vendor.clone(),
                        device: info.device.clone(),
                        driver: info.options[idx].module.to_string(),
                    }
                })
                .collect();
            let mode = {
                let mut d = app.draft.borrow_mut();
                d.gpus = chosen;
                d.mode
            };
            match mode {
                Some(Mode::Foreign) => {
                    app.draft.borrow_mut().source = Some(Source::Network);
                    app.nav.push(&desktop_page::build(&app));
                }
                _ => app.nav.push(&source_page::build(&app)),
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
        .title("Karta graficzna")
        .description(
            "Wykryte karty i dobór otwartych sterowników z jądra Linux. \
             Instalator doinstaluje odpowiedni firmware i włączy wczesny KMS.",
        )
        .child(
            &adw::Clamp::builder()
                .maximum_size(560)
                .child(&content)
                .build(),
        )
        .build();

    wizard_page("Karta graficzna", "gpu", &status)
}
