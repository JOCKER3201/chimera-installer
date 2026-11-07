// Strona wykrywania GPU i wyboru otwartych sterowników z jądra —
// między wyborem trybu a źródłem pakietów / wariantem systemu.

use super::{desktop_page, source_page, wizard_page, App};
use crate::config::{GpuChoice, Mode, Source};
use crate::gpu;
use adw::prelude::*;
use std::rc::Rc;


/// Eksperymentalne, zastrzeżone sterowniki NVIDIA budowane z PR-a cports
/// (#6531) — dostępne wyłącznie w trybie live (budowa przez cbuild na
/// działającym nośniku), wymaga sieci niezależnie od źródła pakietów.
const NVIDIA_EXPERIMENTAL: &[gpu::DriverOption] = &[
    gpu::DriverOption {
        module: "nvidia-620",
        label: "Zastrzeżony NVIDIA 620 — Turing i nowsze (EKSPERYMENTALNY)",
    },
    gpu::DriverOption {
        module: "nvidia-580",
        label: "Zastrzeżony NVIDIA 580 — Maxwell/Pascal (EKSPERYMENTALNY)",
    },
];

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    let live = app.draft.borrow().mode == Some(Mode::Live);
    let gpus = gpu::scan();
    // Karta + jej pełna lista opcji + wiersz, do odczytu przy przejściu dalej.
    let mut rows: Vec<(gpu::GpuInfo, Vec<gpu::DriverOption>, adw::ComboRow)> = Vec::new();

    if gpus.is_empty() {
        list.append(
            &adw::ActionRow::builder()
                .title("Nie wykryto kart graficznych")
                .subtitle("Instalacja będzie kontynuowana bez konfiguracji sterowników.")
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
        .label("Dalej")
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
                // Wyraźna zgoda na wariant eksperymentalny przed przejściem dalej.
                let dialog = adw::AlertDialog::new(
                    Some("Sterownik eksperymentalny"),
                    Some(
                        "Zastrzeżony sterownik NVIDIA zostanie ZBUDOWANY z otwartego PR-a cports \
                         (katalog roboczy /tmp/musl-nvidia) na samym początku instalacji — jeszcze \
                         przed wymazaniem dysku, więc ewentualne niepowodzenie nie naruszy danych. \
                         Budowa pobiera ok. 1,5–2 GB i trwa kilkanaście minut. Wariant jest \
                         nieoficjalny i może nie działać na każdej karcie.",
                    ),
                );
                dialog.add_responses(&[("cancel", "Anuluj"), ("go", "Rozumiem, kontynuuj")]);
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
