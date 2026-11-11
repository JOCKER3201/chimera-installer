// The system variant choice page (base / GNOME / KDE Plasma) —
// for network installations from live and for installations from another system.

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
            "Base system",
            "A console without a graphical environment (base-full). A desktop can be added later.",
            "utilities-terminal-symbolic",
            Desktop::Base,
        ),
        (
            "GNOME",
            "The GNOME environment with the GDM login manager (edition on the next screen).",
            "preferences-desktop-appearance-symbolic",
            Desktop::Gnome,
        ),
        (
            "KDE Plasma",
            "The KDE Plasma environment with the SDDM login manager (edition on the next screen).",
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
            // GNOME/KDE → edition choice (base/full); base system → disk.
            match desktop {
                Desktop::Base => app.nav.push(&disk_page::build(&app)),
                _ => app.nav.push(&edition_page::build(&app)),
            }
        });
        list.append(&row);
    }

    let status = adw::StatusPage::builder()
        .icon_name("preferences-desktop-display-symbolic")
        .title("System variant")
        .description("Choose what to install from the network repository.")
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&list)
                .build(),
        )
        .build();

    wizard_page("System variant", "desktop", &status)
}
