use super::*;
use std::collections::VecDeque;

const MAX_CHATS: usize = 10;
const MAX_MESSAGES: usize = 80;

/// What differs between the rows on screen and a freshly loaded list.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Patch {
    /// New messages after the last one shown.
    pub appended: Vec<MessageInfo>,
    /// Messages whose content changed and need new rows.
    pub replaced: Vec<MessageInfo>,
    /// Messages where only the reactions changed.
    pub reacted: Vec<MessageInfo>,
    /// Messages the sender took back.
    pub unsent: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub(super) enum MessageDiff {
    Unchanged,
    Patch(Patch),
    Rebuild,
}

/// Compare the rows on screen with a freshly loaded list (both oldest first).
/// Messages are matched by id, so new, changed and taken-back messages are
/// updated one by one. Only a missing or reordered message forces a rebuild.
pub(super) fn diff_messages(shown: &[MessageInfo], incoming: &[MessageInfo]) -> MessageDiff {
    let Some(last) = shown.last() else {
        return MessageDiff::Rebuild;
    };
    let Some(pos) = incoming.iter().position(|m| m.id == last.id) else {
        return MessageDiff::Rebuild;
    };
    let overlap = pos + 1;
    if shown.len() < overlap {
        return MessageDiff::Rebuild;
    }
    let mut patch = Patch::default();
    for (old, new) in shown[shown.len() - overlap..]
        .iter()
        .zip(&incoming[..overlap])
    {
        if old.id != new.id {
            return MessageDiff::Rebuild;
        }
        if same_row((old, new)) {
            continue;
        }
        if old.id.is_empty() || old.chat_event.is_some() || new.chat_event.is_some() {
            return MessageDiff::Rebuild;
        }
        let was_unsent = old.content_type == "UNSENT";
        let is_unsent = new.content_type == "UNSENT";
        if is_unsent && !was_unsent {
            patch.unsent.push(new.id.clone());
        } else if was_unsent {
            return MessageDiff::Rebuild;
        } else if only_reactions_differ(old, new) {
            patch.reacted.push(new.clone());
        } else {
            patch.replaced.push(new.clone());
        }
    }
    patch.appended = incoming[overlap..].to_vec();
    if patch == Patch::default() {
        MessageDiff::Unchanged
    } else {
        MessageDiff::Patch(patch)
    }
}

fn only_reactions_differ(old: &MessageInfo, new: &MessageInfo) -> bool {
    let mut old = old.clone();
    old.reactions = new.reactions.clone();
    same_row((&old, new))
}

/// A media file the screen already shows is not a change, wherever it came from.
fn same_row((shown, incoming): (&MessageInfo, &MessageInfo)) -> bool {
    if shown == incoming {
        return true;
    }
    let mut incoming = incoming.clone();
    for (kept, field) in [
        (&shown.image_path, &mut incoming.image_path),
        (&shown.audio_path, &mut incoming.audio_path),
        (&shown.file_path, &mut incoming.file_path),
    ] {
        if kept.is_some() {
            *field = kept.clone();
        }
    }
    *shown == incoming
}

type Key = (String, String);

#[derive(Default)]
struct Snapshots {
    entries: VecDeque<(Key, Vec<MessageInfo>)>,
}

impl Snapshots {
    fn key(account: &str, chat: &str) -> Key {
        (account.to_string(), chat.to_string())
    }

    fn put(&mut self, account: &str, chat: &str, messages: &[MessageInfo]) {
        let key = Self::key(account, chat);
        self.entries.retain(|(k, _)| *k != key);
        if messages.is_empty() {
            return;
        }
        let start = messages.len().saturating_sub(MAX_MESSAGES);
        self.entries.push_front((key, messages[start..].to_vec()));
        self.entries.truncate(MAX_CHATS);
    }

    fn get(&mut self, account: &str, chat: &str) -> Option<Vec<MessageInfo>> {
        let key = Self::key(account, chat);
        let index = self.entries.iter().position(|(k, _)| *k == key)?;
        let entry = self.entries.remove(index)?;
        let messages = entry.1.clone();
        self.entries.push_front(entry);
        Some(messages)
    }

    fn push_live(&mut self, account: &str, chat: &str, message: &MessageInfo) {
        let key = Self::key(account, chat);
        let Some((_, list)) = self.entries.iter_mut().find(|(k, _)| *k == key) else {
            return;
        };
        if list.iter().any(|m| m.id == message.id) {
            return;
        }
        list.push(message.clone());
        list.sort_by_key(|m| m.created_time);
        if list.len() > MAX_MESSAGES {
            list.remove(0);
        }
    }

    fn forget(&mut self, account: &str, chat: &str) {
        let key = Self::key(account, chat);
        self.entries.retain(|(k, _)| *k != key);
    }
}

thread_local! {
    static SNAPSHOTS: RefCell<Snapshots> = RefCell::new(Snapshots::default());
}

fn account(state: &AppState) -> Option<String> {
    state.self_mid.borrow().clone()
}

/// Remember what the open chat shows before another chat replaces it.
pub(super) fn leave(state: &AppState) {
    let (Some(account), Some(chat)) = (account(state), state.current_chat.borrow().clone()) else {
        return;
    };
    let shown = match state.msg_list_fp.borrow().as_ref() {
        Some((mid, list)) if *mid == chat && !history::is_detached(state) => Some(list.clone()),
        _ => None,
    };
    SNAPSHOTS.with(|s| match shown {
        Some(list) => s.borrow_mut().put(&account, &chat, &list),
        None => s.borrow_mut().forget(&account, &chat),
    });
}

pub(super) fn take(state: &AppState, chat: &str) -> Option<Vec<MessageInfo>> {
    let account = account(state)?;
    SNAPSHOTS.with(|s| s.borrow_mut().get(&account, chat))
}

/// A message arrived for a chat that is not on screen.
pub(super) fn push_live(state: &AppState, chat: &str, message: &MessageInfo) {
    if let Some(account) = account(state) {
        SNAPSHOTS.with(|s| s.borrow_mut().push_live(&account, chat, message));
    }
}

pub(super) fn forget(state: &AppState, chat: &str) {
    if let Some(account) = account(state) {
        SNAPSHOTS.with(|s| s.borrow_mut().forget(&account, chat));
    }
}

pub(super) fn clear() {
    SNAPSHOTS.with(|s| s.borrow_mut().entries.clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(n: i64) -> MessageInfo {
        serde_json::from_value(serde_json::json!({
            "id": n.to_string(),
            "text": format!("message {n}"),
            "createdTime": n * 1000,
        }))
        .unwrap()
    }

    fn range(from: i64, to: i64) -> Vec<MessageInfo> {
        (from..=to).map(msg).collect()
    }

    fn ids(list: &[MessageInfo]) -> Vec<String> {
        list.iter().map(|m| m.id.clone()).collect()
    }

    #[test]
    fn same_list_is_unchanged() {
        assert_eq!(
            diff_messages(&range(1, 40), &range(1, 40)),
            MessageDiff::Unchanged
        );
    }

    fn patch(diff: MessageDiff) -> Patch {
        match diff {
            MessageDiff::Patch(patch) => patch,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn new_messages_at_the_end_are_appended() {
        let p = patch(diff_messages(&range(1, 40), &range(1, 43)));
        assert_eq!(ids(&p.appended), ["41", "42", "43"]);
        assert!(p.replaced.is_empty() && p.reacted.is_empty() && p.unsent.is_empty());
    }

    #[test]
    fn window_shifted_forward_still_appends() {
        let p = patch(diff_messages(&range(1, 40), &range(6, 45)));
        assert_eq!(p.appended.len(), 5);
    }

    #[test]
    fn screen_with_extra_older_rows_is_unchanged() {
        assert_eq!(
            diff_messages(&range(1, 80), &range(41, 80)),
            MessageDiff::Unchanged
        );
    }

    #[test]
    fn changed_text_replaces_only_that_message() {
        let mut incoming = range(1, 41);
        incoming[10].text = "decrypted".into();
        let p = patch(diff_messages(&range(1, 40), &incoming));
        assert_eq!(ids(&p.replaced), ["11"]);
        assert_eq!(ids(&p.appended), ["41"]);
    }

    #[test]
    fn reaction_only_changes_are_not_row_replacements() {
        let mut incoming = range(1, 40);
        incoming[5].reactions =
            serde_json::from_value(serde_json::json!([{"from": "u1", "kind": "LIKE"}])).unwrap();
        let p = patch(diff_messages(&range(1, 40), &incoming));
        assert_eq!(ids(&p.reacted), ["6"]);
        assert!(p.replaced.is_empty());
    }

    #[test]
    fn taken_back_messages_are_reported() {
        let mut incoming = range(1, 40);
        incoming[3].content_type = "UNSENT".into();
        let p = patch(diff_messages(&range(1, 40), &incoming));
        assert_eq!(p.unsent, ["4"]);
        assert!(p.replaced.is_empty());
    }

    #[test]
    fn media_found_later_replaces_but_known_media_does_not() {
        let mut incoming = range(1, 3);
        incoming[1].image_path = Some("/cache/a.png".into());
        let p = patch(diff_messages(&range(1, 3), &incoming));
        assert_eq!(ids(&p.replaced), ["2"]);

        let mut shown = range(1, 3);
        shown[1].image_path = Some("/cache/thumb.png".into());
        assert_eq!(diff_messages(&shown, &incoming), MessageDiff::Unchanged);
        assert_eq!(diff_messages(&shown, &range(1, 3)), MessageDiff::Unchanged);
    }

    #[test]
    fn missing_or_reordered_messages_rebuild() {
        let mut gap = range(1, 40);
        gap.remove(20);
        assert_eq!(diff_messages(&range(1, 40), &gap), MessageDiff::Rebuild);
        assert_eq!(
            diff_messages(&range(21, 40), &range(1, 40)),
            MessageDiff::Rebuild
        );
    }

    #[test]
    fn restored_messages_and_chat_events_rebuild() {
        let mut shown = range(1, 3);
        shown[1].content_type = "UNSENT".into();
        assert_eq!(diff_messages(&shown, &range(1, 3)), MessageDiff::Rebuild);

        let mut incoming = range(1, 3);
        incoming[1].chat_event = serde_json::from_value(serde_json::json!({
            "kind": "joined", "actorMid": "u1", "targetMids": []
        }))
        .ok();
        assert!(incoming[1].chat_event.is_some());
        assert_eq!(diff_messages(&range(1, 3), &incoming), MessageDiff::Rebuild);
    }

    #[test]
    fn disjoint_or_empty_lists_rebuild() {
        assert_eq!(
            diff_messages(&range(1, 40), &range(100, 140)),
            MessageDiff::Rebuild
        );
        assert_eq!(diff_messages(&[], &range(1, 3)), MessageDiff::Rebuild);
        assert_eq!(diff_messages(&range(1, 3), &[]), MessageDiff::Rebuild);
    }

    #[test]
    fn snapshots_keep_only_recent_chats() {
        let mut s = Snapshots::default();
        for n in 0..12 {
            s.put("me", &format!("c{n}"), &range(1, 3));
        }
        assert_eq!(s.entries.len(), MAX_CHATS);
        assert!(s.get("me", "c0").is_none());
        assert!(s.get("me", "c11").is_some());
    }

    #[test]
    fn snapshots_are_separate_per_account() {
        let mut s = Snapshots::default();
        s.put("alice", "c1", &range(1, 3));
        assert!(s.get("bob", "c1").is_none());
        assert!(s.get("alice", "c1").is_some());
    }

    #[test]
    fn snapshots_trim_to_the_newest_messages() {
        let mut s = Snapshots::default();
        s.put("me", "c1", &range(1, 200));
        let list = s.get("me", "c1").unwrap();
        assert_eq!(list.len(), MAX_MESSAGES);
        assert_eq!(list.last().unwrap().id, "200");
    }

    #[test]
    fn live_messages_join_only_existing_snapshots_once() {
        let mut s = Snapshots::default();
        s.push_live("me", "c1", &msg(4));
        assert!(s.get("me", "c1").is_none());
        s.put("me", "c1", &range(1, 3));
        s.push_live("me", "c1", &msg(4));
        s.push_live("me", "c1", &msg(4));
        assert_eq!(ids(&s.get("me", "c1").unwrap()), ["1", "2", "3", "4"]);
    }

    #[test]
    fn forgotten_snapshots_are_gone() {
        let mut s = Snapshots::default();
        s.put("me", "c1", &range(1, 3));
        s.forget("me", "c1");
        assert!(s.get("me", "c1").is_none());
    }
}
