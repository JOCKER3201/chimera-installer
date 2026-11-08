// Strona „System": nazwa komputera, strefa czasowa (dialog kontynent→strefa),
// język i układ klawiatury (dialog układ + wariant).

use super::{user_page, wizard_page, App};
use crate::sysdata;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

fn valid_hostname(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Opis układu klawiatury do podtytułu wiersza.
fn kb_label(layout: &str, variant: &str) -> String {
    let lname = sysdata::kb_layouts()
        .into_iter()
        .find(|(c, _)| c == layout)
        .map(|(_, d)| d)
        .unwrap_or_else(|| layout.to_string());
    if variant.is_empty() {
        lname
    } else {
        let vname = sysdata::kb_variants(layout)
            .into_iter()
            .find(|(c, _)| c == variant)
            .map(|(_, d)| d)
            .unwrap_or_else(|| variant.to_string());
        format!("{lname} — {vname}")
    }
}

// ---------------------------------------------------------------------------
// Dialog strefy czasowej: kontynent → strefa
// ---------------------------------------------------------------------------

fn present_timezone_dialog(app: &Rc<App>, row: &adw::ActionRow) {
    let regions = sysdata::tz_regions();
    let region_refs: Vec<&str> = regions.iter().map(String::as_str).collect();

    let region_row = adw::ComboRow::builder()
        .title("Kontynent")
        .model(&gtk::StringList::new(&region_refs))
        .build();
    let city_row = adw::ComboRow::builder().title("Strefa").build();

    // Bieżąca strefa z draftu, np. "Europe/Warsaw".
    let cur_tz = app.draft.borrow().timezone.clone();
    let (cur_region, cur_city) = cur_tz.split_once('/').unwrap_or(("Europe", "Warsaw"));
    let region_start = regions.iter().position(|r| r == cur_region).unwrap_or(0);
    region_row.set_selected(region_start as u32);

    // Miasta zależne od kontynentu (wartości: "Warsaw", "Argentina/Buenos_Aires").
    let cities_state: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let refresh_cities = {
        let regions = regions.clone();
        let city_row = city_row.clone();
        let cities_state = cities_state.clone();
        move |region_idx: usize, preselect: Option<&str>| {
            let cities = sysdata::tz_cities(&regions[region_idx]);
            let labels: Vec<String> = cities.iter().map(|c| c.replace('_', " ")).collect();
            let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            city_row.set_model(Some(&gtk::StringList::new(&label_refs)));
            let sel = preselect
                .and_then(|c| cities.iter().position(|x| x == c))
                .unwrap_or(0);
            city_row.set_selected(sel as u32);
            *cities_state.borrow_mut() = cities;
        }
    };
    refresh_cities(region_start, Some(cur_city));
    {
        let refresh = refresh_cities.clone();
        region_row.connect_selected_notify(move |r| refresh(r.selected() as usize, None));
    }

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    list.append(&region_row);
    list.append(&city_row);

    let choose = gtk::Button::builder()
        .label("Wybierz")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .margin_top(12)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.append(&list);
    content.append(&choose);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(&content));
    let dialog = adw::Dialog::builder()
        .title("Strefa czasowa")
        .content_width(460)
        .child(&tv)
        .build();

    {
        let app = app.clone();
        let row = row.clone();
        let dialog = dialog.clone();
        let regions = regions.clone();
        let cities_state = cities_state.clone();
        let region_row = region_row.clone();
        let city_row = city_row.clone();
        choose.connect_clicked(move |_| {
            let ri = region_row.selected() as usize;
            let region = regions.get(ri).cloned().unwrap_or_default();
            let ci = city_row.selected() as usize;
            let city = cities_state.borrow().get(ci).cloned().unwrap_or_default();
            if !city.is_empty() {
                let full = sysdata::tz_full(&region, &city);
                app.draft.borrow_mut().timezone = full.clone();
                row.set_subtitle(&full);
            }
            dialog.close();
        });
    }

    dialog.present(Some(&app.win));
}

// ---------------------------------------------------------------------------
// Dialog klawiatury: układ + wariant
// ---------------------------------------------------------------------------

fn present_keyboard_dialog(app: &Rc<App>, row: &adw::ActionRow) {
    let layouts = sysdata::kb_layouts();
    let labels: Vec<String> = layouts.iter().map(|(c, d)| format!("{d} ({c})")).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let layout_model = gtk::StringList::new(&label_refs);

    let layout_row = adw::ComboRow::builder()
        .title("Układ")
        .model(&layout_model)
        .build();
    let variant_row = adw::ComboRow::builder().title("Wariant").build();

    // Ustawienie początkowe na bieżący wybór z draftu.
    let cur_layout = app.draft.borrow().kb_layout.clone();
    let start_idx = layouts.iter().position(|(c, _)| *c == cur_layout).unwrap_or(0) as u32;
    layout_row.set_selected(start_idx);

    let variants_state: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
    let refresh_variants = {
        let layouts = layouts.clone();
        let variant_row = variant_row.clone();
        let variants_state = variants_state.clone();
        move |layout_idx: usize| {
            let layout = &layouts[layout_idx].0;
            let variants = sysdata::kb_variants(layout);
            let vlabels: Vec<&str> = variants.iter().map(|(_, d)| d.as_str()).collect();
            variant_row.set_model(Some(&gtk::StringList::new(&vlabels)));
            *variants_state.borrow_mut() = variants;
            variant_row.set_selected(0);
        }
    };
    refresh_variants(start_idx as usize);
    {
        let refresh = refresh_variants.clone();
        layout_row.connect_selected_notify(move |r| refresh(r.selected() as usize));
    }

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    list.append(&layout_row);
    list.append(&variant_row);

    let choose = gtk::Button::builder()
        .label("Wybierz")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .margin_top(12)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.append(&list);
    content.append(&choose);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(&content));
    let dialog = adw::Dialog::builder()
        .title("Układ klawiatury")
        .content_width(460)
        .child(&tv)
        .build();

    {
        let app = app.clone();
        let row = row.clone();
        let dialog = dialog.clone();
        let layouts = layouts.clone();
        let variants_state = variants_state.clone();
        let layout_row = layout_row.clone();
        let variant_row = variant_row.clone();
        choose.connect_clicked(move |_| {
            let li = layout_row.selected() as usize;
            let layout = layouts.get(li).map(|(c, _)| c.clone()).unwrap_or_default();
            let vi = variant_row.selected() as usize;
            let variant = variants_state
                .borrow()
                .get(vi)
                .map(|(c, _)| c.clone())
                .unwrap_or_default();
            {
                let mut d = app.draft.borrow_mut();
                d.kb_layout = layout.clone();
                d.kb_variant = variant.clone();
            }
            row.set_subtitle(&kb_label(&layout, &variant));
            dialog.close();
        });
    }

    dialog.present(Some(&app.win));
}

// ---------------------------------------------------------------------------
// Strona
// ---------------------------------------------------------------------------

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    // Inicjalizacja draftu domyślnymi wartościami (przy pierwszym wejściu).
    {
        let mut d = app.draft.borrow_mut();
        if d.timezone.is_empty() {
            d.timezone = sysdata::host_timezone();
        }
        if d.locale.is_empty() {
            d.locale = "en_US.UTF-8".to_string();
        }
        if d.kb_layout.is_empty() {
            d.kb_layout = "us".to_string();
        }
        if d.hostname.is_empty() {
            d.hostname = "chimera".to_string();
        }
    }
    let (cur_tz, cur_locale, cur_layout, cur_variant, cur_host) = {
        let d = app.draft.borrow();
        (
            d.timezone.clone(),
            d.locale.clone(),
            d.kb_layout.clone(),
            d.kb_variant.clone(),
            d.hostname.clone(),
        )
    };

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    let hostname = adw::EntryRow::builder().title("Nazwa komputera").text(&cur_host).build();
    list.append(&hostname);

    // Strefa czasowa — wiersz otwierający dialog.
    let tz_row = adw::ActionRow::builder()
        .title("Strefa czasowa")
        .subtitle(&cur_tz)
        .activatable(true)
        .build();
    tz_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let app = app.clone();
        let tz_row2 = tz_row.clone();
        tz_row.connect_activated(move |_| present_timezone_dialog(&app, &tz_row2));
    }
    list.append(&tz_row);

    // Język — lista rozwijana.
    let locales = sysdata::locales();
    let loc_labels: Vec<&str> = locales.iter().map(|(n, _)| *n).collect();
    let lang_row = adw::ComboRow::builder()
        .title("Język")
        .subtitle("Pełne tłumaczenia systemu bazowego tylko dla części języków (musl)")
        .model(&gtk::StringList::new(&loc_labels))
        .build();
    let lang_start = locales.iter().position(|(_, v)| *v == cur_locale).unwrap_or(0) as u32;
    lang_row.set_selected(lang_start);
    {
        let app = app.clone();
        let locales = locales.clone();
        lang_row.connect_selected_notify(move |r| {
            if let Some((_, v)) = locales.get(r.selected() as usize) {
                app.draft.borrow_mut().locale = v.to_string();
            }
        });
    }
    list.append(&lang_row);

    // Układ klawiatury — wiersz otwierający dialog.
    let kb_row = adw::ActionRow::builder()
        .title("Układ klawiatury")
        .subtitle(&kb_label(&cur_layout, &cur_variant))
        .activatable(true)
        .build();
    kb_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let app = app.clone();
        let kb_row2 = kb_row.clone();
        kb_row.connect_activated(move |_| present_keyboard_dialog(&app, &kb_row2));
    }
    list.append(&kb_row);

    // Serwer SSH — domyślnie wyłączony.
    let ssh_row = adw::SwitchRow::builder()
        .title("Serwer SSH (sshd)")
        .subtitle("Zdalne logowanie przez SSH; domyślnie wyłączone.")
        .active(app.draft.borrow().enable_ssh)
        .build();
    {
        let app = app.clone();
        ssh_row.connect_active_notify(move |r| {
            app.draft.borrow_mut().enable_ssh = r.is_active();
        });
    }
    list.append(&ssh_row);

    let next = gtk::Button::builder()
        .label("Dalej")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    {
        let app = app.clone();
        let hostname = hostname.clone();
        next.connect_clicked(move |_| {
            let host = hostname.text().trim().to_string();
            if !valid_hostname(&host) {
                app.toast("Niepoprawna nazwa komputera (litery, cyfry i myślniki).");
                return;
            }
            app.draft.borrow_mut().hostname = host;
            app.nav.push(&user_page::build(&app));
        });
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .build();
    content.append(&list);
    content.append(&next);

    let status = adw::StatusPage::builder()
        .icon_name("preferences-system-symbolic")
        .title("System")
        .description("Nazwa komputera, strefa czasowa, język i układ klawiatury (działają też w konsoli tty).")
        .child(&adw::Clamp::builder().maximum_size(520).child(&content).build())
        .build();

    wizard_page("System", "system", &status)
}
