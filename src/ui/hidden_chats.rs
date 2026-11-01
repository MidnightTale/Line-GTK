use super::*;

#[derive(Clone)]
pub(super) struct HiddenChatsUi {
    window: gtk::Window,
    list: gtk::ListBox,
    empty: gtk::Label,
    toast: libadwaita::ToastOverlay,
    account: String,
}

pub(super) fn is_hidden(state: &AppState, mid: &str) -> bool {
    let account = state.self_mid.borrow();
    account
        .as_deref()
        .is_some_and(|account| state.config.borrow().is_chat_hidden(account, mid))
}

pub(super) fn wire(state: &AppState) {
    let action = gio::SimpleAction::new("hide-chat", Some(glib::VariantTy::STRING));
    let s = state.clone();
    action.connect_activate(move |_, value| {
        if let Some(mid) = value.and_then(|value| value.str()) {
            hide(&s, mid);
        }
    });
    state.window.add_action(&action);
}

fn hide(state: &AppState, mid: &str) {
    if !*state.session_ready.borrow() || is_hidden(state, mid) {
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let Some(chat) = state
        .chats
        .borrow()
        .iter()
        .find(|chat| chat.mid == mid)
        .cloned()
    else {
        return;
    };
    let result =
        state
            .config
            .borrow_mut()
            .set_chat_hidden(&state.data_dir, &account, mid, Some(&chat.name));
    if result.is_err() {
        toast(state, &crate::i18n::t("hidden_chat_save_failed"));
        return;
    }
    // 別のトークを自動で開くと既読が付くため、選択のない画面へ戻す。
    if state.current_chat.borrow().as_deref() == Some(mid) {
        finish_voice_record(state, false);
        *state.current_chat.borrow_mut() = None;
        clear_messages(state);
        *state.msg_list_fp.borrow_mut() = None;
        hide_upload_progress(state);
        state.chat_list.unselect_all();
        state.chat_title.set_text(&crate::i18n::t("select_chat"));
        user_profile::update_header(state, None);
        state.chat_subtitle.set_text(&crate::i18n::t("pick_chat"));
        state.status.set_text(&crate::i18n::t("hidden_chat_done"));
        state.mute_btn.set_sensitive(false);
        set_msg_state(state, "idle", None);
        refresh_call_controls(state);
        sync_discord_rpc(state);
    }
    chat_filter::refresh(state);
    refresh_tray_menu(state);
    refresh(state);
    toast(state, &crate::i18n::t("hidden_chat_done"));
}

pub(super) fn wire_context(state: &AppState, row: &gtk::ListBoxRow, chat: &ChatInfo) {
    row.connect_unrealize(|row| {
        let mut child = row.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.is::<gtk::Popover>() {
                widget.unparent();
            }
        }
    });
    let mid = chat.mid.clone();
    let eligible = read_pause::eligible(chat);
    let config = state.config.clone();
    let account = state.self_mid.clone();
    let block_state = state.clone();
    let popup = Rc::new(move |row: &gtk::ListBoxRow, x: f64, y: f64| {
        let menu = gio::Menu::new();
        let item = gio::MenuItem::new(Some(&crate::i18n::t("hide_chat")), None);
        item.set_action_and_target_value(Some("win.hide-chat"), Some(&mid.to_variant()));
        menu.append_item(&item);
        let pinned = account
            .borrow()
            .as_deref()
            .is_some_and(|account| config.borrow().is_chat_pinned(account, &mid));
        pinned_chats::append_menu(&menu, &mid, pinned);
        if eligible {
            let paused = account
                .borrow()
                .as_deref()
                .is_some_and(|account| config.borrow().chat_read_receipts_paused(account, &mid));
            read_pause::append_chat_menu(&menu, &mid, paused);
        }
        contact_blocks::append_menu(&block_state, &menu, &mid);
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(row);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    });
    let click = gtk::GestureClick::new();
    click.set_button(3);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = row.downgrade();
    let popup_click = popup.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        if let Some(row) = weak.upgrade() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            popup_click(&row, x, y);
        }
    });
    row.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let weak = row.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gdk::Key::Menu
            || (key == gdk::Key::F10 && modifiers.contains(gdk::ModifierType::SHIFT_MASK))
        {
            if let Some(row) = weak.upgrade() {
                popup(&row, 10.0, 10.0);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    row.add_controller(keys);
}

pub(super) fn close(state: &AppState) {
    let ui = state.hidden_chats_ui.borrow_mut().take();
    if let Some(ui) = ui {
        ui.window.close();
    }
}

pub(super) fn open(state: &AppState, parent: &gtk::Window) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    if let Some(ui) = state.hidden_chats_ui.borrow().as_ref() {
        ui.window.present();
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let window = gtk::Window::builder()
        .transient_for(parent)
        .destroy_with_parent(true)
        .modal(true)
        .title(crate::i18n::t("hidden_chats"))
        .default_width(480)
        .default_height(460)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    window.set_titlebar(Some(&libadwaita::HeaderBar::new()));
    let description = gtk::Label::builder()
        .label(crate::i18n::t("hidden_chats_description"))
        .wrap(true)
        .xalign(0.0)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(20)
        .margin_end(20)
        .css_classes(["dim-label"])
        .build();
    content.append(&description);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .margin_start(16)
        .margin_end(16)
        .margin_bottom(16)
        .css_classes(["boxed-list"])
        .build();
    let empty = gtk::Label::builder()
        .label(crate::i18n::t("hidden_chats_empty"))
        .margin_top(32)
        .margin_bottom(32)
        .css_classes(["dim-label"])
        .build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    body.append(&list);
    body.append(&empty);
    content.append(
        &gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&body)
            .build(),
    );
    let overlay = libadwaita::ToastOverlay::new();
    overlay.set_child(Some(&content));
    window.set_child(Some(&overlay));
    let s = state.clone();
    window.connect_close_request(move |_| {
        s.hidden_chats_ui.borrow_mut().take();
        glib::Propagation::Proceed
    });
    *state.hidden_chats_ui.borrow_mut() = Some(HiddenChatsUi {
        window: window.clone(),
        list,
        empty,
        toast: overlay,
        account,
    });
    refresh(state);
    window.present();
}

pub(super) fn refresh(state: &AppState) {
    let Some(ui) = state.hidden_chats_ui.borrow().clone() else {
        return;
    };
    if state.self_mid.borrow().as_deref() != Some(ui.account.as_str()) {
        close(state);
        return;
    }
    let hidden = state
        .config
        .borrow()
        .hidden_chats
        .get(&ui.account)
        .cloned()
        .unwrap_or_default();
    clear_list(&ui.list);
    ui.empty.set_visible(hidden.is_empty());
    ui.list.set_visible(!hidden.is_empty());
    for (mid, saved_name) in hidden {
        let name = state
            .chats
            .borrow()
            .iter()
            .find(|chat| chat.mid == mid)
            .map(|chat| chat.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or(saved_name);
        let row = libadwaita::ActionRow::builder()
            .title(&name)
            .use_markup(false)
            .title_lines(1)
            .build();
        let restore = gtk::Button::builder()
            .label(crate::i18n::t("unhide_chat"))
            .valign(gtk::Align::Center)
            .build();
        let s = state.clone();
        let account = ui.account.clone();
        let overlay = ui.toast.clone();
        restore.connect_clicked(move |_| {
            if !*s.session_ready.borrow() || s.self_mid.borrow().as_deref() != Some(&account) {
                return;
            }
            let result = s
                .config
                .borrow_mut()
                .set_chat_hidden(&s.data_dir, &account, &mid, None);
            if result.is_err() {
                overlay.add_toast(libadwaita::Toast::new(&crate::i18n::t(
                    "hidden_chat_save_failed",
                )));
                return;
            }
            // 復元だけでは履歴を開かず、未読状態を維持する。
            chat_filter::refresh(&s);
            refresh_tray_menu(&s);
            refresh(&s);
            if !s.chats.borrow().iter().any(|chat| chat.mid == mid)
                && let Ok(id) = s.sidecar.list_chats()
            {
                s.pending.borrow_mut().insert(id, Pending::ListChats);
            }
            overlay.add_toast(libadwaita::Toast::new(&crate::i18n::t(
                "hidden_chat_restored",
            )));
        });
        row.add_suffix(&restore);
        ui.list.append(&row);
    }
}
