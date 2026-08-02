use crate::protocol::{ProtocolEvent, Request, Response};
use anyhow::{Context, Result, anyhow};
use async_channel::{Receiver, Sender};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Sidecar {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    next_id: AtomicU64,
    generation: Arc<AtomicU64>,
    pending_ids: Arc<Mutex<HashSet<u64>>>,
    timeout_tx: std::sync::mpsc::Sender<(u64, Instant)>,
    event_tx: Sender<ProtocolEvent>,
    pub events: Receiver<ProtocolEvent>,
    repo_root: PathBuf,
    data_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SidecarStatus {
    pub runtime: &'static str,
    pub running: bool,
    pub pid: Option<u32>,
    pub pending_requests: usize,
}

impl Sidecar {
    pub fn spawn(repo_root: &Path, data_dir: &Path) -> Result<Self> {
        let (tx, rx) = async_channel::bounded::<ProtocolEvent>(1024);
        let pending_ids = Arc::new(Mutex::new(HashSet::new()));
        let (timeout_tx, timeout_rx) = std::sync::mpsc::channel();
        spawn_timeout_worker(timeout_rx, pending_ids.clone(), tx.clone());
        let sidecar = Self {
            child: Arc::new(Mutex::new(None)),
            stdin: Arc::new(Mutex::new(None)),
            next_id: AtomicU64::new(1),
            generation: Arc::new(AtomicU64::new(0)),
            pending_ids,
            timeout_tx,
            event_tx: tx,
            events: rx,
            repo_root: repo_root.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
        };
        sidecar.start_process()?;
        Ok(sidecar)
    }

    fn start_process(&self) -> Result<()> {
        let script = self.repo_root.join("protocol/src/main.ts");
        let compiled = self.repo_root.join("protocol/line-gtk-protocol");
        if !compiled.is_file() && !script.exists() {
            return Err(anyhow!("protocol sidecar missing at {}", script.display()));
        }
        crate::config::ensure_private_dir(&self.data_dir)?;
        let cfg = crate::config::AppConfig::load(&self.data_dir);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;

        let (mut command, runtime) = if compiled.is_file() {
            (Command::new(&compiled), compiled)
        } else {
            let deno = find_deno()?;
            let mut command = Command::new(&deno);
            command
                .arg("run")
                .arg("--no-prompt")
                // Attachments may be selected from anywhere, but writes remain
                // confined to application data. Network endpoints are negotiated
                // dynamically by LINE and cannot be safely enumerated.
                .arg("--allow-read")
                .arg(format!("--allow-write={}", self.data_dir.display()))
                .arg("--allow-net")
                .arg("--allow-run=ffmpeg,ffprobe,slurp,wf-recorder")
                .arg("--allow-sys")
                .arg("--allow-env=HOME,PATH,DENO_DIR,Q_DEBUG,NODE_DEBUG,NO_COLOR,LINE_GTK_DATA,LINE_GTK_LANG,LINE_GTK_CACHE_RETENTION,LINE_GTK_AUDIO_INPUT,LINE_GTK_AUDIO_OUTPUT,LINE_DEVICE,LINE_VERSION,LINE_CALL_DEVNAME,LINE_CALL_DEVICE_INFO,LINE_CALL_OPUS_SIGNAL,LINE_CALL_DEBUG")
                .arg(&script);
            (command, deno)
        };

        let mut child = command
            .current_dir(self.repo_root.join("protocol"))
            .env("LINE_GTK_DATA", &self.data_dir)
            .env("LINE_GTK_LANG", cfg.language.clone())
            .env("LINE_GTK_CACHE_RETENTION", cfg.cache_retention.clone())
            .env("LINE_GTK_AUDIO_INPUT", {
                if cfg.audio_input.is_empty() {
                    "default".into()
                } else {
                    cfg.audio_input
                }
            })
            .env("LINE_GTK_AUDIO_OUTPUT", {
                if cfg.audio_output.is_empty() {
                    "default".into()
                } else {
                    cfg.audio_output
                }
            })
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    runtime.parent().unwrap_or(Path::new(".")).display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!("failed to spawn protocol runtime at {}", runtime.display())
            })?;

        let stdout = child.stdout.take().context("sidecar stdout missing")?;
        let stdin = child.stdin.take().context("sidecar stdin missing")?;
        let stderr = child.stderr.take().context("sidecar stderr missing")?;
        let tx = self.event_tx.clone();
        let pending_ids = self.pending_ids.clone();
        let current_generation = self.generation.clone();

        thread::Builder::new()
            .name("line-sidecar-stdout".into())
            .spawn(move || {
                read_loop(stdout, tx.clone(), pending_ids);
                if current_generation.load(Ordering::SeqCst) == generation {
                    let _ = tx.send_blocking(ProtocolEvent::Exited(-1));
                }
            })
            .context("failed to start sidecar reader")?;

        thread::Builder::new()
            .name("line-sidecar-stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(|line| line.ok()) {
                    tracing::warn!(target: "line_gtk::protocol", "{line}");
                }
            })
            .context("failed to start sidecar stderr reader")?;

        *self.child.lock().map_err(|_| anyhow!("child lock"))? = Some(child);
        *self.stdin.lock().map_err(|_| anyhow!("stdin lock"))? = Some(stdin);
        Ok(())
    }

    /// Kill the protocol process and start a fresh one (for Retry during PIN).
    pub fn restart(&self) -> Result<()> {
        self.stop_process();
        // Drop stale auth so Ready always goes through QR again.
        for name in ["auth-token.txt", "auth-device.txt"] {
            let _ = std::fs::remove_file(self.data_dir.join(name));
        }
        self.start_process()
    }

    /// Restart after an unexpected crash while preserving the linked session.
    pub fn recover(&self) -> Result<()> {
        self.stop_process();
        self.start_process()
    }

    pub fn status(&self) -> SidecarStatus {
        let (running, pid) = self
            .child
            .lock()
            .ok()
            .and_then(|mut child| {
                child.as_mut().map(|process| {
                    let running = process.try_wait().ok().flatten().is_none();
                    (running, Some(process.id()))
                })
            })
            .unwrap_or((false, None));
        SidecarStatus {
            runtime: if self.repo_root.join("protocol/line-gtk-protocol").is_file() {
                "compiled"
            } else {
                "deno"
            },
            running,
            pid,
            pending_requests: self.pending_ids.lock().map(|ids| ids.len()).unwrap_or(0),
        }
    }

    fn stop_process(&self) {
        // Invalidate the reader before closing stdout so an intentional restart
        // is not reported as an unexpected crash.
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut guard) = self.child.lock()
            && let Some(mut child) = guard.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Ok(mut stdin) = self.stdin.lock() {
            *stdin = None;
        }
        if let Ok(mut pending) = self.pending_ids.lock() {
            pending.clear();
        }
    }

    pub fn request(&self, method: &str, params: Option<Value>) -> Result<u64> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = Request {
            id,
            method: method.to_string(),
            params,
        };
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        let mut stdin_guard = self.stdin.lock().map_err(|_| anyhow!("stdin lock"))?;
        let stdin = stdin_guard
            .as_mut()
            .ok_or_else(|| anyhow!("sidecar stdin unavailable"))?;
        self.pending_ids
            .lock()
            .map_err(|_| anyhow!("pending request lock"))?
            .insert(id);
        if let Err(error) = stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
        {
            if let Ok(mut pending) = self.pending_ids.lock() {
                pending.remove(&id);
            }
            return Err(error.into());
        }
        let _ = self.timeout_tx.send((id, Instant::now()));
        Ok(id)
    }

    pub fn login_qr(&self) -> Result<u64> {
        self.request("login_qr", None)
    }

    pub fn list_chats(&self) -> Result<u64> {
        self.request("list_chats", Some(json!({ "force": false })))
    }

    pub fn fetch_messages(&self, chat_mid: &str, limit: u32) -> Result<u64> {
        self.request(
            "fetch_messages",
            Some(json!({ "chatMid": chat_mid, "limit": limit, "force": false })),
        )
    }

    pub fn send_message(&self, chat_mid: &str, text: &str) -> Result<u64> {
        self.request(
            "send_message",
            Some(json!({ "chatMid": chat_mid, "text": text })),
        )
    }

    pub fn react_message(&self, chat_mid: &str, message_id: &str, reaction: &str) -> Result<u64> {
        self.request(
            "react_message",
            Some(json!({
                "chatMid": chat_mid,
                "messageId": message_id,
                "reaction": reaction,
            })),
        )
    }

    pub fn unsend_message(&self, chat_mid: &str, message_id: &str) -> Result<u64> {
        self.request(
            "unsend_message",
            Some(json!({ "chatMid": chat_mid, "messageId": message_id })),
        )
    }

    pub fn send_audio(
        &self,
        chat_mid: &str,
        file_path: &str,
        duration_ms: Option<u64>,
    ) -> Result<u64> {
        let mut params = json!({ "chatMid": chat_mid, "filePath": file_path });
        if let Some(ms) = duration_ms {
            params["durationMs"] = json!(ms);
        }
        self.request("send_audio", Some(params))
    }

    pub fn send_media(
        &self,
        chat_mid: &str,
        file_path: &str,
        o_type: &str,
        duration_ms: Option<u64>,
    ) -> Result<u64> {
        let mut params = json!({
            "chatMid": chat_mid,
            "filePath": file_path,
            "oType": o_type,
        });
        if let Some(ms) = duration_ms {
            params["durationMs"] = json!(ms);
        }
        self.request("send_media", Some(params))
    }

    pub fn download_media(&self, chat_mid: &str, message_id: &str) -> Result<u64> {
        self.request(
            "download_media",
            Some(json!({ "chatMid": chat_mid, "messageId": message_id })),
        )
    }

    pub fn send_sticker(
        &self,
        chat_mid: &str,
        sticker_id: &str,
        package_id: &str,
        version: Option<&str>,
    ) -> Result<u64> {
        let mut params = json!({
            "chatMid": chat_mid,
            "stickerId": sticker_id,
            "packageId": package_id,
        });
        if let Some(v) = version.filter(|s| !s.is_empty()) {
            params["version"] = json!(v);
        }
        self.request("send_sticker", Some(params))
    }

    pub fn list_stickers(&self) -> Result<u64> {
        self.request("list_stickers", None)
    }

    pub fn mark_read(&self, chat_mid: &str, last_message_id: &str) -> Result<u64> {
        self.request(
            "mark_read",
            Some(json!({ "chatMid": chat_mid, "lastMessageId": last_message_id })),
        )
    }

    pub fn mute_chat(&self, mid: &str, muted: bool) -> Result<u64> {
        self.request("mute_chat", Some(json!({ "mid": mid, "muted": muted })))
    }

    pub fn call_start(
        &self,
        mid: &str,
        audio_input: &str,
        audio_output: &str,
        mic_gain: f64,
        spk_gain: f64,
        video_capable: bool,
    ) -> Result<u64> {
        self.request(
            "call_start",
            Some(json!({
                "mid": mid,
                "audioInput": if audio_input.is_empty() { "default" } else { audio_input },
                "audioOutput": if audio_output.is_empty() { "default" } else { audio_output },
                "micGain": mic_gain,
                "spkGain": spk_gain,
                "videoCapable": video_capable,
            })),
        )
    }

    pub fn call_answer(
        &self,
        audio_input: &str,
        audio_output: &str,
        mic_gain: f64,
        spk_gain: f64,
    ) -> Result<u64> {
        self.request(
            "call_answer",
            Some(json!({
                "audioInput": if audio_input.is_empty() { "default" } else { audio_input },
                "audioOutput": if audio_output.is_empty() { "default" } else { audio_output },
                "micGain": mic_gain,
                "spkGain": spk_gain,
            })),
        )
    }

    pub fn call_decline(&self) -> Result<u64> {
        self.request("call_decline", None)
    }

    pub fn call_end(&self) -> Result<u64> {
        self.request("call_end", None)
    }

    pub fn call_screen_start(&self) -> Result<u64> {
        self.request("call_screen_start", None)
    }

    pub fn call_screen_stop(&self) -> Result<u64> {
        self.request("call_screen_stop", None)
    }

    pub fn call_set_audio(
        &self,
        muted: bool,
        deafened: bool,
        mic_gain: Option<f64>,
        spk_gain: Option<f64>,
    ) -> Result<u64> {
        let mut body = json!({ "muted": muted, "deafened": deafened });
        if let Some(g) = mic_gain {
            body["micGain"] = json!(g);
        }
        if let Some(g) = spk_gain {
            body["spkGain"] = json!(g);
        }
        self.request("call_set_audio", Some(body))
    }

    pub fn send_postback(
        &self,
        chat_mid: &str,
        message_id: &str,
        data: &str,
        uri: Option<&str>,
    ) -> Result<u64> {
        self.request(
            "send_postback",
            Some(json!({
                "chatMid": chat_mid,
                "messageId": message_id,
                "data": data,
                "uri": uri,
            })),
        )
    }

    pub fn clear_cache(&self, kind: &str) -> Result<u64> {
        self.request("clear_cache", Some(json!({ "kind": kind })))
    }

    pub fn list_friends(&self) -> Result<u64> {
        self.request("list_friends", Some(json!({ "force": false })))
    }

    pub fn add_friend(&self, userid: &str) -> Result<u64> {
        self.request("add_friend", Some(json!({ "userid": userid })))
    }

    pub fn profile_relation(&self, mid: &str) -> Result<u64> {
        self.request("profile_relation", Some(json!({ "mid": mid })))
    }

    pub fn add_friend_mid(&self, mid: &str) -> Result<u64> {
        self.request("add_friend_mid", Some(json!({ "mid": mid })))
    }

    pub fn logout(&self) -> Result<u64> {
        self.request("logout", None)
    }
}

fn find_deno() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("DENO") {
        return Ok(PathBuf::from(p));
    }
    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".deno/bin/deno");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    which("deno").ok_or_else(|| anyhow!("deno not found; install from https://deno.land"))
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let p = dir.join(bin);
            p.exists().then_some(p)
        })
    })
}

fn read_loop<R: std::io::Read>(
    stdout: R,
    tx: Sender<ProtocolEvent>,
    pending_ids: Arc<Mutex<HashSet<u64>>>,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Ignore non-JSON logs (e.g. linejs `[login] ForSecure: ...`).
        if !line.starts_with('{') {
            eprintln!("[sidecar] {line}");
            continue;
        }
        match serde_json::from_str::<Response>(line) {
            Ok(resp) => {
                if let Some(event) = resp.event.as_deref() {
                    let ev = match event {
                        "ready" => ProtocolEvent::Ready {
                            has_auth: resp
                                .extra
                                .get("hasAuth")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        },
                        "session" => ProtocolEvent::Session {
                            mid: resp
                                .extra
                                .get("mid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            display_name: resp
                                .extra
                                .get("displayName")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            status_message: resp
                                .extra
                                .get("statusMessage")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            picture_path: resp
                                .extra
                                .get("picturePath")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            avatar_path: resp
                                .extra
                                .get("avatarPath")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            picture_url: resp
                                .extra
                                .get("pictureUrl")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        },
                        "session_failed" => ProtocolEvent::SessionFailed {
                            error: resp
                                .extra
                                .get("error")
                                .and_then(|v| v.as_str())
                                .unwrap_or("session restore failed")
                                .to_string(),
                        },
                        "qr" => ProtocolEvent::Qr {
                            url: resp
                                .extra
                                .get("url")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        },
                        "pin" => ProtocolEvent::Pin {
                            pin: resp
                                .extra
                                .get("pin")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        },
                        "listening" => ProtocolEvent::Listening,
                        "message" => {
                            if let Some(msg) = resp.extra.get("message") {
                                match serde_json::from_value(msg.clone()) {
                                    Ok(m) => ProtocolEvent::Message(Box::new(m)),
                                    Err(e) => ProtocolEvent::Error(e.to_string()),
                                }
                            } else {
                                ProtocolEvent::Error("message event missing payload".into())
                            }
                        }
                        "chats" => {
                            let chats = resp
                                .extra
                                .get("chats")
                                .cloned()
                                .and_then(|v| serde_json::from_value(v).ok())
                                .unwrap_or_default();
                            let cached = resp
                                .extra
                                .get("cached")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            ProtocolEvent::Chats { chats, cached }
                        }
                        "messages" => {
                            let chat_mid = resp
                                .extra
                                .get("chatMid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let messages = resp
                                .extra
                                .get("messages")
                                .cloned()
                                .and_then(|v| serde_json::from_value(v).ok())
                                .unwrap_or_default();
                            ProtocolEvent::Messages { chat_mid, messages }
                        }
                        "stickers_updated" => ProtocolEvent::StickersUpdated {
                            result: resp.extra.clone(),
                        },
                        "avatar_ready" => ProtocolEvent::AvatarReady {
                            mid: resp
                                .extra
                                .get("mid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            avatar_path: resp
                                .extra
                                .get("avatarPath")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        },
                        "friends_updated" => {
                            let friends = resp
                                .extra
                                .get("friends")
                                .cloned()
                                .and_then(|v| {
                                    serde_json::from_value::<Vec<crate::protocol::ChatInfo>>(v).ok()
                                })
                                .unwrap_or_default();
                            ProtocolEvent::FriendsUpdated { friends }
                        }
                        "chat_upsert" => {
                            let chat = resp.extra.get("chat").cloned().and_then(|v| {
                                serde_json::from_value::<crate::protocol::ChatInfo>(v).ok()
                            });
                            match chat {
                                Some(chat) => ProtocolEvent::ChatUpsert { chat },
                                None => continue,
                            }
                        }
                        "chat_preview" => ProtocolEvent::ChatPreview {
                            mid: resp
                                .extra
                                .get("mid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            preview: resp
                                .extra
                                .get("preview")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            last_activity: resp
                                .extra
                                .get("lastActivity")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0),
                        },
                        "chat_mute" => ProtocolEvent::ChatMute {
                            mid: resp
                                .extra
                                .get("mid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            muted: resp
                                .extra
                                .get("muted")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        },
                        "media_ready" => ProtocolEvent::MediaReady {
                            chat_mid: resp
                                .extra
                                .get("chatMid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            message_id: resp
                                .extra
                                .get("messageId")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            image_path: resp
                                .extra
                                .get("imagePath")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            audio_path: resp
                                .extra
                                .get("audioPath")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            file_path: resp
                                .extra
                                .get("filePath")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string()),
                        },
                        "media_failed" => ProtocolEvent::MediaFailed {
                            chat_mid: resp
                                .extra
                                .get("chatMid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            message_id: resp
                                .extra
                                .get("messageId")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        },
                        "progress" => ProtocolEvent::Progress {
                            scope: resp
                                .extra
                                .get("scope")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            chat_mid: resp
                                .extra
                                .get("chatMid")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            state: resp
                                .extra
                                .get("state")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            error: resp
                                .extra
                                .get("error")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        },
                        "screen_share_state" => ProtocolEvent::ScreenShareState {
                            state: resp
                                .extra
                                .get("state")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            error: resp
                                .extra
                                .get("error")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        },
                        "upload_progress" => ProtocolEvent::UploadProgress {
                            chat_mid: resp
                                .extra
                                .get("chatMid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            progress: resp
                                .extra
                                .get("progress")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0),
                            label: resp
                                .extra
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            done: resp
                                .extra
                                .get("done")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        },
                        "read_receipt" => ProtocolEvent::ReadReceipt {
                            chat_mid: resp
                                .extra
                                .get("chatMid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            message_id: resp
                                .extra
                                .get("messageId")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        },
                        "call_incoming" => ProtocolEvent::CallIncoming {
                            call_id: resp
                                .extra
                                .get("callId")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            from: resp
                                .extra
                                .get("from")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            kind: resp
                                .extra
                                .get("kind")
                                .and_then(|v| v.as_str())
                                .unwrap_or("AUDIO")
                                .to_string(),
                        },
                        "call_canceled" => ProtocolEvent::CallCanceled {
                            call_id: resp
                                .extra
                                .get("callId")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            from: resp
                                .extra
                                .get("from")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            reason: resp
                                .extra
                                .get("reason")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        },
                        "call_state" => ProtocolEvent::CallState {
                            call_id: resp
                                .extra
                                .get("callId")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            peer: resp
                                .extra
                                .get("peer")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            state: resp
                                .extra
                                .get("state")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            error: resp
                                .extra
                                .get("error")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            video_capable: resp
                                .extra
                                .get("videoCapable")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        },
                        other => ProtocolEvent::Error(format!("unknown event: {other}")),
                    };
                    if tx.send_blocking(ev).is_err() {
                        break;
                    }
                } else if let Some(id) = resp.id {
                    if let Ok(mut pending) = pending_ids.lock() {
                        pending.remove(&id);
                    }
                    let ev = ProtocolEvent::Response {
                        id,
                        ok: resp.ok.unwrap_or(false),
                        result: resp.result.unwrap_or(Value::Null),
                        error: resp.error,
                    };
                    if tx.send_blocking(ev).is_err() {
                        break;
                    }
                }
            }
            Err(e) => {
                eprintln!("[sidecar] bad json ({e}): {line}");
            }
        }
    }
}

fn spawn_timeout_worker(
    rx: std::sync::mpsc::Receiver<(u64, Instant)>,
    pending_ids: Arc<Mutex<HashSet<u64>>>,
    event_tx: Sender<ProtocolEvent>,
) {
    let _ = thread::Builder::new()
        .name("line-sidecar-timeouts".into())
        .spawn(move || {
            let mut deadlines = Vec::<(u64, Instant)>::new();
            loop {
                match rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(item) => deadlines.push(item),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
                while let Ok(item) = rx.try_recv() {
                    deadlines.push(item);
                }
                let now = Instant::now();
                deadlines.retain(|(id, started)| {
                    if now.duration_since(*started) < REQUEST_TIMEOUT {
                        return true;
                    }
                    let timed_out = pending_ids
                        .lock()
                        .map(|mut pending| pending.remove(id))
                        .unwrap_or(false);
                    if timed_out {
                        let _ = event_tx.send_blocking(ProtocolEvent::Response {
                            id: *id,
                            ok: false,
                            result: Value::Null,
                            error: Some("protocol request timed out".into()),
                        });
                    }
                    false
                });
            }
        });
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.stop_process();
    }
}
