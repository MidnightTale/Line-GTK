use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    pub id: Option<u64>,
    pub ok: Option<bool>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub event: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StickerItem {
    pub sticker_id: String,
    pub package_id: String,
    pub version: Option<String>,
    pub image_path: Option<String>,
    pub option_key: String,
    pub personal: bool,
    pub animated: bool,
    pub optimized: bool,
    pub optimization: serde_json::Value,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StickerPack {
    pub id: String,
    pub name: String,
    pub source: String,
    pub source_url: Option<String>,
    pub import_pack: Option<String>,
    pub image_only: bool,
    pub incomplete: bool,
    pub lazy: bool,
    pub icon_path: Option<String>,
    pub stickers: Vec<StickerItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub mid: String,
    #[serde(default)]
    pub device: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "statusMessage", default)]
    pub status_message: String,
    #[serde(rename = "picturePath", default)]
    pub picture_path: Option<String>,
    #[serde(rename = "avatarPath", default)]
    pub avatar_path: Option<String>,
    #[serde(rename = "pictureUrl", default)]
    pub picture_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatInfo {
    pub mid: String,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(rename = "avatarPath", default)]
    pub avatar_path: Option<String>,
    #[serde(rename = "lastActivity", default)]
    pub last_activity: i64,
    #[serde(default)]
    pub unread: i64,
    #[serde(default)]
    pub preview: String,
    #[serde(rename = "muted", default)]
    pub muted: bool,
    #[serde(rename = "contactOnly", default)]
    pub contact_only: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockedContact {
    pub mid: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserProfile {
    pub mid: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "statusMessage", default)]
    pub status_message: String,
    #[serde(rename = "avatarPath", default)]
    pub avatar_path: Option<String>,
    #[serde(rename = "isSelf", default)]
    pub is_self: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct FlexAction {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct FlexInfo {
    #[serde(rename = "altText", default)]
    pub alt_text: String,
    #[serde(default)]
    pub texts: Vec<String>,
    #[serde(default)]
    pub actions: Vec<FlexAction>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MessageReadRange {
    #[serde(rename = "startMessageId")]
    pub start_message_id: String,
    #[serde(rename = "endMessageId")]
    pub end_message_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReaderRanges {
    #[serde(rename = "userMid")]
    pub user_mid: String,
    pub ranges: Vec<MessageReadRange>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CallHistory {
    #[serde(default)]
    pub video: bool,
    #[serde(default)]
    pub status: String,
    #[serde(rename = "durationMs", default)]
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ReplyInfo {
    #[serde(rename = "messageId")]
    pub message_id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub mine: bool,
    #[serde(rename = "contentType", default)]
    pub content_type: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LineEmoji {
    pub start: usize,
    pub end: usize,
    pub key: String,
}

/// One person's reaction: `kind` is a LINE reaction name such as `NICE`, or
/// `EMOJI` with the LINE emoji key in `emoji`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MessageReaction {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(rename = "resourceType", default)]
    pub resource_type: Option<i64>,
    #[serde(default)]
    pub version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mention {
    pub start: usize,
    pub end: usize,
    #[serde(default)]
    pub mid: String,
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MessageInfo {
    #[serde(default)]
    pub mentions: Vec<Mention>,
    #[serde(rename = "chatEvent", default)]
    pub chat_event: Option<ChatEventInfo>,
    #[serde(default)]
    pub emojis: Vec<LineEmoji>,
    #[serde(default)]
    pub reactions: Vec<MessageReaction>,
    #[serde(rename = "replyTo", default)]
    pub reply_to: Option<ReplyInfo>,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub mine: bool,
    #[serde(rename = "createdTime", default)]
    pub created_time: i64,
    #[serde(rename = "contentType", default)]
    pub content_type: String,
    #[serde(rename = "decryptionFailed", default)]
    pub decryption_failed: bool,
    #[serde(rename = "imagePath", default)]
    pub image_path: Option<String>,
    #[serde(rename = "audioPath", default)]
    pub audio_path: Option<String>,
    #[serde(rename = "fileName", default)]
    pub file_name: Option<String>,
    #[serde(rename = "filePath", default)]
    pub file_path: Option<String>,
    #[serde(rename = "durationMs", default)]
    pub duration_ms: Option<i64>,
    #[serde(rename = "callHistory", default)]
    pub call_history: Option<CallHistory>,
    #[serde(default)]
    pub flex: Option<FlexInfo>,
}

/// A persisted attachment or URL discovered in one conversation.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SharedMediaItem {
    pub key: String,
    pub message_id: String,
    pub chat_mid: String,
    pub created_time: i64,
    pub sender_mid: String,
    pub kind: String,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub thumbnail_path: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub cache_state: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SharedMediaPage {
    pub chat_mid: String,
    pub category: String,
    pub items: Vec<SharedMediaItem>,
    #[serde(default)]
    pub next_cursor: String,
    pub scan_state: String,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub cached_count: usize,
    #[serde(default)]
    pub pending_count: usize,
    #[serde(default)]
    pub failed_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedMediaDownload {
    pub path: String,
    pub file_name: String,
    pub content_type: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ChatEventInfo {
    pub kind: String,
    #[serde(rename = "actorMid", default)]
    pub actor_mid: String,
    #[serde(rename = "targetMids", default)]
    pub target_mids: Vec<String>,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum ProtocolEvent {
    ChatEvent {
        account_mid: String,
        message: Box<MessageInfo>,
    },
    Ready {
        has_auth: bool,
    },
    Session {
        mid: String,
        device: String,
        display_name: String,
        status_message: String,
        picture_path: Option<String>,
        avatar_path: Option<String>,
        picture_url: Option<String>,
    },
    /// Chats and profile saved by an earlier run, sent before login finishes.
    CachedBoot {
        mid: String,
        display_name: String,
        status_message: String,
        picture_path: Option<String>,
        avatar_path: Option<String>,
        picture_url: Option<String>,
        chats: Vec<ChatInfo>,
    },
    SessionFailed {
        error: String,
    },
    Connection {
        state: String,
        kind: String,
        attempt: u64,
    },
    Qr {
        url: String,
    },
    Pin {
        pin: String,
    },
    Listening,
    MessageUnsent {
        account_mid: String,
        chat_mid: String,
        message_id: String,
    },
    /// A chat's on-disk history is close to, or at, the configured limit.
    MessageCacheLimit {
        chat_mid: String,
        count: u64,
        limit: u64,
        full: bool,
    },
    MessageReaction {
        account_mid: String,
        chat_mid: String,
        message_id: String,
        from: String,
        reaction: Option<MessageReaction>,
    },
    Message(Box<MessageInfo>),
    Chats {
        chats: Vec<ChatInfo>,
        cached: bool,
    },
    Messages {
        chat_mid: String,
        messages: Vec<MessageInfo>,
    },
    AvatarReady {
        mid: String,
        avatar_path: String,
    },
    SenderProfile {
        mid: String,
        name: String,
    },
    StickerImportProgress {
        request_id: u64,
        account_mid: String,
        done: u64,
        total: u64,
    },
    SharedMediaChanged {
        account_mid: String,
        chat_mid: String,
    },
    EmojiReady {
        key: String,
        path: String,
    },
    FriendsUpdated {
        friends: Vec<ChatInfo>,
    },
    ChatUpsert {
        chat: ChatInfo,
    },
    ChatPreview {
        mid: String,
        preview: String,
        last_activity: i64,
        contact_only: Option<bool>,
    },
    ChatMute {
        mid: String,
        muted: bool,
    },
    MediaReady {
        chat_mid: String,
        message_id: String,
        image_path: String,
        audio_path: Option<String>,
        file_path: Option<String>,
    },
    MediaFailed {
        chat_mid: String,
        message_id: String,
    },
    Progress {
        scope: String,
        chat_mid: Option<String>,
        state: String,
        error: Option<String>,
    },
    DownloadProgress {
        message_id: String,
        progress: f64,
    },
    UploadProgress {
        chat_mid: String,
        progress: f64,
        label: String,
        done: bool,
    },
    ChatUnread {
        chat_mid: String,
        account_mid: String,
        through_message_id: String,
        unread: i64,
    },
    ContactBlocks {
        account_mid: String,
        contacts: Vec<BlockedContact>,
    },
    ReadReceipt {
        chat_mid: String,
        user_mid: String,
        message_id: String,
    },
    ReadRanges {
        chat_mid: String,
        readers: Vec<ReaderRanges>,
    },
    CallIncoming {
        call_id: String,
        from: String,
        kind: String,
    },
    CallCanceled {
        call_id: String,
        from: String,
        reason: String,
    },
    CallState {
        call_id: String,
        peer: String,
        state: String,
        error: Option<String>,
    },
    Response {
        id: u64,
        ok: bool,
        result: Value,
        error: Option<String>,
    },
    Error(String),
    Exited(i32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_library_items_keep_stable_settings_and_photo_delivery() {
        let pack: StickerPack = serde_json::from_value(serde_json::json!({
            "id": "personal-pack", "name": "My PNGs", "source": "local_png", "imageOnly": true,
            "stickers": [{ "stickerId": "one", "packageId": "personal-pack", "imagePath": "/library/a.png", "optionKey": "account/pack/one", "personal": true }]
        })).unwrap();
        assert!(pack.image_only);
        assert!(!pack.lazy);
        assert!(pack.stickers[0].personal);
        assert_eq!(pack.stickers[0].option_key, "account/pack/one");
        let native: StickerPack =
            serde_json::from_value(serde_json::json!({ "id": "2", "name": "Owned", "lazy": true }))
                .unwrap();
        assert!(native.lazy);
        assert!(!native.image_only);
    }

    #[test]
    fn parses_event_envelope_and_extra_fields() {
        let response: Response =
            serde_json::from_str(r#"{"event":"ready","hasAuth":true,"dataDir":"/tmp/line-gtk"}"#)
                .unwrap();
        assert_eq!(response.event.as_deref(), Some("ready"));
        assert_eq!(response.extra["hasAuth"], true);
        assert_eq!(response.extra["dataDir"], "/tmp/line-gtk");
    }

    #[test]
    fn message_defaults_tolerate_protocol_evolution() {
        let message: MessageInfo = serde_json::from_str(r#"{"id":"42","text":"hello"}"#).unwrap();
        assert_eq!(message.id, "42");
        assert_eq!(message.text, "hello");
        assert_eq!(message.created_time, 0);
        assert!(!message.mine);
        assert!(message.image_path.is_none());
    }

    #[test]
    fn request_omits_absent_params() {
        let request = Request {
            id: 7,
            method: "list_chats".into(),
            params: None,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "id": 7, "method": "list_chats" })
        );
    }
}
