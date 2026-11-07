// Podsumowanie planu instalacji i ostateczne potwierdzenie.

use super::{progress_page, wizard_page, App};
use crate::config::{format_gib, Desktop, InstallConfig, Mode, Source, Strategy, SECTOR};
use adw::prelude::*;
use std::rc::Rc;

fn row(title: &str, value: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder().title(title).subtitle(value).build();
    r.add_css_class("property");
    r
}

fn build_config(app: &Rc<App>) -> InstallConfig {
    let d = app.draft.borrow();
    let choice = d.choice.clone().expect("brak wyboru dysku");
    InstallConfig {
        mode: d.mode.expect("brak trybu"),
        source: d.source.unwrap_or(Source::Network),
        desktop: d.desktop.unwrap_or(Desktop::Base),
        disk: choice.disk.path.clone(),
        strategy: choice.strategy,
        uefi: crate::disks::host_is_uefi(),
        pttype: choice.disk.pttype.clone(),
        esp_reuse: choice.esp_reuse.clone(),
        biosboot_present: choice.biosboot_present,
        hostname: d.hostname.clone(),
        username: d.username.clone(),
        user_password: d.user_password.clone(),
        root_password: d.root_password.clone(),
        timezone: d.timezone.clone(),
    }
}

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let cfg = build_config(app);

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    list.append(&row(
        "Tryb instalacji",
        match cfg.mode {
            Mode::Live => "z nośnika live Chimery (chimera-bootstrap)",
            Mode::Foreign => "z innego systemu (obraz bazowy + chroot w /tmp/chimera-linux)",
        },
    ));
    list.append(&row(
        "Źródło pakietów",
        match cfg.source {
            Source::Local => "lokalne (z nośnika live)",
            Source::Network => "sieciowe (repo.chimera-linux.org)",
        },
    ));
    list.append(&row(
        "Wariant systemu",
        match cfg.desktop {
            Desktop::Base => "base (konsola)",
            Desktop::Gnome => "GNOME (z GDM)",
            Desktop::Kde => "KDE Plasma (z SDDM)",
        },
    ));
    list.append(&row("Dysk docelowy", &cfg.disk));
    match cfg.strategy {
        Strategy::Whole => list.append(&row(
            "Partycjonowanie",
            "CAŁY DYSK zostanie wymazany (GPT, ESP/BIOS boot + btrfs)",
        )),
        Strategy::Free { sectors, .. } => {
            let mut s = format!(
                "nowe partycje w wolnym miejscu ({}), istniejące dane nietknięte",
                format_gib(sectors * SECTOR)
            );
            if let Some(esp) = &cfg.esp_reuse {
                s.push_str(&format!("; istniejąca ESP {esp} zostanie użyta ponownie"));
            }
            list.append(&row("Partycjonowanie", &s));
        }
    }
    list.append(&row(
        "System plików",
        "btrfs, subwolumeny jak w openSUSE (@, @/.snapshots, @/home, @/var, …)",
    ));
    list.append(&row(
        "Migawki",
        "snapper skonfigurowany jak w openSUSE; wpisy migawek w menu GRUB-a",
    ));
    list.append(&row(
        "Program rozruchowy",
        if cfg.uefi {
            "GRUB (UEFI, x86_64-efi, ESP pod /boot/efi)"
        } else {
            "GRUB (BIOS, i386-pc)"
        },
    ));
    list.append(&row("Nazwa komputera", &cfg.hostname));
    list.append(&row(
        "Użytkownik",
        &format!("{} (wheel); root: {}", cfg.username,
            if cfg.root_password.is_some() { "z hasłem" } else { "zablokowany" }),
    ));
    list.append(&row("Strefa czasowa", &cfg.timezone));

    let install = gtk::Button::builder()
        .label("Zainstaluj")
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();

    {
        let app = app.clone();
        install.connect_clicked(move |_| {
            let cfg = build_config(&app);
            let body = match cfg.strategy {
                Strategy::Whole => format!(
                    "Cały dysk {} zostanie wymazany i podzielony od nowa. \
                     Tej operacji nie można cofnąć.",
                    cfg.disk
                ),
                Strategy::Free { .. } => format!(
                    "Na dysku {} zostaną utworzone nowe partycje w wolnym miejscu. \
                     Istniejące partycje nie będą zmieniane.",
                    cfg.disk
                ),
            };
            let dialog = adw::AlertDialog::new(Some("Rozpocząć instalację?"), Some(&body));
            dialog.add_responses(&[("cancel", "Anuluj"), ("go", "Rozpocznij instalację")]);
            dialog.set_response_appearance("go", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let app2 = app.clone();
            dialog.connect_response(None, move |_, resp| {
                if resp == "go" {
                    progress_page::start(&app2, build_config(&app2));
                }
            });
            dialog.present(Some(&app.win));
        });
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .build();
    content.append(&list);
    content.append(&install);

    let status = adw::StatusPage::builder()
        .icon_name("emblem-documents-symbolic")
        .title("Podsumowanie")
        .description("Sprawdź plan instalacji. Zmiany na dysku zaczną się dopiero po potwierdzeniu.")
        .child(
            &adw::Clamp::builder()
                .maximum_size(560)
                .child(&content)
                .build(),
        )
        .build();

    wizard_page("Podsumowanie", "summary", &status)
}
