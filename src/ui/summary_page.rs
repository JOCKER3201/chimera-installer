// Summary of the installation plan and the final confirmation.

use super::{progress_page, wizard_page, App};
use crate::config::{format_gib, Desktop, Edition, InstallConfig, Mode, Source, Strategy, SECTOR};
use adw::prelude::*;
use std::rc::Rc;

fn row(title: &str, value: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder().title(title).subtitle(value).build();
    r.add_css_class("property");
    r
}

fn build_config(app: &Rc<App>) -> InstallConfig {
    let d = app.draft.borrow();
    let choice = d.choice.clone().expect("no disk choice");
    InstallConfig {
        mode: d.mode.expect("no mode"),
        source: d.source.unwrap_or(Source::Network),
        desktop: d.desktop.unwrap_or(Desktop::Base),
        edition: d.edition.unwrap_or(Edition::Full),
        gpus: d.gpus.clone(),
        disk: choice.disk.path.clone(),
        strategy: choice.strategy,
        uefi: crate::disks::host_is_uefi(),
        pttype: choice.disk.pttype.clone(),
        esp_reuse: choice.esp_reuse.clone(),
        biosboot_present: choice.biosboot_present,
        hostname: d.hostname.clone(),
        username: d.username.clone(),
        user_password: d.user_password.clone(),
        root_password: d.root_password.clone(),
        timezone: d.timezone.clone(),
        locale: if d.locale.is_empty() { "en_US.UTF-8".into() } else { d.locale.clone() },
        kb_layout: if d.kb_layout.is_empty() { "us".into() } else { d.kb_layout.clone() },
        kb_variant: d.kb_variant.clone(),
        enable_ssh: d.enable_ssh,
    }
}

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let cfg = build_config(app);

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    list.append(&row(
        "Installation mode",
        match cfg.mode {
            Mode::Live => "from a Chimera live medium (chimera-bootstrap)",
            Mode::Foreign => "from another system (base image + chroot in /tmp/chimera-linux)",
        },
    ));
    list.append(&row(
        "Package source",
        match cfg.source {
            Source::Local => "local (from the live medium)",
            Source::Network => "network (repo.chimera-linux.org)",
        },
    ));
    let ed = match cfg.edition {
        Edition::Base => "base, lightweight",
        Edition::Full => "full, complete",
    };
    list.append(&row(
        "System variant",
        &match cfg.desktop {
            Desktop::Base => "base system (console)".to_string(),
            Desktop::Gnome => format!("GNOME with GDM ({ed})"),
            Desktop::Kde => format!("KDE Plasma with SDDM ({ed})"),
        },
    ));
    if !cfg.gpus.is_empty() {
        let desc = cfg
            .gpus
            .iter()
            .map(|g| {
                let drv = if g.driver.is_empty() { "no changes" } else { &g.driver };
                if drv.starts_with("nvidia-") {
                    format!("{drv} — EXPERIMENTAL ({}:{})", g.vendor, g.device)
                } else {
                    format!("{drv} ({}:{})", g.vendor, g.device)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        list.append(&row("GPU drivers (open)", &desc));
    }
    list.append(&row("Target disk", &cfg.disk));
    match cfg.strategy {
        Strategy::Whole => list.append(&row(
            "Partitioning",
            "the WHOLE DISK will be wiped (GPT, ESP/BIOS boot + btrfs)",
        )),
        Strategy::Free { sectors, .. } => {
            let mut s = format!(
                "new partitions in the free space ({}), existing data untouched",
                format_gib(sectors * SECTOR)
            );
            if let Some(esp) = &cfg.esp_reuse {
                s.push_str(&format!("; the existing ESP {esp} will be reused"));
            }
            list.append(&row("Partitioning", &s));
        }
    }
    list.append(&row(
        "Filesystem",
        "btrfs, CachyOS layout (fixed subvol=@, @home, @log, @cache, @snapshots)",
    ));
    list.append(&row(
        "Snapshots",
        "snapper configured like openSUSE; snapshot entries in the GRUB menu",
    ));
    list.append(&row(
        "Bootloader",
        if cfg.uefi {
            "GRUB (UEFI, x86_64-efi, ESP at /boot/efi)"
        } else {
            "GRUB (BIOS, i386-pc)"
        },
    ));
    list.append(&row("Computer name", &cfg.hostname));
    list.append(&row(
        "User",
        &format!("{} (wheel); root: {}", cfg.username,
            if cfg.root_password.is_some() { "with a password" } else { "locked" }),
    ));
    list.append(&row("Time zone", &cfg.timezone));
    list.append(&row("Language", &cfg.locale));
    let kb = if cfg.kb_variant.is_empty() {
        cfg.kb_layout.clone()
    } else {
        format!("{} ({})", cfg.kb_layout, cfg.kb_variant)
    };
    list.append(&row("Keyboard layout", &kb));
    list.append(&row(
        "SSH server",
        if cfg.enable_ssh { "enabled" } else { "disabled" },
    ));

    let install = gtk::Button::builder()
        .label("Install")
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();

    {
        let app = app.clone();
        install.connect_clicked(move |_| {
            let cfg = build_config(&app);
            let body = match cfg.strategy {
                Strategy::Whole => format!(
                    "The whole disk {} will be wiped and repartitioned from scratch. \
                     This operation cannot be undone.",
                    cfg.disk
                ),
                Strategy::Free { .. } => format!(
                    "New partitions will be created in the free space on disk {}. \
                     Existing partitions will not be changed.",
                    cfg.disk
                ),
            };
            let dialog = adw::AlertDialog::new(Some("Start the installation?"), Some(&body));
            dialog.add_responses(&[("cancel", "Cancel"), ("go", "Start the installation")]);
            dialog.set_response_appearance("go", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let app2 = app.clone();
            dialog.connect_response(None, move |_, resp| {
                if resp == "go" {
                    progress_page::start(&app2, build_config(&app2));
                }
            });
            dialog.present(Some(&app.win));
        });
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .build();
    content.append(&list);
    content.append(&install);

    let status = adw::StatusPage::builder()
        .icon_name("emblem-documents-symbolic")
        .title("Summary")
        .description("Review the installation plan. Nothing touches the disk until you confirm.")
        .child(
            &adw::Clamp::builder()
                .maximum_size(560)
                .child(&content)
                .build(),
        )
        .build();

    wizard_page("Summary", "summary", &status)
}
