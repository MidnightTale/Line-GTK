use super::*;
use std::cell::Cell;

pub(super) struct SearchUi {
    window: gtk::Window,
    entry: gtk::SearchEntry,
    results: gtk::ListBox,
    message_ids: RefCell<Vec<String>>,
    status: gtk::Label,
    spinner: gtk::Spinner,
    previous: gtk::Button,
    next: gtk::Button,
    chat_mid: String,
    account: String,
    generation: Cell<u64>,
    offset: Cell<u64>,
    closed: Cell<bool>,
}

pub(super) fn wire(state: &AppState) {
    let s = state.clone();
    state.message_search_btn.connect_clicked(move |_| open(&s));
    let keys = gtk::EventControllerKey::new();
    let s = state.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            && matches!(key, gdk::Key::f | gdk::Key::F)
            && s.stack.visible_child_name().as_deref() == Some("shell")
        {
            open(&s);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    state.window.add_controller(keys);
}

fn open(state: &AppState) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let existing = state.message_search_ui.borrow().clone();
    if let Some(ui) = existing {
        if !ui.closed.get() && ui.chat_mid == chat_mid && ui.account == account {
            ui.window.present();
            ui.entry.grab_focus();
            return;
        }
        close(state);
    }
    let window = gtk::Window::builder()
        .transient_for(&state.window)
        .destroy_with_parent(true)
        .title(crate::i18n::t("message_search"))
        .default_width(560)
        .default_height(600)
        .build();
    let title = libadwaita::WindowTitle::new(
        &crate::i18n::t("message_search"),
        &peer_display_name(state, &chat_mid),
    );
    window.set_titlebar(Some(
        &libadwaita::HeaderBar::builder()
            .title_widget(&title)
            .build(),
    ));
    let entry = gtk::SearchEntry::builder()
        .placeholder_text(crate::i18n::t("message_search_placeholder"))
        .hexpand(true)
        .build();
    let label = gtk::Label::builder()
        .label(crate::i18n::t("message_search"))
        .xalign(0.0)
        .mnemonic_widget(&entry)
        .build();
    let hint = gtk::Label::builder()
        .label(crate::i18n::t("message_search_scope"))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .build();
    let status = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .build();
    let spinner = gtk::Spinner::new();
    let feedback = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    feedback.append(&spinner);
    feedback.append(&status);
    let results = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .valign(gtk::Align::Start)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&results)
        .build();
    let previous = gtk::Button::with_label(&crate::i18n::t("shared_media_previous"));
    let next = gtk::Button::with_label(&crate::i18n::t("shared_media_older"));
    previous.set_sensitive(false);
    next.set_sensitive(false);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.append(&previous);
    footer.append(&next);
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    for widget in [
        label.upcast_ref::<gtk::Widget>(),
        entry.upcast_ref(),
        hint.upcast_ref(),
        feedback.upcast_ref(),
        scroll.upcast_ref(),
        footer.upcast_ref(),
    ] {
        root.append(widget);
    }
    window.set_child(Some(&root));
    let ui = Rc::new(SearchUi {
        window,
        entry,
        results,
        message_ids: RefCell::new(Vec::new()),
        status,
        spinner,
        previous,
        next,
        chat_mid,
        account,
        generation: Cell::new(0),
        offset: Cell::new(0),
        closed: Cell::new(false),
    });
    *state.message_search_ui.borrow_mut() = Some(ui.clone());
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    connect_result_activation(&ui.results, move |index| {
        if let Some(ui) = weak.upgrade()
            && valid(&s, &ui)
        {
            let message_id = ui.message_ids.borrow().get(index).cloned();
            if let Some(message_id) = message_id {
                history::jump_to_search_message(&s, &ui.chat_mid, &message_id);
                ui.window.close();
                s.window.present();
            }
        }
    });
    // Invalidate immediately, before SearchEntry's debounce can deliver an old result.
    let weak = Rc::downgrade(&ui);
    ui.entry.connect_changed(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.generation.set(ui.generation.get().wrapping_add(1));
            clear_results(&ui);
            ui.spinner.stop();
            ui.status.set_text("");
            ui.previous.set_sensitive(false);
            ui.next.set_sensitive(false);
        }
    });
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.entry.connect_search_changed(move |_| {
        if let Some(ui) = weak.upgrade() {
            load(&s, &ui, 0);
        }
    });
    for (button, older) in [(&ui.previous, false), (&ui.next, true)] {
        let s = state.clone();
        let weak = Rc::downgrade(&ui);
        button.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let offset = if older {
                    ui.offset.get() + 50
                } else {
                    ui.offset.get().saturating_sub(50)
                };
                load(&s, &ui, offset);
            }
        });
    }
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.window.connect_close_request(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.closed.set(true);
            let current = s.message_search_ui.borrow().clone();
            if current
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &ui))
            {
                s.message_search_ui.borrow_mut().take();
            }
        }
        glib::Propagation::Proceed
    });
    let weak = Rc::downgrade(&ui);
    ui.entry.connect_stop_search(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.window.close();
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
    ui.window.present();
    ui.entry.grab_focus();
}

// Pointer activation is emitted by ListBox; Row::activate is a keyboard action.
// Listening to the list handles both inputs through the same navigation path.
fn connect_result_activation(list: &gtk::ListBox, activate: impl Fn(usize) + 'static) {
    list.connect_row_activated(move |_, row| {
        if let Ok(index) = usize::try_from(row.index()) {
            activate(index);
        }
    });
}

fn clear_results(ui: &SearchUi) {
    ui.message_ids.borrow_mut().clear();
    while let Some(child) = ui.results.first_child() {
        ui.results.remove(&child);
    }
}

fn valid(state: &AppState, ui: &SearchUi) -> bool {
    !ui.closed.get()
        && *state.session_ready.borrow()
        && state.self_mid.borrow().as_deref() == Some(ui.account.as_str())
        && state.current_chat.borrow().as_deref() == Some(ui.chat_mid.as_str())
}

fn load(state: &AppState, ui: &Rc<SearchUi>, offset: u64) {
    if !valid(state, ui) {
        return;
    }
    ui.generation.set(ui.generation.get().wrapping_add(1));
    clear_results(ui);
    ui.previous.set_sensitive(false);
    ui.next.set_sensitive(false);
    if ui.entry.text().trim().is_empty() {
        ui.spinner.stop();
        ui.status.set_text("");
        return;
    }
    ui.offset.set(offset);
    ui.spinner.start();
    ui.status
        .set_text(&crate::i18n::t("message_search_loading"));
    match state.sidecar.request(
        "search_messages",
        Some(serde_json::json!({
            "chatMid": ui.chat_mid, "query": ui.entry.text().as_str(), "offset": offset,
        })),
    ) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::MessageSearch {
                    ui: Rc::downgrade(ui),
                    generation: ui.generation.get(),
                },
            );
        }
        Err(_) => {
            ui.spinner.stop();
            ui.status.set_text(&crate::i18n::t("message_search_failed"));
        }
    }
}

pub(super) fn response(
    state: &AppState,
    ui: &Rc<SearchUi>,
    generation: u64,
    ok: bool,
    result: &serde_json::Value,
) {
    if !valid(state, ui) || generation != ui.generation.get() {
        return;
    }
    ui.spinner.stop();
    if !ok || result["chatMid"].as_str() != Some(ui.chat_mid.as_str()) {
        ui.status.set_text(&crate::i18n::t("message_search_failed"));
        return;
    }
    let total = result["total"].as_u64().unwrap_or(0);
    ui.status.set_text(&if total == 0 {
        crate::i18n::t("message_search_empty")
    } else {
        crate::i18n::t("message_search_count").replace("{count}", &total.to_string())
    });
    ui.previous.set_sensitive(ui.offset.get() > 0);
    ui.next
        .set_sensitive(result["hasMore"].as_bool().unwrap_or(false));
    let messages: Vec<MessageInfo> =
        serde_json::from_value(result["messages"].clone()).unwrap_or_default();
    for message in messages {
        if state
            .unsent_ids
            .borrow()
            .contains(&(ui.account.clone(), message.id.clone()))
        {
            continue;
        }
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let sender = if message.mine {
            state.self_display_name.borrow().clone()
        } else {
            state
                .sender_names
                .borrow()
                .get(&message.from)
                .cloned()
                .unwrap_or_else(|| peer_display_name(state, &message.from))
        };
        let meta = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "caption"])
            .label(format!(
                "{} · {} {}",
                sender,
                messages::format_day_separator(message.created_time),
                messages::format_msg_time(message.created_time)
            ))
            .build();
        let snippet = if message.text.is_empty() {
            message.file_name.as_deref().unwrap_or("")
        } else {
            &message.text
        };
        let text = gtk::Label::builder()
            .label(snippet)
            .xalign(0.0)
            .wrap(true)
            .lines(3)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        content.append(&meta);
        content.append(&text);
        row.set_child(Some(&content));
        ui.message_ids.borrow_mut().push(message.id);
        ui.results.append(&row);
    }
}

pub(super) fn close(state: &AppState) {
    let ui = state.message_search_ui.borrow_mut().take();
    if let Some(ui) = ui {
        ui.closed.set(true);
        ui.window.close();
    }
}

pub(super) fn unsent(state: &AppState, chat_mid: &str) {
    let ui = state.message_search_ui.borrow().clone();
    if let Some(ui) = ui
        && ui.chat_mid == chat_mid
    {
        load(state, &ui, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn pointer_and_keyboard_activation_open_the_selected_result() {
        gtk::test_synced(|| {
            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .build();
            let window = gtk::Window::new();
            window.set_child(Some(&list));
            let first = gtk::ListBoxRow::new();
            let second = gtk::ListBoxRow::new();
            list.append(&first);
            list.append(&second);
            let activated = Rc::new(RefCell::new(Vec::new()));
            let received = activated.clone();
            connect_result_activation(&list, move |index| received.borrow_mut().push(index));

            // GtkListBox emits this signal for a pointer click, without emitting
            // the GtkListBoxRow keyboard action that the old code listened to.
            list.emit_by_name::<()>("row-activated", &[&second]);
            assert_eq!(*activated.borrow(), [1]);
            first.emit_activate();
            assert_eq!(*activated.borrow(), [1, 0]);

            // Results are replaced when the query or page changes. New rows must
            // use the new page's indices, with exactly one handler per activation.
            list.remove(&first);
            list.remove(&second);
            let replacement = gtk::ListBoxRow::new();
            list.append(&replacement);
            list.emit_by_name::<()>("row-activated", &[&replacement]);
            assert_eq!(*activated.borrow(), [1, 0, 0]);
            window.destroy();
        });
    }
}
