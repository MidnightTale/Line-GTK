use super::*;

pub(super) fn open_sticker_picker(state: &AppState) {
    if state.current_chat.borrow().is_none() {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    }
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    // Show a loading placeholder while the sidecar lists / caches thumbs.
    let loading = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(28)
        .margin_end(28)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let spin = gtk::Spinner::builder().spinning(true).build();
    spin.set_size_request(28, 28);
    let label = gtk::Label::builder()
        .label(crate::i18n::t("loading_stickers"))
        .css_classes(["dim-label"])
        .build();
    loading.append(&spin);
    loading.append(&label);
    state.sticker_popover.set_child(Some(&loading));
    state.sticker_popover.popup();

    match state.sidecar.list_stickers() {
        Ok(id) => {
            state.pending.borrow_mut().insert(id, Pending::ListStickers);
        }
        Err(e) => {
            state.sticker_popover.popdown();
            toast(
                state,
                &crate::i18n::tf("sticker_send_failed", &[("error", &e.to_string())]),
            );
        }
    }
}

pub(super) fn fill_sticker_popover(state: &AppState, result: &serde_json::Value) {
    let packs = result
        .get("packs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if packs.is_empty() {
        let empty = gtk::Label::builder()
            .label(crate::i18n::t("stickers_empty"))
            .css_classes(["dim-label"])
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(20)
            .margin_end(20)
            .build();
        state.sticker_popover.set_child(Some(&empty));
        return;
    }

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .css_classes(["line-sticker-chooser"])
        .build();

    let title = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["heading", "line-sticker-pack-title"])
        .margin_start(12)
        .margin_end(12)
        .margin_top(10)
        .margin_bottom(6)
        .build();

    let pages = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .vexpand(true)
        .hexpand(true)
        .css_classes(["line-sticker-pages"])
        .build();

    let tab_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .css_classes(["line-sticker-tabs-scroll"])
        .build();
    let tabs = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .margin_start(8)
        .margin_end(8)
        .margin_top(6)
        .margin_bottom(8)
        .css_classes(["line-sticker-tabs"])
        .build();
    tab_scroll.set_child(Some(&tabs));

    let tab_buttons: Rc<RefCell<Vec<gtk::ToggleButton>>> = Rc::new(RefCell::new(Vec::new()));

    for (idx, pack) in packs.iter().enumerate() {
        let pack_id = pack
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let pack_name = pack
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Stickers")
            .to_string();
        let is_recent = pack
            .get("recent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || pack_id == "__recent__";
        let display_name = if is_recent {
            crate::i18n::t("stickers_recent")
        } else {
            pack_name.clone()
        };

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .css_classes(["line-sticker-scroll"])
            .build();
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(5)
            .min_children_per_line(4)
            .row_spacing(4)
            .column_spacing(4)
            .homogeneous(true)
            .valign(gtk::Align::Start)
            .css_classes(["line-sticker-grid"])
            .build();

        let stickers = pack
            .get("stickers")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        // Lazy-decode: only the first pack loads immediately; others wait until selected.
        let pending_thumbs: Rc<RefCell<Vec<(gtk::Picture, String)>>> =
            Rc::new(RefCell::new(Vec::new()));
        let pack_loaded = Rc::new(RefCell::new(idx == 0));
        for item in stickers {
            let sticker_id = item
                .get("stickerId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let package_id = item
                .get("packageId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let version = item
                .get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if sticker_id.is_empty() || package_id.is_empty() {
                continue;
            }

            let btn = gtk::Button::builder()
                .css_classes(["flat", "line-sticker-cell"])
                .tooltip_text(&display_name)
                .build();
            let pic = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Contain)
                .can_shrink(true)
                .css_classes(["line-sticker-thumb"])
                .build();
            pic.set_size_request(64, 64);
            let path = item
                .get("imagePath")
                .and_then(|v| v.as_str())
                .filter(|p| !p.is_empty())
                .map(|s| s.to_string());
            if idx == 0
                && let Some(path) = path.clone()
            {
                attach_texture_async(pic.clone(), path, 96);
            }
            btn.set_child(Some(&pic));

            let s = state.clone();
            let pop = state.sticker_popover.clone();
            let image_path = path.clone();
            btn.connect_clicked(move |_| {
                pop.popdown();
                send_sticker_now(
                    &s,
                    &sticker_id,
                    &package_id,
                    version.as_deref(),
                    image_path.as_deref(),
                );
            });
            grid.append(&btn);
            if idx != 0
                && let Some(path) = path
            {
                pending_thumbs.borrow_mut().push((pic, path));
            }
        }
        scroll.set_child(Some(&grid));
        let page_name = format!("pack-{idx}");
        pages.add_named(&scroll, Some(&page_name));

        let tab = gtk::ToggleButton::builder()
            .css_classes(["flat", "circular", "line-sticker-tab"])
            .tooltip_text(&display_name)
            .build();
        if is_recent {
            tab.set_icon_name("document-open-recent-symbolic");
        } else {
            let pic = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Contain)
                .can_shrink(true)
                .css_classes(["line-sticker-tab-icon"])
                .build();
            pic.set_size_request(28, 28);
            if let Some(path) = pack.get("iconPath").and_then(|v| v.as_str())
                && !path.is_empty()
            {
                attach_texture_async(pic.clone(), path.to_string(), 48);
            }
            tab.set_child(Some(&pic));
        }
        if idx == 0 {
            tab.set_active(true);
            title.set_text(&display_name);
            pages.set_visible_child_name(&page_name);
        }

        let pages_c = pages.clone();
        let title_c = title.clone();
        let tabs_c = tab_buttons.clone();
        let page_name_c = page_name.clone();
        let display_name_c = display_name.clone();
        let pending_c = pending_thumbs.clone();
        let loaded_c = pack_loaded.clone();
        tab.connect_toggled(move |btn| {
            if !btn.is_active() {
                return;
            }
            for other in tabs_c.borrow().iter() {
                if other != btn && other.is_active() {
                    other.set_active(false);
                }
            }
            title_c.set_text(&display_name_c);
            pages_c.set_visible_child_name(&page_name_c);
            // Decode this pack's thumbs only the first time it is opened.
            if !*loaded_c.borrow() {
                *loaded_c.borrow_mut() = true;
                for (pic, path) in pending_c.borrow_mut().drain(..) {
                    attach_texture_async(pic, path, 96);
                }
            }
        });

        tab_buttons.borrow_mut().push(tab.clone());
        tabs.append(&tab);
    }

    root.append(&title);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&pages);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&tab_scroll);
    state.sticker_popover.set_child(Some(&root));
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
                    ..Default::default()
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
    dialog.open_multiple(
        Some(&state.window),
        None::<&gio::Cancellable>,
        move |result| {
            let Ok(files) = result else {
                return;
            };
            let paths = paths_from_file_model(&files);
            if paths.is_empty() {
                toast(
                    &s,
                    &crate::i18n::tf("media_send_failed", &[("error", "no path")]),
                );
                return;
            }
            open_media_review(&s, paths);
        },
    );
}

fn paths_from_file_model(files: &gio::ListModel) -> Vec<PathBuf> {
    (0..files.n_items())
        .filter_map(|index| files.item(index))
        .filter_map(|item| item.downcast::<gio::File>().ok())
        .filter_map(|file| file.path())
        .filter(|path| path.is_file())
        .collect()
}

fn is_previewable_image(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "avif"
    )
}

fn media_review_icon(path: &std::path::Path) -> &'static str {
    match guess_media_o_type(path) {
        "video" => "video-x-generic-symbolic",
        "audio" => "audio-x-generic-symbolic",
        _ => "text-x-generic-symbolic",
    }
}

fn update_media_review_controls(
    queue: &Rc<RefCell<Vec<PathBuf>>>,
    count: &gtk::Label,
    send: &gtk::Button,
    empty: &gtk::Label,
) {
    let len = queue.borrow().len();
    let n = len.to_string();
    count.set_text(&crate::i18n::tf("media_review_count", &[("n", &n)]));
    send.set_label(&crate::i18n::tf("media_review_send", &[("n", &n)]));
    send.set_sensitive(len > 0);
    empty.set_visible(len == 0);
}

fn append_media_review_item(
    path: PathBuf,
    flow: &gtk::FlowBox,
    queue: &Rc<RefCell<Vec<PathBuf>>>,
    count: &gtk::Label,
    send: &gtk::Button,
    empty: &gtk::Label,
) {
    if !path.is_file() || queue.borrow().contains(&path) {
        return;
    }
    queue.borrow_mut().push(path.clone());

    let preview = gtk::Overlay::builder()
        .width_request(156)
        .height_request(132)
        .css_classes(["line-media-review-preview"])
        .build();
    preview.set_overflow(gtk::Overflow::Hidden);
    if is_previewable_image(&path) {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .width_request(156)
            .height_request(132)
            .css_classes(["line-media-review-thumb"])
            .build();
        attach_texture_async(picture.clone(), path.to_string_lossy().to_string(), 256);
        preview.set_child(Some(&picture));
    } else {
        let icon = gtk::Image::builder()
            .icon_name(media_review_icon(&path))
            .pixel_size(52)
            .css_classes(["dim-label", "line-media-review-file-icon"])
            .build();
        preview.set_child(Some(&icon));
    }

    let remove = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text(crate::i18n::t("media_review_remove"))
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .css_classes(["circular", "line-media-review-remove"])
        .build();
    preview.add_overlay(&remove);

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["line-media-review-card"])
        .tooltip_text(path.to_string_lossy())
        .build();
    card.append(&preview);
    card.append(
        &gtk::Label::builder()
            .label(file_name)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(22)
            .xalign(0.5)
            .css_classes(["caption"])
            .build(),
    );

    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&card));
    flow.append(&child);

    let flow_c = flow.clone();
    let child_c = child.clone();
    let queue_c = queue.clone();
    let count_c = count.clone();
    let send_c = send.clone();
    let empty_c = empty.clone();
    remove.connect_clicked(move |_| {
        queue_c.borrow_mut().retain(|queued| queued != &path);
        flow_c.remove(&child_c);
        update_media_review_controls(&queue_c, &count_c, &send_c, &empty_c);
    });
    update_media_review_controls(queue, count, send, empty);
}

pub(super) fn open_media_review(state: &AppState, paths: Vec<PathBuf>) {
    let window = gtk::Window::builder()
        .title(crate::i18n::t("media_review_title"))
        .transient_for(&state.window)
        .modal(true)
        .default_width(680)
        .default_height(520)
        .css_classes(["line-media-review-window"])
        .build();

    let header = gtk::HeaderBar::builder().show_title_buttons(true).build();
    let title = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(1)
        .build();
    title.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("media_review_title"))
            .css_classes(["heading"])
            .build(),
    );
    title.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("media_review_hint"))
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    header.set_title_widget(Some(&title));

    let add = gtk::Button::builder()
        .label(crate::i18n::t("media_review_add"))
        .tooltip_text(crate::i18n::t("media_review_add"))
        .build();
    header.pack_start(&add);

    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(2)
        .max_children_per_line(4)
        .row_spacing(12)
        .column_spacing(12)
        .homogeneous(true)
        .valign(gtk::Align::Start)
        .css_classes(["line-media-review-grid"])
        .build();
    let empty = gtk::Label::builder()
        .label(crate::i18n::t("media_review_empty"))
        .visible(false)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .css_classes(["dim-label"])
        .build();
    let content = gtk::Overlay::new();
    content.set_child(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&flow)
            .build(),
    ));
    content.add_overlay(&empty);

    let count = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .hexpand(true)
        .css_classes(["dim-label"])
        .build();
    let cancel = gtk::Button::builder()
        .label(crate::i18n::t("cancel"))
        .build();
    let send = gtk::Button::builder()
        .css_classes(["suggested-action"])
        .build();
    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(16)
        .margin_end(16)
        .css_classes(["line-media-review-footer"])
        .build();
    footer.append(&count);
    footer.append(&cancel);
    footer.append(&send);

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["line-media-review"])
        .build();
    root.append(&header);
    root.append(&content);
    root.append(&footer);
    window.set_child(Some(&root));

    let queue = Rc::new(RefCell::new(Vec::new()));
    for path in paths {
        append_media_review_item(path, &flow, &queue, &count, &send, &empty);
    }
    update_media_review_controls(&queue, &count, &send, &empty);

    {
        let window = window.clone();
        cancel.connect_clicked(move |_| window.close());
    }
    {
        let window = window.clone();
        let state = state.clone();
        let queue = queue.clone();
        send.connect_clicked(move |_| {
            let files = queue.borrow().clone();
            if files.is_empty() {
                return;
            }
            window.close();
            for path in files {
                send_local_media_path(&state, path);
            }
        });
    }
    {
        let parent = window.clone();
        let flow = flow.clone();
        let queue = queue.clone();
        let count = count.clone();
        let send = send.clone();
        let empty = empty.clone();
        add.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title(crate::i18n::t("media_review_add"))
                .modal(true)
                .build();
            let flow = flow.clone();
            let queue = queue.clone();
            let count = count.clone();
            let send = send.clone();
            let empty = empty.clone();
            dialog.open_multiple(Some(&parent), None::<&gio::Cancellable>, move |result| {
                let Ok(files) = result else {
                    return;
                };
                for path in paths_from_file_model(&files) {
                    append_media_review_item(path, &flow, &queue, &count, &send, &empty);
                }
            });
        });
    }
    window.present();
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
                        let mut paths = Vec::new();
                        for file in list.files() {
                            if let Some(path) = file.path()
                                && path.is_file()
                            {
                                paths.push(path);
                            }
                        }
                        if paths.is_empty() {
                            paste_clipboard_uri_list(&s);
                        } else {
                            open_media_review(&s, paths);
                        }
                    } else {
                        paste_clipboard_uri_list(&s);
                    }
                }
                Err(_) => paste_clipboard_uri_list(&s),
            },
        );
        return true;
    }

    if has_image {
        let s = state.clone();
        clipboard.read_texture_async(None::<&gio::Cancellable>, move |res| match res {
            Ok(Some(tex)) => match save_clipboard_texture_png(&s, &tex) {
                Ok(path) => open_media_review(&s, vec![path]),
                Err(e) => toast(
                    &s,
                    &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
                ),
            },
            _ => paste_clipboard_image_bytes(&s),
        });
        return true;
    }

    false
}

pub(super) fn paste_clipboard_uri_list(state: &AppState) {
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
            let mut paths = Vec::new();
            for line in buf.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let file = gio::File::for_uri(line);
                if let Some(path) = file.path()
                    && path.is_file()
                {
                    paths.push(path);
                }
            }
            if !paths.is_empty() {
                open_media_review(&s, paths);
            }
        },
    );
}

pub(super) fn paste_clipboard_image_bytes(state: &AppState) {
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
                Ok(path) => open_media_review(&s, vec![path]),
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
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    if !path.is_file() {
        toast(
            state,
            &crate::i18n::tf("media_send_failed", &[("error", "not a file")]),
        );
        return;
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
            return;
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
                    ..Default::default()
                },
            );
            show_upload_progress(state, 0.02, &crate::i18n::t("media_uploading"));
            dismiss_new_marker(state);
            pin_messages_to_latest(state);
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
        ),
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
                    ..Default::default()
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
