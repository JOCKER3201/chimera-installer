// The installation mode choice page — the first one after startup.

use super::{gpu_page, wizard_page, App};
use crate::config::Mode;
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
        .title("Install from a Chimera live medium")
        .subtitle(if live_available {
            "The system will be installed with the chimera-bootstrap tool from the running medium."
        } else {
            "Unavailable: chimera-bootstrap missing — run the installer from a Chimera Linux live image."
        })
        .activatable(live_available)
        .sensitive(live_available)
        .build();
    live.add_prefix(&gtk::Image::from_icon_name("media-optical-symbolic"));
    live.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    let foreign = adw::ActionRow::builder()
        .title("Install from another system")
        .subtitle(
            "Downloads the Chimera Linux base image, unpacks it to /tmp/chimera-linux \
             and installs via chroot. Requires an internet connection.",
        )
        .activatable(true)
        .build();
    foreign.add_prefix(&gtk::Image::from_icon_name("network-server-symbolic"));
    foreign.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    list.append(&live);
    list.append(&foreign);

    // The GPU page always follows the mode; it routes onward (live -> package
    // source, another system -> straight to the variant, as a network install).
    fn pick(app: &Rc<App>, mode: Mode) {
        app.draft.borrow_mut().mode = Some(mode);
        app.nav.push(&gpu_page::build(app));
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
        .title("Chimera Linux Installer")
        .description("Choose how to install the system.")
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&list)
                .build(),
        )
        .build();

    wizard_page("Installation mode", "mode", &status)
}
