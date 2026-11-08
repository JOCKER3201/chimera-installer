// Strona wyboru wariantu systemu (base / GNOME / KDE Plasma) —
// dla instalacji sieciowej z live oraz dla instalacji z innego systemu.

use super::{disk_page, edition_page, wizard_page, App};
use crate::config::Desktop;
use adw::prelude::*;
use std::rc::Rc;

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    let entries: [(&str, &str, &str, Desktop); 3] = [
        (
            "System bazowy",
            "Konsola bez środowiska graficznego (base-full). Pulpit można doinstalować później.",
            "utilities-terminal-symbolic",
            Desktop::Base,
        ),
        (
            "GNOME",
            "Środowisko GNOME z menedżerem logowania GDM (wariant na następnym ekranie).",
            "preferences-desktop-appearance-symbolic",
            Desktop::Gnome,
        ),
        (
            "KDE Plasma",
            "Środowisko KDE Plasma z menedżerem logowania SDDM (wariant na następnym ekranie).",
            "preferences-desktop-theme-symbolic",
            Desktop::Kde,
        ),
    ];

    for (title, subtitle, icon, desktop) in entries {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(subtitle)
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let app = app.clone();
        row.connect_activated(move |_| {
            app.draft.borrow_mut().desktop = Some(desktop);
            // GNOME/KDE → wybór wariantu (base/full); System bazowy → dysk.
            match desktop {
                Desktop::Base => app.nav.push(&disk_page::build(&app)),
                _ => app.nav.push(&edition_page::build(&app)),
            }
        });
        list.append(&row);
    }

    let status = adw::StatusPage::builder()
        .icon_name("preferences-desktop-display-symbolic")
        .title("Wariant systemu")
        .description("Wybierz, co ma zostać zainstalowane z repozytorium sieciowego.")
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&list)
                .build(),
        )
        .build();

    wizard_page("Wariant systemu", "desktop", &status)
}
