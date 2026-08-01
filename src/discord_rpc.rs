//! Discord Rich Presence via IPC (`discord-rich-presence`).
//!
//! Runs on a background thread so the GTK UI never blocks on Discord sockets.
//! Reconnects automatically when Discord restarts.

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

/// Public Discord Application (Client) ID for LINE GTK.
/// Override with Settings or `LINE_GTK_DISCORD_APP_ID`.
pub const DEFAULT_APP_ID: &str = "1533109311545278554";

const GITHUB_URL: &str = "https://github.com/MidnightTale/line-gtk";
const RECV_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub details: String,
    pub state: String,
    pub start_unix: Option<i64>,
    /// HTTPS URL or Discord asset key for the small overlay image (user avatar).
    pub small_image: Option<String>,
    /// Tooltip on the small image (usually display name).
    pub small_text: Option<String>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
}

enum Cmd {
    Configure { enabled: bool, app_id: String },
    Set(Presence),
    Clear,
    Shutdown,
}

/// Cloneable handle that posts presence updates to the IPC worker.
#[derive(Clone)]
pub struct DiscordRpc {
    tx: Sender<Cmd>,
}

impl DiscordRpc {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("discord-rpc".into())
            .spawn(move || worker(rx));
        Self { tx }
    }

    pub fn configure(&self, enabled: bool, app_id: &str) {
        let _ = self.tx.send(Cmd::Configure {
            enabled,
            app_id: app_id.trim().to_string(),
        });
    }

    pub fn set(&self, presence: Presence) {
        let _ = self.tx.send(Cmd::Set(presence));
    }

    pub fn clear(&self) {
        let _ = self.tx.send(Cmd::Clear);
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

fn worker(rx: Receiver<Cmd>) {
    let mut enabled = false;
    let mut app_id = String::new();
    let mut client: Option<DiscordIpcClient> = None;
    let mut last: Option<Presence> = None;
    let mut need_apply = false;

    loop {
        let timed_out = if enabled {
            match rx.recv_timeout(RECV_TIMEOUT) {
                Ok(cmd) => {
                    if handle_cmd(
                        cmd,
                        &mut enabled,
                        &mut app_id,
                        &mut client,
                        &mut last,
                        &mut need_apply,
                    ) {
                        break;
                    }
                    false
                }
                Err(RecvTimeoutError::Timeout) => true,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            // Stay asleep until Settings enables RPC (no periodic wakeups).
            match rx.recv() {
                Ok(cmd) => {
                    if handle_cmd(
                        cmd,
                        &mut enabled,
                        &mut app_id,
                        &mut client,
                        &mut last,
                        &mut need_apply,
                    ) {
                        break;
                    }
                    false
                }
                Err(_) => break,
            }
        };

        // Drain any burst of updates so we only publish the latest.
        loop {
            match rx.try_recv() {
                Ok(cmd) => {
                    if handle_cmd(
                        cmd,
                        &mut enabled,
                        &mut app_id,
                        &mut client,
                        &mut last,
                        &mut need_apply,
                    ) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        if timed_out {
            // Only reconnect when Discord went away; do not re-push unchanged presence.
            if enabled && !app_id.is_empty() && client.is_none() {
                need_apply = true;
            }
        }

        if !enabled || app_id.is_empty() {
            if let Some(mut c) = client.take() {
                let _ = c.clear_activity();
                let _ = c.close();
            }
            need_apply = false;
            continue;
        }

        if client.is_none() {
            let mut c = DiscordIpcClient::new(&app_id);
            match c.connect() {
                Ok(()) => {
                    tracing::debug!(%app_id, "discord rpc connected");
                    client = Some(c);
                    need_apply = true;
                }
                Err(e) => {
                    tracing::debug!("discord rpc connect: {e}");
                    need_apply = false;
                    continue;
                }
            }
        }

        if !need_apply {
            continue;
        }

        let Some(c) = client.as_mut() else {
            continue;
        };

        let result = match last.as_ref() {
            Some(p) => apply_presence(c, p),
            None => c.clear_activity().map_err(|e| e.to_string()),
        };

        match result {
            Ok(()) => need_apply = false,
            Err(e) => {
                tracing::debug!("discord rpc apply: {e}");
                let _ = c.close();
                client = None;
            }
        }
    }
}

/// Returns true if the worker should exit.
fn handle_cmd(
    cmd: Cmd,
    enabled: &mut bool,
    app_id: &mut String,
    client: &mut Option<DiscordIpcClient>,
    last: &mut Option<Presence>,
    need_apply: &mut bool,
) -> bool {
    match cmd {
        Cmd::Shutdown => {
            if let Some(mut c) = client.take() {
                let _ = c.clear_activity();
                let _ = c.close();
            }
            true
        }
        Cmd::Configure {
            enabled: on,
            app_id: id,
        } => {
            if id != *app_id {
                if let Some(mut c) = client.take() {
                    let _ = c.clear_activity();
                    let _ = c.close();
                }
                *app_id = id;
            }
            *enabled = on;
            if !on {
                if let Some(mut c) = client.take() {
                    let _ = c.clear_activity();
                    let _ = c.close();
                }
                *need_apply = false;
            } else {
                *need_apply = true;
            }
            false
        }
        Cmd::Set(p) => {
            if last.as_ref() != Some(&p) {
                *last = Some(p);
                *need_apply = true;
            }
            false
        }
        Cmd::Clear => {
            *last = None;
            *need_apply = true;
            false
        }
    }
}

fn apply_presence(client: &mut DiscordIpcClient, presence: &Presence) -> Result<(), String> {
    let mut act = activity::Activity::new()
        .name("LINE GTK")
        .activity_type(activity::ActivityType::Playing)
        .details(presence.details.as_str())
        .state(presence.state.as_str())
        .buttons(vec![activity::Button::new("GitHub", GITHUB_URL)]);

    if let Some(start) = presence.start_unix {
        act = act.timestamps(activity::Timestamps::new().start(start));
    }

    let mut assets = activity::Assets::new();
    let mut has_assets = false;
    if let Some(img) = presence.large_image.as_deref().filter(|s| !s.is_empty()) {
        assets = assets.large_image(img);
        has_assets = true;
    }
    if let Some(txt) = presence.large_text.as_deref().filter(|s| !s.is_empty()) {
        assets = assets.large_text(txt);
        has_assets = true;
    }
    if let Some(img) = presence.small_image.as_deref().filter(|s| !s.is_empty()) {
        assets = assets.small_image(img);
        has_assets = true;
    }
    if let Some(txt) = presence.small_text.as_deref().filter(|s| !s.is_empty()) {
        assets = assets.small_text(txt);
        has_assets = true;
    }
    if has_assets {
        act = act.assets(assets);
    }

    client
        .set_activity(act)
        .map_err(|e| e.to_string())
}
