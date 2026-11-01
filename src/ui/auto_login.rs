use super::*;
use crate::credentials::{self, Action, Credentials};

pub(super) struct State {
    startup_pending: bool,
    keyring_used: bool,
    generation: u64,
    save_after_login: Option<Action>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            startup_pending: true,
            keyring_used: false,
            generation: 0,
            save_after_login: None,
        }
    }
}
impl State {
    fn cancel(&mut self) {
        self.startup_pending = false;
        self.generation = self.generation.wrapping_add(1);
        self.save_after_login = None;
    }
    fn take_startup(&mut self) -> bool {
        std::mem::replace(&mut self.startup_pending, false)
    }
}

pub(super) fn ready(state: &AppState, has_auth: bool, restarting: bool) -> bool {
    if restarting || has_auth {
        return false;
    }
    let eligible = state.auto_login.borrow_mut().take_startup();
    eligible && begin(state)
}

pub(super) fn restore_failed(state: &AppState, error: &str) -> bool {
    let eligible = state.auto_login.borrow_mut().take_startup();
    eligible && authentication_expired(error) && begin(state)
}

fn authentication_expired(error: &str) -> bool {
    let error = error.to_ascii_uppercase();
    [
        "NOT_AUTHORIZED",
        "LOGGED_OUT",
        "AUTHENTICATION",
        "INVALID_TOKEN",
        "TOKEN_EXPIRED",
        "NO_AUTH_TOKEN",
    ]
    .iter()
    .any(|code| error.contains(code))
}

fn begin(state: &AppState) -> bool {
    if !state.config.borrow().startup_auto_login {
        return false;
    }
    state.auto_login.borrow_mut().keyring_used = true;
    login::set_method(&state.login, login::LoginMethod::Email);
    show_login(state, &crate::i18n::t("auto_login_loading"));
    login::set_busy(&state.login, true);
    let generation = state.auto_login.borrow().generation;
    let response = credentials::request(Action::Load);
    let s = state.clone();
    glib::spawn_future_local(async move {
        let result = response.recv().await;
        if s.auto_login.borrow().generation != generation || *s.session_ready.borrow() {
            return;
        }
        match result {
            Ok(Ok(Some(credentials))) if s.config.borrow().startup_auto_login => {
                s.login.email_entry.set_text(&credentials.email);
                s.login
                    .hint
                    .set_text(&crate::i18n::t("auto_login_signing_in"));
                match s
                    .sidecar
                    .login_email(&credentials.email, &credentials.password)
                {
                    Ok(id) => {
                        s.pending.borrow_mut().insert(id, Pending::LoginEmail);
                    }
                    Err(_) => login::show_failure(&s.login, &crate::i18n::t("login_email_failed")),
                }
            }
            _ => login::show_failure(&s.login, &crate::i18n::t("auto_login_load_failed")),
        }
    });
    true
}

pub(super) fn manual_restart(state: &AppState) {
    state.auto_login.borrow_mut().cancel();
    if state.config.borrow().startup_auto_login {
        state.auto_login.borrow_mut().save_after_login = Some(Action::Clear);
    }
}

pub(super) fn prepare_email(state: &AppState, email: &str, password: &str) {
    state.auto_login.borrow_mut().cancel();
    state.auto_login.borrow_mut().save_after_login = if state.login.remember_login.is_active() {
        Some(Action::Save(Credentials {
            email: email.trim().into(),
            password: password.into(),
        }))
    } else if state.config.borrow().startup_auto_login {
        Some(Action::Clear)
    } else {
        None
    };
}

pub(super) fn failed(state: &AppState) {
    state.auto_login.borrow_mut().cancel();
}

fn enabled(state: &AppState, on: bool) {
    state.config.borrow_mut().startup_auto_login = on;
    state.config.borrow().save(&state.data_dir);
    state.login.remember_login.set_active(on);
}

pub(super) fn logged_in(state: &AppState) {
    let action = {
        let mut auto = state.auto_login.borrow_mut();
        auto.startup_pending = false;
        auto.save_after_login.take()
    };
    let Some(action) = action else {
        return;
    };
    let saving = matches!(action, Action::Save(_));
    state.auto_login.borrow_mut().keyring_used = true;
    enabled(state, false);
    let generation = state.auto_login.borrow().generation;
    let response = credentials::request(action);
    let s = state.clone();
    glib::spawn_future_local(async move {
        let success = matches!(response.recv().await, Ok(Ok(_)));
        if s.auto_login.borrow().generation != generation {
            return;
        }
        if success {
            enabled(&s, saving);
            s.auto_login.borrow_mut().keyring_used = saving;
        }
        if saving || !success {
            toast(
                &s,
                &crate::i18n::t(if success {
                    "auto_login_saved"
                } else {
                    "auto_login_save_failed"
                }),
            );
        }
    });
}

pub(super) fn manual_logout(state: &AppState) {
    state.auto_login.borrow_mut().cancel();
    let was_enabled =
        state.config.borrow().startup_auto_login || state.auto_login.borrow().keyring_used;
    enabled(state, false);
    // 手動ログアウトでは次回起動時にも自動で入り直さない。
    if was_enabled {
        let response = credentials::request(Action::Clear);
        let s = state.clone();
        let generation = state.auto_login.borrow().generation;
        glib::spawn_future_local(async move {
            let success = matches!(response.recv().await, Ok(Ok(_)));
            if s.auto_login.borrow().generation != generation {
                return;
            }
            if success {
                s.auto_login.borrow_mut().keyring_used = false;
            } else {
                toast(&s, &crate::i18n::t("auto_login_save_failed"));
            }
        });
    }
}

pub(super) fn open_settings(state: &AppState, parent: &gtk::Window) {
    let win = gtk::Window::builder()
        .title(crate::i18n::t("auto_login_title"))
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .default_width(440)
        .build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    let note = gtk::Label::builder()
        .label(crate::i18n::t("auto_login_note"))
        .wrap(true)
        .max_width_chars(44)
        .xalign(0.0)
        .build();
    let email = gtk::Entry::builder()
        .placeholder_text(crate::i18n::t("login_email_address"))
        .tooltip_text(crate::i18n::t("login_email_address"))
        .input_purpose(gtk::InputPurpose::Email)
        .build();
    let password = gtk::PasswordEntry::builder()
        .placeholder_text(crate::i18n::t("login_password"))
        .tooltip_text(crate::i18n::t("login_password"))
        .show_peek_icon(true)
        .build();
    let status = gtk::Label::builder()
        .label(crate::i18n::t(
            if state.config.borrow().startup_auto_login {
                "auto_login_enabled"
            } else {
                "auto_login_disabled"
            },
        ))
        .wrap(true)
        .xalign(0.0)
        .build();
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let save = gtk::Button::with_label(&crate::i18n::t("auto_login_save"));
    save.add_css_class("suggested-action");
    let clear = gtk::Button::with_label(&crate::i18n::t("auto_login_forget"));
    clear.add_css_class("destructive-action");
    buttons.append(&save);
    buttons.append(&clear);
    for widget in [
        note.upcast_ref::<gtk::Widget>(),
        email.upcast_ref(),
        password.upcast_ref(),
        status.upcast_ref(),
        buttons.upcast_ref(),
    ] {
        body.append(widget);
    }
    win.set_child(Some(&body));
    for (button, saving) in [(save.clone(), true), (clear.clone(), false)] {
        let s = state.clone();
        let e = email.clone();
        let p = password.clone();
        let label = status.clone();
        let controls = buttons.clone();
        button.connect_clicked(move |_| {
            let action = if saving {
                let credentials = Credentials {
                    email: e.text().trim().into(),
                    password: p.text().into(),
                };
                if !credentials.valid() {
                    label.set_text(&crate::i18n::t("login_email_missing"));
                    return;
                }
                Action::Save(credentials)
            } else {
                Action::Clear
            };
            s.auto_login.borrow_mut().cancel();
            let generation = s.auto_login.borrow().generation;
            s.auto_login.borrow_mut().keyring_used = true;
            enabled(&s, false);
            p.set_text("");
            controls.set_sensitive(false);
            label.set_text(&crate::i18n::t("auto_login_loading"));
            let response = credentials::request(action);
            let s = s.clone();
            let label = label.clone();
            let controls = controls.clone();
            glib::spawn_future_local(async move {
                let success = matches!(response.recv().await, Ok(Ok(_)));
                if s.auto_login.borrow().generation != generation {
                    return;
                }
                controls.set_sensitive(true);
                if success {
                    enabled(&s, saving);
                    s.auto_login.borrow_mut().keyring_used = saving;
                }
                label.set_text(&crate::i18n::t(if success {
                    if saving {
                        "auto_login_saved"
                    } else {
                        "auto_login_forgotten"
                    }
                } else {
                    "auto_login_save_failed"
                }));
            });
        });
    }
    win.present();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_is_consumed_once_and_never_rearmed_after_success_or_cancellation() {
        let mut state = State::default();
        assert!(state.take_startup());
        assert!(!state.take_startup());
        state.cancel();
        assert!(!state.take_startup());
        let generation = state.generation;
        state.cancel();
        assert_ne!(state.generation, generation);
    }
    #[test]
    fn temporary_network_errors_do_not_trigger_password_login() {
        for error in [
            "V3_TOKEN_CLIENT_LOGGED_OUT",
            "NOT_AUTHORIZED_DEVICE",
            "AUTHENTICATION_FAILED",
            "INVALID_TOKEN",
        ] {
            assert!(authentication_expired(error));
        }
        for error in [
            "fetch failed",
            "connection reset",
            "timeout",
            "E2EEKeyMismatch",
        ] {
            assert!(!authentication_expired(error));
        }
    }
}
