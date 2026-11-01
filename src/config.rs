use gtk::CssProvider;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

fn default_font_family(language: &str) -> &'static str {
    match language {
        "ja" => "Noto Sans CJK JP",
        "th" => "Noto Sans Thai",
        _ => "Noto Sans",
    }
}

// Missing fields in existing settings must retain the old default until normalization
// can choose a font using the saved language (rather than the new Vietnamese default).
fn legacy_font_family() -> String {
    "Noto Sans Thai".into()
}

pub const DEFAULT_MESSAGE_CACHE_LIMIT: u32 = 2000;
const MAX_MESSAGE_CACHE_LIMIT: u32 = 1_000_000;
/// Choices offered in Settings; 0 means unlimited.
pub const MESSAGE_CACHE_LIMITS: [u32; 4] = [2000, 5000, 10_000, 0];

/// The next preset above `current`, or unlimited once the presets run out.
pub fn next_message_cache_limit(current: u32) -> u32 {
    if current == 0 {
        return 0;
    }
    MESSAGE_CACHE_LIMITS
        .iter()
        .copied()
        .find(|&limit| limit > current)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 起動時に限り、保存セッションが使えなければキーストアの認証情報でログインする。
    pub startup_auto_login: bool,
    pub theme: String,    // system | dark | light
    pub language: String, // vi | en | th | ja
    pub font_scale: f64,  // UI scale 0.60 .. 1.40 (1.0 = 100%)
    /// Any installed font family name, e.g. "Noto Sans", "Inter", "Sarasa Gothic"
    #[serde(default = "legacy_font_family")]
    pub font_family: String,
    /// UI motion: page crossfades, message enter, hover easing. Off = instant.
    pub animations: bool,
    /// Cache retention: smart | day | week | month | forever
    pub cache_retention: String,
    /// Messages kept per chat on disk; 0 = unlimited.
    pub message_cache_limit: u32,
    pub notifications: bool,
    /// Play the desktop/system notification sound with alerts.
    pub notification_sound: bool,
    /// Notification sound gain for file-based fallbacks (0.0 .. 2.0).
    pub notification_sound_volume: f64,
    pub auto_mark_read: bool,
    /// このアプリからの既読通知を停止する。
    pub pause_read_receipts: bool,
    /// アカウントごとに既読通知を止めるトークID。
    pub read_receipts_paused_chats: BTreeMap<String, BTreeSet<String>>,
    /// Last opened chat mid (restored on launch).
    pub last_chat_mid: String,
    /// Last confirmed read message, isolated by account and chat.
    pub last_read_messages: BTreeMap<String, BTreeMap<String, String>>,
    /// アカウントごとの非表示トーク（トークID → 復元画面用の名前）。
    pub hidden_chats: BTreeMap<String, BTreeMap<String, String>>,
    /// このPCで一覧の上部に固定するトーク。アカウントごとに保存する。
    pub pinned_chats: BTreeMap<String, BTreeSet<String>>,
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
    /// Start with the desktop session (XDG autostart entry / Background portal).
    pub autostart: bool,
    /// Autostarted instances stay in the tray instead of opening the window.
    pub autostart_hidden: bool,
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
            startup_auto_login: false,
            theme: "system".into(),
            language: "vi".into(),
            font_scale: 1.0,
            font_family: default_font_family("vi").into(),
            animations: true,
            cache_retention: "smart".into(),
            message_cache_limit: DEFAULT_MESSAGE_CACHE_LIMIT,
            notifications: true,
            notification_sound: true,
            notification_sound_volume: 1.0,
            auto_mark_read: true,
            pause_read_receipts: false,
            read_receipts_paused_chats: BTreeMap::new(),
            last_chat_mid: String::new(),
            last_read_messages: BTreeMap::new(),
            hidden_chats: BTreeMap::new(),
            pinned_chats: BTreeMap::new(),
            audio_input: String::new(),
            audio_output: String::new(),
            call_mic_volume: 1.0,
            call_spk_volume: 1.0,
            sidebar_width: 320,
            experimental_calls: false,
            tray_enabled: true,
            close_to_tray: true,
            autostart: false,
            autostart_hidden: true,
            discord_rpc: false,
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
    pub fn last_read_message(&self, account: &str, chat: &str) -> Option<&str> {
        self.last_read_messages
            .get(account)?
            .get(chat)
            .map(String::as_str)
    }

    pub fn set_last_read_message(
        &mut self,
        data_dir: &Path,
        account: &str,
        chat: &str,
        message: &str,
    ) -> std::io::Result<()> {
        let Some(id) = message.parse::<u128>().ok().filter(|id| *id > 0) else {
            return Ok(());
        };
        if account.is_empty()
            || chat.is_empty()
            || self
                .last_read_message(account, chat)
                .and_then(|previous| previous.parse::<u128>().ok())
                .is_some_and(|previous| previous >= id)
        {
            return Ok(());
        }
        let previous = self.last_read_messages.clone();
        self.last_read_messages
            .entry(account.into())
            .or_default()
            .insert(chat.into(), message.into());
        if let Err(error) = self.save_atomic(data_dir) {
            self.last_read_messages = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn is_chat_pinned(&self, account: &str, mid: &str) -> bool {
        self.pinned_chats
            .get(account)
            .is_some_and(|chats| chats.contains(mid))
    }

    pub fn set_chat_pinned(
        &mut self,
        data_dir: &Path,
        account: &str,
        mid: &str,
        pinned: bool,
    ) -> std::io::Result<()> {
        if account.trim().is_empty() || mid.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "アカウントとトークIDが必要です",
            ));
        }
        let previous = self.pinned_chats.clone();
        if pinned {
            self.pinned_chats
                .entry(account.into())
                .or_default()
                .insert(mid.into());
        } else if let Some(chats) = self.pinned_chats.get_mut(account) {
            chats.remove(mid);
            if chats.is_empty() {
                self.pinned_chats.remove(account);
            }
        }
        if let Err(error) = self.save_atomic(data_dir) {
            self.pinned_chats = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn is_chat_hidden(&self, account: &str, mid: &str) -> bool {
        self.hidden_chats
            .get(account)
            .is_some_and(|chats| chats.contains_key(mid))
    }

    pub fn set_chat_hidden(
        &mut self,
        data_dir: &Path,
        account: &str,
        mid: &str,
        name: Option<&str>,
    ) -> std::io::Result<()> {
        if account.trim().is_empty() || mid.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "アカウントとトークIDが必要です",
            ));
        }
        let previous = self.hidden_chats.clone();
        let last_chat = self.last_chat_mid.clone();
        if let Some(name) = name {
            self.hidden_chats.entry(account.into()).or_default().insert(
                mid.into(),
                if name.trim().is_empty() { mid } else { name }.into(),
            );
            if self.last_chat_mid == mid {
                self.last_chat_mid.clear();
            }
        } else if let Some(chats) = self.hidden_chats.get_mut(account) {
            chats.remove(mid);
            if chats.is_empty() {
                self.hidden_chats.remove(account);
            }
        }
        if let Err(error) = self.save_atomic(data_dir) {
            self.hidden_chats = previous;
            self.last_chat_mid = last_chat;
            return Err(error);
        }
        Ok(())
    }

    pub fn can_mark_read(&self) -> bool {
        self.auto_mark_read && !self.pause_read_receipts
    }

    pub fn chat_read_receipts_paused(&self, account: &str, mid: &str) -> bool {
        self.read_receipts_paused_chats
            .get(account)
            .is_some_and(|chats| chats.contains(mid))
    }

    pub fn can_mark_chat_read(&self, account: &str, mid: &str) -> bool {
        !account.is_empty()
            && !mid.is_empty()
            && self.can_mark_read()
            && !self.chat_read_receipts_paused(account, mid)
    }

    pub fn set_chat_read_receipts_paused(
        &mut self,
        data_dir: &Path,
        account: &str,
        mid: &str,
        paused: bool,
    ) -> std::io::Result<()> {
        if account.trim().is_empty() || mid.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "アカウントとトークIDが必要です",
            ));
        }
        let previous = self.read_receipts_paused_chats.clone();
        if paused {
            self.read_receipts_paused_chats
                .entry(account.into())
                .or_default()
                .insert(mid.into());
        } else if let Some(chats) = self.read_receipts_paused_chats.get_mut(account) {
            chats.remove(mid);
            if chats.is_empty() {
                self.read_receipts_paused_chats.remove(account);
            }
        }
        if let Err(error) = self.save_atomic(data_dir) {
            self.read_receipts_paused_chats = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn set_read_receipts_paused(
        &mut self,
        data_dir: &Path,
        paused: bool,
    ) -> std::io::Result<()> {
        let previous = self.pause_read_receipts;
        self.pause_read_receipts = paused;
        if let Err(error) = self.save_atomic(data_dir) {
            self.pause_read_receipts = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn set_language(&mut self, language: &str) {
        // 標準フォントだけを言語に合わせる。利用者が指定したフォントは維持する。
        let previous_default = default_font_family(&self.language);
        if self.font_family == previous_default {
            self.font_family = default_font_family(language).into();
        }
        self.language = language.into();
    }

    pub fn path(data_dir: &std::path::Path) -> PathBuf {
        data_dir.join("settings.json")
    }

    pub fn load(data_dir: &std::path::Path) -> Self {
        let path = Self::path(data_dir);
        let mut cfg = match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|error| {
                tracing::error!(?path, %error, "invalid settings; using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        };
        cfg.normalize();
        cfg
    }

    pub fn save(&self, data_dir: &std::path::Path) {
        if let Err(error) = self.save_atomic(data_dir) {
            tracing::error!(?data_dir, %error, "failed to save settings");
        }
    }

    fn save_atomic(&self, data_dir: &Path) -> std::io::Result<()> {
        ensure_private_dir(data_dir)?;
        let mut normalized = self.clone();
        normalized.normalize();
        let contents = serde_json::to_vec_pretty(&normalized)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let path = Self::path(data_dir);
        let temporary = data_dir.join(".settings.json.tmp");
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&contents)?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(&temporary, &path)?;
        Ok(())
    }

    fn normalize(&mut self) {
        if !matches!(self.theme.as_str(), "system" | "dark" | "light") {
            self.theme = "system".into();
        }
        if !matches!(self.language.as_str(), "vi" | "en" | "th" | "ja") {
            self.language = "vi".into();
        }
        // Older versions used the Thai default for Vietnamese and English too.
        if self.font_family == legacy_font_family() {
            self.font_family = default_font_family(&self.language).into();
        }
        if !matches!(
            self.cache_retention.as_str(),
            "smart" | "day" | "week" | "month" | "forever"
        ) {
            self.cache_retention = "smart".into();
        }
        if self.message_cache_limit != 0 {
            self.message_cache_limit = self.message_cache_limit.clamp(100, MAX_MESSAGE_CACHE_LIMIT);
        }
        self.font_scale = finite_or(self.font_scale, 1.0).clamp(0.60, 1.40);
        self.notification_sound_volume =
            finite_or(self.notification_sound_volume, 1.0).clamp(0.0, 2.0);
        self.call_mic_volume = finite_or(self.call_mic_volume, 1.0).clamp(0.0, 2.5);
        self.call_spk_volume = finite_or(self.call_spk_volume, 1.0).clamp(0.0, 2.5);
        self.sidebar_width = self.sidebar_width.clamp(80, 520);
        self.notifications_muted_until = self.notifications_muted_until.max(0);
        self.read_receipts_paused_chats.retain(|account, chats| {
            chats.retain(|mid| !mid.trim().is_empty());
            !account.trim().is_empty() && !chats.is_empty()
        });
        self.hidden_chats.retain(|account, chats| {
            chats.retain(|mid, _| !mid.trim().is_empty());
            !account.trim().is_empty() && !chats.is_empty()
        });
        self.pinned_chats.retain(|account, chats| {
            chats.retain(|mid| !mid.trim().is_empty());
            !account.trim().is_empty() && !chats.is_empty()
        });
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

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

pub fn ensure_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn last_read_positions_persist_per_account_and_only_advance() {
        let dir =
            std::env::temp_dir().join(format!("line-gtk-read-position-{}", std::process::id()));
        let mut cfg: super::AppConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.last_read_message("self", "chat").is_none());
        cfg.set_last_read_message(&dir, "self", "chat", "9007199254740993")
            .unwrap();
        cfg.set_last_read_message(&dir, "self", "chat", "99")
            .unwrap();
        cfg.set_last_read_message(&dir, "other", "chat", "100")
            .unwrap();
        cfg.set_last_read_message(&dir, "self", "other-chat", "200")
            .unwrap();
        cfg.set_last_read_message(&dir, "self", "chat", "pending-1")
            .unwrap();
        let loaded = super::AppConfig::load(&dir);
        assert_eq!(
            loaded.last_read_message("self", "chat"),
            Some("9007199254740993")
        );
        assert_eq!(loaded.last_read_message("other", "chat"), Some("100"));
        assert_eq!(loaded.last_read_message("self", "other-chat"), Some("200"));
        let blocked = dir.join("not-a-directory");
        std::fs::write(&blocked, "file").unwrap();
        assert!(
            cfg.set_last_read_message(&blocked, "other", "chat", "101")
                .is_err()
        );
        assert_eq!(cfg.last_read_message("other", "chat"), Some("100"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    use super::*;

    #[test]
    fn japanese_font_follows_language_and_preserves_custom_fonts() {
        let mut cfg = AppConfig::default();
        cfg.set_language("ja");
        assert_eq!(cfg.font_family, "Noto Sans CJK JP");
        cfg.set_language("th");
        assert_eq!(cfg.font_family, "Noto Sans Thai");
        for family in ["Sarasa Gothic J", ""] {
            cfg.font_family = family.into();
            cfg.set_language("ja");
            cfg.normalize();
            assert_eq!(cfg.font_family, family);
        }
        let mut old = AppConfig {
            language: "ja".into(),
            font_family: legacy_font_family(),
            ..AppConfig::default()
        };
        old.normalize();
        assert_eq!(old.font_family, "Noto Sans CJK JP");
    }

    #[test]
    fn vietnamese_font_follows_language_and_migrates_legacy_settings() {
        let mut cfg = AppConfig::default();
        assert_eq!(cfg.font_family, "Noto Sans");
        for (language, family) in [
            ("th", "Noto Sans Thai"),
            ("vi", "Noto Sans"),
            ("ja", "Noto Sans CJK JP"),
            ("en", "Noto Sans"),
        ] {
            cfg.set_language(language);
            assert_eq!(cfg.font_family, family);

            for saved_font in [None, Some("Noto Sans Thai"), Some("Inter"), Some("")] {
                let mut saved = serde_json::json!({ "language": language });
                if let Some(font) = saved_font {
                    saved["font_family"] = font.into();
                }
                let mut restored: AppConfig = serde_json::from_value(saved).unwrap();
                restored.normalize();
                let expected = match saved_font {
                    Some("Inter") => "Inter",
                    Some("") => "",
                    _ => family,
                };
                assert_eq!(restored.font_family, expected, "{language}: {saved_font:?}");
            }
        }
        for font in ["Inter", ""] {
            cfg.font_family = font.into();
            cfg.set_language("vi");
            cfg.normalize();
            assert_eq!(cfg.font_family, font);
        }
    }

    #[test]
    fn normalize_rejects_invalid_values() {
        let mut cfg = AppConfig {
            theme: "purple".into(),
            language: "xx".into(),
            cache_retention: "century".into(),
            font_scale: f64::NAN,
            notification_sound_volume: 99.0,
            call_mic_volume: -2.0,
            sidebar_width: 9_999,
            notifications_muted_until: -1,
            ..AppConfig::default()
        };
        cfg.normalize();
        assert_eq!(cfg.theme, "system");
        assert_eq!(cfg.language, "vi");
        assert_eq!(cfg.cache_retention, "smart");
        assert_eq!(cfg.font_scale, 1.0);
        assert_eq!(cfg.notification_sound_volume, 2.0);
        assert_eq!(cfg.call_mic_volume, 0.0);
        assert_eq!(cfg.sidebar_width, 520);
        assert_eq!(cfg.notifications_muted_until, 0);
    }

    #[test]
    fn message_cache_limit_is_normalized_and_stepped() {
        let mut cfg = AppConfig {
            message_cache_limit: 7,
            ..AppConfig::default()
        };
        cfg.normalize();
        assert_eq!(cfg.message_cache_limit, 100);
        cfg.message_cache_limit = u32::MAX;
        cfg.normalize();
        assert_eq!(cfg.message_cache_limit, 1_000_000);
        cfg.message_cache_limit = 0;
        cfg.normalize();
        assert_eq!(cfg.message_cache_limit, 0);
        assert_eq!(next_message_cache_limit(2000), 5000);
        assert_eq!(next_message_cache_limit(5000), 10_000);
        assert_eq!(next_message_cache_limit(10_000), 0);
        assert_eq!(next_message_cache_limit(3000), 5000);
        assert_eq!(next_message_cache_limit(0), 0);
    }

    #[test]
    fn normalize_preserves_japanese() {
        let mut cfg = AppConfig {
            language: "ja".into(),
            ..AppConfig::default()
        };
        cfg.normalize();
        assert_eq!(cfg.language, "ja");
    }

    #[test]
    fn read_pause_migrates_and_respects_auto_read() {
        let mut cfg: AppConfig = serde_json::from_str(r#"{"auto_mark_read":true}"#).unwrap();
        assert!(!cfg.pause_read_receipts);
        assert!(cfg.can_mark_read());
        cfg.pause_read_receipts = true;
        assert!(!cfg.can_mark_read());
        cfg.pause_read_receipts = false;
        cfg.auto_mark_read = false;
        assert!(!cfg.can_mark_read());
    }

    #[test]
    fn read_pause_persists_and_rolls_back_on_save_failure() {
        let dir = std::env::temp_dir().join(format!("line-gtk-read-pause-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut cfg = AppConfig::default();
        cfg.save_atomic(&dir).unwrap();
        fs::create_dir(dir.join(".settings.json.tmp")).unwrap();
        assert!(cfg.set_read_receipts_paused(&dir, true).is_err());
        assert!(!cfg.pause_read_receipts);
        assert!(!AppConfig::load(&dir).pause_read_receipts);
        fs::remove_dir(dir.join(".settings.json.tmp")).unwrap();
        cfg.set_read_receipts_paused(&dir, true).unwrap();
        assert!(AppConfig::load(&dir).pause_read_receipts);
        fs::create_dir(dir.join(".settings.json.tmp")).unwrap();
        assert!(cfg.set_read_receipts_paused(&dir, false).is_err());
        assert!(cfg.pause_read_receipts);
        assert!(AppConfig::load(&dir).pause_read_receipts);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pinned_chats_persist_per_account_and_failed_saves_roll_back() {
        let dir = std::env::temp_dir().join(format!("line-gtk-pins-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(!cfg.is_chat_pinned("u-self", "u-friend"));
        cfg.last_chat_mid = "u-open".into();
        cfg.set_chat_pinned(&dir, "u-self", "u-friend", true)
            .unwrap();
        let mut loaded = AppConfig::load(&dir);
        assert!(loaded.is_chat_pinned("u-self", "u-friend"));
        assert!(!loaded.is_chat_pinned("u-other", "u-friend"));
        assert_eq!(loaded.last_chat_mid, "u-open");
        assert!(loaded.can_mark_read());
        fs::create_dir(dir.join(".settings.json.tmp")).unwrap();
        assert!(
            loaded
                .set_chat_pinned(&dir, "u-self", "u-friend", false)
                .is_err()
        );
        assert!(loaded.is_chat_pinned("u-self", "u-friend"));
        assert!(
            loaded
                .set_chat_pinned(&dir, "u-self", "u-new", true)
                .is_err()
        );
        assert!(!loaded.is_chat_pinned("u-self", "u-new"));
        fs::remove_dir(dir.join(".settings.json.tmp")).unwrap();
        loaded
            .set_chat_pinned(&dir, "u-self", "u-friend", false)
            .unwrap();
        assert!(!AppConfig::load(&dir).pinned_chats.contains_key("u-self"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hidden_chats_persist_per_account_without_changing_read_policy() {
        let dir = std::env::temp_dir().join(format!("line-gtk-hidden-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.hidden_chats.is_empty());
        cfg.last_chat_mid = "c-group".into();
        cfg.set_chat_hidden(&dir, "u-first", "c-group", Some("検証グループ"))
            .unwrap();
        let mut loaded = AppConfig::load(&dir);
        assert!(loaded.is_chat_hidden("u-first", "c-group"));
        assert!(!loaded.is_chat_hidden("u-second", "c-group"));
        assert_eq!(loaded.hidden_chats["u-first"]["c-group"], "検証グループ");
        assert!(loaded.last_chat_mid.is_empty());
        assert!(loaded.can_mark_read());
        loaded
            .set_chat_hidden(&dir, "u-second", "c-group", Some("別アカウント"))
            .unwrap();
        loaded
            .set_chat_hidden(&dir, "u-first", "c-group", None)
            .unwrap();
        let loaded = AppConfig::load(&dir);
        assert!(!loaded.is_chat_hidden("u-first", "c-group"));
        assert!(loaded.is_chat_hidden("u-second", "c-group"));
        assert!(!loaded.hidden_chats.contains_key("u-first"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hidden_chat_save_failure_restores_visibility_and_last_chat() {
        let dir =
            std::env::temp_dir().join(format!("line-gtk-hidden-failure-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut cfg = AppConfig {
            last_chat_mid: "u-friend".into(),
            ..AppConfig::default()
        };
        cfg.save_atomic(&dir).unwrap();
        fs::create_dir(dir.join(".settings.json.tmp")).unwrap();
        assert!(
            cfg.set_chat_hidden(&dir, "u-self", "u-friend", Some("友だち"))
                .is_err()
        );
        assert!(!cfg.is_chat_hidden("u-self", "u-friend"));
        assert_eq!(cfg.last_chat_mid, "u-friend");
        fs::remove_dir(dir.join(".settings.json.tmp")).unwrap();
        cfg.set_chat_hidden(&dir, "u-self", "u-friend", Some("友だち"))
            .unwrap();
        fs::create_dir(dir.join(".settings.json.tmp")).unwrap();
        assert!(
            cfg.set_chat_hidden(&dir, "u-self", "u-friend", None)
                .is_err()
        );
        assert!(cfg.is_chat_hidden("u-self", "u-friend"));
        assert!(AppConfig::load(&dir).is_chat_hidden("u-self", "u-friend"));
        assert!(cfg.set_chat_hidden(&dir, "", "u-friend", None).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn per_chat_read_pause_is_persistent_and_account_scoped() {
        let dir = std::env::temp_dir().join(format!("line-gtk-chat-read-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.can_mark_chat_read("u-self", "u-friend"));
        cfg.set_chat_read_receipts_paused(&dir, "u-self", "u-friend", true)
            .unwrap();
        let mut cfg = AppConfig::load(&dir);
        assert!(!cfg.can_mark_chat_read("u-self", "u-friend"));
        assert!(cfg.can_mark_chat_read("u-self", "u-other"));
        assert!(cfg.can_mark_chat_read("u-another-account", "u-friend"));
        assert!(!cfg.can_mark_chat_read("", "u-friend"));
        cfg.pause_read_receipts = true;
        assert!(!cfg.can_mark_chat_read("u-self", "u-other"));
        cfg.set_chat_read_receipts_paused(&dir, "u-self", "u-friend", false)
            .unwrap();
        assert!(!cfg.can_mark_chat_read("u-self", "u-friend"));
        cfg.pause_read_receipts = false;
        cfg.auto_mark_read = false;
        assert!(!cfg.can_mark_chat_read("u-self", "u-friend"));
        assert!(cfg.read_receipts_paused_chats.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn per_chat_read_pause_rolls_back_both_changes_on_save_failure() {
        let dir =
            std::env::temp_dir().join(format!("line-gtk-chat-read-failure-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut cfg = AppConfig::default();
        cfg.save_atomic(&dir).unwrap();
        fs::create_dir(dir.join(".settings.json.tmp")).unwrap();
        assert!(
            cfg.set_chat_read_receipts_paused(&dir, "u-self", "u-friend", true)
                .is_err()
        );
        assert!(!cfg.chat_read_receipts_paused("u-self", "u-friend"));
        fs::remove_dir(dir.join(".settings.json.tmp")).unwrap();
        cfg.set_chat_read_receipts_paused(&dir, "u-self", "u-friend", true)
            .unwrap();
        fs::create_dir(dir.join(".settings.json.tmp")).unwrap();
        assert!(
            cfg.set_chat_read_receipts_paused(&dir, "u-self", "u-friend", false)
                .is_err()
        );
        assert!(cfg.chat_read_receipts_paused("u-self", "u-friend"));
        assert!(AppConfig::load(&dir).chat_read_receipts_paused("u-self", "u-friend"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_is_atomic_and_private() {
        let dir = std::env::temp_dir().join(format!(
            "line-gtk-config-test-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        let _ = fs::remove_dir_all(&dir);
        let cfg = AppConfig {
            language: "ja".into(),
            ..AppConfig::default()
        };
        cfg.save_atomic(&dir).expect("save settings");
        assert_eq!(AppConfig::load(&dir).language, cfg.language);
        assert!(!dir.join(".settings.json.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(AppConfig::path(&dir))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        fs::remove_dir_all(dir).expect("remove test directory");
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
            ".line-shell, .line-shell label, .line-shell entry, .line-shell textview, .line-shell textview text {{ font-size: {size}pt; }}"
        )
    } else {
        let escaped = family.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            ".line-shell, .line-shell label, .line-shell entry, .line-shell textview, .line-shell textview text {{ font-family: \"{escaped}\", \"Noto Sans\", sans-serif; font-size: {size}pt; }}"
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
        provider.load_from_string(&css);
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
            provider.load_from_string(MOTION_CSS);
        } else {
            provider.load_from_string("/* animations disabled */");
        }
    });
}
