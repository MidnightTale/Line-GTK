use super::virtual_list::{Edge, Trimmed};
use super::*;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Messages requested per page when scrolling through history.
pub(super) const PAGE_SIZE: u32 = 50;
/// Messages kept in the list while paging; the far end is dropped beyond this.
pub(super) const WINDOW_MAX: usize = 200;
/// A long-running live chat is trimmed back to `WINDOW_MAX` past this size.
const LIVE_TRIM_AT: usize = 300;
const COOLDOWN: Duration = Duration::from_millis(400);
const RETRY_DELAY: Duration = Duration::from_secs(3);
const MAX_FAILURES: u8 = 3;
/// Keep a missing or very old quote from walking the entire account history.
const MAX_JUMP_PAGES: u32 = 100;

struct Navigation {
    message_id: String,
    chat_mid: String,
    account: Option<String>,
    generation: u64,
    pages: u32,
    settling_until: Option<Instant>,
}

impl Navigation {
    fn matches(&self, chat_mid: Option<&str>, account: Option<&str>, generation: u64) -> bool {
        chat_mid == Some(self.chat_mid.as_str())
            && account == self.account.as_deref()
            && generation == self.generation
    }
}

#[derive(Default)]
pub(super) struct HistoryWindow {
    /// Sentinels only react once the first page of a chat finished rendering.
    armed: bool,
    navigation: Option<Navigation>,
    has_more_older: bool,
    /// The list ends before the newest message because newer rows were dropped.
    has_more_newer: bool,
    older_request: Option<u64>,
    newer_request: Option<u64>,
    older_ready_at: Option<Instant>,
    newer_ready_at: Option<Instant>,
    older_failures: u8,
    newer_failures: u8,
}

impl HistoryWindow {
    fn request(&mut self, edge: Edge) -> &mut Option<u64> {
        match edge {
            Edge::Top => &mut self.older_request,
            Edge::Bottom => &mut self.newer_request,
        }
    }
}

/// Open the bounded source-message window supplied by the shared-media cache.
/// This path never loads the latest messages or changes the read frontier.
pub(super) fn jump_to_shared_message(
    state: &AppState,
    chat_mid: &str,
    message_id: &str,
    created_time: i64,
) {
    jump_to_context(
        state,
        chat_mid,
        message_id,
        created_time,
        "fetch_shared_message_context",
    );
}

pub(super) fn jump_to_search_message(state: &AppState, chat_mid: &str, message_id: &str) {
    jump_to_context(
        state,
        chat_mid,
        message_id,
        0,
        "fetch_search_message_context",
    );
}

fn jump_to_context(
    state: &AppState,
    chat_mid: &str,
    message_id: &str,
    created_time: i64,
    method: &str,
) {
    if message_id.parse::<u128>().is_err() || !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("reply_original_unavailable"));
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let chat = state
        .chats
        .borrow()
        .iter()
        .find(|chat| chat.mid == chat_mid)
        .cloned();
    let Some(chat) = chat else {
        toast(state, &crate::i18n::t("reply_original_unavailable"));
        return;
    };
    let changed_chat = state.current_chat.borrow().as_deref() != Some(chat_mid);
    if !open_chat_for_history(state, &chat) {
        return;
    }
    if changed_chat {
        clear_messages(state);
        *state.msg_list_fp.borrow_mut() = None;
    }

    let generation = state.history_request_generation.borrow().wrapping_add(1);
    *state.history_request_generation.borrow_mut() = generation;
    // Cancel any idle rendering and scroll pinning left by a latest-page load.
    let render_generation = state.message_render_generation.borrow().wrapping_add(1);
    *state.message_render_generation.borrow_mut() = render_generation;
    *state.render_batch.borrow_mut() = None;
    *state.scroll_pin_gen.borrow_mut() += 1;
    *state.scroll_pinning.borrow_mut() = false;
    *state.stick_bottom.borrow_mut() = false;
    *state.history.borrow_mut() = HistoryWindow {
        has_more_newer: true,
        ..HistoryWindow::default()
    };
    state.message_list.set_sentinel(Edge::Top, false);
    state.message_list.set_sentinel(Edge::Bottom, false);
    set_msg_state(state, "loading", None);
    update_jump_banner(state);
    match state.sidecar.request(
        method,
        Some(serde_json::json!({
            "chatMid": chat_mid,
            "messageId": message_id,
            "createdTime": created_time,
            "limit": PAGE_SIZE,
        })),
    ) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::SharedMessageContext {
                    account,
                    chat_mid: chat_mid.into(),
                    message_id: message_id.into(),
                    generation,
                },
            );
        }
        Err(_) => shared_context_failed(state),
    }
}

pub(super) fn shared_context_response(
    state: &AppState,
    pending: Pending,
    ok: bool,
    result: &serde_json::Value,
) {
    let Pending::SharedMessageContext {
        account,
        chat_mid,
        message_id,
        generation,
    } = pending
    else {
        return;
    };
    let owner = Navigation {
        message_id: message_id.clone(),
        chat_mid: chat_mid.clone(),
        account: Some(account),
        generation,
        pages: 0,
        settling_until: None,
    };
    if !*state.session_ready.borrow()
        || !owner.matches(
            state.current_chat.borrow().as_deref(),
            state.self_mid.borrow().as_deref(),
            *state.history_request_generation.borrow(),
        )
    {
        return;
    }
    let messages = ok
        .then(|| shared_context_messages(result, &chat_mid, &message_id))
        .flatten();
    let Some(messages) = messages else {
        shared_context_failed(state);
        return;
    };
    let messages: Vec<_> = messages
        .iter()
        .map(|message| unsend::masked(state, message))
        .collect();
    if !messages
        .iter()
        .any(|message| message.id == message_id && message.content_type != "UNSENT")
    {
        shared_context_failed(state);
        return;
    }

    clear_messages(state);
    *state.msg_list_fp.borrow_mut() = None;
    *state.stick_bottom.borrow_mut() = false;
    *state.history.borrow_mut() = HistoryWindow {
        has_more_older: result
            .get("hasMoreOlder")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        // The source view remains detached until the user returns to latest.
        has_more_newer: true,
        ..HistoryWindow::default()
    };
    for message in messages {
        append_message(state, &message, false);
    }
    set_msg_state(state, "list", None);
    update_chat_subtitle(state);
    jump_to_message(state, &message_id);
    arm(state);
    update_jump_banner(state);
    state.window.present();
}

fn shared_context_failed(state: &AppState) {
    if state.message_list.message_count() > 0 {
        set_msg_state(state, "list", None);
    } else {
        set_msg_state(
            state,
            "empty",
            Some(&crate::i18n::t("reply_original_unavailable")),
        );
    }
    update_jump_banner(state);
    toast(state, &crate::i18n::t("reply_original_unavailable"));
}

/// Reject mismatched results and keep the source inside even an oversized page.
fn shared_context_messages(
    result: &serde_json::Value,
    chat_mid: &str,
    message_id: &str,
) -> Option<Vec<MessageInfo>> {
    if result.get("chatMid").and_then(|value| value.as_str()) != Some(chat_mid)
        || result
            .get("targetMessageId")
            .and_then(|value| value.as_str())
            != Some(message_id)
    {
        return None;
    }
    let mut messages = BTreeMap::new();
    for message in parse_messages(result) {
        if !message.id.is_empty() {
            messages.insert(message.id.clone(), message);
        }
    }
    let mut messages: Vec<_> = messages.into_values().collect();
    messages.sort_by(|a, b| {
        a.created_time.cmp(&b.created_time).then_with(|| {
            match (a.id.parse::<u128>(), b.id.parse::<u128>()) {
                (Ok(a), Ok(b)) => a.cmp(&b),
                _ => a.id.cmp(&b.id),
            }
        })
    });
    let target = messages
        .iter()
        .position(|message| message.id == message_id)?;
    if messages[target].content_type == "UNSENT" || messages[target].decryption_failed {
        return None;
    }
    let start = target.saturating_sub(WINDOW_MAX / 2);
    let end = (start + WINDOW_MAX).min(messages.len());
    Some(messages.drain(start..end).collect())
}

/// Find an original reply in the bounded message window, paging backwards when
/// needed. The existing request generation prevents replies from another chat
/// or a previous account from steering this view.
pub(super) fn jump_to_message(state: &AppState, message_id: &str) {
    if message_id.parse::<u128>().is_err() {
        toast(state, &crate::i18n::t("reply_original_unavailable"));
        return;
    }
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        return;
    };
    *state.stick_bottom.borrow_mut() = false;
    *state.scroll_pin_gen.borrow_mut() += 1;
    *state.scroll_pinning.borrow_mut() = false;
    state.history.borrow_mut().navigation = Some(Navigation {
        message_id: message_id.into(),
        chat_mid,
        account: state.self_mid.borrow().clone(),
        generation: *state.history_request_generation.borrow(),
        pages: 0,
        settling_until: None,
    });
    if advance_navigation(state) {
        let s = state.clone();
        glib::timeout_add_local(Duration::from_millis(150), move || {
            if advance_navigation(&s) {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }
}

/// True while a search still needs another page or allocation.
fn advance_navigation(state: &AppState) -> bool {
    let target = {
        let history = state.history.borrow();
        let Some(nav) = history.navigation.as_ref() else {
            return false;
        };
        if !nav.matches(
            state.current_chat.borrow().as_deref(),
            state.self_mid.borrow().as_deref(),
            *state.history_request_generation.borrow(),
        ) {
            drop(history);
            state.history.borrow_mut().navigation = None;
            return false;
        }
        nav.message_id.clone()
    };
    // Do not jump through a response still changing the window, or through
    // the initial chunked render which would pin the view back to the newest.
    {
        let history = state.history.borrow();
        if !history.armed
            || history.older_request.is_some()
            || history.newer_request.is_some()
            || *state.scroll_pinning.borrow()
        {
            return true;
        }
    }
    let settling_until = state
        .history
        .borrow()
        .navigation
        .as_ref()
        .and_then(|nav| nav.settling_until);
    if settling_until.is_some_and(|until| Instant::now() >= until) {
        state.history.borrow_mut().navigation = None;
        return false;
    }
    *state.stick_bottom.borrow_mut() = false;
    if state.message_list.scroll_to_message(&target) {
        if settling_until.is_none() {
            if let Some(nav) = state.history.borrow_mut().navigation.as_mut() {
                nav.settling_until = Some(Instant::now() + Duration::from_millis(600));
            }
            if let Some(row) = state.message_rows.borrow().get(&target).cloned() {
                row.add_css_class("line-reply-target");
                glib::timeout_add_local_once(Duration::from_secs(2), move || {
                    row.remove_css_class("line-reply-target");
                });
            }
        }
        // Keep sentinel callbacks suppressed while GTK allocates and moves the
        // virtual rows. Repeating also handles an offscreen row's first bind.
        return true;
    }
    let oldest = state.message_list.edge_owner(true, |_| true);
    let cannot_continue = {
        let history = state.history.borrow();
        let Some(nav) = history.navigation.as_ref() else {
            return false;
        };
        !history.has_more_older
            || nav.pages >= MAX_JUMP_PAGES
            || history.older_failures >= MAX_FAILURES
            || oldest
                .as_ref()
                .is_none_or(|oldest| passed_target(&oldest.id, &target))
    };
    if cannot_continue {
        state.history.borrow_mut().navigation = None;
        toast(state, &crate::i18n::t("reply_original_unavailable"));
        return false;
    }
    request(state, Edge::Top);
    true
}

fn passed_target(oldest: &str, target: &str) -> bool {
    match (oldest.parse::<u128>(), target.parse::<u128>()) {
        (Ok(oldest), Ok(target)) => oldest <= target,
        _ => false,
    }
}

pub(super) fn wire(state: &AppState) {
    let s = state.clone();
    state
        .message_list
        .set_edge_handler(move |edge| on_edge(&s, edge));
    // A sentinel only reports when it first comes into view. Watching the
    // scroll position as well keeps paging going when it stays attached.
    let s = state.clone();
    state
        .message_scroll
        .vadjustment()
        .connect_value_changed(move |_| {
            if *s.scroll_pinning.borrow() {
                return;
            }
            for edge in [Edge::Top, Edge::Bottom] {
                if near_edge(&s, edge) {
                    on_edge(&s, edge);
                }
            }
        });
}

/// Within a screen or two of the end of the list that has more history.
fn near_edge(state: &AppState, edge: Edge) -> bool {
    let adj = state.message_scroll.vadjustment();
    let margin = (adj.page_size() * 1.5).max(600.0);
    match edge {
        Edge::Top => adj.value() < margin,
        Edge::Bottom => adj.upper() - adj.value() - adj.page_size() < margin,
    }
}

/// Forget everything about the previous chat.
pub(super) fn reset(state: &AppState) {
    *state.history.borrow_mut() = HistoryWindow::default();
}

/// The list was replaced with the newest page of a chat.
pub(super) fn reset_for_latest(state: &AppState, has_messages: bool) {
    *state.history.borrow_mut() = HistoryWindow {
        has_more_older: has_messages,
        ..HistoryWindow::default()
    };
}

/// The first page finished rendering: start watching the top of the list.
pub(super) fn arm(state: &AppState) {
    {
        let mut history = state.history.borrow_mut();
        if history.armed {
            return;
        }
        history.armed = true;
    }
    restore_sentinels(state);
}

/// Show the sentinels that match the window after the rows were rebuilt.
pub(super) fn restore_sentinels(state: &AppState) {
    let history = state.history.borrow();
    if !history.armed {
        return;
    }
    state
        .message_list
        .set_sentinel(Edge::Top, history.has_more_older);
    state
        .message_list
        .set_sentinel(Edge::Bottom, history.has_more_newer);
}

pub(super) fn is_detached(state: &AppState) -> bool {
    state.history.borrow().has_more_newer
}

/// Newest messages are not in the list, so a live row would land in a gap.
/// Reload the latest page instead.
pub(super) fn reload_latest(state: &AppState) {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        return;
    };
    let generation = state.history_request_generation.borrow().wrapping_add(1);
    *state.history_request_generation.borrow_mut() = generation;
    *state.msg_list_fp.borrow_mut() = None;
    match state
        .sidecar
        .fetch_messages_forced(&chat_mid, PAGE_SIZE.min(40), true)
    {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::FetchMessages {
                    chat_mid,
                    generation,
                },
            );
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("fetch_failed", &[("error", &e.to_string())]),
        ),
    }
}

/// Keep a long live session from growing without bound.
pub(super) fn enforce_live_limit(state: &AppState) {
    if is_detached(state) || state.message_list.message_count() <= LIVE_TRIM_AT {
        return;
    }
    let trimmed = state.message_list.trim_front(WINDOW_MAX);
    if trimmed.ids.is_empty() {
        return;
    }
    cleanup(state, &trimmed);
    state.history.borrow_mut().has_more_older = true;
    ensure_top_separator(state);
    restore_sentinels(state);
}

fn is_real_message(state: &AppState, id: &str) -> bool {
    !id.starts_with("pending-")
        && state
            .reply_messages
            .borrow()
            .get(id)
            .is_some_and(|m| m.chat_event.is_none())
}

fn on_edge(state: &AppState, edge: Edge) {
    let ready = {
        let history = state.history.borrow();
        history.armed
            && history.navigation.is_none()
            && match edge {
                Edge::Top => history.has_more_older,
                Edge::Bottom => history.has_more_newer,
            }
    };
    if ready {
        request(state, edge);
    }
}

fn request(state: &AppState, edge: Edge) {
    // Saved messages only: older pages need the login, so look again later.
    if !*state.session_ready.borrow() {
        recheck_after(state, edge, Duration::from_secs(1));
        return;
    }
    let wait = {
        let mut history = state.history.borrow_mut();
        if history.request(edge).is_some() {
            return;
        }
        let ready_at = match edge {
            Edge::Top => history.older_ready_at,
            Edge::Bottom => history.newer_ready_at,
        };
        ready_at.and_then(|at| at.checked_duration_since(Instant::now()))
    };
    if let Some(wait) = wait {
        recheck_after(state, edge, wait);
        return;
    }
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        return;
    };
    let anchor = state
        .message_list
        .edge_owner(edge == Edge::Top, |owner| is_real_message(state, &owner.id));
    let Some(anchor) = anchor else {
        return;
    };
    let result = match edge {
        Edge::Top => {
            state
                .sidecar
                .fetch_older_messages(&chat_mid, &anchor.id, anchor.created, PAGE_SIZE)
        }
        Edge::Bottom => {
            state
                .sidecar
                .fetch_newer_messages(&chat_mid, &anchor.id, anchor.created, PAGE_SIZE)
        }
    };
    match result {
        Ok(id) => {
            let generation = *state.history_request_generation.borrow();
            let pending = match edge {
                Edge::Top => Pending::FetchOlder {
                    chat_mid,
                    generation,
                },
                Edge::Bottom => Pending::FetchNewer {
                    chat_mid,
                    generation,
                },
            };
            state.pending.borrow_mut().insert(id, pending);
            {
                let mut history = state.history.borrow_mut();
                *history.request(edge) = Some(id);
                if edge == Edge::Top
                    && let Some(nav) = history.navigation.as_mut()
                {
                    nav.pages += 1;
                }
            }
            state.message_list.set_sentinel_loading(edge, true);
            tracing::info!(?edge, anchor = %anchor.id, "[history] requested a page");
        }
        Err(e) => {
            eprintln!("[history] request failed: {e}");
            if state.history.borrow_mut().navigation.take().is_some() {
                toast(state, &crate::i18n::t("reply_original_unavailable"));
            }
        }
    }
}

/// Look again at an edge after a delay: a retry after a failure, or a short
/// list whose sentinel never leaves the view.
fn recheck_after(state: &AppState, edge: Edge, wait: Duration) {
    let s = state.clone();
    glib::timeout_add_local_once(wait + Duration::from_millis(20), move || {
        if *s.scroll_pinning.borrow() {
            recheck_after(&s, edge, Duration::from_millis(200));
        } else if s.message_list.sentinel_in_view(edge) || near_edge(&s, edge) {
            on_edge(&s, edge);
        }
    });
}

/// A reply to `fetch_older_messages` / `fetch_newer_messages`.
pub(super) fn response(
    state: &AppState,
    request_id: u64,
    pending: Pending,
    ok: bool,
    result: &serde_json::Value,
) {
    let (edge, chat_mid, generation) = match pending {
        Pending::FetchOlder {
            chat_mid,
            generation,
        } => (Edge::Top, chat_mid, generation),
        Pending::FetchNewer {
            chat_mid,
            generation,
        } => (Edge::Bottom, chat_mid, generation),
        _ => return,
    };
    {
        let mut history = state.history.borrow_mut();
        let request = history.request(edge);
        if *request != Some(request_id) {
            return;
        }
        *request = None;
    }
    state.message_list.set_sentinel_loading(edge, false);
    if generation != *state.history_request_generation.borrow()
        || state.current_chat.borrow().as_deref() != Some(chat_mid.as_str())
    {
        return;
    }
    if !ok {
        tracing::warn!(?edge, "[history] page request failed");
        let retry = {
            let mut history = state.history.borrow_mut();
            let failures = match edge {
                Edge::Top => &mut history.older_failures,
                Edge::Bottom => &mut history.newer_failures,
            };
            *failures = failures.saturating_add(1);
            let retry = *failures < MAX_FAILURES;
            let ready_at = Some(Instant::now() + RETRY_DELAY);
            match edge {
                Edge::Top => history.older_ready_at = ready_at,
                Edge::Bottom => history.newer_ready_at = ready_at,
            }
            retry
        };
        if retry {
            recheck_after(state, edge, RETRY_DELAY);
        }
        return;
    }
    {
        let mut history = state.history.borrow_mut();
        match edge {
            Edge::Top => history.older_failures = 0,
            Edge::Bottom => history.newer_failures = 0,
        }
    }
    let messages = parse_messages(result);
    let has_more = result
        .get("hasMore")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    tracing::info!(
        ?edge,
        received = messages.len(),
        has_more,
        "[history] page received"
    );
    match edge {
        Edge::Top => apply_older(state, messages, has_more),
        Edge::Bottom => apply_newer(state, messages, has_more),
    }
    let ready_at = Some(Instant::now() + COOLDOWN);
    {
        let mut history = state.history.borrow_mut();
        match edge {
            Edge::Top => history.older_ready_at = ready_at,
            Edge::Bottom => history.newer_ready_at = ready_at,
        }
    }
    // Only a list that still does not fill the view needs another page now;
    // otherwise the user scrolling back to the sentinel asks for it.
    let adj = state.message_scroll.vadjustment();
    if adj.upper() <= adj.page_size() * 1.5 {
        recheck_after(state, edge, COOLDOWN);
    }
}

/// One malformed message must not drop the rest of the page.
fn parse_messages(result: &serde_json::Value) -> Vec<MessageInfo> {
    let Some(items) = result.get("messages").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match serde_json::from_value(item.clone()) {
            Ok(message) => Some(message),
            Err(e) => {
                tracing::warn!("[history] skipped a message that could not be read: {e}");
                None
            }
        })
        .collect()
}

fn unseen(state: &AppState, messages: Vec<MessageInfo>) -> Vec<MessageInfo> {
    let mut fresh: Vec<MessageInfo> = {
        let seen = state.seen_msg_ids.borrow();
        messages
            .iter()
            .map(|m| unsend::masked(state, m))
            .filter(|m| m.id.is_empty() || !seen.contains(&m.id))
            .collect()
    };
    fresh.sort_by_key(|m| m.created_time);
    fresh
}

fn apply_older(state: &AppState, messages: Vec<MessageInfo>, has_more: bool) {
    let fresh = unseen(state, messages);
    if fresh.is_empty() {
        state.history.borrow_mut().has_more_older = false;
        state.message_list.set_sentinel(Edge::Top, false);
        return;
    }
    let oldest = state.message_list.edge_owner(true, |_| true);
    let old_first_row = state.message_list.edge_row(true);
    let junction_day = oldest.as_ref().map(|owner| day_key(owner.created));
    let new_last_day = fresh.last().map(|m| day_key(m.created_time));

    // The newest row of this page and the oldest row on screen share a day, so
    // the day label already on screen would be repeated.
    if junction_day.is_some()
        && junction_day == new_last_day
        && let Some(row) = old_first_row.as_ref()
        && row.has_css_class("line-day-sep-row")
    {
        state.message_list.remove(row);
    }
    let anchor = capture_anchor(state, true);

    let previous_day = state.last_msg_day.borrow().clone();
    let previous_incoming = state.last_incoming_id.borrow().clone();
    *state.last_msg_day.borrow_mut() = None;
    state.message_list.begin_prepend();
    for message in &fresh {
        append_message(state, message, false);
    }
    state.message_list.end_prepend();
    *state.last_msg_day.borrow_mut() = previous_day;
    *state.last_incoming_id.borrow_mut() = previous_incoming;

    state.history.borrow_mut().has_more_older = has_more;
    if state.message_list.message_count() > WINDOW_MAX {
        let trimmed = state.message_list.trim_back(WINDOW_MAX);
        if !trimmed.ids.is_empty() {
            cleanup(state, &trimmed);
            state.history.borrow_mut().has_more_newer = true;
            let newest = state.message_list.edge_owner(false, |_| true);
            *state.last_msg_day.borrow_mut() = newest.map(|owner| day_key(owner.created));
        }
    }
    restore_sentinels(state);
    update_chat_subtitle(state);
    restore_anchor(state, anchor);
}

fn apply_newer(state: &AppState, messages: Vec<MessageInfo>, has_more: bool) {
    let fresh = unseen(state, messages);
    if !has_more || fresh.is_empty() {
        // The stored history ends here. Live messages are not stored, so show
        // the real newest page.
        state.history.borrow_mut().has_more_newer = false;
        state.message_list.set_sentinel(Edge::Bottom, false);
        reload_latest(state);
        return;
    }
    let anchor = capture_anchor(state, false);
    for message in &fresh {
        append_message(state, message, false);
    }
    if state.message_list.message_count() > WINDOW_MAX {
        let trimmed = state.message_list.trim_front(WINDOW_MAX);
        if !trimmed.ids.is_empty() {
            cleanup(state, &trimmed);
            state.history.borrow_mut().has_more_older = true;
            ensure_top_separator(state);
        }
    }
    restore_sentinels(state);
    update_chat_subtitle(state);
    restore_anchor(state, anchor);
}

fn update_chat_subtitle(state: &AppState) {
    state.chat_subtitle.set_text(&crate::i18n::tf(
        "messages_count",
        &[("n", &state.message_list.message_count().to_string())],
    ));
}

/// After dropping the oldest rows the first message may have lost its day label.
fn ensure_top_separator(state: &AppState) {
    let Some(first) = state.message_list.edge_owner(true, |_| true) else {
        return;
    };
    if state
        .message_list
        .edge_row(true)
        .is_some_and(|row| row.has_css_class("line-day-sep-row"))
        || first.created <= 0
    {
        return;
    }
    let created = first.created;
    state.message_list.begin_prepend();
    state.message_list.set_owner(Some(first));
    state.message_list.append(&day_separator_row(created));
    state.message_list.set_owner(None);
    state.message_list.end_prepend();
}

/// Release everything the dropped rows were holding on to.
pub(super) fn cleanup(state: &AppState, trimmed: &Trimmed) {
    for id in &trimmed.ids {
        state.reply_messages.borrow_mut().remove(id);
        state.message_rows.borrow_mut().remove(id);
        reactions::forget(state, id);
        state.seen_msg_ids.borrow_mut().remove(id);
        state.media_slots.borrow_mut().remove(id);
        state.media_msgs.borrow_mut().remove(id);
        state.receipt_slots.borrow_mut().remove(id);
        state.msg_created.borrow_mut().remove(id);
        state.media_ready_paths.borrow_mut().remove(id);
    }
    state
        .media_queue
        .borrow_mut()
        .retain(|(id, _)| !trimmed.ids.contains(id));
    let inside = |widget: &gtk::Widget| trimmed.widgets.iter().any(|row| widget.is_ancestor(row));
    for labels in state.sender_labels.borrow_mut().values_mut() {
        labels.retain(|label| !inside(label.upcast_ref()));
    }
    state
        .chat_event_labels
        .borrow_mut()
        .retain(|(_, label)| !inside(label.upcast_ref()));
    state
        .emoji_views
        .borrow_mut()
        .retain(|view| !trimmed.widgets.iter().any(|row| view.is_inside(row)));
    let new_sep_removed = state.new_sep_row.borrow().as_ref().is_some_and(|row| {
        trimmed
            .widgets
            .iter()
            .any(|w| w == row.upcast_ref::<gtk::Widget>())
    });
    if new_sep_removed {
        *state.new_sep_row.borrow_mut() = None;
    }
}

struct Anchor {
    row: Option<(gtk::Widget, f64)>,
    upper: f64,
    value: f64,
    prepend: bool,
}

/// Remember where an edge row sits in the view so it can be put back after
/// rows are added or removed around it.
fn capture_anchor(state: &AppState, oldest: bool) -> Anchor {
    *state.stick_bottom.borrow_mut() = false;
    let adj = state.message_scroll.vadjustment();
    let row = state.message_list.edge_row(oldest).and_then(|row| {
        let y = row
            .compute_bounds(&state.message_scroll)
            .map(|rect| rect.y() as f64)?;
        Some((row, y))
    });
    Anchor {
        row,
        upper: adj.upper(),
        value: adj.value(),
        prepend: oldest,
    }
}

fn restore_anchor(state: &AppState, anchor: Anchor) {
    if state.history.borrow().navigation.is_some() {
        return;
    }
    let generation = {
        let mut generation = state.scroll_pin_gen.borrow_mut();
        *generation += 1;
        *generation
    };
    *state.scroll_pinning.borrow_mut() = true;
    let s = state.clone();
    let frames = Cell::new(0_u32);
    let steady = Cell::new(0_u32);
    state.message_scroll.add_tick_callback(move |_, _| {
        if *s.scroll_pin_gen.borrow() != generation {
            return glib::ControlFlow::Break;
        }
        frames.set(frames.get() + 1);
        let adj = s.message_scroll.vadjustment();
        let measured = anchor
            .row
            .as_ref()
            .and_then(|(row, wanted)| Some((row.compute_bounds(&s.message_scroll)?, *wanted)));
        match measured {
            Some((rect, wanted_y)) => {
                let delta = rect.y() as f64 - wanted_y;
                if delta.abs() > 0.5 {
                    adj.set_value(adj.value() + delta);
                    steady.set(0);
                } else {
                    steady.set(steady.get() + 1);
                }
            }
            None if anchor.prepend => {
                // The row is not on screen yet: keep the same distance from the
                // bottom by the amount the content grew.
                let wanted = anchor.value + (adj.upper() - anchor.upper);
                if (adj.value() - wanted).abs() > 0.5 {
                    adj.set_value(wanted);
                    steady.set(0);
                } else {
                    steady.set(steady.get() + 1);
                }
            }
            None => steady.set(steady.get() + 1),
        }
        if steady.get() >= 6 || frames.get() >= 90 {
            *s.scroll_pinning.borrow_mut() = false;
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    #[test]
    fn navigation_is_scoped_to_chat_account_and_request_generation() {
        let nav = Navigation {
            message_id: "42".into(),
            chat_mid: "group".into(),
            account: Some("alice".into()),
            generation: 4,
            pages: 0,
            settling_until: None,
        };
        assert!(nav.matches(Some("group"), Some("alice"), 4));
        assert!(!nav.matches(Some("other"), Some("alice"), 4));
        assert!(!nav.matches(Some("group"), Some("bob"), 4));
        assert!(!nav.matches(Some("group"), None, 4));
        assert!(!nav.matches(Some("group"), Some("alice"), 5));
        assert!(!nav.matches(None, Some("alice"), 4));
    }

    #[test]
    fn navigation_compares_large_message_ids_numerically() {
        assert!(passed_target("9007199254740993", "9007199254740994"));
        assert!(passed_target("42", "42"));
        assert!(!passed_target("100", "99"));
        assert!(!passed_target("pending-1", "42"));
    }

    #[test]
    fn shared_context_rejects_another_target_or_conversation_and_unavailable_sources() {
        let mut result = serde_json::json!({
            "chatMid": "group",
            "targetMessageId": "42",
            "messages": [{ "id": "42", "contentType": "IMAGE" }],
        });
        assert!(shared_context_messages(&result, "group", "42").is_some());
        assert!(shared_context_messages(&result, "other", "42").is_none());
        assert!(shared_context_messages(&result, "group", "43").is_none());
        result["messages"][0]["contentType"] = serde_json::json!("UNSENT");
        assert!(shared_context_messages(&result, "group", "42").is_none());
        result["messages"][0]["contentType"] = serde_json::json!("IMAGE");
        result["messages"][0]["decryptionFailed"] = serde_json::json!(true);
        assert!(shared_context_messages(&result, "group", "42").is_none());
        result["messages"] = serde_json::json!([]);
        assert!(shared_context_messages(&result, "group", "42").is_none());
    }

    #[test]
    fn shared_context_keeps_an_old_source_in_a_bounded_window() {
        let mut items: Vec<_> = (1..=500)
            .rev()
            .map(|id| serde_json::json!({ "id": id.to_string(), "createdTime": id }))
            .collect();
        items.push(serde_json::json!({ "id": "5", "createdTime": 5, "text": "fresh" }));
        items.push(serde_json::json!({ "id": "bad", "createdTime": "not a timestamp" }));
        let result = serde_json::json!({
            "chatMid": "group", "targetMessageId": "5", "messages": items,
        });
        let messages = shared_context_messages(&result, "group", "5").unwrap();
        assert_eq!(messages.len(), WINDOW_MAX);
        assert_eq!(messages.first().unwrap().id, "1");
        assert_eq!(messages.last().unwrap().id, "200");
        let sources: Vec<_> = messages
            .iter()
            .filter(|message| message.id == "5")
            .collect();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].text, "fresh");
    }

    #[test]
    fn shared_context_orders_equal_timestamps_by_exact_message_id() {
        let result = serde_json::json!({
            "chatMid": "group",
            "targetMessageId": "9007199254740993",
            "messages": [
                { "id": "9007199254740993", "createdTime": 1 },
                { "id": "10000000000000000", "createdTime": 1 },
                { "id": "9", "createdTime": 1 },
                { "id": "9007199254740992", "createdTime": 1 },
            ],
        });
        let messages = shared_context_messages(&result, "group", "9007199254740993").unwrap();
        let ids: Vec<_> = messages.iter().map(|message| message.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "9",
                "9007199254740992",
                "9007199254740993",
                "10000000000000000"
            ]
        );
    }
}
