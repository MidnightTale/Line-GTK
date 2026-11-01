use super::*;

pub(super) fn apply_chats(state: &AppState, mut chats: Vec<ChatInfo>, cached: bool) {
    for chat in &mut chats {
        if !chat.preview.is_empty() {
            chat.preview = localize_preview(&chat.preview);
        }
    }
    {
        let account = state.self_mid.borrow().clone().unwrap_or_default();
        pinned_chats::sort(&mut chats, &state.config.borrow(), &account);
    }

    if !sync_chat_rows(state, &chats) {
        rebuild_chat_rows(state, &chats);
    }

    let n = chats.len();
    *state.chats.borrow_mut() = chats;
    if let Some(mid) = state.current_chat.borrow().as_deref()
        && let Some(chat) = state.chats.borrow().iter().find(|chat| chat.mid == mid)
    {
        update_mute_btn(state, chat.muted);
    }
    chat_filter::refresh(state);

    if n == 0 {
        set_side_state(state, "empty", Some(&crate::i18n::t("no_chats")));
        state.status.set_text(&crate::i18n::t("status_no_chats"));
    } else {
        set_side_state(state, "list", None);
        let n_str = n.to_string();
        let status = if cached {
            crate::i18n::tf("status_chats_cache", &[("n", &n_str)])
        } else {
            crate::i18n::tf("status_chats_latest", &[("n", &n_str)])
        };
        state.status.set_text(&status);
        // After every rebuild, keep the open chat highlighted in the sidebar.
        select_sidebar_chat(state, state.current_chat.borrow().as_deref());
        maybe_restore_last_chat(state);
    }
    chat_filter::refresh(state);
    refresh_tray_menu(state);
    hidden_chats::refresh(state);
}

fn rebuild_chat_rows(state: &AppState, chats: &[ChatInfo]) {
    clear_list(&state.chat_list);
    state.chat_avatars.borrow_mut().clear();
    state.chat_previews.borrow_mut().clear();
    state.chat_unread_badges.borrow_mut().clear();
    for chat in chats {
        let row = new_chat_row(state, chat);
        state.chat_list.append(&row);
    }
}

fn new_chat_row(state: &AppState, chat: &ChatInfo) -> gtk::ListBoxRow {
    let (row, avatar, preview, badge) = build_chat_row(chat, *state.sidebar_compact.borrow());
    hidden_chats::wire_context(state, &row, chat);
    state
        .chat_avatars
        .borrow_mut()
        .insert(chat.mid.clone(), avatar);
    state
        .chat_previews
        .borrow_mut()
        .insert(chat.mid.clone(), preview);
    state
        .chat_unread_badges
        .borrow_mut()
        .insert(chat.mid.clone(), badge);
    row
}

/// Bring the sidebar to `chats` by editing the rows that exist, so a refresh
/// over the saved list does not clear the list or lose the selection.
/// Returns false when the rows do not match `state.chats` and need a rebuild.
fn sync_chat_rows(state: &AppState, chats: &[ChatInfo]) -> bool {
    let old = state.chats.borrow().clone();
    let mut rows = HashMap::new();
    for (index, chat) in old.iter().enumerate() {
        match state.chat_list.row_at_index(index as i32) {
            Some(row) => rows.insert(chat.mid.clone(), row),
            None => return false,
        };
    }
    if rows.len() != old.len()
        || state.chat_list.row_at_index(old.len() as i32).is_some()
        || chats
            .iter()
            .map(|chat| chat.mid.as_str())
            .collect::<HashSet<_>>()
            .len()
            != chats.len()
    {
        return false;
    }
    let keep: HashSet<_> = chats.iter().map(|chat| chat.mid.as_str()).collect();
    for chat in &old {
        if !keep.contains(chat.mid.as_str())
            && let Some(row) = rows.remove(&chat.mid)
        {
            state.chat_list.remove(&row);
            state.chat_avatars.borrow_mut().remove(&chat.mid);
            state.chat_previews.borrow_mut().remove(&chat.mid);
            state.chat_unread_badges.borrow_mut().remove(&chat.mid);
        }
    }

    let compact = *state.sidebar_compact.borrow();
    let previous: HashMap<_, _> = old.iter().map(|chat| (chat.mid.as_str(), chat)).collect();
    for chat in chats {
        if let (Some(row), Some(before)) = (rows.get(&chat.mid), previous.get(chat.mid.as_str())) {
            update_chat_row(state, row, before, chat, compact);
        } else {
            rows.insert(chat.mid.clone(), new_chat_row(state, chat));
        }
    }

    let current: Vec<String> = old
        .iter()
        .filter(|chat| keep.contains(chat.mid.as_str()))
        .map(|chat| chat.mid.clone())
        .collect();
    let target: Vec<String> = chats.iter().map(|chat| chat.mid.clone()).collect();
    for (mid, index) in cache_boot::row_moves(&current, &target) {
        let Some(row) = rows.get(&mid) else {
            continue;
        };
        if row.parent().is_some() {
            state.chat_list.remove(row);
        }
        state.chat_list.insert(row, index as i32);
    }
    true
}

fn update_chat_row(
    state: &AppState,
    row: &gtk::ListBoxRow,
    before: &ChatInfo,
    chat: &ChatInfo,
    compact: bool,
) {
    update_chat_row_name(row, &chat.name);
    update_chat_row_time(row, &format_activity(chat.last_activity));
    if let Some(label) = state.chat_previews.borrow().get(&chat.mid) {
        label.set_text(&chat_preview_text(chat));
    }
    if let Some(badge) = state.chat_unread_badges.borrow().get(&chat.mid) {
        badge.set_text(&if chat.unread > 0 {
            chat.unread.to_string()
        } else {
            String::new()
        });
        badge.set_visible(chat.unread > 0);
    }
    if chat.avatar_path != before.avatar_path
        && let Some(path) = chat.avatar_path.as_deref()
        && std::path::Path::new(path).exists()
        && let Some(avatar) = state.chat_avatars.borrow().get(&chat.mid).cloned()
    {
        let px = if compact {
            AVATAR_COMPACT_PX
        } else {
            AVATAR_CHAT_PX
        };
        attach_texture_async(avatar, path.to_string(), px * 2);
    }
}

fn chat_preview_text(chat: &ChatInfo) -> String {
    if chat.preview.is_empty() {
        if chat.last_activity > 0 {
            crate::i18n::t("loading_last")
        } else {
            crate::i18n::t("no_recent")
        }
    } else {
        localize_preview(&chat.preview)
    }
}

/// The time label sits right after the name in the row's top line.
fn update_chat_row_time(row: &gtk::ListBoxRow, text: &str) {
    let Some(outer) = row.child() else { return };
    let Ok(box_) = outer.downcast::<gtk::Box>() else {
        return;
    };
    let mut child = box_.first_child();
    while let Some(w) = child {
        if w.css_classes().iter().any(|c| c == "line-chat-text")
            && let Ok(inner) = w.clone().downcast::<gtk::Box>()
            && let Some(top) = inner.first_child()
            && let Ok(top_box) = top.downcast::<gtk::Box>()
            && let Some(name) = top_box.first_child()
            && let Some(time) = name.next_sibling()
            && let Ok(label) = time.downcast::<gtk::Label>()
        {
            label.set_text(text);
            return;
        }
        child = w.next_sibling();
    }
}

pub(super) fn select_sidebar_chat(state: &AppState, mid: Option<&str>) {
    let Some(mid) = mid else {
        return;
    };
    let Some(pos) = state.chats.borrow().iter().position(|c| c.mid == mid) else {
        return;
    };
    if let Some(row) = state.chat_list.row_at_index(pos as i32)
        && row.is_visible()
    {
        state.chat_list.select_row(Some(&row));
        // 新着や送信後の選択同期では、入力欄など操作中のウィジェットからフォーカスを奪わない。
    }
}

/// Ensure a sidebar row exists for this mid (first message from a new friend).
pub(super) fn ensure_chat_visible(state: &AppState, chat: &ChatInfo) {
    if state.chats.borrow().iter().any(|c| c.mid == chat.mid) {
        return;
    }
    upsert_chat_row(state, chat.clone());
}

pub(super) fn upsert_chat_row(state: &AppState, chat: ChatInfo) {
    let mid = chat.mid.clone();
    let existed = state.chats.borrow().iter().any(|c| c.mid == mid);

    if existed {
        let mut newer_activity = false;
        if let Some(cur) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
            cur.contact_only = chat.contact_only;
            if !chat.name.is_empty() && chat.name != mid {
                cur.name = chat.name.clone();
            }
            if !chat.kind.is_empty() {
                cur.kind = chat.kind.clone();
            }
            if !chat.preview.is_empty() {
                cur.preview = chat.preview.clone();
            }
            if chat.last_activity > cur.last_activity {
                newer_activity = true;
                cur.last_activity = chat.last_activity;
            }
            if let Some(path) = chat.avatar_path.clone() {
                cur.avatar_path = Some(path);
            }
        }
        if let Some(label) = state.chat_previews.borrow().get(&mid)
            && !chat.preview.is_empty()
        {
            label.set_text(&chat.preview);
        }
        // Refresh visible name on the row: rebuild is heavy; update via child labels if needed.
        // Promote to top so new activity is obvious.
        if newer_activity {
            promote_chat_to_top(state, &mid);
        }
        // Update name label on the row (first heading label in the row).
        if let Some(pos) = state.chats.borrow().iter().position(|c| c.mid == mid)
            && let Some(row) = state.chat_list.row_at_index(pos as i32)
        {
            update_chat_row_name(&row, &chat.name);
            if let Some(path) = chat.avatar_path.as_deref()
                && std::path::Path::new(path).exists()
                && let Some(img) = state.chat_avatars.borrow().get(&mid).cloned()
            {
                attach_texture_async(img, path.to_string(), 72);
            }
        }
        chat_filter::refresh(state);
        return;
    }

    let (row, avatar, preview, badge) = build_chat_row(&chat, *state.sidebar_compact.borrow());
    hidden_chats::wire_context(state, &row, &chat);
    state.chat_avatars.borrow_mut().insert(mid.clone(), avatar);
    state
        .chat_previews
        .borrow_mut()
        .insert(mid.clone(), preview);
    state
        .chat_unread_badges
        .borrow_mut()
        .insert(mid.clone(), badge);
    state.chats.borrow_mut().insert(0, chat);
    state.chat_list.prepend(&row);
    set_side_state(state, "list", None);
    let n = state.chats.borrow().len();
    state.status.set_text(&crate::i18n::tf(
        "status_chats_live",
        &[("n", &n.to_string())],
    ));
    chat_filter::refresh(state);
    refresh_tray_menu(state);
}

pub(super) fn promote_chat_to_top(state: &AppState, mid: &str) {
    let pos = {
        let chats = state.chats.borrow();
        chats.iter().position(|c| c.mid == mid)
    };
    let Some(pos) = pos else { return };
    if pos == 0 {
        chat_filter::refresh(state);
        return;
    }
    {
        let mut chats = state.chats.borrow_mut();
        let chat = chats.remove(pos);
        chats.insert(0, chat);
    }
    if let Some(row) = state.chat_list.row_at_index(pos as i32) {
        state.chat_list.remove(&row);
        state.chat_list.prepend(&row);
        if state.config.borrow().animations {
            row.add_css_class("line-chat-bump");
            let row_c = row.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(340), move || {
                row_c.remove_css_class("line-chat-bump");
            });
        }
    }
    chat_filter::refresh(state);
    // Keep selection on the open chat after row reorder.
    select_sidebar_chat(state, state.current_chat.borrow().as_deref());
    refresh_tray_menu(state);
}

pub(super) fn update_chat_row_name(row: &gtk::ListBoxRow, name: &str) {
    if name.is_empty() {
        return;
    }
    row.set_tooltip_text(Some(name));
    // row > box > overlay + text_col > top > name label
    let Some(outer) = row.child() else { return };
    let Ok(box_) = outer.downcast::<gtk::Box>() else {
        return;
    };
    let mut child = box_.first_child();
    while let Some(w) = child {
        if w.css_classes().iter().any(|c| c == "line-chat-text")
            && let Ok(inner) = w.clone().downcast::<gtk::Box>()
            && let Some(top) = inner.first_child()
            && let Ok(top_box) = top.downcast::<gtk::Box>()
            && let Some(name_w) = top_box.first_child()
            && let Ok(label) = name_w.downcast::<gtk::Label>()
            && label.css_classes().iter().any(|c| c == "line-chat-name")
        {
            label.set_text(name);
            return;
        }
        child = w.next_sibling();
    }
}

pub(super) fn maybe_restore_last_chat(state: &AppState) {
    if *state.restored_last_chat.borrow() {
        return;
    }
    if state.current_chat.borrow().is_some() {
        *state.restored_last_chat.borrow_mut() = true;
        return;
    }
    if !*state.session_ready.borrow() && !cache_boot::active(state) {
        return;
    }
    let last = state.config.borrow().last_chat_mid.clone();
    if last.is_empty() || hidden_chats::is_hidden(state, &last) {
        *state.restored_last_chat.borrow_mut() = true;
        return;
    }
    let chats = state.chats.borrow();
    let Some(chat) = chats.iter().find(|c| c.mid == last).cloned() else {
        return; // keep trying until the mid appears in a later refresh
    };
    drop(chats);
    *state.restored_last_chat.borrow_mut() = true;
    open_chat(state, &chat);
    select_sidebar_chat(state, Some(chat.mid.as_str()));
}

pub(super) fn build_chat_row(
    chat: &ChatInfo,
    compact: bool,
) -> (gtk::ListBoxRow, gtk::Picture, gtk::Label, gtk::Label) {
    let row = gtk::ListBoxRow::new();
    row.set_tooltip_text(Some(&chat.name));
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(if compact { 0 } else { 12 })
        .margin_start(if compact { 0 } else { 8 })
        .margin_end(if compact { 0 } else { 8 })
        .margin_top(8)
        .margin_bottom(8)
        .hexpand(!compact)
        .halign(if compact {
            gtk::Align::Center
        } else {
            gtk::Align::Fill
        })
        .css_classes(["line-chat-row"])
        .build();

    let avatar_px = if compact {
        AVATAR_COMPACT_PX
    } else {
        AVATAR_CHAT_PX
    };
    let avatar_frame = gtk::Box::builder()
        .width_request(avatar_px)
        .height_request(avatar_px)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-avatar-frame"])
        .build();
    let avatar = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .css_classes(["line-avatar"])
        .build();
    if let Some(path) = chat.avatar_path.as_deref()
        && std::path::Path::new(path).exists()
    {
        attach_texture_async(avatar.clone(), path.to_string(), avatar_px * 2);
    }
    let avatar_overlay = gtk::Overlay::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-avatar-overlay"])
        .build();
    avatar_overlay.set_child(Some(&avatar_frame));
    // 画像の解像度・縦横比を行の寸法に反映せず、表示枠に収める。
    avatar_overlay.add_overlay(&avatar);
    avatar_overlay.set_measure_overlay(&avatar, false);
    avatar_overlay.set_clip_overlay(&avatar, true);

    let badge = gtk::Label::builder()
        .label(if chat.unread > 0 {
            chat.unread.to_string()
        } else {
            String::new()
        })
        .css_classes(["line-unread", "line-unread-overlay"])
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .visible(chat.unread > 0)
        .build();
    avatar_overlay.add_overlay(&badge);
    let read_paused = gtk::Image::builder()
        .icon_name("view-conceal-symbolic")
        .pixel_size(14)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .visible(false)
        .css_classes(["line-chat-read-paused"])
        .build();
    avatar_overlay.add_overlay(&read_paused);
    avatar_overlay.set_measure_overlay(&read_paused, false);
    let pin = gtk::Image::builder()
        .icon_name("view-pin-symbolic")
        .pixel_size(14)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .visible(false)
        .css_classes(["line-chat-pinned"])
        .build();
    avatar_overlay.add_overlay(&pin);
    avatar_overlay.set_measure_overlay(&pin, false);
    let blocked = gtk::Image::builder()
        .icon_name("action-unavailable-symbolic")
        .pixel_size(14)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::End)
        .visible(false)
        .tooltip_text(crate::i18n::t("contact_blocked"))
        .css_classes(["line-chat-blocked"])
        .build();
    avatar_overlay.add_overlay(&blocked);
    avatar_overlay.set_measure_overlay(&blocked, false);

    let text_col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .visible(!compact)
        .css_classes(["line-chat-text"])
        .build();

    let top = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let name = gtk::Label::builder()
        .label(&chat.name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["line-chat-name"])
        .build();
    let time = gtk::Label::builder()
        .label(format_activity(chat.last_activity))
        .css_classes(["dim-label", "caption"])
        .build();
    top.append(&name);
    top.append(&time);

    let bottom = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let preview = gtk::Label::builder()
        .label(chat_preview_text(chat))
        .xalign(0.0)
        .hexpand(true)
        .single_line_mode(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["line-chat-preview"])
        .build();
    bottom.append(&preview);

    text_col.append(&top);
    text_col.append(&bottom);
    box_.append(&avatar_overlay);
    box_.append(&text_col);
    row.set_child(Some(&box_));
    (row, avatar, preview, badge)
}

pub(super) fn format_activity(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let secs = ts / 1000;
    let now = chrono::Utc::now().timestamp();
    let diff = (now - secs).max(0);
    if diff < 60 {
        "now".into()
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else if diff < 86400 * 7 {
        format!("{}d", diff / 86400)
    } else {
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|d| d.format("%m/%d").to_string())
            .unwrap_or_default()
    }
}

pub(super) fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

pub(super) fn mark_chat_read(state: &AppState, chat_mid: &str) {
    // Only saved messages are shown before the login finishes.
    if !*state.session_ready.borrow() || !read_pause::can_mark_read(state, chat_mid) {
        return;
    }
    let Some(last_id) = state.last_incoming_id.borrow().clone() else {
        return;
    };
    if last_id.is_empty() {
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    if let Ok(id) = state.sidecar.mark_read(chat_mid, &last_id) {
        state.pending.borrow_mut().insert(
            id,
            Pending::MarkRead {
                account,
                chat_mid: chat_mid.into(),
                message_id: last_id,
            },
        );
    }
}

pub(super) fn clear_unread_badge(state: &AppState, chat_mid: &str) {
    if read_pause::is_paused(state, chat_mid) {
        return;
    }
    if let Some(chat) = state
        .chats
        .borrow_mut()
        .iter_mut()
        .find(|c| c.mid == chat_mid)
    {
        chat.unread = 0;
    }
    if let Some(badge) = state.chat_unread_badges.borrow().get(chat_mid) {
        badge.set_text("");
        badge.set_visible(false);
    }
    if state.current_chat.borrow().as_deref() == Some(chat_mid) {
        state
            .chat_subtitle
            .set_text(&crate::i18n::t("conversation"));
    }
    refresh_tray_menu(state);
    read_pause::refresh_once_button(state);
}

pub(super) fn bump_unread(state: &AppState, chat_mid: &str) {
    let n = if let Some(chat) = state
        .chats
        .borrow_mut()
        .iter_mut()
        .find(|c| c.mid == chat_mid)
    {
        chat.unread = chat.unread.saturating_add(1);
        chat.unread
    } else {
        1
    };
    if let Some(badge) = state.chat_unread_badges.borrow().get(chat_mid) {
        badge.set_text(&n.to_string());
        badge.set_visible(true);
    }
    refresh_tray_menu(state);
    read_pause::refresh_once_button(state);
}

pub(super) fn update_mute_btn(state: &AppState, muted: bool) {
    if muted {
        state.mute_btn.set_icon_name("audio-volume-muted-symbolic");
        state
            .mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("unmute_chat")));
    } else {
        state.mute_btn.set_icon_name("audio-volume-high-symbolic");
        state
            .mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("mute_chat")));
    }
}

pub(super) fn toggle_chat_mute(state: &AppState) {
    let Some(mid) = state.current_chat.borrow().clone() else {
        return;
    };
    if !mid.starts_with('u') && !mid.starts_with('c') {
        toast(state, &crate::i18n::t("mute_unsupported_chat"));
        return;
    }
    let currently = state
        .chats
        .borrow()
        .iter()
        .find(|c| c.mid == mid)
        .map(|c| c.muted)
        .unwrap_or(false);
    let next = !currently;
    if let Err(e) = state.sidecar.mute_chat(&mid, next) {
        toast(
            state,
            &crate::i18n::tf("mute_failed", &[("error", &e.to_string())]),
        );
    }
}

// 表示している行だけを更新し、スクロール位置と本文は維持する。
pub(super) fn refresh_read_labels(state: &AppState) {
    let times = state.msg_created.borrow();
    for (id, label) in state.receipt_slots.borrow().iter() {
        let (text, tooltip, read) = receipt_status(state, id, times.get(id).copied().unwrap_or(0));
        label.set_text(&text);
        label.set_tooltip_text(Some(&tooltip));
        if read {
            label.add_css_class("line-msg-status-read");
        } else {
            label.remove_css_class("line-msg-status-read");
        }
    }
}

pub(super) fn receipt_status(
    state: &AppState,
    id: &str,
    created_time: i64,
) -> (String, String, bool) {
    if id.starts_with("pending-outbox-") {
        let text = crate::i18n::t("outbox_waiting");
        return (text.clone(), text, false);
    }
    if id.starts_with("pending-") {
        let text = crate::i18n::t("media_sending");
        return (text.clone(), text, false);
    }
    let mid = state.current_chat.borrow().clone().unwrap_or_default();
    let self_mid = state.self_mid.borrow().clone().unwrap_or_default();
    let count = state.peer_reads.borrow().count(&mid, id, &self_mid);
    let group = mid.starts_with('c') || mid.starts_with('r');
    let tooltip = if count > 0 {
        outgoing_read_label(count, group)
    } else {
        crate::i18n::t("status_read_unconfirmed")
    };
    (
        format_outgoing_status(count, group, created_time),
        tooltip,
        count > 0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn multiline_previews_keep_conversation_rows_one_line_high() {
        gtk::init().unwrap();
        let mut chat: ChatInfo = serde_json::from_value(serde_json::json!({
            "mid": "u-test", "name": "Test", "preview": "First line"
        }))
        .unwrap();
        let (row, _, preview, _) = build_chat_row(&chat, false);
        let height = |widget: &gtk::Widget| widget.measure(gtk::Orientation::Vertical, 280);
        let row_height = height(row.upcast_ref());
        let preview_height = height(preview.upcast_ref());
        for text in [
            "First line\nSecond line\n\nFourth line\nFifth line",
            "First line\r\nSecond line\r\nThird line",
            "First line\u{2028}Second line\u{2029}Third line",
        ] {
            // Live updates and newly built rows must both keep their height.
            preview.set_text(text);
            assert_eq!(height(preview.upcast_ref()), preview_height);
            assert_eq!(height(row.upcast_ref()), row_height);
            assert_eq!(preview.text(), text);
            chat.preview = text.to_string();
            let (rebuilt, _, _, _) = build_chat_row(&chat, false);
            assert_eq!(height(rebuilt.upcast_ref()), row_height);
        }
    }
}
