use super::*;
use crate::protocol::UserProfile;
use std::cell::Cell;

struct Ui {
    window: gtk::Window,
    mid: String,
    generation: u64,
    avatar: gtk::Picture,
    name: gtk::Label,
    status: gtk::Label,
    note: gtk::Label,
    spinner: gtk::Spinner,
    message: gtk::Button,
    block: gtk::Button,
    is_self: bool,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static HEADER_AVATAR: RefCell<Option<(gtk::Overlay, gtk::Picture)>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    static ROW_AVATARS: RefCell<HashMap<String, Vec<glib::WeakRef<gtk::Picture>>>> =
        RefCell::new(HashMap::new());
}

pub(super) fn close() {
    if let Some(ui) = UI.with(|ui| ui.borrow_mut().take()) {
        ui.window.destroy();
    }
}

fn known_avatar(state: &AppState, mid: &str) -> Option<String> {
    let path = if state.self_mid.borrow().as_deref() == Some(mid) {
        state.self_avatar_path.borrow().clone()
    } else {
        state
            .chats
            .borrow()
            .iter()
            .find(|chat| chat.mid == mid)
            .and_then(|chat| chat.avatar_path.clone())
    };
    path.filter(|p| std::path::Path::new(p).exists())
}

fn avatar_overlay(size: i32, picture: &gtk::Picture) -> gtk::Overlay {
    let base = gtk::Box::builder()
        .width_request(size)
        .height_request(size)
        .build();
    let overlay = gtk::Overlay::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    overlay.set_child(Some(&base));
    overlay.add_overlay(picture);
    overlay.set_measure_overlay(picture, false);
    overlay.set_clip_overlay(picture, true);
    overlay
}

fn avatar_picture(class: &str) -> gtk::Picture {
    gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .css_classes(["line-avatar", class])
        .build()
}

fn avatar_frame(size: i32, class: &str) -> (gtk::Box, gtk::Picture) {
    let frame = gtk::Box::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Start)
        .css_classes(["line-avatar-frame", class])
        .build();
    let picture = avatar_picture(class);
    frame.append(&avatar_overlay(size, &picture));
    (frame, picture)
}

fn pointer(widget: &impl IsA<gtk::Widget>, state: &AppState, mid: &str) {
    widget.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::builder().button(1).build();
    let (state, mid) = (state.clone(), mid.to_string());
    click.connect_released(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open(&state, &mid);
    });
    widget.add_controller(click);
}

pub(super) fn wire_link(state: &AppState, widget: &impl IsA<gtk::Widget>, mid: &str) {
    pointer(widget, state, mid);
}

pub(super) fn profile_mid_for_chat(chat: &ChatInfo) -> Option<&str> {
    (mentions::is_user_mid(&chat.mid) && !matches!(chat.kind.as_str(), "group" | "room" | "bot"))
        .then_some(chat.mid.as_str())
}

pub(super) fn wire_header(state: &AppState, widget: &impl IsA<gtk::Widget>) {
    let click = gtk::GestureClick::builder().button(1).build();
    let state = state.clone();
    click.connect_released(move |gesture, _, _, _| {
        let mid = {
            let current = state.current_chat.borrow();
            let chats = state.chats.borrow();
            current
                .as_deref()
                .and_then(|mid| chats.iter().find(|chat| chat.mid == mid))
                .and_then(|chat| profile_mid_for_chat(chat).map(str::to_string))
        };
        if let Some(mid) = mid {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            open(&state, &mid);
        }
    });
    widget.add_controller(click);
}

pub(super) fn update_header(state: &AppState, chat: Option<&ChatInfo>) {
    let mid = chat.and_then(profile_mid_for_chat);
    let clickable = mid.is_some();
    state.chat_avatar_frame.set_visible(clickable);
    if clickable {
        state.chat_header.add_css_class("line-chat-header-link");
        state.chat_header.set_cursor_from_name(Some("pointer"));
        state
            .chat_header
            .set_tooltip_text(Some(&crate::i18n::t("user_profile_open_hint")));
    } else {
        state.chat_header.remove_css_class("line-chat-header-link");
        state.chat_header.set_cursor_from_name(None);
        state.chat_header.set_tooltip_text(None);
    }
    if let Some((old, _)) = HEADER_AVATAR.with(|h| h.borrow_mut().take()) {
        state.chat_avatar_frame.remove(&old);
    }
    let Some(chat) = chat.filter(|_| clickable) else {
        return;
    };
    let picture = avatar_picture("line-avatar-xs");
    let overlay = avatar_overlay(32, &picture);
    state.chat_avatar_frame.append(&overlay);
    if let Some(path) = chat
        .avatar_path
        .as_ref()
        .filter(|p| std::path::Path::new(p).exists())
    {
        attach_texture_async(picture.clone(), path.clone(), 64);
    }
    HEADER_AVATAR.with(|h| *h.borrow_mut() = Some((overlay, picture)));
}

pub(super) fn header_avatar_ready(state: &AppState, mid: &str, path: &str) {
    if !std::path::Path::new(path).exists() {
        return;
    }
    let active = state.current_chat.borrow().as_deref() == Some(mid);
    if !active {
        return;
    }
    if let Some((_, picture)) = HEADER_AVATAR.with(|h| h.borrow().clone()) {
        attach_texture_async(picture, path.to_string(), 64);
    }
}

pub(super) fn row_avatar(state: &AppState, mid: &str) -> gtk::Box {
    let (frame, picture) = avatar_frame(32, "line-avatar-xs");
    if let Some(path) = known_avatar(state, mid) {
        attach_texture_async(picture.clone(), path, 64);
    }
    ROW_AVATARS.with(|map| {
        let mut map = map.borrow_mut();
        let list = map.entry(mid.to_string()).or_default();
        list.retain(|weak| weak.upgrade().is_some());
        list.push(picture.downgrade());
    });
    pointer(&frame, state, mid);
    frame
}

pub(super) fn avatar_ready(mid: &str, path: &str) {
    if !std::path::Path::new(path).exists() {
        return;
    }
    let pictures: Vec<gtk::Picture> = ROW_AVATARS.with(|map| {
        let mut map = map.borrow_mut();
        let Some(list) = map.get_mut(mid) else {
            return Vec::new();
        };
        list.retain(|weak| weak.upgrade().is_some());
        list.iter().filter_map(|weak| weak.upgrade()).collect()
    });
    for picture in pictures {
        attach_texture_async(picture, path.to_string(), 64);
    }
    UI.with(|ui| {
        if let Some(ui) = ui.borrow().as_ref()
            && ui.mid == mid
        {
            attach_texture_async(ui.avatar.clone(), path.to_string(), 192);
        }
    });
}

pub(super) fn refresh_block(state: &AppState) {
    UI.with(|ui| {
        let ui = ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        let blocked = contact_blocks::blocked(state, &ui.mid);
        ui.block.set_visible(!ui.is_self);
        ui.block.set_sensitive(blocked.is_some());
        ui.block
            .set_label(&crate::i18n::t(if blocked == Some(true) {
                "unblock_contact"
            } else {
                "block_contact"
            }));
    });
}

fn chat_for(state: &AppState, mid: &str) -> Option<ChatInfo> {
    state.chats.borrow().iter().find(|c| c.mid == mid).cloned()
}

pub(super) fn open(state: &AppState, mid: &str) {
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    if !*state.session_ready.borrow() || !mentions::is_user_mid(mid) {
        return;
    }
    close();
    let generation = GENERATION.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let is_self = account == mid;
    let window = gtk::Window::builder()
        .transient_for(&state.window)
        .modal(true)
        .title(crate::i18n::t("user_profile_title"))
        .default_width(360)
        .build();
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    window.set_titlebar(Some(&libadwaita::HeaderBar::new()));
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(20)
        .margin_end(20)
        .build();
    let (frame, avatar) = avatar_frame(96, "line-avatar-lg");
    frame.set_halign(gtk::Align::Center);
    frame.set_size_request(96, 96);
    body.append(&frame);
    if let Some(path) = known_avatar(state, mid) {
        attach_texture_async(avatar.clone(), path, 192);
    }
    let name = gtk::Label::builder()
        .label(if is_self {
            state.self_display_name.borrow().clone()
        } else {
            message_senders::name(state, mid)
        })
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .justify(gtk::Justification::Center)
        .selectable(true)
        .css_classes(["title-2"])
        .build();
    body.append(&name);
    let spinner = gtk::Spinner::builder().spinning(true).build();
    body.append(&spinner);
    let note = gtk::Label::builder()
        .label(crate::i18n::t("user_profile_loading"))
        .css_classes(["dim-label"])
        .build();
    body.append(&note);
    let status = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .justify(gtk::Justification::Center)
        .visible(false)
        .build();
    body.append(&status);

    let id_row = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    id_row.append(
        &gtk::Label::builder()
            .label(mid)
            .selectable(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(28)
            .css_classes(["dim-label", "caption", "monospace"])
            .build(),
    );
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(crate::i18n::t("user_profile_copy_id"))
        .css_classes(["flat", "circular"])
        .build();
    copy.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "user_profile_copy_id",
    ))]);
    {
        let (state, mid) = (state.clone(), mid.to_string());
        copy.connect_clicked(move |_| {
            state.window.clipboard().set_text(&mid);
            toast(&state, &crate::i18n::t("user_profile_id_copied"));
        });
    }
    id_row.append(&copy);
    body.append(&id_row);

    let buttons = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::Center)
        .margin_top(6)
        .build();
    let message = gtk::Button::with_label(&crate::i18n::t("user_profile_send_message"));
    message.add_css_class("suggested-action");
    message.set_visible(!is_self && chat_for(state, mid).is_some());
    {
        let (state, mid) = (state.clone(), mid.to_string());
        message.connect_clicked(move |_| {
            if let Some(chat) = chat_for(&state, &mid) {
                close();
                open_chat(&state, &chat);
            }
        });
    }
    buttons.append(&message);
    let block = gtk::Button::with_label(&crate::i18n::t("block_contact"));
    block.add_css_class("destructive-action");
    {
        let (state, mid) = (state.clone(), mid.to_string());
        block.connect_clicked(move |_| {
            let blocked = contact_blocks::blocked(&state, &mid) == Some(true);
            contact_blocks::change(&state, &mid, !blocked);
        });
    }
    buttons.append(&block);
    let dismiss = gtk::Button::with_label(&crate::i18n::t("contact_block_cancel"));
    dismiss.connect_clicked(|_| close());
    buttons.append(&dismiss);
    body.append(&buttons);
    root.append(&body);
    window.set_child(Some(&root));
    window.connect_close_request(|_| {
        close();
        glib::Propagation::Proceed
    });
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);

    UI.with(|ui| {
        *ui.borrow_mut() = Some(Ui {
            window: window.clone(),
            mid: mid.to_string(),
            generation,
            avatar,
            name,
            status,
            note,
            spinner,
            message,
            block,
            is_self,
        });
    });
    refresh_block(state);
    if !is_self && contact_blocks::blocked(state, mid).is_none() {
        contact_blocks::request_refresh(state);
    }
    window.present();
    match state.sidecar.user_profile(mid, false) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::UserProfile {
                    account,
                    mid: mid.to_string(),
                    generation,
                },
            );
        }
        Err(_) => fail(),
    }
}

fn fail() {
    UI.with(|ui| {
        if let Some(ui) = ui.borrow().as_ref() {
            ui.spinner.set_spinning(false);
            ui.spinner.set_visible(false);
            ui.note.set_text(&crate::i18n::t("user_profile_failed"));
        }
    });
}

pub(super) fn response(
    state: &AppState,
    account: &str,
    mid: &str,
    generation: u64,
    ok: bool,
    result: &serde_json::Value,
) {
    if state.self_mid.borrow().as_deref() != Some(account) || !*state.session_ready.borrow() {
        return;
    }
    let current = UI.with(|ui| {
        ui.borrow()
            .as_ref()
            .is_some_and(|ui| ui.mid == mid && ui.generation == generation)
    });
    if !current {
        return;
    }
    let profile = ok
        .then(|| serde_json::from_value::<UserProfile>(result.clone()).ok())
        .flatten()
        .filter(|p| p.mid == mid);
    let Some(profile) = profile else {
        fail();
        return;
    };
    if !profile.name.is_empty() && !profile.is_self {
        state
            .sender_names
            .borrow_mut()
            .insert(profile.mid.clone(), profile.name.clone());
    }
    UI.with(|ui| {
        let ui = ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        ui.spinner.set_spinning(false);
        ui.spinner.set_visible(false);
        ui.note.set_visible(false);
        if !profile.name.is_empty() {
            ui.name.set_text(&profile.name);
        }
        ui.status.set_visible(true);
        if profile.status_message.is_empty() {
            ui.status
                .set_text(&crate::i18n::t("user_profile_status_empty"));
            ui.status.add_css_class("dim-label");
        } else {
            ui.status.set_text(&profile.status_message);
            ui.status.remove_css_class("dim-label");
        }
        if let Some(path) = profile.avatar_path.as_deref()
            && std::path::Path::new(path).exists()
        {
            attach_texture_async(ui.avatar.clone(), path.to_string(), 192);
        }
        ui.message
            .set_visible(!ui.is_self && chat_for(state, mid).is_some());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(mid: &str, kind: &str) -> ChatInfo {
        ChatInfo {
            mid: mid.to_string(),
            name: String::new(),
            kind: kind.to_string(),
            avatar_path: None,
            last_activity: 0,
            unread: 0,
            preview: String::new(),
            muted: false,
            contact_only: Some(true),
        }
    }

    #[test]
    fn profile_mid_only_for_user_chats() {
        assert_eq!(profile_mid_for_chat(&chat("u123", "dm")), Some("u123"));
        assert_eq!(profile_mid_for_chat(&chat("u123", "")), Some("u123"));
        assert_eq!(profile_mid_for_chat(&chat("c123", "group")), None);
        assert_eq!(profile_mid_for_chat(&chat("r123", "room")), None);
        assert_eq!(profile_mid_for_chat(&chat("u123", "bot")), None);
        assert_eq!(profile_mid_for_chat(&chat("", "dm")), None);
    }
}
