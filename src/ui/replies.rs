use super::*;
use crate::protocol::ReplyInfo;

#[derive(Clone)]
pub(super) struct ReplyTarget {
    pub chat_mid: String,
    pub quote: ReplyInfo,
}

#[derive(Debug, Clone)]
pub(super) struct ReplyDraft {
    pub account: Option<String>,
    pub chat_mid: String,
    pub text: String,
    pub quote: ReplyInfo,
    pub mentions: Vec<crate::protocol::Mention>,
}

fn eligible(message: &MessageInfo) -> bool {
    message.id.parse::<u128>().is_ok_and(|id| id > 0)
        && message.chat_event.is_none()
        && !message.decryption_failed
        && !["CALL", "CHATEVENT", "CHAT_EVENT", "UNSENT"]
            .contains(&message.content_type.to_ascii_uppercase().as_str())
}

fn snapshot(message: &MessageInfo) -> ReplyInfo {
    ReplyInfo {
        message_id: message.id.clone(),
        text: message.text.chars().take(512).collect(),
        from: message.from.clone(),
        mine: message.mine,
        content_type: message.content_type.clone(),
    }
}

fn quote_text(quote: &ReplyInfo) -> String {
    let key = match quote.content_type.to_ascii_uppercase().as_str() {
        "IMAGE" => Some("reply_media_image"),
        "VIDEO" => Some("reply_media_video"),
        "STICKER" => Some("reply_media_sticker"),
        "AUDIO" => Some("reply_media_audio"),
        "FILE" => Some("reply_media_file"),
        "UNAVAILABLE" | "UNSENT" => Some("reply_unavailable"),
        _ => None,
    };
    if let Some(key) = key {
        crate::i18n::t(key)
    } else if quote.text.trim().is_empty() {
        crate::i18n::t("reply_unavailable")
    } else {
        quote.text.clone()
    }
}

fn author(state: &AppState, quote: &ReplyInfo) -> String {
    if quote.mine {
        return crate::i18n::t("you");
    }
    if let Some(name) = message_senders::known_name(state, &quote.from) {
        return name;
    }
    let chats = state.chats.borrow();
    if let Some(sender) = chats.iter().find(|chat| chat.mid == quote.from) {
        return sender.name.clone();
    }
    let current = state.current_chat.borrow();
    if let Some(chat) = chats
        .iter()
        .find(|chat| Some(chat.mid.as_str()) == current.as_deref())
        && chat.kind == "dm"
        && !quote.from.is_empty()
    {
        return chat.name.clone();
    }
    crate::i18n::t("reply_sender")
}

pub(super) fn wire(state: &AppState) {
    let action = gio::SimpleAction::new("reply-message", Some(glib::VariantTy::STRING));
    let s = state.clone();
    action.connect_activate(move |_, value| {
        let Some(id) = value.and_then(|value| value.str()) else {
            return;
        };
        let message = s.reply_messages.borrow().get(id).cloned();
        let chat_mid = s.current_chat.borrow().clone();
        if let (Some(message), Some(chat_mid)) = (message, chat_mid)
            && eligible(&message)
        {
            *s.reply_target.borrow_mut() = Some(ReplyTarget {
                chat_mid,
                quote: snapshot(&message),
            });
            refresh(&s);
            s.composer.grab_focus();
        }
    });
    state.window.add_action(&action);
    let s = state.clone();
    state.reply_cancel.connect_clicked(move |_| {
        cancel(&s);
        s.composer.grab_focus();
    });
    refresh(state);
}

pub(super) fn cancel(state: &AppState) {
    state.reply_target.borrow_mut().take();
    state.reply_preview.set_reveal_child(false);
}

pub(super) fn refresh(state: &AppState) {
    let cancel_label = crate::i18n::t("reply_cancel");
    state.reply_cancel.set_tooltip_text(Some(&cancel_label));
    state
        .reply_cancel
        .update_property(&[gtk::accessible::Property::Label(&cancel_label)]);
    let target = state.reply_target.borrow();
    if let Some(target) = target.as_ref() {
        state.reply_title.set_text(&crate::i18n::tf(
            "reply_to",
            &[("name", &author(state, &target.quote))],
        ));
        state.reply_text.set_text(&quote_text(&target.quote));
    }
    state.reply_preview.set_reveal_child(target.is_some());
}

pub(super) fn restore_failed(state: &AppState, draft: &ReplyDraft) {
    // 別のトークや、その後に書いた下書きへ失敗した返信を混ぜない。
    if *state.self_mid.borrow() != draft.account
        || state.current_chat.borrow().as_deref() != Some(&draft.chat_mid)
        || !state.composer.text().is_empty()
        || state.reply_target.borrow().is_some()
    {
        return;
    }
    state.composer.set_text(&draft.text);
    super::mentions::restore(state, &draft.mentions);
    let account = state.self_mid.borrow().clone().unwrap_or_default();
    if state
        .unsent_ids
        .borrow()
        .contains(&(account, draft.quote.message_id.clone()))
    {
        return;
    }
    *state.reply_target.borrow_mut() = Some(ReplyTarget {
        chat_mid: draft.chat_mid.clone(),
        quote: draft.quote.clone(),
    });
    refresh(state);
}

pub(super) fn append_quote(state: &AppState, bubble: &gtk::Box, quote: &ReplyInfo) {
    let resolved = state
        .reply_messages
        .borrow()
        .get(&quote.message_id)
        .map(snapshot);
    let quote = resolved.as_ref().unwrap_or(quote);
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .css_classes(["line-reply-quote"])
        .build();
    let heading = gtk::Box::builder().spacing(6).build();
    let arrow = gtk::Image::from_icon_name("mail-reply-sender-symbolic");
    arrow.set_pixel_size(12);
    arrow.set_accessible_role(gtk::AccessibleRole::Presentation);
    heading.append(&arrow);
    let author_label = gtk::Label::builder()
        .label(author(state, quote))
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(36)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["heading", "caption", "line-reply-author"])
        .build();
    if !quote.mine {
        message_senders::register(state, &quote.from, &author_label);
    }
    heading.append(&author_label);
    card.append(&heading);
    card.append(
        &gtk::Label::builder()
            .label(quote_text(quote))
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(38)
            .css_classes(["line-reply-excerpt"])
            .build(),
    );
    let button = gtk::Button::builder()
        .child(&card)
        .css_classes(["flat", "line-reply-quote-button"])
        .halign(gtk::Align::Fill)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "reply_open_original",
    ))]);
    button.set_tooltip_text(Some(&crate::i18n::t("reply_open_original")));
    let message_id = quote.message_id.clone();
    let s = state.clone();
    button.connect_clicked(move |_| history::jump_to_message(&s, &message_id));
    bubble.append(&button);
}

pub(super) fn menu(message: &MessageInfo) -> gio::Menu {
    let menu = gio::Menu::new();
    if eligible(message) {
        let item = gio::MenuItem::new(Some(&crate::i18n::t("reply")), None);
        item.set_action_and_target_value(Some("win.reply-message"), Some(&message.id.to_variant()));
        menu.append_item(&item);
    }
    for item in reactions::menu_items(message) {
        menu.append_item(&item);
    }
    if unsend::eligible(message) {
        let item = gio::MenuItem::new(Some(&crate::i18n::t("unsend_message")), None);
        item.set_action_and_target_value(
            Some("win.unsend-message"),
            Some(&message.id.to_variant()),
        );
        menu.append_item(&item);
    }
    message_copy::append_menu(&menu, message);
    menu
}

pub(super) fn button(message: &MessageInfo) -> Option<gtk::Button> {
    if !eligible(message) {
        return None;
    }
    let label = crate::i18n::t("reply");
    let button = gtk::Button::builder()
        .icon_name("mail-reply-sender-symbolic")
        .tooltip_text(&label)
        .action_target(&message.id.to_variant())
        .action_name("win.reply-message")
        .valign(gtk::Align::Center)
        .css_classes([
            "flat",
            "circular",
            "line-message-action",
            "line-reply-button",
        ])
        .build();
    button.update_property(&[
        gtk::accessible::Property::Label(&label),
        gtk::accessible::Property::Description(&quote_text(&snapshot(message))),
    ]);
    Some(button)
}

fn popup(bubble: &gtk::Box, menu: &gio::Menu, x: f64, y: f64) {
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.set_parent(bubble);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|popover| {
        // 項目のアクションとフォーカス復帰が終わるまで、親との接続を残す。
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
    popover.grab_focus();
}

pub(super) fn wire_context(bubble: &gtk::Box, message: &MessageInfo) {
    if !eligible(message)
        && !unsend::eligible(message)
        && !reactions::eligible(message)
        && !message_copy::eligible(message)
    {
        return;
    }
    // 仮想リストから行が外れたら、開いているメニューも片付ける。
    bubble.connect_unrealize(|bubble| {
        let mut child = bubble.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.is::<gtk::Popover>() {
                widget.unparent();
            }
        }
    });
    let menu = menu(message);
    let click = gtk::GestureClick::new();
    click.set_button(3);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = bubble.downgrade();
    let menu_click = menu.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        if let Some(bubble) = weak.upgrade() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            popup(&bubble, &menu_click, x, y);
        }
    });
    bubble.add_controller(click);
    bubble.set_focusable(true);
    let keys = gtk::EventControllerKey::new();
    let weak = bubble.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gdk::Key::Menu
            || (key == gdk::Key::F10 && modifiers.contains(gdk::ModifierType::SHIFT_MASK))
        {
            if let Some(bubble) = weak.upgrade() {
                popup(&bubble, &menu, 10.0, 10.0);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    bubble.add_controller(keys);
}

pub(super) fn redact(state: &AppState, id: &str) {
    if state
        .reply_target
        .borrow()
        .as_ref()
        .is_some_and(|target| target.quote.message_id == id)
    {
        cancel(state);
    }
    let mut affected = Vec::new();
    for (message_id, message) in state.reply_messages.borrow_mut().iter_mut() {
        if let Some(quote) = &mut message.reply_to
            && quote.message_id == id
        {
            quote.text.clear();
            quote.content_type = "UNAVAILABLE".into();
            affected.push(message_id.clone());
        }
    }
    fn replace(widget: &gtk::Widget) {
        if widget.has_css_class("line-reply-quote") {
            if let Some(label) = widget.last_child().and_downcast::<gtk::Label>() {
                label.set_text(&crate::i18n::t("reply_unavailable"));
            }
            return;
        }
        let mut child = widget.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            replace(&item);
        }
    }
    for id in affected {
        if let Some(row) = state.message_rows.borrow().get(&id) {
            replace(row.upcast_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_targets_require_a_sent_message_and_readable_content() {
        let mut message: MessageInfo =
            serde_json::from_str(r#"{"id":"9007199254740993","text":"こんにちは"}"#).unwrap();
        assert!(eligible(&message));
        assert_eq!(snapshot(&message).message_id, "9007199254740993");
        message.id = "pending-1".into();
        assert!(!eligible(&message));
        message.id = "42".into();
        message.decryption_failed = true;
        assert!(!eligible(&message));
        message.decryption_failed = false;
        message.content_type = "CALL".into();
        assert!(!eligible(&message));
        message.content_type = "CHAT_EVENT".into();
        message.chat_event = Some(crate::protocol::ChatEventInfo {
            kind: "joined".into(),
            actor_mid: "u-peer".into(),
            target_mids: Vec::new(),
            name: String::new(),
        });
        assert!(!eligible(&message));
    }

    #[test]
    fn cached_chat_events_without_metadata_cannot_be_reply_targets() {
        for content_type in ["CHATEVENT", "CHAT_EVENT", "chatEvent", "chat_event"] {
            let message: MessageInfo = serde_json::from_value(serde_json::json!({
                "id": "42",
                "contentType": content_type,
                "text": "[CHATEVENT]"
            }))
            .unwrap();
            assert!(message.chat_event.is_none());
            assert!(!eligible(&message), "{content_type}");
            assert!(button(&message).is_none());
        }
    }

    #[test]
    fn quote_preview_bounds_unicode_without_breaking_characters() {
        let message: MessageInfo =
            serde_json::from_value(serde_json::json!({"id":"42", "text":"あ🙂".repeat(400)}))
                .unwrap();
        let quote = snapshot(&message);
        assert_eq!(quote.text.chars().count(), 512);
        assert!(quote.text.ends_with('🙂'));
    }
}
