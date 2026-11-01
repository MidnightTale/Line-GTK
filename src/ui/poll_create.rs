//! "Create poll" dialog. LINE's poll service posts the poll card into the
//! chat itself, so the new message arrives through the normal event stream.

use super::*;
use serde_json::json;
use std::cell::Cell;

const MAX_CHOICES: usize = 50;
const MAX_TITLE: i32 = 70;
const MAX_CHOICE: i32 = 60;

pub(super) struct CreateUi {
    window: gtk::Window,
    account: String,
    chat_mid: String,
    title: gtk::Entry,
    rows: gtk::Box,
    entries: RefCell<Vec<gtk::Entry>>,
    add: gtk::Button,
    multiple: gtk::CheckButton,
    anonymous: gtk::CheckButton,
    allow_new: gtk::CheckButton,
    close_check: gtk::CheckButton,
    close_days: gtk::SpinButton,
    create: gtk::Button,
    status: gtk::Label,
    busy: Cell<bool>,
}

thread_local! {
    static UI: RefCell<Option<Rc<CreateUi>>> = const { RefCell::new(None) };
}

pub(super) fn wire(state: &AppState) {
    let s = state.clone();
    state.poll_create_btn.connect_clicked(move |_| open(&s));
}

pub(super) fn close() {
    if let Some(ui) = UI.with(|ui| ui.borrow_mut().take()) {
        ui.window.destroy();
    }
}

fn open(state: &AppState) {
    if !*state.session_ready.borrow() {
        return;
    }
    let (Some(account), Some(chat_mid)) = (
        state.self_mid.borrow().clone(),
        state.current_chat.borrow().clone(),
    ) else {
        return;
    };
    close();

    let window = gtk::Window::builder()
        .title(crate::i18n::t("poll_create"))
        .transient_for(&state.window)
        .modal(true)
        .default_width(440)
        .build();
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let title = gtk::Entry::builder()
        .placeholder_text(crate::i18n::t("poll_question"))
        .max_length(MAX_TITLE)
        .activates_default(false)
        .build();
    root.append(&section(&crate::i18n::t("poll_question")));
    root.append(&title);

    root.append(&section(&crate::i18n::t("poll_options")));
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.append(&rows);
    let add = gtk::Button::builder()
        .label(crate::i18n::t("poll_add_option"))
        .icon_name("list-add-symbolic")
        .halign(gtk::Align::Start)
        .css_classes(["flat"])
        .build();
    root.append(&add);

    let multiple = gtk::CheckButton::with_label(&crate::i18n::t("poll_multiple_choice"));
    let anonymous = gtk::CheckButton::with_label(&crate::i18n::t("poll_anonymous_votes"));
    let allow_new = gtk::CheckButton::with_label(&crate::i18n::t("poll_allow_new"));
    let close_check = gtk::CheckButton::with_label(&crate::i18n::t("poll_set_close"));
    let close_days = gtk::SpinButton::with_range(1.0, 30.0, 1.0);
    close_days.set_value(3.0);
    close_days.set_sensitive(false);
    let close_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    close_row.append(&close_check);
    close_row.append(&close_days);
    close_row.append(&gtk::Label::new(Some(&crate::i18n::t("poll_days"))));
    {
        let days = close_days.clone();
        close_check.connect_toggled(move |c| days.set_sensitive(c.is_active()));
    }
    root.append(&section(&crate::i18n::t("poll_settings")));
    root.append(&multiple);
    root.append(&anonymous);
    root.append(&allow_new);
    root.append(&close_row);

    let status = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["error"])
        .build();
    root.append(&status);
    let cancel = gtk::Button::with_label(&crate::i18n::t("cancel"));
    let create = gtk::Button::builder()
        .label(crate::i18n::t("poll_create_action"))
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    let buttons = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    buttons.append(&cancel);
    buttons.append(&create);
    root.append(&buttons);
    window.set_child(Some(&root));

    let ui = Rc::new(CreateUi {
        window: window.clone(),
        account,
        chat_mid,
        title,
        rows,
        entries: RefCell::new(Vec::new()),
        add,
        multiple,
        anonymous,
        allow_new,
        close_check,
        close_days,
        create,
        status,
        busy: Cell::new(false),
    });
    add_row(&ui);
    add_row(&ui);

    {
        let weak = Rc::downgrade(&ui);
        ui.title.connect_changed(move |_| {
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }
    {
        let weak = Rc::downgrade(&ui);
        ui.add.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                add_row(&ui);
            }
        });
    }
    {
        let state = state.clone();
        let weak = Rc::downgrade(&ui);
        ui.create.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                submit(&state, &ui);
            }
        });
    }
    cancel.connect_clicked(move |_| close());
    window.connect_close_request(|_| {
        UI.with(|ui| ui.borrow_mut().take());
        glib::Propagation::Proceed
    });

    UI.with(|slot| *slot.borrow_mut() = Some(ui.clone()));
    window.present();
    ui.title.grab_focus();
}

fn section(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["heading"])
        .build()
}

fn add_row(ui: &Rc<CreateUi>) {
    if ui.entries.borrow().len() >= MAX_CHOICES {
        return;
    }
    let entry = gtk::Entry::builder()
        .hexpand(true)
        .max_length(MAX_CHOICE)
        .placeholder_text(crate::i18n::t("poll_option_placeholder"))
        .build();
    let remove = gtk::Button::builder()
        .icon_name("edit-delete-symbolic")
        .css_classes(["flat", "circular"])
        .build();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&entry);
    row.append(&remove);
    ui.rows.append(&row);
    ui.entries.borrow_mut().push(entry.clone());
    {
        let weak = Rc::downgrade(ui);
        entry.connect_changed(move |_| {
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }
    {
        let weak = Rc::downgrade(ui);
        let entry = entry.clone();
        let row = row.clone();
        remove.connect_clicked(move |_| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if ui.entries.borrow().len() <= 2 {
                return;
            }
            ui.rows.remove(&row);
            ui.entries.borrow_mut().retain(|e| e != &entry);
            refresh(&ui);
        });
    }
    refresh(ui);
}

fn choices(ui: &CreateUi) -> Vec<String> {
    ui.entries
        .borrow()
        .iter()
        .map(|e| e.text().trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn refresh(ui: &CreateUi) {
    ui.create.set_sensitive(
        !ui.busy.get() && !ui.title.text().trim().is_empty() && choices(ui).len() >= 2,
    );
    ui.add
        .set_sensitive(ui.entries.borrow().len() < MAX_CHOICES);
}

fn set_busy(ui: &CreateUi, busy: bool) {
    ui.busy.set(busy);
    for widget in [
        ui.title.upcast_ref::<gtk::Widget>(),
        ui.rows.upcast_ref(),
        ui.multiple.upcast_ref(),
        ui.anonymous.upcast_ref(),
        ui.allow_new.upcast_ref(),
        ui.close_check.upcast_ref(),
    ] {
        widget.set_sensitive(!busy);
    }
    ui.close_days
        .set_sensitive(!busy && ui.close_check.is_active());
    refresh(ui);
}

fn submit(state: &AppState, ui: &Rc<CreateUi>) {
    if ui.busy.get() {
        return;
    }
    if state.self_mid.borrow().as_deref() != Some(ui.account.as_str())
        || !*state.session_ready.borrow()
    {
        ui.status.set_text(&crate::i18n::t("poll_session_changed"));
        return;
    }
    let close_date = ui
        .close_check
        .is_active()
        .then(|| glib::real_time() / 1000 + ui.close_days.value_as_int() as i64 * 86_400_000);
    let poll = json!({
        "title": ui.title.text().trim(),
        "choices": choices(ui),
        "multiple": ui.multiple.is_active(),
        "anonymous": ui.anonymous.is_active(),
        "allowNewChoices": ui.allow_new.is_active(),
        "closeDate": close_date,
    });
    ui.status.set_text("");
    set_busy(ui, true);
    match state.sidecar.poll_create(&ui.chat_mid, poll) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::PollCreate {
                    ui: Rc::downgrade(ui),
                },
            );
        }
        Err(e) => {
            set_busy(ui, false);
            ui.status.set_text(&e.to_string());
        }
    }
}

pub(super) fn response(
    state: &AppState,
    ui: &std::rc::Weak<CreateUi>,
    ok: bool,
    error: Option<&str>,
) {
    let Some(ui) = ui.upgrade() else {
        return;
    };
    if state.self_mid.borrow().as_deref() != Some(ui.account.as_str()) {
        close();
        return;
    }
    if ok {
        close();
        toast(state, &crate::i18n::t("poll_created"));
        return;
    }
    set_busy(&ui, false);
    // Local validation codes are translated; LINE's own messages pass through.
    ui.status
        .set_text(&crate::i18n::t(error.unwrap_or("poll_create_failed")));
}
