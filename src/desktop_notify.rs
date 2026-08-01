//! Desktop notifications (avatar + image preview, click opens chat).

use crate::tray::TrayAction;
use notify_rust::{Hint, Notification, Timeout};
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Stable FreeDesktop replaces-id for a LINE message id.
pub fn notification_id(message_id: &str) -> u32 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    message_id.hash(&mut h);
    (h.finish() as u32).max(1)
}

/// Show (or replace) a chat message notification.
///
/// - `avatar_path`: small icon (sender / chat avatar)
/// - `image_path`: large image-path hint (photo / sticker / video thumb)
/// - Clicking the body sends [`TrayAction::OpenChat`].
pub struct ChatNotification<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub avatar_path: Option<&'a str>,
    pub image_path: Option<&'a str>,
    pub chat_mid: &'a str,
    pub message_id: &'a str,
    pub suppress_sound: bool,
}

pub fn show_chat_notification(
    request: &ChatNotification<'_>,
    tx: async_channel::Sender<TrayAction>,
) -> Result<(), String> {
    let avatar = request
        .avatar_path
        .filter(|p| !p.is_empty() && Path::new(p).exists());
    let image = request
        .image_path
        .filter(|p| !p.is_empty() && Path::new(p).exists());

    let mut n = Notification::new();
    n.summary(request.title)
        .body(request.body)
        .appname("LINE GTK")
        .id(notification_id(request.message_id))
        .timeout(Timeout::Milliseconds(12_000))
        .hint(Hint::DesktopEntry("dev.linegtk.LineGtk".into()))
        .hint(Hint::Category("im.received".into()))
        .hint(Hint::Transient(false))
        .action("default", "Open");

    if request.suppress_sound {
        n.hint(Hint::SuppressSound(true));
    }

    if let Some(path) = avatar {
        n.icon(path);
    } else {
        n.icon("line-gtk");
    }

    if let Some(path) = image {
        n.image_path(path);
    }

    let handle = n.show().map_err(|e| e.to_string())?;
    let mid = request.chat_mid.to_string();
    std::thread::Builder::new()
        .name("line-gtk-notify-action".into())
        .spawn(move || {
            handle.wait_for_action(|action| {
                if action == "default" {
                    let _ = tx.send_blocking(TrayAction::OpenChat { mid });
                }
            });
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
