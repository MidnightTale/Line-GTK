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

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub mid: String,
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
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct FlexInfo {
    #[serde(rename = "altText", default)]
    pub alt_text: String,
    #[serde(default)]
    pub texts: Vec<String>,
    #[serde(default)]
    pub actions: Vec<FlexAction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageInfo {
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
    #[serde(rename = "imagePath", default)]
    pub image_path: Option<String>,
    #[serde(rename = "imageUrl", default)]
    pub image_url: Option<String>,
    #[serde(rename = "audioPath", default)]
    pub audio_path: Option<String>,
    #[serde(rename = "fileName", default)]
    pub file_name: Option<String>,
    #[serde(rename = "filePath", default)]
    pub file_path: Option<String>,
    #[serde(rename = "durationMs", default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub flex: Option<FlexInfo>,
}

#[derive(Debug, Clone)]
pub enum ProtocolEvent {
    Ready { has_auth: bool },
    Session {
        mid: String,
        display_name: String,
        status_message: String,
        picture_path: Option<String>,
        avatar_path: Option<String>,
        picture_url: Option<String>,
    },
    SessionFailed { error: String },
    Qr { url: String },
    Pin { pin: String },
    Listening,
    Message(MessageInfo),
    Chats {
        chats: Vec<ChatInfo>,
        cached: bool,
    },
    Messages {
        chat_mid: String,
        messages: Vec<MessageInfo>,
        cached: bool,
    },
    AvatarReady { mid: String, avatar_path: String },
    FriendsUpdated { friends: Vec<ChatInfo> },
    ChatUpsert {
        chat: ChatInfo,
        created: bool,
    },
    ChatPreview {
        mid: String,
        preview: String,
        last_activity: i64,
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
    UploadProgress {
        chat_mid: String,
        progress: f64,
        label: String,
        done: bool,
    },
    ReadReceipt {
        chat_mid: String,
        user_mid: String,
        message_id: String,
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
