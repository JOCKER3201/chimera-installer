// Strona postępu: uruchamia helper roota przez pkexec, czyta protokół
// z jego stdout w wątku i aktualizuje UI przez async_channel.

use super::{wizard_page, App};
use crate::config::InstallConfig;
use adw::prelude::*;
use gtk::glib;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::rc::Rc;

enum Msg {
    Step(u32, u32, String),
    Log(String),
    Done,
    Fail(String),
}

fn spawn_helper(cfg: &InstallConfig, tx: async_channel::Sender<Msg>) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.send_blocking(Msg::Fail(format!("nie znam własnej ścieżki: {e}")));
            return;
        }
    };
    let json = match serde_json::to_string(cfg) {
        Ok(j) => j,
        Err(e) => {
            let _ = tx.send_blocking(Msg::Fail(format!("serializacja konfiguracji: {e}")));
            return;
        }
    };

    std::thread::spawn(move || {
        let mut child = match Command::new("pkexec")
            .arg(&exe)
            .arg("helper")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send_blocking(Msg::Fail(format!("nie można uruchomić pkexec: {e}")));
                return;
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(json.as_bytes()) {
                let _ = tx.send_blocking(Msg::Fail(format!(
                    "nie można przekazać konfiguracji helperowi: {e}"
                )));
                let _ = child.kill();
                return;
            }
            // Zamknięcie stdin sygnalizuje helperowi koniec konfiguracji.
        }

        // stderr (w tym komunikaty pkexec/polkit) w osobnym wątku do logu.
        if let Some(err) = child.stderr.take() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    let _ = tx.send_blocking(Msg::Log(format!("[stderr] {line}")));
                }
            });
        }

        let mut finished = false;
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(rest) = line.strip_prefix("::P ") {
                    let mut it = rest.splitn(3, ' ');
                    let step = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let total = it.next().and_then(|s| s.parse().ok()).unwrap_or(1);
                    let text = it.next().unwrap_or("").to_string();
                    let _ = tx.send_blocking(Msg::Step(step, total, text));
                } else if let Some(rest) = line.strip_prefix("::L ") {
                    let _ = tx.send_blocking(Msg::Log(rest.to_string()));
                } else if line == "::D" {
                    finished = true;
                    let _ = tx.send_blocking(Msg::Done);
                } else if let Some(rest) = line.strip_prefix("::E ") {
                    finished = true;
                    let _ = tx.send_blocking(Msg::Fail(rest.to_string()));
                } else {
                    let _ = tx.send_blocking(Msg::Log(line));
                }
            }
        }

        let status = child.wait();
        if !finished {
            // Helper zakończył się bez ::D/::E — np. odmowa autoryzacji pkexec
            // (kod 126) albo przerwanie dialogu (127).
            let msg = match status {
                Ok(s) if s.code() == Some(126) => {
                    "autoryzacja pkexec nie powiodła się".to_string()
                }
                Ok(s) if s.code() == Some(127) => {
                    "autoryzacja pkexec została anulowana".to_string()
                }
                Ok(s) => format!("helper zakończył się niespodziewanie ({s})"),
                Err(e) => format!("błąd oczekiwania na helper: {e}"),
            };
            let _ = tx.send_blocking(Msg::Fail(msg));
        }
    });
}

pub fn start(app: &Rc<App>, cfg: InstallConfig) {
    let step_label = gtk::Label::builder()
        .label("Przygotowywanie…")
        .css_classes(["title-2"])
        .wrap(true)
        .build();
    let bar = gtk::ProgressBar::builder().show_text(true).build();

    let log_buffer = gtk::TextBuffer::new(None);
    let log_view = gtk::TextView::builder()
        .buffer(&log_buffer)
        .editable(false)
        .monospace(true)
        .cursor_visible(false)
        .left_margin(8)
        .right_margin(8)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&log_view)
        .min_content_height(260)
        .vexpand(true)
        .css_classes(["card"])
        .build();
    let expander = gtk::Expander::builder()
        .label("Szczegóły (dziennik instalacji)")
        .child(&scroll)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(36)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    content.append(&step_label);
    content.append(&bar);
    content.append(&expander);

    let clamp = adw::Clamp::builder().maximum_size(640).child(&content).build();
    let page = wizard_page("Instalacja", "progress", &clamp);
    page.set_can_pop(false);
    app.nav.push(&page);

    let (tx, rx) = async_channel::unbounded::<Msg>();
    spawn_helper(&cfg, tx);

    let app = app.clone();
    let mode = cfg.mode;
    // Pełny dziennik instalacji zapisywany równolegle na pulpit użytkownika.
    let log_path = desktop_log_path();
    let log_file = std::cell::RefCell::new(std::fs::File::create(&log_path).ok());
    glib::spawn_future_local(async move {
        if let Some(f) = log_file.borrow_mut().as_mut() {
            let _ = writeln!(f, "# dziennik instalacji chimera-installer");
        }
        let append_log = |text: &str| {
            let mut end = log_buffer.end_iter();
            log_buffer.insert(&mut end, text);
            log_buffer.insert(&mut end, "\n");
            let mark = log_buffer.create_mark(None, &log_buffer.end_iter(), false);
            log_view.scroll_to_mark(&mark, 0.0, true, 0.0, 1.0);
            log_buffer.delete_mark(&mark);
            if let Some(f) = log_file.borrow_mut().as_mut() {
                let _ = writeln!(f, "{text}");
                let _ = f.flush();
            }
        };
        while let Ok(msg) = rx.recv().await {
            match msg {
                Msg::Step(step, total, text) => {
                    step_label.set_label(&text);
                    bar.set_fraction(f64::from(step.saturating_sub(1)) / f64::from(total.max(1)));
                    bar.set_text(Some(&format!("Krok {step} z {total}")));
                    append_log(&format!("==> {text}"));
                }
                Msg::Log(line) => append_log(&line),
                Msg::Done => {
                    bar.set_fraction(1.0);
                    bar.set_text(Some("Zakończono"));
                    step_label.set_label("Instalacja zakończona");
                    app.nav.push(&done_page(&app, mode));
                    break;
                }
                Msg::Fail(e) => {
                    step_label.set_label("Błąd instalacji");
                    bar.add_css_class("error");
                    append_log(&format!("BŁĄD: {e}"));
                    expander.set_expanded(true);
                    page.set_can_pop(true);
                    app.toast(&format!("Instalacja nie powiodła się: {e}"));
                    break;
                }
            }
        }
    });
}

/// Ścieżka dziennika na pulpicie użytkownika (XDG_DESKTOP_DIR, potem
/// Desktop/Pulpit, w ostateczności katalog domowy).
fn desktop_log_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    if let Ok(cfg) = std::fs::read_to_string(format!("{home}/.config/user-dirs.dirs")) {
        for line in cfg.lines() {
            if let Some(v) = line.trim().strip_prefix("XDG_DESKTOP_DIR=") {
                let dir = v.trim().trim_matches('"').replace("$HOME", &home);
                let p = std::path::PathBuf::from(dir);
                if p.is_dir() {
                    return p.join("chimera-installer.log");
                }
            }
        }
    }
    for d in ["Desktop", "Pulpit"] {
        let p = std::path::Path::new(&home).join(d);
        if p.is_dir() {
            return p.join("chimera-installer.log");
        }
    }
    std::path::Path::new(&home).join("chimera-installer.log")
}

fn done_page(app: &Rc<App>, mode: crate::config::Mode) -> adw::NavigationPage {
    let close = gtk::Button::builder()
        .label("Zamknij instalator")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    {
        let win = app.win.clone();
        close.connect_clicked(move |_| win.close());
    }

    let desc = match mode {
        crate::config::Mode::Live => {
            "System został zainstalowany. Uruchom komputer ponownie i odłącz nośnik live. \
             Po pierwszym uruchomieniu migawki snappera są gotowe do użycia."
        }
        crate::config::Mode::Foreign => {
            "System został zainstalowany. Katalog roboczy /tmp/chimera-linux można usunąć. \
             Uruchom komputer ponownie i wybierz wpis „chimera” w menu rozruchu."
        }
    };

    let status = adw::StatusPage::builder()
        .icon_name("object-select-symbolic")
        .title("Instalacja zakończona")
        .description(desc)
        .child(&close)
        .build();

    let page = wizard_page("Gotowe", "done", &status);
    page.set_can_pop(false);
    page
}
