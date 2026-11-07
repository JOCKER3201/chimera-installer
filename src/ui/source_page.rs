// Strona wyboru źródła pakietów — tylko dla instalacji z nośnika live.

use super::{desktop_page, disk_page, wizard_page, App};
use crate::config::{Desktop, Source};
use adw::prelude::*;
use std::rc::Rc;

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    let local = adw::ActionRow::builder()
        .title("Lokalnie, z nośnika")
        .subtitle(
            "Wierna kopia systemu z nośnika — razem z jego pulpitem i włączonymi \
             usługami (np. ISO Plasmy daje system z Plasmą i SDDM). Dodatki spoza \
             nośnika — snapper i mikrokod CPU — zostaną pobrane z sieci.",
        )
        .activatable(true)
        .build();
    local.add_prefix(&gtk::Image::from_icon_name("drive-removable-media-symbolic"));
    local.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    let network = adw::ActionRow::builder()
        .title("Z sieci")
        .subtitle(
            "Najnowsze pakiety z repozytorium Chimera Linux — wymaga internetu. \
             Pozwala wybrać wariant systemu (base, GNOME, KDE Plasma).",
        )
        .activatable(true)
        .build();
    network.add_prefix(&gtk::Image::from_icon_name("network-workgroup-symbolic"));
    network.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    list.append(&local);
    list.append(&network);

    {
        let app = app.clone();
        local.connect_activated(move |_| {
            {
                let mut d = app.draft.borrow_mut();
                d.source = Some(Source::Local);
                d.desktop = Some(Desktop::Base);
            }
            app.nav.push(&disk_page::build(&app));
        });
    }
    {
        let app = app.clone();
        network.connect_activated(move |_| {
            app.draft.borrow_mut().source = Some(Source::Network);
            app.nav.push(&desktop_page::build(&app));
        });
    }

    let status = adw::StatusPage::builder()
        .icon_name("folder-download-symbolic")
        .title("Źródło pakietów")
        .description("Skąd mają pochodzić pakiety instalowanego systemu?")
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&list)
                .build(),
        )
        .build();

    wizard_page("Źródło pakietów", "source", &status)
}
