//! StatusNotifierItem tray (ksni) for living in the system tray.
//! GTK3-free so it can coexist with GTK4.

use async_channel::Sender;
use image::GenericImageView;
use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub enum TrayAction {
    Show,
    Quit,
    OpenChat { mid: String },
    /// `secs == 0` clears the timed mute. Otherwise mute notifications for that many seconds.
    MuteFor { secs: u64 },
    /// Flip Discord Rich Presence on/off (persisted in settings).
    ToggleDiscordRpc,
}

#[derive(Debug, Clone)]
pub struct TrayChatItem {
    pub mid: String,
    pub name: String,
    pub unread: i64,
}

pub struct TrayController {
    handle: ksni::blocking::Handle<LineTray>,
}

struct LineTray {
    tx: Sender<TrayAction>,
    recent: Vec<TrayChatItem>,
    /// Unix epoch seconds; 0 means not muted.
    muted_until_epoch: i64,
    discord_rpc: bool,
}

fn tray_icon() -> ksni::Icon {
    static ICON: OnceLock<ksni::Icon> = OnceLock::new();
    ICON.get_or_init(|| {
        let img = image::load_from_memory_with_format(
            include_bytes!("../assets/icons/hicolor/64x64/apps/line-gtk.png"),
            image::ImageFormat::Png,
        )
        .expect("bundled tray icon");
        let (width, height) = img.dimensions();
        let mut data = img.into_rgba8().into_vec();
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1); // rgba -> argb
        }
        ksni::Icon {
            width: width as i32,
            height: height as i32,
            data,
        }
    })
    .clone()
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn menu_label(s: &str) -> String {
    // StatusNotifierItem treats `_` as accelerator markers.
    s.replace('_', "__")
}

fn truncate_name(name: &str, max_chars: usize) -> String {
    let count = name.chars().count();
    if count <= max_chars {
        return name.to_string();
    }
    let mut out: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

impl ksni::Tray for LineTray {
    fn id(&self) -> String {
        "dev.linegtk.LineGtk".into()
    }

    fn title(&self) -> String {
        let muted = self.muted_until_epoch > now_epoch();
        if muted {
            "LINE GTK (muted)".into()
        } else {
            "LINE GTK".into()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon()]
    }

    fn icon_name(&self) -> String {
        "line-gtk".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Communications
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(TrayAction::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut items: Vec<ksni::MenuItem<Self>> = Vec::new();

        let tx_show = self.tx.clone();
        items.push(
            StandardItem {
                label: "Show window".into(),
                icon_name: "view-restore-symbolic".into(),
                activate: Box::new(move |_| {
                    let _ = tx_show.send_blocking(TrayAction::Show);
                }),
                ..Default::default()
            }
            .into(),
        );

        items.push(ksni::MenuItem::Separator);

        // Recent chats (click opens chat + focuses window).
        let mut chat_items: Vec<ksni::MenuItem<Self>> = Vec::new();
        if self.recent.is_empty() {
            chat_items.push(
                StandardItem {
                    label: "No recent chats".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        } else {
            for chat in &self.recent {
                let mid = chat.mid.clone();
                let tx = self.tx.clone();
                let mut label = truncate_name(&chat.name, 28);
                if chat.unread > 0 {
                    label = format!("{label} ({})", chat.unread);
                }
                chat_items.push(
                    StandardItem {
                        label: menu_label(&label),
                        icon_name: if chat.unread > 0 {
                            "mail-unread-symbolic".into()
                        } else {
                            "avatar-default-symbolic".into()
                        },
                        activate: Box::new(move |_| {
                            let _ = tx.send_blocking(TrayAction::OpenChat { mid: mid.clone() });
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
        }
        items.push(
            SubMenu {
                label: "Recent chats".into(),
                icon_name: "user-available-symbolic".into(),
                submenu: chat_items,
                ..Default::default()
            }
            .into(),
        );

        // Global notification mute.
        let muted = self.muted_until_epoch > now_epoch();
        let mute_label = if muted {
            let left = (self.muted_until_epoch - now_epoch()).max(0);
            let mins = (left + 59) / 60;
            if mins >= 60 {
                format!("Mute notifications ({}h left)", (mins + 59) / 60)
            } else {
                format!("Mute notifications ({mins}m left)")
            }
        } else {
            "Mute notifications".into()
        };

        let mute_opts: &[(u64, &str)] = &[
            (15 * 60, "15 minutes"),
            (60 * 60, "1 hour"),
            (3 * 60 * 60, "3 hours"),
            (8 * 60 * 60, "8 hours"),
            (24 * 60 * 60, "24 hours"),
        ];
        let mut mute_items: Vec<ksni::MenuItem<Self>> = Vec::new();
        if muted {
            let tx = self.tx.clone();
            mute_items.push(
                StandardItem {
                    label: "Unmute now".into(),
                    icon_name: "audio-volume-high-symbolic".into(),
                    activate: Box::new(move |_| {
                        let _ = tx.send_blocking(TrayAction::MuteFor { secs: 0 });
                    }),
                    ..Default::default()
                }
                .into(),
            );
            mute_items.push(ksni::MenuItem::Separator);
        }
        for &(secs, label) in mute_opts {
            let tx = self.tx.clone();
            mute_items.push(
                StandardItem {
                    label: format!("Mute for {label}"),
                    icon_name: "audio-volume-muted-symbolic".into(),
                    activate: Box::new(move |_| {
                        let _ = tx.send_blocking(TrayAction::MuteFor { secs });
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(
            SubMenu {
                label: mute_label,
                icon_name: if muted {
                    "audio-volume-muted-symbolic".into()
                } else {
                    "audio-volume-high-symbolic".into()
                },
                submenu: mute_items,
                ..Default::default()
            }
            .into(),
        );

        let tx_rpc = self.tx.clone();
        items.push(
            CheckmarkItem {
                label: "Discord Rich Presence".into(),
                icon_name: "network-workgroup-symbolic".into(),
                checked: self.discord_rpc,
                activate: Box::new(move |tray: &mut LineTray| {
                    tray.discord_rpc = !tray.discord_rpc;
                    let _ = tx_rpc.send_blocking(TrayAction::ToggleDiscordRpc);
                }),
                ..Default::default()
            }
            .into(),
        );

        items.push(ksni::MenuItem::Separator);

        let tx_quit = self.tx.clone();
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(move |_| {
                    let _ = tx_quit.send_blocking(TrayAction::Quit);
                }),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

impl TrayController {
    pub fn spawn(tx: Sender<TrayAction>) -> Option<Self> {
        let tray = LineTray {
            tx,
            recent: Vec::new(),
            muted_until_epoch: 0,
            discord_rpc: true,
        };
        match tray.spawn() {
            Ok(handle) => Some(Self { handle }),
            Err(e) => {
                eprintln!("tray unavailable: {e}");
                None
            }
        }
    }

    pub fn set_state(&self, recent: Vec<TrayChatItem>, muted_until_epoch: i64, discord_rpc: bool) {
        let _ = self.handle.update(|tray| {
            tray.recent = recent;
            tray.muted_until_epoch = muted_until_epoch;
            tray.discord_rpc = discord_rpc;
        });
    }

    pub fn shutdown(self) {
        self.handle.shutdown().wait();
    }
}
