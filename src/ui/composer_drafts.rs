use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Scope {
    account: Option<String>,
    chat: Option<String>,
    generation: u64,
}

#[derive(Default)]
pub(super) struct Drafts {
    scope: Scope,
    paths: Vec<PathBuf>,
}

impl Drafts {
    fn bind(&mut self, account: Option<String>, chat: Option<String>) -> bool {
        if self.scope.account != account || self.scope.chat != chat {
            self.clear();
            self.scope.account = account;
            self.scope.chat = chat;
            return true;
        }
        false
    }

    fn clear(&mut self) {
        self.paths.clear();
        self.scope.generation = self.scope.generation.wrapping_add(1);
    }

    fn stage(&mut self, scope: &Scope, path: PathBuf) -> bool {
        if self.scope != *scope || scope.chat.is_none() {
            return false;
        }
        self.paths.push(path);
        true
    }
}

fn confirm(paths: &mut Vec<PathBuf>, mut send: impl FnMut(&PathBuf) -> bool) {
    paths.retain(|path| !send(path));
}

pub(super) fn sync(state: &AppState) {
    let changed = state.composer_drafts.borrow_mut().bind(
        state.self_mid.borrow().clone(),
        state.current_chat.borrow().clone(),
    );
    if changed {
        refresh(state);
    }
}

pub(super) fn scope(state: &AppState) -> Scope {
    sync(state);
    state.composer_drafts.borrow().scope.clone()
}

pub(super) fn stage(state: &AppState, scope: &Scope, path: PathBuf) {
    sync(state);
    if *state.session_ready.borrow() && state.composer_drafts.borrow_mut().stage(scope, path) {
        refresh(state);
    }
}

pub(super) fn cancel(state: &AppState) {
    state.composer_drafts.borrow_mut().clear();
    refresh(state);
}

/// Returns false when attachments must wait for the session to recover.
pub(super) fn send(state: &AppState) -> bool {
    sync(state);
    if state.composer_drafts.borrow().paths.is_empty() {
        return true;
    }
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return false;
    }
    // Sending can rebuild the message view and sync the draft scope. Do not
    // hold a RefCell borrow across that work.
    let scope = state.composer_drafts.borrow().scope.clone();
    let mut paths = std::mem::take(&mut state.composer_drafts.borrow_mut().paths);
    confirm(&mut paths, |path| {
        try_send_local_media_path(state, path.clone())
    });
    if state.composer_drafts.borrow().scope == scope {
        state.composer_drafts.borrow_mut().paths.splice(0..0, paths);
    }
    let sent = state.composer_drafts.borrow().paths.is_empty();
    refresh(state);
    sent
}

pub(super) fn refresh(state: &AppState) {
    let preview = &state.attachment_preview;
    while let Some(child) = preview.first_child() {
        preview.remove(&child);
    }
    let paths = state.composer_drafts.borrow().paths.clone();
    preview.set_visible(!paths.is_empty());
    if paths.is_empty() {
        return;
    }
    let hint = gtk::Label::builder()
        .label(crate::i18n::t("attachment_draft_hint"))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["caption", "dim-label"])
        .build();
    preview.append(&hint);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (index, path) in paths.iter().enumerate() {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .build();
        if matches!(guess_media_o_type(path), "image" | "gif")
            && let Ok(pixbuf) = Pixbuf::from_file_at_scale(path, 112, 80, true)
        {
            let picture = gtk::Picture::for_paintable(&gdk::Texture::for_pixbuf(&pixbuf));
            picture.set_size_request(112, 80);
            picture.set_alternative_text(Some(&crate::i18n::t("preview_photo")));
            card.append(&picture);
        } else {
            let icon = gtk::Image::from_icon_name("mail-attachment-symbolic");
            icon.set_size_request(112, 80);
            card.append(&icon);
        }
        let details = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let label = gtk::Label::builder()
            .label(name.as_ref())
            .tooltip_text(name.as_ref())
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(12)
            .hexpand(true)
            .css_classes(["caption"])
            .build();
        let remove_label = crate::i18n::t("attachment_remove");
        let remove = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text(&remove_label)
            .css_classes(["flat", "circular"])
            .build();
        remove.update_property(&[gtk::accessible::Property::Label(&remove_label)]);
        let s = state.clone();
        remove.connect_clicked(move |_| {
            s.composer_drafts.borrow_mut().paths.remove(index);
            refresh(&s);
            s.composer.grab_focus();
        });
        details.append(&label);
        details.append(&remove);
        card.append(&details);
        row.append(&card);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&row)
        .build();
    preview.append(&scroll);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> Drafts {
        let mut draft = Drafts::default();
        draft.bind(Some("account-a".into()), Some("chat-a".into()));
        draft
    }

    #[test]
    fn paste_waits_for_confirmation_and_retains_failed_sends() {
        let mut draft = draft();
        let scope = draft.scope.clone();
        assert!(draft.stage(&scope, "one.png".into()));
        assert!(draft.stage(&scope, "two.png".into()));
        assert_eq!(draft.paths.len(), 2);
        let mut sent = Vec::new();
        confirm(&mut draft.paths, |path| {
            sent.push(path.clone());
            path == &PathBuf::from("one.png")
        });
        assert_eq!(sent, vec![PathBuf::from("one.png"), "two.png".into()]);
        assert_eq!(draft.paths, vec![PathBuf::from("two.png")]);
        confirm(&mut draft.paths, |_| true);
        assert!(draft.paths.is_empty());
    }

    #[test]
    fn cancel_discards_draft_and_in_flight_pastes() {
        let mut draft = draft();
        let scope = draft.scope.clone();
        draft.stage(&scope, "one.png".into());
        draft.clear();
        assert!(!draft.stage(&scope, "late.png".into()));
        confirm(&mut draft.paths, |_| {
            panic!("cancelled attachments must not send")
        });
    }

    #[test]
    fn chat_and_account_changes_discard_drafts_and_stale_reads() {
        for (account, chat) in [
            (Some("account-a"), Some("chat-b")),
            (Some("account-b"), Some("chat-a")),
            (Some("account-a"), None),
        ] {
            let mut draft = draft();
            let scope = draft.scope.clone();
            draft.stage(&scope, "one.png".into());
            draft.bind(account.map(str::to_owned), chat.map(str::to_owned));
            assert!(draft.paths.is_empty());
            assert!(!draft.stage(&scope, "late.png".into()));
            draft.bind(scope.account.clone(), scope.chat.clone());
            assert!(!draft.stage(&scope, "late-after-return.png".into()));
        }
    }
}
