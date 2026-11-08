// Listy wyboru dla strony „System": strefy czasowe (z /usr/share/zoneinfo),
// języki (LANG) oraz układy/warianty klawiatury (z xkb evdev.lst hosta).
// Wszystko czytane po stronie GUI (bez roota).

use std::fs;
use std::path::{Path, PathBuf};

const ZONEINFO: &str = "/usr/share/zoneinfo";

/// Kontynenty/oceany pokazywane jako pierwszy poziom wyboru strefy.
const TZ_REGIONS: &[&str] = &[
    "Africa", "America", "Antarctica", "Arctic", "Asia", "Atlantic", "Australia", "Europe",
    "Indian", "Pacific",
];

/// Regiony stref czasowych faktycznie obecne na hoście (zachowana kolejność).
pub fn tz_regions() -> Vec<String> {
    TZ_REGIONS
        .iter()
        .filter(|r| Path::new(ZONEINFO).join(r).is_dir())
        .map(|r| r.to_string())
        .collect()
}

fn collect_zones(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let p = e.path();
        if p.is_dir() {
            collect_zones(&p, &rel, out);
        } else {
            out.push(rel);
        }
    }
}

/// Strefy (miasta) w regionie, np. "Warsaw" albo "Argentina/Buenos_Aires".
pub fn tz_cities(region: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_zones(&Path::new(ZONEINFO).join(region), "", &mut out);
    out.sort();
    out
}

/// Pełna nazwa strefy, np. "Europe/Warsaw".
pub fn tz_full(region: &str, city: &str) -> String {
    format!("{region}/{city}")
}

/// Lista języków (nazwa do wyświetlenia, wartość LANG). Na górze języki
/// z pełnym wsparciem libc musl (musl-locales); pozostałe działają przez
/// tłumaczenia aplikacji (gettext), ale komunikaty musl zostają angielskie.
pub fn locales() -> Vec<(&'static str, &'static str)> {
    vec![
        ("English (US)", "en_US.UTF-8"),
        ("English (UK)", "en_GB.UTF-8"),
        ("Deutsch (Deutschland)", "de_DE.UTF-8"),
        ("Deutsch (Schweiz)", "de_CH.UTF-8"),
        ("Español", "es_ES.UTF-8"),
        ("Français", "fr_FR.UTF-8"),
        ("Italiano", "it_IT.UTF-8"),
        ("Čeština", "cs_CZ.UTF-8"),
        ("Русский", "ru_RU.UTF-8"),
        ("Português (Brasil)", "pt_BR.UTF-8"),
        ("Português (Portugal)", "pt_PT.UTF-8"),
        ("Nederlands", "nl_NL.UTF-8"),
        ("Suomi", "fi_FI.UTF-8"),
        ("Svenska", "sv_SE.UTF-8"),
        ("Norsk bokmål", "nb_NO.UTF-8"),
        ("Norsk nynorsk", "nn_NO.UTF-8"),
        ("Srpski", "sr_RS.UTF-8"),
        // Poniżej: tłumaczenia aplikacji, libc musl pozostaje po angielsku.
        ("Polski", "pl_PL.UTF-8"),
        ("C (POSIX)", "C.UTF-8"),
    ]
}

fn evdev_lst() -> Option<String> {
    for p in [
        "/usr/share/X11/xkb/rules/evdev.lst",
        "/usr/share/X11/xkb/rules/base.lst",
    ] {
        if let Ok(s) = fs::read_to_string(p) {
            return Some(s);
        }
    }
    None
}

/// Parsuje sekcję "! <name>" z evdev.lst na pary (kod, opis).
fn parse_section(lst: &str, name: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_sec = false;
    for line in lst.lines() {
        if line.starts_with('!') {
            in_sec = line.trim_start_matches('!').trim() == name;
            continue;
        }
        if !in_sec {
            continue;
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some((code, desc)) = t.split_once(char::is_whitespace) {
            out.push((code.to_string(), desc.trim().to_string()));
        }
    }
    out
}

/// Fallback, gdy host nie ma xkb rules — garść najczęstszych układów.
fn fallback_layouts() -> Vec<(String, String)> {
    [
        ("us", "English (US)"),
        ("gb", "English (UK)"),
        ("de", "German"),
        ("fr", "French"),
        ("pl", "Polish"),
        ("es", "Spanish"),
        ("it", "Italian"),
        ("ru", "Russian"),
        ("cz", "Czech"),
        ("se", "Swedish"),
    ]
    .iter()
    .map(|(c, d)| (c.to_string(), d.to_string()))
    .collect()
}

/// Układy klawiatury (kod xkb, opis), posortowane po opisie.
pub fn kb_layouts() -> Vec<(String, String)> {
    let mut v = match evdev_lst() {
        Some(lst) => parse_section(&lst, "layout"),
        None => fallback_layouts(),
    };
    if v.is_empty() {
        v = fallback_layouts();
    }
    v.sort_by(|a, b| a.1.cmp(&b.1));
    v
}

/// Warianty dla danego układu (kod, opis). Pierwszy zawsze „domyślny" (pusty).
pub fn kb_variants(layout: &str) -> Vec<(String, String)> {
    let mut out = vec![(String::new(), "domyślny".to_string())];
    if let Some(lst) = evdev_lst() {
        let prefix = format!("{layout}:");
        for (code, desc) in parse_section(&lst, "variant") {
            // Opis wariantu ma postać "<layout>: Opis".
            if let Some(rest) = desc.strip_prefix(&prefix) {
                out.push((code, rest.trim().to_string()));
            }
        }
    }
    out
}

/// Domyślna strefa wykryta z hosta (z dowiązania /etc/localtime).
pub fn host_timezone() -> String {
    fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p: PathBuf| {
            let s = p.to_string_lossy().to_string();
            s.split("zoneinfo/").nth(1).map(|z| z.to_string())
        })
        .unwrap_or_else(|| "Europe/Warsaw".to_string())
}
