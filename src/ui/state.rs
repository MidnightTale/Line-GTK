use super::{call_window, friends, login, virtual_list};
use crate::config::AppConfig;
use crate::protocol::{ChatInfo, MessageInfo};
use crate::sidecar::Sidecar;
use gtk::glib;
use libadwaita::{Application, ApplicationWindow};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

/// Currently playing voice bubble (UI + process).
pub(super) struct VoicePlayback {
    pub child: std::process::Child,
    pub msg_id: String,
    pub play_btn: gtk::Button,
    pub wave: gtk::DrawingArea,
    pub dur: gtk::Label,
    pub duration_ms: u64,
    pub started: std::time::Instant,
    pub progress: Rc<RefCell<f32>>,
    pub total_label: String,
    pub tick: Option<glib::SourceId>,
}

pub(super) type MessageListFingerprint = (String, usize, String, String);

#[derive(Clone)]
pub(super) struct AppState {
    pub app: Application,
    pub sidecar: Rc<Sidecar>,
    pub window: ApplicationWindow,
    pub toast_overlay: libadwaita::ToastOverlay,
    pub stack: gtk::Stack,
    pub chat_list: gtk::ListBox,
    pub message_list: virtual_list::VirtualMessageList,
    pub message_scroll: gtk::ScrolledWindow,
    pub composer: gtk::Entry,
    pub composer_row: gtk::Box,
    pub conversation: gtk::Box,
    pub send_btn: gtk::Button,
    pub status: gtk::Label,
    pub login: Rc<login::LoginWidgets>,
    pub profile_label: gtk::Label,
    pub profile_avatar: gtk::Picture,
    pub brand_label: gtk::Label,
    pub brand_icon: gtk::Image,
    pub chat_title: gtk::Label,
    pub chat_subtitle: gtk::Label,
    pub side_stack: gtk::Stack,
    pub side_spinner: gtk::Spinner,
    pub side_empty: gtk::Label,
    pub side_load_label: gtk::Label,
    pub msg_stack: gtk::Stack,
    pub msg_spinner: gtk::Spinner,
    pub msg_empty: gtk::Label,
    pub msg_load_label: gtk::Label,
    pub msg_idle: gtk::Label,
    pub current_chat: Rc<RefCell<Option<String>>>,
    pub chats: Rc<RefCell<Vec<ChatInfo>>>,
    pub chat_avatars: Rc<RefCell<HashMap<String, gtk::Picture>>>,
    pub chat_previews: Rc<RefCell<HashMap<String, gtk::Label>>>,
    pub chat_unread_badges: Rc<RefCell<HashMap<String, gtk::Label>>>,
    pub media_slots: Rc<RefCell<HashMap<String, gtk::Box>>>,
    pub media_msgs: Rc<RefCell<HashMap<String, MessageInfo>>>,
    pub receipt_slots: Rc<RefCell<HashMap<String, gtk::Label>>>,
    pub msg_created: Rc<RefCell<HashMap<String, i64>>>,
    pub last_msg_day: Rc<RefCell<Option<String>>>,
    pub seen_msg_ids: Rc<RefCell<HashSet<String>>>,
    pub last_incoming_id: Rc<RefCell<Option<String>>>,
    pub read_upto: Rc<RefCell<HashMap<String, String>>>,
    pub restored_last_chat: Rc<RefCell<bool>>,
    pub media_queue: Rc<RefCell<VecDeque<(String, String)>>>,
    pub media_pumping: Rc<RefCell<bool>>,
    pub stick_bottom: Rc<RefCell<bool>>,
    pub scroll_pinning: Rc<RefCell<bool>>,
    pub scroll_pin_gen: Rc<RefCell<u64>>,
    pub new_sep_row: Rc<RefCell<Option<gtk::ListBoxRow>>>,
    pub pending_new_below: Rc<RefCell<u32>>,
    pub jump_banner: gtk::Revealer,
    pub jump_banner_btn: gtk::Button,
    pub jump_banner_label: gtk::Label,
    pub pending: Rc<RefCell<HashMap<u64, Pending>>>,
    pub pending_rows: Rc<RefCell<HashMap<String, gtk::ListBoxRow>>>,
    pub restarting: Rc<RefCell<bool>>,
    pub recovery_attempts: Rc<RefCell<u8>>,
    pub repo_root: PathBuf,
    pub data_dir: PathBuf,
    pub config: Rc<RefCell<AppConfig>>,
    pub settings_btn: gtk::Button,
    pub friends_btn: gtk::Button,
    pub friends_ui: Rc<RefCell<Option<friends::FriendsUi>>>,
    pub side_title: gtk::Label,
    pub search_entry: gtk::SearchEntry,
    pub compact_search_btn: gtk::Button,
    pub compact_search_entry: gtk::SearchEntry,
    pub sidebar: gtk::Box,
    pub sidebar_paned: gtk::Paned,
    pub side_header: gtk::Box,
    pub sidebar_compact: Rc<RefCell<bool>>,
    pub composer_narrow: Rc<RefCell<bool>>,
    pub mic_btn: gtk::Button,
    pub attach_btn: gtk::Button,
    pub sticker_btn: gtk::Button,
    pub sticker_popover: gtk::Popover,
    pub call_btn: gtk::Button,
    pub mute_btn: gtk::Button,
    pub pin_btn: gtk::Button,
    pub album_btn: gtk::Button,
    pub composer_stack: gtk::Stack,
    pub record_cancel_btn: gtk::Button,
    pub record_send_btn: gtk::Button,
    pub record_timer: gtk::Label,
    pub record_wave: gtk::DrawingArea,
    pub upload_revealer: gtk::Revealer,
    pub upload_bar: gtk::ProgressBar,
    pub upload_label: gtk::Label,
    pub recording: Rc<RefCell<Option<std::process::Child>>>,
    pub recording_started: Rc<RefCell<Option<std::time::Instant>>>,
    pub recording_levels: Rc<RefCell<Vec<f32>>>,
    pub recording_tick: Rc<RefCell<Option<glib::SourceId>>>,
    pub voice_playback: Rc<RefCell<Option<VoicePlayback>>>,
    pub session_ready: Rc<RefCell<bool>>,
    pub active_call_peer: Rc<RefCell<Option<String>>>,
    pub incoming_call_from: Rc<RefCell<Option<String>>>,
    pub call_ui: Rc<RefCell<Option<call_window::CallUi>>>,
    pub call_mic_muted: Rc<RefCell<bool>>,
    pub call_deafened: Rc<RefCell<bool>>,
    pub call_video_capable: Rc<RefCell<bool>>,
    pub call_screen_sharing: Rc<RefCell<bool>>,
    pub tray: Rc<RefCell<Option<crate::tray::TrayController>>>,
    pub tray_tx: async_channel::Sender<crate::tray::TrayAction>,
    pub discord: crate::discord_rpc::DiscordRpc,
    pub discord_session_start: Rc<RefCell<Option<i64>>>,
    pub self_mid: Rc<RefCell<Option<String>>>,
    pub self_display_name: Rc<RefCell<String>>,
    pub self_avatar_path: Rc<RefCell<Option<String>>>,
    pub self_picture_url: Rc<RefCell<Option<String>>>,
    pub msg_list_fp: Rc<RefCell<Option<MessageListFingerprint>>>,
    pub notif_pending: Rc<RefCell<HashMap<String, PendingNotif>>>,
    pub media_ready_paths: Rc<RefCell<HashMap<String, String>>>,
}

#[derive(Debug, Clone)]
pub(super) struct PendingNotif {
    pub chat_mid: String,
    pub title: String,
    pub body: String,
    pub avatar_path: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ProfileChatTarget {
    pub mid: String,
    pub name: String,
    pub avatar_path: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ProfilePendingUi {
    pub target: Rc<RefCell<ProfileChatTarget>>,
    pub avatar: gtk::Picture,
    pub name_label: gtk::Label,
    pub bio_label: gtk::Label,
    pub status: gtk::Label,
    pub add_btn: gtk::Button,
    pub chat_btn: gtk::Button,
}

#[derive(Debug, Clone)]
pub(super) enum Pending {
    Login,
    ListChats,
    ListFriends,
    FetchMessages {
        chat_mid: String,
    },
    Send {
        chat_mid: String,
        placeholder_id: String,
    },
    ListStickers,
    DownloadMedia {
        message_id: String,
        action: String,
        content_type: String,
        suggest_name: String,
    },
    ProfileLookup(ProfilePendingUi),
    ProfileAddFriend(ProfilePendingUi),
    ReactMessage,
    UnsendMessage,
    CallScreenStart,
    CallScreenStop,
}
