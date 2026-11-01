//! Start-up from the saved cache: show the last chats and conversation while
//! the network login is still running, then refresh them in the background.
//! Text typed in the meantime waits in an outbox and is sent after the first
//! refresh, so messages that arrived while the app was closed stay above it.

use super::*;
use crate::protocol::{Mention, ReplyInfo};
use std::time::Duration;

/// Longest wait for the first refresh before queued messages are sent anyway.
const REFRESH_FALLBACK: Duration = Duration::from_secs(15);
const SETTLE_POLL: Duration = Duration::from_millis(60);
const SETTLE_TRIES: u32 = 100;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Queued {
    pub placeholder_id: String,
    pub account: Option<String>,
    pub chat_mid: String,
    pub text: String,
    pub mentions: Vec<Mention>,
    pub reply_to: Option<ReplyInfo>,
    pub created_time: i64,
}

#[derive(Debug, Default)]
pub(super) struct Outbox {
    items: Vec<Queued>,
    counter: u64,
}

impl Outbox {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Queue a message and return it with the id of its placeholder bubble.
    pub fn push(&mut self, mut item: Queued) -> Queued {
        self.counter += 1;
        item.placeholder_id = format!("pending-outbox-{}", self.counter);
        self.items.push(item.clone());
        item
    }

    pub fn take(&mut self) -> Vec<Queued> {
        std::mem::take(&mut self.items)
    }

    pub fn for_chat(&self, chat_mid: &str) -> Vec<Queued> {
        self.items
            .iter()
            .filter(|item| item.chat_mid == chat_mid)
            .cloned()
            .collect()
    }
}

/// Rows to move so `current` becomes `target`. `current` must only hold ids
/// that are also in `target`; ids missing from it are inserted.
pub(super) fn row_moves(current: &[String], target: &[String]) -> Vec<(String, usize)> {
    let mut order = current.to_vec();
    let mut moves = Vec::new();
    for (index, mid) in target.iter().enumerate() {
        if order.get(index) == Some(mid) {
            continue;
        }
        order.retain(|existing| existing != mid);
        order.insert(index.min(order.len()), mid.clone());
        moves.push((mid.clone(), index));
    }
    moves
}

/// True while the screen shows saved data and the first refresh is pending.
pub(super) fn active(state: &AppState) -> bool {
    *state.cache_preview.borrow()
}

/// The saved profile and chat list arrived before the login finished.
pub(super) fn show(
    state: &AppState,
    mid: &str,
    display_name: &str,
    status_message: &str,
    avatar_path: Option<&str>,
    picture_url: Option<&str>,
    chats: Vec<ChatInfo>,
) {
    if mid.is_empty()
        || chats.is_empty()
        || *state.session_ready.borrow()
        || !state.chats.borrow().is_empty()
    {
        return;
    }
    *state.cache_preview.borrow_mut() = true;
    apply_self_profile(state, mid, display_name, avatar_path, picture_url);
    state
        .profile_label
        .set_tooltip_text((!status_message.is_empty()).then_some(status_message));
    apply_chats(state, chats, true);
    state.status.set_text(&crate::i18n::t("restoring"));
}

/// Stored messages for the chat opened from the saved view.
pub(super) fn cached_messages(
    state: &AppState,
    chat_mid: &str,
    generation: u64,
    ok: bool,
    result: &serde_json::Value,
) {
    if !ok
        || generation != *state.history_request_generation.borrow()
        || state.current_chat.borrow().as_deref() != Some(chat_mid)
        || (*state.session_ready.borrow() && has_message_content(state))
    {
        return;
    }
    let messages: Vec<MessageInfo> = result
        .get("messages")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    // Nothing stored: keep the loading page until the network answers.
    if messages.is_empty() {
        return;
    }
    apply_messages(state, messages);
}

/// Call before the profile is applied: the saved account must be the one that
/// logged in, otherwise the saved view is dropped.
pub(super) fn verify_account(state: &AppState, account: &str) {
    if active(state)
        && state
            .self_mid
            .borrow()
            .as_deref()
            .is_some_and(|mid| mid != account)
    {
        discard(state);
    }
}

/// The login finished. Refreshes the open chat first when saved data is on
/// screen or messages are waiting; the outbox is sent once that is done.
pub(super) fn logged_in(state: &AppState) {
    let preview = active(state);
    let waiting = preview || !state.outbox.borrow().is_empty();
    let current = state.current_chat.borrow().clone();
    let Some(chat_mid) = current.filter(|_| waiting) else {
        end(state);
        return;
    };
    *state.cache_preview.borrow_mut() = true;
    match state.sidecar.fetch_messages_quiet(&chat_mid, 40) {
        Ok(id) => {
            set_syncing(state, true);
            state.pending.borrow_mut().insert(
                id,
                Pending::FetchMessages {
                    chat_mid,
                    generation: *state.history_request_generation.borrow(),
                },
            );
            let s = state.clone();
            glib::timeout_add_local_once(REFRESH_FALLBACK, move || end(&s));
        }
        Err(_) => end(state),
    }
}

/// The saved account is not the one that logged in: start from a clean screen.
fn discard(state: &AppState) {
    abort(state);
    *state.current_chat.borrow_mut() = None;
    *state.restored_last_chat.borrow_mut() = false;
    clear_messages(state);
    clear_list(&state.chat_list);
    state.chats.borrow_mut().clear();
    state.chat_avatars.borrow_mut().clear();
    state.chat_previews.borrow_mut().clear();
    state.chat_unread_badges.borrow_mut().clear();
    set_msg_state(state, "idle", None);
    set_side_state(state, "loading", None);
}

/// First refresh done (or given up): leave the preview and send what waited.
pub(super) fn end(state: &AppState) {
    if !active(state) {
        return;
    }
    *state.cache_preview.borrow_mut() = false;
    flush(state);
    if !*state.session_ready.borrow() {
        return;
    }
    let Some(mid) = state.current_chat.borrow().clone() else {
        return;
    };
    // Reading was held back while only saved messages were shown.
    if *state.stick_bottom.borrow() {
        mark_chat_read(state, &mid);
        clear_unread_badge(state, &mid);
    }
}

/// Login failed or ended: drop what waited and give the text back.
pub(super) fn abort(state: &AppState) {
    *state.cache_preview.borrow_mut() = false;
    let items = state.outbox.borrow_mut().take();
    if items.is_empty() {
        return;
    }
    for item in &items {
        give_back(state, item);
    }
    toast(state, &crate::i18n::t("outbox_not_sent"));
}

fn drafts(item: &Queued) -> (Option<replies::ReplyDraft>, Option<mentions::MentionDraft>) {
    let reply = item.reply_to.clone().map(|quote| replies::ReplyDraft {
        account: item.account.clone(),
        chat_mid: item.chat_mid.clone(),
        text: item.text.clone(),
        mentions: item.mentions.clone(),
        quote,
    });
    let mention = (!item.mentions.is_empty()).then(|| mentions::MentionDraft {
        account: item.account.clone(),
        chat_mid: item.chat_mid.clone(),
        text: item.text.clone(),
        mentions: item.mentions.clone(),
    });
    (reply, mention)
}

fn give_back(state: &AppState, item: &Queued) {
    remove_pending_placeholder(state, &item.placeholder_id);
    let (reply, mention) = drafts(item);
    if let Some(draft) = reply {
        replies::restore_failed(state, &draft);
    } else if let Some(draft) = mention {
        mentions::restore_failed(state, &draft);
    } else if *state.self_mid.borrow() == item.account
        && state.current_chat.borrow().as_deref() == Some(item.chat_mid.as_str())
        && state.composer.text().is_empty()
    {
        state.composer.set_text(&item.text);
    }
}

fn flush(state: &AppState) {
    let items = state.outbox.borrow_mut().take();
    let mut unsent = false;
    for item in items {
        if *state.self_mid.borrow() != item.account {
            give_back(state, &item);
            unsent = true;
            continue;
        }
        let quote = item.reply_to.as_ref().map(|q| q.message_id.as_str());
        match state
            .sidecar
            .send_mentions(&item.chat_mid, &item.text, quote, &item.mentions)
        {
            Ok(id) => {
                let (reply_draft, mention_draft) = drafts(&item);
                state.pending.borrow_mut().insert(
                    id,
                    Pending::Send {
                        chat_mid: item.chat_mid.clone(),
                        placeholder_id: item.placeholder_id.clone(),
                        reply_draft,
                        mention_draft,
                    },
                );
            }
            Err(_) => {
                give_back(state, &item);
                unsent = true;
            }
        }
    }
    if unsent {
        toast(state, &crate::i18n::t("outbox_not_sent"));
    }
}

fn placeholder(item: &Queued) -> MessageInfo {
    MessageInfo {
        chat_event: None,
        emojis: Vec::new(),
        mentions: item.mentions.clone(),
        reactions: Vec::new(),
        id: item.placeholder_id.clone(),
        text: item.text.clone(),
        from: String::new(),
        to: item.chat_mid.clone(),
        mine: true,
        created_time: item.created_time,
        content_type: "NONE".into(),
        image_path: None,
        audio_path: None,
        file_name: None,
        file_path: None,
        duration_ms: None,
        flex: None,
        call_history: None,
        decryption_failed: false,
        reply_to: item.reply_to.clone(),
    }
}

/// Hold a message back until the first refresh has been shown.
pub(super) fn enqueue(
    state: &AppState,
    chat_mid: &str,
    text: String,
    mentions: Vec<Mention>,
    reply_to: Option<ReplyInfo>,
) {
    let item = state.outbox.borrow_mut().push(Queued {
        placeholder_id: String::new(),
        account: state.self_mid.borrow().clone(),
        chat_mid: chat_mid.to_string(),
        text,
        mentions,
        reply_to,
        created_time: now_ms(),
    });
    if history::is_detached(state) {
        toast(state, &crate::i18n::t("outbox_waiting"));
        return;
    }
    if state.msg_stack.visible_child_name().as_deref() != Some("list") {
        set_msg_state(state, "list", None);
    }
    append_message(state, &placeholder(&item), true);
}

/// Keep waiting bubbles last after the list was patched or rebuilt.
pub(super) fn settle(state: &AppState) {
    if state.outbox.borrow().is_empty() {
        return;
    }
    let s = state.clone();
    let mut tries = 0;
    glib::timeout_add_local(SETTLE_POLL, move || {
        tries += 1;
        if s.render_batch.borrow().is_some() && tries < SETTLE_TRIES {
            return glib::ControlFlow::Continue;
        }
        redraw(&s);
        glib::ControlFlow::Break
    });
}

fn redraw(state: &AppState) {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        return;
    };
    let items = state.outbox.borrow().for_chat(&chat_mid);
    if items.is_empty() || history::is_detached(state) {
        return;
    }
    let stuck = *state.stick_bottom.borrow();
    for item in &items {
        remove_pending_placeholder(state, &item.placeholder_id);
    }
    if state.msg_stack.visible_child_name().as_deref() != Some("list") {
        set_msg_state(state, "list", None);
    }
    for item in &items {
        append_message(state, &placeholder(item), false);
    }
    if stuck {
        pin_messages_to_latest(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mids(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn apply(current: &[String], target: &[String]) -> Vec<String> {
        let mut order = current.to_vec();
        for (mid, index) in row_moves(current, target) {
            order.retain(|existing| *existing != mid);
            order.insert(index, mid);
        }
        order
    }

    fn queued(chat: &str, text: &str) -> Queued {
        Queued {
            placeholder_id: String::new(),
            account: Some("uaaa".into()),
            chat_mid: chat.into(),
            text: text.into(),
            mentions: Vec::new(),
            reply_to: None,
            created_time: 1,
        }
    }

    #[test]
    fn identical_order_needs_no_moves() {
        let order = mids(&["a", "b", "c"]);
        assert!(row_moves(&order, &order).is_empty());
    }

    #[test]
    fn moves_reach_the_target_order() {
        for (current, target) in [
            (vec!["a", "b", "c"], vec!["c", "a", "b"]),
            (vec!["a", "b", "c"], vec!["b", "a", "c"]),
            (vec!["a", "b", "c"], vec!["c", "b", "a"]),
            (vec!["a", "b"], vec!["n", "a", "b", "m"]),
            (vec![], vec!["a", "b"]),
        ] {
            let (current, target) = (mids(&current), mids(&target));
            assert_eq!(apply(&current, &target), target);
        }
    }

    #[test]
    fn a_new_activity_only_moves_that_chat() {
        let moves = row_moves(&mids(&["a", "b", "c", "d"]), &mids(&["c", "a", "b", "d"]));
        assert_eq!(moves, vec![("c".to_string(), 0)]);
    }

    #[test]
    fn outbox_keeps_order_and_gives_each_message_its_own_placeholder() {
        let mut outbox = Outbox::default();
        let first = outbox.push(queued("u1", "一つ目"));
        let second = outbox.push(queued("u2", "二つ目"));
        let third = outbox.push(queued("u1", "三つ目"));
        assert!(first.placeholder_id.starts_with("pending-"));
        assert_ne!(first.placeholder_id, second.placeholder_id);
        assert_ne!(second.placeholder_id, third.placeholder_id);
        let texts: Vec<_> = outbox
            .for_chat("u1")
            .into_iter()
            .map(|item| item.text)
            .collect();
        assert_eq!(texts, ["一つ目", "三つ目"]);
        let all: Vec<_> = outbox.take().into_iter().map(|item| item.text).collect();
        assert_eq!(all, ["一つ目", "二つ目", "三つ目"]);
        assert!(outbox.is_empty());
    }

    #[test]
    fn placeholder_ids_are_not_reused_after_a_flush() {
        let mut outbox = Outbox::default();
        let first = outbox.push(queued("u1", "a"));
        outbox.take();
        let second = outbox.push(queued("u1", "b"));
        assert_ne!(first.placeholder_id, second.placeholder_id);
    }

    #[test]
    fn drafts_follow_reply_and_mentions() {
        let mut item = queued("u1", "本文");
        let (reply, mention) = drafts(&item);
        assert!(reply.is_none() && mention.is_none());
        item.mentions = vec![Mention {
            start: 0,
            end: 1,
            mid: "u2".into(),
            all: false,
        }];
        let (reply, mention) = drafts(&item);
        assert!(reply.is_none());
        assert_eq!(mention.unwrap().text, "本文");
    }
}
