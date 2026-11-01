use super::*;

pub(super) fn same_message_list(
    state: &AppState,
    chat_mid: &str,
    messages: &[MessageInfo],
) -> bool {
    matches!(
        state.msg_list_fp.borrow().as_ref(),
        Some((mid, previous)) if mid == chat_mid && previous == messages
    )
}

pub(super) fn apply_messages(state: &AppState, messages: Vec<MessageInfo>) {
    apply_messages_inner(state, messages);
    // Messages waiting for the connection stay below whatever was just loaded.
    cache_boot::settle(state);
}

fn apply_messages_inner(state: &AppState, mut messages: Vec<MessageInfo>) {
    if let Some(account) = state.self_mid.borrow().clone() {
        state.unsent_ids.borrow_mut().extend(
            messages
                .iter()
                .filter(|m| m.content_type == "UNSENT")
                .map(|m| (account.clone(), m.id.clone())),
        );
    }
    messages = messages
        .iter()
        .map(|message| unsend::masked(state, message))
        .collect();
    // 取得中に届いたイベントを、先に開始した履歴の応答で消さない。
    let oldest_fetched = messages.iter().map(|m| m.created_time).min().unwrap_or(0);
    let existing_events: Vec<_> = state
        .reply_messages
        .borrow()
        .values()
        .filter(|m| {
            m.chat_event.is_some()
                && m.created_time >= oldest_fetched
                && !messages.iter().any(|item| item.id == m.id)
        })
        .cloned()
        .collect();
    messages.extend(existing_events);
    messages.sort_by_key(|m| m.created_time);
    if apply_in_place(state, &messages) {
        return;
    }
    clear_messages(state);
    history::reset_for_latest(state, !messages.is_empty());
    state
        .reply_messages
        .borrow_mut()
        .extend(messages.iter().map(|m| (m.id.clone(), m.clone())));
    state.media_queue.borrow_mut().clear();
    *state.stick_bottom.borrow_mut() = true;
    if let Some(mid) = state.current_chat.borrow().clone() {
        // 同じIDの行でも、再取得で復号された本文を画面へ反映する。
        *state.msg_list_fp.borrow_mut() = Some((mid, messages.clone()));
    } else {
        *state.msg_list_fp.borrow_mut() = None;
    }
    if messages.is_empty() {
        set_msg_state(state, "empty", Some(&crate::i18n::t("no_messages")));
        state.chat_subtitle.set_text(&crate::i18n::t("empty"));
        return;
    }

    // Append in idle chunks so image-heavy threads don't freeze the window.
    let total = messages.len();
    set_msg_state(state, "list", None);
    state.chat_subtitle.set_text(&crate::i18n::tf(
        "messages_count",
        &[("n", &total.to_string())],
    ));

    let last_incoming = messages
        .iter()
        .rev()
        .find(|m| !m.mine && m.chat_event.is_none() && m.content_type != "UNSENT")
        .map(|m| m.id.clone());
    if let Some(id) = last_incoming.clone() {
        *state.last_incoming_id.borrow_mut() = Some(id);
    }

    let chat_mid = state.current_chat.borrow().clone();
    let generation = *state.message_render_generation.borrow();
    let state_c = state.clone();
    let batch = Rc::new(RefCell::new(messages));
    *state.render_batch.borrow_mut() = Some(batch.clone());
    glib::idle_add_local(move || {
        // トーク変更後に、前の履歴や引用候補を挿入しない。
        if *state_c.current_chat.borrow() != chat_mid
            || *state_c.message_render_generation.borrow() != generation
        {
            return glib::ControlFlow::Break;
        }
        let mut left = batch.borrow_mut();
        let take = left.len().min(14);
        if take == 0 {
            *state_c.render_batch.borrow_mut() = None;
            pin_messages_to_latest(&state_c);
            history::arm(&state_c);
            if let Some(mid) = chat_mid.as_deref() {
                mark_chat_read(&state_c, mid);
                clear_unread_badge(&state_c, mid);
            }
            return glib::ControlFlow::Break;
        }
        let chunk: Vec<_> = left.drain(0..take).collect();
        drop(left);
        for msg in &chunk {
            append_message(&state_c, msg, false);
        }
        // Stay on the latest message while rows are still being inserted.
        scroll_messages_to_end(&state_c);
        glib::ControlFlow::Continue
    });
}

/// Show a reloaded list without rebuilding rows that are already on screen.
/// Returns false when the list has to be rebuilt.
fn apply_in_place(state: &AppState, messages: &[MessageInfo]) -> bool {
    let Some(chat) = state.current_chat.borrow().clone() else {
        return false;
    };
    let drawing = state.render_batch.borrow().is_some();
    if (state.message_list.first_child().is_none() && !drawing) || history::is_detached(state) {
        return false;
    }
    let shown = match state.msg_list_fp.borrow().as_ref() {
        Some((mid, list)) if *mid == chat => list.clone(),
        _ => return false,
    };
    match chat_snapshots::diff_messages(&shown, messages) {
        chat_snapshots::MessageDiff::Unchanged => {
            if state.msg_stack.visible_child_name().as_deref() != Some("list") {
                set_msg_state(state, "list", None);
            }
            true
        }
        chat_snapshots::MessageDiff::Patch(patch) => {
            apply_patch(state, &chat, messages, patch);
            true
        }
        chat_snapshots::MessageDiff::Rebuild => false,
    }
}

/// Update a message that is still waiting to be drawn.
fn update_pending(batch: &RenderBatch, msg: &MessageInfo) -> bool {
    let mut left = batch.borrow_mut();
    match left.iter_mut().find(|m| m.id == msg.id) {
        Some(slot) => {
            *slot = msg.clone();
            true
        }
        None => false,
    }
}

/// Swap the rows of a shown message for ones built from `msg`.
fn replace_shown_message(state: &AppState, msg: &MessageInfo) -> bool {
    let last_day = state.last_msg_day.borrow().clone();
    let last_incoming = state.last_incoming_id.borrow().clone();
    let ready_path = state.media_ready_paths.borrow().get(&msg.id).cloned();
    let replaced = state.message_list.replace_message(
        &msg.id,
        |old| {
            let gone = super::virtual_list::Trimmed {
                ids: vec![msg.id.clone()],
                widgets: old.to_vec(),
            };
            history::cleanup(state, &gone);
        },
        || {
            // The day separator in front of the message stays where it is.
            if msg.created_time > 0 {
                *state.last_msg_day.borrow_mut() = Some(day_key(msg.created_time));
            }
            append_message_rows(state, msg, false);
        },
    );
    *state.last_msg_day.borrow_mut() = last_day;
    *state.last_incoming_id.borrow_mut() = last_incoming;
    if let Some(path) = ready_path {
        state
            .media_ready_paths
            .borrow_mut()
            .insert(msg.id.clone(), path);
    }
    replaced
}

/// Bring the shown rows in line with a reload by changing only what differs.
fn apply_patch(
    state: &AppState,
    chat: &str,
    messages: &[MessageInfo],
    patch: chat_snapshots::Patch,
) {
    let was_stuck = *state.stick_bottom.borrow();
    let batch = state.render_batch.borrow().clone();

    for msg in &patch.replaced {
        if batch.as_ref().is_some_and(|b| update_pending(b, msg)) {
            continue;
        }
        replace_shown_message(state, msg);
    }
    for msg in &patch.reacted {
        if batch.as_ref().is_some_and(|b| update_pending(b, msg)) {
            continue;
        }
        state
            .reply_messages
            .borrow_mut()
            .insert(msg.id.clone(), msg.clone());
        reactions::refresh(state, msg);
    }
    let account = state.self_mid.borrow().clone().unwrap_or_default();
    for id in &patch.unsent {
        let Some(msg) = messages.iter().find(|m| m.id == *id) else {
            continue;
        };
        if batch.as_ref().is_some_and(|b| update_pending(b, msg)) {
            continue;
        }
        unsend::apply(state, &account, chat, id);
    }

    // Rows that arrived live while the list was loading are already shown.
    let appended: Vec<_> = {
        let known = state.reply_messages.borrow();
        patch
            .appended
            .into_iter()
            .filter(|m| !known.contains_key(&m.id))
            .collect()
    };
    // Unsending resets the remembered list, so set it after the changes above.
    *state.msg_list_fp.borrow_mut() = Some((chat.to_string(), messages.to_vec()));
    state.chat_subtitle.set_text(&crate::i18n::tf(
        "messages_count",
        &[("n", &messages.len().to_string())],
    ));
    let incoming = appended
        .iter()
        .rev()
        .find(|m| !m.mine && m.chat_event.is_none() && m.content_type != "UNSENT")
        .map(|m| m.id.clone());
    if !was_stuck && incoming.is_some() {
        ensure_new_message_separator(state);
    }
    for msg in &appended {
        match &batch {
            // Drawn after the rows that are still waiting, so the order holds.
            Some(b) => b.borrow_mut().push(msg.clone()),
            None => append_message(state, msg, false),
        }
    }
    if let Some(id) = incoming {
        *state.last_incoming_id.borrow_mut() = Some(id);
        if was_stuck && batch.is_none() {
            mark_chat_read(state, chat);
            clear_unread_badge(state, chat);
        }
    }
    if was_stuck && batch.is_none() {
        pin_messages_to_latest(state);
        history::enforce_live_limit(state);
    }
    if state.msg_stack.visible_child_name().as_deref() != Some("list") {
        set_msg_state(state, "list", None);
    }
}

/// The open chat has something to read, drawn or about to be drawn.
pub(super) fn has_message_content(state: &AppState) -> bool {
    if state.render_batch.borrow().is_some() || state.message_list.first_child().is_some() {
        return true;
    }
    let current = state.current_chat.borrow();
    state
        .msg_list_fp
        .borrow()
        .as_ref()
        .is_some_and(|(mid, list)| Some(mid.as_str()) == current.as_deref() && !list.is_empty())
}

/// Show or hide the small refresh indicator next to the chat subtitle.
pub(super) fn set_syncing(state: &AppState, on: bool) {
    state.sync_spinner.set_spinning(on);
    state.sync_spinner.set_visible(on);
}

/// Keep the remembered list in step with a row added after the load.
pub(super) fn note_appended(state: &AppState, msg: &MessageInfo) {
    if let Some((_, list)) = state.msg_list_fp.borrow_mut().as_mut()
        && !list.iter().any(|m| m.id == msg.id)
    {
        list.push(msg.clone());
    }
}

/// Keep the remembered list in step with media that finished downloading.
pub(super) fn note_media(
    state: &AppState,
    id: &str,
    image: Option<&str>,
    audio: Option<&str>,
    file: Option<&str>,
) {
    if let Some((_, list)) = state.msg_list_fp.borrow_mut().as_mut()
        && let Some(msg) = list.iter_mut().find(|m| m.id == id)
    {
        if let Some(path) = image.filter(|p| !p.is_empty()) {
            msg.image_path = Some(path.to_string());
        }
        if let Some(path) = audio {
            msg.audio_path = Some(path.to_string());
        }
        if let Some(path) = file {
            msg.file_path = Some(path.to_string());
        }
    }
}

pub(super) fn append_message(state: &AppState, msg: &MessageInfo, live: bool) {
    state
        .message_list
        .set_owner(Some(super::virtual_list::Owner {
            id: msg.id.clone(),
            created: msg.created_time,
        }));
    append_message_rows(state, msg, live);
    state.message_list.set_owner(None);
}

pub(super) fn day_separator_row(created_time: i64) -> gtk::Box {
    let sep_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["line-message-row", "line-day-sep-row"])
        .build();
    let sep = gtk::Label::builder()
        .label(format_day_separator(created_time))
        .halign(gtk::Align::Center)
        .css_classes(["line-day-sep"])
        .build();
    sep_row.append(&sep);
    sep_row
}

fn append_message_rows(state: &AppState, msg: &MessageInfo, live: bool) {
    let masked = unsend::masked(state, msg);
    let msg = &masked;
    if !msg.id.is_empty() && !state.seen_msg_ids.borrow_mut().insert(msg.id.clone()) {
        return;
    }

    // Prefer a path that already finished hydrating (race with list rebuild).
    let ready_path = {
        let missing = msg.content_type != "UNSENT"
            && msg
                .image_path
                .as_deref()
                .map(|p| !std::path::Path::new(p).exists())
                .unwrap_or(true);
        if missing {
            state
                .media_ready_paths
                .borrow()
                .get(&msg.id)
                .filter(|p| std::path::Path::new(p).exists())
                .cloned()
        } else {
            None
        }
    };
    let msg_owned;
    let msg = if let Some(path) = ready_path {
        msg_owned = MessageInfo {
            image_path: Some(path),
            ..msg.clone()
        };
        &msg_owned
    } else {
        msg
    };

    state
        .reply_messages
        .borrow_mut()
        .insert(msg.id.clone(), msg.clone());
    let list = &state.message_list;

    // Date separator like mobile LINE (Today / Yesterday / Fri, Jul 24).
    if msg.created_time > 0 {
        let day = day_key(msg.created_time);
        let need_sep = state.last_msg_day.borrow().as_deref() != Some(day.as_str());
        if need_sep {
            *state.last_msg_day.borrow_mut() = Some(day);
            list.append(&day_separator_row(msg.created_time));
        }
    }

    if let Some(event) = &msg.chat_event {
        chat_events::append(state, event);
        return;
    }
    if matches!(
        msg.content_type.to_ascii_uppercase().as_str(),
        "CHATEVENT" | "CHAT_EVENT" | "18"
    ) {
        chat_events::append(state, &chat_events::unknown());
        return;
    }
    // ListViewが行のフォーカスを管理するため、ListBox専用の行を入れ子にしない。
    let row = gtk::Box::builder()
        .css_classes(["line-message-row"])
        .build();
    state
        .message_rows
        .borrow_mut()
        .insert(msg.id.clone(), row.clone());
    if msg.content_type == "UNSENT" {
        unsend::placeholder(&row);
        list.append(&row);
        return;
    }
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(gtk::Align::End)
        .build();

    let is_sticker = msg.content_type.eq_ignore_ascii_case("sticker");
    let is_flex = msg.content_type.eq_ignore_ascii_case("flex") || msg.flex.is_some();
    let is_pending = msg.id.starts_with("pending-");
    let bubble = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(if msg.mine {
            gtk::Align::End
        } else {
            gtk::Align::Start
        })
        .css_classes(if is_sticker {
            vec!["line-bubble", "line-bubble-sticker"]
        } else if is_flex {
            vec!["line-bubble", "line-bubble-in"]
        } else {
            vec![
                "line-bubble",
                if msg.mine {
                    "line-bubble-out"
                } else {
                    "line-bubble-in"
                },
            ]
        })
        .build();
    if is_pending {
        bubble.add_css_class("line-bubble-pending");
        outer.add_css_class("line-msg-pending");
    }

    let has_image = msg
        .image_path
        .as_deref()
        .map(|p| std::path::Path::new(p).exists())
        .unwrap_or(false);
    let waiting_visual = (msg.content_type.eq_ignore_ascii_case("image")
        || msg.content_type.eq_ignore_ascii_case("sticker")
        || msg.content_type.eq_ignore_ascii_case("video"))
        && !has_image;
    let is_audio = msg.content_type.eq_ignore_ascii_case("audio");
    let is_file = msg.content_type.eq_ignore_ascii_case("file");
    let is_video = msg.content_type.eq_ignore_ascii_case("video");
    let is_image = msg.content_type.eq_ignore_ascii_case("image");
    let has_audio = msg
        .audio_path
        .as_deref()
        .map(|p| std::path::Path::new(p).exists())
        .unwrap_or(false);

    if let Some(quote) = msg.reply_to.as_ref() {
        replies::append_quote(state, &bubble, quote);
    }
    replies::wire_context(&bubble, msg);

    if msg.decryption_failed {
        let label = gtk::Label::builder()
            .label(crate::i18n::t("message_decryption_failed"))
            .wrap(true)
            .max_width_chars(36)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        bubble.append(&label);
        let repair = gtk::Button::builder()
            .label(crate::i18n::t("e2ee_repair"))
            .action_name("app.repair-e2ee")
            .build();
        bubble.append(&repair);
    } else if msg.content_type.eq_ignore_ascii_case("CALL") {
        call_history::append_history_card(&bubble, msg);
    } else if let Some(flex) = msg.flex.as_ref() {
        append_flex_card(state, &bubble, msg, flex);
    } else if is_audio {
        append_voice_card(state, &bubble, msg, has_audio);
    } else if is_file {
        append_file_card(state, &bubble, msg);
    } else {
        let show_text = !msg.text.is_empty()
            && msg.text != "[Image]"
            && msg.text != "[Video]"
            && msg.text != "[Sticker]"
            && !is_sticker;

        if show_text && !msg.emojis.is_empty() {
            emojis::append(state, &bubble, msg);
            link_previews::append(&bubble, &msg.text);
        } else if show_text {
            let label = gtk::Label::builder()
                .label(&msg.text)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .max_width_chars(42)
                .css_classes(["line-bubble-text"])
                .build();
            label.set_attributes(Some(&mentions::attributes(
                &label,
                &msg.text,
                &msg.mentions,
            )));
            mentions::wire_label_click(state, &label, &msg.text, &msg.mentions);
            bubble.append(&label);
            link_previews::append(&bubble, &msg.text);
        } else if !has_image && !waiting_visual {
            let fallback = if msg.content_type.is_empty() {
                "(message)".to_string()
            } else {
                format!("({})", msg.content_type.to_lowercase())
            };
            let label = gtk::Label::builder()
                .label(&fallback)
                .xalign(0.0)
                .css_classes(["dim-label"])
                .build();
            bubble.append(&label);
        }
    }

    if has_image {
        if let Some(path) = msg.image_path.as_deref() {
            let pic = make_media_picture_placeholder(is_sticker);
            let animate = state.config.borrow().animations
                && (is_sticker || path.to_ascii_lowercase().ends_with(".gif"));
            attach_texture_async_anim(
                pic.clone(),
                path.to_string(),
                if is_sticker { 128 } else { 320 },
                animate,
            );
            if !is_sticker && (is_image || is_video) {
                if is_video {
                    let overlay = wrap_video_thumb(state, &pic, msg);
                    bubble.append(&overlay);
                } else {
                    wire_media_open_click(state, &pic, msg, "image");
                    pic.set_tooltip_text(Some(&crate::i18n::t("media_open_image")));
                    bubble.append(&pic);
                }
            } else {
                bubble.append(&pic);
            }
        }
    } else if waiting_visual {
        if is_video {
            append_video_placeholder(state, &bubble, msg, false);
        } else {
            let placeholder = gtk::Label::builder()
                .label(if is_sticker {
                    crate::i18n::t("loading_sticker")
                } else {
                    crate::i18n::t("loading_image")
                })
                .xalign(0.0)
                .css_classes(["dim-label", "line-media-placeholder"])
                .build();
            bubble.append(&placeholder);
        }
    }

    if !msg.id.is_empty() && message_tracks_media(msg) {
        state
            .media_slots
            .borrow_mut()
            .insert(msg.id.clone(), bubble.clone());
        state
            .media_msgs
            .borrow_mut()
            .insert(msg.id.clone(), msg.clone());
    }

    if !msg.mine && !msg.id.is_empty() {
        *state.last_incoming_id.borrow_mut() = Some(msg.id.clone());
    }
    if !msg.id.is_empty() && msg.created_time > 0 {
        state
            .msg_created
            .borrow_mut()
            .insert(msg.id.clone(), msg.created_time);
    }

    let time_txt = format_msg_time(msg.created_time);
    // リアクションは吹き出しの外に置く。画像の差し替えで子要素が入れ替わるため。
    let bubble_column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .halign(bubble.halign())
        .build();
    bubble_column.append(&bubble);
    bubble_column.append(&reactions::strip(state, msg, &bubble));
    let bubble = bubble_column;

    if msg.mine {
        let col = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .halign(gtk::Align::End)
            .hexpand(true)
            .build();
        if let Some(reply) = replies::button(msg) {
            let content = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .halign(gtk::Align::End)
                .build();
            let actions = gtk::Box::builder()
                .valign(gtk::Align::Center)
                .css_classes(["line-message-actions"])
                .build();
            actions.append(&reply);
            content.append(&actions);
            content.append(&bubble);
            col.append(&content);
        } else {
            col.append(&bubble);
        }

        let tracks_receipt = !msg.content_type.eq_ignore_ascii_case("CALL");
        let (text, tooltip, read) = if tracks_receipt {
            receipt_status(state, &msg.id, msg.created_time)
        } else {
            (time_txt.clone(), String::new(), false)
        };
        let status = gtk::Label::builder()
            .label(text)
            .halign(gtk::Align::End)
            .tooltip_text(tooltip)
            .css_classes(if read {
                vec!["line-msg-status", "line-msg-status-read"]
            } else {
                vec!["line-msg-status"]
            })
            .build();
        if tracks_receipt && !msg.id.is_empty() {
            state
                .receipt_slots
                .borrow_mut()
                .insert(msg.id.clone(), status.clone());
        }
        col.append(&status);
        outer.append(&gtk::Box::builder().hexpand(true).build());
        outer.append(&col);
    } else {
        // Incoming: bubble + time on the right (mobile style).
        let incoming = if message_senders::is_group(state) {
            let column = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(3)
                .hexpand(true)
                .build();
            let name = message_senders::name(state, &msg.from);
            let sender = gtk::Label::builder()
                .label(&name)
                .tooltip_text(&name)
                .xalign(0.0)
                .halign(gtk::Align::Start)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(36)
                .css_classes(["line-message-sender"])
                .build();
            message_senders::register(state, &msg.from, &sender);
            if mentions::is_user_mid(&msg.from) {
                user_profile::wire_link(state, &sender, &msg.from);
                sender.add_css_class("line-message-sender-link");
                outer.append(&user_profile::row_avatar(state, &msg.from));
            }
            column.append(&sender);
            let content = gtk::Box::builder().spacing(8).hexpand(true).build();
            column.append(&content);
            outer.append(&column);
            content
        } else {
            outer.clone()
        };
        let time = gtk::Label::builder()
            .label(&time_txt)
            .valign(gtk::Align::End)
            .css_classes(["line-msg-time"])
            .build();
        incoming.append(&bubble);
        let metadata = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .valign(gtk::Align::End)
            .build();
        let actions = gtk::Box::builder()
            .halign(gtk::Align::Start)
            .css_classes(["line-message-actions"])
            .build();
        if let Some(reply) = replies::button(msg) {
            actions.append(&reply);
        }
        if let Some(react) = reactions::button(msg) {
            actions.append(&react);
        }
        if actions.first_child().is_some() {
            metadata.append(&actions);
        }
        time.set_halign(gtk::Align::Start);
        metadata.append(&time);
        incoming.append(&metadata);
        incoming.append(&gtk::Box::builder().hexpand(true).build());
    }

    row.append(&outer);
    if is_pending {
        row.add_css_class("line-msg-pending-row");
        state
            .pending_rows
            .borrow_mut()
            .insert(msg.id.clone(), row.clone());
    }
    if live && state.config.borrow().animations {
        row.add_css_class("line-msg-enter");
        if msg.mine {
            row.add_css_class("line-msg-out");
        }
        let row_c = row.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(260), move || {
            row_c.remove_css_class("line-msg-enter");
            row_c.remove_css_class("line-msg-out");
        });
    }
    list.append(&row);
}

pub(super) fn outgoing_read_label(read_count: usize, group: bool) -> String {
    if read_count == 0 {
        crate::i18n::t("status_sent")
    } else if group {
        crate::i18n::tf("status_read_count", &[("n", &read_count.to_string())])
    } else {
        crate::i18n::t("status_read")
    }
}

pub(super) fn format_outgoing_status(read_count: usize, group: bool, created_ms: i64) -> String {
    let status = outgoing_read_label(read_count, group);
    let time = format_msg_time(created_ms);
    if time.is_empty() {
        status
    } else {
        format!("{status} · {time}")
    }
}

pub(super) fn day_key(ts_ms: i64) -> String {
    let secs = if ts_ms > 1_000_000_000_000 {
        ts_ms / 1000
    } else {
        ts_ms
    };
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

pub(super) fn format_day_separator(ts_ms: i64) -> String {
    let secs = if ts_ms > 1_000_000_000_000 {
        ts_ms / 1000
    } else {
        ts_ms
    };
    let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) else {
        return String::new();
    };
    let local = dt.with_timezone(&chrono::Local);
    let today = chrono::Local::now().date_naive();
    let day = local.date_naive();
    if day == today {
        crate::i18n::t("day_today")
    } else if day == today - chrono::Duration::days(1) {
        crate::i18n::t("day_yesterday")
    } else {
        local.format(&crate::i18n::t("date_format")).to_string()
    }
}

pub(super) fn format_msg_time(ts_ms: i64) -> String {
    if ts_ms <= 0 {
        return String::new();
    }
    let secs = if ts_ms > 1_000_000_000_000 {
        ts_ms / 1000
    } else {
        ts_ms
    };
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format(&crate::i18n::t("time_format"))
                .to_string()
        })
        .unwrap_or_default()
}

pub(super) fn append_voice_card(
    state: &AppState,
    bubble: &gtk::Box,
    msg: &MessageInfo,
    has_audio: bool,
) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["line-voice-card"])
        .build();
    let play = gtk::Button::builder()
        .icon_name(if has_audio {
            "media-playback-start-symbolic"
        } else {
            "content-loading-symbolic"
        })
        .sensitive(has_audio)
        .css_classes(["circular", "line-voice-play"])
        .tooltip_text(crate::i18n::t("play_voice"))
        .build();

    let peaks = Rc::new(RefCell::new(placeholder_peaks(36)));
    let progress = Rc::new(RefCell::new(0.0_f32));
    let mine = msg.mine;
    let wave = gtk::DrawingArea::builder()
        .hexpand(true)
        .content_width(160)
        .content_height(32)
        .css_classes(["line-voice-wave"])
        .build();
    {
        let peaks_draw = peaks.clone();
        let progress_draw = progress.clone();
        wave.set_draw_func(move |area, cr, width, height| {
            draw_waveform(
                area,
                cr,
                width as f64,
                height as f64,
                &peaks_draw.borrow(),
                mine,
                Some(*progress_draw.borrow()),
            );
        });
    }

    let duration_ms = if let Some(ms) = msg.duration_ms.filter(|v| *v > 0) {
        ms as u64
    } else if has_audio {
        msg.audio_path
            .as_deref()
            .and_then(|p| ffprobe_duration_ms(std::path::Path::new(p)))
            .unwrap_or(0)
    } else {
        0
    };
    let dur_txt = if duration_ms > 0 {
        format_voice_duration(duration_ms)
    } else if has_audio {
        "--:--".into()
    } else {
        crate::i18n::t("loading_voice")
    };
    let dur = gtk::Label::builder()
        .label(&dur_txt)
        .css_classes(["line-voice-dur"])
        .build();

    if has_audio && let Some(path) = msg.audio_path.clone() {
        let s = state.clone();
        let play_btn = play.clone();
        let wave_btn = wave.clone();
        let dur_btn = dur.clone();
        let progress_btn = progress.clone();
        let msg_id = msg.id.clone();
        let total_label = dur_txt.clone();
        play.connect_clicked(move |_| {
            toggle_voice_playback(VoicePlaybackRequest {
                state: &s,
                msg_id: &msg_id,
                path: &path,
                play_btn: &play_btn,
                wave: &wave_btn,
                duration_label: &dur_btn,
                progress: progress_btn.clone(),
                duration_ms,
                total_label: &total_label,
            });
        });
    }

    let dl = gtk::Button::builder()
        .icon_name("folder-download-symbolic")
        .css_classes(["flat", "circular"])
        .tooltip_text(crate::i18n::t("media_download"))
        .build();
    {
        let s = state.clone();
        let msg = msg.clone();
        dl.connect_clicked(move |_| {
            request_media_download(&s, &msg, "save_dialog");
        });
    }

    row.append(&play);
    row.append(&wave);
    row.append(&dur);
    row.append(&dl);
    bubble.append(&row);

    if has_audio && let Some(path) = msg.audio_path.clone() {
        let wave2 = wave.clone();
        let peaks2 = peaks.clone();
        let (tx, rx) = async_channel::bounded::<Vec<f32>>(1);
        std::thread::spawn(move || {
            let extracted = extract_audio_peaks(std::path::Path::new(&path), 36)
                .unwrap_or_else(|| placeholder_peaks(36));
            let _ = tx.send_blocking(extracted);
        });
        glib::spawn_future_local(async move {
            if let Ok(extracted) = rx.recv().await {
                *peaks2.borrow_mut() = extracted;
                wave2.queue_draw();
            }
        });
    }
}

pub(super) struct VoicePlaybackRequest<'a> {
    state: &'a AppState,
    msg_id: &'a str,
    path: &'a str,
    play_btn: &'a gtk::Button,
    wave: &'a gtk::DrawingArea,
    duration_label: &'a gtk::Label,
    progress: Rc<RefCell<f32>>,
    duration_ms: u64,
    total_label: &'a str,
}

pub(super) fn toggle_voice_playback(request: VoicePlaybackRequest<'_>) {
    let VoicePlaybackRequest {
        state,
        msg_id,
        path,
        play_btn,
        wave,
        duration_label,
        progress,
        duration_ms,
        total_label,
    } = request;
    // Second press on the same bubble stops playback.
    // Drop the borrow before stop_voice_playback (it needs borrow_mut).
    let same_bubble = state
        .voice_playback
        .borrow()
        .as_ref()
        .is_some_and(|cur| cur.msg_id == msg_id);
    if same_bubble {
        stop_voice_playback(state);
        return;
    }
    stop_voice_playback(state);

    let child = match spawn_audio_player(path, &state.config.borrow().audio_output, 1.0) {
        Ok(c) => c,
        Err(e) => {
            toast(
                state,
                &crate::i18n::tf("voice_play_failed", &[("error", &e)]),
            );
            return;
        }
    };

    play_btn.set_icon_name("media-playback-stop-symbolic");
    play_btn.set_tooltip_text(Some(&crate::i18n::t("stop_voice")));
    play_btn.add_css_class("playing");
    wave.add_css_class("playing");
    *progress.borrow_mut() = 0.0;
    wave.queue_draw();

    let playback = VoicePlayback {
        child,
        msg_id: msg_id.to_string(),
        play_btn: play_btn.clone(),
        wave: wave.clone(),
        dur: duration_label.clone(),
        duration_ms: duration_ms.max(500),
        started: std::time::Instant::now(),
        progress: progress.clone(),
        total_label: total_label.to_string(),
        tick: None,
    };
    *state.voice_playback.borrow_mut() = Some(playback);

    let s = state.clone();
    let tick = glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let mut slot = s.voice_playback.borrow_mut();
        let Some(pb) = slot.as_mut() else {
            return glib::ControlFlow::Break;
        };
        // Finished?
        match pb.child.try_wait() {
            Ok(Some(_)) => {
                drop(slot);
                stop_voice_playback(&s);
                return glib::ControlFlow::Break;
            }
            Err(_) => {
                drop(slot);
                stop_voice_playback(&s);
                return glib::ControlFlow::Break;
            }
            Ok(None) => {}
        }
        let elapsed = pb.started.elapsed().as_millis() as u64;
        let frac = (elapsed as f32 / pb.duration_ms as f32).clamp(0.0, 1.0);
        *pb.progress.borrow_mut() = frac;
        pb.wave.queue_draw();
        let remain = pb.duration_ms.saturating_sub(elapsed);
        pb.dur.set_text(&format_voice_duration(remain.max(1)));
        if frac >= 1.0 {
            // Duration reached but process still alive — keep waiting for exit.
        }
        glib::ControlFlow::Continue
    });
    if let Some(pb) = state.voice_playback.borrow_mut().as_mut() {
        pb.tick = Some(tick);
    }
}

pub(super) fn stop_voice_playback(state: &AppState) {
    let Some(mut pb) = state.voice_playback.borrow_mut().take() else {
        return;
    };
    if let Some(tick) = pb.tick.take() {
        tick.remove();
    }
    let _ = pb.child.kill();
    let _ = pb.child.wait();
    *pb.progress.borrow_mut() = 0.0;
    pb.play_btn.set_icon_name("media-playback-start-symbolic");
    pb.play_btn
        .set_tooltip_text(Some(&crate::i18n::t("play_voice")));
    pb.play_btn.remove_css_class("playing");
    pb.wave.remove_css_class("playing");
    pb.wave.queue_draw();
    pb.dur.set_text(&pb.total_label);
}

pub(super) fn append_file_card(state: &AppState, bubble: &gtk::Box, msg: &MessageInfo) {
    let name = msg
        .file_name
        .clone()
        .filter(|s| !s.is_empty() && s != "[File]")
        .unwrap_or_else(|| {
            if msg.text.is_empty() || msg.text == "[File]" {
                crate::i18n::t("preview_file")
            } else {
                msg.text.clone()
            }
        });
    let kind = viewer_kind_for("FILE", "", &name);
    let icon_name = match kind {
        ViewerKind::Pdf => "application-pdf-symbolic",
        ViewerKind::Text | ViewerKind::Markdown => "text-x-generic-symbolic",
        ViewerKind::Image => "image-x-generic-symbolic",
        ViewerKind::Video => "video-x-generic-symbolic",
        ViewerKind::Audio => "audio-x-generic-symbolic",
        ViewerKind::Generic => "folder-download-symbolic",
    };
    let row = gtk::Button::builder()
        .css_classes(["flat", "line-file-card"])
        .tooltip_text(crate::i18n::t("media_open_file"))
        .halign(gtk::Align::Fill)
        .build();
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(22);
    let label = gtk::Label::builder()
        .label(&name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(28)
        .hexpand(true)
        .css_classes(["line-bubble-text"])
        .build();
    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    inner.append(&icon);
    inner.append(&label);
    inner.append(&chevron);
    row.set_child(Some(&inner));
    let s = state.clone();
    let msg = msg.clone();
    {
        let s = s.clone();
        let msg = msg.clone();
        row.connect_clicked(move |_| {
            request_media_download(&s, &msg, "open_viewer");
        });
    }
    bubble.append(&row);

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .css_classes(["line-file-actions"])
        .build();
    for (label_key, action) in [
        ("file_action_open", "open_viewer"),
        ("file_action_show_folder", "show_folder"),
    ] {
        let button = gtk::Button::builder()
            .label(crate::i18n::t(label_key))
            .css_classes(["flat", "caption"])
            .build();
        let s = s.clone();
        let msg = msg.clone();
        button.connect_clicked(move |_| request_media_download(&s, &msg, action));
        actions.append(&button);
    }
    bubble.append(&actions);
}

pub(super) fn format_voice_duration(ms: u64) -> String {
    let secs = ((ms + 500) / 1000).max(1);
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

pub(super) fn placeholder_peaks(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            0.18 + 0.12 * ((t * std::f32::consts::PI * 4.0).sin()).abs()
        })
        .collect()
}

pub(super) fn draw_waveform(
    area: &gtk::DrawingArea,
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    peaks: &[f32],
    mine: bool,
    progress: Option<f32>,
) {
    if width <= 1.0 || height <= 1.0 || peaks.is_empty() {
        return;
    }
    // Named theme colors still require StyleContext::lookup_color (GTK < 4.10 path).
    #[allow(deprecated)]
    let accent = area
        .style_context()
        .lookup_color("accent_bg_color")
        .or_else(|| area.style_context().lookup_color("accent_color"));
    let muted = area.color();
    let (base_r, base_g, base_b) = if mine {
        match accent {
            Some(c) => (
                f64::from(c.red()),
                f64::from(c.green()),
                f64::from(c.blue()),
            ),
            None => (
                f64::from(muted.red()),
                f64::from(muted.green()),
                f64::from(muted.blue()),
            ),
        }
    } else {
        (
            f64::from(muted.red()),
            f64::from(muted.green()),
            f64::from(muted.blue()),
        )
    };
    let n = peaks.len() as f64;
    let gap = 2.0_f64.min((width / n) * 0.35).max(1.0);
    // Spread bars across the full width (do not cap bar width).
    let bar_w = ((width - gap * (n - 1.0)) / n).max(1.0);
    let mid = height * 0.5;
    let max_h = (height * 0.42).max(3.0);
    let prog = progress.unwrap_or(-1.0);
    for (i, p) in peaks.iter().enumerate() {
        let amp = (*p).clamp(0.08, 1.0) as f64;
        let h = max_h * amp;
        let x = i as f64 * (bar_w + gap);
        let t = (i as f32 + 0.5) / peaks.len() as f32;
        let played = prog >= 0.0 && t <= prog;
        let alpha = if played {
            1.0
        } else if prog >= 0.0 {
            0.35
        } else if mine {
            0.95
        } else {
            0.75
        };
        cr.set_source_rgba(base_r, base_g, base_b, alpha);
        cr.rectangle(x, mid - h, bar_w, h * 2.0);
        let _ = cr.fill();
    }
}

/// How many live spectrum bars fit the record-wave width (~3px bar + gap).
pub(super) fn spectrum_bar_count(width: f64) -> usize {
    let slot = 5.0_f64;
    ((width / slot).round() as usize).clamp(32, 160)
}

pub(super) fn extract_audio_peaks(path: &std::path::Path, bars: usize) -> Option<Vec<f32>> {
    if bars == 0 || !path.exists() {
        return None;
    }
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            path.to_str()?,
            "-ac",
            "1",
            "-ar",
            "8000",
            "-f",
            "s16le",
            "-",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.len() < 4 {
        return None;
    }
    let pcm = out.stdout;
    let n = pcm.len() / 2;
    if n == 0 {
        return None;
    }
    let mut peaks = vec![0.0f32; bars];
    for i in 0..n {
        let s = i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]) as f32 / 32768.0;
        let bi = (i * bars) / n;
        if bi < bars {
            peaks[bi] = peaks[bi].max(s.abs());
        }
    }
    let max = peaks.iter().cloned().fold(0.05f32, f32::max);
    Some(
        peaks
            .into_iter()
            .map(|p| (p / max).clamp(0.08, 1.0))
            .collect(),
    )
}

pub(super) fn wav_tail_level(path: &std::path::Path) -> f32 {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return 0.08;
    };
    let Ok(meta) = file.metadata() else {
        return 0.08;
    };
    let len = meta.len() as usize;
    if len < 44 + 512 {
        return 0.08;
    }
    let bytes = (1024 * 2).min(len.saturating_sub(44));
    let start = (len - bytes) as u64;
    if file.seek(SeekFrom::Start(start)).is_err() {
        return 0.08;
    }
    let mut buf = vec![0u8; bytes];
    if file.read_exact(&mut buf).is_err() {
        return 0.08;
    }
    let samples = buf.len() / 2;
    if samples == 0 {
        return 0.08;
    }
    let mut sum = 0.0f64;
    for i in 0..samples {
        let s = i16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]) as f64 / 32768.0;
        sum += s * s;
    }
    let rms = (sum / samples as f64).sqrt() as f32;
    (rms * 4.5).clamp(0.08, 1.0)
}

pub(super) fn wire_record_wave(state: &AppState) {
    let levels = state.recording_levels.clone();
    state
        .record_wave
        .set_draw_func(move |area, cr, width, height| {
            draw_waveform(
                area,
                cr,
                width as f64,
                height as f64,
                &levels.borrow(),
                true,
                None,
            );
        });

    // Custom DrawingAreas do not auto-repaint when the color scheme flips.
    let root = state.window.clone().upcast::<gtk::Widget>();
    let record_wave = state.record_wave.clone();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        record_wave.queue_draw();
        queue_draw_voice_waves(&root);
    });
}

pub(super) fn queue_draw_voice_waves(widget: &gtk::Widget) {
    if let Ok(area) = widget.clone().downcast::<gtk::DrawingArea>()
        && (area.has_css_class("line-voice-wave") || area.has_css_class("line-record-wave"))
    {
        area.queue_draw();
    }
    let mut child = widget.first_child();
    while let Some(w) = child {
        queue_draw_voice_waves(&w);
        child = w.next_sibling();
    }
}

pub(super) fn attach_audio_to_slot(state: &AppState, message_id: &str, path: &str) {
    let Some(bubble) = state.media_slots.borrow().get(message_id).cloned() else {
        return;
    };
    while let Some(child) = bubble.first_child() {
        bubble.remove(&child);
    }
    let original = state.reply_messages.borrow().get(message_id).cloned();
    if let Some(quote) = original
        .as_ref()
        .and_then(|message| message.reply_to.as_ref())
    {
        replies::append_quote(state, &bubble, quote);
    }
    let duration_ms = ffprobe_duration_ms(std::path::Path::new(path)).map(|v| v as i64);
    let msg = MessageInfo {
        chat_event: None,
        emojis: Vec::new(),
        mentions: Vec::new(),
        reactions: Vec::new(),
        id: message_id.to_string(),
        text: "Voice message".into(),
        from: String::new(),
        to: String::new(),
        mine: true,
        created_time: 0,
        content_type: "AUDIO".into(),
        image_path: None,
        audio_path: Some(path.to_string()),
        file_name: None,
        file_path: Some(path.to_string()),
        duration_ms,
        flex: None,
        call_history: None,
        decryption_failed: false,
        reply_to: None,
    };
    append_voice_card(state, &bubble, &msg, true);
}

pub(super) fn play_audio_file(path: &str, output_sink: &str) -> Result<(), String> {
    spawn_audio_player(path, output_sink, 1.0).map(|mut child| {
        // Detach: notifications / one-shot viewers don't track the process.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    })
}

pub(super) fn spawn_audio_player(
    path: &str,
    output_sink: &str,
    gain: f64,
) -> Result<std::process::Child, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() < 256 {
        return Err("file too small / corrupt".into());
    }
    let sink = if output_sink.is_empty() || output_sink == "default" {
        None
    } else {
        Some(output_sink.to_string())
    };
    let gain = gain.clamp(0.0, 2.5);
    let mpv_vol = (gain * 100.0).round().clamp(0.0, 250.0);

    let mut mpv = std::process::Command::new("mpv");
    mpv.args(["--no-video", "--really-quiet"]);
    mpv.arg(format!("--volume={mpv_vol}"));
    if let Some(ref s) = sink {
        mpv.arg(format!("--audio-device=pulse/{s}"));
    }
    mpv.arg(path);
    mpv.stdout(std::process::Stdio::null());
    mpv.stderr(std::process::Stdio::null());
    if let Ok(child) = mpv.spawn() {
        return Ok(child);
    }

    let mut ff = std::process::Command::new("ffplay");
    ff.args(["-nodisp", "-autoexit", "-loglevel", "quiet"]);
    if gain != 1.0 {
        ff.args(["-af", &format!("volume={gain}")]);
    }
    if let Some(ref s) = sink {
        ff.args(["-audiodevice", s]);
    }
    ff.arg(path);
    ff.stdout(std::process::Stdio::null());
    ff.stderr(std::process::Stdio::null());
    if let Ok(child) = ff.spawn() {
        return Ok(child);
    }

    if (gain - 1.0).abs() < 0.01 {
        let mut paplay = std::process::Command::new("paplay");
        if let Some(ref s) = sink {
            paplay.args(["--device", s]);
        }
        paplay.arg(path);
        paplay.stdout(std::process::Stdio::null());
        paplay.stderr(std::process::Stdio::null());
        if let Ok(child) = paplay.spawn() {
            return Ok(child);
        }
    }

    Err("no audio player (mpv/ffplay)".into())
}

pub(super) fn filter_chats(state: &AppState, query: &str) {
    chat_filter::set_query(state, query);
}

pub(super) fn append_flex_card(
    state: &AppState,
    bubble: &gtk::Box,
    msg: &MessageInfo,
    flex: &crate::protocol::FlexInfo,
) {
    if let Some(uri) = line_polls::poll_uri(flex)
        && line_polls::append_card(state, bubble, flex, uri)
    {
        return;
    }
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["line-flex-card"])
        .build();

    let title = if flex.alt_text.is_empty() {
        msg.text.clone()
    } else {
        flex.alt_text.clone()
    };
    if !title.is_empty() {
        card.append(
            &gtk::Label::builder()
                .label(&title)
                .xalign(0.0)
                .wrap(true)
                .max_width_chars(40)
                .css_classes(["line-flex-title"])
                .build(),
        );
    }
    for t in flex.texts.iter().take(6) {
        if *t == title || flex.actions.iter().any(|a| a.label == *t) {
            continue;
        }
        card.append(
            &gtk::Label::builder()
                .label(t)
                .xalign(0.0)
                .wrap(true)
                .max_width_chars(40)
                .css_classes(["dim-label"])
                .build(),
        );
    }

    for action in flex.actions.iter().take(12) {
        let btn = gtk::Button::builder()
            .label(&action.label)
            .hexpand(true)
            .css_classes(["line-flex-action"])
            .build();
        let state = state.clone();
        let action = action.clone();
        let msg_id = msg.id.clone();
        let chat = state.current_chat.borrow().clone().unwrap_or_default();
        btn.connect_clicked(move |_| {
            handle_flex_action(&state, &chat, &msg_id, &action);
        });
        card.append(&btn);
    }

    bubble.append(&card);
}

pub(super) fn handle_flex_action(
    state: &AppState,
    chat_mid: &str,
    message_id: &str,
    action: &FlexAction,
) {
    let kind = action.kind.to_ascii_lowercase();
    if (kind == "uri" || kind == "url")
        && let Some(uri) = action.uri.as_deref()
    {
        open_uri(uri);
        return;
    }
    if kind == "message" {
        let text = action.data.clone().unwrap_or_else(|| action.label.clone());
        if let Err(e) = state.sidecar.send_message(chat_mid, &text) {
            toast(
                state,
                &crate::i18n::tf("send_failed", &[("error", &e.to_string())]),
            );
        }
        return;
    }
    // postback / default
    let data = action.data.clone().unwrap_or_else(|| action.label.clone());
    match state
        .sidecar
        .send_postback(chat_mid, message_id, &data, action.uri.as_deref())
    {
        Ok(_) => toast(
            state,
            &crate::i18n::tf("action_sent", &[("label", &action.label)]),
        ),
        Err(e) => toast(
            state,
            &crate::i18n::tf("action_failed", &[("error", &e.to_string())]),
        ),
    }
}

pub(super) fn clear_messages(state: &AppState) {
    composer_drafts::sync(state);
    state.chat_event_labels.borrow_mut().clear();
    let generation = state.message_render_generation.borrow().wrapping_add(1);
    *state.message_render_generation.borrow_mut() = generation;
    state.emoji_views.borrow_mut().clear();
    if state.emoji_textures.borrow().len() > 1024 {
        state.emoji_textures.borrow_mut().clear();
    }
    let changed_chat = state.reply_target.borrow().as_ref().is_some_and(|target| {
        state.current_chat.borrow().as_deref() != Some(target.chat_mid.as_str())
    });
    if changed_chat {
        replies::cancel(state);
    }
    *state.render_batch.borrow_mut() = None;
    state.reply_messages.borrow_mut().clear();
    state.message_rows.borrow_mut().clear();
    reactions::clear(state);
    stop_voice_playback(state);
    state.message_list.clear();
    state.media_slots.borrow_mut().clear();
    state.media_msgs.borrow_mut().clear();
    state.receipt_slots.borrow_mut().clear();
    state.sender_labels.borrow_mut().clear();
    state.msg_created.borrow_mut().clear();
    state.pending_rows.borrow_mut().clear();
    *state.last_msg_day.borrow_mut() = None;
    state.seen_msg_ids.borrow_mut().clear();
    *state.last_incoming_id.borrow_mut() = None;
    state.media_queue.borrow_mut().clear();
    *state.media_pumping.borrow_mut() = false;
    *state.new_sep_row.borrow_mut() = None;
    *state.pending_new_below.borrow_mut() = 0;
    state.jump_banner.set_reveal_child(false);
    // Keep media_ready_paths across rebuilds so late slots can still attach.
}

pub(super) fn message_tracks_media(msg: &MessageInfo) -> bool {
    let ct = msg.content_type.to_ascii_uppercase();
    matches!(
        ct.as_str(),
        "IMAGE" | "VIDEO" | "AUDIO" | "FILE" | "STICKER"
    ) || msg.image_path.is_some()
        || msg.audio_path.is_some()
        || msg.file_path.is_some()
        || msg.flex.is_some()
}

pub(super) fn set_side_state(state: &AppState, name: &str, empty_text: Option<&str>) {
    if let Some(text) = empty_text {
        state.side_empty.set_text(text);
    }
    if name == "loading" {
        state.side_spinner.set_spinning(true);
        state.side_spinner.set_visible(true);
    }
    state.side_stack.set_visible_child_name(name);
}

pub(super) fn set_msg_state(state: &AppState, name: &str, empty_text: Option<&str>) {
    if let Some(text) = empty_text {
        state.msg_empty.set_text(text);
    }
    if name == "loading" {
        state.msg_spinner.set_spinning(true);
    }
    state.msg_stack.set_visible_child_name(name);
}

pub(super) fn preview_body_ui(msg: &MessageInfo) -> String {
    if msg.chat_event.is_some()
        || matches!(
            msg.content_type.to_ascii_uppercase().as_str(),
            "CHATEVENT" | "CHAT_EVENT" | "18"
        )
    {
        return crate::i18n::t("chat_event_unknown");
    }
    if let Some(preview) = call_history::history_preview(msg) {
        return preview;
    }
    let ct = msg.content_type.to_ascii_uppercase();
    match ct.as_str() {
        "UNSENT" => crate::i18n::t("message_unsent"),
        "IMAGE" => crate::i18n::t("preview_photo"),
        "VIDEO" => crate::i18n::t("preview_video"),
        "STICKER" => crate::i18n::t("preview_sticker"),
        "AUDIO" => crate::i18n::t("voice_message"),
        "FILE" => crate::i18n::t("preview_file"),
        "FLEX" => {
            let t = msg.text.trim();
            if t.is_empty() {
                crate::i18n::t("preview_flex")
            } else {
                t.to_string()
            }
        }
        _ => {
            let t = msg.text.trim();
            if t.is_empty() {
                if ct.is_empty() {
                    crate::i18n::t("preview_message")
                } else {
                    ct.to_ascii_lowercase()
                }
            } else if t.chars().count() > 64 {
                format!("{}…", t.chars().take(63).collect::<String>())
            } else {
                t.to_string()
            }
        }
    }
}

/// Rewrite cached English (or mismatched) preview prefixes into the active UI language.
pub(super) fn localize_preview(preview: &str) -> String {
    let mut s = preview.to_string();
    let pairs = [
        ("You: ", "you"),
        ("They: ", "they"),
        ("คุณ: ", "you"),
        ("เขา: ", "they"),
        ("自分: ", "you"),
        ("相手: ", "they"),
    ];
    for (prefix, key) in pairs {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = format!("{}: {}", crate::i18n::t(key), rest);
            break;
        }
    }
    // Localized media placeholders that may appear after the who-prefix.
    let replacements = [
        ("Photo", "preview_photo"),
        ("Video", "preview_video"),
        ("Sticker", "preview_sticker"),
        ("Voice message", "voice_message"),
        ("File", "preview_file"),
        ("Flex message", "preview_flex"),
        ("Message", "preview_message"),
        ("Tap to open", "tap_open"),
        ("Loading last message…", "loading_last"),
        ("Loading last message...", "loading_last"),
        ("No recent messages", "no_recent"),
        ("รูปภาพ", "preview_photo"),
        ("วิดีโอ", "preview_video"),
        ("สติกเกอร์", "preview_sticker"),
        ("ข้อความเสียง", "voice_message"),
        ("ไฟล์", "preview_file"),
        ("ข้อความ Flex", "preview_flex"),
        ("ข้อความ", "preview_message"),
        ("แตะเพื่อเปิด", "tap_open"),
        ("กำลังโหลดข้อความล่าสุด…", "loading_last"),
        ("ไม่มีข้อความล่าสุด", "no_recent"),
        ("写真", "preview_photo"),
        ("動画", "preview_video"),
        ("スタンプ", "preview_sticker"),
        ("ボイスメッセージ", "voice_message"),
        ("ファイル", "preview_file"),
        ("Flexメッセージ", "preview_flex"),
        ("メッセージ", "preview_message"),
        ("タップして開く", "tap_open"),
        ("最新のメッセージを読み込み中…", "loading_last"),
        ("最近のメッセージはありません", "no_recent"),
    ];
    if let Some((who, body)) = s.split_once(": ") {
        let mut body = body.to_string();
        for (from, key) in replacements {
            if body == from {
                body = crate::i18n::t(key);
                break;
            }
        }
        return format!("{who}: {body}");
    }
    for (from, key) in replacements {
        if s == from {
            return crate::i18n::t(key);
        }
    }
    s
}

pub(super) fn snap_adj_to_bottom(adj: &gtk::Adjustment) {
    let target = (adj.upper() - adj.page_size()).max(0.0);
    if (adj.value() - target).abs() > 0.5 {
        adj.set_value(target);
    }
}

pub(super) fn scroll_last_row_into_view(state: &AppState) {
    state.message_list.scroll_to_end();
    snap_adj_to_bottom(&state.message_scroll.vadjustment());
}

pub(super) fn scroll_messages_to_end(state: &AppState) {
    if !*state.stick_bottom.borrow() {
        return;
    }
    // Supersede any previous pin chain — overlapping clears were leaving stick=false
    // and stopping one message short of the latest.
    let pin_gen = {
        let mut g = state.scroll_pin_gen.borrow_mut();
        *g = g.saturating_add(1);
        *g
    };
    *state.scroll_pinning.borrow_mut() = true;
    state.message_list.queue_allocate();

    let state_c = state.clone();
    let step = Rc::new(RefCell::new(None::<Rc<dyn Fn(u32)>>));
    let step_fn = {
        let step = step.clone();
        let state_c = state_c.clone();
        Rc::new(move |attempt: u32| {
            if *state_c.scroll_pin_gen.borrow() != pin_gen {
                return;
            }
            if !*state_c.stick_bottom.borrow() {
                *state_c.scroll_pinning.borrow_mut() = false;
                return;
            }
            *state_c.scroll_pinning.borrow_mut() = true;
            scroll_last_row_into_view(&state_c);
            snap_adj_to_bottom(&state_c.message_scroll.vadjustment());

            let adj = state_c.message_scroll.vadjustment();
            let target = (adj.upper() - adj.page_size()).max(0.0);
            let at_bottom = adj.value() + 1.0 >= target;
            // Keep pinning across delayed ListBox allocation (~0.5–0.7s).
            // Require a few consecutive at-bottom frames so we don't stop one row short.
            if attempt >= 48 {
                *state_c.scroll_pinning.borrow_mut() = false;
                if *state_c.stick_bottom.borrow() {
                    *state_c.scroll_pinning.borrow_mut() = true;
                    scroll_last_row_into_view(&state_c);
                    snap_adj_to_bottom(&adj);
                    *state_c.scroll_pinning.borrow_mut() = false;
                }
                return;
            }
            if at_bottom && attempt >= 12 {
                // One more settle pass on the next ticks, then release.
                if attempt >= 18 {
                    *state_c.scroll_pinning.borrow_mut() = false;
                    return;
                }
            }
            let step = step.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(16), move || {
                if let Some(f) = step.borrow().as_ref() {
                    f(attempt + 1);
                }
            });
        }) as Rc<dyn Fn(u32)>
    };
    *step.borrow_mut() = Some(step_fn.clone());
    step_fn(0);
}

pub(super) fn pin_messages_to_latest(state: &AppState) {
    *state.stick_bottom.borrow_mut() = true;
    scroll_messages_to_end(state);
}

pub(super) fn state_chat_open(state: &AppState) -> bool {
    state.current_chat.borrow().is_some()
        && state.msg_stack.visible_child_name().as_deref() == Some("list")
}

pub(super) fn ensure_new_message_separator(state: &AppState) {
    if state.new_sep_row.borrow().is_some() {
        return;
    }
    let sep_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["line-message-row", "line-new-sep-row"])
        .build();
    let row_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .css_classes(["line-new-sep"])
        .build();
    let line = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(["line-new-sep-line"])
        .build();
    line.set_height_request(1);
    let badge = gtk::Label::builder()
        .label(crate::i18n::t("new_messages_badge"))
        .css_classes(["line-new-sep-badge"])
        .valign(gtk::Align::Center)
        .build();
    row_box.append(&line);
    row_box.append(&badge);
    sep_row.append(&row_box);
    state.message_list.append(&sep_row);
    *state.new_sep_row.borrow_mut() = Some(sep_row);
}

pub(super) fn dismiss_new_marker(state: &AppState) {
    if let Some(row) = state.new_sep_row.borrow_mut().take() {
        state.message_list.remove(&row);
    }
    *state.pending_new_below.borrow_mut() = 0;
    update_jump_banner(state);
}

pub(super) fn update_jump_banner(state: &AppState) {
    let n = *state.pending_new_below.borrow();
    let detached = history::is_detached(state);
    let show = detached || (n > 0 && !*state.stick_bottom.borrow());
    if show {
        let label = if detached && n == 0 {
            crate::i18n::t("shared_media_back_to_latest")
        } else if n == 1 {
            crate::i18n::t("jump_new_one")
        } else {
            crate::i18n::tf("jump_new_n", &[("n", &n.to_string())])
        };
        state.jump_banner_label.set_text(&label);
    }
    state.jump_banner.set_reveal_child(show);
}

pub(super) fn jump_to_latest(state: &AppState) {
    *state.pending_new_below.borrow_mut() = 0;
    update_jump_banner(state);
    if history::is_detached(state) {
        history::reload_latest(state);
        return;
    }
    pin_messages_to_latest(state);
    if let Some(mid) = state.current_chat.borrow().clone() {
        mark_chat_read(state, &mid);
        clear_unread_badge(state, &mid);
    }
}

#[cfg(test)]
mod localization_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn cached_previews_and_dates_follow_japanese() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        crate::i18n::set_lang(root, "ja");
        assert_eq!(localize_preview("You: Photo"), "自分: 写真");
        assert_eq!(localize_preview("เขา: สติกเกอร์"), "相手: スタンプ");
        assert_eq!(
            localize_preview("You: 日本語の本文です"),
            "自分: 日本語の本文です"
        );
        let timestamp = chrono::Local
            .with_ymd_and_hms(2020, 1, 2, 13, 5, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(format_day_separator(timestamp), "2020年1月2日");
        assert_eq!(format_msg_time(timestamp), "13:05");
        assert_eq!(
            format_outgoing_status(0, false, timestamp),
            "送信済み · 13:05"
        );
        assert_eq!(format_outgoing_status(1, false, timestamp), "既読 · 13:05");
        assert_eq!(format_outgoing_status(2, true, timestamp), "既読 2 · 13:05");
        crate::i18n::set_lang(root, "en");
        assert_eq!(localize_preview("自分: 写真"), "You: Photo");
        assert_eq!(localize_preview("相手: スタンプ"), "They: Sticker");
        assert_eq!(format_msg_time(timestamp), "1:05 PM");
    }
}
