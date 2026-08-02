use super::*;

pub(super) fn suggest_media_name(msg: &MessageInfo) -> String {
    if let Some(n) = msg.file_name.as_deref().filter(|s| !s.is_empty()) {
        return n.to_string();
    }
    let ct = msg.content_type.to_ascii_uppercase();
    match ct.as_str() {
        "IMAGE" => format!("{}.jpg", msg.id),
        "VIDEO" => format!("{}.mp4", msg.id),
        "AUDIO" => format!("{}.m4a", msg.id),
        "FILE" if !msg.text.is_empty() && msg.text != "[File]" => msg.text.clone(),
        _ => format!("line-{}", msg.id),
    }
}

pub(super) fn is_thumb_media_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains(".thumb.") || lower.ends_with(".thumb.jpg") || lower.ends_with(".thumb.png")
}

pub(super) fn is_full_media_path(path: &str) -> bool {
    path.to_ascii_lowercase().contains(".full.")
}

pub(super) fn image_looks_low_res(path: &str) -> bool {
    if is_full_media_path(path) {
        return false;
    }
    if is_thumb_media_path(path) {
        return true;
    }
    match gdk_pixbuf::Pixbuf::file_info(path) {
        Some((_, width, height)) => width <= 512 && height <= 512,
        None => false,
    }
}

pub(super) fn full_image_candidate(path: &str) -> Option<String> {
    if is_full_media_path(path) && std::path::Path::new(path).exists() {
        return Some(path.to_string());
    }
    if !is_thumb_media_path(path)
        && std::path::Path::new(path).exists()
        && !image_looks_low_res(path)
    {
        return Some(path.to_string());
    }
    // Try sibling .full. next to a polluted preview or thumb.
    let p = std::path::Path::new(path);
    let parent = p.parent()?;
    let name = p.file_name()?.to_str()?;
    let id = name.split('.').next().filter(|s| !s.is_empty())?;
    for ext in ["jpg", "jpeg", "png", "webp", "gif"] {
        let cand = parent.join(format!("{id}.full.{ext}"));
        if cand.is_file() {
            return Some(cand.to_string_lossy().to_string());
        }
    }
    if is_thumb_media_path(path) {
        for ext in ["jpg", "jpeg", "png", "webp", "gif"] {
            let cand = parent.join(format!("{id}.{ext}"));
            if cand.is_file() && !image_looks_low_res(&cand.to_string_lossy()) {
                return Some(cand.to_string_lossy().to_string());
            }
        }
    }
    None
}

pub(super) fn local_media_path(msg: &MessageInfo, for_viewer: bool) -> Option<String> {
    let ct = msg.content_type.to_ascii_uppercase();
    if ct == "AUDIO" {
        return msg
            .audio_path
            .as_ref()
            .or(msg.file_path.as_ref())
            .filter(|p| std::path::Path::new(p).exists())
            .cloned();
    }
    if ct == "VIDEO" {
        if let Some(p) = msg.file_path.as_ref().filter(|p| {
            std::path::Path::new(p).exists() && p.to_ascii_lowercase().ends_with(".mp4")
        }) {
            return Some(p.clone());
        }
        // Viewer needs full video; thumb alone is not enough.
        if for_viewer {
            return None;
        }
    }
    if let Some(p) = msg
        .file_path
        .as_ref()
        .filter(|p| std::path::Path::new(p).exists() && !is_thumb_media_path(p))
        && !(for_viewer && ct == "IMAGE" && image_looks_low_res(p))
    {
        return Some(p.clone());
    }
    if ct == "IMAGE" {
        if for_viewer {
            // Never open the chat thumbnail / OBS preview in the viewer.
            if let Some(p) = msg.file_path.as_ref().and_then(|p| full_image_candidate(p)) {
                return Some(p);
            }
            if let Some(p) = msg
                .image_path
                .as_ref()
                .and_then(|p| full_image_candidate(p))
            {
                return Some(p);
            }
            return None;
        }
        if let Some(p) = msg
            .image_path
            .as_ref()
            .filter(|p| std::path::Path::new(p).exists())
        {
            return Some(p.clone());
        }
    } else if !for_viewer
        && let Some(p) = msg
            .image_path
            .as_ref()
            .filter(|p| std::path::Path::new(p).exists())
    {
        return Some(p.clone());
    }
    None
}

pub(super) fn request_media_download(state: &AppState, msg: &MessageInfo, action: &str) {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if msg.id.is_empty() {
        toast(state, &crate::i18n::t("media_download_failed"));
        return;
    }
    // Prefer freshest paths from hydrate / download cache.
    let msg = state
        .media_msgs
        .borrow()
        .get(&msg.id)
        .cloned()
        .unwrap_or_else(|| msg.clone());
    let for_viewer = action == "open_viewer";
    if let Some(path) = local_media_path(&msg, for_viewer) {
        finish_media_action(
            state,
            action,
            &path,
            &suggest_media_name(&msg),
            &msg.content_type,
        );
        return;
    }
    state.status.set_text(&crate::i18n::t("media_downloading"));
    match state.sidecar.download_media(&chat_mid, &msg.id) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::DownloadMedia {
                    message_id: msg.id.clone(),
                    action: action.to_string(),
                    content_type: msg.content_type.clone(),
                    suggest_name: suggest_media_name(&msg),
                },
            );
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("media_download_failed_err", &[("error", &e.to_string())]),
        ),
    }
}

pub(super) fn finish_media_action(
    state: &AppState,
    action: &str,
    path: &str,
    suggest_name: &str,
    content_type: &str,
) {
    match action {
        "open_viewer" => open_media_viewer(state, path, content_type, suggest_name),
        "save_dialog" => save_media_as(state, path, suggest_name, content_type),
        "play_audio" => {
            if let Err(e) = play_audio_file(path, &state.config.borrow().audio_output) {
                toast(
                    state,
                    &crate::i18n::tf("voice_play_failed", &[("error", &e)]),
                );
            }
        }
        _ => save_media_as(state, path, suggest_name, content_type),
    }
}

pub(super) fn copy_media_to_dest(state: &AppState, src: &str, dest: &std::path::Path) {
    match std::fs::copy(src, dest) {
        Ok(_) => {
            toast(
                state,
                &crate::i18n::tf("media_saved", &[("path", &dest.display().to_string())]),
            );
            state.status.set_text(&crate::i18n::t("media_saved_status"));
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("media_download_failed_err", &[("error", &e.to_string())]),
        ),
    }
}

pub(super) fn save_media_as(state: &AppState, path: &str, suggest_name: &str, content_type: &str) {
    let cfg = state.config.borrow().clone();
    let dest_dir = cfg.download_dir_for(content_type);
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        toast(
            state,
            &crate::i18n::tf("media_download_failed_err", &[("error", &e.to_string())]),
        );
        return;
    }

    if !cfg.download_ask_every_time {
        let dest = downloads::unique_download_dest(&dest_dir, suggest_name);
        copy_media_to_dest(state, path, &dest);
        return;
    }

    let filter = gtk::FileFilter::new();
    filter.set_name(Some("All files"));
    filter.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(crate::i18n::t("media_save_as"))
        .modal(true)
        .initial_name(suggest_name)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    dialog.set_initial_folder(Some(&gio::File::for_path(&dest_dir)));
    let s = state.clone();
    let src = path.to_string();
    dialog.save(
        Some(&state.window),
        None::<&gio::Cancellable>,
        move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(dest) = file.path() else {
                toast(&s, &crate::i18n::t("media_download_failed"));
                return;
            };
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            copy_media_to_dest(&s, &src, &dest);
        },
    );
}

pub(super) fn wire_media_open_click(
    state: &AppState,
    pic: &gtk::Picture,
    msg: &MessageInfo,
    _kind: &str,
) {
    wire_media_open_click_widget(state, pic.upcast_ref::<gtk::Widget>(), msg);
}

#[derive(Clone)]
struct ViewerMediaItem {
    path: String,
    content_type: String,
    name: String,
}

fn normalized_media_path(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

fn image_gallery_items(
    state: &AppState,
    current: ViewerMediaItem,
) -> (Vec<ViewerMediaItem>, usize) {
    let mut messages: Vec<MessageInfo> = state
        .media_msgs
        .borrow()
        .values()
        .filter(|message| message.content_type.eq_ignore_ascii_case("image"))
        .cloned()
        .collect();
    messages.sort_by_key(|message| message.created_time);

    let mut items = Vec::<ViewerMediaItem>::new();
    let mut positions = HashMap::<PathBuf, usize>::new();
    let mut current_index = None;
    let current_path = normalized_media_path(&current.path);
    for message in messages {
        let Some(path) = local_media_path(&message, true) else {
            continue;
        };
        let normalized = normalized_media_path(&path);
        if let Some(index) = positions.get(&normalized).copied() {
            if normalized == current_path {
                current_index = Some(index);
            }
            continue;
        }
        let index = items.len();
        positions.insert(normalized.clone(), index);
        if normalized == current_path {
            current_index = Some(index);
        }
        let name = suggest_media_name(&message);
        items.push(ViewerMediaItem {
            path,
            content_type: message.content_type,
            name,
        });
    }

    let index = current_index.unwrap_or_else(|| {
        let index = items.len();
        items.push(current);
        index
    });
    (items, index)
}

fn adjacent_image_index(current: usize, length: usize, delta: isize) -> Option<usize> {
    let next = current.checked_add_signed(delta)?;
    (next < length).then_some(next)
}

fn clear_viewer_box(box_: &gtk::Box) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_image_gallery_item(
    state: &AppState,
    win: &gtk::Window,
    tools: &gtk::Box,
    body: &gtk::Box,
    title: &gtk::Label,
    previous: &gtk::Button,
    next: &gtk::Button,
    items: &[ViewerMediaItem],
    index: usize,
    current_item: &Rc<RefCell<ViewerMediaItem>>,
) {
    let Some(item) = items.get(index).cloned() else {
        return;
    };
    clear_viewer_box(tools);
    clear_viewer_box(body);
    if items.len() > 1 {
        title.set_label(&format!("{} · {}/{}", item.name, index + 1, items.len()));
    } else {
        title.set_label(&item.name);
    }
    previous.set_sensitive(index > 0);
    next.set_sensitive(index + 1 < items.len());
    *current_item.borrow_mut() = item.clone();
    append_image_viewer(state, tools, body, &item.path, win);
}

#[allow(clippy::too_many_arguments)]
fn install_image_gallery(
    state: &AppState,
    win: &gtk::Window,
    tools: &gtk::Box,
    body: &gtk::Box,
    title: &gtk::Label,
    previous: &gtk::Button,
    next: &gtk::Button,
    current_item: Rc<RefCell<ViewerMediaItem>>,
) {
    let (items, initial_index) = image_gallery_items(state, current_item.borrow().clone());
    let items = Rc::new(items);
    let index = Rc::new(std::cell::Cell::new(initial_index));
    let show_navigation = items.len() > 1;
    previous.set_visible(show_navigation);
    next.set_visible(show_navigation);
    render_image_gallery_item(
        state,
        win,
        tools,
        body,
        title,
        previous,
        next,
        &items,
        initial_index,
        &current_item,
    );

    {
        let state = state.clone();
        let win = win.clone();
        let tools = tools.clone();
        let body = body.clone();
        let title = title.clone();
        let previous = previous.clone();
        let next = next.clone();
        let items = items.clone();
        let index = index.clone();
        let current_item = current_item.clone();
        previous.clone().connect_clicked(move |_| {
            let Some(target) = adjacent_image_index(index.get(), items.len(), -1) else {
                return;
            };
            index.set(target);
            render_image_gallery_item(
                &state,
                &win,
                &tools,
                &body,
                &title,
                &previous,
                &next,
                &items,
                target,
                &current_item,
            );
        });
    }
    {
        let state = state.clone();
        let win = win.clone();
        let tools = tools.clone();
        let body = body.clone();
        let title = title.clone();
        let previous = previous.clone();
        let next = next.clone();
        let items = items.clone();
        let index = index.clone();
        let current_item = current_item.clone();
        next.clone().connect_clicked(move |_| {
            let Some(target) = adjacent_image_index(index.get(), items.len(), 1) else {
                return;
            };
            index.set(target);
            render_image_gallery_item(
                &state,
                &win,
                &tools,
                &body,
                &title,
                &previous,
                &next,
                &items,
                target,
                &current_item,
            );
        });
    }

    let controller = gtk::EventControllerKey::new();
    {
        let state = state.clone();
        let win = win.clone();
        let tools = tools.clone();
        let body = body.clone();
        let title = title.clone();
        let previous = previous.clone();
        let next = next.clone();
        let items = items.clone();
        let index = index.clone();
        controller.connect_key_pressed(move |_, key, _, modifiers| {
            if modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            let delta = match key {
                gdk::Key::Left | gdk::Key::KP_Left => -1,
                gdk::Key::Right | gdk::Key::KP_Right => 1,
                _ => return glib::Propagation::Proceed,
            };
            let Some(target) = adjacent_image_index(index.get(), items.len(), delta) else {
                return glib::Propagation::Stop;
            };
            index.set(target);
            render_image_gallery_item(
                &state,
                &win,
                &tools,
                &body,
                &title,
                &previous,
                &next,
                &items,
                target,
                &current_item,
            );
            glib::Propagation::Stop
        });
    }
    win.add_controller(controller);
}

pub(super) fn open_media_viewer(
    state: &AppState,
    path: &str,
    content_type: &str,
    suggest_name: &str,
) {
    let kind = viewer_kind_for(content_type, path, suggest_name);
    let (def_w, def_h) = match kind {
        ViewerKind::Video => (1100, 720),
        ViewerKind::Image => (1080, 760),
        ViewerKind::Pdf | ViewerKind::Text => (920, 700),
        _ => (720, 480),
    };

    let win = gtk::Window::builder()
        .transient_for(&state.window)
        .modal(true)
        .title(crate::i18n::t("media_viewer_title"))
        .default_width(def_w)
        .default_height(def_h)
        .css_classes(["line-media-viewer"])
        .build();

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .css_classes(["line-media-viewer-root"])
        .build();

    let bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_start(12)
        .margin_end(12)
        .margin_top(8)
        .margin_bottom(8)
        .css_classes(["line-media-viewer-bar"])
        .build();
    let title = gtk::Label::builder()
        .label(suggest_name)
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["heading"])
        .build();
    let previous = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text(crate::i18n::t("media_previous_image"))
        .css_classes(["flat", "circular"])
        .visible(false)
        .build();
    let next = gtk::Button::builder()
        .icon_name("go-next-symbolic")
        .tooltip_text(crate::i18n::t("media_next_image"))
        .css_classes(["flat", "circular"])
        .visible(false)
        .build();
    let image_tools = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();

    let open_ext = gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(crate::i18n::t("media_open_externally"))
        .css_classes(["flat", "circular"])
        .build();
    let dl = gtk::Button::builder()
        .icon_name("folder-download-symbolic")
        .tooltip_text(crate::i18n::t("media_download"))
        .css_classes(["flat", "circular"])
        .build();
    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text(crate::i18n::t("media_close"))
        .css_classes(["flat", "circular"])
        .build();
    bar.append(&previous);
    bar.append(&next);
    bar.append(&title);
    bar.append(&image_tools);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .css_classes(["line-media-viewer-body"])
        .build();

    let current_item = Rc::new(RefCell::new(ViewerMediaItem {
        path: path.to_string(),
        content_type: content_type.to_string(),
        name: suggest_name.to_string(),
    }));
    match kind {
        ViewerKind::Video => {
            append_gpu_video_viewer(&body, path);
        }
        ViewerKind::Image => {
            install_image_gallery(
                state,
                &win,
                &image_tools,
                &body,
                &title,
                &previous,
                &next,
                current_item.clone(),
            );
        }
        ViewerKind::Pdf => {
            append_pdf_viewer(&body, path, suggest_name);
        }
        ViewerKind::Text => {
            append_text_viewer(&body, path);
        }
        ViewerKind::Audio => {
            append_audio_viewer(state, &body, path);
        }
        ViewerKind::Generic => {
            append_generic_file_viewer(&body, path, suggest_name);
        }
    }

    bar.append(&open_ext);
    bar.append(&dl);
    bar.append(&close);
    root.append(&bar);
    root.append(&body);
    win.set_child(Some(&root));

    {
        let w = win.clone();
        close.connect_clicked(move |_| w.close());
    }
    {
        let current_item = current_item.clone();
        open_ext.connect_clicked(move |_| {
            open_path_externally(&current_item.borrow().path);
        });
    }
    {
        let s = state.clone();
        let current_item = current_item.clone();
        dl.connect_clicked(move |_| {
            let item = current_item.borrow().clone();
            save_media_as(&s, &item.path, &item.name, &item.content_type);
        });
    }

    let controller = gtk::EventControllerKey::new();
    let w = win.clone();
    controller.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(controller);
    win.present();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ViewerKind {
    Image,
    Video,
    Pdf,
    Text,
    Audio,
    Generic,
}

pub(super) fn viewer_kind_for(content_type: &str, path: &str, name: &str) -> ViewerKind {
    let ct = content_type.to_ascii_uppercase();
    let lower = format!(
        "{} {}",
        path.to_ascii_lowercase(),
        name.to_ascii_lowercase()
    );
    if ct == "VIDEO"
        || lower.contains(".mp4")
        || lower.contains(".webm")
        || lower.contains(".mkv")
        || lower.contains(".mov")
        || lower.contains(".m4v")
    {
        return ViewerKind::Video;
    }
    if ct == "IMAGE"
        || lower.contains(".jpg")
        || lower.contains(".jpeg")
        || lower.contains(".png")
        || lower.contains(".webp")
        || lower.contains(".gif")
        || lower.contains(".bmp")
    {
        return ViewerKind::Image;
    }
    if ct == "AUDIO"
        || lower.contains(".m4a")
        || lower.contains(".mp3")
        || lower.contains(".aac")
        || lower.contains(".ogg")
        || lower.contains(".wav")
        || lower.contains(".flac")
    {
        return ViewerKind::Audio;
    }
    if lower.contains(".pdf") {
        return ViewerKind::Pdf;
    }
    if lower.contains(".txt")
        || lower.contains(".md")
        || lower.contains(".log")
        || lower.contains(".json")
        || lower.contains(".csv")
        || lower.contains(".xml")
        || lower.contains(".yaml")
        || lower.contains(".yml")
        || lower.contains(".toml")
        || lower.contains(".rs")
        || lower.contains(".py")
        || lower.contains(".js")
        || lower.contains(".ts")
        || lower.contains(".css")
        || lower.contains(".html")
        || lower.contains(".c")
        || lower.contains(".h")
        || lower.contains(".go")
    {
        return ViewerKind::Text;
    }
    // Sniff text files without extension.
    if (ct == "FILE" || ct.is_empty()) && looks_like_text_file(path) {
        return ViewerKind::Text;
    }
    ViewerKind::Generic
}

pub(super) fn looks_like_text_file(path: &str) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut buf = [0u8; 512];
    let Ok(n) = f.read(&mut buf) else {
        return false;
    };
    if n == 0 {
        return false;
    }
    let sample = &buf[..n];
    if sample.contains(&0) {
        return false;
    }
    let textish = sample
        .iter()
        .filter(|b| b.is_ascii_graphic() || b.is_ascii_whitespace())
        .count();
    textish * 100 / n >= 90
}

pub(super) fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = n as f64;
    if v >= GB {
        format!("{:.1} GB", v / GB)
    } else if v >= MB {
        format!("{:.1} MB", v / MB)
    } else if v >= KB {
        format!("{:.0} KB", v / KB)
    } else {
        format!("{n} B")
    }
}

pub(super) fn open_path_externally(path: &str) {
    let uri = format!("file://{}", path);
    if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_ok() {
        return;
    }
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// Prefer GPU decode + offloaded composition for in-app video.
pub(super) fn append_gpu_video_viewer(body: &gtk::Box, path: &str) {
    let file = gio::File::for_path(path);
    let stream = gtk::MediaFile::for_file(&file);
    stream.set_loop(false);

    let video = gtk::Video::for_media_stream(Some(&stream));
    video.set_autoplay(true);
    video.set_loop(false);
    video.set_hexpand(true);
    video.set_vexpand(true);
    video.add_css_class("line-media-viewer-video");
    // GTK 4.14+: keep decoded frames on the GPU instead of round-tripping through CPU.
    video.set_graphics_offload(gtk::GraphicsOffloadEnabled::Enabled);

    let offload = gtk::GraphicsOffload::new(Some(&video));
    offload.set_hexpand(true);
    offload.set_vexpand(true);
    offload.add_css_class("line-media-viewer-video");

    body.append(&offload);
    stream.play();
}

pub(super) fn append_image_viewer(
    state: &AppState,
    bar: &gtk::Box,
    body: &gtk::Box,
    path: &str,
    win: &gtk::Window,
) {
    #[derive(Clone)]
    struct Stroke {
        color: (f64, f64, f64, f64),
        width: f64,
        points: Vec<(f64, f64)>,
    }

    let Ok(base_pixbuf) = gdk_pixbuf::Pixbuf::from_file(path) else {
        let label = gtk::Label::builder()
            .label(crate::i18n::t("media_unsupported_preview"))
            .css_classes(["dim-label"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        body.append(&label);
        return;
    };
    let nat_w = base_pixbuf.width().max(1);
    let nat_h = base_pixbuf.height().max(1);

    // View transform: image pixel (ix,iy) -> widget (ox + ix*zoom, oy + iy*zoom).
    let zoom = Rc::new(RefCell::new(1.0_f64));
    let offset = Rc::new(RefCell::new((0.0_f64, 0.0_f64)));
    let fitted = Rc::new(RefCell::new(false));
    let fast_paint = Rc::new(RefCell::new(false));
    let pixbuf = Rc::new(base_pixbuf);
    // Reused viewport-sized buffer so pan/zoom never reallocates every frame.
    let scratch = Rc::new(RefCell::new(None::<gdk_pixbuf::Pixbuf>));
    let strokes = Rc::new(RefCell::new(Vec::<Stroke>::new()));
    let draw_mode = Rc::new(RefCell::new(false));
    let brush_color = Rc::new(RefCell::new((0.90_f64, 0.22, 0.21, 1.0)));
    let brush_width = Rc::new(RefCell::new(6.0_f64));
    let active_stroke = Rc::new(RefCell::new(None::<Stroke>));
    let pan_origin = Rc::new(RefCell::new((0.0_f64, 0.0)));

    let zoom_out = gtk::Button::builder()
        .icon_name("zoom-out-symbolic")
        .tooltip_text(crate::i18n::t("media_zoom_out"))
        .css_classes(["flat", "circular"])
        .build();
    let zoom_reset = gtk::Button::builder()
        .icon_name("zoom-fit-best-symbolic")
        .tooltip_text(crate::i18n::t("media_zoom_reset"))
        .css_classes(["flat", "circular"])
        .build();
    let zoom_in = gtk::Button::builder()
        .icon_name("zoom-in-symbolic")
        .tooltip_text(crate::i18n::t("media_zoom_in"))
        .css_classes(["flat", "circular"])
        .build();
    bar.append(&zoom_out);
    bar.append(&zoom_reset);
    bar.append(&zoom_in);

    let draw_btn = gtk::ToggleButton::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text(crate::i18n::t("media_draw"))
        .css_classes(["flat", "circular"])
        .build();
    let undo_btn = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text(crate::i18n::t("media_draw_undo"))
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .build();
    let clear_btn = gtk::Button::builder()
        .icon_name("edit-clear-all-symbolic")
        .tooltip_text(crate::i18n::t("media_draw_clear"))
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .build();
    let send_btn = gtk::Button::builder()
        .icon_name("mail-send-symbolic")
        .tooltip_text(crate::i18n::t("media_draw_send"))
        .css_classes(["flat", "circular", "suggested-action"])
        .build();
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&draw_btn);
    for (r, g, b, class, tip) in [
        (0.90, 0.22, 0.21, "line-draw-c-red", "Red"),
        (0.26, 0.63, 0.28, "line-draw-c-green", "Green"),
        (0.12, 0.53, 0.90, "line-draw-c-blue", "Blue"),
        (0.99, 0.85, 0.21, "line-draw-c-yellow", "Yellow"),
        (1.0, 1.0, 1.0, "line-draw-c-white", "White"),
        (0.13, 0.13, 0.13, "line-draw-c-black", "Black"),
    ] {
        let btn = gtk::Button::builder()
            .label("●")
            .tooltip_text(tip)
            .css_classes(["flat", "circular", "line-draw-swatch", class])
            .build();
        let brush_color = brush_color.clone();
        let draw_btn = draw_btn.clone();
        btn.connect_clicked(move |_| {
            *brush_color.borrow_mut() = (r, g, b, 1.0);
            if !draw_btn.is_active() {
                draw_btn.set_active(true);
            }
        });
        bar.append(&btn);
    }
    bar.append(&undo_btn);
    bar.append(&clear_btn);
    bar.append(&send_btn);

    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .css_classes(["line-media-viewer-image"])
        .build();
    area.set_cursor_from_name(Some("grab"));
    area.set_draw_func({
        let pixbuf = pixbuf.clone();
        let scratch = scratch.clone();
        let zoom = zoom.clone();
        let offset = offset.clone();
        let fast_paint = fast_paint.clone();
        let strokes = strokes.clone();
        let active_stroke = active_stroke.clone();
        move |_area, cr, width, height| {
            if width <= 0 || height <= 0 {
                return;
            }
            // Clear is handled by GTK/CSS background; only composite the scaled viewport.
            let z = (*zoom.borrow()).max(0.01);
            let (ox, oy) = *offset.borrow();
            let interp = if *fast_paint.borrow() {
                gdk_pixbuf::InterpType::Nearest
            } else {
                gdk_pixbuf::InterpType::Bilinear
            };

            {
                let mut slot = scratch.borrow_mut();
                let needs = slot
                    .as_ref()
                    .map(|p| p.width() != width || p.height() != height)
                    .unwrap_or(true);
                if needs {
                    *slot = gdk_pixbuf::Pixbuf::new(
                        gdk_pixbuf::Colorspace::Rgb,
                        true,
                        8,
                        width,
                        height,
                    );
                }
                if let Some(dest) = slot.as_ref() {
                    // Transparent edges so the themed DrawingArea background shows through.
                    dest.fill(0);
                    pixbuf.scale(dest, 0, 0, width, height, ox, oy, z, z, interp);
                    gdk::prelude::GdkCairoContextExt::set_source_pixbuf(cr, dest, 0.0, 0.0);
                    let _ = cr.paint();
                }
            }

            // Strokes in image space → widget space.
            let paint_stroke = |cr: &gtk::cairo::Context, stroke: &Stroke| {
                if stroke.points.is_empty() {
                    return;
                }
                let (r, g, b, a) = stroke.color;
                cr.set_source_rgba(r, g, b, a);
                cr.set_line_width((stroke.width * z).max(1.0));
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_line_join(gtk::cairo::LineJoin::Round);
                let (x0, y0) = stroke.points[0];
                cr.move_to(ox + x0 * z, oy + y0 * z);
                for &(x, y) in stroke.points.iter().skip(1) {
                    cr.line_to(ox + x * z, oy + y * z);
                }
                if stroke.points.len() == 1 {
                    cr.line_to(ox + x0 * z + 0.01, oy + y0 * z);
                }
                let _ = cr.stroke();
            };
            for stroke in strokes.borrow().iter() {
                paint_stroke(cr, stroke);
            }
            if let Some(stroke) = active_stroke.borrow().as_ref() {
                paint_stroke(cr, stroke);
            }
        }
    });

    let fit_to_area = {
        let zoom = zoom.clone();
        let offset = offset.clone();
        let area = area.clone();
        Rc::new(move || {
            let aw = area.width().max(1) as f64;
            let ah = area.height().max(1) as f64;
            let z = (aw / nat_w as f64).min(ah / nat_h as f64).clamp(0.05, 1.0);
            *zoom.borrow_mut() = z;
            *offset.borrow_mut() = ((aw - nat_w as f64 * z) * 0.5, (ah - nat_h as f64 * z) * 0.5);
            area.queue_draw();
        })
    };

    let zoom_at = {
        let zoom = zoom.clone();
        let offset = offset.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        Rc::new(move |factor: f64, pivot_x: f64, pivot_y: f64, fast: bool| {
            let old = (*zoom.borrow()).max(0.01);
            let new_z = (old * factor).clamp(0.05, 16.0);
            if (new_z - old).abs() < f64::EPSILON {
                return;
            }
            let (ox, oy) = *offset.borrow();
            // Keep the image point under the pivot fixed.
            let ox2 = pivot_x - (pivot_x - ox) * (new_z / old);
            let oy2 = pivot_y - (pivot_y - oy) * (new_z / old);
            *zoom.borrow_mut() = new_z;
            *offset.borrow_mut() = (ox2, oy2);
            *fast_paint.borrow_mut() = fast;
            area.queue_draw();
        })
    };

    {
        let area = area.clone();
        let fitted = fitted.clone();
        let fit_to_area = fit_to_area.clone();
        area.connect_resize(move |_, _, _| {
            if !*fitted.borrow() {
                *fitted.borrow_mut() = true;
                fit_to_area();
            }
        });
    }

    {
        let zoom_at = zoom_at.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        zoom_in.connect_clicked(move |_| {
            let cx = area.width() as f64 * 0.5;
            let cy = area.height() as f64 * 0.5;
            zoom_at(1.25, cx, cy, false);
            *fast_paint.borrow_mut() = false;
            area.queue_draw();
        });
    }
    {
        let zoom_at = zoom_at.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        zoom_out.connect_clicked(move |_| {
            let cx = area.width() as f64 * 0.5;
            let cy = area.height() as f64 * 0.5;
            zoom_at(1.0 / 1.25, cx, cy, false);
            *fast_paint.borrow_mut() = false;
            area.queue_draw();
        });
    }
    {
        let fit_to_area = fit_to_area.clone();
        let fast_paint = fast_paint.clone();
        zoom_reset.connect_clicked(move |_| {
            *fast_paint.borrow_mut() = false;
            fit_to_area();
        });
    }

    let refresh_edit_btns = {
        let undo_btn = undo_btn.clone();
        let clear_btn = clear_btn.clone();
        let strokes = strokes.clone();
        Rc::new(move || {
            let n = strokes.borrow().len();
            undo_btn.set_sensitive(n > 0);
            clear_btn.set_sensitive(n > 0);
        })
    };

    {
        let draw_mode = draw_mode.clone();
        let area = area.clone();
        draw_btn.connect_toggled(move |btn| {
            let on = btn.is_active();
            *draw_mode.borrow_mut() = on;
            area.set_cursor_from_name(Some(if on { "crosshair" } else { "grab" }));
        });
    }
    {
        let strokes = strokes.clone();
        let area = area.clone();
        let refresh = refresh_edit_btns.clone();
        undo_btn.connect_clicked(move |_| {
            strokes.borrow_mut().pop();
            refresh();
            area.queue_draw();
        });
    }
    {
        let strokes = strokes.clone();
        let area = area.clone();
        let refresh = refresh_edit_btns.clone();
        clear_btn.connect_clicked(move |_| {
            strokes.borrow_mut().clear();
            refresh();
            area.queue_draw();
        });
    }
    {
        let s = state.clone();
        let win = win.clone();
        let pixbuf = pixbuf.clone();
        let strokes = strokes.clone();
        let src_path = path.to_string();
        send_btn.connect_clicked(move |_| {
            let out = if strokes.borrow().is_empty() {
                std::path::PathBuf::from(&src_path)
            } else {
                let packed: Vec<_> = strokes
                    .borrow()
                    .iter()
                    .map(|st| (st.color, st.width, st.points.clone()))
                    .collect();
                match export_annotated_image(pixbuf.as_ref(), &packed, &s.data_dir) {
                    Ok(p) => p,
                    Err(e) => {
                        toast(
                            &s,
                            &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
                        );
                        return;
                    }
                }
            };
            if send_local_image_file(&s, &out) {
                win.close();
            }
        });
    }

    // Scroll: wheel pans; Ctrl+wheel zooms toward pointer.
    let scroll_ctl = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
    );
    {
        let zoom_at = zoom_at.clone();
        let offset = offset.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        scroll_ctl.connect_scroll(move |ctl, dx, dy| {
            let state = ctl.current_event_state();
            if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                let px = area.width() as f64 * 0.5;
                let py = area.height() as f64 * 0.5;
                let factor = if dy < 0.0 {
                    1.15
                } else if dy > 0.0 {
                    1.0 / 1.15
                } else {
                    return glib::Propagation::Stop;
                };
                zoom_at(factor, px, py, true);
                let fast_paint = fast_paint.clone();
                let area = area.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                    *fast_paint.borrow_mut() = false;
                    area.queue_draw();
                });
                return glib::Propagation::Stop;
            }
            // Pan with wheel.
            let mut off = offset.borrow_mut();
            off.0 -= dx * 48.0;
            off.1 -= dy * 48.0;
            drop(off);
            area.queue_draw();
            glib::Propagation::Stop
        });
    }
    area.add_controller(scroll_ctl);

    // Drag: pan when browsing, draw when edit mode is on.
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    {
        let draw_mode = draw_mode.clone();
        let pan_origin = pan_origin.clone();
        let offset = offset.clone();
        let active_stroke = active_stroke.clone();
        let brush_color = brush_color.clone();
        let brush_width = brush_width.clone();
        let zoom = zoom.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        drag.connect_drag_begin(move |gesture, x, y| {
            if *draw_mode.borrow() {
                let z = (*zoom.borrow()).max(0.01);
                let (ox, oy) = *offset.borrow();
                let color = *brush_color.borrow();
                let width = (*brush_width.borrow() / z).max(1.0);
                *active_stroke.borrow_mut() = Some(Stroke {
                    color,
                    width,
                    points: vec![((x - ox) / z, (y - oy) / z)],
                });
                area.set_cursor_from_name(Some("crosshair"));
            } else {
                *pan_origin.borrow_mut() = *offset.borrow();
                *fast_paint.borrow_mut() = true;
                area.set_cursor_from_name(Some("grabbing"));
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
    }
    {
        let draw_mode = draw_mode.clone();
        let pan_origin = pan_origin.clone();
        let offset = offset.clone();
        let active_stroke = active_stroke.clone();
        let zoom = zoom.clone();
        let area = area.clone();
        drag.connect_drag_update(move |gesture, _x, _y| {
            let Some((dx, dy)) = gesture.offset() else {
                return;
            };
            let Some((sx, sy)) = gesture.start_point() else {
                return;
            };
            if *draw_mode.borrow() {
                let z = (*zoom.borrow()).max(0.01);
                let (ox, oy) = *offset.borrow();
                if let Some(stroke) = active_stroke.borrow_mut().as_mut() {
                    stroke.points.push(((sx + dx - ox) / z, (sy + dy - oy) / z));
                }
                area.queue_draw();
            } else {
                let (ox, oy) = *pan_origin.borrow();
                *offset.borrow_mut() = (ox + dx, oy + dy);
                area.queue_draw();
            }
        });
    }
    {
        let draw_mode = draw_mode.clone();
        let active_stroke = active_stroke.clone();
        let strokes = strokes.clone();
        let area = area.clone();
        let refresh = refresh_edit_btns.clone();
        let fast_paint = fast_paint.clone();
        drag.connect_drag_end(move |_, _x, _y| {
            if *draw_mode.borrow() {
                if let Some(stroke) = active_stroke.borrow_mut().take()
                    && !stroke.points.is_empty()
                {
                    strokes.borrow_mut().push(stroke);
                    refresh();
                }
                area.set_cursor_from_name(Some("crosshair"));
            } else {
                *fast_paint.borrow_mut() = false;
                area.set_cursor_from_name(Some("grab"));
            }
            area.queue_draw();
        });
    }
    area.add_controller(drag);

    body.append(&area);
}

pub(super) type PackedStroke = ((f64, f64, f64, f64), f64, Vec<(f64, f64)>);

pub(super) fn export_annotated_image(
    pixbuf: &gdk_pixbuf::Pixbuf,
    strokes: &[PackedStroke],
    data_dir: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    let w = pixbuf.width().max(1);
    let h = pixbuf.height().max(1);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h)
        .map_err(|e| anyhow::anyhow!("cairo surface: {e}"))?;
    let cr = cairo::Context::new(&surface).map_err(|e| anyhow::anyhow!("cairo: {e}"))?;
    gdk::prelude::GdkCairoContextExt::set_source_pixbuf(&cr, pixbuf, 0.0, 0.0);
    cr.paint().map_err(|e| anyhow::anyhow!("paint: {e}"))?;

    for (color, width, points) in strokes {
        if points.is_empty() {
            continue;
        }
        let (r, g, b, a) = *color;
        cr.set_source_rgba(r, g, b, a);
        cr.set_line_width(*width);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.move_to(points[0].0, points[0].1);
        for &(x, y) in points.iter().skip(1) {
            cr.line_to(x, y);
        }
        if points.len() == 1 {
            cr.line_to(points[0].0 + 0.01, points[0].1);
        }
        cr.stroke().map_err(|e| anyhow::anyhow!("stroke: {e}"))?;
    }

    let dir = data_dir.join("cache/media");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!(
        "annotated-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let mut file = std::fs::File::create(&dest)?;
    surface
        .write_to_png(&mut file)
        .map_err(|e| anyhow::anyhow!("png: {e}"))?;
    Ok(dest)
}

pub(super) fn send_local_image_file(state: &AppState, path: &std::path::Path) -> bool {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return false;
    };
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return false;
    }
    let cached = match copy_into_media_cache(state, path) {
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
        .unwrap_or("image.png")
        .to_string();
    match state
        .sidecar
        .send_media(&chat_mid, &path_str, "image", None)
    {
        Ok(id) => {
            begin_optimistic_send(
                state,
                id,
                &chat_mid,
                MessageInfo {
                    id: String::new(),
                    text: "[Image]".into(),
                    from: String::new(),
                    to: chat_mid.clone(),
                    mine: true,
                    created_time: now_ms(),
                    content_type: "IMAGE".into(),
                    image_path: Some(path_str.clone()),
                    audio_path: None,
                    file_name: Some(file_name),
                    file_path: Some(path_str),
                    duration_ms: None,
                    flex: None,
                    ..Default::default()
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

pub(super) fn append_pdf_viewer(body: &gtk::Box, path: &str, suggest_name: &str) {
    let wrap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(16)
        .margin_end(16)
        .margin_top(12)
        .margin_bottom(12)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-media-viewer-doc"])
        .build();

    let preview_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .css_classes(["line-media-viewer-pdf-preview"])
        .build();

    // Render first page with pdftoppm when available.
    let cache = std::env::temp_dir().join(format!(
        "line-gtk-pdf-{}-preview",
        std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("doc")
    ));
    let _ = std::fs::create_dir_all(&cache);
    let out_prefix = cache.join("page");
    let rendered = std::process::Command::new("pdftoppm")
        .args([
            "-jpeg",
            "-f",
            "1",
            "-l",
            "1",
            "-r",
            "120",
            path,
            out_prefix.to_str().unwrap_or("/tmp/line-gtk-pdf"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let preview_path = cache.join("page-1.jpg");
    if rendered && preview_path.is_file() {
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        let pic = gtk::Picture::for_filename(&preview_path);
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_can_shrink(true);
        pic.set_hexpand(true);
        pic.set_vexpand(true);
        scroll.set_child(Some(&pic));
        preview_box.append(&scroll);
    } else {
        let hint = gtk::Label::builder()
            .label(crate::i18n::t("media_pdf_preview"))
            .css_classes(["title-3"])
            .build();
        preview_box.append(&hint);
    }

    let meta = gtk::Label::builder()
        .label({
            let size = std::fs::metadata(path)
                .map(|m| format_bytes(m.len()))
                .unwrap_or_else(|_| "?".into());
            format!(
                "{suggest_name} · {}",
                crate::i18n::tf("media_file_size", &[("size", &size)])
            )
        })
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();

    // Optional text extract for searchable peek.
    let text_out = std::process::Command::new("pdftotext")
        .args(["-f", "1", "-l", "2", "-layout", path, "-"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_default();
    if !text_out.trim().is_empty() {
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .min_content_height(140)
            .css_classes(["line-media-viewer-text-scroll"])
            .build();
        let tv = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .monospace(true)
            .css_classes(["line-media-viewer-text"])
            .build();
        let buf = tv.buffer();
        let snippet: String = text_out.chars().take(4_000).collect();
        buf.set_text(&snippet);
        scroll.set_child(Some(&tv));
        wrap.append(&preview_box);
        wrap.append(&meta);
        wrap.append(&scroll);
    } else {
        wrap.append(&preview_box);
        wrap.append(&meta);
    }

    let open_btn = gtk::Button::builder()
        .label(crate::i18n::t("media_open_externally"))
        .halign(gtk::Align::Start)
        .css_classes(["suggested-action"])
        .build();
    let path_owned = path.to_string();
    open_btn.connect_clicked(move |_| open_path_externally(&path_owned));
    wrap.append(&open_btn);
    body.append(&wrap);
}

pub(super) fn append_text_viewer(body: &gtk::Box, path: &str) {
    let scroll = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-media-viewer-text-scroll"])
        .build();
    let tv = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .monospace(true)
        .left_margin(14)
        .right_margin(14)
        .top_margin(12)
        .bottom_margin(12)
        .css_classes(["line-media-viewer-text"])
        .build();
    let buf = tv.buffer();
    const LIMIT: u64 = 1_500_000;
    let meta = std::fs::metadata(path).ok();
    let truncated = meta.map(|m| m.len() > LIMIT).unwrap_or(false);
    match std::fs::read(path) {
        Ok(bytes) => {
            let slice = if bytes.len() as u64 > LIMIT {
                &bytes[..LIMIT as usize]
            } else {
                &bytes[..]
            };
            let text = String::from_utf8_lossy(slice);
            if truncated {
                buf.set_text(&format!(
                    "{}\n\n{}",
                    crate::i18n::t("media_text_truncated"),
                    text
                ));
            } else {
                buf.set_text(&text);
            }
        }
        Err(e) => {
            buf.set_text(&crate::i18n::tf(
                "file_read_failed",
                &[("error", &e.to_string())],
            ));
        }
    }
    scroll.set_child(Some(&tv));
    body.append(&scroll);
}

pub(super) fn append_audio_viewer(state: &AppState, body: &gtk::Box, path: &str) {
    let wrap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-media-viewer-audio"])
        .build();
    let icon = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    icon.set_pixel_size(72);
    let size = std::fs::metadata(path)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".into());
    let label = gtk::Label::builder()
        .label(crate::i18n::tf("media_file_size", &[("size", &size)]))
        .css_classes(["dim-label"])
        .build();
    let play = gtk::Button::builder()
        .label(crate::i18n::t("play_voice"))
        .css_classes(["suggested-action", "pill"])
        .build();
    let cfg = state.config.clone();
    let toast_state = state.clone();
    let path = path.to_string();
    play.connect_clicked(move |_| {
        if let Err(e) = play_audio_file(&path, &cfg.borrow().audio_output) {
            toast(
                &toast_state,
                &crate::i18n::tf("voice_play_failed", &[("error", &e)]),
            );
        }
    });
    wrap.append(&icon);
    wrap.append(&label);
    wrap.append(&play);
    body.append(&wrap);
}

pub(super) fn append_generic_file_viewer(body: &gtk::Box, path: &str, suggest_name: &str) {
    let wrap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .margin_start(24)
        .margin_end(24)
        .css_classes(["line-media-viewer-generic"])
        .build();
    let icon = gtk::Image::from_icon_name("folder-download-symbolic");
    icon.set_pixel_size(64);
    let name = gtk::Label::builder()
        .label(suggest_name)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["title-3"])
        .build();
    let size = std::fs::metadata(path)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".into());
    let meta = gtk::Label::builder()
        .label(crate::i18n::tf("media_file_size", &[("size", &size)]))
        .css_classes(["dim-label"])
        .build();
    let hint = gtk::Label::builder()
        .label(crate::i18n::t("media_unsupported_preview"))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label"])
        .build();
    let open_btn = gtk::Button::builder()
        .label(crate::i18n::t("media_open_externally"))
        .css_classes(["suggested-action"])
        .build();
    let path_owned = path.to_string();
    open_btn.connect_clicked(move |_| open_path_externally(&path_owned));
    wrap.append(&icon);
    wrap.append(&name);
    wrap.append(&meta);
    wrap.append(&hint);
    wrap.append(&open_btn);
    body.append(&wrap);
}

pub(super) fn pulse_device_list(kind: &str) -> Vec<(String, String)> {
    // kind: "sources" | "sinks"
    let out = std::process::Command::new("pactl")
        .args(["list", "short", kind])
        .output()
        .ok();
    let Some(out) = out else {
        return vec![("default".into(), "Default".into())];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut list = vec![("default".into(), "Default".into())];
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 {
            continue;
        }
        let name = cols[1].to_string();
        // Skip monitors as record sources unless nothing else.
        if kind == "sources" && name.contains(".monitor") {
            continue;
        }
        let label = name.clone();
        list.push((name, label));
    }
    list
}

pub fn pulse_sources() -> Vec<(String, String)> {
    pulse_device_list("sources")
}

pub fn pulse_sinks() -> Vec<(String, String)> {
    pulse_device_list("sinks")
}

pub(super) fn guess_media_o_type(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "gif" => "gif",
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "heic" => "image",
        "mp4" | "webm" | "mov" | "mkv" | "m4v" => "video",
        "m4a" | "mp3" | "wav" | "ogg" | "oga" | "aac" | "flac" => "audio",
        _ => "file",
    }
}

pub(super) fn ffprobe_duration_ms(path: &std::path::Path) -> Option<u64> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path.to_str()?,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let secs: f64 = text.trim().parse().ok()?;
    if !secs.is_finite() || secs <= 0.0 {
        return None;
    }
    Some((secs * 1000.0).round() as u64)
}

pub(super) fn copy_into_media_cache(
    state: &AppState,
    src: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    let dir = state.data_dir.join("cache/media");
    std::fs::create_dir_all(&dir)?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("bin");
    let name = format!(
        "out-{}-{}.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        src.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(40)
            .collect::<String>(),
        ext
    );
    let dest = dir.join(name);
    std::fs::copy(src, &dest)?;
    Ok(dest)
}

pub(super) fn mark_media_failed(state: &AppState, message_id: &str) {
    let Some(bubble) = state.media_slots.borrow().get(message_id).cloned() else {
        return;
    };
    let msg = state.media_msgs.borrow().get(message_id).cloned();
    let is_video = msg
        .as_ref()
        .map(|m| m.content_type.eq_ignore_ascii_case("video"))
        .unwrap_or(false);
    // Don't clobber a voice card with "image unavailable".
    let mut child = bubble.first_child();
    while let Some(w) = child {
        let next = w.next_sibling();
        if w.css_classes().iter().any(|c| c == "line-voice-card") {
            // Keep voice UI; just mark play unavailable text if needed.
            let fail = gtk::Label::builder()
                .label(crate::i18n::t("voice_unavailable"))
                .xalign(0.0)
                .css_classes(["dim-label", "line-media-failed"])
                .build();
            bubble.append(&fail);
            return;
        }
        child = next;
    }
    let mut child = bubble.first_child();
    while let Some(w) = child {
        let next = w.next_sibling();
        let drop = w.css_classes().iter().any(|c| {
            c == "line-media-placeholder"
                || c == "line-bubble-image"
                || c == "line-media-failed"
                || c == "line-video-wrap"
                || c == "line-video-placeholder"
        }) || w.downcast_ref::<gtk::Picture>().is_some()
            || w.downcast_ref::<gtk::Overlay>().is_some();
        if drop {
            bubble.remove(&w);
        }
        child = next;
    }
    if is_video && let Some(msg) = msg.as_ref() {
        append_video_placeholder(state, &bubble, msg, true);
        return;
    }
    let label = gtk::Label::builder()
        .label(crate::i18n::t("image_unavailable"))
        .xalign(0.0)
        .css_classes(["dim-label", "line-media-failed"])
        .build();
    bubble.append(&label);
}

pub(super) fn make_media_picture_placeholder(sticker: bool) -> gtk::Picture {
    let (w, h) = if sticker { (128, 128) } else { (220, 160) };
    gtk::Picture::builder()
        .content_fit(if sticker {
            gtk::ContentFit::Contain
        } else {
            gtk::ContentFit::Cover
        })
        .can_shrink(true)
        .width_request(w)
        .height_request(h)
        .css_classes(if sticker {
            ["line-sticker-image"]
        } else {
            ["line-bubble-image"]
        })
        .build()
}

pub(super) fn wrap_video_thumb(
    state: &AppState,
    pic: &gtk::Picture,
    msg: &MessageInfo,
) -> gtk::Overlay {
    pic.add_css_class("line-video-thumb");
    pic.set_tooltip_text(Some(&crate::i18n::t("media_open_video")));
    let overlay = gtk::Overlay::builder()
        .child(pic)
        .css_classes(["line-video-wrap"])
        .build();
    let badge = gtk::Image::builder()
        .icon_name("media-playback-start-symbolic")
        .pixel_size(28)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-video-play-badge"])
        .build();
    overlay.add_overlay(&badge);
    wire_media_open_click_widget(state, overlay.upcast_ref::<gtk::Widget>(), msg);
    overlay
}

pub(super) fn append_video_placeholder(
    state: &AppState,
    bubble: &gtk::Box,
    msg: &MessageInfo,
    failed: bool,
) {
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .width_request(180)
        .height_request(110)
        .css_classes(if failed {
            vec!["line-video-placeholder", "line-media-failed"]
        } else {
            vec!["line-video-placeholder", "line-media-placeholder"]
        })
        .build();
    let icon = gtk::Image::builder()
        .icon_name(if failed {
            "media-playback-start-symbolic"
        } else {
            "content-loading-symbolic"
        })
        .pixel_size(28)
        .css_classes(["line-video-play-badge"])
        .build();
    let label = gtk::Label::builder()
        .label(if failed {
            crate::i18n::t("video_unavailable")
        } else {
            crate::i18n::t("media_open_video")
        })
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label"])
        .build();
    box_.append(&icon);
    box_.append(&label);
    wire_media_open_click_widget(state, box_.upcast_ref::<gtk::Widget>(), msg);
    bubble.append(&box_);
}

pub(super) fn show_upload_progress(state: &AppState, progress: f64, label: &str) {
    let frac = progress.clamp(0.0, 1.0);
    state.upload_bar.set_fraction(frac);
    let text = if label.is_empty() {
        crate::i18n::t("media_uploading")
    } else {
        label.to_string()
    };
    state.upload_label.set_text(&text);
    state.upload_revealer.set_reveal_child(true);
    state.status.set_text(&text);
}

pub(super) fn hide_upload_progress(state: &AppState) {
    state.upload_revealer.set_reveal_child(false);
    state.upload_bar.set_fraction(0.0);
    state.upload_label.set_text("");
}

pub(super) fn wire_media_open_click_widget(
    state: &AppState,
    widget: &gtk::Widget,
    msg: &MessageInfo,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let s = state.clone();
    let msg = msg.clone();
    click.connect_released(move |_, _n, _x, _y| {
        request_media_download(&s, &msg, "open_viewer");
    });
    widget.add_controller(click);
    widget.set_cursor_from_name(Some("pointer"));
}

pub(super) fn raw_frame_to_pixbuf(frame: &crate::sticker_anim::RawFrame) -> Option<Pixbuf> {
    let bytes = glib::Bytes::from_owned(frame.rgba.clone());
    Some(Pixbuf::from_bytes(
        &bytes,
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        frame.width,
        frame.height,
        frame.width * 4,
    ))
}

pub(super) fn apply_frames_to_picture(
    picture: &gtk::Picture,
    frames: crate::sticker_anim::AnimFrames,
    animate: bool,
) {
    let Some(first) = frames.frames.first().and_then(raw_frame_to_pixbuf) else {
        return;
    };
    picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&first)));
    if !animate || frames.frames.len() <= 1 {
        return;
    }

    let max_plays = frames.plays;
    let pixbufs: Vec<(Pixbuf, u32)> = frames
        .frames
        .iter()
        .filter_map(|f| {
            let pb = raw_frame_to_pixbuf(f)?;
            Some((pb, f.delay_ms))
        })
        .collect();
    if pixbufs.len() <= 1 {
        return;
    }

    let frames = std::rc::Rc::new(pixbufs);
    let idx = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let completed_plays = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let picture = picture.clone();

    fn schedule(
        picture: gtk::Picture,
        frames: std::rc::Rc<Vec<(Pixbuf, u32)>>,
        idx: std::rc::Rc<std::cell::Cell<usize>>,
        completed_plays: std::rc::Rc<std::cell::Cell<u32>>,
        max_plays: u32,
    ) {
        let delay = frames
            .get(idx.get())
            .map(|(_, d)| (*d as u64).max(20))
            .unwrap_or(50);
        let picture_weak = picture.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
            let Some(picture) = picture_weak.upgrade() else {
                return;
            };
            if picture.parent().is_none() {
                return;
            }
            let next = (idx.get() + 1) % frames.len();
            if next == 0 {
                let completed = completed_plays.get().saturating_add(1);
                completed_plays.set(completed);
                if max_plays > 0 && completed >= max_plays {
                    return;
                }
            }
            idx.set(next);
            if let Some((pb, _)) = frames.get(next) {
                picture.set_paintable(Some(&gdk::Texture::for_pixbuf(pb)));
            }
            schedule(picture, frames, idx, completed_plays, max_plays);
        });
    }

    schedule(picture, frames, idx, completed_plays, max_plays);
}

pub(super) fn attach_texture_async(picture: gtk::Picture, path: String, max_px: i32) {
    attach_texture_async_anim(picture, path, max_px, false);
}

/// Load a center-cropped square at its exact logical size. GtkPicture otherwise
/// keeps the source texture's natural dimensions, so differently sized profile
/// files can make nominally identical avatar rows allocate at different sizes.
pub(super) fn attach_avatar_texture_async(picture: gtk::Picture, path: String, size_px: i32) {
    let size_px = size_px.max(1);
    picture.set_width_request(size_px);
    picture.set_height_request(size_px);
    picture.set_hexpand(false);
    picture.set_vexpand(false);
    picture.set_overflow(gtk::Overflow::Hidden);

    let (tx, rx) = async_channel::bounded::<Option<crate::sticker_anim::RawFrame>>(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(crate::sticker_anim::load_square(&path, size_px));
    });
    glib::spawn_future_local(async move {
        if let Ok(Some(frame)) = rx.recv().await
            && let Some(pixbuf) = raw_frame_to_pixbuf(&frame)
        {
            picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf)));
        }
    });
}

pub(super) fn attach_texture_async_anim(
    picture: gtk::Picture,
    path: String,
    max_px: i32,
    animate: bool,
) {
    let fixed_width = picture.width_request();
    let fixed_height = picture.height_request();
    let has_fixed_canvas = fixed_width > 0 && fixed_height > 0;
    let cover = picture.content_fit() == gtk::ContentFit::Cover;
    if has_fixed_canvas {
        picture.set_hexpand(false);
        picture.set_vexpand(false);
        picture.set_halign(gtk::Align::Center);
        picture.set_valign(gtk::Align::Center);
        picture.set_overflow(gtk::Overflow::Hidden);
    }
    let (tx, rx) = async_channel::bounded::<Option<crate::sticker_anim::AnimFrames>>(1);
    std::thread::spawn(move || {
        let frames = if has_fixed_canvas {
            crate::sticker_anim::load_fitted(&path, fixed_width, fixed_height, cover, animate)
        } else {
            crate::sticker_anim::load_scaled(&path, max_px, animate)
        };
        let _ = tx.send_blocking(frames);
    });
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Some(frames)) => apply_frames_to_picture(&picture, frames, animate),
            _ => {
                if let Some(parent) = picture.parent()
                    && let Some(box_) = parent.downcast_ref::<gtk::Box>()
                {
                    box_.remove(&picture);
                    let label = gtk::Label::builder()
                        .label(crate::i18n::t("image_unavailable"))
                        .xalign(0.0)
                        .css_classes(["dim-label", "line-media-failed"])
                        .build();
                    box_.append(&label);
                }
            }
        }
    });
}

pub(super) fn pump_media_queue(state: &AppState) {
    if *state.media_pumping.borrow() {
        return;
    }
    let next = state.media_queue.borrow_mut().pop_front();
    let Some((message_id, image_path)) = next else {
        return;
    };
    if !std::path::Path::new(&image_path).exists() {
        mark_media_failed(state, &message_id);
        let state = state.clone();
        glib::idle_add_local_once(move || pump_media_queue(&state));
        return;
    }

    let Some(bubble) = state.media_slots.borrow().get(&message_id).cloned() else {
        // Bubble not mounted yet (list rebuild / idle chunk). Keep the path so
        // append_message can attach it when the row appears; do not spin the queue.
        state
            .media_ready_paths
            .borrow_mut()
            .insert(message_id, image_path);
        let state = state.clone();
        glib::idle_add_local_once(move || pump_media_queue(&state));
        return;
    };

    *state.media_pumping.borrow_mut() = true;
    let is_sticker = bubble
        .css_classes()
        .iter()
        .any(|c| c == "line-bubble-sticker");

    // Keep the "Loading…" label until decode succeeds.
    let max_px = if is_sticker { 128 } else { 320 };
    let animate = is_sticker && state.config.borrow().animations;
    let state2 = state.clone();
    let bubble2 = bubble.clone();
    let message_id2 = message_id.clone();

    let (tx, rx) = async_channel::bounded::<Option<crate::sticker_anim::AnimFrames>>(1);
    let image_path2 = image_path.clone();
    std::thread::spawn(move || {
        let loaded =
            crate::sticker_anim::load_scaled(&image_path2, max_px, animate).or_else(|| {
                // Brief retry — avoids rare races right after atomic rename.
                std::thread::sleep(std::time::Duration::from_millis(30));
                crate::sticker_anim::load_scaled(&image_path2, max_px, animate)
            });
        let _ = tx.send_blocking(loaded);
    });
    glib::spawn_future_local(async move {
        let frames = rx.recv().await.ok().flatten();
        if let Some(frames) = frames {
            let mut child = bubble2.first_child();
            while let Some(w) = child {
                let next_w = w.next_sibling();
                if w.css_classes().iter().any(|c| {
                    c == "line-media-placeholder"
                        || c == "line-media-failed"
                        || c == "line-bubble-image"
                        || c == "line-video-wrap"
                        || c == "line-video-placeholder"
                }) || w.downcast_ref::<gtk::Picture>().is_some()
                    || w.downcast_ref::<gtk::Overlay>().is_some()
                {
                    bubble2.remove(&w);
                }
                child = next_w;
            }
            let pic = make_media_picture_placeholder(is_sticker);
            apply_frames_to_picture(&pic, frames, animate);
            let msg = state2.media_msgs.borrow().get(&message_id2).cloned();
            let is_video = msg
                .as_ref()
                .map(|m| m.content_type.eq_ignore_ascii_case("video"))
                .unwrap_or(false);
            let is_image = msg
                .as_ref()
                .map(|m| m.content_type.eq_ignore_ascii_case("image"))
                .unwrap_or(false);
            if !is_sticker && is_video {
                if let Some(msg) = msg.as_ref() {
                    let overlay = wrap_video_thumb(&state2, &pic, msg);
                    bubble2.append(&overlay);
                } else {
                    bubble2.append(&pic);
                }
            } else {
                bubble2.append(&pic);
                if !is_sticker
                    && is_image
                    && let Some(msg) = msg.as_ref()
                {
                    wire_media_open_click(&state2, &pic, msg, "image");
                    pic.set_tooltip_text(Some(&crate::i18n::t("media_open_image")));
                }
            }
            if *state2.stick_bottom.borrow() {
                scroll_messages_to_end(&state2);
            }
        } else {
            mark_media_failed(&state2, &message_id2);
        }
        *state2.media_pumping.borrow_mut() = false;
        let state3 = state2.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(12), move || {
            pump_media_queue(&state3);
        });
    });
}

#[cfg(test)]
mod navigation_tests {
    use super::adjacent_image_index;

    #[test]
    fn image_navigation_stops_at_gallery_edges() {
        assert_eq!(adjacent_image_index(1, 3, -1), Some(0));
        assert_eq!(adjacent_image_index(1, 3, 1), Some(2));
        assert_eq!(adjacent_image_index(0, 3, -1), None);
        assert_eq!(adjacent_image_index(2, 3, 1), None);
        assert_eq!(adjacent_image_index(0, 0, 1), None);
    }
}
