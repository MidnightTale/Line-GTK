use super::*;
use crate::protocol::{SharedMediaDownload, SharedMediaItem, SharedMediaPage};
use std::cell::Cell;
use std::time::Duration;

const CATEGORIES: [&str; 3] = ["photos_videos", "links", "files"];

#[derive(Clone, Debug)]
pub(super) enum SharedMediaRequest {
    List {
        category: usize,
        cursor: String,
        reset_scroll: bool,
    },
    Download {
        item: Box<SharedMediaItem>,
        save: bool,
    },
}

#[derive(Default)]
struct TabPage {
    cursors: Vec<String>,
    result: Option<SharedMediaPage>,
    scroll: f64,
}

impl TabPage {
    fn cursor(&self) -> &str {
        self.cursors.last().map(String::as_str).unwrap_or("")
    }

    fn next(&mut self) -> bool {
        let Some(cursor) = self.result.as_ref().map(|page| page.next_cursor.clone()) else {
            return false;
        };
        if cursor.is_empty() || cursor == self.cursor() || self.cursors.contains(&cursor) {
            return false;
        }
        self.cursors.push(cursor);
        self.result = None;
        self.scroll = 0.0;
        true
    }
}

pub(super) struct SharedMediaUi {
    window: gtk::Window,
    toast: libadwaita::ToastOverlay,
    contents: gtk::Box,
    scroll: gtk::ScrolledWindow,
    status: gtk::Label,
    spinner: gtk::Spinner,
    previous: gtk::Button,
    next: gtk::Button,
    page_label: gtk::Label,
    chat_mid: String,
    account: Option<String>,
    category: Cell<usize>,
    pages: RefCell<[TabPage; 3]>,
    generation: Cell<u64>,
    busy: Cell<bool>,
    closed: Cell<bool>,
    downloads: RefCell<HashSet<(String, bool)>>,
}

pub(super) fn open(state: &AppState) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    let existing = state.shared_media_ui.borrow().clone();
    if let Some(ui) = existing {
        if ui.chat_mid == chat_mid && ui.account == *state.self_mid.borrow() {
            ui.window.present();
            load_page(state, &ui, false, false);
            return;
        }
        close(state);
    }
    let window = gtk::Window::builder()
        .transient_for(&state.window)
        .destroy_with_parent(true)
        .title(crate::i18n::t("shared_media"))
        .default_width(760)
        .default_height(680)
        .build();
    let title = gtk::Box::new(gtk::Orientation::Vertical, 2);
    title.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("shared_media"))
            .css_classes(["heading"])
            .build(),
    );
    title.append(
        &gtk::Label::builder()
            .label(peer_display_name(state, &chat_mid))
            .max_width_chars(35)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label", "caption"])
            .build(),
    );
    let header = libadwaita::HeaderBar::builder()
        .title_widget(&title)
        .build();
    let refresh = icon_button("view-refresh-symbolic", "shared_media_refresh");
    header.pack_end(&refresh);
    let tabs = gtk::Box::builder()
        .halign(gtk::Align::Center)
        .css_classes(["linked"])
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let tab_buttons: Vec<_> = [
        "shared_media_photos_videos",
        "shared_media_links",
        "shared_media_files",
    ]
    .iter()
    .map(|key| gtk::ToggleButton::with_label(&crate::i18n::t(key)))
    .collect();
    for (index, button) in tab_buttons.iter().enumerate() {
        if index > 0 {
            button.set_group(Some(&tab_buttons[0]));
        }
        tabs.append(button);
    }
    tab_buttons[0].set_active(true);
    let status = gtk::Label::builder()
        .label(crate::i18n::t("shared_media_loading"))
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .build();
    let spinner = gtk::Spinner::new();
    let feedback = gtk::Box::builder()
        .spacing(8)
        .margin_start(16)
        .margin_end(16)
        .margin_bottom(8)
        .build();
    feedback.append(&spinner);
    feedback.append(&status);
    let contents = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(16)
        .margin_end(16)
        .margin_bottom(16)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&contents)
        .build();
    let previous = gtk::Button::with_label(&crate::i18n::t("shared_media_previous"));
    let next = gtk::Button::with_label(&crate::i18n::t("shared_media_older"));
    let page_label = gtk::Label::builder().hexpand(true).build();
    let footer = gtk::Box::builder()
        .spacing(12)
        .margin_start(16)
        .margin_end(16)
        .margin_top(8)
        .margin_bottom(12)
        .build();
    footer.append(&previous);
    footer.append(&page_label);
    footer.append(&next);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.set_titlebar(Some(&header));
    root.append(&tabs);
    root.append(&feedback);
    root.append(&scroll);
    root.append(&footer);
    let overlay = libadwaita::ToastOverlay::new();
    overlay.set_child(Some(&root));
    window.set_child(Some(&overlay));
    let ui = Rc::new(SharedMediaUi {
        window,
        toast: overlay,
        contents,
        scroll,
        status,
        spinner,
        previous,
        next,
        page_label,
        chat_mid,
        account: state.self_mid.borrow().clone(),
        category: Cell::new(0),
        pages: RefCell::new(std::array::from_fn(|_| TabPage::default())),
        generation: Cell::new(0),
        busy: Cell::new(false),
        closed: Cell::new(false),
        downloads: RefCell::new(HashSet::new()),
    });
    *state.shared_media_ui.borrow_mut() = Some(ui.clone());
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.window.connect_close_request(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.closed.set(true);
            let _ = s.sidecar.stop_shared_media(&ui.chat_mid);
            s.shared_media_ui.borrow_mut().take();
        }
        glib::Propagation::Proceed
    });
    for (index, button) in tab_buttons.iter().enumerate() {
        let s = state.clone();
        let weak = Rc::downgrade(&ui);
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            if let Some(ui) = weak.upgrade() {
                ui.pages.borrow_mut()[ui.category.get()].scroll = ui.scroll.vadjustment().value();
                ui.category.set(index);
                invalidate_request(&ui);
                render(&s, &ui, false);
                load_page(&s, &ui, false, false);
            }
        });
    }
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    refresh.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            invalidate_request(&ui);
            load_page(&s, &ui, true, false);
        }
    });
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.previous.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            let category = ui.category.get();
            if ui.pages.borrow_mut()[category].cursors.pop().is_some() {
                invalidate_request(&ui);
                load_page(&s, &ui, false, true);
            }
        }
    });
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.next.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            let category = ui.category.get();
            if ui.pages.borrow_mut()[category].next() {
                invalidate_request(&ui);
                load_page(&s, &ui, false, true);
            }
        }
    });
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&ui);
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            if let Some(ui) = weak.upgrade() {
                ui.window.close();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    ui.window.add_controller(keys);
    // Polling also recovers a coalesced/missed change notification without
    // retaining widgets or decoded textures for the complete disk index.
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    glib::timeout_add_local(Duration::from_secs(3), move || {
        let Some(ui) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if ui.closed.get() || ui.account != *s.self_mid.borrow() {
            return glib::ControlFlow::Break;
        }
        let needs_update = ui.pages.borrow()[ui.category.get()]
            .result
            .as_ref()
            .is_none_or(|page| {
                page.error.is_none()
                    && (page.busy || page.scan_state == "more" || page.pending_count > 0)
            });
        if ui.window.is_visible() && needs_update {
            load_page(&s, &ui, false, false);
        }
        glib::ControlFlow::Continue
    });
    ui.window.present();
    load_page(state, &ui, false, true);
}

pub(super) fn close(state: &AppState) {
    let ui = state.shared_media_ui.borrow_mut().take();
    if let Some(ui) = ui {
        ui.closed.set(true);
        let _ = state.sidecar.stop_shared_media(&ui.chat_mid);
        ui.window.close();
    }
}

fn invalidate_request(ui: &SharedMediaUi) {
    ui.generation.set(ui.generation.get().wrapping_add(1));
    ui.busy.set(false);
}

fn load_page(state: &AppState, ui: &Rc<SharedMediaUi>, refresh: bool, reset_scroll: bool) {
    if ui.closed.get() || ui.busy.replace(true) {
        return;
    }
    let category = ui.category.get();
    let cursor = ui.pages.borrow()[category].cursor().to_string();
    let request = SharedMediaRequest::List {
        category,
        cursor: cursor.clone(),
        reset_scroll,
    };
    ui.spinner.set_spinning(true);
    ui.previous.set_sensitive(false);
    ui.next.set_sensitive(false);
    let sent =
        state
            .sidecar
            .list_shared_media(&ui.chat_mid, CATEGORIES[category], &cursor, refresh);
    enqueue(state, ui, request, sent);
}

fn enqueue(
    state: &AppState,
    ui: &Rc<SharedMediaUi>,
    request: SharedMediaRequest,
    sent: anyhow::Result<u64>,
) {
    match sent {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::SharedMedia {
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
            None,
        ),
    }
}

pub(super) fn response(
    state: &AppState,
    ui: &Rc<SharedMediaUi>,
    generation: u64,
    request: SharedMediaRequest,
    ok: bool,
    result: &serde_json::Value,
    _error: Option<&str>,
) {
    if ui.closed.get() || ui.account != *state.self_mid.borrow() || !*state.session_ready.borrow() {
        return;
    }
    match request {
        SharedMediaRequest::List {
            category,
            cursor,
            reset_scroll,
        } => {
            if generation != ui.generation.get()
                || category != ui.category.get()
                || cursor != ui.pages.borrow()[category].cursor()
            {
                return;
            }
            ui.busy.set(false);
            ui.spinner.set_spinning(false);
            let parsed = ok
                .then(|| serde_json::from_value::<SharedMediaPage>(result.clone()).ok())
                .flatten();
            let Some(mut page) = parsed.filter(|page| {
                page.chat_mid == ui.chat_mid && page.category == CATEGORIES[category]
            }) else {
                ui.status.set_text(&crate::i18n::t("shared_media_failed"));
                update_navigation(ui);
                return;
            };
            page.items.retain(|item| {
                item.chat_mid == ui.chat_mid && !is_unsent(state, ui, &item.message_id)
            });
            // Server and UI limits are independent safeguards; cached records on
            // disk are never truncated by this rendering window.
            page.items.truncate(60);
            let changed = ui.pages.borrow()[category]
                .result
                .as_ref()
                .is_none_or(|old| old.items != page.items);
            if !reset_scroll {
                ui.pages.borrow_mut()[category].scroll = ui.scroll.vadjustment().value();
            } else {
                ui.pages.borrow_mut()[category].scroll = 0.0;
            }
            ui.pages.borrow_mut()[category].result = Some(page);
            if changed || reset_scroll {
                render(state, ui, reset_scroll);
            } else {
                update_status(ui);
                update_navigation(ui);
            }
        }
        SharedMediaRequest::Download { item, save } => {
            ui.downloads
                .borrow_mut()
                .remove(&(item.message_id.clone(), save));
            if is_unsent(state, ui, &item.message_id) {
                return;
            }
            let downloaded = ok
                .then(|| serde_json::from_value::<SharedMediaDownload>(result.clone()).ok())
                .flatten();
            if let Some(downloaded) =
                downloaded.filter(|download| std::path::Path::new(&download.path).is_file())
            {
                if save {
                    save_media_as(
                        state,
                        &downloaded.path,
                        &downloaded.file_name,
                        &downloaded.content_type,
                    );
                } else {
                    open_media_viewer_for(
                        state,
                        &ui.window,
                        &downloaded.path,
                        &downloaded.content_type,
                        &downloaded.file_name,
                        None,
                    );
                }
            } else {
                ui.toast.add_toast(libadwaita::Toast::new(&crate::i18n::t(
                    "shared_media_original_failed",
                )));
            }
            load_page(state, ui, false, false);
        }
    }
}

fn is_unsent(state: &AppState, ui: &SharedMediaUi, id: &str) -> bool {
    state
        .unsent_ids
        .borrow()
        .contains(&(ui.account.clone().unwrap_or_default(), id.into()))
}

pub(super) fn changed(state: &AppState, chat_mid: &str) {
    let ui = state.shared_media_ui.borrow().clone();
    if let Some(ui) = ui.filter(|ui| ui.chat_mid == chat_mid && ui.window.is_visible()) {
        load_page(state, &ui, false, false);
    }
}

pub(super) fn unsent(state: &AppState, chat_mid: &str, message_id: &str) {
    let ui = state.shared_media_ui.borrow().clone();
    if let Some(ui) = ui.filter(|ui| ui.chat_mid == chat_mid) {
        for tab in ui.pages.borrow_mut().iter_mut() {
            if let Some(page) = &mut tab.result {
                page.items.retain(|item| item.message_id != message_id);
            }
        }
        render(state, &ui, false);
    }
}

fn update_navigation(ui: &SharedMediaUi) {
    let pages = ui.pages.borrow();
    let tab = &pages[ui.category.get()];
    ui.previous
        .set_sensitive(!ui.busy.get() && !tab.cursors.is_empty());
    ui.next.set_sensitive(
        !ui.busy.get()
            && tab
                .result
                .as_ref()
                .is_some_and(|page| !page.next_cursor.is_empty()),
    );
    ui.page_label.set_text(&crate::i18n::tf(
        "shared_media_page",
        &[("n", &(tab.cursors.len() + 1).to_string())],
    ));
}

fn update_status(ui: &SharedMediaUi) {
    let pages = ui.pages.borrow();
    let Some(page) = &pages[ui.category.get()].result else {
        ui.status.set_text(&crate::i18n::t("shared_media_loading"));
        return;
    };
    let key = if page.error.is_some() {
        "shared_media_partial"
    } else if page.busy || page.scan_state == "more" || page.pending_count > 0 {
        "shared_media_caching"
    } else if page.items.is_empty() {
        "shared_media_empty"
    } else {
        "shared_media_complete"
    };
    ui.status.set_text(&crate::i18n::tf(
        key,
        &[
            ("cached", &page.cached_count.to_string()),
            ("pending", &page.pending_count.to_string()),
            ("failed", &page.failed_count.to_string()),
        ],
    ));
    ui.spinner.set_spinning(page.busy);
}

fn render(state: &AppState, ui: &Rc<SharedMediaUi>, reset_scroll: bool) {
    while let Some(child) = ui.contents.first_child() {
        ui.contents.remove(&child);
    }
    let category = ui.category.get();
    let (items, position) = {
        let pages = ui.pages.borrow();
        (
            pages[category]
                .result
                .as_ref()
                .map(|page| page.items.clone())
                .unwrap_or_default(),
            pages[category].scroll,
        )
    };
    let mut month = String::new();
    let mut flow: Option<gtk::FlowBox> = None;
    for item in items {
        let key = month_key(item.created_time);
        if key != month || ui.contents.first_child().is_none() {
            month = key;
            ui.contents.append(
                &gtk::Label::builder()
                    .label(if month.is_empty() {
                        crate::i18n::t("shared_media_unknown_date")
                    } else {
                        month.clone()
                    })
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .margin_top(8)
                    .build(),
            );
            flow = None;
            if category == 0 {
                let grid = gtk::FlowBox::builder()
                    .selection_mode(gtk::SelectionMode::None)
                    .homogeneous(true)
                    .min_children_per_line(1)
                    .max_children_per_line(4)
                    .column_spacing(8)
                    .row_spacing(8)
                    .valign(gtk::Align::Start)
                    .build();
                ui.contents.append(&grid);
                flow = Some(grid);
            }
        }
        let widget = item_widget(state, ui, item, category == 0);
        if let Some(flow) = &flow {
            flow.insert(&widget, -1);
        } else {
            ui.contents.append(&widget);
        }
    }
    update_status(ui);
    update_navigation(ui);
    let adjustment = ui.scroll.vadjustment();
    let weak = Rc::downgrade(ui);
    let generation = ui.generation.get();
    glib::idle_add_local_once(move || {
        if weak.upgrade().is_some_and(|ui| {
            !ui.closed.get() && ui.generation.get() == generation && ui.category.get() == category
        }) {
            adjustment.set_value(if reset_scroll { 0.0 } else { position });
        }
    });
}

fn item_widget(
    state: &AppState,
    ui: &Rc<SharedMediaUi>,
    item: SharedMediaItem,
    visual: bool,
) -> gtk::Box {
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    let sender = peer_display_name(state, &item.sender_mid);
    let date = sent_date(item.created_time);
    let name = item_name(&item);
    let open = gtk::Button::builder()
        .hexpand(true)
        .tooltip_text(format!("{name}\n{sender} · {date}"))
        .build();
    open.update_property(&[gtk::accessible::Property::Label(&format!(
        "{name}, {sender}, {date}"
    ))]);
    if visual {
        let frame = gtk::Box::builder()
            .width_request(144)
            .height_request(132)
            .build();
        let overlay = gtk::Overlay::builder().child(&frame).build();
        let path = item.thumbnail_path.as_deref().or(if item.kind == "image" {
            item.path.as_deref()
        } else {
            None
        });
        if let Some(path) = path.filter(|path| std::path::Path::new(path).is_file()) {
            let picture = gtk::Picture::builder()
                .can_shrink(true)
                .content_fit(gtk::ContentFit::Cover)
                .build();
            overlay.add_overlay(&picture);
            overlay.set_measure_overlay(&picture, false);
            overlay.set_clip_overlay(&picture, true);
            attach_texture_async(picture, path.into(), 320);
        } else {
            let icon = gtk::Image::from_icon_name(if item.kind == "video" {
                "video-x-generic-symbolic"
            } else {
                "image-x-generic-symbolic"
            });
            icon.set_pixel_size(40);
            icon.set_halign(gtk::Align::Center);
            icon.set_valign(gtk::Align::Center);
            overlay.add_overlay(&icon);
        }
        if item.kind == "video" {
            let label = gtk::Label::builder()
                .label(video_label(item.duration_ms))
                .halign(gtk::Align::End)
                .valign(gtk::Align::End)
                .margin_end(6)
                .margin_bottom(6)
                .css_classes(["osd"])
                .build();
            overlay.add_overlay(&label);
        }
        open.set_child(Some(&overlay));
    } else {
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
        labels.append(
            &gtk::Label::builder()
                .label(&name)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(45)
                .css_classes(["heading"])
                .build(),
        );
        let details = if let Some(url) = &item.url {
            url.clone()
        } else {
            item.file_size.map(format_bytes).unwrap_or_default()
        };
        if !details.is_empty() {
            labels.append(
                &gtk::Label::builder()
                    .label(&details)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .max_width_chars(55)
                    .css_classes(["dim-label"])
                    .build(),
            );
        }
        open.set_child(Some(&labels));
    }
    let s = state.clone();
    let weak = Rc::downgrade(ui);
    let source = item.clone();
    open.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            activate(&s, &ui, &source, false);
        }
    });
    outer.append(&open);
    let bottom = gtk::Box::builder().spacing(4).build();
    let details = gtk::Label::builder()
        .label(format!("{sender} · {date}"))
        .hexpand(true)
        .xalign(0.0)
        .max_width_chars(if visual { 16 } else { 45 })
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["dim-label", "caption"])
        .build();
    bottom.append(&details);
    if item.kind != "link" {
        let save = icon_button("document-save-symbolic", "shared_media_save");
        let s = state.clone();
        let weak = Rc::downgrade(ui);
        let source = item.clone();
        save.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                activate(&s, &ui, &source, true);
            }
        });
        bottom.append(&save);
    } else if let Some(url) = item.url.clone() {
        let copy = icon_button("edit-copy-symbolic", "shared_media_copy_link");
        let clipboard = ui.window.clipboard();
        let weak = Rc::downgrade(ui);
        copy.connect_clicked(move |_| {
            clipboard.set_text(&url);
            if let Some(ui) = weak.upgrade() {
                ui.toast.add_toast(libadwaita::Toast::new(&crate::i18n::t(
                    "shared_media_link_copied",
                )));
            }
        });
        bottom.append(&copy);
    }
    let jump = icon_button("go-jump-symbolic", "shared_media_show_message");
    let s = state.clone();
    let weak = Rc::downgrade(ui);
    let source = item.clone();
    jump.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            if is_unsent(&s, &ui, &source.message_id) {
                return;
            }
            ui.window.set_visible(false);
            history::jump_to_shared_message(
                &s,
                &ui.chat_mid,
                &source.message_id,
                source.created_time,
            );
        }
    });
    bottom.append(&jump);
    outer.append(&bottom);
    if item.cache_state == "failed" {
        outer.append(
            &gtk::Label::builder()
                .label(crate::i18n::t("shared_media_retry_hint"))
                .wrap(true)
                .css_classes(["dim-label", "caption"])
                .build(),
        );
    }
    outer
}

fn activate(state: &AppState, ui: &Rc<SharedMediaUi>, item: &SharedMediaItem, save: bool) {
    if ui.closed.get()
        || ui.account != *state.self_mid.borrow()
        || is_unsent(state, ui, &item.message_id)
    {
        return;
    }
    if let Some(url) = item
        .url
        .as_deref()
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
    {
        open_uri(url);
        return;
    }
    if !ui
        .downloads
        .borrow_mut()
        .insert((item.message_id.clone(), save))
    {
        return;
    }
    let sent = state
        .sidecar
        .download_shared_media(&ui.chat_mid, &item.message_id);
    enqueue(
        state,
        ui,
        SharedMediaRequest::Download {
            item: Box::new(item.clone()),
            save,
        },
        sent,
    );
}

fn icon_button(icon: &str, key: &str) -> gtk::Button {
    let label = crate::i18n::t(key);
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(&label)
        .css_classes(["flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&label)]);
    button
}

fn item_name(item: &SharedMediaItem) -> String {
    if let Some(url) = &item.url {
        return url
            .split_once("://")
            .map(|(_, tail)| tail.split('/').next().unwrap_or(tail))
            .unwrap_or(url)
            .to_string();
    }
    item.file_name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            format!(
                "{} · {}",
                crate::i18n::t(if item.kind == "video" {
                    "shared_media_video"
                } else if item.kind == "image" {
                    "shared_media_photo"
                } else {
                    "shared_media_file"
                }),
                item.message_id
            )
        })
}

fn local_time(created_time: i64) -> Option<chrono::DateTime<chrono::Local>> {
    if created_time <= 0 {
        return None;
    }
    chrono::DateTime::from_timestamp_millis(created_time)
        .map(|time| time.with_timezone(&chrono::Local))
}

fn month_key(created_time: i64) -> String {
    local_time(created_time)
        .map(|time| time.format("%Y-%m").to_string())
        .unwrap_or_default()
}

fn sent_date(created_time: i64) -> String {
    local_time(created_time)
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| crate::i18n::t("shared_media_unknown_date"))
}

fn video_label(duration: Option<i64>) -> String {
    let duration = duration.unwrap_or_default().max(0) / 1000;
    if duration > 0 {
        format!("▶ {}:{:02}", duration / 60, duration % 60)
    } else {
        crate::i18n::t("shared_media_video")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_cannot_repeat_or_advance_without_a_continuation() {
        let mut tab = TabPage::default();
        assert!(!tab.next());
        let mut page: SharedMediaPage = serde_json::from_value(serde_json::json!({
            "chatMid":"c-one", "category":"files", "items":[],
            "nextCursor":"cursor-one", "scanState":"more"
        }))
        .unwrap();
        tab.result = Some(page.clone());
        assert!(tab.next());
        assert_eq!(tab.cursor(), "cursor-one");
        page.next_cursor = "cursor-one".into();
        tab.result = Some(page);
        assert!(!tab.next());
    }

    #[test]
    fn missing_timestamps_do_not_create_a_january_1970_section() {
        assert!(month_key(0).is_empty());
        assert!(month_key(-1).is_empty());
        assert_eq!(video_label(Some(83_000)), "▶ 1:23");
    }

    #[test]
    fn protocol_keeps_large_message_ids_and_unknown_optional_metadata() {
        let item: SharedMediaItem = serde_json::from_value(serde_json::json!({
            "key":"u-one:c-one:9007199254740993", "messageId":"9007199254740993",
            "chatMid":"c-one", "createdTime":1_792_771_200_000_i64,
            "senderMid":"u-two", "kind":"file", "cacheState":"pending"
        }))
        .unwrap();
        assert_eq!(item.message_id, "9007199254740993");
        assert!(item.file_size.is_none());
        assert!(item.path.is_none());
        assert!(!month_key(item.created_time).is_empty());
    }
}
