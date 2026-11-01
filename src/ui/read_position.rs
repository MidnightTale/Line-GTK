use super::*;

/// Keep the entry position stable even when opening the chat marks newer rows read.
#[derive(Default)]
pub(super) struct ReadPosition {
    account: String,
    chat: String,
    target: Option<String>,
    unread_on_open: i64,
}

impl ReadPosition {
    fn matches(&self, account: Option<&str>, chat: Option<&str>) -> bool {
        account == Some(self.account.as_str()) && chat == Some(self.chat.as_str())
    }

    fn infer(&mut self, messages: &[MessageInfo]) {
        if self.target.is_some() || self.unread_on_open <= 0 {
            return;
        }
        let mut ordered: Vec<_> = messages
            .iter()
            .filter(|m| {
                m.chat_event.is_none() && m.content_type != "UNSENT" && m.id.parse::<u128>().is_ok()
            })
            .collect();
        ordered.sort_by_key(|m| (m.created_time, m.id.parse::<u128>().unwrap_or(0)));
        let incoming: Vec<_> = ordered
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.mine)
            .collect();
        let Ok(unread) = usize::try_from(self.unread_on_open) else {
            return;
        };
        // If the page contains only unread rows, the boundary is still unknown.
        if incoming.len() < unread {
            return;
        }
        let first_unread = incoming[incoming.len() - unread].0;
        self.target = first_unread
            .checked_sub(1)
            .map(|index| ordered[index].id.clone());
    }
}

pub(super) fn open(state: &AppState, chat: &ChatInfo) {
    let account = state.self_mid.borrow().clone().unwrap_or_default();
    let target = state
        .config
        .borrow()
        .last_read_message(&account, &chat.mid)
        .map(str::to_owned);
    *state.last_read_position.borrow_mut() = ReadPosition {
        account,
        chat: chat.mid.clone(),
        target,
        unread_on_open: chat.unread,
    };
    refresh(state);
}

/// Infer an initial boundary only from a fresh latest page, never a stale snapshot.
pub(super) fn loaded(state: &AppState, messages: &[MessageInfo]) {
    let matches = state.last_read_position.borrow().matches(
        state.self_mid.borrow().as_deref(),
        state.current_chat.borrow().as_deref(),
    );
    if matches {
        state.last_read_position.borrow_mut().infer(messages);
        refresh(state);
    }
}

pub(super) fn record(state: &AppState, account: &str, chat: &str, message: &str) {
    if !*state.session_ready.borrow() || state.self_mid.borrow().as_deref() != Some(account) {
        return;
    }
    if let Err(error) =
        state
            .config
            .borrow_mut()
            .set_last_read_message(&state.data_dir, account, chat, message)
    {
        tracing::warn!(%error, "Could not save last read position");
    }
}

pub(super) fn refresh(state: &AppState) {
    let position = state.last_read_position.borrow();
    let visible = position.matches(
        state.self_mid.borrow().as_deref(),
        state.current_chat.borrow().as_deref(),
    ) && position.target.is_some();
    let label = crate::i18n::t("jump_last_read");
    state.last_read_btn.set_visible(visible);
    state
        .last_read_btn
        .set_sensitive(*state.session_ready.borrow());
    state.last_read_btn.set_tooltip_text(Some(&label));
    state
        .last_read_btn
        .update_property(&[gtk::accessible::Property::Label(&label)]);
}

pub(super) fn reset(state: &AppState) {
    *state.last_read_position.borrow_mut() = ReadPosition::default();
    refresh(state);
}

pub(super) fn wire(state: &AppState) {
    let s = state.clone();
    state.last_read_btn.connect_clicked(move |_| {
        let position = s.last_read_position.borrow();
        if !*s.session_ready.borrow()
            || !position.matches(
                s.self_mid.borrow().as_deref(),
                s.current_chat.borrow().as_deref(),
            )
        {
            return;
        }
        let Some(target) = position.target.clone() else {
            return;
        };
        let chat = position.chat.clone();
        drop(position);
        let before_window = s
            .message_list
            .edge_owner(true, |owner| owner.id.parse::<u128>().is_ok())
            .is_some_and(|oldest| {
                oldest
                    .id
                    .parse::<u128>()
                    .ok()
                    .zip(target.parse::<u128>().ok())
                    .is_some_and(|(oldest, target)| target < oldest)
            });
        if s.seen_msg_ids.borrow().contains(&target) || before_window {
            // Paging can recover older rows from disk or the server after cache eviction.
            history::jump_to_message(&s, &target);
        } else {
            // A newer target may have been trimmed while browsing older history.
            history::jump_to_search_message(&s, &chat, &target);
        }
    });
    refresh(state);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, mine: bool) -> MessageInfo {
        serde_json::from_value(serde_json::json!({
            "id": id, "mine": mine, "createdTime": 100,
        }))
        .unwrap()
    }

    #[test]
    fn entry_boundary_survives_later_messages_and_reads() {
        let mut position = ReadPosition {
            account: "self".into(),
            chat: "chat".into(),
            unread_on_open: 2,
            ..Default::default()
        };
        position.infer(&[
            message("12", false),
            message("9", false),
            message("10", true),
            message("11", false),
        ]);
        assert_eq!(position.target.as_deref(), Some("10"));
        position.infer(&[
            message("13", false),
            message("14", false),
            message("15", false),
        ]);
        assert_eq!(position.target.as_deref(), Some("10"));
        assert!(position.matches(Some("self"), Some("chat")));
        assert!(!position.matches(Some("other"), Some("chat")));
        assert!(!position.matches(Some("self"), Some("other")));
    }

    #[test]
    fn unread_only_page_does_not_invent_a_read_position() {
        let mut position = ReadPosition {
            unread_on_open: 50,
            ..Default::default()
        };
        position.infer(&[message("10", false), message("11", false)]);
        assert!(position.target.is_none());
        position.unread_on_open = 2;
        position.infer(&[message("10", false), message("11", false)]);
        assert!(position.target.is_none());
    }
}

#[cfg(test)]
#[path = "read_position_test.rs"]
mod integration_tests;
