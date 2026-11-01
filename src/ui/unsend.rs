use super::*;

pub(super) fn eligible(message: &MessageInfo) -> bool {
    message.mine
        && message.id.parse::<u128>().is_ok_and(|id| id > 0)
        && message.chat_event.is_none()
        && !["CALL", "CHATEVENT", "CHAT_EVENT", "UNSENT"]
            .contains(&message.content_type.to_ascii_uppercase().as_str())
}

pub(super) fn masked(state: &AppState, message: &MessageInfo) -> MessageInfo {
    let account = state.self_mid.borrow().clone().unwrap_or_default();
    let ids = state.unsent_ids.borrow();
    if ids.contains(&(account.clone(), message.id.clone())) || message.content_type == "UNSENT" {
        return MessageInfo {
            text: String::new(),
            content_type: "UNSENT".into(),
            image_path: None,
            audio_path: None,
            file_path: None,
            file_name: None,
            flex: None,
            call_history: None,
            reply_to: None,
            chat_event: None,
            emojis: Vec::new(),
            mentions: Vec::new(),
            reactions: Vec::new(),
            decryption_failed: false,
            duration_ms: None,
            ..message.clone()
        };
    }
    let mut message = message.clone();
    if let Some(quote) = &mut message.reply_to
        && ids.contains(&(account, quote.message_id.clone()))
    {
        quote.text.clear();
        quote.content_type = "UNAVAILABLE".into();
    }
    message
}

pub(super) fn placeholder(row: &gtk::Box) {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    row.set_halign(gtk::Align::Fill);
    let label = gtk::Label::builder()
        .label(crate::i18n::t("message_unsent"))
        .halign(gtk::Align::Center)
        .hexpand(true)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .margin_top(8)
        .margin_bottom(8)
        .build();
    row.append(&label);
}

pub(super) fn apply(state: &AppState, account: &str, chat_mid: &str, id: &str) {
    if !*state.session_ready.borrow()
        || state.self_mid.borrow().as_deref() != Some(account)
        || !id.parse::<u128>().is_ok_and(|id| id > 0)
    {
        return;
    }
    state
        .unsent_ids
        .borrow_mut()
        .insert((account.into(), id.into()));
    state.notif_pending.borrow_mut().remove(id);
    let playing = state
        .voice_playback
        .borrow()
        .as_ref()
        .is_some_and(|p| p.msg_id == id);
    if playing {
        messages::stop_voice_playback(state);
    }
    if state.current_chat.borrow().as_deref() != Some(chat_mid) {
        return;
    }
    let original = state.reply_messages.borrow().get(id).cloned();
    if let Some(original) = original {
        state
            .reply_messages
            .borrow_mut()
            .insert(id.into(), masked(state, &original));
    }
    if let Some(row) = state.message_rows.borrow().get(id) {
        placeholder(row);
    }
    state.media_slots.borrow_mut().remove(id);
    state.media_msgs.borrow_mut().remove(id);
    state.media_ready_paths.borrow_mut().remove(id);
    state.receipt_slots.borrow_mut().remove(id);
    state.msg_created.borrow_mut().remove(id);
    reactions::forget(state, id);
    let incoming = state
        .reply_messages
        .borrow()
        .values()
        .filter(|m| !m.mine && m.chat_event.is_none() && m.content_type != "UNSENT")
        .max_by_key(|m| m.created_time)
        .map(|m| m.id.clone());
    *state.last_incoming_id.borrow_mut() = incoming;
    replies::redact(state, id);
    *state.msg_list_fp.borrow_mut() = None;
    if let Some(chat) = state.current_chat.borrow().as_deref() {
        chat_snapshots::forget(state, chat);
    }
    state.message_list.queue_allocate();
}

fn pending(state: &AppState, account: &str, id: &str) -> bool {
    state.pending.borrow().values().any(|p| matches!(p, Pending::Unsend { account: a, message_id, .. } if a == account && message_id == id))
}

pub(super) fn wire(state: &AppState) {
    let action = gio::SimpleAction::new("unsend-message", Some(glib::VariantTy::STRING));
    let s = state.clone();
    action.connect_activate(move |_, value| {
        let Some(id) = value.and_then(|v| v.str()) else { return; };
        let Some(account) = s.self_mid.borrow().clone() else { return; };
        let Some(chat_mid) = s.current_chat.borrow().clone() else { return; };
        if !*s.session_ready.borrow() || pending(&s, &account, id) { return; }
        let Some(message) = s.reply_messages.borrow().get(id).filter(|m| eligible(m)).cloned() else { return; };
        let dialog = libadwaita::AlertDialog::new(Some(&crate::i18n::t("unsend_confirm")), Some(&crate::i18n::t("unsend_body")));
        dialog.add_response("cancel", &crate::i18n::t("unsend_cancel"));
        dialog.add_response("unsend", &crate::i18n::t("unsend_message"));
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        dialog.set_response_appearance("unsend", libadwaita::ResponseAppearance::Destructive);
        let s = s.clone();
        let parent = s.window.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "unsend" || !*s.session_ready.borrow() || s.self_mid.borrow().as_deref() != Some(&account) || s.current_chat.borrow().as_deref() != Some(&chat_mid) || pending(&s, &account, &message.id) { return; }
            if !s.reply_messages.borrow().get(&message.id).is_some_and(eligible) { return; }
            match s.sidecar.request("unsend_message", Some(serde_json::json!({"chatMid":chat_mid,"messageId":message.id,"createdTime":message.created_time}))) {
                Ok(request) => { s.pending.borrow_mut().insert(request, Pending::Unsend { account: account.clone(), chat_mid: chat_mid.clone(), message_id: message.id.clone() }); }
                Err(_) => toast(&s, &crate::i18n::t("unsend_failed")),
            }
        });
        dialog.present(Some(&parent));
    });
    state.window.add_action(&action);
}

pub(super) fn response(state: &AppState, account: &str, chat_mid: &str, id: &str, ok: bool) {
    if state.self_mid.borrow().as_deref() != Some(account) || !*state.session_ready.borrow() {
        return;
    }
    if ok {
        apply(state, account, chat_mid, id);
    } else {
        toast(state, &crate::i18n::t("unsend_failed"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_confirmed_own_messages_can_be_unsent() {
        let mut message: MessageInfo = serde_json::from_value(
            serde_json::json!({"id":"9007199254740993","mine":true,"contentType":"IMAGE"}),
        )
        .unwrap();
        assert!(eligible(&message));
        message.mine = false;
        assert!(!eligible(&message));
        message.mine = true;
        message.id = "pending-1".into();
        assert!(!eligible(&message));
        message.id = "10".into();
        message.content_type = "UNSENT".into();
        assert!(!eligible(&message));
        message.content_type = "CALL".into();
        assert!(!eligible(&message));
    }

    #[test]
    fn cached_chat_events_without_metadata_cannot_be_unsent() {
        for content_type in ["CHATEVENT", "CHAT_EVENT", "chatEvent", "chat_event"] {
            let message: MessageInfo = serde_json::from_value(serde_json::json!({
                "id": "42",
                "mine": true,
                "contentType": content_type,
                "text": "[CHATEVENT]"
            }))
            .unwrap();
            assert!(message.chat_event.is_none());
            assert!(!eligible(&message), "{content_type}");
        }
    }
}
