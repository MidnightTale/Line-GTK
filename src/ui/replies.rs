use super::*;
use crate::protocol::ReplyInfo;

#[derive(Clone)]
pub(super) struct ReplyTarget {
    pub chat_mid: String,
    pub quote: ReplyInfo,
}

#[derive(Debug, Clone)]
pub(super) struct ReplyDraft {
    pub chat_mid: String,
    pub text: String,
    pub quote: ReplyInfo,
}

fn eligible(message: &MessageInfo) -> bool {
    message.id.parse::<u128>().is_ok_and(|id| id > 0)
        && !message.decryption_failed
        && !message.content_type.eq_ignore_ascii_case("CALL")
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
        "UNAVAILABLE" => Some("reply_unavailable"),
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
    let copy = gio::SimpleAction::new("copy-message", Some(glib::VariantTy::STRING));
    let s = state.clone();
    copy.connect_activate(move |_, value| {
        if let Some(id) = value.and_then(|value| value.str())
            && let Some(message) = s.reply_messages.borrow().get(id)
        {
            s.window.clipboard().set_text(&message.text);
        }
    });
    state.window.add_action(&copy);
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
    state
        .reply_cancel
        .set_tooltip_text(Some(&crate::i18n::t("reply_cancel")));
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
    if state.current_chat.borrow().as_deref() != Some(&draft.chat_mid)
        || !state.composer.text().is_empty()
        || state.reply_target.borrow().is_some()
    {
        return;
    }
    state.composer.set_text(&draft.text);
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
        .spacing(2)
        .css_classes(["line-reply-quote"])
        .build();
    card.append(
        &gtk::Label::builder()
            .label(author(state, quote))
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(36)
            .xalign(0.0)
            .css_classes(["heading", "caption"])
            .build(),
    );
    card.append(
        &gtk::Label::builder()
            .label(quote_text(quote))
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(38)
            .build(),
    );
    bubble.append(&card);
}

pub(super) fn menu(message: &MessageInfo) -> gio::Menu {
    let menu = gio::Menu::new();
    if eligible(message) {
        let item = gio::MenuItem::new(Some(&crate::i18n::t("reply")), None);
        item.set_action_and_target_value(Some("win.reply-message"), Some(&message.id.to_variant()));
        menu.append_item(&item);
    }
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
        .css_classes(["flat", "circular", "line-reply-button"])
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
    if !eligible(message) {
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
    if !message.text.is_empty() {
        let copy = gio::MenuItem::new(Some(&crate::i18n::t("copy_message")), None);
        copy.set_action_and_target_value(Some("win.copy-message"), Some(&message.id.to_variant()));
        menu.append_item(&copy);
    }
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
