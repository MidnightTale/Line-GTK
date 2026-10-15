use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ChatCategory {
    #[default]
    All,
    Friends,
    Groups,
    Official,
}

impl ChatCategory {
    pub const ITEMS: [(Self, &str, &str); 4] = [
        (Self::All, "all", "chat_filter_all"),
        (Self::Friends, "friends", "chat_filter_friends"),
        (Self::Groups, "groups", "chat_filter_groups"),
        (Self::Official, "official", "chat_filter_official"),
    ];

    fn matches(self, chat: &ChatInfo, query: &str) -> bool {
        let group =
            matches!(chat.kind.as_str(), "group" | "room") || chat.mid.starts_with(['c', 'r']);
        let official = chat.kind == "bot";
        let category = match self {
            Self::All => true,
            Self::Friends => !group && !official,
            Self::Groups => group,
            Self::Official => !group && official,
        };
        category
            && (query.is_empty()
                || chat.name.to_lowercase().contains(query)
                || chat.preview.to_lowercase().contains(query))
    }
}

pub(super) fn wire(state: &AppState) {
    let action = gio::SimpleAction::new_stateful(
        "chat-category",
        Some(glib::VariantTy::STRING),
        &"all".to_variant(),
    );
    let s = state.clone();
    action.connect_activate(move |action, value| {
        let Some(value) = value.and_then(|value| value.str()) else {
            return;
        };
        let Some((category, _, _)) = ChatCategory::ITEMS.iter().find(|(_, id, _)| *id == value)
        else {
            return;
        };
        *s.chat_category.borrow_mut() = *category;
        action.set_state(&value.to_variant());
        apply_language(&s);
        refresh(&s);
    });
    state.window.add_action(&action);
}

pub(super) fn apply_language(state: &AppState) {
    let menu = gio::Menu::new();
    for ((category, id, key), button) in ChatCategory::ITEMS.iter().zip(&state.chat_filter_buttons)
    {
        let label = crate::i18n::t(key);
        button.set_label(&label);
        button.set_tooltip_text(Some(&label));
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some("win.chat-category"), Some(&id.to_variant()));
        menu.append_item(&item);
        if *category == *state.chat_category.borrow() {
            let title = crate::i18n::tf("chat_filter_current", &[("category", &label)]);
            state.compact_filter_btn.set_tooltip_text(Some(&title));
            state
                .compact_filter_btn
                .update_property(&[gtk::accessible::Property::Label(&title)]);
        }
    }
    state.compact_filter_btn.set_menu_model(Some(&menu));
}

pub(super) fn set_query(state: &AppState, query: &str) {
    *state.chat_query.borrow_mut() = query.trim().to_lowercase();
    // 幅を切り替えても、通常の検索欄と省幅時の検索欄で条件を共有する。
    if state.search_entry.text() != query {
        state.search_entry.set_text(query);
    }
    if state.compact_search_entry.text() != query {
        state.compact_search_entry.set_text(query);
    }
    refresh(state);
}

pub(super) fn refresh(state: &AppState) {
    let category = *state.chat_category.borrow();
    let query = state.chat_query.borrow();
    let chats = state.chats.borrow();
    let mut visible = 0;
    for (index, chat) in chats.iter().enumerate() {
        if let Some(row) = state.chat_list.row_at_index(index as i32) {
            let matches = category.matches(chat, &query);
            row.set_visible(matches);
            visible += usize::from(matches);
            if !matches && row.is_selected() {
                state.chat_list.unselect_row(&row);
            }
        }
    }
    if let Some(mid) = state.current_chat.borrow().as_deref()
        && let Some(index) = chats.iter().position(|chat| chat.mid == mid)
        && let Some(row) = state.chat_list.row_at_index(index as i32)
        && row.is_visible()
    {
        state.chat_list.select_row(Some(&row));
    }
    if !chats.is_empty() {
        if visible == 0 {
            set_side_state(state, "empty", Some(&crate::i18n::t("chat_filter_empty")));
        } else {
            set_side_state(state, "list", None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_partition_chats_using_metadata_instead_of_names() {
        for (mid, kind, category) in [
            ("u-friend", "dm", ChatCategory::Friends),
            ("u-official", "bot", ChatCategory::Official),
            ("c-group", "group", ChatCategory::Groups),
            ("r-room", "", ChatCategory::Groups),
            ("c-unknown", "", ChatCategory::Groups),
        ] {
            let chat: ChatInfo = serde_json::from_value(serde_json::json!({
                "mid": mid, "kind": kind, "name": "同じ名前", "preview": "最新の本文"
            }))
            .unwrap();
            assert!(ChatCategory::All.matches(&chat, ""));
            for (candidate, _, _) in ChatCategory::ITEMS.into_iter().skip(1) {
                assert_eq!(candidate.matches(&chat, ""), candidate == category);
            }
            assert!(category.matches(&chat, "本文"));
            assert!(!category.matches(&chat, "見つからない"));
        }
    }
}
