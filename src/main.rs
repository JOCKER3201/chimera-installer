// chimera-installer — graphical installer for Chimera Linux.
//
// One binary, two roles:
//   no arguments        -> GUI (GTK4 + libadwaita),
//   `helper` as argv1   -> the root part, launched by the GUI as
//                          `pkexec <binary> helper` with the configuration
//                          on stdin.
//
// Written entirely by an AI (Claude, Anthropic) at the user's request.

mod config;
mod disks;
mod gpu;
mod helper;
mod sysdata;
mod ui;

fn main() {
    let mut args = std::env::args();
    let _exe = args.next();
    match args.next().as_deref() {
        Some("helper") => std::process::exit(helper::main()),
        // Diagnostics: print the detected disks and the simple-mode decisions.
        Some("scan") => std::process::exit(scan_debug()),
        _ => std::process::exit(ui::run()),
    }
}

fn scan_debug() -> i32 {
    println!(
        "host boot mode: {}",
        if disks::host_is_uefi() { "UEFI" } else { "BIOS" }
    );
    for g in gpu::scan() {
        println!(
            "GPU {}: {} {}:{} — drivers: {}",
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
                    "{} — {} ({}, table: {}){}",
                    d.path,
                    if d.model.is_empty() { "(no model)" } else { &d.model },
                    config::format_gib(d.size),
                    d.pttype.as_deref().unwrap_or("none"),
                    if d.busy_system { " [RUNNING SYSTEM]" } else { "" }
                );
                for p in &d.parts {
                    println!(
                        "    {} start={} size={} type={} fs={} mnt={:?}",
                        p.path,
                        p.start,
                        config::format_gib(p.size),
                        p.parttype.as_deref().unwrap_or("-"),
                        p.fstype.as_deref().unwrap_or("-"),
                        p.mountpoints
                    );
                }
                if disks::is_empty(d) {
                    println!("    => empty disk: installation on the whole disk");
                } else {
                    match disks::largest_gap(d) {
                        Some(g) => println!(
                            "    => largest gap: start={} sectors, {}",
                            g.start,
                            config::format_gib(g.bytes())
                        ),
                        None => println!("    => no free space"),
                    }
                    if let Some(esp) = disks::find_esp(d) {
                        println!("    => existing ESP: {esp}");
                    }
                }
            }
            0
        }
        Err(e) => {
            eprintln!("scan error: {e}");
            1
        }
    }
}
