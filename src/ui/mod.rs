// GUI instalatora: libadwaita, nawigacja stronami kreatora.

mod desktop_page;
mod disk_page;
mod gpu_page;
mod mode_page;
mod progress_page;
mod source_page;
mod summary_page;
mod user_page;

use crate::config::{Desktop, GpuChoice, Mode, Source, Strategy};
use crate::disks::DiskInfo;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Wybór dysku wraz z decyzją o strategii z okna dialogowego.
#[derive(Clone)]
pub struct DiskChoice {
    pub disk: DiskInfo,
    pub strategy: Strategy,
    pub esp_reuse: Option<String>,
    pub biosboot_present: bool,
}

/// Dane zbierane przez kolejne strony kreatora.
#[derive(Default)]
pub struct Draft {
    pub mode: Option<Mode>,
    pub source: Option<Source>,
    pub desktop: Option<Desktop>,
    pub gpus: Vec<GpuChoice>,
    pub choice: Option<DiskChoice>,
    pub hostname: String,
    pub username: String,
    pub user_password: String,
    pub root_password: Option<String>,
    pub timezone: String,
}

pub struct App {
    pub win: adw::ApplicationWindow,
    pub nav: adw::NavigationView,
    pub toasts: adw::ToastOverlay,
    pub draft: RefCell<Draft>,
}

impl App {
    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }
}

/// Strona kreatora w standardowym opakowaniu (pasek nagłówka + zawartość).
pub fn wizard_page(title: &str, tag: &str, content: &impl IsA<gtk::Widget>) -> adw::NavigationPage {
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(content));
    adw::NavigationPage::builder()
        .title(title)
        .tag(tag)
        .child(&tv)
        .build()
}

/// Atrapa trybu zaawansowanego — zgodnie z założeniem na razie tylko informuje.
pub fn advanced_mode_stub(app: &Rc<App>) {
    app.toast("Tryb zaawansowany nie jest jeszcze dostępny w tej wersji instalatora.");
}

pub fn run() -> i32 {
    let application = adw::Application::builder()
        .application_id("io.github.jocker3201.ChimeraInstaller")
        .build();

    application.connect_activate(|gtk_app| {
        let nav = adw::NavigationView::new();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&nav));

        let win = adw::ApplicationWindow::builder()
            .application(gtk_app)
            .title("Instalator Chimera Linux")
            .default_width(760)
            .default_height(600)
            .content(&toasts)
            .build();

        let app = Rc::new(App {
            win: win.clone(),
            nav: nav.clone(),
            toasts,
            draft: RefCell::new(Draft::default()),
        });

        nav.push(&mode_page::build(&app));
        win.present();
    });

    // Pusta lista argumentów — argv obsłużyliśmy sami w main().
    application.run_with_args::<&str>(&[]).value()
}
