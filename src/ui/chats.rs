use super::*;

pub(super) fn apply_chats(state: &AppState, mut chats: Vec<ChatInfo>, cached: bool) {
    clear_list(&state.chat_list);
    state.chat_avatars.borrow_mut().clear();
    state.chat_previews.borrow_mut().clear();
    state.chat_unread_badges.borrow_mut().clear();

    for chat in &mut chats {
        if !chat.preview.is_empty() {
            chat.preview = localize_preview(&chat.preview);
        }
        if state.config.borrow().pinned_chats.contains(&chat.mid) {
            chat.pinned = true;
        }
    }
    chats.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.last_activity.cmp(&a.last_activity))
            .then_with(|| a.name.cmp(&b.name))
    });

    for chat in &chats {
        let (row, avatar, preview, badge) = build_chat_row(chat, *state.sidebar_compact.borrow());
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
        state.chat_list.append(&row);
    }

    let n = chats.len();
    *state.chats.borrow_mut() = chats;

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
    refresh_tray_menu(state);
}

pub(super) fn select_sidebar_chat(state: &AppState, mid: Option<&str>) {
    let Some(mid) = mid else {
        return;
    };
    let Some(pos) = state.chats.borrow().iter().position(|c| c.mid == mid) else {
        return;
    };
    if let Some(row) = state.chat_list.row_at_index(pos as i32) {
        state.chat_list.select_row(Some(&row));
        // Ensure the selected row is scrolled into view.
        row.grab_focus();
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
        if let Some(cur) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
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
                cur.last_activity = chat.last_activity;
            }
            if let Some(path) = chat.avatar_path.clone() {
                cur.avatar_path = Some(path);
            }
            cur.pinned = chat.pinned;
        }
        if let Some(label) = state.chat_previews.borrow().get(&mid)
            && !chat.preview.is_empty()
        {
            label.set_text(&chat.preview);
        }
        // Refresh visible name on the row: rebuild is heavy; update via child labels if needed.
        // Promote to top so new activity is obvious.
        promote_chat_to_top(state, &mid);
        // Update name label on the row (first heading label in the row).
        if let Some(pos) = state.chats.borrow().iter().position(|c| c.mid == mid)
            && let Some(row) = state.chat_list.row_at_index(pos as i32)
        {
            update_chat_row_name(&row, &chat.name);
            if let Some(path) = chat.avatar_path.as_deref()
                && std::path::Path::new(path).exists()
                && let Some(img) = state.chat_avatars.borrow().get(&mid).cloned()
            {
                let px = img.width_request().max(AVATAR_COMPACT_PX);
                attach_avatar_texture_async(img, path.to_string(), px);
            }
        }
        return;
    }

    let (row, avatar, preview, badge) = build_chat_row(&chat, *state.sidebar_compact.borrow());
    state.chat_avatars.borrow_mut().insert(mid.clone(), avatar);
    state
        .chat_previews
        .borrow_mut()
        .insert(mid.clone(), preview);
    state
        .chat_unread_badges
        .borrow_mut()
        .insert(mid.clone(), badge);
    let insert_at = {
        let chats = state.chats.borrow();
        if chat.pinned {
            0
        } else {
            chats.iter().take_while(|item| item.pinned).count()
        }
    };
    state.chats.borrow_mut().insert(insert_at, chat);
    state.chat_list.insert(&row, insert_at as i32);
    set_side_state(state, "list", None);
    let n = state.chats.borrow().len();
    state.status.set_text(&crate::i18n::tf(
        "status_chats_live",
        &[("n", &n.to_string())],
    ));
    refresh_tray_menu(state);
}

pub(super) fn promote_chat_to_top(state: &AppState, mid: &str) {
    let pos = {
        let chats = state.chats.borrow();
        chats.iter().position(|c| c.mid == mid)
    };
    let Some(pos) = pos else { return };
    let target = {
        let chats = state.chats.borrow();
        if chats[pos].pinned {
            0
        } else {
            chats.iter().take_while(|chat| chat.pinned).count()
        }
    };
    if pos == target {
        return;
    }
    let target = {
        let mut chats = state.chats.borrow_mut();
        let chat = chats.remove(pos);
        let target = if chat.pinned {
            0
        } else {
            chats.iter().take_while(|item| item.pinned).count()
        };
        chats.insert(target, chat);
        target
    };
    if let Some(row) = state.chat_list.row_at_index(pos as i32) {
        state.chat_list.remove(&row);
        state.chat_list.insert(&row, target as i32);
        if state.config.borrow().animations {
            row.add_css_class("line-chat-bump");
            let row_c = row.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(340), move || {
                row_c.remove_css_class("line-chat-bump");
            });
        }
    }
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
    if !*state.session_ready.borrow() {
        return;
    }
    let last = state.config.borrow().last_chat_mid.clone();
    if last.is_empty() {
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
    avatar_frame.set_overflow(gtk::Overflow::Hidden);
    let avatar = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(avatar_px)
        .height_request(avatar_px)
        .css_classes(["line-avatar"])
        .build();
    avatar.set_overflow(gtk::Overflow::Hidden);
    if let Some(path) = chat.avatar_path.as_deref()
        && std::path::Path::new(path).exists()
    {
        attach_avatar_texture_async(avatar.clone(), path.to_string(), avatar_px);
    }
    avatar_frame.append(&avatar);

    let avatar_overlay = gtk::Overlay::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-avatar-overlay"])
        .build();
    avatar_overlay.set_child(Some(&avatar_frame));

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
    if chat.pinned {
        top.append(
            &gtk::Image::builder()
                .icon_name("starred-symbolic")
                .pixel_size(14)
                .tooltip_text(crate::i18n::t("unpin_chat"))
                .css_classes(["line-chat-pinned"])
                .build(),
        );
    }
    top.append(&time);

    let bottom = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let preview_text = if chat.preview.is_empty() {
        if chat.last_activity > 0 {
            crate::i18n::t("loading_last")
        } else {
            crate::i18n::t("no_recent")
        }
    } else {
        localize_preview(&chat.preview)
    };
    let preview = gtk::Label::builder()
        .label(&preview_text)
        .xalign(0.0)
        .hexpand(true)
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

pub(super) fn msg_id_le(a: &str, b: &str) -> bool {
    match (a.parse::<u128>(), b.parse::<u128>()) {
        (Ok(x), Ok(y)) => x <= y,
        _ => a <= b,
    }
}

pub(super) fn mark_chat_read(state: &AppState, chat_mid: &str) {
    if !state.config.borrow().auto_mark_read {
        return;
    }
    let Some(last_id) = state.last_incoming_id.borrow().clone() else {
        return;
    };
    if last_id.is_empty() {
        return;
    }
    let _ = state.sidecar.mark_read(chat_mid, &last_id);
}

pub(super) fn clear_unread_badge(state: &AppState, chat_mid: &str) {
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
    if !mid.starts_with('u') {
        toast(state, &crate::i18n::t("mute_dm_only"));
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
    match state.sidecar.mute_chat(&mid, next) {
        Ok(_) => {
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                chat.muted = next;
            }
            update_mute_btn(state, next);
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("mute_failed", &[("error", &e.to_string())]),
        ),
    }
}

pub(super) fn update_pin_btn(state: &AppState, pinned: bool) {
    state.pin_btn.set_icon_name(if pinned {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    });
    state
        .pin_btn
        .set_tooltip_text(Some(&crate::i18n::t(if pinned {
            "unpin_chat"
        } else {
            "pin_chat"
        })));
}

pub(super) fn toggle_chat_pin(state: &AppState) {
    let Some(mid) = state.current_chat.borrow().clone() else {
        return;
    };
    let next = !state
        .chats
        .borrow()
        .iter()
        .find(|chat| chat.mid == mid)
        .map(|chat| chat.pinned)
        .unwrap_or(false);
    {
        let mut config = state.config.borrow_mut();
        config.pinned_chats.retain(|item| item != &mid);
        if next {
            config.pinned_chats.push(mid.clone());
        }
        config.save(&state.data_dir);
    }
    let mut chats = state.chats.borrow().clone();
    if let Some(chat) = chats.iter_mut().find(|chat| chat.mid == mid) {
        chat.pinned = next;
    }
    apply_chats(state, chats, false);
    update_pin_btn(state, next);
}

pub(super) fn open_chat_album(state: &AppState) {
    let mut messages: Vec<MessageInfo> = state
        .media_msgs
        .borrow()
        .values()
        .filter(|message| {
            matches!(
                message.content_type.to_ascii_uppercase().as_str(),
                "IMAGE" | "VIDEO"
            ) && message
                .image_path
                .as_deref()
                .is_some_and(|path| std::path::Path::new(path).exists())
        })
        .cloned()
        .collect();
    messages.sort_by_key(|message| std::cmp::Reverse(message.created_time));

    let title = state
        .current_chat
        .borrow()
        .as_ref()
        .and_then(|mid| {
            state
                .chats
                .borrow()
                .iter()
                .find(|chat| &chat.mid == mid)
                .cloned()
        })
        .map(|chat| format!("{} · {}", crate::i18n::t("chat_album"), chat.name))
        .unwrap_or_else(|| crate::i18n::t("chat_album"));
    let window = gtk::Window::builder()
        .transient_for(&state.window)
        .modal(true)
        .title(title)
        .default_width(760)
        .default_height(620)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    if messages.is_empty() {
        scroll.set_child(Some(
            &gtk::Label::builder()
                .label(crate::i18n::t("chat_album_empty"))
                .valign(gtk::Align::Center)
                .vexpand(true)
                .css_classes(["dim-label"])
                .build(),
        ));
    } else {
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(2)
            .max_children_per_line(4)
            .column_spacing(10)
            .row_spacing(10)
            .margin_top(14)
            .margin_bottom(14)
            .margin_start(14)
            .margin_end(14)
            .build();
        for message in messages {
            let Some(path) = message.image_path.clone() else {
                continue;
            };
            let picture = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Cover)
                .can_shrink(true)
                .width_request(160)
                .height_request(160)
                .css_classes(["line-album-thumb"])
                .build();
            attach_texture_async(picture.clone(), path.clone(), 360);
            let button = gtk::Button::builder()
                .child(&picture)
                .css_classes(["flat", "line-album-thumb-btn"])
                .build();
            let state = state.clone();
            let content_type = message.content_type.clone();
            let name = message
                .file_name
                .clone()
                .unwrap_or_else(|| format!("{}.jpg", message.id));
            button.connect_clicked(move |_| {
                open_media_viewer(&state, &path, &content_type, &name);
            });
            flow.insert(&button, -1);
        }
        scroll.set_child(Some(&flow));
    }
    window.set_child(Some(&scroll));
    window.present();
}

pub(super) fn apply_peer_read(state: &AppState, chat_mid: &str, message_id: &str) {
    if chat_mid.is_empty() || message_id.is_empty() {
        return;
    }
    let prev = state.read_upto.borrow().get(chat_mid).cloned();
    if let Some(p) = prev.as_deref()
        && msg_id_le(message_id, p)
    {
        return;
    }
    state
        .read_upto
        .borrow_mut()
        .insert(chat_mid.to_string(), message_id.to_string());

    if state.current_chat.borrow().as_deref() != Some(chat_mid) {
        return;
    }
    let times = state.msg_created.borrow().clone();
    for (id, label) in state.receipt_slots.borrow().iter() {
        if msg_id_le(id, message_id) {
            let ts = times.get(id).copied().unwrap_or(0);
            label.set_text(&format_outgoing_status(true, ts));
            label.set_tooltip_text(Some(&crate::i18n::t("status_read")));
            label.add_css_class("line-msg-status-read");
        }
    }
}
