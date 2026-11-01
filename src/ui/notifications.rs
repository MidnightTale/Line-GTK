use super::{AppState, PendingNotif, message_senders, notifications_allowed, preview_body_ui};
use crate::desktop_notify::{ChatNotification, NotificationTicket};
use crate::protocol::MessageInfo;
use gtk::gio;
use gtk::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const NOTIFICATION_WINDOW: Duration = Duration::from_secs(12);

pub(super) fn notify_incoming(state: &AppState, msg: &MessageInfo, peer_mid: &str) {
    let viewing =
        state.current_chat.borrow().as_deref() == Some(peer_mid) && state.window.is_active();
    if viewing || !notifications_allowed(state) || chat_muted(state, peer_mid) {
        return;
    }
    if !msg.id.is_empty() && state.notif_pending.borrow().contains_key(&msg.id) {
        return;
    }
    let (name, avatar_path) = state
        .chats
        .borrow()
        .iter()
        .find(|c| c.mid == peer_mid)
        .map(|c| (c.name.clone(), c.avatar_path.clone()))
        .unwrap_or_else(|| (peer_mid.to_string(), None));
    let sticker = msg.content_type.eq_ignore_ascii_case("STICKER");
    let image_path = msg
        .image_path
        .as_deref()
        .filter(|p| !p.is_empty() && Path::new(p).is_file())
        .map(str::to_string)
        .or_else(|| state.media_ready_paths.borrow().get(&msg.id).cloned());
    // Keep one visible bubble per conversation while the previous one is alive.
    // The pending entry also lets delayed sender/image updates target that bubble.
    let account = state.self_mid.borrow().clone();
    let pending = state.notif_pending.borrow();
    let previous = pending
        .values()
        .filter(|meta| {
            meta.account == account
                && meta.chat_mid == peer_mid
                && meta.notified
                && meta.created.elapsed() < NOTIFICATION_WINDOW
                && meta
                    .ticket
                    .as_ref()
                    .is_none_or(|ticket| !ticket.is_closed())
        })
        .max_by_key(|meta| meta.created);
    let coalesced = previous.is_some();
    let replace = previous
        .and_then(|meta| meta.ticket.as_ref())
        .filter(|ticket| !ticket.is_closed())
        .cloned();
    drop(pending);
    let meta = PendingNotif {
        mentioned: super::mentions::mentions_account(msg, state.self_mid.borrow().as_deref()),
        chat_mid: peer_mid.to_string(),
        sender_mid: message_senders::is_group_chat(state, peer_mid).then(|| msg.from.clone()),
        displayed_sender: None,
        image_path,
        title: name,
        body: preview_body_ui(msg),
        avatar_path,
        sticker,
        ticket: replace,
        coalesced,
        notified: false,
        created: Instant::now(),
        account: state.self_mid.borrow().clone(),
    };
    if !msg.id.is_empty() {
        let mut pending = state.notif_pending.borrow_mut();
        pending.retain(|_, meta| meta.created.elapsed() < Duration::from_secs(30));
        if pending.len() >= 64
            && let Some(oldest) = pending
                .iter()
                .min_by_key(|(_, meta)| meta.created)
                .map(|(id, _)| id.clone())
        {
            pending.remove(&oldest);
        }
        pending.insert(msg.id.clone(), meta.clone());
    }
    if wait_for_details(state, &meta) && !msg.id.is_empty() {
        // 名前とスタンプ画像を短時間待ち、同じ通知の更新で補完する。
        let s = state.clone();
        let id = msg.id.clone();
        glib::timeout_add_local_once(Duration::from_millis(1200), move || {
            if let Some(meta) = pending_for_delivery(&s, &id)
                && !meta.notified
            {
                deliver(&s, &id, meta);
            }
        });
    } else {
        deliver(state, &msg.id, meta);
    }
    if !coalesced && state.config.borrow().notification_sound {
        play_notification_sound(state);
    }
}

fn chat_muted(state: &AppState, mid: &str) -> bool {
    state
        .chats
        .borrow()
        .iter()
        .find(|c| c.mid == mid)
        .is_some_and(|c| c.muted)
}

fn pending_for_delivery(state: &AppState, id: &str) -> Option<PendingNotif> {
    let meta = state.notif_pending.borrow().get(id).cloned()?;
    // A slower media/name lookup for an older message must not overwrite the
    // newest message in the conversation's notification.
    let superseded = is_superseded(&state.notif_pending.borrow(), id, &meta);
    let viewing = state.current_chat.borrow().as_deref() == Some(meta.chat_mid.as_str())
        && state.window.is_active();
    if superseded {
        return None;
    }
    if !notifications_allowed(state)
        || chat_muted(state, &meta.chat_mid)
        || viewing
        || !*state.session_ready.borrow()
        || meta.account != *state.self_mid.borrow()
        || meta.created.elapsed() >= Duration::from_secs(30)
        || (meta.notified
            && meta
                .ticket
                .as_ref()
                .is_some_and(NotificationTicket::is_closed))
    {
        state.notif_pending.borrow_mut().remove(id);
        return None;
    }
    Some(meta)
}

fn is_superseded(pending: &HashMap<String, PendingNotif>, id: &str, meta: &PendingNotif) -> bool {
    pending.iter().any(|(other_id, other)| {
        other_id != id
            && other.account == meta.account
            && other.chat_mid == meta.chat_mid
            && other.created > meta.created
    })
}

fn wait_for_details(state: &AppState, meta: &PendingNotif) -> bool {
    meta.created.elapsed() < Duration::from_millis(1200)
        && ((meta.sticker && meta.image_path.is_none())
            || meta.sender_mid.as_deref().is_some_and(|mid| {
                !mid.is_empty() && message_senders::known_name(state, mid).is_none()
            }))
}

pub(super) fn refresh_notification_media(state: &AppState, message_id: &str, image_path: &str) {
    if !Path::new(image_path).is_file() {
        return;
    }
    if let Some(meta) = state.notif_pending.borrow_mut().get_mut(message_id) {
        meta.image_path = Some(image_path.to_string());
    }
    refresh_pending(state, message_id);
}

pub(super) fn refresh_notification_sender(state: &AppState, mid: &str) {
    let Some(name) = message_senders::known_name(state, mid) else {
        return;
    };
    let ids: Vec<String> = state
        .notif_pending
        .borrow()
        .iter()
        .filter(|(_, meta)| {
            meta.sender_mid.as_deref() == Some(mid)
                && meta.displayed_sender.as_deref() != Some(name.as_str())
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        refresh_pending(state, &id);
    }
}

fn refresh_pending(state: &AppState, message_id: &str) {
    let Some(meta) = pending_for_delivery(state, message_id) else {
        return;
    };
    if !meta.notified && wait_for_details(state, &meta) {
        return;
    }
    deliver(state, message_id, meta);
}

fn deliver(state: &AppState, message_id: &str, meta: PendingNotif) {
    let preview = if meta.sticker {
        meta.image_path.as_deref().and_then(|path| {
            crate::desktop_notify::sticker_preview(
                Path::new(path),
                &state.data_dir.join("cache/stickers"),
            )
        })
    } else {
        None
    };
    let image_path = if meta.sticker {
        preview.as_ref().and_then(|p| p.to_str())
    } else {
        meta.image_path.as_deref()
    };
    // スタンプの本文が画像だけになる通知サーバーでも、送信者を必ず表示する。
    let sender_name = meta
        .sender_mid
        .as_deref()
        .map(|mid| message_senders::name(state, mid));
    let title = if let Some(sender) = sender_name.as_deref() {
        crate::i18n::tf(
            if meta.mentioned {
                "notification_group_mention"
            } else {
                "notification_group_sender"
            },
            &[("sender", sender), ("group", &meta.title)],
        )
    } else {
        meta.title.clone()
    };
    let ticket = send_desktop_notification(
        state,
        ChatNotification {
            title: &title,
            body: &meta.body,
            avatar_path: meta.avatar_path.as_deref(),
            image_path,
            chat_mid: &meta.chat_mid,
            suppress_sound: meta.notified || meta.coalesced,
            sticker: meta.sticker,
            replace: meta.ticket.as_ref().filter(|ticket| !ticket.is_closed()),
        },
    );
    if let Some(pending) = state.notif_pending.borrow_mut().get_mut(message_id) {
        pending.notified = true;
        pending.displayed_sender = sender_name;
        pending.ticket = ticket;
    }
}

fn send_desktop_notification(
    state: &AppState,
    request: ChatNotification<'_>,
) -> Option<NotificationTicket> {
    match crate::desktop_notify::show_chat_notification(&request, state.tray_tx.clone()) {
        Ok(ticket) => Some(ticket),
        Err(error) => {
            tracing::warn!(%error, "デスクトップ通知をGioで再試行します");
            let notification = gio::Notification::new(request.title);
            notification.set_body(Some(request.body));
            notification.set_priority(gio::NotificationPriority::Normal);
            notification.set_default_action_and_target_value(
                "app.open-chat",
                Some(&request.chat_mid.to_variant()),
            );
            // 本文画像に対応しない経路でも、スタンプ自体をアイコンにする。
            let icon = if request.sticker {
                request.image_path.or(request.avatar_path)
            } else {
                request.avatar_path.or(request.image_path)
            };
            if let Some(path) = icon.filter(|p| Path::new(p).is_file()) {
                notification.set_icon(&gio::FileIcon::new(&gio::File::for_path(path)));
            } else {
                notification.set_icon(&gio::ThemedIcon::new("line-gtk"));
            }
            let account = state.self_mid.borrow();
            let id = format!(
                "line-gtk-{}-{}",
                account.as_deref().unwrap_or("unknown"),
                request.chat_mid
            );
            state.app.send_notification(Some(&id), &notification);
            None
        }
    }
}

fn play_notification_sound(state: &AppState) {
    let volume = state
        .config
        .borrow()
        .notification_sound_volume
        .clamp(0.0, 2.0);
    let _ = play_system_event_sound("message-new-instant", volume)
        || play_system_event_sound("message", volume)
        || play_system_event_sound("bell", volume);
}

fn play_system_event_sound(event: &str, volume: f64) -> bool {
    if std::process::Command::new("canberra-gtk-play")
        .args(["-i", event])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }
    if std::process::Command::new("canberra-gtk-play")
        .args(["--id", event])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }
    let Some(path) = find_xdg_theme_sound(event) else {
        return false;
    };
    play_system_sound_file(&path, volume)
}

fn xdg_data_dirs() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        if !data_home.is_empty() {
            paths.push(PathBuf::from(data_home));
        }
    } else if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".local/share"));
    }
    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        paths.extend(
            data_dirs
                .split(':')
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        );
    }
    if paths.is_empty() {
        paths.push(PathBuf::from("/usr/local/share"));
        paths.push(PathBuf::from("/usr/share"));
    }
    paths
}

fn find_xdg_theme_sound(event: &str) -> Option<PathBuf> {
    const THEMES: &[&str] = &[
        "freedesktop",
        "gnome",
        "oxygen",
        "Yaru",
        "deepin",
        "ubuntu",
        "elementary",
    ];
    const EXTENSIONS: &[&str] = &["oga", "ogg", "wav", "flac", "mp3"];
    for root in xdg_data_dirs() {
        let sounds = root.join("sounds");
        for theme in THEMES {
            for extension in EXTENSIONS {
                for subdirectory in ["stereo", ""] {
                    let path = if subdirectory.is_empty() {
                        sounds.join(theme).join(format!("{event}.{extension}"))
                    } else {
                        sounds
                            .join(theme)
                            .join(subdirectory)
                            .join(format!("{event}.{extension}"))
                    };
                    if path.is_file() {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

fn play_system_sound_file(path: &Path, volume: f64) -> bool {
    let path = path.to_string_lossy().to_string();
    let paplay_volume = ((volume * 65536.0).round() as i32).clamp(0, 130_000);
    if std::process::Command::new("paplay")
        .args(["--volume", &paplay_volume.to_string(), &path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }
    std::process::Command::new("pw-play")
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(chat_mid: &str, account: &str, created: Instant) -> PendingNotif {
        PendingNotif {
            mentioned: false,
            chat_mid: chat_mid.into(),
            sender_mid: None,
            displayed_sender: None,
            image_path: None,
            title: String::new(),
            body: String::new(),
            avatar_path: None,
            sticker: false,
            ticket: None,
            coalesced: false,
            notified: true,
            created,
            account: Some(account.into()),
        }
    }

    #[test]
    fn late_details_cannot_replace_newer_message_or_another_account() {
        let now = Instant::now();
        let old = pending("c-group", "u-account", now);
        let mut by_id = HashMap::new();
        by_id.insert("old".into(), old.clone());
        by_id.insert(
            "new".into(),
            pending("c-group", "u-account", now + Duration::from_millis(1)),
        );
        by_id.insert(
            "other-chat".into(),
            pending("c-other", "u-account", now + Duration::from_secs(1)),
        );
        by_id.insert(
            "other-account".into(),
            pending("c-group", "u-other", now + Duration::from_secs(1)),
        );
        assert!(is_superseded(&by_id, "old", &old));
        assert!(!is_superseded(&by_id, "new", &by_id["new"]));
    }
}
