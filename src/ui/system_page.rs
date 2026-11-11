// The "System" page: computer name, time zone (continent→zone dialog),
// language and keyboard layout (layout + variant dialog).

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

/// Keyboard layout description for the row subtitle.
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
// Time zone dialog: continent → zone
// ---------------------------------------------------------------------------

fn present_timezone_dialog(app: &Rc<App>, row: &adw::ActionRow) {
    let regions = sysdata::tz_regions();
    let region_refs: Vec<&str> = regions.iter().map(String::as_str).collect();

    let region_row = adw::ComboRow::builder()
        .title("Continent")
        .model(&gtk::StringList::new(&region_refs))
        .build();
    let city_row = adw::ComboRow::builder().title("Zone").build();

    // The current zone from the draft, e.g. "Europe/Warsaw".
    let cur_tz = app.draft.borrow().timezone.clone();
    let (cur_region, cur_city) = cur_tz.split_once('/').unwrap_or(("Europe", "Warsaw"));
    let region_start = regions.iter().position(|r| r == cur_region).unwrap_or(0);
    region_row.set_selected(region_start as u32);

    // Cities depend on the continent (values: "Warsaw", "Argentina/Buenos_Aires").
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
        .label("Choose")
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
        .title("Time zone")
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
// Keyboard dialog: layout + variant
// ---------------------------------------------------------------------------

fn present_keyboard_dialog(app: &Rc<App>, row: &adw::ActionRow) {
    let layouts = sysdata::kb_layouts();
    let labels: Vec<String> = layouts.iter().map(|(c, d)| format!("{d} ({c})")).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let layout_model = gtk::StringList::new(&label_refs);

    let layout_row = adw::ComboRow::builder()
        .title("Layout")
        .model(&layout_model)
        .build();
    let variant_row = adw::ComboRow::builder().title("Variant").build();

    // Initial selection from the current draft value.
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
        .label("Choose")
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
        .title("Keyboard layout")
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
// Page
// ---------------------------------------------------------------------------

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    // Initialize the draft with defaults (on first entry).
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

    let hostname = adw::EntryRow::builder().title("Computer name").text(&cur_host).build();
    list.append(&hostname);

    // Time zone — a row opening the dialog.
    let tz_row = adw::ActionRow::builder()
        .title("Time zone")
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

    // Language — a dropdown list.
    let locales = sysdata::locales();
    let loc_labels: Vec<&str> = locales.iter().map(|(n, _)| *n).collect();
    let lang_row = adw::ComboRow::builder()
        .title("Language")
        .subtitle("Full base system translations only for some languages (musl)")
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

    // Keyboard layout — a row opening the dialog.
    let kb_row = adw::ActionRow::builder()
        .title("Keyboard layout")
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

    // SSH server — disabled by default.
    let ssh_row = adw::SwitchRow::builder()
        .title("SSH server (sshd)")
        .subtitle("Remote login over SSH; disabled by default.")
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
        .label("Next")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    {
        let app = app.clone();
        let hostname = hostname.clone();
        next.connect_clicked(move |_| {
            let host = hostname.text().trim().to_string();
            if !valid_hostname(&host) {
                app.toast("Invalid computer name (letters, digits and hyphens).");
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
        .description("Computer name, time zone, language and keyboard layout (they apply in the tty console too).")
        .child(&adw::Clamp::builder().maximum_size(520).child(&content).build())
        .build();

    wizard_page("System", "system", &status)
}
