use super::*;

pub(super) fn sort(chats: &mut [ChatInfo], config: &AppConfig, account: &str) {
    chats.sort_by(|a, b| {
        config
            .is_chat_pinned(account, &b.mid)
            .cmp(&config.is_chat_pinned(account, &a.mid))
            .then_with(|| b.last_activity.cmp(&a.last_activity))
    });
}

pub(super) fn reorder(state: &AppState) {
    let account = state.self_mid.borrow().clone().unwrap_or_default();
    let rows: HashMap<_, _> = state
        .chats
        .borrow()
        .iter()
        .enumerate()
        .filter_map(|(i, chat)| {
            state
                .chat_list
                .row_at_index(i as i32)
                .map(|row| (chat.mid.clone(), row))
        })
        .collect();
    sort(
        &mut state.chats.borrow_mut(),
        &state.config.borrow(),
        &account,
    );
    // 行を作り直さず移動し、開いている会話と下書きを維持する。
    for (i, chat) in state.chats.borrow().iter().enumerate() {
        if let Some(row) = rows.get(&chat.mid)
            && row.index() != i as i32
        {
            state.chat_list.remove(row);
            state.chat_list.insert(row, i as i32);
        }
    }
}

pub(super) fn append_menu(menu: &gio::Menu, mid: &str, pinned: bool) {
    let item = gio::MenuItem::new(
        Some(&crate::i18n::t(if pinned {
            "unpin_chat"
        } else {
            "pin_chat"
        })),
        None,
    );
    item.set_action_and_target_value(Some("win.toggle-chat-pin"), Some(&mid.to_variant()));
    menu.append_item(&item);
}

pub(super) fn refresh_row(state: &AppState, chat: &ChatInfo, row: &gtk::ListBoxRow) {
    let pinned = state
        .self_mid
        .borrow()
        .as_deref()
        .is_some_and(|account| state.config.borrow().is_chat_pinned(account, &chat.mid));
    let label = crate::i18n::t("chat_pinned");
    fn update(widget: &gtk::Widget, pinned: bool, label: &str) {
        if widget.has_css_class("line-chat-pinned") {
            widget.set_visible(pinned);
            widget.set_tooltip_text(Some(label));
            widget.update_property(&[gtk::accessible::Property::Label(label)]);
            return;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            update(&next, pinned, label);
        }
    }
    if let Some(child) = row.child() {
        update(&child, pinned, &label);
    }
    if pinned {
        row.set_tooltip_text(Some(&format!(
            "{}\n{label}",
            row.tooltip_text().unwrap_or_default()
        )));
    }
}

pub(super) fn wire(state: &AppState) {
    let action = gio::SimpleAction::new("toggle-chat-pin", Some(glib::VariantTy::STRING));
    let s = state.clone();
    action.connect_activate(move |_, value| {
        if !*s.session_ready.borrow() {
            return;
        }
        let Some(mid) = value.and_then(|value| value.str()) else {
            return;
        };
        if !s.chats.borrow().iter().any(|chat| chat.mid == mid) {
            return;
        }
        let Some(account) = s.self_mid.borrow().clone() else {
            return;
        };
        let pinned = !s.config.borrow().is_chat_pinned(&account, mid);
        if s.config
            .borrow_mut()
            .set_chat_pinned(&s.data_dir, &account, mid, pinned)
            .is_err()
        {
            toast(&s, &crate::i18n::t("hidden_chat_save_failed"));
            return;
        }
        chat_filter::refresh(&s);
        refresh_tray_menu(&s);
        toast(
            &s,
            &crate::i18n::t(if pinned {
                "chat_pin_done"
            } else {
                "chat_unpin_done"
            }),
        );
    });
    state.window.add_action(&action);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_chats_stay_above_new_activity_and_unpin_returns_to_activity_order() {
        let chat = |mid, last_activity| {
            serde_json::from_value::<ChatInfo>(
                serde_json::json!({"mid":mid,"name":mid,"lastActivity":last_activity}),
            )
            .unwrap()
        };
        let mut chats = vec![chat("u-new", 100), chat("u-pin", 1), chat("u-pin2", 20)];
        let mut config = AppConfig::default();
        config.pinned_chats.insert(
            "u-self".into(),
            ["u-pin".into(), "u-pin2".into()].into_iter().collect(),
        );
        sort(&mut chats, &config, "u-self");
        assert_eq!(
            chats.iter().map(|c| c.mid.as_str()).collect::<Vec<_>>(),
            ["u-pin2", "u-pin", "u-new"]
        );
        sort(&mut chats, &config, "u-other");
        assert_eq!(
            chats.iter().map(|c| c.mid.as_str()).collect::<Vec<_>>(),
            ["u-new", "u-pin2", "u-pin"]
        );
        config.pinned_chats.clear();
        sort(&mut chats, &config, "u-self");
        assert_eq!(chats[0].mid, "u-new");
    }
}
