use gtk::CssProvider;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub theme: String,   // system | dark | light
    pub language: String, // en | th
    pub font_scale: f64, // UI scale 0.60 .. 1.40 (1.0 = 100%)
    /// Any installed font family name, e.g. "Noto Sans Thai", "Inter", "Sarasa Gothic"
    pub font_family: String,
    /// UI motion: page crossfades, message enter, hover easing. Off = instant.
    pub animations: bool,
    /// Cache retention: smart | day | week | month | forever
    pub cache_retention: String,
    pub notifications: bool,
    /// Play the desktop/system notification sound with alerts.
    pub notification_sound: bool,
    /// Notification sound gain for file-based fallbacks (0.0 .. 2.0).
    pub notification_sound_volume: f64,
    pub auto_mark_read: bool,
    /// Last opened chat mid (restored on launch).
    pub last_chat_mid: String,
    /// PulseAudio / PipeWire source name, or empty / "default".
    pub audio_input: String,
    /// PulseAudio / PipeWire sink name, or empty / "default".
    pub audio_output: String,
    /// Call microphone gain (0.0 .. 2.5), 1.0 = unity.
    pub call_mic_volume: f64,
    /// Call speaker gain (0.0 .. 2.5), 1.0 = unity.
    pub call_spk_volume: f64,
    /// Chat list sidebar width in px (remembered across launches).
    pub sidebar_width: i32,
    /// Unlock experimental voice calls (unstable; off by default).
    pub experimental_calls: bool,
    /// Keep a StatusNotifierItem tray icon while running.
    pub tray_enabled: bool,
    /// Close window button hides to tray instead of quitting (requires tray).
    pub close_to_tray: bool,
    /// Show Discord Rich Presence while the app is running.
    pub discord_rpc: bool,
    /// Discord Application (Client) ID. Empty uses the built-in LINE GTK app ID.
    pub discord_rpc_client_id: String,
    /// Include the open chat name in Discord presence (privacy trade-off).
    pub discord_rpc_show_chat: bool,
    /// Show your LINE profile photo as the Discord small image.
    pub discord_rpc_show_avatar: bool,
    /// Show your LINE display name in Discord presence (details / avatar tooltip).
    pub discord_rpc_show_name: bool,
    /// Unix epoch seconds until which desktop notifications are muted (0 = not muted).
    pub notifications_muted_until: i64,
    /// When true, always show a Save dialog. When false, save into the type folder.
    pub download_ask_every_time: bool,
    /// Destination folders (empty = system Downloads). Absolute paths preferred.
    pub download_dir_image: String,
    pub download_dir_video: String,
    pub download_dir_audio: String,
    pub download_dir_file: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            language: "th".into(),
            font_scale: 1.0,
            font_family: "Noto Sans Thai".into(),
            animations: true,
            cache_retention: "smart".into(),
            notifications: true,
            notification_sound: true,
            notification_sound_volume: 1.0,
            auto_mark_read: true,
            last_chat_mid: String::new(),
            audio_input: String::new(),
            audio_output: String::new(),
            call_mic_volume: 1.0,
            call_spk_volume: 1.0,
            sidebar_width: 320,
            experimental_calls: false,
            tray_enabled: true,
            close_to_tray: true,
            discord_rpc: true,
            discord_rpc_client_id: String::new(),
            discord_rpc_show_chat: false,
            discord_rpc_show_avatar: false,
            discord_rpc_show_name: false,
            notifications_muted_until: 0,
            download_ask_every_time: false,
            download_dir_image: String::new(),
            download_dir_video: String::new(),
            download_dir_audio: String::new(),
            download_dir_file: String::new(),
        }
    }
}

impl AppConfig {
    pub fn path(data_dir: &std::path::Path) -> PathBuf {
        data_dir.join("settings.json")
    }

    pub fn load(data_dir: &std::path::Path) -> Self {
        let path = Self::path(data_dir);
        let mut cfg = match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        cfg.font_scale = cfg.font_scale.clamp(0.60, 1.40);
        cfg
    }

    pub fn save(&self, data_dir: &std::path::Path) {
        let path = Self::path(data_dir);
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, s);
        }
    }

    /// Discord Application ID from settings, env, or the built-in default.
    pub fn discord_app_id(&self) -> String {
        let custom = self.discord_rpc_client_id.trim();
        if !custom.is_empty() {
            return custom.to_string();
        }
        if let Ok(env_id) = std::env::var("LINE_GTK_DISCORD_APP_ID") {
            let env_id = env_id.trim();
            if !env_id.is_empty() {
                return env_id.to_string();
            }
        }
        crate::discord_rpc::DEFAULT_APP_ID.trim().to_string()
    }

    pub fn system_downloads_dir() -> PathBuf {
        dirs::download_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn resolve_download_dir(configured: &str) -> PathBuf {
        let trimmed = configured.trim();
        if trimmed.is_empty() {
            return Self::system_downloads_dir();
        }
        let p = PathBuf::from(trimmed);
        if p.is_absolute() {
            p
        } else if let Some(home) = dirs::home_dir() {
            home.join(trimmed.trim_start_matches("~/").trim_start_matches('/'))
        } else {
            p
        }
    }

    /// Folder used for saving a downloaded media type (IMAGE/VIDEO/AUDIO/FILE…).
    pub fn download_dir_for(&self, content_type: &str) -> PathBuf {
        let configured = match content_type.to_ascii_uppercase().as_str() {
            "IMAGE" | "STICKER" => &self.download_dir_image,
            "VIDEO" => &self.download_dir_video,
            "AUDIO" => &self.download_dir_audio,
            _ => &self.download_dir_file,
        };
        Self::resolve_download_dir(configured)
    }

    pub fn set_download_dir_for(&mut self, content_type: &str, path: String) {
        match content_type.to_ascii_uppercase().as_str() {
            "IMAGE" | "STICKER" => self.download_dir_image = path,
            "VIDEO" => self.download_dir_video = path,
            "AUDIO" => self.download_dir_audio = path,
            _ => self.download_dir_file = path,
        }
    }

    pub fn download_dir_display(configured: &str) -> String {
        if configured.trim().is_empty() {
            Self::system_downloads_dir().display().to_string()
        } else {
            Self::resolve_download_dir(configured).display().to_string()
        }
    }
}

pub fn apply_theme(theme: &str) {
    let mgr = libadwaita::StyleManager::default();
    let scheme = match theme {
        "dark" => libadwaita::ColorScheme::ForceDark,
        "light" => libadwaita::ColorScheme::ForceLight,
        _ => libadwaita::ColorScheme::Default,
    };
    mgr.set_color_scheme(scheme);
}

pub fn apply_font(family: &str, scale: f64) {
    let scale = scale.clamp(0.60, 1.40);
    let family = family.trim();
    let size = 11.0 * scale;
    let css = if family.is_empty() {
        format!(
            ".line-shell, .line-shell label, .line-shell entry {{ font-size: {size}pt; }}"
        )
    } else {
        let escaped = family.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            ".line-shell, .line-shell label, .line-shell entry {{ font-family: \"{escaped}\", sans-serif; font-size: {size}pt; }}"
        )
    };
    FONT_PROVIDER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let provider = CssProvider::new();
            if let Some(display) = gtk::gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 10,
                );
            }
            *slot = Some(provider);
        }
        let provider = slot.as_ref().expect("font provider");
        provider.load_from_data(&css);
    });
}

thread_local! {
    static FONT_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static MOTION_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
}

const MOTION_CSS: &str = r#"
@keyframes line-msg-in {
  from { opacity: 0; margin-bottom: -10px; }
  to   { opacity: 1; margin-bottom: 0; }
}
@keyframes line-chat-bump {
  0%   { background-color: alpha(@accent_bg_color, 0.0); }
  35%  { background-color: alpha(@accent_bg_color, 0.22); }
  100% { background-color: alpha(@accent_bg_color, 0.0); }
}
@keyframes line-badge-pop {
  0%   { opacity: 0.4; }
  100% { opacity: 1; }
}
@keyframes line-fade-in {
  from { opacity: 0; }
  to   { opacity: 1; }
}

.line-anim .line-msg-enter {
  animation: line-msg-in 220ms cubic-bezier(0.2, 0.8, 0.2, 1);
}
.line-anim .line-chat-bump {
  animation: line-chat-bump 320ms ease-out;
}
.line-anim .line-unread {
  animation: line-badge-pop 180ms ease-out;
}
.line-anim .line-chat-list row {
  transition: background-color 140ms ease;
}
.line-anim .line-friend-list row {
  transition: background-color 140ms ease;
}
.line-anim button.circular,
.line-anim button.pill,
.line-anim button.flat {
  transition: opacity 120ms ease, background-color 120ms ease;
}
.line-anim .line-composer-entry {
  transition: box-shadow 160ms ease, background-color 160ms ease;
}
.line-anim .line-jump-banner {
  transition: opacity 160ms ease;
}
.line-anim .line-bubble {
  transition: background-color 120ms ease;
}
.line-anim .line-day-sep {
  animation: line-fade-in 200ms ease-out;
}
"#;

/// Load or clear motion CSS (keyframes / transitions). Widget transitions are applied separately.
pub fn apply_animations(enabled: bool) {
    MOTION_PROVIDER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let provider = CssProvider::new();
            if let Some(display) = gtk::gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 25,
                );
            }
            *slot = Some(provider);
        }
        let provider = slot.as_ref().expect("motion provider");
        if enabled {
            provider.load_from_data(MOTION_CSS);
        } else {
            provider.load_from_data("/* animations disabled */");
        }
    });
}
