// Strona wyboru trybu instalacji — pierwsza po uruchomieniu.

use super::{desktop_page, source_page, wizard_page, App};
use crate::config::{Mode, Source};
use adw::prelude::*;
use std::path::Path;
use std::rc::Rc;

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    let live_available = Path::new("/usr/bin/chimera-bootstrap").exists();

    let live = adw::ActionRow::builder()
        .title("Instalacja z nośnika live Chimery")
        .subtitle(if live_available {
            "System zostanie zainstalowany narzędziem chimera-bootstrap z uruchomionego nośnika."
        } else {
            "Niedostępne: brak chimera-bootstrap — uruchom instalator z obrazu live Chimera Linux."
        })
        .activatable(live_available)
        .sensitive(live_available)
        .build();
    live.add_prefix(&gtk::Image::from_icon_name("media-optical-symbolic"));
    live.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    let foreign = adw::ActionRow::builder()
        .title("Instalacja z innego systemu")
        .subtitle(
            "Pobiera bazowy obraz Chimera Linux, rozpakowuje go do /tmp/chimera-linux \
             i instaluje przez chroot. Wymaga połączenia z internetem.",
        )
        .activatable(true)
        .build();
    foreign.add_prefix(&gtk::Image::from_icon_name("network-server-symbolic"));
    foreign.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    list.append(&live);
    list.append(&foreign);

    fn pick(app: &Rc<App>, mode: Mode) {
        app.draft.borrow_mut().mode = Some(mode);
        match mode {
            // Na live wybiera się źródło pakietów (lokalne/sieć)…
            Mode::Live => app.nav.push(&source_page::build(app)),
            // …a tryb z innego systemu działa zawsze jak sieciowy.
            Mode::Foreign => {
                app.draft.borrow_mut().source = Some(Source::Network);
                app.nav.push(&desktop_page::build(app));
            }
        }
    }
    {
        let app = app.clone();
        live.connect_activated(move |_| pick(&app, Mode::Live));
    }
    {
        let app = app.clone();
        foreign.connect_activated(move |_| pick(&app, Mode::Foreign));
    }

    let status = adw::StatusPage::builder()
        .icon_name("system-software-install-symbolic")
        .title("Instalator Chimera Linux")
        .description("Wybierz sposób instalacji systemu.")
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&list)
                .build(),
        )
        .build();

    wizard_page("Tryb instalacji", "mode", &status)
}
