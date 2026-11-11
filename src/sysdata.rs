// Choice lists for the "System" page: time zones (from /usr/share/zoneinfo),
// languages (LANG) and keyboard layouts/variants (from the host's xkb
// evdev.lst). All read on the GUI side (no root).

use std::fs;
use std::path::{Path, PathBuf};

const ZONEINFO: &str = "/usr/share/zoneinfo";

/// Continents/oceans shown as the first level of the zone choice.
const TZ_REGIONS: &[&str] = &[
    "Africa", "America", "Antarctica", "Arctic", "Asia", "Atlantic", "Australia", "Europe",
    "Indian", "Pacific",
];

/// Time zone regions actually present on the host (order preserved).
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

/// Zones (cities) within a region, e.g. "Warsaw" or "Argentina/Buenos_Aires".
pub fn tz_cities(region: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_zones(&Path::new(ZONEINFO).join(region), "", &mut out);
    out.sort();
    out
}

/// Full zone name, e.g. "Europe/Warsaw".
pub fn tz_full(region: &str, city: &str) -> String {
    format!("{region}/{city}")
}

/// Language list (display name, LANG value). Languages with full musl libc
/// support (musl-locales) come first; the rest work through application
/// translations (gettext), but musl's own messages stay English.
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
        // Below: application translations only; musl libc stays English.
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

/// Parses a "! <name>" section of evdev.lst into (code, description) pairs.
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

/// Fallback for hosts without xkb rules — a handful of the most common layouts.
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

/// Keyboard layouts (xkb code, description), sorted by description.
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

/// Variants for a given layout (code, description). The first is always
/// "default" (empty code).
pub fn kb_variants(layout: &str) -> Vec<(String, String)> {
    let mut out = vec![(String::new(), "default".to_string())];
    if let Some(lst) = evdev_lst() {
        let prefix = format!("{layout}:");
        for (code, desc) in parse_section(&lst, "variant") {
            // A variant description has the form "<layout>: Description".
            if let Some(rest) = desc.strip_prefix(&prefix) {
                out.push((code, rest.trim().to_string()));
            }
        }
    }
    out
}

/// Default zone detected from the host (from the /etc/localtime symlink).
pub fn host_timezone() -> String {
    fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p: PathBuf| {
            let s = p.to_string_lossy().to_string();
            s.split("zoneinfo/").nth(1).map(|z| z.to_string())
        })
        .unwrap_or_else(|| "Europe/Warsaw".to_string())
}
