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
        ProtocolEvent::MessageUnsent {
            account_mid,
            chat_mid,
            message_id,
        } => {
            unsend::apply(state, &account_mid, &chat_mid, &message_id);
            if state.self_mid.borrow().as_deref() == Some(account_mid.as_str()) {
                shared_media::unsent(state, &chat_mid, &message_id);
                message_search::unsent(state, &chat_mid);
            }
        }
        ProtocolEvent::MessageCacheLimit {
            chat_mid,
            count,
            limit,
            full,
        } => cache_limit::warn(state, &chat_mid, count, limit, full),
        ProtocolEvent::MessageReaction {
            account_mid,
            chat_mid,
            message_id,
            from,
            reaction,
        } => reactions::apply(state, &account_mid, &chat_mid, &message_id, &from, reaction),
        ProtocolEvent::ChatEvent {
            account_mid,
            message,
        } => {
            if *state.session_ready.borrow()
                && state.self_mid.borrow().as_deref() == Some(account_mid.as_str())
                && state.current_chat.borrow().as_deref() == Some(message.to.as_str())
            {
                if history::is_detached(state) {
                    return;
                }
                let stick = *state.stick_bottom.borrow();
                set_msg_state(state, "list", None);
                append_message(state, &message, true);
                note_appended(state, &message);
                if stick {
                    pin_messages_to_latest(state);
                    history::enforce_live_limit(state);
                }
            }
        }
        ProtocolEvent::Ready { has_auth } => {
            let restarting = *state.restarting.borrow();
            if restarting {
                *state.restarting.borrow_mut() = false;
            }
            if auto_login::ready(state, has_auth, restarting) {
                return;
            }
            if has_auth && !restarting {
                // Sidecar is already restoring; keep shell visible (no QR flash).
                show_shell_restoring(state);
            } else if restarting || !has_auth {
                login::set_busy(&state.login, false);
                show_login(state, &login::method_hint(&state.login));
                if state.login.method.get() == login::LoginMethod::Qr {
                    if let Ok(id) = state.sidecar.login_qr() {
                        state.pending.borrow_mut().insert(id, Pending::Login);
                    } else {
                        login::show_failure(&state.login, &crate::i18n::t("login_email_failed"));
                    }
                }
            }
        }
        ProtocolEvent::Session {
            mid,
            device,
            display_name,
            status_message,
            picture_path,
            avatar_path,
            picture_url,
        } => {
            if state.self_mid.borrow().as_deref() != Some(mid.as_str()) {
                message_search::close(state);
            }
            // A restored profile does not yet prove that linejs polling stays
            // alive. Reset only after a stable period, so a crashing listener
            // cannot create an endless restore/crash loop.
            let s = state.clone();
            let pid = state.sidecar.status().pid;
            glib::timeout_add_local_once(std::time::Duration::from_secs(30), move || {
                let status = s.sidecar.status();
                if status.running && status.pid == pid && *s.session_ready.borrow() {
                    *s.recovery_attempts.borrow_mut() = 0;
                }
            });
            let result = serde_json::json!({
                "mid": mid,
                "device": device,
                "displayName": display_name,
                "statusMessage": status_message,
                "picturePath": picture_path,
                "avatarPath": avatar_path,
                "pictureUrl": picture_url,
            });
            on_logged_in(state, &result);
        }
        ProtocolEvent::CachedBoot {
            mid,
            display_name,
            status_message,
            picture_path,
            avatar_path,
            picture_url,
            chats,
        } => cache_boot::show(
            state,
            &mid,
            &display_name,
            &status_message,
            avatar_path.as_deref(),
            picture_url.as_deref().or(picture_path.as_deref()),
            chats,
        ),
        ProtocolEvent::SessionFailed { error } => {
            cache_boot::abort(state);
            *state.session_ready.borrow_mut() = false;
            hide_upload_progress(state);
            if auto_login::restore_failed(state, &error) {
                return;
            }
            auto_login::failed(state);
            let msg = if error.contains("relogin_android_required") {
                crate::i18n::t("relogin_android_required")
            } else if error == "line_auth_expired"
                || error.contains("NOT_AUTHORIZED_DEVICE")
                || error.contains("V3_TOKEN_CLIENT_LOGGED_OUT")
                || error.contains("LOGGED_OUT")
            {
                crate::i18n::t("logged_out_hint")
            } else {
                error.clone()
            };
            show_login(state, &msg);
            login::show_failure(&state.login, &msg);
            *state.discord_session_start.borrow_mut() = None;
            sync_discord_rpc(state);
            toast(state, &msg);
        }
        ProtocolEvent::Connection {
            state: connection,
            kind,
            attempt,
        } => {
            if connection == "connection_retry" {
                state.status.set_text(&crate::i18n::tf(
                    "line_reconnecting",
                    &[("attempt", &attempt.to_string())],
                ));
            } else if connection == "connection_lost" {
                schedule_sidecar_recovery(state);
            } else {
                let key = match kind.as_str() {
                    "network" => "line_network_unavailable",
                    "server" => "line_server_unavailable",
                    _ => "line_protocol_failed",
                };
                let message = crate::i18n::t(key);
                state.status.set_text(&message);
                let notice = libadwaita::Toast::new(&message);
                notice.set_button_label(Some(&crate::i18n::t("retry")));
                // This action preserves credentials; the login screen's Retry
                // intentionally clears them to request a fresh QR instead.
                notice.set_action_name(Some("app.retry-protocol"));
                notice.set_timeout(0);
                state.toast_overlay.add_toast(notice);
            }
        }
        ProtocolEvent::Qr { url } => {
            if state.login.method.get() != login::LoginMethod::Qr {
                return;
            }
            show_login(state, &crate::i18n::t("login_qr_hint"));
            login::show_qr_stage(&state.login);
            if let Err(e) = login::set_qr(&state.login.qr_picture, &url) {
                state.login.hint.set_text(&format!("QR URL: {url}\n({e})"));
            }
        }
        ProtocolEvent::Pin { pin } => {
            if !state
                .pending
                .borrow()
                .values()
                .any(|request| matches!(request, Pending::Login | Pending::LoginEmail))
            {
                return;
            }
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
        } => {
            tracing::debug!(%call_id, %peer, state = %call_state, ?error, "call state changed");
            handle_call_state(state, &peer, &call_state, error.as_deref());
        }
        ProtocolEvent::Message(msg) => {
            if msg.chat_event.is_some() {
                let account_mid = state.self_mid.borrow().clone().unwrap_or_default();
                handle_event(
                    state,
                    ProtocolEvent::ChatEvent {
                        account_mid,
                        message: msg,
                    },
                );
                return;
            }
            let msg = unsend::masked(state, &msg);
            // 取消後に遅れて届いた本文で、一覧や通知を復活させない。
            if msg.content_type == "UNSENT" {
                return;
            }
            let peer = if msg.mine || msg.to.starts_with(['c', 'r']) {
                msg.to.clone()
            } else {
                msg.from.clone()
            };
            let already_read = state.unread_sync.borrow().already_read(&peer, &msg.id);
            state.unread_sync.borrow_mut().observe(&peer, &msg.id);
            let mut preview = format!(
                "{}: {}",
                if msg.mine {
                    crate::i18n::t("you")
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
                    kind: if peer.starts_with('c') {
                        "group".into()
                    } else if peer.starts_with('r') {
                        "room".into()
                    } else {
                        "dm".into()
                    },
                    avatar_path: None,
                    last_activity: msg.created_time,
                    unread: 0,
                    preview: preview.clone(),
                    muted: false,
                    contact_only: Some(false),
                },
            );

            if let Some(label) = state.chat_previews.borrow().get(&peer) {
                label.set_text(&preview);
            }
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == peer) {
                chat.preview = preview.clone();
                chat.contact_only = Some(false);
                if msg.created_time > chat.last_activity {
                    chat.last_activity = msg.created_time;
                }
            }
            promote_chat_to_top(state, &peer);

            let current = state.current_chat.borrow().clone();
            let viewing = current.as_deref() == Some(peer.as_str());
            if !viewing {
                chat_snapshots::push_live(state, &peer, &msg);
            }
            let window_focused = state.window.is_active();
            if viewing && history::is_detached(state) {
                // The newest rows are not on screen; reaching them reloads the latest page.
                if !msg.mine && !already_read {
                    let n = state.pending_new_below.borrow().saturating_add(1);
                    *state.pending_new_below.borrow_mut() = n;
                    update_jump_banner(state);
                    bump_unread(state, &peer);
                    notify_incoming(state, &msg, &peer);
                }
            } else if viewing {
                if state.msg_stack.visible_child_name().as_deref() != Some("list") {
                    set_msg_state(state, "list", None);
                }
                // Capture before append — layout growth can briefly look "scrolled up".
                let was_stuck = *state.stick_bottom.borrow();
                // "New" marker only when the user is scrolled up (LINE-like).
                if !msg.mine && !already_read && !was_stuck {
                    ensure_new_message_separator(state);
                }
                append_message(state, &msg, true);
                note_appended(state, &msg);
                if was_stuck {
                    pin_messages_to_latest(state);
                    history::enforce_live_limit(state);
                    if !msg.mine && !already_read {
                        // Only suppress alerts when this chat is open AND focused.
                        if window_focused {
                            if read_pause::is_paused(state, &peer)
                                || resume_sync::in_progress(state)
                            {
                                bump_unread(state, &peer);
                            } else {
                                mark_chat_read(state, &peer);
                                clear_unread_badge(state, &peer);
                            }
                        } else {
                            bump_unread(state, &peer);
                            notify_incoming(state, &msg, &peer);
                        }
                    }
                } else if !msg.mine && !already_read {
                    let n = state.pending_new_below.borrow().saturating_add(1);
                    *state.pending_new_below.borrow_mut() = n;
                    update_jump_banner(state);
                    bump_unread(state, &peer);
                    notify_incoming(state, &msg, &peer);
                }
            } else if !msg.mine && !already_read {
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
        ProtocolEvent::ContactBlocks {
            account_mid,
            contacts,
        } => contact_blocks::apply(state, &account_mid, contacts),
        ProtocolEvent::ChatUnread {
            chat_mid,
            account_mid,
            through_message_id,
            unread,
        } => {
            unread_sync::apply(state, &account_mid, &chat_mid, &through_message_id, unread);
        }
        ProtocolEvent::ReadReceipt {
            chat_mid,
            user_mid,
            message_id,
        } => {
            let self_mid = state.self_mid.borrow().clone().unwrap_or_default();
            if state
                .peer_reads
                .borrow_mut()
                .observe(&chat_mid, &user_mid, &message_id, &self_mid)
                && state.current_chat.borrow().as_deref() == Some(chat_mid.as_str())
            {
                refresh_read_labels(state);
            }
        }
        ProtocolEvent::ReadRanges { chat_mid, readers } => {
            let self_mid = state.self_mid.borrow().clone().unwrap_or_default();
            if state
                .peer_reads
                .borrow_mut()
                .apply_snapshot(&chat_mid, &readers, &self_mid)
                && state.current_chat.borrow().as_deref() == Some(chat_mid.as_str())
            {
                refresh_read_labels(state);
            }
        }
        ProtocolEvent::Chats { chats, cached } => {
            apply_chats(state, chats, cached);
        }
        ProtocolEvent::Messages { chat_mid, messages } => {
            if state.current_chat.borrow().as_deref() == Some(chat_mid.as_str())
                && !resume_sync::stale_history_event(state, &chat_mid)
                && !history::is_detached(state)
            {
                apply_messages(state, messages);
                let refreshing = state.pending.borrow().values().any(|p| {
                    matches!(p, Pending::FetchMessages { chat_mid: mid, .. } if *mid == chat_mid)
                });
                set_syncing(state, refreshing);
            }
        }
        ProtocolEvent::StickerImportProgress {
            request_id,
            account_mid,
            done,
            total,
        } => {
            sticker_library::progress(state, request_id, &account_mid, done, total);
        }
        ProtocolEvent::SharedMediaChanged {
            account_mid,
            chat_mid,
        } => {
            if state.self_mid.borrow().as_deref() == Some(account_mid.as_str()) {
                shared_media::changed(state, &chat_mid);
            }
        }
        ProtocolEvent::SenderProfile { mid, name } => message_senders::apply(state, mid, name),
        ProtocolEvent::EmojiReady { key, path } => emojis::ready(state, key, path),
        ProtocolEvent::AvatarReady { mid, avatar_path } => {
            mentions::avatar_ready(state, &mid, &avatar_path);
            user_profile::avatar_ready(&mid, &avatar_path);
            user_profile::header_avatar_ready(state, &mid, &avatar_path);
            if state.self_mid.borrow().as_deref() == Some(mid.as_str()) {
                *state.self_avatar_path.borrow_mut() = Some(avatar_path.clone());
                if std::path::Path::new(&avatar_path).exists() {
                    attach_texture_async(state.profile_avatar.clone(), avatar_path.clone(), 64);
                }
                sync_discord_rpc(state);
            }
            if let Some(img) = state.chat_avatars.borrow().get(&mid).cloned()
                && std::path::Path::new(&avatar_path).exists()
            {
                attach_texture_async(img, avatar_path.clone(), 72);
            }
            if let Some(ui) = state.friends_ui.borrow().as_ref() {
                if let Some(img) = ui.avatars.borrow().get(&mid).cloned()
                    && std::path::Path::new(&avatar_path).exists()
                {
                    attach_texture_async(img, avatar_path.clone(), 80);
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
            contact_only,
        } => {
            let preview = localize_preview(&preview);
            if let Some(label) = state.chat_previews.borrow().get(&mid) {
                label.set_text(&preview);
            }
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                chat.preview = preview;
                if let Some(contact_only) = contact_only {
                    chat.contact_only = Some(contact_only);
                }
                if last_activity > chat.last_activity {
                    chat.last_activity = last_activity;
                }
            }
            chat_filter::refresh(state);
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
            note_media(
                state,
                &message_id,
                Some(image_path.as_str()),
                audio_path.as_deref(),
                file_path.as_deref(),
            );
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
                        chat_filter::refresh(state);
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
                            if has_message_content(state) {
                                set_syncing(state, true);
                            } else {
                                set_msg_state(state, "loading", None);
                            }
                        }
                        "ready" => {
                            if has_message_content(state) {
                                set_msg_state(state, "list", None);
                            }
                        }
                        "empty" => {
                            // Don't clobber a thread that already has rows (e.g. optimistic send).
                            if !has_message_content(state) {
                                set_msg_state(state, "empty", Some(&crate::i18n::t("no_messages")));
                                state.chat_subtitle.set_text(&crate::i18n::t("empty"));
                            }
                        }
                        "error" => {
                            let msg = error.unwrap_or_else(|| "Failed to load messages".into());
                            set_syncing(state, false);
                            if !has_message_content(state) {
                                set_msg_state(state, "empty", Some(&msg));
                            }
                            toast(state, &msg);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        ProtocolEvent::DownloadProgress {
            message_id,
            progress,
        } => show_message_download_percent(state, &message_id, progress),
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
            if let Some(Pending::StickerLibrary(request)) = pending {
                sticker_library::response(state, &request, ok, &result, error.as_deref());
                return;
            }
            if let Some(Pending::MentionMembers {
                account,
                chat_mid,
                generation,
            }) = pending
            {
                mentions::response(state, &account, &chat_mid, generation, ok, &result);
                return;
            }
            if let Some(Pending::Unsend {
                account,
                chat_mid,
                message_id,
            }) = pending
            {
                unsend::response(
                    state,
                    &account,
                    &chat_mid,
                    &message_id,
                    ok && result.get("unsent").and_then(|v| v.as_bool()) == Some(true),
                );
                return;
            }
            if let Some(Pending::PollCreate { ui }) = pending {
                poll_create::response(state, &ui, ok, error.as_deref());
                return;
            }
            if let Some(Pending::Poll { card, vote }) = pending {
                line_polls::response(state, &card, vote, ok, &result, error.as_deref());
                return;
            }
            if let Some(Pending::UserProfile {
                account,
                mid,
                generation,
            }) = pending
            {
                user_profile::response(state, &account, &mid, generation, ok, &result);
                return;
            }
            if let Some(Pending::React { account, .. }) = pending {
                reactions::response(state, &account, ok, error.as_deref());
                return;
            }
            if let Some(Pending::ReactionEmojis { account }) = pending {
                reactions::recent_response(state, &account, ok, &result);
                return;
            }
            if let Some(Pending::ReactionPacks { account }) = pending {
                reactions::packs_response(state, &account, ok, &result);
                return;
            }
            if let Some(Pending::ReactionPack { account, product }) = pending {
                reactions::pack_response(state, &account, &product, ok, &result);
                return;
            }
            if let Some(Pending::ContactBlocks { account, change }) = pending {
                contact_blocks::response(state, &account, change, ok);
                return;
            }
            if let Some(Pending::MarkReadOnce { account, chat_mid }) = pending {
                read_pause::once_response(state, &account, &chat_mid, ok, &result);
                return;
            }
            if let Some(Pending::Album {
                ui,
                generation,
                request,
            }) = pending
            {
                if let Some(ui) = ui.upgrade() {
                    albums::response(
                        state,
                        &ui,
                        generation,
                        request,
                        ok,
                        &result,
                        error.as_deref(),
                    );
                }
                return;
            }
            if let Some(Pending::SharedMedia {
                ui,
                generation,
                request,
            }) = pending
            {
                if let Some(ui) = ui.upgrade() {
                    shared_media::response(
                        state,
                        &ui,
                        generation,
                        request,
                        ok,
                        &result,
                        error.as_deref(),
                    );
                }
                return;
            }
            if let Some(Pending::MessageSearch { ui, generation }) = pending {
                if let Some(ui) = ui.upgrade() {
                    message_search::response(state, &ui, generation, ok, &result);
                }
                return;
            }
            if let Some(context @ Pending::SharedMessageContext { .. }) = pending {
                history::shared_context_response(state, context, ok, &result);
                return;
            }
            if let Some(Pending::ResumeSync {
                generation,
                account,
                chat_mid,
            }) = pending
            {
                resume_sync::response(state, generation, account, chat_mid, ok, result);
                return;
            }
            if let Some(
                older_or_newer @ (Pending::FetchOlder { .. } | Pending::FetchNewer { .. }),
            ) = pending
            {
                history::response(state, id, older_or_newer, ok, &result);
                return;
            }
            if let Some(Pending::CachedMessages {
                chat_mid,
                generation,
            }) = pending
            {
                cache_boot::cached_messages(state, &chat_mid, generation, ok, &result);
                return;
            }
            if let Some(Pending::MarkRead {
                account,
                chat_mid,
                message_id,
            }) = pending.as_ref()
            {
                if ok
                    && result.get("marked").and_then(|value| value.as_bool()) == Some(true)
                    && result.get("chatMid").and_then(|value| value.as_str())
                        == Some(chat_mid.as_str())
                    && result.get("lastMessageId").and_then(|value| value.as_str())
                        == Some(message_id.as_str())
                {
                    read_position::record(state, account, chat_mid, message_id);
                }
                return;
            }
            if let Some(Pending::FetchMessages { generation, .. }) = pending.as_ref()
                && *generation != *state.history_request_generation.borrow()
            {
                cache_boot::end(state);
                return;
            }
            if matches!(pending, Some(Pending::FetchMessages { .. })) {
                set_syncing(state, false);
            }
            if !ok {
                if matches!(pending, Some(Pending::Login | Pending::LoginEmail))
                    && *state.restarting.borrow()
                {
                    return;
                }
                if matches!(pending, Some(Pending::Login | Pending::LoginEmail)) {
                    auto_login::failed(state);
                    let code = error.as_deref().unwrap_or("login_email_failed");
                    let message = if code.starts_with("login_") {
                        crate::i18n::t(code)
                    } else if code.contains("timed out") {
                        crate::i18n::t("login_email_timeout")
                    } else {
                        code.into()
                    };
                    show_login(state, &message);
                    login::show_failure(&state.login, &message);
                    // タイムアウトした通信が後から認証を確定しないようキャンセルする。
                    if code.contains("timed out") {
                        *state.restarting.borrow_mut() = true;
                        if state.sidecar.restart_for_login().is_err() {
                            *state.restarting.borrow_mut() = false;
                        }
                    }
                    toast(state, &message);
                    return;
                }
                if matches!(pending, Some(Pending::ListChats)) {
                    set_side_state(
                        state,
                        "empty",
                        Some(error.as_deref().unwrap_or("Failed to load chats")),
                    );
                }
                if matches!(pending, Some(Pending::FetchMessages { .. })) {
                    cache_boot::end(state);
                }
                if matches!(pending, Some(Pending::FetchMessages { .. }))
                    && !has_message_content(state)
                {
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
                if let Some(Pending::Send {
                    placeholder_id,
                    reply_draft,
                    mention_draft,
                    ..
                }) = pending.as_ref()
                {
                    hide_upload_progress(state);
                    remove_pending_placeholder(state, placeholder_id);
                    if let Some(draft) = reply_draft {
                        replies::restore_failed(state, draft);
                    } else if let Some(draft) = mention_draft {
                        mentions::restore_failed(state, draft);
                    }
                }
                let err = error.as_deref().unwrap_or("request failed");
                if err == "sticker_import_invalid" || err == "sticker_import_unavailable" {
                    toast(state, &crate::i18n::t(err));
                    return;
                }
                if err.contains("sticker_not_owned") || err.contains("USER_NOT_STICKER_OWNER") {
                    toast(state, &crate::i18n::t("sticker_not_owned"));
                } else if err.contains("contact_blocked_send") {
                    toast(state, &crate::i18n::t("contact_blocked_send"));
                } else {
                    toast(state, err);
                }
                return;
            }
            match pending {
                Some(Pending::Login | Pending::LoginEmail) => on_logged_in(state, &result),
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
                Some(Pending::FetchMessages { chat_mid, .. }) => {
                    if state.current_chat.borrow().as_deref() == Some(chat_mid.as_str()) {
                        let messages: Vec<MessageInfo> = result
                            .get("messages")
                            .cloned()
                            .and_then(|v| serde_json::from_value(v).ok())
                            .unwrap_or_default();
                        read_position::loaded(state, &messages);
                        if !same_message_list(state, &chat_mid, &messages) {
                            apply_messages(state, messages);
                        }
                    }
                    cache_boot::end(state);
                }
                Some(Pending::Send {
                    placeholder_id,
                    chat_mid,
                    ..
                }) => {
                    remove_pending_placeholder(state, &placeholder_id);
                    if state.current_chat.borrow().as_deref() != Some(chat_mid.as_str()) {
                        return;
                    }
                    if let Ok(msg) = serde_json::from_value::<MessageInfo>(
                        result.get("message").cloned().unwrap_or_default(),
                    ) {
                        if history::is_detached(state) {
                            history::reload_latest(state);
                            return;
                        }
                        if state.msg_stack.visible_child_name().as_deref() != Some("list") {
                            set_msg_state(state, "list", None);
                        }
                        append_message(state, &msg, true);
                        note_appended(state, &msg);
                        pin_messages_to_latest(state);
                        history::enforce_live_limit(state);
                    }
                }
                Some(
                    Pending::CachedMessages { .. }
                    | Pending::FetchOlder { .. }
                    | Pending::FetchNewer { .. },
                ) => {}
                Some(Pending::DownloadMedia {
                    action,
                    content_type,
                    suggest_name,
                    message_id,
                    ..
                }) => {
                    end_download_progress(state);
                    clear_message_download_percent(state, &message_id);
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
                Some(
                    Pending::MentionMembers { .. }
                    | Pending::StickerLibrary(_)
                    | Pending::Album { .. }
                    | Pending::SharedMedia { .. }
                    | Pending::MessageSearch { .. }
                    | Pending::SharedMessageContext { .. }
                    | Pending::Unsend { .. }
                    | Pending::Poll { .. }
                    | Pending::PollCreate { .. }
                    | Pending::UserProfile { .. }
                    | Pending::React { .. }
                    | Pending::ReactionEmojis { .. }
                    | Pending::ReactionPacks { .. }
                    | Pending::ReactionPack { .. }
                    | Pending::ResumeSync { .. }
                    | Pending::MarkRead { .. }
                    | Pending::MarkReadOnce { .. }
                    | Pending::ContactBlocks { .. },
                )
                | None => {}
            }
        }
        ProtocolEvent::Error(e) => toast(state, &e),
        ProtocolEvent::Exited(code) => {
            resume_sync::cancel(state);
            albums::close(state);
            shared_media::close(state);
            sticker_library::close(state);
            hidden_chats::close(state);
            user_profile::close();
            user_profile::update_header(state, None);
            if !*state.restarting.borrow() {
                tracing::warn!(code, "protocol engine exited unexpectedly");
                schedule_sidecar_recovery(state);
            }
        }
    }
}

const MAX_SIDECAR_RECOVERY_ATTEMPTS: u8 = 3;

pub(super) fn schedule_sidecar_recovery(state: &AppState) {
    if *state.recovery_scheduled.borrow() {
        return;
    }
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
    *state.recovery_scheduled.borrow_mut() = true;
    state.status.set_text(&crate::i18n::tf(
        "protocol_reconnecting",
        &[("attempt", &attempt.to_string())],
    ));
    let s = state.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(delay), move || {
        if *s.recovery_scheduled.borrow() && *s.recovery_attempts.borrow() == attempt {
            recover_sidecar(&s);
        }
    });
}

pub(super) fn recover_sidecar(state: &AppState) {
    *state.recovery_scheduled.borrow_mut() = false;
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
    let saved_view = cache_boot::active(state);
    chat_snapshots::clear();
    contact_blocks::reset(state);
    user_profile::close();
    user_profile::update_header(state, None);
    *state.unread_sync.borrow_mut() = unread_sync::UnreadSync::default();
    state.notif_pending.borrow_mut().clear();
    mentions::reset(state);
    resume_sync::cancel(state);
    state.sender_names.borrow_mut().clear();
    albums::close(state);
    shared_media::close(state);
    sticker_library::close(state);
    hidden_chats::close(state);
    match serde_json::from_value::<Profile>(result.clone()) {
        Ok(profile) => {
            cache_boot::verify_account(state, &profile.mid);
            auto_login::logged_in(state);
            if state.self_mid.borrow().as_deref() != Some(profile.mid.as_str()) {
                state.peer_reads.borrow_mut().clear();
            }
            state.login.password_entry.set_text("");
            login::set_busy(&state.login, false);
            *state.login.error.borrow_mut() = None;
            *state.session_device.borrow_mut() = profile.device.clone();
            refresh_call_controls(state);
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
            read_position::refresh(state);
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
            // Saved chats stay on screen while the latest ones load.
            cache_boot::logged_in(state);
            let keep_view = saved_view && !state.chats.borrow().is_empty();
            if keep_view {
                state.side_spinner.set_spinning(true);
                state.side_spinner.set_visible(true);
            } else {
                set_side_state(state, "loading", None);
            }
            if state.current_chat.borrow().is_none() {
                set_msg_state(state, "idle", None);
            }
            contact_blocks::request_refresh(state);
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
    read_position::reset(state);
    cache_boot::abort(state);
    state.shared_media_btn.set_sensitive(false);
    state.poll_create_btn.set_sensitive(false);
    poll_create::close();
    chat_snapshots::clear();
    *state.unread_sync.borrow_mut() = unread_sync::UnreadSync::default();
    state.notif_pending.borrow_mut().clear();
    mentions::reset(state);
    resume_sync::cancel(state);
    state.sender_names.borrow_mut().clear();
    albums::close(state);
    shared_media::close(state);
    sticker_library::close(state);
    hidden_chats::close(state);
    state.stack.set_visible_child_name("login");
    login::show_method_stage(&state.login);
    state.login.hint.set_text(hint);
    sync_discord_rpc(state);
}
