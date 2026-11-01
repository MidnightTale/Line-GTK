use super::*;

pub(super) fn open_sticker_picker(state: &AppState) {
    sticker_library::open(state);
}

pub(super) fn send_sticker_now(
    state: &AppState,
    sticker_id: &str,
    package_id: &str,
    version: Option<&str>,
    image_path: Option<&str>,
) {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    match state
        .sidecar
        .send_sticker(&chat_mid, sticker_id, package_id, version)
    {
        Ok(id) => {
            begin_optimistic_send(
                state,
                id,
                &chat_mid,
                MessageInfo {
                    chat_event: None,
                    emojis: Vec::new(),
                    mentions: Vec::new(),
                    reactions: Vec::new(),
                    id: String::new(),
                    text: "[Sticker]".into(),
                    from: String::new(),
                    to: chat_mid.clone(),
                    mine: true,
                    created_time: now_ms(),
                    content_type: "STICKER".into(),
                    image_path: image_path.map(|p| p.to_string()),
                    audio_path: None,
                    file_name: None,
                    file_path: None,
                    duration_ms: None,
                    flex: None,
                    call_history: None,
                    decryption_failed: false,
                    reply_to: None,
                },
            );
            dismiss_new_marker(state);
            pin_messages_to_latest(state);
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("sticker_send_failed", &[("error", &e.to_string())]),
        ),
    }
}

pub(super) fn pick_and_send_media(state: &AppState) {
    let Some(_chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }

    let filter_all = gtk::FileFilter::new();
    filter_all.set_name(Some("Media & files"));
    filter_all.add_mime_type("image/*");
    filter_all.add_mime_type("video/*");
    filter_all.add_mime_type("audio/*");
    filter_all.add_pattern("*");

    let filter_img = gtk::FileFilter::new();
    filter_img.set_name(Some("Images"));
    filter_img.add_mime_type("image/*");

    let filter_vid = gtk::FileFilter::new();
    filter_vid.set_name(Some("Videos"));
    filter_vid.add_mime_type("video/*");

    let filter_aud = gtk::FileFilter::new();
    filter_aud.set_name(Some("Audio"));
    filter_aud.add_mime_type("audio/*");

    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter_all);
    filters.append(&filter_img);
    filters.append(&filter_vid);
    filters.append(&filter_aud);

    let dialog = gtk::FileDialog::builder()
        .title(crate::i18n::t("attach_file"))
        .modal(true)
        .filters(&filters)
        .default_filter(&filter_all)
        .build();

    let s = state.clone();
    dialog.open(
        Some(&state.window),
        None::<&gio::Cancellable>,
        move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file.path() else {
                toast(
                    &s,
                    &crate::i18n::tf("media_send_failed", &[("error", "no path")]),
                );
                return;
            };
            send_local_media_path(&s, path);
        },
    );
}

/// Ctrl+V attachment: files (URI list / FileList) or a clipboard image.
/// Returns true when paste was handled so text paste is suppressed.
pub(super) fn try_paste_clipboard_attachment(state: &AppState) -> bool {
    if state.current_chat.borrow().is_none() {
        return false;
    }
    if !*state.session_ready.borrow() {
        return false;
    }

    let scope = composer_drafts::scope(state);
    let clipboard = state.composer.clipboard();
    let formats = clipboard.formats();
    let has_files = formats.contains_type(gdk::FileList::static_type())
        || formats.contain_mime_type("text/uri-list");
    let has_image = formats.contains_type(gdk::Texture::static_type())
        || formats.contain_mime_type("image/png")
        || formats.contain_mime_type("image/jpeg")
        || formats.contain_mime_type("image/bmp")
        || formats.contain_mime_type("image/tiff");

    if has_files {
        let s = state.clone();
        clipboard.read_value_async(
            gdk::FileList::static_type(),
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |res| match res {
                Ok(value) => {
                    if let Ok(list) = value.get::<gdk::FileList>() {
                        let mut any = false;
                        for file in list.files() {
                            if let Some(path) = file.path()
                                && path.is_file()
                            {
                                composer_drafts::stage(&s, &scope, path);
                                any = true;
                            }
                        }
                        if !any {
                            paste_clipboard_uri_list(&s, &scope);
                        }
                    } else {
                        paste_clipboard_uri_list(&s, &scope);
                    }
                }
                Err(_) => paste_clipboard_uri_list(&s, &scope),
            },
        );
        return true;
    }

    if has_image {
        let s = state.clone();
        clipboard.read_texture_async(None::<&gio::Cancellable>, move |res| match res {
            Ok(Some(tex)) => match save_clipboard_texture_png(&s, &tex) {
                Ok(path) => composer_drafts::stage(&s, &scope, path),
                Err(e) => toast(
                    &s,
                    &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
                ),
            },
            _ => paste_clipboard_image_bytes(&s, &scope),
        });
        return true;
    }

    false
}

fn paste_clipboard_uri_list(state: &AppState, scope: &composer_drafts::Scope) {
    let scope = scope.clone();
    let clipboard = state.composer.clipboard();
    let s = state.clone();
    clipboard.read_async(
        &["text/uri-list"],
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |res| {
            let Ok((stream, _)) = res else {
                return;
            };
            use std::io::Read;
            let mut buf = String::new();
            let mut input = gio::InputStream::into_read(stream);
            if input.read_to_string(&mut buf).is_err() {
                return;
            }
            for line in buf.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let file = gio::File::for_uri(line);
                if let Some(path) = file.path()
                    && path.is_file()
                {
                    composer_drafts::stage(&s, &scope, path);
                }
            }
        },
    );
}

fn paste_clipboard_image_bytes(state: &AppState, scope: &composer_drafts::Scope) {
    let scope = scope.clone();
    let clipboard = state.composer.clipboard();
    let s = state.clone();
    clipboard.read_async(
        &["image/png", "image/jpeg", "image/bmp", "image/tiff"],
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |res| {
            let Ok((stream, mime)) = res else {
                return;
            };
            use std::io::Read;
            let mut bytes = Vec::new();
            let mut input = gio::InputStream::into_read(stream);
            if input.read_to_end(&mut bytes).is_err() || bytes.is_empty() {
                return;
            }
            let ext = if mime.as_str().contains("jpeg") {
                "jpg"
            } else if mime.as_str().contains("bmp") {
                "bmp"
            } else if mime.as_str().contains("tiff") {
                "tiff"
            } else {
                "png"
            };
            match write_clipboard_bytes(&s, &bytes, ext) {
                Ok(path) => composer_drafts::stage(&s, &scope, path),
                Err(e) => toast(
                    &s,
                    &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
                ),
            }
        },
    );
}

pub(super) fn save_clipboard_texture_png(
    state: &AppState,
    tex: &gdk::Texture,
) -> anyhow::Result<PathBuf> {
    use gdk::prelude::TextureExt;
    let dir = state.data_dir.join("cache/media");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!(
        "paste-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    tex.save_to_png(&dest)?;
    Ok(dest)
}

pub(super) fn write_clipboard_bytes(
    state: &AppState,
    bytes: &[u8],
    ext: &str,
) -> anyhow::Result<PathBuf> {
    let dir = state.data_dir.join("cache/media");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!(
        "paste-{}.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        ext
    ));
    std::fs::write(&dest, bytes)?;
    Ok(dest)
}

pub(super) fn send_local_media_path(state: &AppState, path: PathBuf) {
    try_send_local_media_path(state, path);
}

pub(super) fn try_send_local_media_path(state: &AppState, path: PathBuf) -> bool {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return false;
    };
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return false;
    }
    if !path.is_file() {
        toast(
            state,
            &crate::i18n::tf("media_send_failed", &[("error", "not a file")]),
        );
        return false;
    }

    let o_type = guess_media_o_type(&path);
    let duration_ms = if o_type == "audio" || o_type == "video" {
        ffprobe_duration_ms(&path)
    } else {
        None
    };
    let cached = match copy_into_media_cache(state, &path) {
        Ok(p) => p,
        Err(e) => {
            toast(
                state,
                &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
            );
            return false;
        }
    };
    let path_str = cached.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    match state
        .sidecar
        .send_media(&chat_mid, &path_str, o_type, duration_ms)
    {
        Ok(id) => {
            let (content_type, text, image_path, audio_path, file_path) = match o_type {
                "image" | "gif" => (
                    "IMAGE",
                    "[Image]".to_string(),
                    Some(path_str.clone()),
                    None,
                    Some(path_str.clone()),
                ),
                "video" => (
                    "VIDEO",
                    "[Video]".to_string(),
                    None,
                    None,
                    Some(path_str.clone()),
                ),
                "audio" => (
                    "AUDIO",
                    "Voice message".to_string(),
                    None,
                    Some(path_str.clone()),
                    Some(path_str.clone()),
                ),
                _ => (
                    "FILE",
                    file_name.clone(),
                    None,
                    None,
                    Some(path_str.clone()),
                ),
            };
            begin_optimistic_send(
                state,
                id,
                &chat_mid,
                MessageInfo {
                    chat_event: None,
                    emojis: Vec::new(),
                    mentions: Vec::new(),
                    reactions: Vec::new(),
                    id: String::new(),
                    text,
                    from: String::new(),
                    to: chat_mid.clone(),
                    mine: true,
                    created_time: now_ms(),
                    content_type: content_type.into(),
                    image_path,
                    audio_path,
                    file_name: Some(file_name),
                    file_path,
                    duration_ms: duration_ms.map(|v| v as i64),
                    flex: None,
                    call_history: None,
                    decryption_failed: false,
                    reply_to: None,
                },
            );
            show_upload_progress(state, 0.02, &crate::i18n::t("media_uploading"));
            dismiss_new_marker(state);
            pin_messages_to_latest(state);
            true
        }
        Err(e) => {
            toast(
                state,
                &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
            );
            false
        }
    }
}

pub(super) fn start_voice_record(state: &AppState) {
    let Some(_chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    if state.recording.borrow().is_some() {
        return;
    }

    let wav = state.data_dir.join("cache/media/voice-out.wav");
    let _ = std::fs::create_dir_all(wav.parent().unwrap_or(std::path::Path::new(".")));
    let _ = std::fs::remove_file(&wav);
    let source = {
        let s = state.config.borrow().audio_input.clone();
        if s.is_empty() || s == "default" {
            "default".to_string()
        } else {
            s
        }
    };
    let child = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "pulse",
            "-i",
            &source,
            "-ac",
            "1",
            "-ar",
            "48000",
            "-t",
            "60",
            wav.to_str().unwrap_or("/tmp/voice-out.wav"),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match child {
        Ok(c) => {
            *state.recording.borrow_mut() = Some(c);
            *state.recording_started.borrow_mut() = Some(std::time::Instant::now());
            let bars = spectrum_bar_count(state.record_wave.width().max(320) as f64);
            *state.recording_levels.borrow_mut() = vec![0.12; bars];
            state.record_timer.set_text("0:00");
            state.composer_stack.set_visible_child_name("record");
            state.record_wave.queue_draw();
            state.status.set_text(&crate::i18n::t("voice_recording"));
            start_record_tick(state);
        }
        Err(_) => {
            toast(state, &crate::i18n::t("voice_record_failed"));
        }
    }
}

pub(super) fn stop_record_tick(state: &AppState) {
    if let Some(id) = state.recording_tick.borrow_mut().take() {
        id.remove();
    }
}

pub(super) fn start_record_tick(state: &AppState) {
    stop_record_tick(state);
    let s = state.clone();
    let id = glib::timeout_add_local(std::time::Duration::from_millis(80), move || {
        if s.recording.borrow().is_none() {
            *s.recording_tick.borrow_mut() = None;
            return glib::ControlFlow::Break;
        }
        let elapsed = s
            .recording_started
            .borrow()
            .map(|t| t.elapsed())
            .unwrap_or_default();
        let total = elapsed.as_secs().min(60);
        s.record_timer
            .set_text(&format!("{}:{:02}", total / 60, total % 60));

        let wav = s.data_dir.join("cache/media/voice-out.wav");
        let level = wav_tail_level(&wav);
        {
            let target = spectrum_bar_count(s.record_wave.width().max(1) as f64);
            let mut levels = s.recording_levels.borrow_mut();
            let len = levels.len();
            if len < target {
                levels.splice(0..0, std::iter::repeat_n(0.12, target - len));
            } else if len > target {
                levels.drain(0..(len - target));
            }
            if !levels.is_empty() {
                levels.remove(0);
                levels.push(level);
            }
        }
        s.record_wave.queue_draw();

        // Auto-stop & send at 60s like LINE.
        if elapsed.as_secs() >= 60 {
            finish_voice_record(&s, true);
            *s.recording_tick.borrow_mut() = None;
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    *state.recording_tick.borrow_mut() = Some(id);
}

pub(super) fn stop_ffmpeg_recording(state: &AppState) -> Option<std::time::Instant> {
    stop_record_tick(state);
    let mut child = state.recording.borrow_mut().take();
    let started = state.recording_started.borrow_mut().take();
    if let Some(ref mut c) = child {
        if let Some(mut stdin) = c.stdin.take() {
            use std::io::Write;
            let _ = writeln!(stdin, "q");
            let _ = stdin.flush();
        }
        let _ = c.wait();
    }
    state.composer_stack.set_visible_child_name("compose");
    started
}

/// `send == true` remuxes and uploads; `false` cancels and discards.
pub(super) fn finish_voice_record(state: &AppState, send: bool) {
    if state.recording.borrow().is_none() && state.recording_started.borrow().is_none() {
        state.composer_stack.set_visible_child_name("compose");
        return;
    }
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        let _ = stop_ffmpeg_recording(state);
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };

    let started = stop_ffmpeg_recording(state);
    let wav = state.data_dir.join("cache/media/voice-out.wav");
    let m4a = state.data_dir.join("cache/media/voice-out.m4a");

    if !send {
        let _ = std::fs::remove_file(&wav);
        let _ = std::fs::remove_file(&m4a);
        state.status.set_text(&crate::i18n::t("voice_cancelled"));
        return;
    }

    let _ = std::fs::remove_file(&m4a);
    let remux = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            wav.to_str().unwrap_or(""),
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            m4a.to_str().unwrap_or(""),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let ok_file = remux.map(|s| s.success()).unwrap_or(false)
        && m4a.metadata().map(|m| m.len() >= 1024).unwrap_or(false);
    if !ok_file {
        toast(state, &crate::i18n::t("voice_record_failed"));
        return;
    }
    let duration_ms = started
        .map(|t| t.elapsed().as_millis() as u64)
        .unwrap_or(2000)
        .max(500);
    match state
        .sidecar
        .send_audio(&chat_mid, m4a.to_str().unwrap_or(""), Some(duration_ms))
    {
        Ok(id) => {
            let path_str = m4a.to_string_lossy().to_string();
            begin_optimistic_send(
                state,
                id,
                &chat_mid,
                MessageInfo {
                    chat_event: None,
                    emojis: Vec::new(),
                    mentions: Vec::new(),
                    reactions: Vec::new(),
                    id: String::new(),
                    text: "Voice message".into(),
                    from: String::new(),
                    to: chat_mid.clone(),
                    mine: true,
                    created_time: now_ms(),
                    content_type: "AUDIO".into(),
                    image_path: None,
                    audio_path: Some(path_str.clone()),
                    file_name: None,
                    file_path: Some(path_str),
                    duration_ms: Some(duration_ms as i64),
                    flex: None,
                    call_history: None,
                    decryption_failed: false,
                    reply_to: None,
                },
            );
            show_upload_progress(state, 0.02, &crate::i18n::t("media_uploading"));
            dismiss_new_marker(state);
            pin_messages_to_latest(state);
            state.status.set_text(&crate::i18n::t("voice_sending"));
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("send_failed", &[("error", &e.to_string())]),
        ),
    }
}
