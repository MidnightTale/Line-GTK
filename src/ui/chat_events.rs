use super::*;
use crate::protocol::ChatEventInfo;

fn text(state: &AppState, event: &ChatEventInfo) -> String {
    let name = |mid: &str| {
        if state.self_mid.borrow().as_deref() == Some(mid) {
            crate::i18n::t("you")
        } else {
            message_senders::name(state, mid)
        }
    };
    let actor = if event.actor_mid.is_empty() {
        String::new()
    } else {
        name(&event.actor_mid)
    };
    let targets = event
        .target_mids
        .iter()
        .map(|mid| name(mid))
        .collect::<Vec<_>>()
        .join(&crate::i18n::t("chat_event_separator"));
    let actor = if matches!(event.kind.as_str(), "joined" | "left") && !targets.is_empty() {
        targets.clone()
    } else {
        actor
    };
    let key = match event.kind.as_str() {
        "invited" => "chat_event_invited",
        "joined" => "chat_event_joined",
        "left" => "chat_event_left",
        "removed" if actor.is_empty() => "chat_event_removed_members",
        "removed" => "chat_event_removed",
        "invitation_canceled" => "chat_event_invitation_canceled",
        "renamed" if !event.name.is_empty() => "chat_event_renamed_to",
        "renamed" => "chat_event_renamed",
        "picture_changed" => "chat_event_picture_changed",
        "link_join_allowed" => "chat_event_link_join_allowed",
        "link_join_disabled" => "chat_event_link_join_disabled",
        "created" => "chat_event_created",
        _ => "chat_event_unknown",
    };
    crate::i18n::tf(
        key,
        &[
            ("actor", &actor),
            ("targets", &targets),
            ("name", &event.name),
        ],
    )
}

pub(super) fn unknown() -> ChatEventInfo {
    ChatEventInfo {
        kind: "unknown".into(),
        actor_mid: String::new(),
        target_mids: Vec::new(),
        name: String::new(),
    }
}
pub(super) fn append(state: &AppState, event: &ChatEventInfo) {
    let label = gtk::Label::builder()
        .label(text(state, event))
        .use_markup(false)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(60)
        .xalign(0.5)
        .halign(gtk::Align::Center)
        .hexpand(true)
        .margin_start(24)
        .margin_end(24)
        .margin_top(8)
        .margin_bottom(8)
        .css_classes(["dim-label", "caption", "line-chat-event"])
        .build();
    let row = gtk::Box::builder()
        .css_classes(["line-message-row", "line-chat-event-row"])
        .build();
    row.append(&label);
    state.message_list.append(&row);
    state
        .chat_event_labels
        .borrow_mut()
        .push((event.clone(), label));
}
pub(super) fn refresh(state: &AppState) {
    for (event, label) in state.chat_event_labels.borrow().iter() {
        label.set_text(&text(state, event));
    }
}
