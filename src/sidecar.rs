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

fn request_timeout(method: &str) -> Duration {
    if method.starts_with("login_") {
        Duration::from_secs(420)
    } else if matches!(
        method,
        "import_sticker_pack"
            | "import_sticker_pngs"
            | "sticker_owned_pack"
            | "list_stickers"
            | "adopt_sticker_library"
    ) {
        Duration::from_secs(600)
    } else if matches!(method, "preview_sticker_pack" | "prepare_sticker_image") {
        Duration::from_secs(120)
    } else {
        REQUEST_TIMEOUT
    }
}

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
        sidecar.start_process(true)?;
        Ok(sidecar)
    }

    fn start_process(&self, restore_session: bool) -> Result<()> {
        let script = self.repo_root.join("protocol/src/main.ts");
        let compiled = self.repo_root.join("protocol/line-gtk-protocol");
        if !compiled.is_file() && !script.exists() {
            return Err(anyhow!("protocol sidecar missing at {}", script.display()));
        }
        crate::config::ensure_private_dir(&self.data_dir)?;
        let cfg = crate::config::AppConfig::load(&self.data_dir);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let cache_key = crate::credentials::cache_key();
        if cache_key.is_none() {
            tracing::warn!("no Secret Service key; message cache will not be encrypted");
        }

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
                .arg("--allow-run=ffmpeg,ffprobe")
                .arg("--allow-sys")
                .arg("--allow-env=HOME,PATH,DENO_DIR,Q_DEBUG,NODE_DEBUG,NO_COLOR,LINE_GTK_DATA,LINE_GTK_SKIP_RESTORE,LINE_GTK_LANG,LINE_GTK_CACHE_RETENTION,LINE_GTK_CACHE_KEY,LINE_GTK_AUDIO_INPUT,LINE_GTK_AUDIO_OUTPUT,LINE_DEVICE,LINE_VERSION,LINE_CALL_DEVNAME,LINE_CALL_DEVICE_INFO,LINE_CALL_OPUS_SIGNAL,LINE_CALL_DEBUG")
                .arg(&script);
            (command, deno)
        };

        let mut child = command
            .current_dir(self.repo_root.join("protocol"))
            .env("LINE_GTK_DATA", &self.data_dir)
            .env(
                "LINE_GTK_SKIP_RESTORE",
                if restore_session { "0" } else { "1" },
            )
            .env("LINE_GTK_LANG", cfg.language.clone())
            .env("LINE_GTK_CACHE_RETENTION", cfg.cache_retention.clone())
            .env("LINE_GTK_CACHE_KEY", cache_key.unwrap_or_default())
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
                read_loop(
                    stdout,
                    tx.clone(),
                    pending_ids,
                    &current_generation,
                    generation,
                );
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
        self.start_process(false)
    }

    /// 実行中の認証をキャンセルし、保存済みのセッションは維持する。
    pub fn restart_for_login(&self) -> Result<()> {
        self.stop_process();
        while self.events.try_recv().is_ok() {}
        self.start_process(false)
    }

    /// Restart after an unexpected crash while preserving the linked session.
    pub fn recover(&self) -> Result<()> {
        self.stop_process();
        self.start_process(true)
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
        let _ = self
            .timeout_tx
            .send((id, Instant::now() + request_timeout(method)));
        Ok(id)
    }

    pub fn login_qr(&self) -> Result<u64> {
        self.request("login_qr", None)
    }

    pub fn login_email(&self, email: &str, password: &str) -> Result<u64> {
        self.request(
            "login_email",
            Some(json!({ "email": email, "password": password })),
        )
    }

    pub fn list_chats(&self) -> Result<u64> {
        self.request("list_chats", Some(json!({ "force": false })))
    }

    pub fn list_shared_media(
        &self,
        chat_mid: &str,
        category: &str,
        cursor: &str,
        refresh: bool,
    ) -> Result<u64> {
        self.request(
            "list_shared_media",
            Some(
                json!({"chatMid": chat_mid, "category": category, "cursor": cursor,
                "limit": 60, "refresh": refresh}),
            ),
        )
    }

    pub fn download_shared_media(&self, chat_mid: &str, message_id: &str) -> Result<u64> {
        self.request(
            "download_shared_media",
            Some(json!({"chatMid": chat_mid, "messageId": message_id})),
        )
    }

    pub fn stop_shared_media(&self, chat_mid: &str) -> Result<u64> {
        self.request("stop_shared_media", Some(json!({"chatMid": chat_mid})))
    }

    pub fn fetch_messages(&self, chat_mid: &str, limit: u32) -> Result<u64> {
        self.fetch_messages_forced(chat_mid, limit, false)
    }

    pub fn fetch_messages_forced(&self, chat_mid: &str, limit: u32, force: bool) -> Result<u64> {
        self.request(
            "fetch_messages",
            Some(json!({ "chatMid": chat_mid, "limit": limit, "force": force })),
        )
    }

    /// Stored messages of a chat, answered without a login (cache preview).
    pub fn load_cached_messages(&self, chat_mid: &str, limit: u32) -> Result<u64> {
        self.request(
            "load_cached_messages",
            Some(json!({ "chatMid": chat_mid, "limit": limit })),
        )
    }

    /// Refresh a chat that is already on screen without status events.
    pub fn fetch_messages_quiet(&self, chat_mid: &str, limit: u32) -> Result<u64> {
        self.request(
            "fetch_messages",
            Some(json!({ "chatMid": chat_mid, "limit": limit, "force": false, "quiet": true })),
        )
    }

    pub fn fetch_older_messages(
        &self,
        chat_mid: &str,
        before_id: &str,
        before_time: i64,
        limit: u32,
    ) -> Result<u64> {
        self.request(
            "fetch_older_messages",
            Some(json!({
                "chatMid": chat_mid,
                "beforeId": before_id,
                "beforeTime": before_time,
                "limit": limit,
            })),
        )
    }

    pub fn fetch_newer_messages(
        &self,
        chat_mid: &str,
        after_id: &str,
        after_time: i64,
        limit: u32,
    ) -> Result<u64> {
        self.request(
            "fetch_newer_messages",
            Some(json!({
                "chatMid": chat_mid,
                "afterId": after_id,
                "afterTime": after_time,
                "limit": limit,
            })),
        )
    }

    pub fn send_message(&self, chat_mid: &str, text: &str) -> Result<u64> {
        self.send_reply(chat_mid, text, None)
    }

    pub fn send_reply(&self, chat_mid: &str, text: &str, reply_to: Option<&str>) -> Result<u64> {
        self.send_mentions(chat_mid, text, reply_to, &[])
    }

    pub fn send_mentions(
        &self,
        chat_mid: &str,
        text: &str,
        reply_to: Option<&str>,
        mentions: &[crate::protocol::Mention],
    ) -> Result<u64> {
        let mut params = json!({ "chatMid": chat_mid, "text": text, "mentions": mentions });
        if let Some(message_id) = reply_to {
            params["replyToMessageId"] = json!(message_id);
        }
        self.request("send_message", Some(params))
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

    pub fn download_media(
        &self,
        chat_mid: &str,
        message_id: &str,
        content_type: &str,
    ) -> Result<u64> {
        self.request(
            "download_media",
            Some(json!({ "chatMid": chat_mid, "messageId": message_id, "contentType": content_type })),
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

    pub fn mark_read(&self, chat_mid: &str, last_message_id: &str) -> Result<u64> {
        self.request(
            "mark_read",
            Some(json!({ "chatMid": chat_mid, "lastMessageId": last_message_id })),
        )
    }

    pub fn mute_chat(&self, mid: &str, muted: bool) -> Result<u64> {
        self.request("mute_chat", Some(json!({ "mid": mid, "muted": muted })))
    }

    pub fn blocked_contacts(&self) -> Result<u64> {
        self.request("get_blocked_contacts", None)
    }

    pub fn user_profile(&self, mid: &str, force: bool) -> Result<u64> {
        self.request(
            "get_user_profile",
            Some(json!({ "mid": mid, "force": force })),
        )
    }

    pub fn set_contact_blocked(&self, mid: &str, blocked: bool) -> Result<u64> {
        self.request(
            "set_contact_blocked",
            Some(json!({ "mid": mid, "blocked": blocked })),
        )
    }

    pub fn mark_read_once(&self, chat_mid: &str, through: &str) -> Result<u64> {
        self.request(
            "mark_read_once",
            Some(json!({ "chatMid": chat_mid, "lastMessageId": through })),
        )
    }

    pub fn call_start(
        &self,
        mid: &str,
        audio_input: &str,
        audio_output: &str,
        mic_gain: f64,
        spk_gain: f64,
    ) -> Result<u64> {
        self.request(
            "call_start",
            Some(json!({
                "mid": mid,
                "audioInput": if audio_input.is_empty() { "default" } else { audio_input },
                "audioOutput": if audio_output.is_empty() { "default" } else { audio_output },
                "micGain": mic_gain,
                "spkGain": spk_gain,
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

    pub fn poll_get(&self, chat_mid: &str, poll_id: &str) -> Result<u64> {
        self.request(
            "poll_get",
            Some(json!({ "chatMid": chat_mid, "pollId": poll_id })),
        )
    }

    pub fn poll_create(&self, chat_mid: &str, poll: Value) -> Result<u64> {
        self.request(
            "poll_create",
            Some(json!({ "chatMid": chat_mid, "poll": poll })),
        )
    }

    pub fn poll_vote(&self, chat_mid: &str, poll_id: &str, choice_ids: &[String]) -> Result<u64> {
        self.request(
            "poll_vote",
            Some(json!({
                "chatMid": chat_mid,
                "pollId": poll_id,
                "choiceIds": choice_ids,
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
    generation_counter: &AtomicU64,
    generation: u64,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if generation_counter.load(Ordering::SeqCst) != generation {
            break;
        }
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
                            device: resp
                                .extra
                                .get("device")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
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
                        "sticker_import_progress" => ProtocolEvent::StickerImportProgress {
                            request_id: resp
                                .extra
                                .get("requestId")
                                .and_then(Value::as_u64)
                                .unwrap_or_default(),
                            account_mid: resp
                                .extra
                                .get("accountMid")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            done: resp
                                .extra
                                .get("done")
                                .and_then(Value::as_u64)
                                .unwrap_or_default(),
                            total: resp
                                .extra
                                .get("total")
                                .and_then(Value::as_u64)
                                .unwrap_or_default(),
                        },
                        "shared_media_changed" => ProtocolEvent::SharedMediaChanged {
                            account_mid: resp
                                .extra
                                .get("accountMid")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            chat_mid: resp
                                .extra
                                .get("chatMid")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        },
                        "sender_profile" => ProtocolEvent::SenderProfile {
                            mid: resp
                                .extra
                                .get("mid")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            name: resp
                                .extra
                                .get("name")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        },
                        "emoji_ready" => ProtocolEvent::EmojiReady {
                            key: resp
                                .extra
                                .get("key")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .into(),
                            path: resp
                                .extra
                                .get("path")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .into(),
                        },
                        "session_failed" => ProtocolEvent::SessionFailed {
                            error: resp
                                .error
                                .as_deref()
                                .unwrap_or("session restore failed")
                                .to_string(),
                        },
                        "connection_retry" | "connection_lost" | "connection_failed" => {
                            ProtocolEvent::Connection {
                                state: event.to_string(),
                                kind: resp
                                    .extra
                                    .get("kind")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("network")
                                    .to_string(),
                                attempt: resp
                                    .extra
                                    .get("attempt")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0),
                            }
                        }
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
                        "message_unsent" => {
                            let field = |key| {
                                resp.extra
                                    .get(key)
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_string()
                            };
                            ProtocolEvent::MessageUnsent {
                                account_mid: field("accountMid"),
                                chat_mid: field("chatMid"),
                                message_id: field("messageId"),
                            }
                        }
                        "message_cache_limit" => {
                            let number =
                                |key| resp.extra.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
                            ProtocolEvent::MessageCacheLimit {
                                chat_mid: resp
                                    .extra
                                    .get("chatMid")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                                count: number("count"),
                                limit: number("limit"),
                                full: resp.extra.get("full").and_then(|v| v.as_bool())
                                    == Some(true),
                            }
                        }
                        "message_reaction" => {
                            let field = |key| {
                                resp.extra
                                    .get(key)
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_string()
                            };
                            ProtocolEvent::MessageReaction {
                                account_mid: field("accountMid"),
                                chat_mid: field("chatMid"),
                                message_id: field("messageId"),
                                from: field("from"),
                                reaction: resp
                                    .extra
                                    .get("reaction")
                                    .filter(|value| !value.is_null())
                                    .and_then(|value| serde_json::from_value(value.clone()).ok()),
                            }
                        }
                        "chat_event" => {
                            if let Some(message) = resp.extra.get("message") {
                                match serde_json::from_value(message.clone()) {
                                    Ok(message) => ProtocolEvent::ChatEvent {
                                        account_mid: resp
                                            .extra
                                            .get("accountMid")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or_default()
                                            .to_string(),
                                        message,
                                    },
                                    Err(error) => ProtocolEvent::Error(error.to_string()),
                                }
                            } else {
                                ProtocolEvent::Error("グループイベントの内容がありません".into())
                            }
                        }
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
                        "cached_boot" => {
                            let text = |value: Option<&serde_json::Value>| {
                                value.and_then(|v| v.as_str()).map(|s| s.to_string())
                            };
                            let profile = resp.extra.get("profile");
                            let field = |name: &str| text(profile.and_then(|p| p.get(name)));
                            ProtocolEvent::CachedBoot {
                                mid: text(resp.extra.get("accountMid")).unwrap_or_default(),
                                display_name: field("displayName").unwrap_or_default(),
                                status_message: field("statusMessage").unwrap_or_default(),
                                picture_path: field("picturePath"),
                                avatar_path: field("avatarPath"),
                                picture_url: field("pictureUrl"),
                                chats: resp
                                    .extra
                                    .get("chats")
                                    .cloned()
                                    .and_then(|v| serde_json::from_value(v).ok())
                                    .unwrap_or_default(),
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
                            contact_only: resp.extra.get("contactOnly").and_then(|v| v.as_bool()),
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
                        "download_progress" => ProtocolEvent::DownloadProgress {
                            message_id: resp
                                .extra
                                .get("messageId")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            progress: resp
                                .extra
                                .get("progress")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0),
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
                        "contact_blocks" => {
                            let Some(contacts) = resp
                                .extra
                                .get("contacts")
                                .cloned()
                                .and_then(|v| serde_json::from_value(v).ok())
                            else {
                                continue;
                            };
                            ProtocolEvent::ContactBlocks {
                                account_mid: resp
                                    .extra
                                    .get("accountMid")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                                contacts,
                            }
                        }
                        "chat_unread" => ProtocolEvent::ChatUnread {
                            chat_mid: resp
                                .extra
                                .get("chatMid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            account_mid: resp
                                .extra
                                .get("accountMid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            through_message_id: resp
                                .extra
                                .get("throughMessageId")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            unread: resp
                                .extra
                                .get("unread")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(-1),
                        },
                        "read_receipt" => ProtocolEvent::ReadReceipt {
                            chat_mid: resp
                                .extra
                                .get("chatMid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            user_mid: resp
                                .extra
                                .get("userMid")
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
                        "read_ranges" => ProtocolEvent::ReadRanges {
                            chat_mid: resp
                                .extra
                                .get("chatMid")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            readers: resp
                                .extra
                                .get("readers")
                                .cloned()
                                .and_then(|value| serde_json::from_value(value).ok())
                                .unwrap_or_default(),
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
                deadlines.retain(|(id, deadline)| {
                    if now < *deadline {
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

#[cfg(test)]
mod authentication_tests {
    use super::*;

    #[test]
    fn session_failure_preserves_the_authentication_reason() {
        let (tx, rx) = async_channel::unbounded();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let generation = AtomicU64::new(1);
        let event = b"{\"event\":\"session_failed\",\"error\":\"NOT_AUTHORIZED_DEVICE\"}\n";
        read_loop(&event[..], tx, pending, &generation, 1);
        assert!(
            matches!(rx.try_recv().unwrap(),ProtocolEvent::SessionFailed {error} if error=="NOT_AUTHORIZED_DEVICE")
        );
    }

    #[test]
    fn authentication_allows_mobile_verification_time() {
        assert_eq!(request_timeout("ping"), Duration::from_secs(30));
        assert!(request_timeout("login_email") > Duration::from_secs(360));
        assert!(request_timeout("login_qr") > Duration::from_secs(360));
    }

    #[test]
    fn cancelled_process_cannot_deliver_a_late_pin() {
        let (tx, rx) = async_channel::unbounded();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let generation = AtomicU64::new(2);
        let pin = b"{\"event\":\"pin\",\"pin\":\"123456\"}\n";
        read_loop(&pin[..], tx.clone(), pending.clone(), &generation, 1);
        assert!(rx.try_recv().is_err());
        read_loop(&pin[..], tx, pending, &generation, 2);
        assert!(matches!(rx.try_recv().unwrap(), ProtocolEvent::Pin { pin } if pin == "123456"));
    }
    #[test]
    fn read_events_preserve_reader_and_range_bounds() {
        let (tx, rx) = async_channel::unbounded();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let generation = AtomicU64::new(1);
        let events = concat!(
            "{\"event\":\"read_receipt\",\"chatMid\":\"c-group\",\"userMid\":\"u-one\",\"messageId\":\"9007199254740993\"}\n",
            "{\"event\":\"read_ranges\",\"chatMid\":\"c-group\",\"readers\":[{\"userMid\":\"u-two\",\"ranges\":[{\"startMessageId\":\"9007199254740992\",\"endMessageId\":\"9007199254740993\"}]}]}\n"
        );
        read_loop(events.as_bytes(), tx, pending, &generation, 1);
        assert!(
            matches!(rx.try_recv().unwrap(), ProtocolEvent::ReadReceipt { user_mid, message_id, .. } if user_mid == "u-one" && message_id == "9007199254740993")
        );
        match rx.try_recv().unwrap() {
            ProtocolEvent::ReadRanges { readers, .. } => {
                assert_eq!(readers[0].user_mid, "u-two");
                assert_eq!(readers[0].ranges[0].start_message_id, "9007199254740992");
                assert_eq!(readers[0].ranges[0].end_message_id, "9007199254740993");
            }
            _ => panic!("既読範囲のイベントではありません"),
        }
    }

    #[test]
    fn emoji_ready_keeps_the_asset_key_and_cache_path() {
        let (tx, rx) = async_channel::unbounded();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let generation = AtomicU64::new(1);
        let event = b"{\"event\":\"emoji_ready\",\"key\":\"5ac1bfd5040ab15980c9b435_001\",\"path\":\"/cache/emoji.png\"}\n";
        read_loop(&event[..], tx, pending, &generation, 1);
        assert!(
            matches!(rx.try_recv().unwrap(), ProtocolEvent::EmojiReady { key, path }
            if key == "5ac1bfd5040ab15980c9b435_001" && path == "/cache/emoji.png")
        );
    }

    #[test]
    fn reaction_events_keep_the_reaction_or_its_removal() {
        let (tx, rx) = async_channel::unbounded();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let generation = AtomicU64::new(1);
        let events = concat!(
            "{\"event\":\"message_reaction\",\"accountMid\":\"u-self\",\"chatMid\":\"c-group\",\"messageId\":\"9007199254740993\",\"from\":\"u-one\",\"reaction\":{\"from\":\"u-one\",\"at\":5,\"kind\":\"EMOJI\",\"emoji\":\"5ac1bfd5040ab15980c9b435_001\",\"resourceType\":2}}\n",
            "{\"event\":\"message_reaction\",\"accountMid\":\"u-self\",\"chatMid\":\"c-group\",\"messageId\":\"9007199254740993\",\"from\":\"u-one\",\"reaction\":null}\n"
        );
        read_loop(events.as_bytes(), tx, pending, &generation, 1);
        match rx.try_recv().unwrap() {
            ProtocolEvent::MessageReaction {
                message_id,
                from,
                reaction: Some(reaction),
                ..
            } => {
                assert_eq!(message_id, "9007199254740993");
                assert_eq!(from, "u-one");
                assert_eq!(reaction.kind, "EMOJI");
                assert_eq!(
                    reaction.emoji.as_deref(),
                    Some("5ac1bfd5040ab15980c9b435_001")
                );
                assert_eq!(reaction.resource_type, Some(2));
            }
            _ => panic!("リアクションのイベントではありません"),
        }
        assert!(matches!(
            rx.try_recv().unwrap(),
            ProtocolEvent::MessageReaction { reaction: None, .. }
        ));
    }

    #[test]
    fn shared_media_changes_keep_the_account_and_conversation() {
        let (tx, rx) = async_channel::unbounded();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let generation = AtomicU64::new(1);
        let event = b"{\"event\":\"shared_media_changed\",\"accountMid\":\"u-self\",\"chatMid\":\"c-group\"}\n";
        read_loop(&event[..], tx, pending, &generation, 1);
        assert!(
            matches!(rx.try_recv().unwrap(), ProtocolEvent::SharedMediaChanged {
            account_mid, chat_mid
        } if account_mid == "u-self" && chat_mid == "c-group")
        );
    }

    #[test]
    fn sticker_progress_keeps_the_request_and_account() {
        let (tx, rx) = async_channel::unbounded();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let generation = AtomicU64::new(1);
        let event = b"{\"event\":\"sticker_import_progress\",\"requestId\":42,\"accountMid\":\"u-one\",\"done\":3,\"total\":40}\n";
        read_loop(event.as_slice(), tx, pending, &generation, 1);
        assert!(
            matches!(rx.try_recv().unwrap(), ProtocolEvent::StickerImportProgress { request_id: 42, account_mid, done: 3, total: 40 } if account_mid == "u-one")
        );
        assert!(request_timeout("import_sticker_pack") >= Duration::from_secs(600));
    }

    #[test]
    fn sender_profiles_preserve_names_without_becoming_messages() {
        let (tx, rx) = async_channel::unbounded();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let generation = AtomicU64::new(1);
        let event =
            "{\"event\":\"sender_profile\",\"mid\":\"u-member\",\"name\":\"グループのメンバー\"}\n";
        read_loop(event.as_bytes(), tx, pending, &generation, 1);
        assert!(
            matches!(rx.try_recv().unwrap(), ProtocolEvent::SenderProfile { mid, name } if mid == "u-member" && name == "グループのメンバー")
        );
        assert!(rx.try_recv().is_err());
    }
}
