use super::*;
use serde::Deserialize;
use std::cell::Cell;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct AlbumInfo {
    pub id: String,
    #[serde(rename = "chatMid")]
    pub chat_mid: String,
    pub title: String,
    #[serde(rename = "photoCount")]
    pub photo_count: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct PhotoInfo {
    pub id: String,
    pub kind: String,
}

#[derive(Deserialize)]
struct AlbumPage {
    albums: Vec<AlbumInfo>,
    cursor: String,
}

#[derive(Deserialize)]
struct PhotoPage {
    photos: Vec<PhotoInfo>,
    cursor: String,
}

#[derive(Clone, Debug)]
pub(super) struct PhotoSlot {
    open: gtk::Button,
    save: gtk::Button,
    retry: gtk::Button,
    caption: gtk::Label,
    cached: Rc<RefCell<Option<(String, String)>>>,
}

#[derive(Clone, Debug)]
pub(super) enum AlbumRequest {
    List {
        cursor: String,
    },
    Photos {
        album: AlbumInfo,
        cursor: String,
    },
    Image {
        album: AlbumInfo,
        photo: PhotoInfo,
        slot: PhotoSlot,
    },
}

pub(super) struct AlbumUi {
    window: gtk::Window,
    toast: libadwaita::ToastOverlay,
    title: gtk::Label,
    context: gtk::Label,
    back: gtk::Button,
    all: gtk::CheckButton,
    refresh: gtk::Button,
    message: gtk::Label,
    spinner: gtk::Spinner,
    more: gtk::Button,
    album_list: gtk::Box,
    photos: gtk::FlowBox,
    scroll: gtk::ScrolledWindow,
    chat_mid: Option<String>,
    chat_name: String,
    chat_names: HashMap<String, String>,
    session_mid: Option<String>,
    album: RefCell<Option<AlbumInfo>>,
    cursor: RefCell<String>,
    cursors: RefCell<HashSet<String>>,
    keys: RefCell<HashSet<String>>,
    generation: Cell<u64>,
    closed: Cell<bool>,
    busy: Cell<bool>,
    count: Cell<usize>,
    active_images: Cell<usize>,
    queue: RefCell<VecDeque<AlbumRequest>>,
}

pub(super) fn wire(state: &AppState) {
    let s = state.clone();
    state.album_btn.connect_clicked(move |_| open(&s));
}

pub(super) fn close(state: &AppState) {
    let ui = state.album_ui.borrow_mut().take();
    if let Some(ui) = ui {
        ui.closed.set(true);
        ui.queue.borrow_mut().clear();
        ui.window.close();
    }
}

fn open(state: &AppState) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    if let Some(ui) = state.album_ui.borrow().as_ref() {
        ui.window.present();
        return;
    }
    let chat_mid = state.current_chat.borrow().clone();
    let chat_name = chat_mid
        .as_ref()
        .map(|mid| peer_display_name(state, mid))
        .unwrap_or_default();
    let window = gtk::Window::builder()
        .transient_for(&state.window)
        .destroy_with_parent(true)
        .modal(true)
        .title(crate::i18n::t("albums"))
        .default_width(760)
        .default_height(660)
        .build();
    let title = gtk::Label::builder()
        .label(crate::i18n::t("albums"))
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(36)
        .css_classes(["heading"])
        .build();
    let context = gtk::Label::builder()
        .css_classes(["dim-label", "caption"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let title_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    title_box.append(&title);
    title_box.append(&context);
    let header = libadwaita::HeaderBar::builder()
        .title_widget(&title_box)
        .build();
    let back = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text(crate::i18n::t("album_back"))
        .visible(false)
        .build();
    let refresh = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(crate::i18n::t("album_refresh"))
        .build();
    header.pack_start(&back);
    header.pack_end(&refresh);
    let all = gtk::CheckButton::builder()
        .label(crate::i18n::t("album_all_chats"))
        .active(chat_mid.is_none())
        .visible(chat_mid.is_some())
        .build();
    let controls = gtk::Box::builder()
        .spacing(12)
        .margin_start(16)
        .margin_end(16)
        .margin_top(10)
        .margin_bottom(10)
        .build();
    let spinner = gtk::Spinner::new();
    let message = gtk::Label::builder()
        .hexpand(true)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();
    controls.append(&all);
    controls.append(&spinner);
    controls.append(&message);
    let album_list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    let photos = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(4)
        .column_spacing(12)
        .row_spacing(12)
        .valign(gtk::Align::Start)
        .visible(false)
        .build();
    let more = gtk::Button::builder()
        .label(crate::i18n::t("album_more"))
        .halign(gtk::Align::Center)
        .visible(false)
        .margin_top(16)
        .build();
    let contents = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_start(16)
        .margin_end(16)
        .margin_top(4)
        .margin_bottom(20)
        .build();
    contents.append(&album_list);
    contents.append(&photos);
    contents.append(&more);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&contents)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.set_titlebar(Some(&header));
    root.append(&controls);
    root.append(&scroll);
    let overlay = libadwaita::ToastOverlay::new();
    overlay.set_child(Some(&root));
    window.set_child(Some(&overlay));
    let ui = Rc::new(AlbumUi {
        window,
        toast: overlay,
        title,
        context,
        back,
        all,
        refresh,
        message,
        spinner,
        more,
        album_list,
        photos,
        scroll,
        chat_mid,
        chat_name,
        chat_names: state
            .chats
            .borrow()
            .iter()
            .map(|chat| (chat.mid.clone(), chat.name.clone()))
            .collect(),
        session_mid: state.self_mid.borrow().clone(),
        album: RefCell::new(None),
        cursor: RefCell::new(String::new()),
        cursors: RefCell::new(HashSet::new()),
        keys: RefCell::new(HashSet::new()),
        generation: Cell::new(0),
        closed: Cell::new(false),
        busy: Cell::new(false),
        count: Cell::new(0),
        active_images: Cell::new(0),
        queue: RefCell::new(VecDeque::new()),
    });
    *state.album_ui.borrow_mut() = Some(ui.clone());
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.window.connect_close_request(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.closed.set(true);
            ui.queue.borrow_mut().clear();
            s.album_ui.borrow_mut().take();
        }
        glib::Propagation::Proceed
    });
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.all.connect_toggled(move |_| {
        if let Some(ui) = weak.upgrade() {
            reset(&ui, None);
            load_page(&s, &ui);
        }
    });
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.back.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            reset(&ui, None);
            load_page(&s, &ui);
        }
    });
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.refresh.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            let album = ui.album.borrow().clone();
            reset(&ui, album);
            load_page(&s, &ui);
        }
    });
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.more.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            load_page(&s, &ui);
        }
    });
    ui.window.present();
    reset(&ui, None);
    load_page(state, &ui);
}

fn reset(ui: &AlbumUi, album: Option<AlbumInfo>) {
    ui.generation.set(ui.generation.get().wrapping_add(1));
    ui.queue.borrow_mut().clear();
    ui.active_images.set(0);
    ui.busy.set(false);
    ui.count.set(0);
    ui.keys.borrow_mut().clear();
    ui.cursors.borrow_mut().clear();
    ui.cursor.borrow_mut().clear();
    while let Some(child) = ui.album_list.first_child() {
        ui.album_list.remove(&child);
    }
    while let Some(child) = ui.photos.first_child() {
        ui.photos.remove(&child);
    }
    ui.back.set_visible(album.is_some());
    ui.all.set_visible(album.is_none() && ui.chat_mid.is_some());
    ui.album_list.set_visible(album.is_none());
    ui.photos.set_visible(album.is_some());
    ui.title.set_text(
        &album
            .as_ref()
            .map(album_title)
            .unwrap_or_else(|| crate::i18n::t("albums")),
    );
    let context = if let Some(album) = album.as_ref() {
        ui.chat_names
            .get(&album.chat_mid)
            .cloned()
            .unwrap_or_else(|| crate::i18n::t("album_other_chat"))
    } else if ui.all.is_active() {
        crate::i18n::t("album_all_chats")
    } else {
        ui.chat_name.clone()
    };
    ui.context.set_text(&context);
    *ui.album.borrow_mut() = album;
    ui.more.set_visible(false);
    ui.scroll.vadjustment().set_value(0.0);
}

fn load_page(state: &AppState, ui: &Rc<AlbumUi>) {
    if ui.closed.get() || ui.busy.replace(true) {
        return;
    }
    ui.spinner.set_spinning(true);
    ui.spinner.set_visible(true);
    ui.more.set_sensitive(false);
    ui.refresh.set_sensitive(false);
    ui.message.set_text(&crate::i18n::t("album_loading"));
    let cursor = ui.cursor.borrow().clone();
    let request = if let Some(album) = ui.album.borrow().clone() {
        AlbumRequest::Photos { album, cursor }
    } else {
        AlbumRequest::List { cursor }
    };
    send(state, ui, request);
}

fn send(state: &AppState, ui: &Rc<AlbumUi>, request: AlbumRequest) {
    let (method, params) = match &request {
        AlbumRequest::List { cursor } => (
            "list_albums",
            serde_json::json!({"cursor": cursor, "chatMid": if ui.all.is_active() { None } else { ui.chat_mid.as_ref() }}),
        ),
        AlbumRequest::Photos { album, cursor } => (
            "list_album_photos",
            serde_json::json!({"cursor": cursor, "chatMid": album.chat_mid, "albumId": album.id}),
        ),
        AlbumRequest::Image { album, photo, .. } => (
            "download_album_photo",
            serde_json::json!({"chatMid": album.chat_mid, "albumId": album.id, "photoId": photo.id}),
        ),
    };
    match state.sidecar.request(method, Some(params)) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::Album {
                    ui: Rc::downgrade(ui),
                    generation: ui.generation.get(),
                    request,
                },
            );
        }
        Err(_) => response(
            state,
            ui,
            ui.generation.get(),
            request,
            false,
            &serde_json::Value::Null,
            Some("album_request_failed"),
        ),
    }
}

fn album_title(album: &AlbumInfo) -> String {
    if album.title.trim().is_empty() {
        crate::i18n::t("album_untitled")
    } else {
        album.title.clone()
    }
}

fn page_finished(ui: &AlbumUi, input: String, next: String) {
    let mut cursors = ui.cursors.borrow_mut();
    cursors.insert(input);
    let next = if cursors.contains(&next) {
        String::new()
    } else {
        next
    };
    ui.more.set_visible(!next.is_empty());
    *ui.cursor.borrow_mut() = next;
    ui.more.set_label(&crate::i18n::t("album_more"));
    ui.more.set_sensitive(true);
    ui.refresh.set_sensitive(true);
    ui.busy.set(false);
    ui.spinner.set_spinning(false);
    ui.spinner.set_visible(false);
    ui.message.set_text(&if ui.count.get() == 0 {
        crate::i18n::t(if ui.more.is_visible() {
            "album_empty_page"
        } else if ui.album.borrow().is_some() {
            "album_no_photos"
        } else {
            "album_empty"
        })
    } else {
        crate::i18n::tf("album_loaded", &[("n", &ui.count.get().to_string())])
    });
}

pub(super) fn response(
    state: &AppState,
    ui: &Rc<AlbumUi>,
    generation: u64,
    request: AlbumRequest,
    ok: bool,
    result: &serde_json::Value,
    error: Option<&str>,
) {
    if ui.closed.get()
        || generation != ui.generation.get()
        || ui.session_mid != *state.self_mid.borrow()
        || !*state.session_ready.borrow()
    {
        return;
    }
    let parsed = if ok {
        match &request {
            AlbumRequest::List { .. } => {
                serde_json::from_value::<AlbumPage>(result.clone()).is_ok()
            }
            AlbumRequest::Photos { .. } => {
                serde_json::from_value::<PhotoPage>(result.clone()).is_ok()
            }
            AlbumRequest::Image { .. } => {
                result
                    .get("path")
                    .and_then(|v| v.as_str())
                    .is_some_and(|path| std::path::Path::new(path).is_file())
                    && result.get("name").and_then(|v| v.as_str()).is_some()
            }
        }
    } else {
        false
    };
    if !parsed {
        let code = error
            .filter(|code| {
                matches!(
                    *code,
                    "album_not_logged_in"
                        | "album_session_changed"
                        | "album_photo_expired"
                        | "album_image_unsupported"
                        | "album_image_too_large"
                )
            })
            .unwrap_or("album_request_failed");
        if let AlbumRequest::Image { slot, .. } = request {
            slot.caption.set_text(&crate::i18n::t(code));
            slot.retry.set_visible(true);
            slot.retry.set_sensitive(true);
            ui.active_images
                .set(ui.active_images.get().saturating_sub(1));
            pump_images(state, ui);
        } else {
            ui.busy.set(false);
            ui.refresh.set_sensitive(true);
            ui.spinner.set_spinning(false);
            ui.spinner.set_visible(false);
            ui.message.set_text(&crate::i18n::t(code));
            ui.more.set_label(&crate::i18n::t("album_retry"));
            ui.more.set_visible(true);
            ui.more.set_sensitive(true);
        }
        return;
    }
    match request {
        AlbumRequest::List { cursor } => {
            let page: AlbumPage = serde_json::from_value(result.clone()).unwrap();
            for album in page.albums {
                if !ui
                    .keys
                    .borrow_mut()
                    .insert(format!("{}:{}", album.chat_mid, album.id))
                {
                    continue;
                }
                append_album(state, ui, album);
                ui.count.set(ui.count.get() + 1);
            }
            page_finished(ui, cursor, page.cursor);
        }
        AlbumRequest::Photos { album, cursor } => {
            let page: PhotoPage = serde_json::from_value(result.clone()).unwrap();
            for photo in page.photos {
                if !ui.keys.borrow_mut().insert(photo.id.clone()) {
                    continue;
                }
                ui.count.set(ui.count.get() + 1);
                append_photo(state, ui, &album, photo, ui.count.get());
            }
            page_finished(ui, cursor, page.cursor);
            pump_images(state, ui);
        }
        AlbumRequest::Image { slot, .. } => {
            let path = result["path"].as_str().unwrap().to_string();
            let name = result["name"].as_str().unwrap().to_string();
            let frame = gtk::Box::builder()
                .width_request(144)
                .height_request(116)
                .build();
            let overlay = gtk::Overlay::builder().child(&frame).build();
            let picture = gtk::Picture::builder()
                .can_shrink(true)
                .content_fit(gtk::ContentFit::Cover)
                .build();
            overlay.add_overlay(&picture);
            overlay.set_measure_overlay(&picture, false);
            overlay.set_clip_overlay(&picture, true);
            attach_texture_async(picture, path.clone(), 320);
            slot.open.set_child(Some(&overlay));
            label_photo_action(&slot.open);
            slot.open.set_sensitive(true);
            slot.save.set_sensitive(true);
            slot.retry.set_visible(false);
            slot.caption.set_text("");
            *slot.cached.borrow_mut() = Some((path, name));
            ui.active_images
                .set(ui.active_images.get().saturating_sub(1));
            pump_images(state, ui);
        }
    }
}

fn append_album(state: &AppState, ui: &Rc<AlbumUi>, album: AlbumInfo) {
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 4);
    labels.append(
        &gtk::Label::builder()
            .label(album_title(&album))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build(),
    );
    let chat_name = ui
        .chat_names
        .get(&album.chat_mid)
        .cloned()
        .unwrap_or_else(|| crate::i18n::t("album_other_chat"));
    labels.append(
        &gtk::Label::builder()
            .label(format!(
                "{} · {}",
                chat_name,
                crate::i18n::tf(
                    "album_photo_count",
                    &[("n", &album.photo_count.to_string())]
                )
            ))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label"])
            .build(),
    );
    let row = gtk::Box::builder()
        .spacing(12)
        .margin_start(8)
        .margin_end(8)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    row.append(&gtk::Image::from_icon_name("folder-pictures-symbolic"));
    labels.set_hexpand(true);
    row.append(&labels);
    row.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    let button = gtk::Button::builder().child(&row).build();
    button.update_property(&[gtk::accessible::Property::Label(&album_title(&album))]);
    let s = state.clone();
    let weak = Rc::downgrade(ui);
    button.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            reset(&ui, Some(album.clone()));
            load_page(&s, &ui);
        }
    });
    ui.album_list.append(&button);
}

fn append_photo(
    state: &AppState,
    ui: &Rc<AlbumUi>,
    album: &AlbumInfo,
    photo: PhotoInfo,
    index: usize,
) {
    let title = crate::i18n::tf("album_photo", &[("n", &index.to_string())]);
    let open = gtk::Button::builder()
        .label(crate::i18n::t("album_loading"))
        .tooltip_text(&title)
        .css_classes(["line-album-photo"])
        .width_request(152)
        .height_request(124)
        .sensitive(false)
        .build();
    open.update_property(&[gtk::accessible::Property::Label(&title)]);
    let save = gtk::Button::builder()
        .label(crate::i18n::t("album_save"))
        .tooltip_text(crate::i18n::tf(
            "album_save_photo",
            &[("n", &index.to_string())],
        ))
        .sensitive(false)
        .build();
    save.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tf(
        "album_save_photo",
        &[("n", &index.to_string())],
    ))]);
    let retry = gtk::Button::builder()
        .label(crate::i18n::t("album_retry"))
        .visible(false)
        .build();
    label_photo_action(&open);
    label_photo_action(&save);
    let caption = gtk::Label::builder()
        .wrap(true)
        .max_width_chars(18)
        .css_classes(["dim-label", "caption"])
        .build();
    let tile = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .valign(gtk::Align::Start)
        .build();
    tile.append(&open);
    tile.append(&gtk::Label::new(Some(&title)));
    tile.append(&caption);
    let actions = gtk::Box::builder()
        .spacing(4)
        .halign(gtk::Align::Center)
        .build();
    actions.append(&save);
    actions.append(&retry);
    tile.append(&actions);
    ui.photos.insert(&tile, -1);
    if photo.kind != "image" {
        open.set_label(&crate::i18n::t("album_video"));
        save.set_visible(false);
        caption.set_text(&crate::i18n::t("album_video_unsupported"));
        return;
    }
    let slot = PhotoSlot {
        open,
        save,
        retry,
        caption,
        cached: Rc::new(RefCell::new(None)),
    };
    let s = state.clone();
    let weak = Rc::downgrade(ui);
    let cached = slot.cached.clone();
    slot.save.connect_clicked(move |_| {
        if let (Some(ui), Some((path, name))) = (weak.upgrade(), cached.borrow().clone()) {
            save_photo(&s, &ui, &ui.window, &path, &name);
        }
    });
    let s = state.clone();
    let weak = Rc::downgrade(ui);
    let cached = slot.cached.clone();
    slot.open.connect_clicked(move |_| {
        if let (Some(ui), Some((path, name))) = (weak.upgrade(), cached.borrow().clone()) {
            let s2 = s.clone();
            let weak2 = Rc::downgrade(&ui);
            let save_path = path.clone();
            let save_name = name.clone();
            media::open_media_viewer_for(
                &s,
                &ui.window,
                &path,
                "IMAGE",
                &name,
                Some(Rc::new(move |parent| {
                    if let Some(ui) = weak2.upgrade() {
                        save_photo(&s2, &ui, parent, &save_path, &save_name);
                    }
                })),
            );
        }
    });
    let s = state.clone();
    let weak = Rc::downgrade(ui);
    let retry_album = album.clone();
    let retry_photo = photo.clone();
    let retry_open = slot.open.downgrade();
    let retry_save = slot.save.downgrade();
    let retry_caption = slot.caption.downgrade();
    let retry_cached = slot.cached.clone();
    slot.retry.connect_clicked(move |button| {
        if let (Some(ui), Some(open), Some(save), Some(caption)) = (
            weak.upgrade(),
            retry_open.upgrade(),
            retry_save.upgrade(),
            retry_caption.upgrade(),
        ) {
            button.set_sensitive(false);
            ui.queue.borrow_mut().push_back(AlbumRequest::Image {
                album: retry_album.clone(),
                photo: retry_photo.clone(),
                slot: PhotoSlot {
                    open,
                    save,
                    retry: button.clone(),
                    caption,
                    cached: retry_cached.clone(),
                },
            });
            pump_images(&s, &ui);
        }
    });
    ui.queue.borrow_mut().push_back(AlbumRequest::Image {
        album: album.clone(),
        photo,
        slot,
    });
}

fn label_photo_action(button: &gtk::Button) {
    // GtkButtonが本文ラベルへ自動設定する関連は、明示した読み上げ名より優先される。
    button.reset_relation(gtk::AccessibleRelation::LabelledBy);
    if let Some(label) = button.tooltip_text() {
        button.update_property(&[gtk::accessible::Property::Label(&label)]);
    }
}

fn pump_images(state: &AppState, ui: &Rc<AlbumUi>) {
    while !ui.closed.get() && ui.active_images.get() < 3 {
        let request = ui.queue.borrow_mut().pop_front();
        let Some(request) = request else {
            break;
        };
        ui.active_images.set(ui.active_images.get() + 1);
        send(state, ui, request);
    }
}

fn save_photo(
    state: &AppState,
    ui: &Rc<AlbumUi>,
    parent: &impl IsA<gtk::Window>,
    path: &str,
    name: &str,
) {
    let cfg = state.config.borrow().clone();
    let dir = cfg.download_dir_for("IMAGE");
    if std::fs::create_dir_all(&dir).is_err() {
        ui.toast
            .add_toast(libadwaita::Toast::new(&crate::i18n::t("album_save_failed")));
        return;
    }
    if !cfg.download_ask_every_time {
        copy_photo(
            ui,
            path,
            &downloads::unique_download_dest(&dir, name),
            false,
        );
        return;
    }
    let dialog = gtk::FileDialog::builder()
        .title(crate::i18n::t("media_save_as"))
        .initial_name(name)
        .initial_folder(&gio::File::for_path(dir))
        .build();
    let weak = Rc::downgrade(ui);
    let path = path.to_string();
    dialog.save(Some(parent), None::<&gio::Cancellable>, move |result| {
        if let (Some(ui), Ok(file)) = (weak.upgrade(), result)
            && !ui.closed.get()
            && let Some(dest) = file.path()
        {
            copy_photo(&ui, &path, &dest, true);
        }
    });
}

fn copy_photo(ui: &Rc<AlbumUi>, path: &str, dest: &std::path::Path, overwrite: bool) {
    let destination = dest.to_path_buf();
    let weak = Rc::downgrade(ui);
    gio::File::for_path(path).copy_async(
        &gio::File::for_path(dest),
        if overwrite {
            gio::FileCopyFlags::OVERWRITE
        } else {
            gio::FileCopyFlags::NONE
        },
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        None,
        move |result| {
            if let Some(ui) = weak.upgrade()
                && !ui.closed.get()
            {
                let message = if result.is_ok() {
                    crate::i18n::tf(
                        "media_saved",
                        &[("path", &destination.display().to_string())],
                    )
                } else {
                    crate::i18n::t("album_save_failed")
                };
                ui.toast.add_toast(libadwaita::Toast::new(&message));
            }
        },
    );
}
