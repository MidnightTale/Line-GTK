use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ChatCategory {
    #[default]
    All,
    Friends,
    Groups,
    Official,
    NoMessages,
}

impl ChatCategory {
    pub const ITEMS: [(Self, &str, &str); 5] = [
        (Self::All, "all", "chat_filter_all"),
        (Self::Friends, "friends", "chat_filter_friends"),
        (Self::Groups, "groups", "chat_filter_groups"),
        (Self::Official, "official", "chat_filter_official"),
        (Self::NoMessages, "no-messages", "chat_filter_no_messages"),
    ];

    fn matches(self, chat: &ChatInfo, query: &str) -> bool {
        let group =
            matches!(chat.kind.as_str(), "group" | "room") || chat.mid.starts_with(['c', 'r']);
        let official = chat.kind == "bot";
        // Older caches did not distinguish contacts from conversations.
        let contact_only = !group
            && chat
                .contact_only
                .unwrap_or(chat.last_activity <= 0 && chat.unread == 0 && chat.preview.is_empty());
        let category = match self {
            Self::All => !contact_only,
            Self::Friends => !contact_only && !group && !official,
            Self::Groups => group,
            Self::Official => !contact_only && !group && official,
            Self::NoMessages => contact_only,
        };
        category
            && (query.is_empty()
                || chat.name.to_lowercase().contains(query)
                || chat.preview.to_lowercase().contains(query))
    }
}

// Bind text immediately. Mirroring from `search-changed` can replay a stale
// query from the hidden entry while the user is already typing the next one.
fn bind_search_entries(normal: &gtk::SearchEntry, compact: &gtk::SearchEntry) {
    normal
        .bind_property("text", compact, "text")
        .bidirectional()
        .sync_create()
        .build();
}

pub(super) fn wire(state: &AppState) {
    bind_search_entries(&state.search_entry, &state.compact_search_entry);
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
    for (category, id, key) in ChatCategory::ITEMS {
        let label = crate::i18n::t(key);
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some("win.chat-category"), Some(&id.to_variant()));
        menu.append_item(&item);
        if category == *state.chat_category.borrow() {
            let title = crate::i18n::tf("chat_filter_current", &[("category", &label)]);
            state.chat_filter_label.set_text(&label);
            state.chat_filter_btn.set_tooltip_text(Some(&title));
            state
                .chat_filter_btn
                .update_property(&[gtk::accessible::Property::Label(&title)]);
            state.compact_filter_btn.set_tooltip_text(Some(&title));
            state
                .compact_filter_btn
                .update_property(&[gtk::accessible::Property::Label(&title)]);
        }
    }
    state.chat_filter_btn.set_menu_model(Some(&menu));
    state.compact_filter_btn.set_menu_model(Some(&menu));
}

pub(super) fn set_query(state: &AppState, query: &str) {
    let query = query.trim().to_lowercase();
    if *state.chat_query.borrow() == query {
        return;
    }
    *state.chat_query.borrow_mut() = query;
    // SearchEntry already debounces `search-changed`. Only filter here; never
    // write back into either entry from a delayed callback (including IME input).
    refresh(state);
}

pub(super) fn refresh(state: &AppState) {
    read_pause::refresh_once_button(state);
    pinned_chats::reorder(state);
    let category = *state.chat_category.borrow();
    let query = state.chat_query.borrow();
    let chats = state.chats.borrow();
    let mut visible = 0;
    for (index, chat) in chats.iter().enumerate() {
        if let Some(row) = state.chat_list.row_at_index(index as i32) {
            read_pause::refresh_chat_row(state, chat, &row);
            pinned_chats::refresh_row(state, chat, &row);
            contact_blocks::refresh_row(state, chat, &row);
            let matches =
                !hidden_chats::is_hidden(state, &chat.mid) && category.matches(chat, &query);
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
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn search_entries_share_unicode_edits_in_both_directions() {
        gtk::test_synced(|| {
            let normal = gtk::SearchEntry::new();
            let compact = gtk::SearchEntry::new();
            bind_search_entries(&normal, &compact);

            for (source, mirror) in [(&normal, &compact), (&compact, &normal)] {
                source.set_text("");
                let mut position = 0;
                for ch in "Alice tiếng Việt 日本語".chars() {
                    source.insert_text(&ch.to_string(), &mut position);
                    assert_eq!(source.text(), mirror.text());
                    assert_eq!(position as usize, source.text().chars().count());
                }
                // Preserve raw input; normalization belongs to filtering.
                source.set_text("  Tiếng Việt 日本語  ");
                assert_eq!(source.text(), "  Tiếng Việt 日本語  ");
                assert_eq!(source.text(), mirror.text());
                source.set_text("");
                assert!(mirror.text().is_empty());
            }
        });
    }

    #[test]
    fn contacts_without_messages_have_their_own_category() {
        for kind in ["dm", "bot"] {
            for metadata in [
                serde_json::json!({}),
                serde_json::json!({"contactOnly": true, "lastActivity": 123, "preview": "Tap to open"}),
            ] {
                let mut raw =
                    serde_json::json!({"mid": "u-contact", "kind": kind, "name": "Alice"});
                raw.as_object_mut()
                    .unwrap()
                    .extend(metadata.as_object().unwrap().clone());
                let chat: ChatInfo = serde_json::from_value(raw).unwrap();
                for (category, _, _) in ChatCategory::ITEMS {
                    assert_eq!(
                        category.matches(&chat, ""),
                        category == ChatCategory::NoMessages
                    );
                }
                assert!(ChatCategory::NoMessages.matches(&chat, "alice"));
                assert!(!ChatCategory::NoMessages.matches(&chat, "bob"));
            }
        }
    }

    #[test]
    fn real_conversations_stay_visible_while_previews_are_loading() {
        for metadata in [
            serde_json::json!({"lastActivity": 123}),
            serde_json::json!({"unread": 1}),
            serde_json::json!({"contactOnly": false}),
        ] {
            let mut raw = serde_json::json!({"mid": "u-chat", "kind": "dm", "name": "Alice"});
            raw.as_object_mut()
                .unwrap()
                .extend(metadata.as_object().unwrap().clone());
            let chat: ChatInfo = serde_json::from_value(raw).unwrap();
            assert!(ChatCategory::All.matches(&chat, ""));
            assert!(ChatCategory::Friends.matches(&chat, ""));
            assert!(!ChatCategory::NoMessages.matches(&chat, ""));
        }
    }

    #[test]
    fn empty_groups_remain_in_group_and_all_categories() {
        let chat: ChatInfo = serde_json::from_value(serde_json::json!({
            "mid": "c-group", "name": "New group", "kind": "group"
        }))
        .unwrap();
        assert!(ChatCategory::All.matches(&chat, ""));
        assert!(ChatCategory::Groups.matches(&chat, ""));
        assert!(!ChatCategory::NoMessages.matches(&chat, ""));
    }

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
