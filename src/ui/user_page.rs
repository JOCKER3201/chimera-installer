// The user and root account page.

use super::{summary_page, wizard_page, App};
use adw::prelude::*;
use std::rc::Rc;

fn valid_username(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn build(app: &Rc<App>) -> adw::NavigationPage {
    let user_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let username = adw::EntryRow::builder()
        .title("Username (wheel group)")
        .build();
    let pass1 = adw::PasswordEntryRow::builder().title("Password").build();
    let pass2 = adw::PasswordEntryRow::builder().title("Repeat password").build();
    user_list.append(&username);
    user_list.append(&pass1);
    user_list.append(&pass2);

    let root_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let root1 = adw::PasswordEntryRow::builder()
        .title("Root password (empty = account locked)")
        .build();
    let root2 = adw::PasswordEntryRow::builder()
        .title("Repeat root password")
        .build();
    root_list.append(&root1);
    root_list.append(&root2);

    let next = gtk::Button::builder()
        .label("Next")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();

    {
        let app = app.clone();
        let (username, pass1, pass2) = (username.clone(), pass1.clone(), pass2.clone());
        let (root1, root2) = (root1.clone(), root2.clone());
        next.connect_clicked(move |_| {
            let user = username.text().trim().to_string();
            if !valid_username(&user) {
                app.toast("Invalid username (lowercase letters, digits, _ and -).");
                return;
            }
            if pass1.text().is_empty() {
                app.toast("The user password cannot be empty.");
                return;
            }
            if pass1.text() != pass2.text() {
                app.toast("The user passwords do not match.");
                return;
            }
            if root1.text() != root2.text() {
                app.toast("The root passwords do not match.");
                return;
            }
            {
                let mut d = app.draft.borrow_mut();
                d.username = user;
                d.user_password = pass1.text().to_string();
                d.root_password = if root1.text().is_empty() {
                    None
                } else {
                    Some(root1.text().to_string())
                };
            }
            app.nav.push(&summary_page::build(&app));
        });
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .build();
    content.append(&user_list);
    content.append(&root_list);
    content.append(&next);

    let status = adw::StatusPage::builder()
        .icon_name("system-users-symbolic")
        .title("User")
        .description("The account will be added to the wheel group (administrative rights via doas).")
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&content)
                .build(),
        )
        .build();

    wizard_page("User", "user", &status)
}
