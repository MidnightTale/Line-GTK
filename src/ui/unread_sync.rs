use super::*;

#[derive(Default)]
pub(super) struct UnreadSync {
    latest: HashMap<String, String>,
    read_through: HashMap<String, String>,
}

fn numeric_id(id: &str) -> Option<&str> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let normalized = id.trim_start_matches('0');
    Some(if normalized.is_empty() {
        "0"
    } else {
        normalized
    })
}
fn at_least(a: &str, b: &str) -> bool {
    (a.len(), a) >= (b.len(), b)
}
impl UnreadSync {
    pub fn observe(&mut self, mid: &str, id: &str) {
        let Some(id) = numeric_id(id) else {
            return;
        };
        let previous = self.latest.entry(mid.to_string()).or_default();
        if at_least(id, previous) {
            *previous = id.to_string();
        }
    }
    pub fn already_read(&self, mid: &str, id: &str) -> bool {
        numeric_id(id).is_some_and(|id| {
            self.read_through
                .get(mid)
                .is_some_and(|read| at_least(read, id))
        })
    }
    pub fn note_read(&mut self, mid: &str, through: &str) {
        let Some(through) = numeric_id(through) else {
            return;
        };
        let read = self.read_through.entry(mid.to_string()).or_default();
        if at_least(through, read) {
            *read = through.to_string();
        }
    }
    pub fn accept(&mut self, mid: &str, through: &str, unread: i64) -> bool {
        let Some(through) = numeric_id(through) else {
            return false;
        };
        if unread < 0
            || self
                .latest
                .get(mid)
                .is_some_and(|latest| !at_least(through, latest))
        {
            return false;
        }
        self.observe(mid, through);
        if unread == 0 {
            self.note_read(mid, through);
        }
        true
    }
}

pub(super) fn apply(state: &AppState, account: &str, mid: &str, through: &str, unread: i64) {
    if !*state.session_ready.borrow()
        || state.self_mid.borrow().as_deref() != Some(account)
        || !state.unread_sync.borrow_mut().accept(mid, through, unread)
    {
        return;
    }
    // スマホ等ですでに読んだ結果を反映する。PCから既読は送らない。
    if unread == 0 {
        read_position::record(state, account, mid, through);
    }
    if let Some(chat) = state
        .chats
        .borrow_mut()
        .iter_mut()
        .find(|chat| chat.mid == mid)
    {
        chat.unread = unread;
    }
    if let Some(badge) = state.chat_unread_badges.borrow().get(mid) {
        badge.set_text(&if unread > 0 {
            unread.to_string()
        } else {
            String::new()
        });
        badge.set_visible(unread > 0);
    }
    if state.current_chat.borrow().as_deref() == Some(mid) {
        state.chat_subtitle.set_text(&if unread > 0 {
            crate::i18n::tf("unread", &[("n", &unread.to_string())])
        } else {
            crate::i18n::t("conversation")
        });
        if unread == 0 {
            *state.pending_new_below.borrow_mut() = 0;
            state.jump_banner.set_reveal_child(false);
        }
    }
    refresh_tray_menu(state);
    read_pause::refresh_once_button(state);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_time_read_tracks_only_the_confirmed_boundary_even_with_newer_unread() {
        let mut state = UnreadSync::default();
        state.observe("u-peer", "102");
        state.note_read("u-peer", "000101");
        assert!(state.already_read("u-peer", "101"));
        assert!(!state.already_read("u-peer", "102"));
        state.note_read("u-peer", "100");
        state.note_read("u-peer", "event:999");
        assert!(state.already_read("u-peer", "101"));
        assert!(!state.accept("u-peer", "101", 0));
        assert!(state.accept("u-peer", "102", 1));
        assert!(!state.already_read("u-peer", "102"));
    }
    #[test]
    fn older_phone_reads_do_not_clear_newer_incoming_messages() {
        let mut state = UnreadSync::default();
        state.observe("u-peer", "9007199254740993");
        assert!(!state.accept("u-peer", "9007199254740992", 0));
        assert!(state.accept("u-peer", "9007199254740993", 0));
        assert!(state.already_read("u-peer", "9007199254740992"));
        state.observe("u-peer", "9007199254740994");
        assert!(!state.already_read("u-peer", "9007199254740994"));
        assert!(!state.accept("u-peer", "9007199254740993", 0));
        assert!(state.accept("u-peer", "9007199254740994", 1));
        assert!(!state.already_read("u-peer", "9007199254740994"));
        assert!(state.accept("u-peer", "9007199254740994", 0));
    }
    #[test]
    fn invalid_updates_and_other_chats_cannot_change_the_read_frontier() {
        let mut state = UnreadSync::default();
        assert!(!state.accept("c-group", "pending-1", 0));
        assert!(!state.accept("c-group", "11", -1));
        assert!(state.accept("c-group", "00010", 0));
        assert!(state.already_read("c-group", "9"));
        assert!(!state.already_read("u-other", "9"));
        assert!(!state.already_read("c-group", "pending-1"));
        assert!(!state.accept("c-group", "9", 0));
    }
}
