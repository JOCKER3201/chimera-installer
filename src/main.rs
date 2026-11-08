// chimera-installer — graficzny instalator Chimera Linux.
//
// Jedna binarka, dwie role:
//   bez argumentów      -> GUI (GTK4 + libadwaita),
//   `helper` jako argv1 -> część rootowa, uruchamiana przez GUI poleceniem
//                          `pkexec <binarka> helper` z konfiguracją na stdin.
//
// Napisane w całości przez AI (Claude, Anthropic) na zamówienie użytkownika.

mod config;
mod disks;
mod gpu;
mod helper;
mod ui;

fn main() {
    let mut args = std::env::args();
    let _exe = args.next();
    match args.next().as_deref() {
        Some("helper") => std::process::exit(helper::main()),
        // Diagnostyka: wydruk wykrytych dysków i decyzji prostego trybu.
        Some("scan") => std::process::exit(scan_debug()),
        _ => std::process::exit(ui::run()),
    }
}

fn scan_debug() -> i32 {
    println!(
        "tryb rozruchu hosta: {}",
        if disks::host_is_uefi() { "UEFI" } else { "BIOS" }
    );
    for g in gpu::scan() {
        println!(
            "GPU {}: {} {}:{} — sterowniki: {}",
            g.slot,
            g.vendor_name,
            g.vendor,
            g.device,
            g.options
                .iter()
                .map(|o| o.module)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    match disks::scan() {
        Ok(found) => {
            for d in &found {
                println!(
                    "{} — {} ({}, tablica: {}){}",
                    d.path,
                    if d.model.is_empty() { "(bez modelu)" } else { &d.model },
                    config::format_gib(d.size),
                    d.pttype.as_deref().unwrap_or("brak"),
                    if d.busy_system { " [URUCHOMIONY SYSTEM]" } else { "" }
                );
                for p in &d.parts {
                    println!(
                        "    {} start={} rozmiar={} typ={} fs={} mnt={:?}",
                        p.path,
                        p.start,
                        config::format_gib(p.size),
                        p.parttype.as_deref().unwrap_or("-"),
                        p.fstype.as_deref().unwrap_or("-"),
                        p.mountpoints
                    );
                }
                if disks::is_empty(d) {
                    println!("    => pusty dysk: instalacja na całym dysku");
                } else {
                    match disks::largest_gap(d) {
                        Some(g) => println!(
                            "    => największa luka: start={} sektorów, {}",
                            g.start,
                            config::format_gib(g.bytes())
                        ),
                        None => println!("    => brak wolnego miejsca"),
                    }
                    if let Some(esp) = disks::find_esp(d) {
                        println!("    => istniejąca ESP: {esp}");
                    }
                }
            }
            0
        }
        Err(e) => {
            eprintln!("błąd skanowania: {e}");
            1
        }
    }
}
