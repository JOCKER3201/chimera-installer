// The package source choice page — only for installations from a live medium.

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
        .title("Locally, from the medium")
        .subtitle(
            "A faithful copy of the medium's system — together with its desktop and \
             enabled services (e.g. the Plasma ISO yields a system with Plasma and \
             SDDM). Extras not on the medium — snapper and CPU microcode — will be \
             downloaded from the network.",
        )
        .activatable(true)
        .build();
    local.add_prefix(&gtk::Image::from_icon_name("drive-removable-media-symbolic"));
    local.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    let network = adw::ActionRow::builder()
        .title("From the network")
        .subtitle(
            "The latest packages from the Chimera Linux repository — requires \
             internet. Lets you choose the system variant (base, GNOME, KDE Plasma).",
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
        .title("Package source")
        .description("Where should the installed system's packages come from?")
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&list)
                .build(),
        )
        .build();

    wizard_page("Package source", "source", &status)
}
