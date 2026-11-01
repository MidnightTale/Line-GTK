use super::*;
use std::time::{Duration, SystemTime};

#[derive(Default)]
pub(super) struct ResumeSync {
    generation: u64,
    busy: bool,
    scheduled: bool,
    sleeping: bool,
    closed: bool,
    attempts: usize,
    wake_time: Option<SystemTime>,
    subscription: Option<gio::SignalSubscription>,
}

fn missed_ticks(previous: SystemTime, now: SystemTime) -> bool {
    now.duration_since(previous)
        .is_ok_and(|elapsed| elapsed > Duration::from_secs(20))
}

pub(super) fn cancel(state: &AppState) {
    invalidate_history(state);
    let mut sync = state.resume_sync.borrow_mut();
    sync.generation = sync.generation.wrapping_add(1);
    sync.busy = false;
    sync.sleeping = false;
    sync.scheduled = false;
    sync.attempts = 0;
}

fn invalidate_history(state: &AppState) {
    let generation = state.history_request_generation.borrow().wrapping_add(1);
    *state.history_request_generation.borrow_mut() = generation;
    let generation = state.message_render_generation.borrow().wrapping_add(1);
    *state.message_render_generation.borrow_mut() = generation;
}

pub(super) fn stale_history_event(state: &AppState, mid: &str) -> bool {
    let generation = *state.history_request_generation.borrow();
    let pending = state.pending.borrow();
    let mut found = false;
    for request in pending.values() {
        if let Pending::FetchMessages {
            chat_mid,
            generation: requested,
        } = request
            && chat_mid == mid
        {
            if *requested == generation {
                return false;
            }
            found = true;
        }
    }
    found
}

pub(super) fn in_progress(state: &AppState) -> bool {
    let sync = state.resume_sync.borrow();
    sync.busy || sync.scheduled || sync.sleeping
}

pub(super) fn wire(state: &AppState) {
    let s = state.clone();
    let action = gio::SimpleAction::new("sync-after-resume", None);
    action.connect_activate(move |_, _| request(&s));
    state.window.add_action(&action);

    let s = state.clone();
    glib::spawn_future_local(async move {
        let Ok(bus) = gio::bus_get_future(gio::BusType::System).await else {
            // logindのない環境でも、時刻差とネットワーク復旧で再取得する。
            return;
        };
        let s2 = s.clone();
        let subscription = bus.subscribe_to_signal(
            Some("org.freedesktop.login1"),
            Some("org.freedesktop.login1.Manager"),
            Some("PrepareForSleep"),
            Some("/org/freedesktop/login1"),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if let Some((sleeping,)) = signal.parameters.get::<(bool,)>() {
                    if sleeping {
                        cancel(&s2);
                        s2.resume_sync.borrow_mut().sleeping = true;
                    } else {
                        let mut sync = s2.resume_sync.borrow_mut();
                        sync.sleeping = false;
                        sync.wake_time = Some(SystemTime::now());
                        drop(sync);
                        request(&s2);
                    }
                }
            },
        );
        if !s.resume_sync.borrow().closed {
            s.resume_sync.borrow_mut().subscription = Some(subscription);
        }
    });

    let monitor = gio::NetworkMonitor::default();
    let was_offline = Rc::new(std::cell::Cell::new(!monitor.is_network_available()));
    let s = state.clone();
    monitor.connect_network_changed(move |_, available| {
        // 初回通知でも実際のオフライン→オンラインなら復旧する。
        if !available {
            was_offline.set(true);
        } else if was_offline.replace(false) {
            request(&s);
        }
    });

    let s = state.clone();
    let mut previous = SystemTime::now();
    glib::timeout_add_local(Duration::from_secs(5), move || {
        if s.resume_sync.borrow().closed {
            return glib::ControlFlow::Break;
        }
        let now = SystemTime::now();
        if missed_ticks(previous, now) {
            let handled = s
                .resume_sync
                .borrow()
                .wake_time
                .is_some_and(|time| time >= previous && time <= now);
            if !handled {
                // 古い要求が休止前から残っていても、新しい世代で取り直す。
                cancel(&s);
                request(&s);
            }
        }
        previous = now;
        glib::ControlFlow::Continue
    });
    let sync = state.resume_sync.clone();
    state.app.connect_shutdown(move |_| {
        let mut sync = sync.borrow_mut();
        sync.closed = true;
        sync.subscription.take();
    });
}

fn valid(state: &AppState, generation: u64, account: &str) -> bool {
    let sync = state.resume_sync.borrow();
    !sync.closed
        && !sync.sleeping
        && sync.generation == generation
        && *state.session_ready.borrow()
        && !*state.restarting.borrow()
        && state.self_mid.borrow().as_deref() == Some(account)
}

fn request(state: &AppState) {
    if !*state.session_ready.borrow() || *state.restarting.borrow() {
        return;
    }
    {
        let mut sync = state.resume_sync.borrow_mut();
        if sync.closed || sync.sleeping || sync.busy || sync.scheduled {
            return;
        }
        sync.attempts = 0;
        sync.generation = sync.generation.wrapping_add(1);
    }
    schedule(state, 2);
}

fn schedule(state: &AppState, delay: u64) {
    let generation = {
        let mut sync = state.resume_sync.borrow_mut();
        sync.scheduled = true;
        sync.generation
    };
    let s = state.clone();
    glib::timeout_add_local_once(Duration::from_secs(delay), move || {
        if s.resume_sync.borrow().generation != generation {
            return;
        }
        s.resume_sync.borrow_mut().scheduled = false;
        let Some(account) = s.self_mid.borrow().clone() else {
            return;
        };
        if !valid(&s, generation, &account) {
            return;
        }
        {
            let mut sync = s.resume_sync.borrow_mut();
            sync.busy = true;
            sync.attempts += 1;
        }
        s.status.set_text(&crate::i18n::t("resume_sync_loading"));
        match s.sidecar.request(
            "list_chats",
            // 履歴の再取得だけでなく、休止で止まった新着の受信接続も作り直す。
            Some(serde_json::json!({"force":true,"quiet":true,"reconnect":true})),
        ) {
            Ok(id) => {
                s.pending.borrow_mut().insert(
                    id,
                    Pending::ResumeSync {
                        generation,
                        account,
                        chat_mid: None,
                    },
                );
            }
            Err(_) => failed(&s),
        }
    });
}

fn failed(state: &AppState) {
    let attempt = {
        let mut sync = state.resume_sync.borrow_mut();
        sync.busy = false;
        sync.attempts
    };
    if attempt < 4 {
        state
            .status
            .set_text(&crate::i18n::t("resume_sync_waiting"));
        schedule(state, [3, 10, 30][attempt.saturating_sub(1)]);
    } else {
        state.status.set_text(&crate::i18n::t("resume_sync_failed"));
        let notice = libadwaita::Toast::new(&crate::i18n::t("resume_sync_failed"));
        notice.set_button_label(Some(&crate::i18n::t("retry")));
        notice.set_action_name(Some("win.sync-after-resume"));
        state.toast_overlay.add_toast(notice);
    }
}

fn finished(state: &AppState) {
    let mut sync = state.resume_sync.borrow_mut();
    sync.busy = false;
    sync.attempts = 0;
    state.status.set_text(&crate::i18n::t("resume_sync_done"));
}

pub(super) fn response(
    state: &AppState,
    generation: u64,
    account: String,
    chat_mid: Option<String>,
    ok: bool,
    result: serde_json::Value,
) {
    if !valid(state, generation, &account) {
        return;
    }
    if !ok {
        failed(state);
        return;
    }
    if let Some(mid) = chat_mid {
        if state.current_chat.borrow().as_deref() != Some(mid.as_str()) {
            finished(state);
            return;
        }
        let Ok(messages) = serde_json::from_value::<Vec<MessageInfo>>(
            result.get("messages").cloned().unwrap_or_default(),
        ) else {
            failed(state);
            return;
        };
        // 送信中の吹き出しを消さず、完了後に最新の表示データと統合する。
        if state.pending_rows.borrow().is_empty() {
            apply_resumed_messages(state, messages);
            finished(state);
        } else {
            let s = state.clone();
            let mut messages = Some(messages);
            glib::timeout_add_local(Duration::from_secs(1), move || {
                if !valid(&s, generation, &account) {
                    return glib::ControlFlow::Break;
                }
                if s.current_chat.borrow().as_deref() != Some(mid.as_str()) {
                    finished(&s);
                    return glib::ControlFlow::Break;
                }
                if s.pending_rows.borrow().is_empty() {
                    apply_resumed_messages(&s, messages.take().unwrap_or_default());
                    finished(&s);
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        }
    } else {
        let Ok(chats) = serde_json::from_value::<Vec<ChatInfo>>(
            result.get("chats").cloned().unwrap_or_default(),
        ) else {
            failed(state);
            return;
        };
        apply_chats(state, chats, false);
        contact_blocks::request_refresh(state);
        let mid = state.current_chat.borrow().clone();
        let Some(mid) = mid else {
            finished(state);
            return;
        };
        if hidden_chats::is_hidden(state, &mid) {
            finished(state);
            return;
        }
        match state.sidecar.request(
            "fetch_messages",
            Some(serde_json::json!({"chatMid":mid,"limit":80,"force":true,"quiet":true})),
        ) {
            Ok(id) => {
                state.pending.borrow_mut().insert(
                    id,
                    Pending::ResumeSync {
                        generation,
                        account,
                        chat_mid: Some(mid),
                    },
                );
            }
            Err(_) => failed(state),
        }
    }
}

fn message_order(a: &MessageInfo, b: &MessageInfo) -> std::cmp::Ordering {
    a.created_time
        .cmp(&b.created_time)
        .then_with(|| a.id.len().cmp(&b.id.len()).then_with(|| a.id.cmp(&b.id)))
}

fn merged_messages(
    current: impl Iterator<Item = MessageInfo>,
    incoming: Vec<MessageInfo>,
) -> Vec<MessageInfo> {
    let mut messages: HashMap<String, MessageInfo> = current
        .map(|message| (message.id.clone(), message))
        .collect();
    let oldest = messages
        .values()
        .min_by(|a, b| message_order(a, b))
        .cloned();
    for message in incoming {
        // より古い履歴を先頭に足して、読んでいる位置をずらさない。
        if !message.id.is_empty()
            && (messages.contains_key(&message.id)
                || oldest
                    .as_ref()
                    .is_none_or(|oldest| !message_order(&message, oldest).is_lt()))
        {
            messages.insert(message.id.clone(), message);
        }
    }
    let mut messages: Vec<_> = messages.into_values().collect();
    messages.sort_by(message_order);
    messages
}

fn apply_resumed_messages(state: &AppState, incoming: Vec<MessageInfo>) {
    if history::is_detached(state) {
        // A reconnect must preserve an explicitly opened historical context.
        return;
    }
    invalidate_history(state);
    let messages = merged_messages(state.reply_messages.borrow().values().cloned(), incoming);
    let stick = *state.stick_bottom.borrow();
    let scroll = state.message_scroll.vadjustment().value();
    clear_messages(state);
    *state.msg_list_fp.borrow_mut() = None;
    for message in messages {
        append_message(state, &message, false);
    }
    if state.message_list.first_child().is_some() {
        set_msg_state(state, "list", None);
    }
    history::restore_sentinels(state);
    replies::refresh(state);
    // 復帰だけでは既読を送らず、読んでいる位置も維持する。
    if stick {
        pin_messages_to_latest(state);
    } else {
        let generation = *state.message_render_generation.borrow();
        let s = state.clone();
        glib::idle_add_local_once(move || {
            if *s.message_render_generation.borrow() == generation {
                s.message_scroll.vadjustment().set_value(scroll);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_gap_detects_suspend_but_not_normal_ticks_or_backward_adjustments() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        assert!(!missed_ticks(start, start + Duration::from_secs(5)));
        assert!(missed_ticks(start, start + Duration::from_secs(300)));
        assert!(!missed_ticks(start, start - Duration::from_secs(60)));
    }
    #[test]
    fn resumed_snapshot_keeps_live_messages_and_deduplicates_existing_ids() {
        let message = |id: &str, time, text| {
            serde_json::from_value::<MessageInfo>(
                serde_json::json!({"id":id,"createdTime":time,"text":text}),
            )
            .unwrap()
        };
        let merged = merged_messages(
            vec![message("9", 10, "既存"), message("11", 11, "取得中の新着")].into_iter(),
            vec![
                message("8", 9, "読んでいる位置をずらす古い履歴"),
                message("9", 10, "取得後の本文"),
                message("10", 10, "休止中の新着"),
            ],
        );
        assert_eq!(
            merged
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["9", "10", "11"]
        );
        assert_eq!(merged[0].text, "取得後の本文");
    }
}
