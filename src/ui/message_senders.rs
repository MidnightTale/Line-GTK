use super::*;

pub(super) fn is_group(state: &AppState) -> bool {
    state
        .current_chat
        .borrow()
        .as_deref()
        .is_some_and(|mid| is_group_chat(state, mid))
}

pub(super) fn is_group_chat(state: &AppState, mid: &str) -> bool {
    mid.starts_with(['c', 'r'])
        || state
            .chats
            .borrow()
            .iter()
            .any(|chat| chat.mid == mid && matches!(chat.kind.as_str(), "group" | "room"))
}

pub(super) fn known_name(state: &AppState, mid: &str) -> Option<String> {
    state.sender_names.borrow().get(mid).cloned().or_else(|| {
        state
            .chats
            .borrow()
            .iter()
            .find(|chat| chat.mid == mid && chat.name != mid)
            .map(|chat| chat.name.clone())
    })
}

pub(super) fn name(state: &AppState, mid: &str) -> String {
    known_name(state, mid).unwrap_or_else(|| {
        if mid.is_empty() {
            crate::i18n::t("message_sender_unknown")
        } else {
            let suffix: String = mid
                .chars()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            crate::i18n::tf("message_sender_pending", &[("id", &suffix)])
        }
    })
}

pub(super) fn register(state: &AppState, mid: &str, label: &gtk::Label) {
    if !mid.is_empty() {
        state
            .sender_labels
            .borrow_mut()
            .entry(mid.to_string())
            .or_default()
            .push(label.clone());
    }
}

pub(super) fn apply(state: &AppState, mid: String, name: String) {
    if !*state.session_ready.borrow() || mid.is_empty() || name.trim().is_empty() || mid == name {
        return;
    }
    state
        .sender_names
        .borrow_mut()
        .insert(mid.clone(), name.clone());
    if let Some(labels) = state.sender_labels.borrow().get(&mid) {
        for label in labels {
            label.set_text(&name);
            label.set_tooltip_text(Some(&name));
        }
    }
    super::notifications::refresh_notification_sender(state, &mid);
    replies::refresh(state);
    chat_events::refresh(state);
}
