use super::*;

pub(super) fn eligible(message: &MessageInfo) -> bool {
    !message.text.is_empty()
        && !message.decryption_failed
        && message.chat_event.is_none()
        && !["UNSENT", "CHATEVENT", "CHAT_EVENT", "18"]
            .contains(&message.content_type.to_ascii_uppercase().as_str())
}

pub(super) fn append_menu(menu: &gio::Menu, message: &MessageInfo) {
    if !eligible(message) {
        return;
    }
    let section = gio::Menu::new();
    for (key, action) in [
        ("copy_message", "win.copy-message"),
        ("copy_message_select", "win.select-copy-message"),
    ] {
        let item = gio::MenuItem::new(Some(&crate::i18n::t(key)), None);
        item.set_action_and_target_value(Some(action), Some(&message.id.to_variant()));
        section.append_item(&item);
    }
    menu.append_section(None, &section);
}

pub(super) fn wire(state: &AppState) {
    for (name, select) in [("copy-message", false), ("select-copy-message", true)] {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let s = state.clone();
        action.connect_activate(move |_, value| {
            let Some(id) = value.and_then(|value| value.str()) else {
                return;
            };
            let text = s
                .reply_messages
                .borrow()
                .get(id)
                .filter(|message| eligible(message))
                .map(|message| message.text.clone());
            let Some(text) = text else { return };
            if select {
                let dialog = selection_dialog(&text);
                dialog.present(Some(&s.window));
            } else {
                s.window.clipboard().set_text(&text);
            }
        });
        state.window.add_action(&action);
    }
}

fn selection_dialog(text: &str) -> libadwaita::Dialog {
    let dialog = libadwaita::Dialog::builder()
        .title(crate::i18n::t("copy_message_select"))
        .content_width(560)
        .content_height(400)
        .build();
    let toolbar = libadwaita::ToolbarView::new();
    toolbar.add_top_bar(&libadwaita::HeaderBar::new());
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(18)
        .margin_end(18)
        .margin_top(12)
        .margin_bottom(18)
        .build();
    body.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("copy_message_select_hint"))
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    // Select the original text, including emoji fallback text, rather than the
    // display buffer where custom emoji are replaced by image objects.
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(true)
        .accepts_tab(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(12)
        .right_margin(12)
        .top_margin(12)
        .bottom_margin(12)
        .build();
    view.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "copy_message_text",
    ))]);
    let buffer = view.buffer();
    buffer.set_text(text);
    body.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .css_classes(["frame"])
            .child(&view)
            .build(),
    );
    let actions = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::End)
        .build();
    let copy_all = gtk::Button::with_label(&crate::i18n::t("copy_message_all"));
    let copy_selection = gtk::Button::builder()
        .label(crate::i18n::t("copy_message_selection"))
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    let weak = copy_selection.downgrade();
    buffer.connect_has_selection_notify(move |buffer| {
        if let Some(button) = weak.upgrade() {
            button.set_sensitive(buffer.has_selection());
        }
    });
    let weak_dialog = dialog.downgrade();
    let selection_buffer = buffer.clone();
    copy_selection.connect_clicked(move |button| {
        if let Some((start, end)) = selection_buffer.selection_bounds() {
            button
                .clipboard()
                .set_text(&selection_buffer.text(&start, &end, false));
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.close();
            }
        }
    });
    let text = text.to_string();
    let weak_dialog = dialog.downgrade();
    copy_all.connect_clicked(move |button| {
        button.clipboard().set_text(&text);
        if let Some(dialog) = weak_dialog.upgrade() {
            dialog.close();
        }
    });
    actions.append(&copy_all);
    actions.append(&copy_selection);
    body.append(&actions);
    toolbar.set_content(Some(&body));
    dialog.set_child(Some(&toolbar));
    dialog.set_focus(Some(&view));
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn selection_dialog_preserves_exact_text_and_enables_partial_copy() {
        libadwaita::init().unwrap();
        fn find<T: IsA<gtk::Widget> + Clone + 'static>(widget: &gtk::Widget) -> Option<T> {
            if let Ok(found) = widget.clone().downcast::<T>() {
                return Some(found);
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                if let Some(found) = find(&widget) {
                    return Some(found);
                }
            }
            None
        }
        let text = "\nGET /path?q=a&b=2 HTTP/1.1\nX-Name: 😀\n\t<literal>  \n";
        let dialog = selection_dialog(text);
        let view = find::<gtk::TextView>(&dialog.child().unwrap()).unwrap();
        let buffer = view.buffer();
        assert!(!view.is_editable());
        assert!(view.is_cursor_visible());
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), false),
            text
        );
        let actions = dialog
            .child()
            .unwrap()
            .downcast::<libadwaita::ToolbarView>()
            .unwrap()
            .content()
            .unwrap()
            .last_child()
            .unwrap();
        let copy_selection = actions.last_child().and_downcast::<gtk::Button>().unwrap();
        assert!(!copy_selection.is_sensitive());
        let start = text.chars().position(|ch| ch == '😀').unwrap() as i32;
        buffer.select_range(
            &buffer.iter_at_offset(start),
            &buffer.iter_at_offset(start + 1),
        );
        assert!(copy_selection.is_sensitive());
        let (start, end) = buffer.selection_bounds().unwrap();
        assert_eq!(buffer.text(&start, &end, false), "😀");
        buffer.place_cursor(&buffer.start_iter());
        assert!(!copy_selection.is_sensitive());
    }

    #[test]
    fn copying_requires_readable_message_text() {
        let mut message: MessageInfo = serde_json::from_value(serde_json::json!({
            "id": "pending-1", "text": "😀 @Amy\n<hello>"
        }))
        .unwrap();
        assert!(eligible(&message));
        message.decryption_failed = true;
        assert!(!eligible(&message));
        message.decryption_failed = false;
        for content_type in ["UNSENT", "CHATEVENT", "chat_event", "18"] {
            message.content_type = content_type.into();
            assert!(!eligible(&message));
        }
        message.content_type = "TEXT".into();
        message.text.clear();
        assert!(!eligible(&message));
    }
}
