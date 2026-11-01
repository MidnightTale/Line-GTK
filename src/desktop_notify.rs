//! デスクトップ通知。スタンプの画像表示と、同じ通知の更新を扱う。

use crate::tray::TrayAction;
use notify_rust::{Hint, Notification, Timeout};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone)]
pub struct NotificationTicket {
    id: u32,
    closed: Arc<AtomicBool>,
}

impl NotificationTicket {
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

/// 送信者アイコンと本文画像を持つ通知。クリックでトークを開く。
pub struct ChatNotification<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub avatar_path: Option<&'a str>,
    pub image_path: Option<&'a str>,
    pub chat_mid: &'a str,
    pub suppress_sound: bool,
    pub sticker: bool,
    pub replace: Option<&'a NotificationTicket>,
}

fn notification_body(
    body: &str,
    image: Option<&str>,
    sticker: bool,
    capabilities: &[String],
) -> String {
    if !capabilities.iter().any(|cap| cap == "body-markup") {
        return body.to_string();
    }
    let escaped = glib::markup_escape_text(body);
    if sticker
        && capabilities.iter().any(|cap| cap == "body-images")
        && let Some(uri) = image.and_then(|path| glib::filename_to_uri(path, None).ok())
    {
        return format!(
            "<img src=\"{}\" alt=\"{}\"/>",
            glib::markup_escape_text(&uri),
            escaped
        );
    }
    escaped.to_string()
}

/// 通知本文向けに縦100・横200ピクセル以内の静止画を作る。
pub fn sticker_preview(path: &Path, cache: &Path) -> Option<PathBuf> {
    let metadata = path.metadata().ok()?;
    if metadata.len() > 8 * 1024 * 1024 {
        return None;
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    metadata.len().hash(&mut hash);
    metadata.modified().ok()?.hash(&mut hash);
    let dest = cache.join(format!("notification-{:016x}.png", hash.finish()));
    if dest.is_file() {
        return Some(dest);
    }
    let mut reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(200, 100);
    crate::config::ensure_private_dir(cache).ok()?;
    image.save(&dest).ok()?;
    Some(dest)
}

pub fn show_chat_notification(
    request: &ChatNotification<'_>,
    tx: async_channel::Sender<TrayAction>,
) -> Result<NotificationTicket, String> {
    let avatar = request
        .avatar_path
        .filter(|p| !p.is_empty() && Path::new(p).exists());
    let image = request
        .image_path
        .filter(|p| !p.is_empty() && Path::new(p).exists());

    let capabilities = notify_rust::get_capabilities().unwrap_or_default();
    let body = notification_body(request.body, image, request.sticker, &capabilities);
    let mut n = Notification::new();
    n.summary(request.title)
        .body(&body)
        .appname("LINE GTK")
        .id(request.replace.map(|ticket| ticket.id).unwrap_or(0))
        .timeout(Timeout::Milliseconds(12_000))
        .hint(Hint::DesktopEntry("dev.linegtk.LineGtk".into()))
        .hint(Hint::Category("im.received".into()))
        .hint(Hint::Transient(false))
        .action("default", "Open");

    if request.suppress_sound {
        n.hint(Hint::SuppressSound(true));
    }

    let icon = if request.sticker && !capabilities.iter().any(|cap| cap == "body-images") {
        image.or(avatar)
    } else {
        avatar
    };
    if let Some(path) = icon {
        n.icon(path);
    } else {
        n.icon("line-gtk");
    }

    if let Some(path) = image
        && let Ok(uri) = glib::filename_to_uri(path, None)
    {
        n.image_path(&uri);
    }

    let handle = n.show().map_err(|e| e.to_string())?;
    if let Some(ticket) = request.replace {
        return Ok(ticket.clone());
    }
    let ticket = NotificationTicket {
        id: handle.id(),
        closed: Arc::new(AtomicBool::new(false)),
    };
    let closed = ticket.closed.clone();
    let mid = request.chat_mid.to_string();
    if let Err(error) = std::thread::Builder::new()
        .name("line-gtk-notify-action".into())
        .spawn(move || {
            handle.wait_for_action(|action| {
                closed.store(true, Ordering::Release);
                if action == "default" {
                    let _ = tx.send_blocking(TrayAction::OpenChat { mid });
                }
            });
        })
    {
        tracing::warn!(%error, "通知のクリック監視を開始できませんでした");
    }
    Ok(ticket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticker_markup_uses_a_local_uri_and_escapes_alternative_text() {
        let caps = vec!["body-markup".into(), "body-images".into()];
        let body = notification_body("スタンプ<&\"", Some("/tmp/絵 空白.png"), true, &caps);
        assert!(body.starts_with("<img src=\"file:///tmp/"));
        assert!(body.contains("%20"));
        assert!(body.contains("alt=\"スタンプ&lt;&amp;&quot;\""));
        assert!(!body.contains(" width="));
    }

    #[test]
    fn unsupported_images_keep_text_and_user_text_cannot_add_markup() {
        let text = "<img src=\"https://example.invalid/x\"/> & 本文";
        assert_eq!(
            notification_body(text, Some("/tmp/sticker.png"), true, &[]),
            text
        );
        let markup = vec!["body-markup".into()];
        assert_eq!(
            notification_body("スタンプ", Some("/tmp/sticker.png"), true, &markup),
            "スタンプ"
        );
        let caps = vec!["body-markup".into(), "body-images".into()];
        let body = notification_body(text, Some("/tmp/photo.png"), false, &caps);
        assert!(!body.contains("<img"));
        assert!(body.starts_with("&lt;img"));
        assert_eq!(notification_body("スタンプ", None, true, &caps), "スタンプ");
    }

    #[test]
    fn preview_is_small_transparent_cached_and_rejects_invalid_images() {
        let temp = std::env::temp_dir().join(format!(
            "line-notify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&temp).unwrap();
        let source = temp.join("スタンプ.png");
        image::RgbaImage::from_pixel(400, 300, image::Rgba([20, 80, 120, 0]))
            .save(&source)
            .unwrap();
        let cache = temp.join("cache");
        let path = sticker_preview(&source, &cache).unwrap();
        let preview = image::open(&path).unwrap().into_rgba8();
        assert!(preview.width() <= 200 && preview.height() <= 100);
        assert_eq!(preview.get_pixel(0, 0)[3], 0);
        assert_eq!(sticker_preview(&source, &cache), Some(path));
        std::fs::write(&source, "画像ではない").unwrap();
        assert!(sticker_preview(&source, &cache).is_none());
        std::fs::remove_dir_all(temp).unwrap();
    }
}
