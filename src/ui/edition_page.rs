// Strona wyboru wariantu pulpitu (base / full) — po wyborze GNOME lub KDE.

use super::{disk_page, wizard_page, App};
use crate::config::{Desktop, Edition};
use adw::prelude::*;
use std::rc::Rc;

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let desktop = app.draft.borrow().desktop.unwrap_or(Desktop::Gnome);
    let name = match desktop {
        Desktop::Kde => "KDE Plasma",
        _ => "GNOME",
    };

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    let base_sub = match desktop {
        Desktop::Kde => {
            "Plasma bez zestawu aplikacji i multimediów, z samym terminalem (Konsole). \
             Dodatkowe programy doinstalujesz później."
        }
        _ => {
            "GNOME bez pakietu aplikacji, z samym terminalem (Console). \
             Dodatkowe programy doinstalujesz później."
        }
    };

    let base = adw::ActionRow::builder()
        .title("Base — lekki")
        .subtitle(base_sub)
        .activatable(true)
        .build();
    base.add_prefix(&gtk::Image::from_icon_name("application-x-executable-symbolic"));
    base.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    let full = adw::ActionRow::builder()
        .title("Full — pełny")
        .subtitle(format!(
            "Kompletne środowisko {name} ze wszystkimi domyślnymi aplikacjami."
        ))
        .activatable(true)
        .build();
    full.add_prefix(&gtk::Image::from_icon_name("view-grid-symbolic"));
    full.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    list.append(&base);
    list.append(&full);

    let pick = |app: &Rc<App>, edition: Edition| {
        app.draft.borrow_mut().edition = Some(edition);
        app.nav.push(&disk_page::build(app));
    };
    {
        let app = app.clone();
        base.connect_activated(move |_| pick(&app, Edition::Base));
    }
    {
        let app = app.clone();
        full.connect_activated(move |_| pick(&app, Edition::Full));
    }

    let status = adw::StatusPage::builder()
        .icon_name("preferences-desktop-apps-symbolic")
        .title(format!("Wariant: {name}"))
        .description("Ile aplikacji zainstalować razem ze środowiskiem graficznym?")
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&list)
                .build(),
        )
        .build();

    wizard_page("Wariant pulpitu", "edition", &status)
}
