use super::*;

pub(super) fn pump_events(state: AppState) {
    let rx = state.sidecar.events.clone();
    glib::spawn_future_local(async move {
        while let Ok(ev) = rx.recv().await {
            handle_event(&state, ev);
            // Drain a short burst so one flood cannot stall forever on await.
            for _ in 0..64 {
                match rx.try_recv() {
                    Ok(ev) => handle_event(&state, ev),
                    Err(_) => break,
                }
            }
        }
    });
}

pub(super) fn handle_event(state: &AppState, ev: ProtocolEvent) {
    match ev {
        ProtocolEvent::Ready { has_auth } => {
            let restarting = *state.restarting.borrow();
            if restarting {
                *state.restarting.borrow_mut() = false;
            }
            if has_auth && !restarting {
                // Sidecar is already restoring; keep shell visible (no QR flash).
                show_shell_restoring(state);
            } else if restarting || !has_auth {
                show_login(state, &crate::i18n::t("login_qr_hint"));
                if let Ok(id) = state.sidecar.login_qr() {
                    state.pending.borrow_mut().insert(id, Pending::Login);
                }
            }
        }
        ProtocolEvent::Session {
            mid,
            display_name,
            status_message,
            picture_path,
            avatar_path,
            picture_url,
        } => {
            *state.recovery_attempts.borrow_mut() = 0;
            let result = serde_json::json!({
                "mid": mid,
                "displayName": display_name,
                "statusMessage": status_message,
                "picturePath": picture_path,
                "avatarPath": avatar_path,
                "pictureUrl": picture_url,
            });
            on_logged_in(state, &result);
        }
        ProtocolEvent::SessionFailed { error } => {
            *state.session_ready.borrow_mut() = false;
            hide_upload_progress(state);
            let msg = if error.contains("relogin_android_required") {
                crate::i18n::t("relogin_android_required")
            } else if error.contains("NOT_AUTHORIZED_DEVICE")
                || error.contains("V3_TOKEN_CLIENT_LOGGED_OUT")
                || error.contains("LOGGED_OUT")
            {
                crate::i18n::t("logged_out_hint")
            } else {
                error.clone()
            };
            show_login(state, &format!("{msg}\nTap Retry to sign in with QR."));
            state.login.retry_btn.set_visible(true);
            *state.discord_session_start.borrow_mut() = None;
            sync_discord_rpc(state);
            toast(state, &msg);
        }
        ProtocolEvent::Qr { url } => {
            show_login(state, &crate::i18n::t("login_qr_hint"));
            login::show_qr_stage(&state.login);
            if let Err(e) = login::set_qr(&state.login.qr_picture, &url) {
                state.login.hint.set_text(&format!("QR URL: {url}\n({e})"));
            }
        }
        ProtocolEvent::Pin { pin } => {
            show_login(state, &crate::i18n::t("login_pin_waiting"));
            login::show_pin_stage(&state.login, &pin);
        }
        ProtocolEvent::Listening => {
            state.status.set_text(&crate::i18n::t("live"));
            sync_discord_rpc(state);
        }
        ProtocolEvent::CallIncoming {
            call_id,
            from,
            kind,
        } => {
            tracing::info!(%call_id, %from, %kind, "incoming call");
            if !calls_experimental_enabled(state) {
                // Locked: ignore ringing UI (still experimental).
                let _ = state.sidecar.call_decline();
                return;
            }
            let name = peer_display_name(state, &from);
            *state.incoming_call_from.borrow_mut() = Some(from.clone());
            ensure_call_window(
                state,
                &name,
                CallMode::Incoming(&crate::i18n::tf("call_incoming", &[("name", &name)])),
            );
            notify_call_incoming(state, &name);
        }
        ProtocolEvent::CallCanceled {
            call_id,
            from,
            reason,
        } => {
            tracing::info!(%call_id, %from, %reason, "call canceled");
            let name = peer_display_name(state, &from);
            toast(state, &crate::i18n::tf("call_canceled", &[("name", &name)]));
            let was_incoming = state.incoming_call_from.borrow().as_deref() == Some(from.as_str());
            let was_active = state.active_call_peer.borrow().as_deref() == Some(from.as_str());
            if was_incoming {
                *state.incoming_call_from.borrow_mut() = None;
            }
            if was_incoming || was_active {
                close_active_call_ui(state);
            }
        }
        ProtocolEvent::CallState {
            call_id,
            peer,
            state: call_state,
            error,
            video_capable,
        } => {
            tracing::debug!(%call_id, %peer, state = %call_state, ?error, "call state changed");
            handle_call_state(state, &peer, &call_state, error.as_deref(), video_capable);
        }
        ProtocolEvent::ScreenShareState {
            state: screen_state,
            error,
        } => {
            handle_screen_share_state(state, &screen_state, error.as_deref());
        }
        ProtocolEvent::Message(msg) => {
            let peer = if !msg.chat_mid.is_empty() {
                msg.chat_mid.clone()
            } else if msg.mine {
                msg.to.clone()
            } else {
                msg.from.clone()
            };
            let mut preview = format!(
                "{}: {}",
                if msg.mine {
                    crate::i18n::t("you")
                } else if !msg.sender_name.is_empty() {
                    msg.sender_name.clone()
                } else {
                    crate::i18n::t("they")
                },
                preview_body_ui(&msg)
            );
            preview = localize_preview(&preview);

            // New friend / first message: create the sidebar row immediately.
            ensure_chat_visible(
                state,
                &ChatInfo {
                    mid: peer.clone(),
                    name: peer_display_name(state, &peer),
                    kind: if peer.starts_with('m') {
                        "openchat".into()
                    } else if peer.starts_with('c') {
                        "group".into()
                    } else {
                        "dm".into()
                    },
                    avatar_path: None,
                    last_activity: msg.created_time,
                    unread: 0,
                    preview: preview.clone(),
                    muted: false,
                    pinned: false,
                },
            );

            if let Some(label) = state.chat_previews.borrow().get(&peer) {
                label.set_text(&preview);
            }
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == peer) {
                chat.preview = preview.clone();
                if msg.created_time > chat.last_activity {
                    chat.last_activity = msg.created_time;
                }
            }
            promote_chat_to_top(state, &peer);

            let current = state.current_chat.borrow().clone();
            let viewing = current.as_deref() == Some(peer.as_str());
            let window_focused = state.window.is_active();
            if viewing {
                if state.msg_stack.visible_child_name().as_deref() != Some("list") {
                    set_msg_state(state, "list", None);
                }
                // Capture before append — layout growth can briefly look "scrolled up".
                let was_stuck = *state.stick_bottom.borrow();
                // "New" marker only when the user is scrolled up (LINE-like).
                if !msg.mine && !was_stuck {
                    ensure_new_message_separator(state);
                }
                append_message(state, &msg, true);
                if was_stuck {
                    pin_messages_to_latest(state);
                    if !msg.mine {
                        // Only suppress alerts when this chat is open AND focused.
                        if window_focused {
                            mark_chat_read(state, &peer);
                            clear_unread_badge(state, &peer);
                        } else {
                            bump_unread(state, &peer);
                            notify_incoming(state, &msg, &peer);
                        }
                    }
                } else if !msg.mine {
                    let n = state.pending_new_below.borrow().saturating_add(1);
                    *state.pending_new_below.borrow_mut() = n;
                    update_jump_banner(state);
                    bump_unread(state, &peer);
                    notify_incoming(state, &msg, &peer);
                }
            } else if !msg.mine {
                bump_unread(state, &peer);
                notify_incoming(state, &msg, &peer);
            }
        }
        ProtocolEvent::ChatUpsert { chat } => {
            let mut chat = chat;
            if !chat.preview.is_empty() {
                chat.preview = localize_preview(&chat.preview);
            }
            upsert_chat_row(state, chat);
        }
        ProtocolEvent::ReadReceipt {
            chat_mid,
            message_id,
        } => {
            apply_peer_read(state, &chat_mid, &message_id);
        }
        ProtocolEvent::Chats { chats, cached } => {
            apply_chats(state, chats, cached);
        }
        ProtocolEvent::Messages { chat_mid, messages } => {
            if state.current_chat.borrow().as_deref() == Some(chat_mid.as_str()) {
                apply_messages(state, messages);
            }
        }
        ProtocolEvent::StickersUpdated { result } => {
            if state.sticker_popover.is_visible() {
                fill_sticker_popover(state, &result);
            }
        }
        ProtocolEvent::AvatarReady { mid, avatar_path } => {
            if state.self_mid.borrow().as_deref() == Some(mid.as_str()) {
                *state.self_avatar_path.borrow_mut() = Some(avatar_path.clone());
                if std::path::Path::new(&avatar_path).exists() {
                    attach_avatar_texture_async(
                        state.profile_avatar.clone(),
                        avatar_path.clone(),
                        26,
                    );
                }
                sync_discord_rpc(state);
            }
            if let Some(img) = state.chat_avatars.borrow().get(&mid).cloned()
                && std::path::Path::new(&avatar_path).exists()
            {
                let px = img.width_request().max(AVATAR_COMPACT_PX);
                attach_avatar_texture_async(img, avatar_path.clone(), px);
            }
            if let Some(ui) = state.friends_ui.borrow().as_ref() {
                if let Some(img) = ui.avatars.borrow().get(&mid).cloned()
                    && std::path::Path::new(&avatar_path).exists()
                {
                    attach_avatar_texture_async(img, avatar_path.clone(), 40);
                }
                if let Some(f) = ui.friends.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                    f.avatar_path = Some(avatar_path.clone());
                }
            }
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                chat.avatar_path = Some(avatar_path);
            }
        }
        ProtocolEvent::FriendsUpdated { friends } => {
            if let Some(ui) = state.friends_ui.borrow().as_ref() {
                friends::apply_friends(ui, friends);
            }
        }
        ProtocolEvent::ChatPreview {
            mid,
            preview,
            last_activity,
        } => {
            let preview = localize_preview(&preview);
            if let Some(label) = state.chat_previews.borrow().get(&mid) {
                label.set_text(&preview);
            }
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                chat.preview = preview;
                if last_activity > chat.last_activity {
                    chat.last_activity = last_activity;
                }
            }
        }
        ProtocolEvent::ChatMute { mid, muted } => {
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                chat.muted = muted;
            }
            if state.current_chat.borrow().as_deref() == Some(mid.as_str()) {
                update_mute_btn(state, muted);
            }
            toast(
                state,
                &if muted {
                    crate::i18n::t("chat_muted")
                } else {
                    crate::i18n::t("chat_unmuted")
                },
            );
        }
        ProtocolEvent::MediaReady {
            chat_mid,
            message_id,
            image_path,
            audio_path,
            file_path,
        } => {
            // Always remember hydrated paths so list rebuilds / late bubbles can attach.
            if !image_path.is_empty() && std::path::Path::new(&image_path).exists() {
                state
                    .media_ready_paths
                    .borrow_mut()
                    .insert(message_id.clone(), image_path.clone());
                refresh_notification_media(state, &message_id, &image_path);
            }

            if state.current_chat.borrow().as_deref() != Some(chat_mid.as_str()) {
                return;
            }
            if let Some(meta) = state.media_msgs.borrow_mut().get_mut(&message_id) {
                if !image_path.is_empty() {
                    meta.image_path = Some(image_path.clone());
                }
                if let Some(ap) = audio_path.as_ref() {
                    meta.audio_path = Some(ap.clone());
                }
                if let Some(fp) = file_path.as_ref() {
                    meta.file_path = Some(fp.clone());
                }
            }
            if let Some(path) = audio_path {
                if std::path::Path::new(&path).exists() {
                    attach_audio_to_slot(state, &message_id, &path);
                }
                return;
            }
            if image_path.is_empty() || !std::path::Path::new(&image_path).exists() {
                mark_media_failed(state, &message_id);
                return;
            }
            state
                .media_queue
                .borrow_mut()
                .push_back((message_id, image_path));
            pump_media_queue(state);
        }
        ProtocolEvent::MediaFailed {
            chat_mid,
            message_id,
        } => {
            if state.current_chat.borrow().as_deref() != Some(chat_mid.as_str()) {
                return;
            }
            // A later/parallel hydrate may already have the bytes on disk.
            if let Some(path) = state.media_ready_paths.borrow().get(&message_id).cloned()
                && std::path::Path::new(&path).exists()
            {
                state.media_queue.borrow_mut().push_back((message_id, path));
                pump_media_queue(state);
                return;
            }
            mark_media_failed(state, &message_id);
        }
        ProtocolEvent::Progress {
            scope,
            chat_mid,
            state: prog,
            error,
        } => {
            match scope.as_str() {
                "chats" => match prog.as_str() {
                    "loading" => {
                        state.side_spinner.set_spinning(true);
                        state.side_spinner.set_visible(true);
                        if state.chats.borrow().is_empty() {
                            set_side_state(state, "loading", None);
                        }
                        state.status.set_text(&crate::i18n::t("refreshing_chats"));
                    }
                    "ready" => {
                        state.side_spinner.set_spinning(false);
                        state.side_spinner.set_visible(false);
                        if !state.chats.borrow().is_empty() {
                            set_side_state(state, "list", None);
                        }
                    }
                    "empty" => {
                        state.side_spinner.set_spinning(false);
                        state.side_spinner.set_visible(false);
                        set_side_state(state, "empty", Some(&crate::i18n::t("no_chats")));
                        state.status.set_text(&crate::i18n::t("status_no_chats"));
                    }
                    "error" => {
                        state.side_spinner.set_spinning(false);
                        state.side_spinner.set_visible(false);
                        let msg = error.unwrap_or_else(|| "Failed to load chats".into());
                        set_side_state(state, "empty", Some(&msg));
                        toast(state, &msg);
                    }
                    _ => {}
                },
                "messages" => {
                    let current = state.current_chat.borrow().clone();
                    if chat_mid.as_deref() != current.as_deref() {
                        return;
                    }
                    match prog.as_str() {
                        "loading" => {
                            if state.message_list.first_child().is_none() {
                                set_msg_state(state, "loading", None);
                            } else {
                                state.chat_subtitle.set_text(&crate::i18n::t("refreshing"));
                            }
                        }
                        "ready" => {
                            if state.message_list.first_child().is_some() {
                                set_msg_state(state, "list", None);
                            }
                        }
                        "empty" => {
                            // Don't clobber a thread that already has rows (e.g. optimistic send).
                            if state.message_list.first_child().is_none() {
                                set_msg_state(state, "empty", Some(&crate::i18n::t("no_messages")));
                                state.chat_subtitle.set_text(&crate::i18n::t("empty"));
                            }
                        }
                        "error" => {
                            let msg = error.unwrap_or_else(|| "Failed to load messages".into());
                            set_msg_state(state, "empty", Some(&msg));
                            toast(state, &msg);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        ProtocolEvent::UploadProgress {
            chat_mid,
            progress,
            label,
            done,
        } => {
            if state.current_chat.borrow().as_deref() != Some(chat_mid.as_str()) {
                return;
            }
            if done {
                if progress >= 1.0 {
                    state.upload_bar.set_fraction(1.0);
                    let text = if label.is_empty() {
                        crate::i18n::t("media_upload_sent")
                    } else {
                        label
                    };
                    state.upload_label.set_text(&text);
                    let s = state.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(450),
                        move || {
                            hide_upload_progress(&s);
                        },
                    );
                } else {
                    hide_upload_progress(state);
                }
                return;
            }
            show_upload_progress(state, progress, &label);
        }
        ProtocolEvent::Response {
            id,
            ok,
            result,
            error,
        } => {
            let pending = state.pending.borrow_mut().remove(&id);
            if !ok {
                if matches!(pending, Some(Pending::Login)) && *state.restarting.borrow() {
                    return;
                }
                if matches!(pending, Some(Pending::Login)) {
                    show_login(
                        state,
                        error
                            .as_deref()
                            .unwrap_or("Login failed. Tap Retry to start over."),
                    );
                    login::show_qr_stage(&state.login);
                    state.login.retry_btn.set_visible(true);
                }
                if matches!(pending, Some(Pending::ListChats)) {
                    set_side_state(
                        state,
                        "empty",
                        Some(error.as_deref().unwrap_or("Failed to load chats")),
                    );
                }
                if matches!(pending, Some(Pending::FetchMessages { .. })) {
                    set_msg_state(
                        state,
                        "empty",
                        Some(error.as_deref().unwrap_or("Failed to load messages")),
                    );
                }
                if matches!(pending, Some(Pending::ListFriends))
                    && let Some(ui) = state.friends_ui.borrow().as_ref()
                {
                    ui.stack.set_visible_child_name("empty");
                    ui.empty.set_text(
                        error
                            .as_deref()
                            .unwrap_or(&crate::i18n::t("friends_load_failed")),
                    );
                }
                if let Some(Pending::Send { placeholder_id, .. }) = pending.as_ref() {
                    hide_upload_progress(state);
                    remove_pending_placeholder(state, placeholder_id);
                }
                let err = error.as_deref().unwrap_or("request failed");
                if let Some(Pending::ProfileLookup(ui)) = pending.as_ref() {
                    let message = crate::i18n::tf("profile_check_failed", &[("error", err)]);
                    ui.status.set_text(&message);
                    ui.add_btn.set_sensitive(false);
                    ui.chat_btn.set_sensitive(false);
                    toast(state, &message);
                    return;
                }
                if let Some(Pending::ProfileAddFriend(ui)) = pending.as_ref() {
                    let message = crate::i18n::tf("profile_add_failed", &[("error", err)]);
                    ui.status.set_text(&message);
                    ui.add_btn.set_sensitive(true);
                    ui.chat_btn.set_sensitive(false);
                    toast(state, &message);
                    return;
                }
                if matches!(pending, Some(Pending::CallScreenStart)) {
                    *state.call_screen_sharing.borrow_mut() = false;
                    if let Some(ui) = state.call_ui.borrow().as_ref() {
                        call_window::update_screen_visual(ui, false);
                    }
                    toast(
                        state,
                        &crate::i18n::tf("call_share_failed", &[("error", err)]),
                    );
                    return;
                }
                if matches!(pending, Some(Pending::CallScreenStop)) {
                    *state.call_screen_sharing.borrow_mut() = true;
                    if let Some(ui) = state.call_ui.borrow().as_ref() {
                        call_window::update_screen_visual(ui, true);
                    }
                    toast(
                        state,
                        &crate::i18n::tf("call_share_failed", &[("error", err)]),
                    );
                    return;
                }
                if matches!(pending, Some(Pending::ReactMessage)) {
                    toast(
                        state,
                        &crate::i18n::tf("message_reaction_failed", &[("error", err)]),
                    );
                    return;
                }
                if matches!(pending, Some(Pending::UnsendMessage)) {
                    toast(
                        state,
                        &crate::i18n::tf("message_unsend_failed", &[("error", err)]),
                    );
                    return;
                }
                if err.contains("sticker_not_owned") || err.contains("USER_NOT_STICKER_OWNER") {
                    toast(state, &crate::i18n::t("sticker_not_owned"));
                } else {
                    toast(state, err);
                }
                return;
            }
            match pending {
                Some(Pending::Login) => on_logged_in(state, &result),
                Some(Pending::ListChats) => {
                    let chats: Vec<ChatInfo> = result
                        .get("chats")
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    let cached = result
                        .get("cached")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    apply_chats(state, chats, cached);
                }
                Some(Pending::ListFriends) => {
                    let friends: Vec<ChatInfo> = result
                        .get("friends")
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    if let Some(ui) = state.friends_ui.borrow().as_ref() {
                        friends::apply_friends(ui, friends);
                    }
                }
                Some(Pending::FetchMessages { chat_mid }) => {
                    if state.current_chat.borrow().as_deref() == Some(chat_mid.as_str()) {
                        let messages: Vec<MessageInfo> = result
                            .get("messages")
                            .cloned()
                            .and_then(|v| serde_json::from_value(v).ok())
                            .unwrap_or_default();
                        if !same_message_list(state, &chat_mid, &messages) {
                            apply_messages(state, messages);
                        }
                    }
                }
                Some(Pending::Send {
                    placeholder_id,
                    chat_mid,
                }) => {
                    remove_pending_placeholder(state, &placeholder_id);
                    if state.current_chat.borrow().as_deref() != Some(chat_mid.as_str()) {
                        return;
                    }
                    if let Ok(msg) = serde_json::from_value::<MessageInfo>(
                        result.get("message").cloned().unwrap_or_default(),
                    ) {
                        if state.msg_stack.visible_child_name().as_deref() != Some("list") {
                            set_msg_state(state, "list", None);
                        }
                        append_message(state, &msg, true);
                        pin_messages_to_latest(state);
                    }
                }
                Some(Pending::ListStickers) => {
                    fill_sticker_popover(state, &result);
                }
                Some(Pending::DownloadMedia {
                    action,
                    content_type,
                    suggest_name,
                    message_id,
                    ..
                }) => {
                    if let Some(path) = result.get("path").and_then(|v| v.as_str()) {
                        if let Some(meta) = state.media_msgs.borrow_mut().get_mut(&message_id) {
                            meta.file_path = Some(path.to_string());
                            if content_type.eq_ignore_ascii_case("image") {
                                meta.image_path = Some(path.to_string());
                            } else if content_type.eq_ignore_ascii_case("audio") {
                                meta.audio_path = Some(path.to_string());
                            }
                        }
                        finish_media_action(state, &action, path, &suggest_name, &content_type);
                    } else {
                        toast(state, &crate::i18n::t("media_download_failed"));
                    }
                }
                Some(Pending::ProfileLookup(ui)) => {
                    apply_profile_relation(state, &ui, &result, false);
                }
                Some(Pending::ProfileAddFriend(ui)) => {
                    apply_profile_relation(state, &ui, &result, true);
                }
                Some(Pending::ReactMessage) => {}
                Some(Pending::UnsendMessage) => {
                    toast(state, &crate::i18n::t("message_unsent"));
                }
                Some(Pending::CallScreenStart | Pending::CallScreenStop) => {}
                None => {}
            }
        }
        ProtocolEvent::Error(e) => {
            tracing::error!(error = %e, "protocol error");
            toast(state, &e);
        }
        ProtocolEvent::Exited(code) => {
            if !*state.restarting.borrow() {
                tracing::warn!(code, "protocol engine exited unexpectedly");
                schedule_sidecar_recovery(state);
            }
        }
    }
}

fn apply_profile_relation(
    state: &AppState,
    ui: &ProfilePendingUi,
    result: &serde_json::Value,
    just_added: bool,
) {
    {
        let mut target = ui.target.borrow_mut();
        if let Some(mid) = result.get("mid").and_then(|value| value.as_str()) {
            target.mid = mid.to_string();
        }
        if let Some(name) = result
            .get("displayName")
            .and_then(|value| value.as_str())
            .filter(|name| !name.is_empty())
        {
            target.name = name.to_string();
            ui.name_label.set_text(name);
        }
        if let Some(path) = result
            .get("avatarPath")
            .and_then(|value| value.as_str())
            .filter(|path| std::path::Path::new(path).exists())
        {
            target.avatar_path = Some(path.to_string());
            attach_avatar_texture_async(ui.avatar.clone(), path.to_string(), 96);
        }
    }
    let bio = result
        .get("statusMessage")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    ui.bio_label.set_text(bio);
    ui.bio_label.set_visible(!bio.is_empty());

    let is_friend = just_added
        || result
            .get("isFriend")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    let blocked = result
        .get("blocked")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let can_add = !just_added
        && result
            .get("canAdd")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    let can_chat = is_friend
        && result
            .get("canChat")
            .and_then(|value| value.as_bool())
            .unwrap_or(is_friend);

    ui.add_btn.set_visible(!is_friend);
    ui.add_btn.set_sensitive(can_add);
    ui.chat_btn.set_sensitive(can_chat);
    let status = if just_added {
        crate::i18n::t("profile_add_success")
    } else if is_friend {
        crate::i18n::t("profile_friend")
    } else if blocked {
        crate::i18n::t("profile_blocked")
    } else {
        crate::i18n::t("profile_not_friend")
    };
    ui.status.set_text(&status);
    if just_added {
        toast(state, &crate::i18n::t("profile_add_success"));
    }
}

const MAX_SIDECAR_RECOVERY_ATTEMPTS: u8 = 3;

pub(super) fn schedule_sidecar_recovery(state: &AppState) {
    let attempt = {
        let mut attempts = state.recovery_attempts.borrow_mut();
        if *attempts >= MAX_SIDECAR_RECOVERY_ATTEMPTS {
            let notice = libadwaita::Toast::new(&crate::i18n::t("protocol_recovery_failed"));
            notice.set_button_label(Some(&crate::i18n::t("retry")));
            notice.set_action_name(Some("app.retry-protocol"));
            notice.set_timeout(0);
            state.toast_overlay.add_toast(notice);
            state.status.set_text(&crate::i18n::t("protocol_offline"));
            return;
        }
        *attempts += 1;
        *attempts
    };
    let delay = 1_u64 << (attempt - 1);
    state.status.set_text(&crate::i18n::tf(
        "protocol_reconnecting",
        &[("attempt", &attempt.to_string())],
    ));
    let s = state.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(delay), move || {
        recover_sidecar(&s);
    });
}

pub(super) fn recover_sidecar(state: &AppState) {
    match state.sidecar.recover() {
        Ok(()) => {
            state
                .status
                .set_text(&crate::i18n::t("protocol_restarting"));
        }
        Err(error) => {
            tracing::error!(%error, "protocol recovery failed");
            schedule_sidecar_recovery(state);
        }
    }
}

pub(super) fn on_logged_in(state: &AppState, result: &serde_json::Value) {
    match serde_json::from_value::<Profile>(result.clone()) {
        Ok(profile) => {
            *state.session_ready.borrow_mut() = true;
            apply_self_profile(
                state,
                &profile.mid,
                &profile.display_name,
                profile.avatar_path.as_deref(),
                profile
                    .picture_url
                    .as_deref()
                    .or(profile.picture_path.as_deref()),
            );
            state.profile_label.set_tooltip_text(
                (!profile.status_message.is_empty()).then_some(profile.status_message.as_str()),
            );
            state.stack.set_visible_child_name("shell");
            // Soft transitions only after we're past the cold boot.
            if state.config.borrow().animations {
                state
                    .stack
                    .set_transition_type(gtk::StackTransitionType::Crossfade);
                state.stack.set_transition_duration(180);
            } else {
                state
                    .stack
                    .set_transition_type(gtk::StackTransitionType::None);
                state.stack.set_transition_duration(0);
            }
            state.status.set_text(&crate::i18n::t("loading_chats"));
            set_side_state(state, "loading", None);
            set_msg_state(state, "idle", None);
            if let Ok(id) = state.sidecar.list_chats() {
                state.pending.borrow_mut().insert(id, Pending::ListChats);
            }
            // If chats were already painted from a warm cache event, open last chat now.
            maybe_restore_last_chat(state);
            sync_discord_rpc(state);
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("bad_profile", &[("error", &e.to_string())]),
        ),
    }
}

pub(super) fn saved_auth_exists(data_dir: &std::path::Path) -> bool {
    let path = data_dir.join("auth-token.txt");
    match std::fs::read_to_string(&path) {
        Ok(s) => !s.trim().is_empty(),
        Err(_) => false,
    }
}

pub(super) fn show_shell_restoring(state: &AppState) {
    state.stack.set_visible_child_name("shell");
    state.status.set_text(&crate::i18n::t("restoring"));
    state.side_spinner.set_spinning(true);
    state.side_spinner.set_visible(true);
    if state.chats.borrow().is_empty() {
        set_side_state(state, "loading", None);
    }
}

pub(super) fn show_login(state: &AppState, hint: &str) {
    state.stack.set_visible_child_name("login");
    login::show_qr_stage(&state.login);
    state.login.hint.set_text(hint);
    sync_discord_rpc(state);
}
