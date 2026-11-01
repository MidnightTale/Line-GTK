use super::*;
use crate::protocol::BlockedContact;

#[derive(Default)]
pub(super) struct BlockState {
    account: String,
    contacts: Option<Vec<BlockedContact>>,
    failed: bool,
    ui: Option<BlockUi>,
}

struct BlockUi {
    window: gtk::Window,
    list: gtk::ListBox,
    status: gtk::Label,
}

pub(super) fn blocked(state: &AppState, mid: &str) -> Option<bool> {
    let blocks = state.contact_blocks.borrow();
    if state.self_mid.borrow().as_deref() != Some(blocks.account.as_str()) {
        return None;
    }
    blocks
        .contacts
        .as_ref()
        .map(|contacts| contacts.iter().any(|c| c.mid == mid))
}

fn busy(state: &AppState, mid: Option<&str>) -> bool {
    state.pending.borrow().values().any(|p| match p {
        Pending::ContactBlocks { change, .. } => match (mid, change) {
            (Some(mid), Some((target, _))) => target == mid,
            (None, None) => true,
            _ => false,
        },
        _ => false,
    })
}

pub(super) fn reset(state: &AppState) {
    if let Some(ui) = state.contact_blocks.borrow_mut().ui.take() {
        ui.window.destroy();
    }
    *state.contact_blocks.borrow_mut() = BlockState::default();
}

pub(super) fn request_refresh(state: &AppState) {
    if !*state.session_ready.borrow() || busy(state, None) {
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    if let Ok(id) = state.sidecar.blocked_contacts() {
        state.pending.borrow_mut().insert(
            id,
            Pending::ContactBlocks {
                account,
                change: None,
            },
        );
    }
}

pub(super) fn apply(state: &AppState, account: &str, contacts: Vec<BlockedContact>) {
    if !*state.session_ready.borrow()
        || state.self_mid.borrow().as_deref() != Some(account)
        || contacts
            .iter()
            .any(|c| !c.mid.starts_with('u') || c.mid == account)
    {
        return;
    }
    {
        let mut blocks = state.contact_blocks.borrow_mut();
        blocks.account = account.into();
        blocks.contacts = Some(contacts);
        blocks.failed = false;
    }
    chat_filter::refresh(state);
    refresh_controls(state);
    render(state);
    let friends_open = state.friends_ui.borrow().is_some();
    let friends_pending = state
        .pending
        .borrow()
        .values()
        .any(|p| matches!(p, Pending::ListFriends));
    if friends_open
        && !friends_pending
        && let Ok(id) = state.sidecar.list_friends()
    {
        state.pending.borrow_mut().insert(id, Pending::ListFriends);
    }
}

pub(super) fn response(state: &AppState, account: &str, change: Option<(String, bool)>, ok: bool) {
    if state.self_mid.borrow().as_deref() != Some(account) || !*state.session_ready.borrow() {
        return;
    }
    if !ok {
        state.contact_blocks.borrow_mut().failed = true;
        toast(state, &crate::i18n::t("contact_block_failed"));
        if let Some(ui) = state.contact_blocks.borrow().ui.as_ref() {
            ui.status.set_text(&crate::i18n::t("contact_block_failed"));
        }
    } else if let Some((_, blocked)) = change {
        toast(
            state,
            &crate::i18n::t(if blocked {
                "contact_block_done"
            } else {
                "contact_unblock_done"
            }),
        );
    }
    refresh_controls(state);
    render(state);
}

fn submit(state: &AppState, account: &str, mid: &str, next: bool) {
    if state.self_mid.borrow().as_deref() != Some(account)
        || !*state.session_ready.borrow()
        || busy(state, Some(mid))
        || !mid.starts_with('u')
        || mid == account
    {
        return;
    }
    if blocked(state, mid) == Some(next) {
        return;
    }
    match state.sidecar.set_contact_blocked(mid, next) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::ContactBlocks {
                    account: account.into(),
                    change: Some((mid.into(), next)),
                },
            );
            render(state);
        }
        Err(_) => toast(state, &crate::i18n::t("contact_block_failed")),
    }
}

pub(super) fn change(state: &AppState, mid: &str, next: bool) {
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    if blocked(state, mid).is_none() {
        request_refresh(state);
        return;
    }
    if !next {
        submit(state, &account, mid, false);
        return;
    }
    let name = state
        .chats
        .borrow()
        .iter()
        .find(|c| c.mid == mid)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| mid.into());
    let dialog = libadwaita::AlertDialog::new(
        Some(&crate::i18n::tf(
            "contact_block_confirm",
            &[("name", &name)],
        )),
        Some(&crate::i18n::t("contact_block_body")),
    );
    dialog.add_response("cancel", &crate::i18n::t("contact_block_cancel"));
    dialog.add_response("block", &crate::i18n::t("block_contact"));
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("block", libadwaita::ResponseAppearance::Destructive);
    let s = state.clone();
    let mid = mid.to_string();
    dialog.connect_response(None, move |_, response| {
        if response == "block" {
            submit(&s, &account, &mid, true);
        }
    });
    dialog.present(Some(&state.window));
}

pub(super) fn wire(state: &AppState) {
    let action = gio::SimpleAction::new(
        "set-contact-blocked",
        Some(glib::VariantTy::new("(sb)").unwrap()),
    );
    let s = state.clone();
    action.connect_activate(move |_, value| {
        if let Some((mid, next)) = value.and_then(|v| v.get::<(String, bool)>()) {
            change(&s, &mid, next);
        }
    });
    state.window.add_action(&action);
    let action = gio::SimpleAction::new("refresh-contact-blocks", None);
    let s = state.clone();
    action.connect_activate(move |_, _| request_refresh(&s));
    state.window.add_action(&action);
}

pub(super) fn append_menu(state: &AppState, menu: &gio::Menu, mid: &str) {
    if !mid.starts_with('u') || state.self_mid.borrow().as_deref() == Some(mid) {
        return;
    }
    if let Some(blocked) = blocked(state, mid) {
        let item = gio::MenuItem::new(
            Some(&crate::i18n::t(if blocked {
                "unblock_contact"
            } else {
                "block_contact"
            })),
            None,
        );
        item.set_action_and_target_value(
            Some("win.set-contact-blocked"),
            Some(&(mid, !blocked).to_variant()),
        );
        menu.append_item(&item);
    } else {
        menu.append(
            Some(&crate::i18n::t("contact_blocks_refresh")),
            Some("win.refresh-contact-blocks"),
        );
    }
}

pub(super) fn refresh_row(state: &AppState, chat: &ChatInfo, row: &gtk::ListBoxRow) {
    let is_blocked = blocked(state, &chat.mid) == Some(true);
    fn visit(widget: &gtk::Widget, visible: bool) {
        if widget.has_css_class("line-chat-blocked") {
            widget.set_visible(visible);
            return;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            visit(&next, visible);
        }
    }
    if let Some(child) = row.child() {
        visit(&child, is_blocked);
    }
    if is_blocked {
        let label = crate::i18n::t("contact_blocked");
        row.set_tooltip_text(Some(&format!(
            "{}\n{label}",
            row.tooltip_text()
                .unwrap_or_else(|| chat.name.clone().into())
        )));
    }
}

pub(super) fn refresh_controls(state: &AppState) {
    let blocked = state
        .current_chat
        .borrow()
        .as_deref()
        .is_some_and(|mid| blocked(state, mid) == Some(true));
    user_profile::refresh_block(state);
    state.composer_row.set_sensitive(!blocked);
    state.reply_preview.set_sensitive(!blocked);
}

fn render(state: &AppState) {
    let blocks = state.contact_blocks.borrow();
    let Some(ui) = blocks.ui.as_ref() else { return };
    clear_list(&ui.list);
    let Some(contacts) = blocks.contacts.as_ref() else {
        ui.status.set_text(&crate::i18n::t(if blocks.failed {
            "contact_block_failed"
        } else {
            "contact_blocks_loading"
        }));
        return;
    };
    ui.status.set_text(&crate::i18n::t(if blocks.failed {
        "contact_block_failed"
    } else if contacts.is_empty() {
        "blocked_contacts_empty"
    } else {
        "blocked_contacts_description"
    }));
    for contact in contacts {
        let row = gtk::Box::builder()
            .spacing(12)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        row.append(
            &gtk::Label::builder()
                .label(&contact.name)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        let button = gtk::Button::with_label(&crate::i18n::t("unblock_contact"));
        button.set_sensitive(!busy(state, Some(&contact.mid)));
        let s = state.clone();
        let mid = contact.mid.clone();
        button.connect_clicked(move |_| change(&s, &mid, false));
        row.append(&button);
        ui.list.append(&row);
    }
}

pub(super) fn open(state: &AppState, parent: &gtk::Window) {
    if let Some(ui) = state.contact_blocks.borrow().ui.as_ref() {
        ui.window.present();
        request_refresh(state);
        return;
    }
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(crate::i18n::t("blocked_contacts"))
        .default_width(440)
        .default_height(480)
        .build();
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    window.set_titlebar(Some(&libadwaita::HeaderBar::new()));
    let refresh = gtk::Button::with_label(&crate::i18n::t("contact_blocks_refresh"));
    let s = state.clone();
    refresh.connect_clicked(move |_| request_refresh(&s));
    root.append(&refresh);
    let status = gtk::Label::builder()
        .wrap(true)
        .margin_start(12)
        .margin_end(12)
        .build();
    root.append(&status);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&list)
        .build();
    root.append(&scroll);
    window.set_child(Some(&root));
    window.connect_close_request(|win| {
        win.set_visible(false);
        glib::Propagation::Stop
    });
    state.contact_blocks.borrow_mut().ui = Some(BlockUi {
        window: window.clone(),
        list,
        status,
    });
    render(state);
    request_refresh(state);
    window.present();
}
