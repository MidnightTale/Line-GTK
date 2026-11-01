use super::*;
use crate::protocol::{StickerItem, StickerPack};
use serde_json::{Value, json};

type Completion = dyn Fn(&AppState, &Value);

pub(super) struct Request {
    scope: composer_drafts::Scope,
    status: glib::WeakRef<gtk::Label>,
    completed: Box<Completion>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StickerLibraryRequest")
            .finish_non_exhaustive()
    }
}

fn request(
    state: &AppState,
    method: &str,
    params: Value,
    status: &gtk::Label,
    completed: impl Fn(&AppState, &Value) + 'static,
) -> Option<u64> {
    match state.sidecar.request(method, Some(params)) {
        Ok(id) => {
            status.set_text(&crate::i18n::t("loading_stickers"));
            state.pending.borrow_mut().insert(
                id,
                Pending::StickerLibrary(Rc::new(Request {
                    scope: composer_drafts::scope(state),
                    status: status.downgrade(),
                    completed: Box::new(completed),
                })),
            );
            Some(id)
        }
        Err(e) => {
            status.set_text(&e.to_string());
            None
        }
    }
}

pub(super) fn response(
    state: &AppState,
    req: &Request,
    ok: bool,
    result: &Value,
    error: Option<&str>,
) {
    if composer_drafts::scope(state) != req.scope {
        return;
    }
    let Some(status) = req.status.upgrade() else {
        return;
    };
    if !ok {
        status.set_text(&crate::i18n::t(
            error.unwrap_or("sticker_import_unavailable"),
        ));
        return;
    }
    if let Some(account) = result.get("accountMid").and_then(Value::as_str)
        && state.self_mid.borrow().as_deref() != Some(account)
    {
        return;
    }
    status.set_text("");
    if let Some(aliases) = result.get("migratedOptions").and_then(Value::as_object)
        && let Err(error) = crate::sticker_image::migrate_options(&state.data_dir, aliases)
    {
        tracing::warn!(%error, "could not migrate sticker image settings");
    }
    (req.completed)(state, result);
}

pub(super) fn progress(state: &AppState, id: u64, account: &str, done: u64, total: u64) {
    if state.self_mid.borrow().as_deref() != Some(account) {
        return;
    }
    let pending = state.pending.borrow();
    if let Some(Pending::StickerLibrary(req)) = pending.get(&id)
        && req.scope == composer_drafts::scope(state)
        && let Some(status) = req.status.upgrade()
    {
        status.set_text(&crate::i18n::tf(
            "sticker_library_progress",
            &[("done", &done.to_string()), ("total", &total.to_string())],
        ));
    }
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build()
}

fn body() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build()
}

fn dialog(state: &AppState, title: &str) -> (libadwaita::Dialog, gtk::Box) {
    let dialog = libadwaita::Dialog::builder()
        .title(title)
        .content_width(480)
        .content_height(520)
        .build();
    let toolbar = libadwaita::ToolbarView::new();
    toolbar.add_top_bar(&libadwaita::HeaderBar::new());
    let content = body();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();
    toolbar.set_content(Some(&scroll));
    dialog.set_child(Some(&toolbar));
    track_dialog(state, &dialog);
    (dialog, content)
}

pub(super) fn track_dialog(state: &AppState, dialog: &impl IsA<libadwaita::Dialog>) {
    let dialog: &libadwaita::Dialog = dialog.as_ref();
    let mut dialogs = state.sticker_dialogs.borrow_mut();
    dialogs.retain(|d| d.upgrade().is_some());
    dialogs.push(dialog.downgrade());
}

pub(super) fn close(state: &AppState) {
    state.sticker_popover.popdown();
    let dialogs = state.sticker_dialogs.replace(Vec::new());
    for dialog in dialogs.into_iter().filter_map(|d| d.upgrade()) {
        if dialog.parent().is_some() {
            dialog.close();
        }
    }
}

pub(super) fn open(state: &AppState) {
    if state.current_chat.borrow().is_none() {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    }
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    let content = body();
    let status = label(&crate::i18n::t("loading_stickers"));
    content.append(&status);
    state.sticker_popover.set_child(Some(&content));
    state.sticker_popover.popup();
    request(state, "list_sticker_library", json!({}), &status, fill);
}

fn fill(state: &AppState, result: &Value) {
    fill_selected(state, result, None);
}

fn fill_selected(state: &AppState, result: &Value, selected: Option<String>) {
    let packs: Rc<Vec<StickerPack>> = Rc::new(
        serde_json::from_value(result.get("packs").cloned().unwrap_or(json!([])))
            .unwrap_or_default(),
    );
    let root = body();
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let store = gtk::Button::with_label(&crate::i18n::t("sticker_library_store"));
    let png = gtk::Button::with_label(&crate::i18n::t("sticker_library_png"));
    let s = state.clone();
    store.connect_clicked(move |_| {
        s.sticker_popover.popdown();
        store_dialog(&s);
    });
    let s = state.clone();
    let choices = packs.clone();
    png.connect_clicked(move |_| {
        s.sticker_popover.popdown();
        choose_pngs(&s, choices.clone(), None);
    });
    actions.append(&store);
    actions.append(&png);
    root.append(&actions);
    if result.get("legacyAvailable").and_then(Value::as_bool) == Some(true) {
        let adopt = gtk::Button::with_label(&crate::i18n::t("sticker_library_adopt"));
        adopt.set_tooltip_text(Some(&crate::i18n::t("sticker_library_adopt_hint")));
        let status = label("");
        root.append(&adopt);
        root.append(&status);
        let s = state.clone();
        adopt.connect_clicked(move |_| {
            request(&s, "adopt_sticker_library", json!({}), &status, |s, _| {
                open(s);
            });
        });
    }
    let selected_id = Rc::new(RefCell::new(selected));
    if packs.is_empty() {
        root.append(&label(&crate::i18n::t("stickers_empty")));
    } else {
        let tabs = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .build();
        for pack in packs.iter() {
            let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
            tabs.add_titled(&page, Some(&pack.id), &pack.name);
        }
        let switcher = gtk::StackSwitcher::builder().stack(&tabs).build();
        let mut child = switcher.first_child();
        for pack in packs.iter() {
            let Some(tab) = child else {
                break;
            };
            child = tab.next_sibling();
            tab.set_tooltip_text(Some(&pack.name));
            if let Some(button) = tab.downcast_ref::<gtk::ToggleButton>() {
                if let Some(path) = &pack.icon_path {
                    // Show the pack's thumbnail so packs are recognizable
                    // at a glance; the name stays as tooltip/accessible label.
                    let icon = gtk::Picture::builder()
                        .content_fit(gtk::ContentFit::Contain)
                        .can_shrink(true)
                        .build();
                    icon.set_size_request(36, 36);
                    icon.set_alternative_text(Some(&pack.name));
                    let path = path.clone();
                    icon.connect_map(move |picture| {
                        attach_texture_async_anim(picture.clone(), path.clone(), 72, false);
                    });
                    button.set_child(Some(&icon));
                    button.update_property(&[gtk::accessible::Property::Label(&pack.name)]);
                } else if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label.set_width_chars(10);
                    label.set_max_width_chars(16);
                }
            }
            if pack.image_only {
                wire_pack_menu(state, &tab, pack, packs.clone());
            }
        }
        let tab_scroll = gtk::ScrolledWindow::builder()
            // No visible bar: it sat over the tabs/stickers and stole clicks.
            // Wheel and touch scrolling still work.
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(300)
            .max_content_width(340)
            .propagate_natural_width(true)
            .child(&switcher)
            .build();
        root.append(&tab_scroll);
        root.append(&tabs);
        let index = selected_id
            .borrow()
            .as_ref()
            .and_then(|id| packs.iter().position(|p| &p.id == id))
            .unwrap_or(0);
        tabs.set_visible_child_name(&packs[index].id);
        *selected_id.borrow_mut() = Some(packs[index].id.clone());
        let panel = tabs.visible_child().unwrap();
        show_pack(
            state,
            panel.downcast_ref::<gtk::Box>().unwrap(),
            &packs[index],
        );
        let s = state.clone();
        let choices = packs.clone();
        let selected = selected_id.clone();
        tabs.connect_visible_child_notify(move |tabs| {
            let Some(id) = tabs.visible_child_name() else {
                return;
            };
            if let Some(pack) = choices.iter().find(|p| p.id == id) {
                *selected.borrow_mut() = Some(pack.id.clone());
                // Release hidden galleries and their previews. Only the active
                // pack loads images or requests native sticker metadata.
                let mut child = tabs.first_child();
                while let Some(page) = child {
                    child = page.next_sibling();
                    if let Some(panel) = page.downcast_ref::<gtk::Box>() {
                        clear(panel);
                    }
                }
                if let Some(page) = tabs.visible_child()
                    && let Some(panel) = page.downcast_ref::<gtk::Box>()
                {
                    show_pack(&s, panel, pack);
                }
            }
        });
    }
    let native_status = label("");
    root.append(&native_status);
    if result.get("ownedUnavailable").and_then(Value::as_bool) == Some(true) {
        native_status.set_text(&crate::i18n::t("sticker_library_owned_unavailable"));
        let retry = gtk::Button::with_label(&crate::i18n::t("sticker_library_retry_owned"));
        let s = state.clone();
        let status = native_status.clone();
        retry.connect_clicked(move |_| {
            request(
                &s,
                "list_stickers",
                json!({ "refreshOwned": true }),
                &status,
                fill,
            );
        });
        root.append(&retry);
    }
    state.sticker_popover.set_child(Some(&root));
    if result.get("ownedPackages").is_none() {
        request(
            state,
            "list_stickers",
            json!({}),
            &native_status,
            move |s, r| fill_selected(s, r, selected_id.borrow().clone()),
        );
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn show_pack(state: &AppState, panel: &gtk::Box, pack: &StickerPack) {
    clear(panel);
    let status = label("");
    panel.append(&status);
    let gallery = gtk::Box::new(gtk::Orientation::Vertical, 6);
    panel.append(&gallery);
    if pack.lazy {
        let weak = gallery.downgrade();
        let original = pack.clone();
        request(
            state,
            "sticker_owned_pack",
            json!({ "packageId": pack.id }),
            &status,
            move |s, r| {
                let Some(gallery) = weak.upgrade() else {
                    return;
                };
                let mut pack = original.clone();
                pack.lazy = false;
                pack.stickers =
                    serde_json::from_value(r.get("stickers").cloned().unwrap_or(json!([])))
                        .unwrap_or_default();
                render_page(s, &gallery, Rc::new(pack), 0);
            },
        );
    } else {
        render_page(state, &gallery, Rc::new(pack.clone()), 0);
    }
    panel.prepend(&label(&crate::i18n::t(if pack.image_only {
        "sticker_library_photos"
    } else {
        "sticker_library_native"
    })));
}

fn menu_action(
    state: &AppState,
    actions: &gio::SimpleActionGroup,
    menu: &gio::Menu,
    name: &str,
    text: &str,
    activate: impl Fn(&AppState) + 'static,
) {
    let action = gio::SimpleAction::new(name, None);
    let s = state.clone();
    let scope = composer_drafts::scope(state);
    action.connect_activate(move |_, _| {
        if composer_drafts::scope(&s) == scope {
            s.sticker_popover.popdown();
            activate(&s);
        }
    });
    actions.add_action(&action);
    menu.append(
        Some(&crate::i18n::t(text)),
        Some(&format!("sticker-library.{name}")),
    );
}

fn wire_pack_menu(
    state: &AppState,
    widget: &impl IsA<gtk::Widget>,
    pack: &StickerPack,
    choices: Rc<Vec<StickerPack>>,
) {
    let actions = gio::SimpleActionGroup::new();
    let menu = gio::Menu::new();
    if let Some(url) = pack.source_url.clone() {
        menu_action(
            state,
            &actions,
            &menu,
            "source",
            "sticker_library_source",
            move |_| open_uri(&url),
        );
    }
    if pack.source == "local_png" {
        let id = pack.id.clone();
        menu_action(
            state,
            &actions,
            &menu,
            "add",
            "sticker_library_add_png",
            move |s| choose_pngs(s, choices.clone(), Some(id.clone())),
        );
    }
    if pack.incomplete
        && let Some(link) = pack.import_pack.clone()
    {
        menu_action(
            state,
            &actions,
            &menu,
            "retry",
            "sticker_library_retry",
            move |s| download_dialog(s, &link),
        );
    }
    let p = pack.clone();
    menu_action(
        state,
        &actions,
        &menu,
        "rename",
        "sticker_library_rename",
        move |s| rename_dialog(s, &p),
    );
    let p = pack.clone();
    menu_action(
        state,
        &actions,
        &menu,
        "remove-pack",
        "sticker_library_remove_pack",
        move |s| confirm_remove(s, &p, None),
    );
    widget.insert_action_group("sticker-library", Some(&actions));
    wire_context_menu(widget, menu);
}

fn wire_image_menu(state: &AppState, button: &gtk::Button, pack: &StickerPack, item: &StickerItem) {
    let actions = gio::SimpleActionGroup::new();
    let menu = gio::Menu::new();
    if let Some(source) = image_source(item) {
        let input = source.clone();
        menu_action(
            state,
            &actions,
            &menu,
            "optimize",
            "sticker_optimize",
            move |s| optimize_dialog(s, input.clone()),
        );
        if !source.animated {
            menu_action(
                state,
                &actions,
                &menu,
                "edit",
                "sticker_library_edit",
                move |s| sticker_image_editor::open(s, source.clone()),
            );
        }
    }
    let p = pack.clone();
    let id = item.sticker_id.clone();
    menu_action(
        state,
        &actions,
        &menu,
        "remove-item",
        "sticker_library_remove_item",
        move |s| confirm_remove(s, &p, Some(id.clone())),
    );
    button.insert_action_group("sticker-library", Some(&actions));
    wire_context_menu(button, menu);
}

fn wire_context_menu(widget: &impl IsA<gtk::Widget>, menu: gio::Menu) {
    let widget = widget.as_ref();
    let popup = Rc::new(move |widget: &gtk::Widget, x: f64, y: f64| {
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(widget);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
        popover.grab_focus();
    });
    widget.connect_unrealize(|widget| {
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.is::<gtk::Popover>() {
                widget.unparent();
            }
        }
    });
    let click = gtk::GestureClick::new();
    click.set_button(3);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = widget.downgrade();
    let popup_c = popup.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        if let Some(widget) = weak.upgrade() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            popup_c(&widget, x, y);
        }
    });
    widget.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let weak = widget.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gdk::Key::Menu
            || (key == gdk::Key::F10 && modifiers.contains(gdk::ModifierType::SHIFT_MASK))
        {
            if let Some(widget) = weak.upgrade() {
                popup(&widget, 10.0, 10.0);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    widget.add_controller(keys);
}

fn render_page(state: &AppState, gallery: &gtk::Box, pack: Rc<StickerPack>, page: usize) {
    clear(gallery);
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(3)
        .max_children_per_line(4)
        .homogeneous(true)
        .row_spacing(4)
        .column_spacing(4)
        .build();
    let hover = sticker_preview::HoverPreview::default();
    for (index, item) in pack.stickers.iter().enumerate().skip(page * 60).take(60) {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let button = gtk::Button::builder()
            .css_classes(["flat", "line-sticker-cell"])
            .build();
        let choice = crate::i18n::tf(
            "sticker_choice",
            &[("pack", &pack.name), ("number", &(index + 1).to_string())],
        );
        button.update_property(&[gtk::accessible::Property::Label(&choice)]);
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .build();
        picture.set_size_request(64, 64);
        picture.set_alternative_text(Some(&choice));
        if let Some(path) = &item.image_path {
            let path = path.clone();
            let animate = state.config.borrow().animations;
            picture.connect_map(move |picture| {
                attach_texture_async_anim(picture.clone(), path.clone(), 96, animate);
            });
        }
        button.set_child(Some(&picture));
        hover.wire(
            &button,
            item.image_path.clone(),
            pack.name.clone(),
            state.config.borrow().animations,
        );
        let s = state.clone();
        let scope = composer_drafts::scope(state);
        let image_only = pack.image_only;
        let item_c = item.clone();
        button.connect_clicked(move |_| {
            if composer_drafts::scope(&s) != scope {
                return;
            }
            s.sticker_popover.popdown();
            if image_only {
                if let Some(source) = image_source(&item_c) {
                    sticker_image_editor::send_last(&s, &source);
                }
            } else {
                send_sticker_now(
                    &s,
                    &item_c.sticker_id,
                    &item_c.package_id,
                    item_c.version.as_deref(),
                    item_c.image_path.as_deref(),
                );
            }
        });
        if pack.image_only {
            button.set_sensitive(item.image_path.is_some());
            if let Some(error) = &item.error {
                button.set_tooltip_text(Some(&crate::i18n::t(error)));
            }
            wire_image_menu(state, &button, &pack, item);
        }
        cell.prepend(&button);
        grid.append(&cell);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        // Reserve a gutter so the scrollbar never overlays sticker buttons.
        .overlay_scrolling(false)
        .min_content_height(230)
        .max_content_height(360)
        .propagate_natural_height(true)
        .child(&grid)
        .build();
    gallery.append(&scroll);
    if pack.stickers.len() > 60 {
        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let previous = gtk::Button::with_label(&crate::i18n::t("sticker_library_previous"));
        previous.set_sensitive(page > 0);
        let next = gtk::Button::with_label(&crate::i18n::t("sticker_library_next"));
        next.set_sensitive((page + 1) * 60 < pack.stickers.len());
        let s = state.clone();
        let weak = gallery.downgrade();
        let p = pack.clone();
        previous.connect_clicked(move |_| {
            if let Some(g) = weak.upgrade() {
                render_page(&s, &g, p.clone(), page.saturating_sub(1));
            }
        });
        let s = state.clone();
        let weak = gallery.downgrade();
        let p = pack.clone();
        next.connect_clicked(move |_| {
            if let Some(g) = weak.upgrade() {
                render_page(&s, &g, p.clone(), page + 1);
            }
        });
        nav.append(&previous);
        nav.append(&label(&format!(
            "{} / {}",
            page + 1,
            pack.stickers.len().div_ceil(60)
        )));
        nav.append(&next);
        gallery.append(&nav);
    }
}

fn image_source(item: &StickerItem) -> Option<sticker_image_editor::Source> {
    Some(sticker_image_editor::Source {
        path: PathBuf::from(item.image_path.as_ref()?),
        key: item.option_key.clone(),
        personal: item.personal,
        animated: item.animated,
        optimized: item.optimized,
        optimization: item.optimization.clone(),
    })
}

pub(super) fn send_optimized(
    state: &AppState,
    source: &sticker_image_editor::Source,
    input: Option<PathBuf>,
) {
    let (dialog, content) = dialog(state, &crate::i18n::t("sticker_optimize_preparing"));
    let status = label("");
    content.append(&status);
    let cancel = gtk::Button::with_label(&crate::i18n::t("unsend_cancel"));
    content.append(&cancel);
    let weak = dialog.downgrade();
    let id = request(
        state,
        "prepare_sticker_image",
        json!({ "optionKey": source.key, "inputPath": input }),
        &status,
        move |s, r| {
            if let Some(path) = r.get("path").and_then(Value::as_str) {
                if let Some(d) = weak.upgrade() {
                    d.close();
                }
                try_send_local_media_path(s, PathBuf::from(path));
            }
        },
    );
    let weak = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.close();
        }
    });
    let s = state.clone();
    dialog.connect_closed(move |_| {
        if let Some(id) = id {
            let _ = s
                .sidecar
                .request("cancel_sticker_import", Some(json!({ "requestId": id })));
        }
    });
    dialog.present(Some(&state.window));
}

pub(super) fn optimize_dialog(state: &AppState, source: sticker_image_editor::Source) {
    let scope = composer_drafts::scope(state);
    let input = match sticker_image_editor::optimization_input(state, &source) {
        Ok(input) => input,
        Err(error) => {
            toast(state, &error.to_string());
            return;
        }
    };
    let (dialog, content) = dialog(state, &crate::i18n::t("sticker_optimize"));
    dialog.set_content_height(if source.animated { 680 } else { 580 });
    content.append(&label(&crate::i18n::t("sticker_optimize_hint")));
    if source.animated {
        content.append(&label(&crate::i18n::t("sticker_optimize_gif_hint")));
    }
    let dimensions = [0u64, 96, 128, 256, 384, 512, 1024];
    let original = crate::i18n::t("sticker_image_original");
    let resolution = gtk::DropDown::from_strings(&[
        &original, "96 px", "128 px", "256 px", "384 px", "512 px", "1024 px",
    ]);
    resolution.set_selected(
        dimensions
            .iter()
            .position(|n| {
                Some(*n)
                    == source
                        .optimization
                        .get("maxDimension")
                        .and_then(Value::as_u64)
            })
            .unwrap_or(if source.animated { 2 } else { 0 }) as u32,
    );
    resolution.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "sticker_optimize_resolution",
    ))]);

    let rates = [5u64, 8, 10, 15, 20, 30];
    let fps =
        gtk::DropDown::from_strings(&["5 fps", "8 fps", "10 fps", "15 fps", "20 fps", "30 fps"]);
    fps.set_selected(
        rates
            .iter()
            .position(|n| Some(*n) == source.optimization.get("fps").and_then(Value::as_u64))
            .unwrap_or(if source.animated { 2 } else { 3 }) as u32,
    );
    fps.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "sticker_optimize_fps",
    ))]);
    let palette = [32u64, 64, 128, 256];
    let colors = gtk::DropDown::from_strings(&["32", "64", "128", "256"]);
    colors.set_selected(
        palette
            .iter()
            .position(|n| Some(*n) == source.optimization.get("colors").and_then(Value::as_u64))
            .unwrap_or(if source.animated { 0 } else { 2 }) as u32,
    );
    colors.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "sticker_optimize_colors",
    ))]);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.set_homogeneous(source.animated);
    let resolution_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    resolution_box.set_hexpand(true);
    resolution_box.append(&label(&crate::i18n::t("sticker_optimize_resolution")));
    resolution_box.append(&resolution);
    controls.append(&resolution_box);
    if source.animated {
        let rate_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        rate_box.append(&label(&crate::i18n::t("sticker_optimize_fps")));
        rate_box.append(&fps);
        let colors_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        colors_box.append(&label(&crate::i18n::t("sticker_optimize_colors")));
        colors_box.append(&colors);
        controls.append(&rate_box);
        controls.append(&colors_box);
    }
    content.append(&controls);
    let preview = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let pic = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .alternative_text(crate::i18n::t("sticker_image_preview"))
        .build();
    pic.set_size_request(240, 200);
    preview.append(&pic);
    content.append(&preview);
    attach_texture_async_anim(
        pic,
        input
            .as_ref()
            .unwrap_or(&source.path)
            .to_string_lossy()
            .into_owned(),
        240,
        state.config.borrow().animations,
    );
    let status = label("");
    let build = gtk::Button::with_label(&crate::i18n::t("sticker_optimize_preview"));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let save = gtk::Button::with_label(&crate::i18n::t("sticker_optimize_save"));
    save.set_sensitive(false);
    let send = gtk::Button::with_label(&crate::i18n::t(if source.animated {
        "sticker_optimize_send_gif"
    } else {
        "send"
    }));
    send.set_sensitive(false);
    send.add_css_class("suggested-action");
    actions.append(&save);
    actions.append(&send);
    let footer = body();
    footer.append(&status);
    let preview_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    if source.animated {
        let compact = gtk::Button::with_label(&crate::i18n::t("sticker_optimize_small_gif"));
        compact.set_tooltip_text(Some(&crate::i18n::t("sticker_optimize_small_gif_hint")));
        let resolution = resolution.clone();
        let fps = fps.clone();
        let colors = colors.clone();
        let build = build.clone();
        compact.connect_clicked(move |_| {
            resolution.set_selected(2);
            fps.set_selected(2);
            colors.set_selected(0);
            build.emit_clicked();
        });
        preview_actions.append(&compact);
    }
    build.set_hexpand(true);
    preview_actions.append(&build);
    footer.append(&preview_actions);
    footer.append(&actions);
    if let Some(toolbar) = dialog
        .child()
        .and_then(|child| child.downcast::<libadwaita::ToolbarView>().ok())
    {
        toolbar.add_bottom_bar(&footer);
    }
    let result: Rc<RefCell<Option<Value>>> = Rc::new(RefCell::new(None));
    let generation = Rc::new(std::cell::Cell::new(0u64));
    let pending = Rc::new(std::cell::Cell::new(None));
    for control in [&resolution, &fps, &colors] {
        let result = result.clone();
        let generation = generation.clone();
        let save = save.downgrade();
        let send = send.downgrade();
        control.connect_selected_notify(move |_| {
            *result.borrow_mut() = None;
            generation.set(generation.get().wrapping_add(1));
            if let Some(b) = save.upgrade() {
                b.set_sensitive(false);
            }
            if let Some(b) = send.upgrade() {
                b.set_sensitive(false);
            }
        });
    }
    let s = state.clone();
    let result_c = result.clone();
    let pending_c = pending.clone();
    let generation_c = generation.clone();
    let key = source.key.clone();
    let weak_save = save.downgrade();
    let weak_send = send.downgrade();
    let weak_status = status.downgrade();
    build.connect_clicked(move |_| {
        if composer_drafts::scope(&s) != scope { return; }
        if let Some(id) = pending_c.get() { let _ = s.sidecar.request("cancel_sticker_import", Some(json!({ "requestId": id }))); }
        let options = json!({ "maxDimension": dimensions[resolution.selected() as usize], "fps": rates[fps.selected() as usize], "colors": palette[colors.selected() as usize] });
        let version = generation_c.get().wrapping_add(1); generation_c.set(version);
        *result_c.borrow_mut() = None;
        if let Some(b) = weak_save.upgrade() { b.set_sensitive(false); }
        if let Some(b) = weak_send.upgrade() { b.set_sensitive(false); }
        let result = result_c.clone(); let current = generation_c.clone(); let weak_preview = preview.downgrade();
        let weak_save = weak_save.clone(); let weak_send = weak_send.clone(); let weak_status = weak_status.clone();
        let Some(status) = weak_status.upgrade() else { return; };
        let id = request(&s, "prepare_sticker_image", json!({ "optionKey": key, "options": options, "inputPath": input }), &status, move |s, r| {
            if current.get() != version { return; }
            if let (Some(preview), Some(path)) = (weak_preview.upgrade(), r.get("path").and_then(Value::as_str)) {
                clear(&preview);
                let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).can_shrink(true).alternative_text(crate::i18n::t("sticker_image_preview")).build();
                picture.set_size_request(240, 200); preview.append(&picture);
                attach_texture_async_anim(picture, path.to_string(), 240, s.config.borrow().animations);
            }
            if let Some(status) = weak_status.upgrade() {
                let before = r.get("originalBytes").and_then(Value::as_u64).unwrap_or(0);
                let after = r.get("optimizedBytes").and_then(Value::as_u64).unwrap_or(0);
                status.set_text(&crate::i18n::tf("sticker_optimize_result", &[("before", &format!("{:.1}", before as f64 / 1024.0)), ("after", &format!("{:.1}", after as f64 / 1024.0))]));
            }
            *result.borrow_mut() = Some(r.clone());
            if let Some(b) = weak_save.upgrade() { b.set_sensitive(true); }
        if let Some(b) = weak_send.upgrade() { b.set_sensitive(true); }
        });
        pending_c.set(id);
    });
    let s = state.clone();
    let weak = dialog.downgrade();
    let result_c = result.clone();
    let status_c = status.clone();
    let key = source.key.clone();
    let scope = composer_drafts::scope(state);
    let applying = pending.clone();
    save.connect_clicked(move |_| {
        if composer_drafts::scope(&s) != scope {
            return;
        }
        if let Some(id) = applying.get()
            && s.pending.borrow().contains_key(&id)
        {
            return;
        }
        let Some(r) = result_c.borrow().clone() else {
            return;
        };
        let weak = weak.clone();
        let id = request(
            &s,
            "set_sticker_optimization",
            json!({ "optionKey": key, "options": r.get("options") }),
            &status_c,
            move |s, _| {
                if let Some(d) = weak.upgrade() {
                    d.close();
                }
                open(s);
            },
        );
        applying.set(id);
    });
    let s = state.clone();
    let weak = dialog.downgrade();
    let key = source.key.clone();
    let scope = composer_drafts::scope(state);
    let applying = pending.clone();
    send.connect_clicked(move |_| {
        if composer_drafts::scope(&s) != scope {
            return;
        }
        if let Some(id) = applying.get()
            && s.pending.borrow().contains_key(&id)
        {
            return;
        }
        let Some(r) = result.borrow().clone() else {
            return;
        };
        let weak = weak.clone();
        let id = request(
            &s,
            "set_sticker_optimization",
            json!({ "optionKey": key, "options": r.get("options") }),
            &status,
            move |s, _| {
                if let Some(d) = weak.upgrade() {
                    d.close();
                }
                if let Some(path) = r.get("path").and_then(Value::as_str) {
                    try_send_local_media_path(s, PathBuf::from(path));
                }
            },
        );
        applying.set(id);
    });
    let s = state.clone();
    dialog.connect_closed(move |_| {
        if let Some(id) = pending.get() {
            let _ = s
                .sidecar
                .request("cancel_sticker_import", Some(json!({ "requestId": id })));
        }
    });
    dialog.present(Some(&state.window));
}

fn store_dialog(state: &AppState) {
    let scope = composer_drafts::scope(state);
    let (dialog, content) = dialog(state, &crate::i18n::t("sticker_library_store"));
    content.append(&gtk::LinkButton::with_label(
        "https://store.line.me/stickershop/home/user/en",
        &crate::i18n::t("sticker_import_browse"),
    ));
    let entry = gtk::Entry::builder()
        .placeholder_text(crate::i18n::t("sticker_import_placeholder"))
        .build();
    let title = label(&crate::i18n::t("sticker_import_label"));
    title.set_mnemonic_widget(Some(&entry));
    entry.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "sticker_import_label",
    ))]);
    content.append(&title);
    content.append(&entry);
    let preview = gtk::Button::with_label(&crate::i18n::t("sticker_library_preview"));
    content.append(&preview);
    let status = label("");
    content.append(&status);
    let result_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.append(&result_box);
    let s = state.clone();
    let weak_dialog = dialog.downgrade();
    let generation = Rc::new(std::cell::Cell::new(0u64));
    preview.connect_clicked(move |_| {
        if composer_drafts::scope(&s) != scope {
            return;
        }
        let version = generation.get().wrapping_add(1);
        generation.set(version);
        let current = generation.clone();
        clear(&result_box);
        let link = entry.text().trim().to_string();
        let weak_box = result_box.downgrade();
        let weak_dialog = weak_dialog.clone();
        request(
            &s,
            "preview_sticker_pack",
            json!({ "link": link }),
            &status,
            move |s, r| {
                if current.get() != version {
                    return;
                }
                let Some(content) = weak_box.upgrade() else {
                    return;
                };
                clear(&content);
                content.append(&label(
                    r.get("name").and_then(Value::as_str).unwrap_or_default(),
                ));
                content.append(&label(&crate::i18n::tf(
                    "sticker_library_count",
                    &[(
                        "count",
                        &r.get("count")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            .to_string(),
                    )],
                )));
                content.append(&label(&crate::i18n::t("sticker_library_photos")));
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                for path in r
                    .get("previews")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    let pic = gtk::Picture::new();
                    pic.set_size_request(80, 80);
                    pic.set_content_fit(gtk::ContentFit::Contain);
                    attach_texture_async_anim(
                        pic.clone(),
                        path.to_string(),
                        96,
                        s.config.borrow().animations,
                    );
                    row.append(&pic);
                }
                content.append(&row);
                let download = gtk::Button::with_label(&crate::i18n::t("sticker_import_download"));
                download.add_css_class("suggested-action");
                content.append(&download);
                let s = s.clone();
                let link = link.clone();
                let weak_dialog = weak_dialog.clone();
                let scope = composer_drafts::scope(&s);
                download.connect_clicked(move |_| {
                    if composer_drafts::scope(&s) != scope {
                        return;
                    }
                    if let Some(d) = weak_dialog.upgrade() {
                        d.close();
                    }
                    download_dialog(&s, &link);
                });
            },
        );
    });
    dialog.present(Some(&state.window));
}

fn download_dialog(state: &AppState, link: &str) {
    let (dialog, content) = dialog(state, &crate::i18n::t("sticker_import_download"));
    let status = label("");
    content.append(&status);
    let cancel = gtk::Button::with_label(&crate::i18n::t("unsend_cancel"));
    content.append(&cancel);
    let weak = dialog.downgrade();
    let id = request(
        state,
        "import_sticker_pack",
        json!({ "link": link }),
        &status,
        move |s, r| {
            if let Some(d) = weak.upgrade() {
                d.close();
            }
            if r.get("cancelled").and_then(Value::as_bool) == Some(true) {
                toast(s, &crate::i18n::t("sticker_import_cancelled"));
            }
            open(s);
        },
    );
    let s = state.clone();
    let weak = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(id) = id {
            let _ = s
                .sidecar
                .request("cancel_sticker_import", Some(json!({ "requestId": id })));
        }
        if let Some(d) = weak.upgrade() {
            d.close();
        }
    });
    let s = state.clone();
    dialog.connect_closed(move |_| {
        if let Some(id) = id {
            let _ = s
                .sidecar
                .request("cancel_sticker_import", Some(json!({ "requestId": id })));
        }
    });
    dialog.present(Some(&state.window));
}

fn choose_pngs(state: &AppState, packs: Rc<Vec<StickerPack>>, target: Option<String>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("PNG"));
    filter.add_mime_type("image/png");
    filter.add_mime_type("image/apng");
    filter.add_pattern("*.png");
    filter.add_pattern("*.apng");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let chooser = gtk::FileDialog::builder()
        .title(crate::i18n::t("sticker_library_png"))
        .filters(&filters)
        .default_filter(&filter)
        .modal(true)
        .build();
    let s = state.clone();
    let scope = composer_drafts::scope(state);
    chooser.open_multiple(
        Some(&state.window),
        None::<&gio::Cancellable>,
        move |result| {
            if composer_drafts::scope(&s) != scope {
                return;
            }
            let Ok(files) = result else {
                return;
            };
            let paths: Vec<String> = (0..files.n_items())
                .filter_map(|i| {
                    files
                        .item(i)?
                        .downcast::<gio::File>()
                        .ok()?
                        .path()?
                        .to_str()
                        .map(String::from)
                })
                .collect();
            if paths.is_empty() {
                return;
            }
            png_review(&s, paths, packs.clone(), target.clone());
        },
    );
}

fn png_review(
    state: &AppState,
    paths: Vec<String>,
    packs: Rc<Vec<StickerPack>>,
    target: Option<String>,
) {
    let (dialog, content) = dialog(state, &crate::i18n::t("sticker_library_png"));
    content.append(&label(&crate::i18n::t("sticker_library_copies")));
    let local: Vec<StickerPack> = packs
        .iter()
        .filter(|p| p.source == "local_png")
        .cloned()
        .collect();
    let mut names = vec![crate::i18n::t("sticker_library_new_pack")];
    names.extend(local.iter().map(|p| p.name.clone()));
    let names_ref: Vec<&str> = names.iter().map(String::as_str).collect();
    let select = gtk::DropDown::from_strings(&names_ref);
    select.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "sticker_library_pack",
    ))]);
    if let Some(id) = target
        && let Some(index) = local.iter().position(|p| p.id == id)
    {
        select.set_selected(index as u32 + 1);
    }
    content.append(&label(&crate::i18n::t("sticker_library_pack")));
    content.append(&select);
    let name = gtk::Entry::builder()
        .placeholder_text(crate::i18n::t("sticker_library_pack_name"))
        .build();
    name.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "sticker_library_pack_name",
    ))]);
    name.set_sensitive(select.selected() == 0);
    content.append(&name);
    let weak_name = name.downgrade();
    select.connect_selected_notify(move |select| {
        if let Some(name) = weak_name.upgrade() {
            name.set_sensitive(select.selected() == 0);
        }
    });
    let status = label("");
    content.append(&status);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.append(&rows);
    let import = gtk::Button::with_label(&crate::i18n::t("sticker_library_import"));
    import.add_css_class("suggested-action");
    import.set_sensitive(false);
    content.append(&import);
    let close = gtk::Button::with_label(&crate::i18n::t("sticker_library_done"));
    content.append(&close);
    let weak = dialog.downgrade();
    let s = state.clone();
    let scope = composer_drafts::scope(state);
    close.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.close();
        }
        if composer_drafts::scope(&s) == scope {
            open(&s);
        }
    });
    let selections: Rc<RefCell<Vec<(String, gtk::CheckButton, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let chosen = selections.clone();
    let weak_rows = rows.downgrade();
    let weak_import = import.downgrade();
    let originals = paths.clone();
    request(
        state,
        "inspect_sticker_pngs",
        json!({ "paths": paths }),
        &status,
        move |s, r| {
            let (Some(rows), Some(import)) = (weak_rows.upgrade(), weak_import.upgrade()) else {
                return;
            };
            for file in r
                .get("files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let index = file
                    .get("index")
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::MAX) as usize;
                let Some(path) = originals.get(index) else {
                    continue;
                };
                let valid = file.get("valid").and_then(Value::as_bool) == Some(true);
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let file_name = std::path::Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let select = gtk::CheckButton::with_label(&file_name);
                select.set_active(valid);
                select.set_sensitive(valid);
                if valid {
                    let picture = gtk::Picture::new();
                    picture.set_size_request(64, 64);
                    attach_texture_async_anim(
                        picture.clone(),
                        file.get("previewPath")
                            .and_then(Value::as_str)
                            .unwrap_or(path)
                            .to_string(),
                        80,
                        s.config.borrow().animations,
                    );
                    row.append(&picture);
                }
                row.append(&select);
                rows.append(&row);
                if let Some(error) = file.get("error").and_then(Value::as_str) {
                    rows.append(&label(&crate::i18n::t(error)));
                }
                chosen.borrow_mut().push((
                    path.clone(),
                    select,
                    file.get("digest")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ));
            }
            import.set_sensitive(chosen.borrow().iter().any(|(_, b, _)| b.is_active()));
        },
    );
    let s = state.clone();
    let weak_status = status.downgrade();
    let pending = Rc::new(std::cell::Cell::new(None));
    let pending_c = pending.clone();
    let scope = composer_drafts::scope(state);
    import.connect_clicked(move |button| {
        if composer_drafts::scope(&s) != scope {
            return;
        }
        if let Some(id) = pending_c.get()
            && s.pending.borrow().contains_key(&id)
        {
            return;
        }
        let Some(status) = weak_status.upgrade() else {
            return;
        };
        let selected: Vec<String> = selections
            .borrow()
            .iter()
            .filter(|(_, b, _)| b.is_active())
            .map(|(p, _, _)| p.clone())
            .collect();
        if selected.is_empty() {
            return;
        }
        let digests: Vec<String> = selections.borrow().iter().filter(|(_, b, _)| b.is_active()).map(|(_, _, digest)| digest.clone()).collect();
        let target = select
            .selected()
            .checked_sub(1)
            .and_then(|i| local.get(i as usize))
            .map(|p| p.id.clone());
        if target.is_none() && name.text().trim().is_empty() {
            status.set_text(&crate::i18n::t("sticker_pack_name_invalid"));
            name.grab_focus();
            return;
        }
        let weak_status = status.downgrade();
        let weak_button = button.downgrade();
        let chosen = selections.clone();
        let pending = pending_c.clone();
        let original_selected = selected.clone();
        let id = request(
            &s,
            "import_sticker_pngs",
            json!({ "paths": selected, "name": name.text().as_str(), "packId": target, "digests": digests }),
            &status,
            move |_, r| {
                pending.set(None);
                let errors = r
                    .get("errors")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut details = crate::i18n::tf(
                    "sticker_library_imported",
                    &[(
                        "count",
                        &r.get("added")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            .to_string(),
                    )],
                );
                for e in &errors {
                    if let Some(index) = e.get("index").and_then(Value::as_u64)
                        && let Some(path) = original_selected.get(index as usize)
                    {
                        let file = std::path::Path::new(path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy();
                        details.push_str(&format!(
                            "\n{file}: {}",
                            crate::i18n::t(
                                e.get("error")
                                    .and_then(Value::as_str)
                                    .unwrap_or("sticker_png_unreadable")
                            )
                        ));
                    }
                }
                for (path, check, _) in chosen.borrow().iter() {
                    if let Some(index) = original_selected.iter().position(|p| p == path)
                        && !errors
                            .iter()
                            .any(|e| e.get("index").and_then(Value::as_u64) == Some(index as u64))
                    {
                        check.set_active(false);
                    }
                }
                if let Some(status) = weak_status.upgrade() {
                    status.set_text(&details);
                }
                if let Some(button) = weak_button.upgrade() {
                    button.set_sensitive(!errors.is_empty());
                }
            },
        );
        pending_c.set(id);
    });
    let s = state.clone();
    dialog.connect_closed(move |_| {
        if let Some(id) = pending.get() {
            let _ = s
                .sidecar
                .request("cancel_sticker_import", Some(json!({ "requestId": id })));
        }
    });
    dialog.present(Some(&state.window));
}

fn rename_dialog(state: &AppState, pack: &StickerPack) {
    let scope = composer_drafts::scope(state);
    let (dialog, content) = dialog(state, &crate::i18n::t("sticker_library_rename"));
    let entry = gtk::Entry::builder()
        .text(&pack.name)
        .max_length(100)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "sticker_library_pack_name",
    ))]);
    content.append(&label(&crate::i18n::t("sticker_library_pack_name")));
    content.append(&entry);
    let save = gtk::Button::with_label(&crate::i18n::t("sticker_library_save"));
    content.append(&save);
    let status = label("");
    content.append(&status);
    let s = state.clone();
    let id = pack.id.clone();
    let weak = dialog.downgrade();
    save.connect_clicked(move |_| {
        if composer_drafts::scope(&s) != scope {
            return;
        }
        let weak = weak.clone();
        request(
            &s,
            "update_sticker_library",
            json!({ "packId": id, "action": "rename", "value": entry.text().as_str() }),
            &status,
            move |s, _| {
                if let Some(d) = weak.upgrade() {
                    d.close();
                }
                open(s);
            },
        );
    });
    dialog.present(Some(&state.window));
}

fn confirm_remove(state: &AppState, pack: &StickerPack, item: Option<String>) {
    let scope = composer_drafts::scope(state);
    let dialog = libadwaita::AlertDialog::new(
        Some(&crate::i18n::t(if item.is_some() {
            "sticker_library_remove_item"
        } else {
            "sticker_library_remove_pack"
        })),
        Some(&crate::i18n::t("sticker_library_remove_hint")),
    );
    dialog.add_response("cancel", &crate::i18n::t("unsend_cancel"));
    dialog.add_response("remove", &crate::i18n::t("sticker_library_remove_item"));
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));
    dialog.set_response_appearance("remove", libadwaita::ResponseAppearance::Destructive);
    let s = state.clone();
    let id = pack.id.clone();
    dialog.connect_response(None, move |_, action| {
        if action != "remove" || composer_drafts::scope(&s) != scope { return; }
        let content = body(); let status = label(""); content.append(&status); s.sticker_popover.set_child(Some(&content));
        request(&s, "update_sticker_library", json!({ "packId": id, "action": if item.is_some() { "remove_item" } else { "remove_pack" }, "value": item.clone().unwrap_or_default() }), &status, |s, _| open(s));
    });
    track_dialog(state, &dialog);
    dialog.present(Some(&state.window));
}

#[cfg(test)]
mod tests;
