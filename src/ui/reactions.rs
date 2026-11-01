use super::*;
use crate::protocol::MessageReaction;

/// LINE's standard reactions: name, icon file in `assets/reactions`, label key.
const PREDEFINED: [(&str, &str, &str); 6] = [
    ("NICE", "nice.svg", "reaction_nice"),
    ("LOVE", "love.svg", "reaction_love"),
    ("FUN", "fun.svg", "reaction_fun"),
    ("AMAZING", "amazing.svg", "reaction_amazing"),
    ("SAD", "sad.svg", "reaction_sad"),
    ("OMG", "omg.svg", "reaction_omg"),
];
const EMOJI: &str = "EMOJI";

struct Slot {
    strip: gtk::Box,
    anchor: glib::WeakRef<gtk::Box>,
}

struct Picker {
    message_id: String,
    grid: glib::WeakRef<gtk::FlowBox>,
    tabs: glib::WeakRef<gtk::Box>,
}

#[derive(Clone)]
struct Pack {
    id: String,
    name: String,
    icon: Option<String>,
}

#[derive(Default)]
pub(super) struct ReactionUi {
    slots: HashMap<String, Slot>,
    recent: Vec<(String, i64, i64)>,
    recent_loading: bool,
    picker: Option<Picker>,
    packs: Vec<Pack>,
    packs_loaded: bool,
    packs_loading: bool,
    pack_emojis: HashMap<String, Vec<(String, i64, i64)>>,
    loading_packs: HashSet<String>,
    /// Empty means the recently seen emoji; otherwise a pack's product id.
    selected_tab: String,
    picker_images: HashMap<String, Vec<glib::WeakRef<gtk::Image>>>,
}

#[derive(Debug, Clone, PartialEq)]
struct Choice {
    kind: String,
    emoji: Option<String>,
    resource_type: i64,
    version: i64,
}

impl Choice {
    fn of(reaction: &MessageReaction) -> Self {
        Self {
            kind: reaction.kind.clone(),
            emoji: reaction.emoji.clone(),
            resource_type: reaction.resource_type.unwrap_or(1),
            version: reaction.version.unwrap_or(1),
        }
    }

    fn same(&self, reaction: &MessageReaction) -> bool {
        self.kind == reaction.kind && (self.kind != EMOJI || self.emoji == reaction.emoji)
    }
}

/// LINE only lets people react to messages others sent.
pub(super) fn eligible(message: &MessageInfo) -> bool {
    !message.mine
        && message.id.parse::<u128>().is_ok_and(|id| id > 0)
        && message.chat_event.is_none()
        && !message.decryption_failed
        && !["CALL", "CHATEVENT", "CHAT_EVENT", "UNSENT"]
            .contains(&message.content_type.to_ascii_uppercase().as_str())
}

fn known(reaction: &MessageReaction) -> bool {
    if reaction.kind == EMOJI {
        reaction.emoji.as_deref().is_some_and(valid_emoji_key)
    } else {
        PREDEFINED.iter().any(|(kind, ..)| *kind == reaction.kind)
    }
}

fn valid_emoji_key(key: &str) -> bool {
    key.split_once('_').is_some_and(|(product, id)| {
        product.len() == 24
            && product.chars().all(|c| c.is_ascii_hexdigit())
            && (1..=5).contains(&id.len())
            && id.chars().all(|c| c.is_ascii_digit())
    })
}

fn label_of(reaction: &MessageReaction) -> String {
    PREDEFINED
        .iter()
        .find(|(kind, ..)| *kind == reaction.kind)
        .map(|(_, _, key)| crate::i18n::t(key))
        .unwrap_or_else(|| crate::i18n::t("reaction_line_emoji"))
}

/// Reactions grouped by what was chosen, in the order they first appeared.
fn groups(reactions: &[MessageReaction]) -> Vec<Vec<&MessageReaction>> {
    let mut result: Vec<Vec<&MessageReaction>> = Vec::new();
    for reaction in reactions.iter().filter(|r| known(r)) {
        match result
            .iter_mut()
            .find(|group| Choice::of(group[0]).same(reaction))
        {
            Some(group) => group.push(reaction),
            None => result.push(vec![reaction]),
        }
    }
    result
}

fn update(list: &mut Vec<MessageReaction>, from: &str, reaction: Option<MessageReaction>) {
    list.retain(|item| item.from != from);
    if let Some(reaction) = reaction {
        list.push(reaction);
    }
}

fn self_mid(state: &AppState) -> String {
    state.self_mid.borrow().clone().unwrap_or_default()
}

fn mine(state: &AppState, message: &MessageInfo) -> Option<MessageReaction> {
    let account = self_mid(state);
    message
        .reactions
        .iter()
        .find(|reaction| !account.is_empty() && reaction.from == account)
        .cloned()
}

fn person(state: &AppState, mid: &str) -> String {
    if mid == self_mid(state) {
        crate::i18n::t("you")
    } else {
        message_senders::name(state, mid)
    }
}

fn icon(state: &AppState, kind: &str, emoji: Option<&str>, size: i32) -> gtk::Widget {
    if let Some((_, file, _)) = PREDEFINED.iter().find(|(name, ..)| *name == kind) {
        let path = state.repo_root.join("assets/reactions").join(file);
        let image = if path.is_file() {
            gtk::Image::from_file(path)
        } else {
            gtk::Image::from_icon_name("face-smile-symbolic")
        };
        image.set_pixel_size(size);
        image.add_css_class("line-reaction-icon");
        return image.upcast();
    }
    let image = match emoji.and_then(|key| state.emoji_textures.borrow().get(key).cloned()) {
        Some(texture) => gtk::Image::from_paintable(Some(&texture)),
        None => gtk::Image::from_icon_name("face-smile-symbolic"),
    };
    image.set_pixel_size(size);
    image.upcast()
}

fn choice_icon(child: &impl IsA<gtk::Widget>, selected: bool) -> gtk::Overlay {
    let overlay = gtk::Overlay::builder().child(child).build();
    if selected {
        let check = gtk::Image::builder()
            .icon_name("object-select-symbolic")
            .pixel_size(12)
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .can_target(false)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .css_classes(["line-reaction-check"])
            .build();
        overlay.add_overlay(&check);
    }
    overlay
}

fn pressed(button: &gtk::Button, selected: bool) {
    button.update_state(&[gtk::accessible::State::Pressed(if selected {
        gtk::AccessibleTristate::True
    } else {
        gtk::AccessibleTristate::False
    })]);
}

pub(super) fn strip(state: &AppState, message: &MessageInfo, anchor: &gtk::Box) -> gtk::Box {
    let strip = gtk::Box::builder()
        .spacing(4)
        .halign(if message.mine {
            gtk::Align::End
        } else {
            gtk::Align::Start
        })
        .css_classes(["line-reaction-strip"])
        .build();
    if message.id.parse::<u128>().is_ok_and(|id| id > 0) {
        let id = message.id.clone();
        let s = state.clone();
        let click = gtk::GestureClick::new();
        click.set_button(3);
        click.connect_pressed(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            show_list(&s, &id);
        });
        strip.add_controller(click);
        let id = message.id.clone();
        let s = state.clone();
        let press = gtk::GestureLongPress::new();
        press.connect_pressed(move |gesture, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            show_list(&s, &id);
        });
        strip.add_controller(press);
        // 仮想リストから行が外れたら、開いている選択欄も片付ける。
        anchor.connect_unrealize(|anchor| {
            let mut child = anchor.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                if widget.is::<gtk::Popover>() {
                    widget.unparent();
                }
            }
        });
        strip.connect_unrealize(|strip| {
            let mut child = strip.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                if widget.is::<gtk::Popover>() {
                    widget.unparent();
                }
            }
        });
        state.reactions.borrow_mut().slots.insert(
            message.id.clone(),
            Slot {
                strip: strip.clone(),
                anchor: anchor.downgrade(),
            },
        );
    }
    render(state, &strip, message);
    strip
}

fn render(state: &AppState, strip: &gtk::Box, message: &MessageInfo) {
    while let Some(child) = strip.first_child() {
        if child.is::<gtk::Popover>() {
            child.unparent();
        } else {
            strip.remove(&child);
        }
    }
    let account = self_mid(state);
    let groups = groups(&message.reactions);
    strip.set_visible(!groups.is_empty());
    for group in groups {
        let first = group[0];
        let own = group
            .iter()
            .any(|r| !account.is_empty() && r.from == account);
        let content = gtk::Box::builder().spacing(3).build();
        content.append(&icon(state, &first.kind, first.emoji.as_deref(), 18));
        content.append(
            &gtk::Label::builder()
                .label(group.len().to_string())
                .css_classes(["line-reaction-count"])
                .build(),
        );
        let names: Vec<String> = group
            .iter()
            .rev()
            .take(10)
            .map(|r| person(state, &r.from))
            .collect();
        let mut tooltip = names.join(", ");
        if group.len() > names.len() {
            tooltip.push_str(&crate::i18n::tf(
                "reaction_more_people",
                &[("n", &(group.len() - names.len()).to_string())],
            ));
        }
        let chip = gtk::Button::builder()
            .child(&content)
            .tooltip_text(&tooltip)
            .css_classes(["flat", "line-reaction-chip"])
            .build();
        if own {
            chip.add_css_class("line-reaction-mine");
            let check = gtk::Image::from_icon_name("object-select-symbolic");
            check.set_pixel_size(12);
            check.set_accessible_role(gtk::AccessibleRole::Presentation);
            content.append(&check);
        }
        pressed(&chip, own);
        chip.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tf(
            "reaction_chip",
            &[
                ("reaction", &label_of(first)),
                ("count", &group.len().to_string()),
            ],
        ))]);
        let s = state.clone();
        let id = message.id.clone();
        let choice = Choice::of(first);
        let can_react = eligible(message);
        chip.connect_clicked(move |_| {
            if can_react {
                send(&s, &id, if own { None } else { Some(choice.clone()) });
            } else {
                show_list(&s, &id);
            }
        });
        strip.append(&chip);
    }
    if strip.first_child().is_some() {
        let label = crate::i18n::t("reaction_list");
        let details = gtk::Button::builder()
            .icon_name("system-users-symbolic")
            .tooltip_text(&label)
            .action_name("win.show-reactions")
            .action_target(&message.id.to_variant())
            .css_classes(["flat", "circular", "line-reaction-details"])
            .build();
        details.update_property(&[gtk::accessible::Property::Label(&label)]);
        strip.append(&details);
    }
}

pub(super) fn refresh(state: &AppState, message: &MessageInfo) {
    let strip = state
        .reactions
        .borrow()
        .slots
        .get(&message.id)
        .map(|slot| slot.strip.clone());
    if let Some(strip) = strip {
        render(state, &strip, message);
        state.message_list.queue_allocate();
    }
}

/// A reaction was added, changed or removed, here or on another device.
pub(super) fn apply(
    state: &AppState,
    account: &str,
    _chat_mid: &str,
    message_id: &str,
    from: &str,
    reaction: Option<MessageReaction>,
) {
    // 表示中の吹き出しはIDで探す。1対1の chatMid は相手側とは限らない。
    if !*state.session_ready.borrow()
        || state.self_mid.borrow().as_deref() != Some(account)
        || from.is_empty()
        || reaction.as_ref().is_some_and(|r| r.from != from)
    {
        return;
    }
    let updated = {
        let mut messages = state.reply_messages.borrow_mut();
        let Some(message) = messages.get_mut(message_id) else {
            return;
        };
        if message.content_type == "UNSENT" {
            return;
        }
        update(&mut message.reactions, from, reaction.clone());
        message.clone()
    };
    // 同じ内容の再取得で、表示中の一覧を作り直さない。
    if let Some((_, list)) = state.msg_list_fp.borrow_mut().as_mut()
        && let Some(message) = list.iter_mut().find(|m| m.id == message_id)
    {
        update(&mut message.reactions, from, reaction);
    }
    refresh(state, &updated);
}

pub(super) fn forget(state: &AppState, id: &str) {
    let mut ui = state.reactions.borrow_mut();
    ui.slots.remove(id);
    if ui.picker.as_ref().is_some_and(|p| p.message_id == id) {
        ui.picker = None;
    }
}

pub(super) fn clear(state: &AppState) {
    let mut ui = state.reactions.borrow_mut();
    ui.slots.clear();
    ui.picker = None;
    ui.picker_images.clear();
    ui.packs.clear();
    ui.packs_loaded = false;
    ui.pack_emojis.clear();
    ui.loading_packs.clear();
    ui.selected_tab.clear();
}

pub(super) fn emoji_ready(state: &AppState, key: &str) {
    let affected: Vec<MessageInfo> = {
        let ui = state.reactions.borrow();
        let messages = state.reply_messages.borrow();
        ui.slots
            .keys()
            .filter_map(|id| messages.get(id))
            .filter(|m| m.reactions.iter().any(|r| r.emoji.as_deref() == Some(key)))
            .cloned()
            .collect()
    };
    for message in affected {
        refresh(state, &message);
    }
    let images = state
        .reactions
        .borrow_mut()
        .picker_images
        .remove(key)
        .unwrap_or_default();
    let texture = state.emoji_textures.borrow().get(key).cloned();
    if let Some(texture) = texture {
        for image in images.iter().filter_map(|image| image.upgrade()) {
            image.set_paintable(Some(&texture));
        }
    }
}

fn pending(state: &AppState, id: &str) -> bool {
    state
        .pending
        .borrow()
        .values()
        .any(|p| matches!(p, Pending::React { message_id, .. } if message_id == id))
}

fn send(state: &AppState, id: &str, choice: Option<Choice>) {
    if !*state.session_ready.borrow() || pending(state, id) {
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        return;
    };
    if !state.reply_messages.borrow().get(id).is_some_and(eligible) {
        return;
    }
    let request = match &choice {
        Some(choice) => state.sidecar.request(
            "react_message",
            Some(serde_json::json!({
                "chatMid": chat_mid,
                "messageId": id,
                "reaction": {
                    "kind": choice.kind,
                    "emoji": choice.emoji,
                    "resourceType": choice.resource_type,
                    "version": choice.version,
                },
            })),
        ),
        None => state.sidecar.request(
            "cancel_reaction",
            Some(serde_json::json!({"chatMid": chat_mid, "messageId": id})),
        ),
    };
    match request {
        Ok(request) => {
            state.pending.borrow_mut().insert(
                request,
                Pending::React {
                    account,
                    message_id: id.into(),
                },
            );
        }
        Err(_) => toast(state, &crate::i18n::t("reaction_failed")),
    }
}

fn error_text(error: Option<&str>) -> String {
    let error = error.unwrap_or_default();
    let key = [
        "reaction_expired",
        "reaction_unsent",
        "reaction_emoji_unavailable",
        "contact_blocked_send",
    ]
    .into_iter()
    .find(|key| error.contains(key))
    .unwrap_or("reaction_failed");
    crate::i18n::t(key)
}

pub(super) fn response(state: &AppState, account: &str, ok: bool, error: Option<&str>) {
    if ok || state.self_mid.borrow().as_deref() != Some(account) {
        return;
    }
    toast(state, &error_text(error));
}

fn request_recent(state: &AppState) {
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    if state.reactions.borrow().recent_loading {
        return;
    }
    if let Ok(request) = state.sidecar.request("reaction_emojis", None) {
        state.reactions.borrow_mut().recent_loading = true;
        state
            .pending
            .borrow_mut()
            .insert(request, Pending::ReactionEmojis { account });
    }
}

pub(super) fn recent_response(
    state: &AppState,
    account: &str,
    ok: bool,
    result: &serde_json::Value,
) {
    state.reactions.borrow_mut().recent_loading = false;
    if !ok || state.self_mid.borrow().as_deref() != Some(account) {
        return;
    }
    let recent: Vec<(String, i64, i64)> = result
        .get("emojis")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let key = item.get("key")?.as_str()?;
            valid_emoji_key(key).then(|| {
                (
                    key.to_string(),
                    item.get("resourceType")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(1),
                    item.get("version").and_then(|v| v.as_i64()).unwrap_or(1),
                )
            })
        })
        .take(48)
        .collect();
    state.reactions.borrow_mut().recent = recent;
    refill_picker(state);
}

fn popover_for(anchor: &gtk::Box, css: &str) -> gtk::Popover {
    let popover = gtk::Popover::builder()
        .css_classes([css])
        .position(gtk::PositionType::Top)
        .build();
    popover.set_parent(anchor);
    popover.connect_closed(|popover| {
        // 項目のアクションが終わるまで、親との接続を残す。
        let popover = popover.clone();
        glib::idle_add_local_once(move || {
            if popover.parent().is_some() {
                popover.unparent();
            }
        });
    });
    popover
}

fn anchor(state: &AppState, id: &str) -> Option<gtk::Box> {
    state
        .reactions
        .borrow()
        .slots
        .get(id)
        .and_then(|slot| slot.anchor.upgrade())
}

fn open_picker(state: &AppState, id: &str) {
    let Some(message) = state
        .reply_messages
        .borrow()
        .get(id)
        .filter(|m| eligible(m))
        .cloned()
    else {
        return;
    };
    let Some(anchor) = anchor(state, id) else {
        return;
    };
    let current = mine(state, &message);
    let popover = popover_for(&anchor, "line-reaction-picker");
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();
    root.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("react_message"))
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    let row = gtk::Box::builder().spacing(4).build();
    for (kind, _, key) in PREDEFINED {
        let selected = current.as_ref().is_some_and(|r| r.kind == kind);
        let button = gtk::Button::builder()
            .child(&choice_icon(&icon(state, kind, None, 28), selected))
            .tooltip_text(crate::i18n::t(key))
            .css_classes(["flat", "line-reaction-option"])
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(key))]);
        pressed(&button, selected);
        if selected {
            button.add_css_class("line-reaction-selected");
        }
        let s = state.clone();
        let id = id.to_string();
        let weak = popover.downgrade();
        button.connect_clicked(move |_| {
            if let Some(popover) = weak.upgrade() {
                popover.popdown();
            }
            let choice = Choice {
                kind: kind.into(),
                emoji: None,
                resource_type: 1,
                version: 1,
            };
            send(&s, &id, (!selected).then_some(choice));
        });
        row.append(&button);
    }
    root.append(&row);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("reaction_line_emoji"))
            .xalign(0.0)
            .css_classes(["heading", "caption"])
            .build(),
    );
    let tabs = gtk::Box::builder().spacing(2).build();
    root.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(288)
            .propagate_natural_height(true)
            .child(&tabs)
            .build(),
    );
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(6)
        .min_children_per_line(6)
        .homogeneous(true)
        .row_spacing(4)
        .column_spacing(4)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(168)
        .child(&grid)
        .build();
    root.append(&scroll);
    if current.is_some() {
        let remove = gtk::Button::builder()
            .label(crate::i18n::t("reaction_remove"))
            .css_classes(["flat"])
            .build();
        let s = state.clone();
        let id = id.to_string();
        let weak = popover.downgrade();
        remove.connect_clicked(move |_| {
            if let Some(popover) = weak.upgrade() {
                popover.popdown();
            }
            send(&s, &id, None);
        });
        root.append(&remove);
    }
    popover.set_child(Some(&root));
    state.reactions.borrow_mut().picker = Some(Picker {
        message_id: id.to_string(),
        grid: grid.downgrade(),
        tabs: tabs.downgrade(),
    });
    let s = state.clone();
    let picker_id = id.to_string();
    popover.connect_closed(move |_| {
        let mut ui = s.reactions.borrow_mut();
        if ui
            .picker
            .as_ref()
            .is_some_and(|p| p.message_id == picker_id)
        {
            ui.picker = None;
            ui.picker_images.clear();
        }
    });
    refill_tabs(state);
    refill_picker(state);
    request_recent(state);
    request_packs(state);
    let selected = state.reactions.borrow().selected_tab.clone();
    if !selected.is_empty() {
        request_pack(state, &selected);
    }
    popover.popup();
}

fn emoji_image(state: &AppState, key: &str, size: i32) -> gtk::Image {
    let texture = state.emoji_textures.borrow().get(key).cloned();
    let image = match &texture {
        Some(texture) => gtk::Image::from_paintable(Some(texture)),
        None => gtk::Image::from_icon_name("face-smile-symbolic"),
    };
    image.set_pixel_size(size);
    if texture.is_none() {
        state
            .reactions
            .borrow_mut()
            .picker_images
            .entry(key.to_string())
            .or_default()
            .push(image.downgrade());
    }
    image
}

fn tab_button(
    state: &AppState,
    child: &impl IsA<gtk::Widget>,
    tip: &str,
    tab: &str,
) -> gtk::Button {
    let active = state.reactions.borrow().selected_tab == tab;
    let button = gtk::Button::builder()
        .child(child)
        .tooltip_text(tip)
        .css_classes(["flat", "line-reaction-tab"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(tip)]);
    pressed(&button, active);
    if active {
        button.add_css_class("line-reaction-selected");
    }
    let s = state.clone();
    let tab = tab.to_string();
    button.connect_clicked(move |_| select_tab(&s, &tab));
    button
}

fn refill_tabs(state: &AppState) {
    let (tabs, packs) = {
        let ui = state.reactions.borrow();
        let Some(tabs) = ui.picker.as_ref().and_then(|p| p.tabs.upgrade()) else {
            return;
        };
        (tabs, ui.packs.clone())
    };
    while let Some(child) = tabs.first_child() {
        tabs.remove(&child);
    }
    let recent = gtk::Image::from_icon_name("document-open-recent-symbolic");
    recent.set_pixel_size(20);
    tabs.append(&tab_button(
        state,
        &recent,
        &crate::i18n::t("reaction_recent_tab"),
        "",
    ));
    for pack in packs {
        let image = match pack
            .icon
            .as_deref()
            .filter(|p| std::path::Path::new(p).is_file())
        {
            Some(path) => gtk::Image::from_file(path),
            None => gtk::Image::from_icon_name("face-smile-symbolic"),
        };
        image.set_pixel_size(24);
        let tip = if pack.name.is_empty() {
            crate::i18n::t("reaction_line_emoji")
        } else {
            pack.name.clone()
        };
        tabs.append(&tab_button(state, &image, &tip, &pack.id));
    }
}

fn select_tab(state: &AppState, tab: &str) {
    state.reactions.borrow_mut().selected_tab = tab.to_string();
    if !tab.is_empty() {
        request_pack(state, tab);
    }
    refill_tabs(state);
    refill_picker(state);
}

fn grid_note(grid: &gtk::FlowBox, key: &str) {
    grid.append(
        &gtk::Label::builder()
            .label(crate::i18n::t(key))
            .wrap(true)
            .max_width_chars(30)
            .css_classes(["dim-label", "caption"])
            .build(),
    );
}

fn refill_picker(state: &AppState) {
    let (id, grid, items, selected, loading) = {
        let ui = state.reactions.borrow();
        let Some(picker) = ui.picker.as_ref() else {
            return;
        };
        let Some(grid) = picker.grid.upgrade() else {
            return;
        };
        let selected = ui.selected_tab.clone();
        let items = if selected.is_empty() {
            Some(ui.recent.clone())
        } else {
            ui.pack_emojis.get(&selected).cloned()
        };
        let loading = ui.loading_packs.contains(&selected);
        (picker.message_id.clone(), grid, items, selected, loading)
    };
    let Some(message) = state.reply_messages.borrow().get(&id).cloned() else {
        return;
    };
    let current = mine(state, &message);
    state.reactions.borrow_mut().picker_images.clear();
    grid.remove_all();
    let Some(items) = items else {
        grid_note(
            &grid,
            if loading {
                "reaction_pack_loading"
            } else {
                "reaction_pack_failed"
            },
        );
        return;
    };
    if items.is_empty() {
        grid_note(
            &grid,
            if selected.is_empty() {
                "reaction_no_recent_emoji"
            } else {
                "reaction_pack_failed"
            },
        );
        return;
    }
    for (key, resource_type, version) in items {
        let selected = current
            .as_ref()
            .is_some_and(|r| r.kind == EMOJI && r.emoji.as_deref() == Some(key.as_str()));
        let button = gtk::Button::builder()
            .child(&choice_icon(&emoji_image(state, &key, 28), selected))
            .tooltip_text(crate::i18n::t("reaction_line_emoji"))
            .css_classes(["flat", "line-reaction-option"])
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
            "reaction_line_emoji",
        ))]);
        pressed(&button, selected);
        if selected {
            button.add_css_class("line-reaction-selected");
        }
        let s = state.clone();
        let id = id.clone();
        button.connect_clicked(move |button| {
            if let Some(popover) = button
                .ancestor(gtk::Popover::static_type())
                .and_downcast::<gtk::Popover>()
            {
                popover.popdown();
            }
            let choice = Choice {
                kind: EMOJI.into(),
                emoji: Some(key.clone()),
                resource_type,
                version,
            };
            send(&s, &id, (!selected).then_some(choice));
        });
        grid.append(&button);
    }
}

fn request_packs(state: &AppState) {
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    {
        let ui = state.reactions.borrow();
        if ui.packs_loaded || ui.packs_loading {
            return;
        }
    }
    if let Ok(request) = state.sidecar.request("reaction_emoji_packs", None) {
        state.reactions.borrow_mut().packs_loading = true;
        state
            .pending
            .borrow_mut()
            .insert(request, Pending::ReactionPacks { account });
    }
}

pub(super) fn packs_response(
    state: &AppState,
    account: &str,
    ok: bool,
    result: &serde_json::Value,
) {
    state.reactions.borrow_mut().packs_loading = false;
    if state.self_mid.borrow().as_deref() != Some(account) {
        return;
    }
    if !ok {
        toast(state, &crate::i18n::t("reaction_packs_failed"));
        return;
    }
    let packs: Vec<Pack> = result
        .get("packs")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?.to_string();
            valid_product_id(&id).then(|| Pack {
                name: item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                icon: item
                    .get("iconPath")
                    .and_then(|v| v.as_str())
                    .map(Into::into),
                id,
            })
        })
        .collect();
    {
        let mut ui = state.reactions.borrow_mut();
        if !ui.selected_tab.is_empty() && !packs.iter().any(|p| p.id == ui.selected_tab) {
            ui.selected_tab.clear();
        }
        ui.packs = packs;
        ui.packs_loaded = true;
    }
    refill_tabs(state);
    refill_picker(state);
}

fn request_pack(state: &AppState, product: &str) {
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    {
        let ui = state.reactions.borrow();
        if !valid_product_id(product)
            || ui.pack_emojis.contains_key(product)
            || ui.loading_packs.contains(product)
        {
            return;
        }
    }
    if let Ok(request) = state.sidecar.request(
        "reaction_emoji_pack",
        Some(serde_json::json!({"productId": product})),
    ) {
        state
            .reactions
            .borrow_mut()
            .loading_packs
            .insert(product.to_string());
        state.pending.borrow_mut().insert(
            request,
            Pending::ReactionPack {
                account,
                product: product.to_string(),
            },
        );
    }
}

pub(super) fn pack_response(
    state: &AppState,
    account: &str,
    product: &str,
    ok: bool,
    result: &serde_json::Value,
) {
    state.reactions.borrow_mut().loading_packs.remove(product);
    if state.self_mid.borrow().as_deref() != Some(account) {
        return;
    }
    if ok {
        let emojis: Vec<(String, i64, i64)> = result
            .get("emojis")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|item| {
                let key = item.get("key")?.as_str()?;
                (valid_emoji_key(key) && key.starts_with(product)).then(|| {
                    (
                        key.to_string(),
                        item.get("resourceType")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(1),
                        item.get("version").and_then(|v| v.as_i64()).unwrap_or(1),
                    )
                })
            })
            .collect();
        state
            .reactions
            .borrow_mut()
            .pack_emojis
            .insert(product.to_string(), emojis);
    }
    if state.reactions.borrow().selected_tab == product {
        refill_picker(state);
    }
}

fn valid_product_id(id: &str) -> bool {
    id.len() == 24 && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn show_list(state: &AppState, id: &str) {
    let Some(message) = state.reply_messages.borrow().get(id).cloned() else {
        return;
    };
    let mut reactions: Vec<MessageReaction> = message
        .reactions
        .iter()
        .filter(|r| known(r))
        .cloned()
        .collect();
    if reactions.is_empty() {
        return;
    }
    let Some(anchor) = state
        .reactions
        .borrow()
        .slots
        .get(id)
        .map(|slot| slot.strip.clone())
    else {
        return;
    };
    reactions.sort_by_key(|r| std::cmp::Reverse(r.at));
    let popover = popover_for(&anchor, "line-reaction-list");
    popover.set_position(gtk::PositionType::Bottom);
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();
    root.append(
        &gtk::Label::builder()
            .label(crate::i18n::tf(
                "reaction_list_title",
                &[("n", &reactions.len().to_string())],
            ))
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    for reaction in &reactions {
        let row = gtk::Box::builder().spacing(12).build();
        row.append(&user_profile::row_avatar(state, &reaction.from));
        let name = person(state, &reaction.from);
        let label = gtk::Label::builder()
            .label(&name)
            .tooltip_text(&name)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .width_chars(20)
            .max_width_chars(24)
            .build();
        if reaction.from != self_mid(state) {
            message_senders::register(state, &reaction.from, &label);
        }
        row.append(&label);
        row.append(&icon(state, &reaction.kind, reaction.emoji.as_deref(), 28));
        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "{name}: {}",
            label_of(reaction)
        ))]);
        list.append(&row);
    }
    root.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(280)
            .min_content_width(280)
            .child(&list)
            .build(),
    );
    popover.set_child(Some(&root));
    popover.popup();
}

pub(super) fn menu_items(message: &MessageInfo) -> Vec<gio::MenuItem> {
    let mut items = Vec::new();
    let target = message.id.to_variant();
    if eligible(message) {
        let item = gio::MenuItem::new(Some(&crate::i18n::t("react_message")), None);
        item.set_action_and_target_value(Some("win.react-message"), Some(&target));
        items.push(item);
    }
    if message.reactions.iter().any(known) {
        let item = gio::MenuItem::new(Some(&crate::i18n::t("reaction_list")), None);
        item.set_action_and_target_value(Some("win.show-reactions"), Some(&target));
        items.push(item);
    }
    items
}

pub(super) fn button(message: &MessageInfo) -> Option<gtk::Button> {
    if !eligible(message) {
        return None;
    }
    let label = crate::i18n::t("react_message");
    let button = gtk::Button::builder()
        .icon_name("face-smile-symbolic")
        .tooltip_text(&label)
        .action_target(&message.id.to_variant())
        .action_name("win.react-message")
        .valign(gtk::Align::Center)
        .css_classes([
            "flat",
            "circular",
            "line-message-action",
            "line-react-button",
        ])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&label)]);
    Some(button)
}

pub(super) fn wire(state: &AppState) {
    let react = gio::SimpleAction::new("react-message", Some(glib::VariantTy::STRING));
    let s = state.clone();
    react.connect_activate(move |_, value| {
        if let Some(id) = value.and_then(|v| v.str()) {
            open_picker(&s, id);
        }
    });
    state.window.add_action(&react);
    let list = gio::SimpleAction::new("show-reactions", Some(glib::VariantTy::STRING));
    let s = state.clone();
    list.connect_activate(move |_, value| {
        if let Some(id) = value.and_then(|v| v.str()) {
            show_list(&s, id);
        }
    });
    state.window.add_action(&list);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reaction(from: &str, kind: &str, emoji: Option<&str>) -> MessageReaction {
        MessageReaction {
            from: from.into(),
            at: 1,
            kind: kind.into(),
            emoji: emoji.map(Into::into),
            resource_type: None,
            version: None,
        }
    }

    #[test]
    fn only_others_confirmed_messages_can_be_reacted_to() {
        let mut message: MessageInfo = serde_json::from_value(
            serde_json::json!({"id":"9007199254740993","contentType":"NONE"}),
        )
        .unwrap();
        assert!(eligible(&message));
        message.mine = true;
        assert!(!eligible(&message));
        message.mine = false;
        message.id = "pending-1".into();
        assert!(!eligible(&message));
        message.id = "10".into();
        message.content_type = "UNSENT".into();
        assert!(!eligible(&message));
    }

    #[test]
    fn cached_chat_events_without_metadata_cannot_receive_reactions() {
        for content_type in ["CHATEVENT", "CHAT_EVENT", "chatEvent", "chat_event"] {
            let message: MessageInfo = serde_json::from_value(serde_json::json!({
                "id": "42",
                "contentType": content_type,
                "text": "[CHATEVENT]"
            }))
            .unwrap();
            assert!(message.chat_event.is_none());
            assert!(!eligible(&message), "{content_type}");
            assert!(button(&message).is_none());
        }
    }

    #[test]
    fn reactions_group_by_choice_and_skip_unknown_kinds() {
        let key = "5ac1bfd5040ab15980c9b435_001";
        let other = "5ac1bfd5040ab15980c9b435_002";
        let list = vec![
            reaction("u-a", "LOVE", None),
            reaction("u-b", EMOJI, Some(key)),
            reaction("u-c", "LOVE", None),
            reaction("u-d", EMOJI, Some(other)),
            reaction("u-e", EMOJI, Some("../bad")),
            reaction("u-f", "UNDO", None),
        ];
        let grouped = groups(&list);
        let sizes: Vec<_> = grouped
            .iter()
            .map(|g| (g[0].kind.as_str(), g[0].emoji.as_deref(), g.len()))
            .collect();
        assert_eq!(
            sizes,
            vec![
                ("LOVE", None, 2),
                (EMOJI, Some(key), 1),
                (EMOJI, Some(other), 1)
            ]
        );
    }

    #[test]
    fn one_reaction_per_person_is_replaced_or_removed() {
        let mut list = vec![reaction("u-a", "LOVE", None), reaction("u-b", "SAD", None)];
        update(&mut list, "u-a", Some(reaction("u-a", "FUN", None)));
        assert_eq!(
            list.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
            ["SAD", "FUN"]
        );
        update(&mut list, "u-b", None);
        assert_eq!(list.len(), 1);
        assert!(valid_emoji_key("5ac1bfd5040ab15980c9b435_001"));
        assert!(!valid_emoji_key("5ac1bfd5040ab15980c9b435_"));
    }

    #[test]
    fn errors_map_to_translated_messages() {
        assert_eq!(
            error_text(Some("reaction_expired")),
            crate::i18n::t("reaction_expired")
        );
        assert_eq!(
            error_text(Some("socket closed")),
            crate::i18n::t("reaction_failed")
        );
    }

    #[test]
    fn custom_reaction_choices_preserve_the_pack_version() {
        let mut value = reaction("u-a", EMOJI, Some("5ac1bfd5040ab15980c9b435_001"));
        assert_eq!(Choice::of(&value).version, 1);
        value.version = Some(7);
        value.resource_type = Some(2);
        let choice = Choice::of(&value);
        assert_eq!(choice.version, 7);
        assert_eq!(choice.resource_type, 2);
    }
}
