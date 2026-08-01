mod call_window;
mod diagnostics;
mod downloads;
mod friends;
mod login;
mod notifications;
mod settings;
mod shell;
mod state;
mod virtual_list;

use crate::config::{AppConfig, apply_animations, apply_font, apply_theme};
use crate::protocol::{ChatInfo, FlexAction, MessageInfo, Profile, ProtocolEvent};
use crate::sidecar::Sidecar;
use anyhow::Result;
use gdk_pixbuf::Pixbuf;
use gtk::gdk;
use gtk::prelude::*;
use gtk::{gio, glib};
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow};
use notifications::{notify_incoming, refresh_notification_media};
use state::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

pub fn run(repo_root: PathBuf, data_dir: PathBuf) -> Result<()> {
    let app = Application::builder()
        .application_id("dev.linegtk.LineGtk")
        .flags(gio::ApplicationFlags::FLAGS_NONE)
        .build();

    app.connect_activate(move |app| {
        if let Err(e) = build_ui(app, repo_root.clone(), data_dir.clone()) {
            eprintln!("failed to start UI: {e:#}");
        }
    });

    let code = app.run();
    if code != glib::ExitCode::SUCCESS {
        anyhow::bail!("gtk exited with {code:?}");
    }
    Ok(())
}

fn build_ui(app: &Application, repo_root: PathBuf, data_dir: PathBuf) -> Result<()> {
    shell::load_css();
    register_app_icons(&repo_root);
    let config = Rc::new(RefCell::new(AppConfig::load(&data_dir)));
    apply_theme(&config.borrow().theme);
    apply_font(&config.borrow().font_family, config.borrow().font_scale);
    apply_animations(config.borrow().animations);

    let sidecar = Rc::new(Sidecar::spawn(&repo_root, &data_dir)?);
    crate::i18n::set_lang(&repo_root, &config.borrow().language);

    let window = ApplicationWindow::builder()
        .application(app)
        .title(crate::i18n::t("app_title"))
        .icon_name("line-gtk")
        .default_width(1180)
        .default_height(760)
        .build();
    window.set_icon_name(Some("line-gtk"));
    window.set_size_request(WINDOW_MIN_W, WINDOW_MIN_H);

    let toast_overlay = libadwaita::ToastOverlay::new();
    let stack = gtk::Stack::new();
    // No crossfade on boot — avoids a visible login→shell flash.
    stack.set_transition_type(gtk::StackTransitionType::None);
    stack.set_transition_duration(0);

    let login = Rc::new(login::build_login_page());
    let shell = shell::build_shell_page();

    stack.add_named(&login.page, Some("login"));
    stack.add_named(&shell.page, Some("shell"));

    let has_saved_auth = saved_auth_exists(&data_dir);
    shell.brand_label.set_text(&crate::i18n::t("app_name"));
    apply_brand_icon(&shell.brand_icon, &repo_root);
    if has_saved_auth {
        // Already linked: open the main UI immediately while the sidecar restores.
        stack.set_visible_child_name("shell");
        shell.status.set_text(&crate::i18n::t("restoring"));
        shell.side_spinner.set_spinning(true);
        shell.side_spinner.set_visible(true);
        shell.side_stack.set_visible_child_name("loading");
        shell.msg_stack.set_visible_child_name("idle");
        shell.profile_label.set_text(&crate::i18n::t("app_name"));
    } else {
        stack.set_visible_child_name("login");
        login.hint.set_text(&crate::i18n::t("starting"));
    }

    toast_overlay.set_child(Some(&stack));
    window.set_content(Some(&toast_overlay));
    // Apply again after the display/window exist so ForceLight/ForceDark sticks.
    apply_theme(&config.borrow().theme);
    window.present();

    let (tray_tx, tray_rx) = async_channel::unbounded::<crate::tray::TrayAction>();

    let state = AppState {
        app: app.clone(),
        sidecar: sidecar.clone(),
        window: window.clone(),
        toast_overlay,
        stack,
        chat_list: shell.chat_list,
        message_list: shell.message_list,
        message_scroll: shell.message_scroll,
        composer: shell.composer,
        composer_row: shell.composer_row,
        conversation: shell.conversation,
        send_btn: shell.send_btn,
        status: shell.status,
        login,
        profile_label: shell.profile_label,
        profile_avatar: shell.profile_avatar,
        brand_label: shell.brand_label,
        brand_icon: shell.brand_icon,
        chat_title: shell.chat_title,
        chat_subtitle: shell.chat_subtitle,
        side_stack: shell.side_stack,
        side_spinner: shell.side_spinner,
        side_empty: shell.side_empty,
        side_load_label: shell.side_load_label,
        msg_stack: shell.msg_stack,
        msg_spinner: shell.msg_spinner,
        msg_empty: shell.msg_empty,
        msg_load_label: shell.msg_load_label,
        msg_idle: shell.msg_idle,
        current_chat: Rc::new(RefCell::new(None)),
        chats: Rc::new(RefCell::new(Vec::new())),
        chat_avatars: Rc::new(RefCell::new(HashMap::new())),
        chat_previews: Rc::new(RefCell::new(HashMap::new())),
        chat_unread_badges: Rc::new(RefCell::new(HashMap::new())),
        media_slots: Rc::new(RefCell::new(HashMap::new())),
        media_msgs: Rc::new(RefCell::new(HashMap::new())),
        receipt_slots: Rc::new(RefCell::new(HashMap::new())),
        msg_created: Rc::new(RefCell::new(HashMap::new())),
        last_msg_day: Rc::new(RefCell::new(None)),
        seen_msg_ids: Rc::new(RefCell::new(HashSet::new())),
        last_incoming_id: Rc::new(RefCell::new(None)),
        read_upto: Rc::new(RefCell::new(HashMap::new())),
        restored_last_chat: Rc::new(RefCell::new(false)),
        media_queue: Rc::new(RefCell::new(VecDeque::new())),
        media_pumping: Rc::new(RefCell::new(false)),
        stick_bottom: Rc::new(RefCell::new(true)),
        scroll_pinning: Rc::new(RefCell::new(false)),
        scroll_pin_gen: Rc::new(RefCell::new(0)),
        new_sep_row: Rc::new(RefCell::new(None)),
        pending_new_below: Rc::new(RefCell::new(0)),
        jump_banner: shell.jump_banner,
        jump_banner_btn: shell.jump_banner_btn,
        jump_banner_label: shell.jump_banner_label,
        pending: Rc::new(RefCell::new(HashMap::new())),
        pending_rows: Rc::new(RefCell::new(HashMap::new())),
        restarting: Rc::new(RefCell::new(false)),
        recovery_attempts: Rc::new(RefCell::new(0)),
        repo_root,
        data_dir,
        config,
        settings_btn: shell.settings_btn,
        friends_btn: shell.friends_btn,
        friends_ui: Rc::new(RefCell::new(None)),
        side_title: shell.side_title,
        search_entry: shell.search_entry,
        compact_search_btn: shell.compact_search_btn,
        compact_search_entry: shell.compact_search_entry,
        sidebar: shell.sidebar,
        sidebar_paned: shell.sidebar_paned,
        side_header: shell.side_header,
        sidebar_compact: Rc::new(RefCell::new(false)),
        composer_narrow: Rc::new(RefCell::new(false)),
        mic_btn: shell.mic_btn,
        attach_btn: shell.attach_btn,
        sticker_btn: shell.sticker_btn,
        sticker_popover: shell.sticker_popover,
        call_btn: shell.call_btn,
        mute_btn: shell.mute_btn,
        composer_stack: shell.composer_stack,
        record_cancel_btn: shell.record_cancel_btn,
        record_send_btn: shell.record_send_btn,
        record_timer: shell.record_timer,
        record_wave: shell.record_wave,
        upload_revealer: shell.upload_revealer,
        upload_bar: shell.upload_bar,
        upload_label: shell.upload_label,
        recording: Rc::new(RefCell::new(None)),
        recording_started: Rc::new(RefCell::new(None)),
        recording_levels: Rc::new(RefCell::new(vec![0.12; 40])),
        recording_tick: Rc::new(RefCell::new(None)),
        voice_playback: Rc::new(RefCell::new(None)),
        session_ready: Rc::new(RefCell::new(false)),
        active_call_peer: Rc::new(RefCell::new(None)),
        incoming_call_from: Rc::new(RefCell::new(None)),
        call_ui: Rc::new(RefCell::new(None)),
        call_mic_muted: Rc::new(RefCell::new(false)),
        call_deafened: Rc::new(RefCell::new(false)),
        tray: Rc::new(RefCell::new(None)),
        tray_tx,
        discord: crate::discord_rpc::DiscordRpc::start(),
        discord_session_start: Rc::new(RefCell::new(None)),
        self_mid: Rc::new(RefCell::new(None)),
        self_display_name: Rc::new(RefCell::new(String::new())),
        self_avatar_path: Rc::new(RefCell::new(None)),
        self_picture_url: Rc::new(RefCell::new(None)),
        msg_list_fp: Rc::new(RefCell::new(None)),
        notif_pending: Rc::new(RefCell::new(HashMap::new())),
        media_ready_paths: Rc::new(RefCell::new(HashMap::new())),
    };

    apply_ui_language(&state);
    apply_ui_motion(&state);
    wire_sidebar(&state);
    wire_composer_narrow(&state);
    wire_actions(&state);
    wire_scroll_pin(&state);
    wire_notification_actions(&state);
    sync_tray(&state);
    apply_close_behavior(&state);
    sync_discord_rpc(&state);
    {
        let discord = state.discord.clone();
        state.window.connect_destroy(move |_| {
            discord.shutdown();
        });
    }
    pump_tray_actions(state.clone(), tray_rx);
    pump_events(state);
    Ok(())
}

const SIDEBAR_MIN_PX: i32 = 80;
const SIDEBAR_MAX_PX: i32 = 520;
/// Enter avatar-only mode at or below this width.
const SIDEBAR_COMPACT_ENTER_PX: i32 = 180;
/// Leave compact only after dragging past this (hysteresis avoids snap-back).
const SIDEBAR_COMPACT_EXIT_PX: i32 = 280;
/// When leaving compact, snap open so header/search fit in one step.
const SIDEBAR_EXPAND_SNAP_PX: i32 = 320;
/// Leave at least this much for the conversation pane.
const CHAT_MIN_PX: i32 = 360;
const WINDOW_MIN_W: i32 = 720;
const WINDOW_MIN_H: i32 = 480;

fn register_app_icons(repo_root: &std::path::Path) {
    let icon_root = repo_root.join("assets/icons");
    if !icon_root.is_dir() {
        return;
    }
    if let Some(display) = gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        theme.add_search_path(&icon_root);
        // Some environments also probe size folders directly under the search path.
        for size in ["48x48", "64x64", "128x128", "256x256"] {
            theme.add_search_path(icon_root.join("hicolor").join(size).join("apps"));
        }
    }
    gtk::Window::set_default_icon_name("line-gtk");
}

fn apply_brand_icon(image: &gtk::Image, repo_root: &std::path::Path) {
    image.set_pixel_size(22);
    if let Some(display) = gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        if theme.has_icon("line-gtk") {
            image.set_icon_name(Some("line-gtk"));
            return;
        }
    }
    let png = repo_root.join("assets/icons/hicolor/64x64/apps/line-gtk.png");
    if png.is_file() {
        image.set_from_file(Some(&png));
    } else {
        image.set_icon_name(Some("line-gtk"));
    }
}

fn apply_close_behavior(state: &AppState) {
    let hide = {
        let cfg = state.config.borrow();
        cfg.tray_enabled && cfg.close_to_tray
    };
    state.window.set_hide_on_close(hide);
}

fn sync_tray(state: &AppState) {
    let want = state.config.borrow().tray_enabled;
    let has = state.tray.borrow().is_some();
    if want && !has {
        if let Some(ctl) = crate::tray::TrayController::spawn(state.tray_tx.clone()) {
            *state.tray.borrow_mut() = Some(ctl);
            refresh_tray_menu(state);
        }
    } else if !want && has {
        if let Some(ctl) = state.tray.borrow_mut().take() {
            ctl.shutdown();
        }
        // Never leave the app running with no window and no tray.
        state.window.set_visible(true);
        state.window.present();
    } else if want && has {
        refresh_tray_menu(state);
    }
    apply_close_behavior(state);
}

fn sync_discord_rpc(state: &AppState) {
    let cfg = state.config.borrow().clone();
    let app_id = cfg.discord_app_id();
    let enabled = cfg.discord_rpc && !app_id.is_empty();
    state.discord.configure(enabled, &app_id);

    if !enabled {
        state.discord.clear();
        return;
    }

    let display_name = state.self_display_name.borrow().clone();
    let avatar_url = if cfg.discord_rpc_show_avatar {
        state.self_picture_url.borrow().clone()
    } else {
        None
    };
    let show_name = cfg.discord_rpc_show_name && !display_name.is_empty();
    let app_name = crate::i18n::t("app_name");
    let small_text = if cfg.discord_rpc_show_avatar && show_name {
        Some(display_name.clone())
    } else if cfg.discord_rpc_show_avatar {
        Some(app_name.clone())
    } else {
        None
    };

    if !*state.session_ready.borrow() {
        let line = if state.stack.visible_child_name().as_deref() == Some("login") {
            crate::i18n::t("rpc_signing_in")
        } else {
            crate::i18n::t("rpc_connecting")
        };
        state.discord.set(crate::discord_rpc::Presence {
            details: app_name.clone(),
            state: line,
            start_unix: None,
            small_image: avatar_url,
            small_text,
            large_image: None,
            large_text: Some(app_name),
        });
        return;
    }

    if state.discord_session_start.borrow().is_none() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        *state.discord_session_start.borrow_mut() = Some(now);
    }
    let start = *state.discord_session_start.borrow();

    let state_line = if let Some(mid) = state.current_chat.borrow().as_ref() {
        if cfg.discord_rpc_show_chat {
            let name = state
                .chats
                .borrow()
                .iter()
                .find(|c| &c.mid == mid)
                .map(|c| c.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| crate::i18n::t("rpc_chat_fallback"));
            crate::i18n::tf("rpc_chatting_with", &[("name", &name)])
        } else {
            crate::i18n::t("rpc_in_conversation")
        }
    } else {
        crate::i18n::t("rpc_browsing_chats")
    };

    let details = if show_name {
        crate::i18n::tf("rpc_as_user", &[("name", &display_name)])
    } else {
        crate::i18n::t("rpc_unofficial_client")
    };

    state.discord.set(crate::discord_rpc::Presence {
        details,
        state: state_line,
        start_unix: start,
        small_image: avatar_url,
        small_text,
        large_image: None,
        large_text: Some(app_name),
    });
}

fn apply_self_profile(
    state: &AppState,
    mid: &str,
    display_name: &str,
    avatar_path: Option<&str>,
    picture_url: Option<&str>,
) {
    if !mid.is_empty() {
        *state.self_mid.borrow_mut() = Some(mid.to_string());
    }
    if !display_name.is_empty() {
        *state.self_display_name.borrow_mut() = display_name.to_string();
        state.profile_label.set_text(display_name);
        state.profile_label.set_tooltip_text(Some(display_name));
        state.profile_avatar.set_tooltip_text(Some(display_name));
    }
    if let Some(url) = picture_url.filter(|s| !s.is_empty()) {
        let full = if url.starts_with("http") {
            url.to_string()
        } else {
            format!(
                "https://profile.line-scdn.net{}{}",
                if url.starts_with('/') { "" } else { "/" },
                url
            )
        };
        *state.self_picture_url.borrow_mut() = Some(full);
    }
    state.profile_avatar.set_visible(true);
    if let Some(path) = avatar_path.filter(|p| !p.is_empty() && std::path::Path::new(p).exists()) {
        *state.self_avatar_path.borrow_mut() = Some(path.to_string());
        attach_texture_async(state.profile_avatar.clone(), path.to_string(), 64);
    }
    sync_discord_rpc(state);
}

fn notifications_muted_until_epoch(state: &AppState) -> i64 {
    let until = state.config.borrow().notifications_muted_until;
    if until <= 0 {
        return 0;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if until <= now {
        let mut cfg = state.config.borrow_mut();
        if cfg.notifications_muted_until != 0 {
            cfg.notifications_muted_until = 0;
            cfg.save(&state.data_dir);
        }
        0
    } else {
        until
    }
}

fn notifications_allowed(state: &AppState) -> bool {
    if !state.config.borrow().notifications {
        return false;
    }
    notifications_muted_until_epoch(state) == 0
}

fn refresh_tray_menu(state: &AppState) {
    let tray = state.tray.borrow();
    let Some(ctl) = tray.as_ref() else {
        return;
    };
    let muted_until = notifications_muted_until_epoch(state);
    let mut chats = state.chats.borrow().clone();
    chats.sort_by_key(|chat| std::cmp::Reverse(chat.last_activity));
    let recent: Vec<crate::tray::TrayChatItem> = chats
        .into_iter()
        .take(8)
        .map(|c| crate::tray::TrayChatItem {
            mid: c.mid,
            name: if c.name.is_empty() {
                "Chat".into()
            } else {
                c.name
            },
            unread: c.unread,
        })
        .collect();
    ctl.set_state(recent, muted_until, state.config.borrow().discord_rpc);
}

fn set_global_notif_mute(state: &AppState, secs: u64) {
    let until = if secs == 0 {
        0
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        now.saturating_add(secs as i64)
    };
    {
        let mut cfg = state.config.borrow_mut();
        cfg.notifications_muted_until = until;
        cfg.save(&state.data_dir);
    }
    refresh_tray_menu(state);
    if secs == 0 {
        toast(state, &crate::i18n::t("tray_unmute_ok"));
    } else if secs < 3600 {
        let m = (secs / 60).max(1);
        toast(
            state,
            &crate::i18n::tf("tray_mute_ok_min", &[("n", &m.to_string())]),
        );
    } else {
        let h = secs.div_ceil(3600).max(1);
        toast(
            state,
            &crate::i18n::tf("tray_mute_ok_hour", &[("n", &h.to_string())]),
        );
    }
    // Refresh tray title/menu when the mute expires.
    if secs > 0 {
        let s = state.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(secs + 1), move || {
            let _ = notifications_muted_until_epoch(&s);
            refresh_tray_menu(&s);
        });
    }
}

fn pump_tray_actions(state: AppState, rx: async_channel::Receiver<crate::tray::TrayAction>) {
    glib::spawn_future_local(async move {
        while let Ok(action) = rx.recv().await {
            match action {
                crate::tray::TrayAction::Show => {
                    present_main_window(&state);
                }
                crate::tray::TrayAction::OpenChat { mid } => {
                    present_and_open_chat(&state, &mid);
                }
                crate::tray::TrayAction::MuteFor { secs } => {
                    set_global_notif_mute(&state, secs);
                }
                crate::tray::TrayAction::ToggleDiscordRpc => {
                    {
                        let mut cfg = state.config.borrow_mut();
                        cfg.discord_rpc = !cfg.discord_rpc;
                        cfg.save(&state.data_dir);
                    }
                    sync_discord_rpc(&state);
                    refresh_tray_menu(&state);
                }
                crate::tray::TrayAction::Quit => {
                    if let Some(ctl) = state.tray.borrow_mut().take() {
                        ctl.shutdown();
                    }
                    state.app.quit();
                }
            }
        }
    });
}

fn present_main_window(state: &AppState) {
    state.window.set_visible(true);
    state.window.present();
}

fn present_and_open_chat(state: &AppState, mid: &str) {
    present_main_window(state);
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }
    let chat = state.chats.borrow().iter().find(|c| c.mid == mid).cloned();
    if let Some(chat) = chat {
        open_chat(state, &chat);
    } else {
        toast(state, &crate::i18n::t("tray_chat_missing"));
    }
}

fn wire_notification_actions(state: &AppState) {
    let open = gio::SimpleAction::new("open-chat", Some(glib::VariantTy::STRING));
    let s = state.clone();
    open.connect_activate(move |_a, param| {
        let Some(mid) = param.and_then(|p| p.str().map(str::to_string)) else {
            return;
        };
        present_and_open_chat(&s, &mid);
    });
    state.app.add_action(&open);

    let retry = gio::SimpleAction::new("retry-protocol", None);
    let s = state.clone();
    retry.connect_activate(move |_, _| {
        *s.recovery_attempts.borrow_mut() = 0;
        recover_sidecar(&s);
    });
    state.app.add_action(&retry);
}
/// Composer switches to icon-only send below this conversation width.
const COMPOSER_NARROW_PX: i32 = 420;
const AVATAR_CHAT_PX: i32 = 48;
const AVATAR_COMPACT_PX: i32 = 36;

fn wire_sidebar(state: &AppState) {
    // Hard floor so the paned cannot crush avatars.
    state.sidebar.set_size_request(SIDEBAR_MIN_PX, -1);
    state.sidebar_paned.set_shrink_start_child(false);
    state.sidebar_paned.set_resize_start_child(false);

    let saved = state
        .config
        .borrow()
        .sidebar_width
        .clamp(SIDEBAR_MIN_PX, SIDEBAR_MAX_PX);
    state.sidebar_paned.set_position(saved);
    apply_sidebar_compact(state, saved);

    let s = state.clone();
    let save_timer = Rc::new(RefCell::new(None::<glib::SourceId>));
    state
        .sidebar_paned
        .connect_notify_local(Some("position"), move |paned, _| {
            let pos = clamp_sidebar_position(&s, paned.position());
            if pos != paned.position() {
                paned.set_position(pos);
            }
            apply_sidebar_compact(&s, pos);
            if let Some(id) = save_timer.borrow_mut().take() {
                id.remove();
            }
            let s2 = s.clone();
            let timer = save_timer.clone();
            let timer_clear = save_timer.clone();
            *timer.borrow_mut() = Some(glib::timeout_add_local_once(
                std::time::Duration::from_millis(250),
                move || {
                    *timer_clear.borrow_mut() = None;
                    let width = clamp_sidebar_position(&s2, s2.sidebar_paned.position());
                    // Keep the last expanded width so compact sessions can restore later.
                    if width < SIDEBAR_COMPACT_EXIT_PX {
                        return;
                    }
                    {
                        let mut cfg = s2.config.borrow_mut();
                        if cfg.sidebar_width != width {
                            cfg.sidebar_width = width;
                            cfg.save(&s2.data_dir);
                        }
                    }
                },
            ));
        });

    // Re-clamp when the window shrinks so the chat pane stays usable.
    let s = state.clone();
    state
        .sidebar_paned
        .connect_notify_local(Some("width"), move |_, _| {
            let pos = clamp_sidebar_position(&s, s.sidebar_paned.position());
            if pos != s.sidebar_paned.position() {
                s.sidebar_paned.set_position(pos);
            }
            apply_composer_narrow(&s);
        });
}

fn clamp_sidebar_position(state: &AppState, desired: i32) -> i32 {
    let total = state.sidebar_paned.width();
    let max_for_chat = if total > CHAT_MIN_PX + SIDEBAR_MIN_PX {
        (total - CHAT_MIN_PX).min(SIDEBAR_MAX_PX)
    } else if total > 0 {
        SIDEBAR_MIN_PX.max(total / 3)
    } else {
        SIDEBAR_MAX_PX
    };
    desired.clamp(SIDEBAR_MIN_PX, max_for_chat.max(SIDEBAR_MIN_PX))
}

fn wire_composer_narrow(state: &AppState) {
    let s = state.clone();
    state
        .conversation
        .connect_notify_local(Some("width"), move |_, _| {
            apply_composer_narrow(&s);
        });
    apply_composer_narrow(state);
}

fn apply_composer_narrow(state: &AppState) {
    let width = state.conversation.width();
    if width <= 1 {
        return;
    }
    let narrow = width < COMPOSER_NARROW_PX;
    let was = *state.composer_narrow.borrow();
    if was == narrow {
        return;
    }
    *state.composer_narrow.borrow_mut() = narrow;
    if narrow {
        state.composer_row.add_css_class("line-composer-narrow");
        state.send_btn.set_label("");
        state.send_btn.set_icon_name("mail-send-symbolic");
        state
            .send_btn
            .set_tooltip_text(Some(&crate::i18n::t("send")));
    } else {
        state.composer_row.remove_css_class("line-composer-narrow");
        state.send_btn.set_icon_name("");
        state.send_btn.set_label(&crate::i18n::t("send"));
        state.send_btn.set_tooltip_text(None);
    }
}

fn apply_sidebar_compact(state: &AppState, width: i32) {
    let was = *state.sidebar_compact.borrow();
    let compact = if was {
        width < SIDEBAR_COMPACT_EXIT_PX
    } else {
        width <= SIDEBAR_COMPACT_ENTER_PX
    };
    if was != compact {
        *state.sidebar_compact.borrow_mut() = compact;
        if compact {
            state.sidebar.add_css_class("line-sidebar-compact");
        } else {
            state.sidebar.remove_css_class("line-sidebar-compact");
            // Expanding reveals header/search; snap wide enough so min-size does not
            // push the paned back into compact.
            if width < SIDEBAR_EXPAND_SNAP_PX {
                let snap = clamp_sidebar_position(state, SIDEBAR_EXPAND_SNAP_PX);
                if snap != state.sidebar_paned.position() {
                    state.sidebar_paned.set_position(snap);
                }
            }
        }
        state.search_entry.set_visible(!compact);
        state.side_title.set_visible(!compact);
        state.side_header.set_visible(!compact);
        state.compact_search_btn.set_visible(compact);
    }
    set_chat_rows_compact(state, compact);
}

fn set_chat_rows_compact(state: &AppState, compact: bool) {
    let mut child = state.chat_list.first_child();
    while let Some(w) = child {
        let next = w.next_sibling();
        if let Ok(row) = w.downcast::<gtk::ListBoxRow>() {
            apply_chat_row_compact(&row, compact);
        }
        child = next;
    }
}

fn apply_chat_row_compact(row: &gtk::ListBoxRow, compact: bool) {
    let Some(box_w) = row.child() else {
        return;
    };
    let Ok(box_) = box_w.downcast::<gtk::Box>() else {
        return;
    };
    let mut child = box_.first_child();
    while let Some(w) = child {
        let next = w.next_sibling();
        if w.css_classes().iter().any(|c| c == "line-chat-text") {
            w.set_visible(!compact);
        }
        if let Ok(overlay) = w.clone().downcast::<gtk::Overlay>() {
            set_avatar_tier(&overlay, compact);
        }
        child = next;
    }
    if compact {
        box_.set_halign(gtk::Align::Center);
        box_.set_hexpand(false);
        box_.set_margin_start(0);
        box_.set_margin_end(0);
        box_.set_margin_top(8);
        box_.set_margin_bottom(8);
        box_.set_spacing(0);
    } else {
        box_.set_halign(gtk::Align::Fill);
        box_.set_hexpand(true);
        box_.set_margin_start(8);
        box_.set_margin_end(8);
        box_.set_margin_top(8);
        box_.set_margin_bottom(8);
        box_.set_spacing(12);
    }
}

fn set_avatar_tier(overlay: &gtk::Overlay, compact: bool) {
    let px = if compact {
        AVATAR_COMPACT_PX
    } else {
        AVATAR_CHAT_PX
    };
    if let Some(frame_w) = overlay.child()
        && let Ok(frame) = frame_w.downcast::<gtk::Box>()
    {
        frame.set_width_request(px);
        frame.set_height_request(px);
        if let Some(pic_w) = frame.first_child()
            && let Ok(pic) = pic_w.downcast::<gtk::Picture>()
        {
            pic.set_width_request(px);
            pic.set_height_request(px);
        }
    }
}

fn apply_ui_motion(state: &AppState) {
    let on = state.config.borrow().animations;
    apply_animations(on);
    if on {
        state.window.add_css_class("line-anim");
    } else {
        state.window.remove_css_class("line-anim");
    }

    let (stack_ty, stack_ms) = if on {
        (gtk::StackTransitionType::Crossfade, 180)
    } else {
        (gtk::StackTransitionType::None, 0)
    };
    // Keep boot→shell instant until logged in; only animate shell sub-stacks here.
    state.side_stack.set_transition_type(stack_ty);
    state.side_stack.set_transition_duration(stack_ms);
    state.msg_stack.set_transition_type(stack_ty);
    state.msg_stack.set_transition_duration(stack_ms);

    let (rev_ty, rev_ms) = if on {
        (gtk::RevealerTransitionType::SlideUp, 180)
    } else {
        (gtk::RevealerTransitionType::None, 0)
    };
    state.jump_banner.set_transition_type(rev_ty);
    state.jump_banner.set_transition_duration(rev_ms);

    state.login.stage.set_transition_type(stack_ty);
    state
        .login
        .stage
        .set_transition_duration(if on { 200 } else { 0 });
}

fn wire_scroll_pin(state: &AppState) {
    // Content height changed: if we are stuck, stay on the real bottom.
    let stick = state.stick_bottom.clone();
    let pinning = state.scroll_pinning.clone();
    let adj = state.message_scroll.vadjustment();
    adj.connect_changed(move |adj| {
        if !*stick.borrow() {
            return;
        }
        let already = *pinning.borrow();
        *pinning.borrow_mut() = true;
        let target = (adj.upper() - adj.page_size()).max(0.0);
        if (adj.value() - target).abs() > 0.5 {
            adj.set_value(target);
        }
        // Don't clear an in-flight pin loop.
        if !already {
            *pinning.borrow_mut() = false;
        }
    });

    // User wheel / touchpad: scrolling toward older messages unsticks.
    let s_scroll = state.clone();
    let wheel = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::KINETIC,
    );
    wheel.connect_scroll(move |_, _dx, dy| {
        if *s_scroll.scroll_pinning.borrow() {
            return glib::Propagation::Proceed;
        }
        // Negative dy = scroll up (older messages).
        if dy < -0.1 && *s_scroll.stick_bottom.borrow() {
            let adj = s_scroll.message_scroll.vadjustment();
            let target = (adj.upper() - adj.page_size()).max(0.0);
            if adj.value() + 120.0 < target {
                *s_scroll.stick_bottom.borrow_mut() = false;
            }
        }
        glib::Propagation::Proceed
    });
    state.message_scroll.add_controller(wheel);

    let s = state.clone();
    let adj = state.message_scroll.vadjustment();
    adj.connect_value_changed(move |adj| {
        // Ignore programmatic pins (append / layout settle / changed snap).
        if *s.scroll_pinning.borrow() {
            return;
        }
        let target = (adj.upper() - adj.page_size()).max(0.0);
        let near_bottom = adj.value() + 120.0 >= target;
        if near_bottom {
            if !*s.stick_bottom.borrow() {
                *s.stick_bottom.borrow_mut() = true;
                *s.pending_new_below.borrow_mut() = 0;
                update_jump_banner(&s);
            }
        } else if *s.stick_bottom.borrow() {
            // Scrollbar drag / key scroll away from bottom.
            *s.stick_bottom.borrow_mut() = false;
        }
    });
}

fn send_current(state: &AppState) {
    let text = state.composer.text().trim().to_string();
    if text.is_empty() {
        return;
    }
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    match state.sidecar.send_message(&chat_mid, &text) {
        Ok(id) => {
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
                    content_type: "NONE".into(),
                    image_path: None,
                    audio_path: None,
                    file_name: None,
                    file_path: None,
                    duration_ms: None,
                    flex: None,
                },
            );
            state.composer.set_text("");
            dismiss_new_marker(state);
            pin_messages_to_latest(state);
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("send_failed", &[("error", &e.to_string())]),
        ),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn remove_pending_placeholder(state: &AppState, placeholder_id: &str) {
    if let Some(row) = state.pending_rows.borrow_mut().remove(placeholder_id) {
        state.message_list.remove(&row);
    }
    state.seen_msg_ids.borrow_mut().remove(placeholder_id);
    state.media_slots.borrow_mut().remove(placeholder_id);
    state.media_msgs.borrow_mut().remove(placeholder_id);
    state.receipt_slots.borrow_mut().remove(placeholder_id);
    state.msg_created.borrow_mut().remove(placeholder_id);
}

fn begin_optimistic_send(state: &AppState, req_id: u64, chat_mid: &str, mut msg: MessageInfo) {
    let placeholder_id = format!("pending-{req_id}");
    msg.id = placeholder_id.clone();
    msg.mine = true;
    if msg.created_time <= 0 {
        msg.created_time = now_ms();
    }
    if state.current_chat.borrow().as_deref() == Some(chat_mid) {
        if state.msg_stack.visible_child_name().as_deref() != Some("list") {
            set_msg_state(state, "list", None);
        }
        append_message(state, &msg, true);
    }
    state.pending.borrow_mut().insert(
        req_id,
        Pending::Send {
            chat_mid: chat_mid.into(),
            placeholder_id,
        },
    );
}

fn apply_ui_language(state: &AppState) {
    crate::i18n::set_lang(&state.repo_root, &state.config.borrow().language);
    state.side_title.set_text(&crate::i18n::t("chats"));
    state
        .composer
        .set_placeholder_text(Some(&crate::i18n::t("type_message")));
    if *state.composer_narrow.borrow() {
        state.send_btn.set_label("");
        state.send_btn.set_icon_name("mail-send-symbolic");
        state
            .send_btn
            .set_tooltip_text(Some(&crate::i18n::t("send")));
    } else {
        state.send_btn.set_icon_name("");
        state.send_btn.set_label(&crate::i18n::t("send"));
        state.send_btn.set_tooltip_text(None);
    }
    state
        .settings_btn
        .set_tooltip_text(Some(&crate::i18n::t("settings")));
    state
        .friends_btn
        .set_tooltip_text(Some(&crate::i18n::t("friends")));
    state
        .mic_btn
        .set_tooltip_text(Some(&crate::i18n::t("voice_message")));
    state
        .record_cancel_btn
        .set_tooltip_text(Some(&crate::i18n::t("voice_cancel")));
    state
        .record_send_btn
        .set_tooltip_text(Some(&crate::i18n::t("voice_send")));
    state
        .attach_btn
        .set_tooltip_text(Some(&crate::i18n::t("attach_file")));
    state
        .sticker_btn
        .set_tooltip_text(Some(&crate::i18n::t("stickers")));
    state
        .search_entry
        .set_placeholder_text(Some(&crate::i18n::t("search")));
    state
        .compact_search_entry
        .set_placeholder_text(Some(&crate::i18n::t("search")));
    state
        .compact_search_btn
        .set_tooltip_text(Some(&crate::i18n::t("search")));
    state
        .side_load_label
        .set_text(&crate::i18n::t("loading_chats"));
    state.side_empty.set_text(&crate::i18n::t("no_chats"));
    state
        .msg_load_label
        .set_text(&crate::i18n::t("loading_messages"));
    state.msg_empty.set_text(&crate::i18n::t("no_messages"));
    state
        .msg_idle
        .set_text(&crate::i18n::t("select_chat_start"));
    if state.current_chat.borrow().is_none() {
        state.chat_title.set_text(&crate::i18n::t("select_chat"));
        state.chat_subtitle.set_text(&crate::i18n::t("pick_chat"));
    }
    login::apply_login_language(&state.login);
    refresh_call_controls(state);
    state.window.set_title(Some(&crate::i18n::t("app_title")));
    state.brand_label.set_text(&crate::i18n::t("app_name"));
    apply_brand_icon(&state.brand_icon, &state.repo_root);

    // Relocalize chat list previews (cached English → Thai/English).
    for chat in state.chats.borrow_mut().iter_mut() {
        if chat.preview.is_empty() {
            continue;
        }
        chat.preview = localize_preview(&chat.preview);
        if let Some(label) = state.chat_previews.borrow().get(&chat.mid) {
            label.set_text(&chat.preview);
        }
    }
    sync_discord_rpc(state);
}

fn open_uri(url: &str) {
    if let Err(e) = gio::AppInfo::launch_default_for_uri(url, None::<&gio::AppLaunchContext>) {
        eprintln!("open uri failed: {e}");
    }
}

fn youtube_id(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.contains("youtube.com") || lower.contains("youtu.be")) {
        return None;
    }
    if let Some(idx) = url.find("v=") {
        let rest = &url[idx + 2..];
        let id = rest.split('&').next().unwrap_or("");
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    if let Some(idx) = url.find("youtu.be/") {
        let rest = &url[idx + 9..];
        let id = rest
            .split('?')
            .next()
            .unwrap_or("")
            .split('/')
            .next()
            .unwrap_or("");
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    None
}

fn extract_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        let t = token.trim_matches(|c: char| {
            matches!(c, ')' | '(' | ']' | '[' | '.' | ',' | ';' | '"' | '\'')
        });
        if t.starts_with("http://") || t.starts_with("https://") {
            out.push(t.to_string());
        }
    }
    out
}

/// Drag-and-drop files/images onto the conversation (chat + composer).
fn wire_drop_attachments(state: &AppState) {
    let drop = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
    drop.set_types(&[
        gdk::FileList::static_type(),
        gio::File::static_type(),
        gdk::Texture::static_type(),
    ]);
    let s = state.clone();
    drop.connect_drop(move |_, value, _x, _y| {
        if let Ok(list) = value.get::<gdk::FileList>() {
            let mut any = false;
            for file in list.files() {
                if let Some(path) = file.path()
                    && path.is_file()
                {
                    send_local_media_path(&s, path);
                    any = true;
                }
            }
            return any;
        }
        if let Ok(file) = value.get::<gio::File>() {
            if let Some(path) = file.path()
                && path.is_file()
            {
                send_local_media_path(&s, path);
                return true;
            }
            return false;
        }
        if let Ok(tex) = value.get::<gdk::Texture>() {
            match save_clipboard_texture_png(&s, &tex) {
                Ok(path) => {
                    send_local_media_path(&s, path);
                    true
                }
                Err(e) => {
                    toast(
                        &s,
                        &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
                    );
                    false
                }
            }
        } else {
            false
        }
    });
    state.conversation.add_controller(drop);
}

fn wire_actions(state: &AppState) {
    let s = state.clone();
    state.composer.connect_activate(move |_| send_current(&s));

    let s = state.clone();
    state.send_btn.connect_clicked(move |_| send_current(&s));

    // Ctrl+V / Shift+Insert: attach clipboard files or images (text paste stays default).
    let s = state.clone();
    let paste = gtk::EventControllerKey::new();
    paste.set_propagation_phase(gtk::PropagationPhase::Capture);
    paste.connect_key_pressed(move |_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let is_paste = (ctrl && (key == gdk::Key::v || key == gdk::Key::V))
            || (shift && key == gdk::Key::Insert);
        if !is_paste {
            return glib::Propagation::Proceed;
        }
        if try_paste_clipboard_attachment(&s) {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    state.composer.add_controller(paste);

    wire_drop_attachments(state);

    // Esc in composer / chat dismisses the "New" separator.
    let s = state.clone();
    let esc = gtk::EventControllerKey::new();
    esc.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            dismiss_new_marker(&s);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    state.composer.add_controller(esc);

    let s = state.clone();
    let esc_win = gtk::EventControllerKey::new();
    esc_win.set_propagation_phase(gtk::PropagationPhase::Capture);
    esc_win.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape && state_chat_open(&s) {
            dismiss_new_marker(&s);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    state.window.add_controller(esc_win);

    let s = state.clone();
    state.jump_banner_btn.connect_clicked(move |_| {
        jump_to_latest(&s);
    });

    let s = state.clone();
    state.chat_list.connect_row_activated(move |_list, row| {
        let idx = row.index() as usize;
        let chats = s.chats.borrow();
        let Some(chat) = chats.get(idx).cloned() else {
            return;
        };
        drop(chats);
        open_chat(&s, &chat);
    });

    let s = state.clone();
    state.login.retry_btn.connect_clicked(move |_| {
        restart_qr_login(&s);
    });

    let s = state.clone();
    state.settings_btn.connect_clicked(move |_| {
        let deps = settings::SettingsDeps {
            sidecar: s.sidecar.clone(),
            data_dir: s.data_dir.clone(),
            config: s.config.clone(),
            on_logout: {
                let s2 = s.clone();
                Rc::new(move || {
                    *s2.session_ready.borrow_mut() = false;
                    s2.pending.borrow_mut().clear();
                    *s2.current_chat.borrow_mut() = None;
                    *s2.discord_session_start.borrow_mut() = None;
                    clear_list(&s2.chat_list);
                    clear_messages(&s2);
                    s2.chats.borrow_mut().clear();
                    login::show_qr_stage(&s2.login);
                    s2.stack.set_visible_child_name("login");
                    s2.login.hint.set_text(&crate::i18n::t("logged_out_hint"));
                    sync_discord_rpc(&s2);
                    // Kill the old Deno process so listen/hydrate cannot keep writing.
                    *s2.restarting.borrow_mut() = true;
                    if let Err(e) = s2.sidecar.restart() {
                        *s2.restarting.borrow_mut() = false;
                        toast(
                            &s2,
                            &crate::i18n::tf("restart_failed_err", &[("error", &e.to_string())]),
                        );
                    }
                    // Fresh sidecar emits Ready → QR login (see ProtocolEvent::Ready).
                })
            },
            on_lang: {
                let s2 = s.clone();
                Rc::new(move || apply_ui_language(&s2))
            },
            on_animations: {
                let s2 = s.clone();
                Rc::new(move |_on: bool| apply_ui_motion(&s2))
            },
            on_experimental_calls: {
                let s2 = s.clone();
                Rc::new(move |_on: bool| {
                    refresh_call_controls(&s2);
                    // Experimental call mode binds to session/device path: force re-login.
                    *s2.session_ready.borrow_mut() = false;
                    s2.pending.borrow_mut().clear();
                    *s2.current_chat.borrow_mut() = None;
                    clear_list(&s2.chat_list);
                    clear_messages(&s2);
                    s2.chats.borrow_mut().clear();
                    close_active_call_ui(&s2);
                    *s2.active_call_peer.borrow_mut() = None;
                    *s2.incoming_call_from.borrow_mut() = None;
                    login::show_qr_stage(&s2.login);
                    s2.stack.set_visible_child_name("login");
                    s2.login
                        .hint
                        .set_text(&crate::i18n::t("exp_calls_relogin_hint"));
                    *s2.restarting.borrow_mut() = true;
                    if let Err(e) = s2.sidecar.restart() {
                        *s2.restarting.borrow_mut() = false;
                        toast(
                            &s2,
                            &crate::i18n::tf("restart_failed_err", &[("error", &e.to_string())]),
                        );
                    }
                })
            },
            on_tray_settings: {
                let s2 = s.clone();
                Rc::new(move || {
                    sync_tray(&s2);
                    apply_close_behavior(&s2);
                })
            },
            on_discord_rpc: {
                let s2 = s.clone();
                Rc::new(move || {
                    sync_discord_rpc(&s2);
                })
            },
            toast: {
                let s2 = s.clone();
                Rc::new(move |msg: &str| toast(&s2, msg))
            },
        };
        settings::open_settings(&s.window, deps);
    });

    let s = state.clone();
    state.friends_btn.connect_clicked(move |_| {
        open_friends_popup(&s);
    });

    let s = state.clone();
    state.search_entry.connect_search_changed(move |entry| {
        filter_chats(&s, &entry.text());
    });

    let s = state.clone();
    state
        .compact_search_entry
        .connect_search_changed(move |entry| {
            filter_chats(&s, &entry.text());
        });

    let s = state.clone();
    state.mic_btn.connect_clicked(move |_| {
        start_voice_record(&s);
    });

    let s = state.clone();
    state.record_send_btn.connect_clicked(move |_| {
        finish_voice_record(&s, true);
    });

    let s = state.clone();
    state.record_cancel_btn.connect_clicked(move |_| {
        finish_voice_record(&s, false);
    });

    wire_record_wave(state);

    let s = state.clone();
    state.attach_btn.connect_clicked(move |_| {
        pick_and_send_media(&s);
    });

    let s = state.clone();
    state.sticker_btn.connect_clicked(move |_| {
        open_sticker_picker(&s);
    });
    // Drop decoded sticker thumbs when the picker closes.
    {
        let pop = state.sticker_popover.clone();
        pop.connect_closed(move |p| {
            p.set_child(None::<&gtk::Widget>);
        });
    }

    let s = state.clone();
    state.call_btn.connect_clicked(move |_| {
        start_voice_call(&s);
    });
    let s = state.clone();
    state.mute_btn.connect_clicked(move |_| {
        toggle_chat_mute(&s);
    });
}

fn open_friends_popup(state: &AppState) {
    if let Some(ui) = state.friends_ui.borrow().as_ref() {
        // Re-present only if the window is still alive.
        if ui.window.is_visible() || ui.window.is_mapped() {
            if ui.friends.borrow().is_empty() {
                friends::set_friends_loading(ui);
            }
            ui.window.present();
            request_friends_list(state);
            return;
        }
    }
    // Stale handle (closed without clearing) — drop and recreate.
    *state.friends_ui.borrow_mut() = None;

    let s = state.clone();
    let ui = friends::open_friends(
        &state.window,
        friends::FriendsDeps {
            sidecar: state.sidecar.clone(),
            toast: {
                let s2 = state.clone();
                Rc::new(move |msg: &str| toast(&s2, msg))
            },
            on_open: {
                let s2 = state.clone();
                Rc::new(move |friend: ChatInfo| {
                    // Prefer richer chat row data if we already have it.
                    let chat = s2
                        .chats
                        .borrow()
                        .iter()
                        .find(|c| c.mid == friend.mid)
                        .cloned()
                        .unwrap_or(friend);
                    open_chat(&s2, &chat);
                })
            },
            request_list: {
                let s2 = state.clone();
                Rc::new(move || request_friends_list(&s2))
            },
        },
    );
    {
        let s2 = s.clone();
        ui.window.connect_destroy(move |_| {
            *s2.friends_ui.borrow_mut() = None;
        });
    }
    *state.friends_ui.borrow_mut() = Some(ui);
    request_friends_list(state);
}

fn request_friends_list(state: &AppState) {
    let show_loading = state
        .friends_ui
        .borrow()
        .as_ref()
        .map(|ui| ui.friends.borrow().is_empty())
        .unwrap_or(true);
    if show_loading && let Some(ui) = state.friends_ui.borrow().as_ref() {
        friends::set_friends_loading(ui);
    }
    match state.sidecar.list_friends() {
        Ok(id) => {
            state.pending.borrow_mut().insert(id, Pending::ListFriends);
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("friend_failed", &[("error", &e.to_string())]),
        ),
    }
}

fn open_chat(state: &AppState, chat: &ChatInfo) {
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return;
    }

    let already_open = state.current_chat.borrow().as_deref() == Some(chat.mid.as_str());
    if already_open {
        // Tap again while already open: mark read without reloading the thread.
        mark_chat_read(state, &chat.mid);
        clear_unread_badge(state, &chat.mid);
        return;
    }

    // Also select when user opens a chat manually.
    select_sidebar_chat(state, Some(chat.mid.as_str()));
    state.current_chat.borrow_mut().replace(chat.mid.clone());
    hide_upload_progress(state);
    {
        let mut cfg = state.config.borrow_mut();
        if cfg.last_chat_mid != chat.mid {
            cfg.last_chat_mid = chat.mid.clone();
            cfg.save(&state.data_dir);
        }
    }
    *state.stick_bottom.borrow_mut() = true;
    state.chat_title.set_text(&chat.name);
    let subtitle = if chat.unread > 0 {
        crate::i18n::tf("unread", &[("n", &chat.unread.to_string())])
    } else {
        crate::i18n::t("conversation")
    };
    state.chat_subtitle.set_text(&subtitle);
    state
        .status
        .set_text(&crate::i18n::tf("chat_prefix", &[("name", &chat.name)]));
    // Voice calls are 1:1 user chats only (bots/groups unsupported).
    refresh_call_controls(state);
    let can_mute = chat.mid.starts_with('u');
    state.mute_btn.set_sensitive(can_mute);
    update_mute_btn(state, chat.muted);
    state.media_queue.borrow_mut().clear();
    set_msg_state(state, "loading", None);
    clear_messages(state);
    *state.msg_list_fp.borrow_mut() = None;
    match state.sidecar.fetch_messages(&chat.mid, 40) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::FetchMessages {
                    chat_mid: chat.mid.clone(),
                },
            );
        }
        Err(e) => {
            set_msg_state(state, "empty", Some(&format!("Failed: {e}")));
            toast(
                state,
                &crate::i18n::tf("fetch_failed", &[("error", &e.to_string())]),
            );
        }
    }
    sync_discord_rpc(state);
}

fn restart_qr_login(state: &AppState) {
    *state.restarting.borrow_mut() = true;
    state.pending.borrow_mut().clear();
    login::show_qr_stage(&state.login);
    state
        .login
        .hint
        .set_text(&crate::i18n::t("restarting_login"));
    state.stack.set_visible_child_name("login");

    if let Err(e) = state.sidecar.restart() {
        *state.restarting.borrow_mut() = false;
        toast(
            state,
            &crate::i18n::tf("restart_failed_err", &[("error", &e.to_string())]),
        );
    }
}

fn pump_events(state: AppState) {
    let rx = state.sidecar.events.clone();
    glib::spawn_future_local(async move {
        while let Ok(ev) = rx.recv().await {
            handle_event(&state, ev);
            // Drain a short burst so one flood cannot stall forever on await.
            for _ in 0..64 {
                match rx.try_recv() {
                    Ok(ev) => handle_event(&state, ev),
                    Err(_) => break,
                }
            }
        }
    });
}

fn handle_event(state: &AppState, ev: ProtocolEvent) {
    match ev {
        ProtocolEvent::Ready { has_auth } => {
            let restarting = *state.restarting.borrow();
            if restarting {
                *state.restarting.borrow_mut() = false;
            }
            if has_auth && !restarting {
                // Sidecar is already restoring; keep shell visible (no QR flash).
                show_shell_restoring(state);
            } else if restarting || !has_auth {
                show_login(state, &crate::i18n::t("login_qr_hint"));
                if let Ok(id) = state.sidecar.login_qr() {
                    state.pending.borrow_mut().insert(id, Pending::Login);
                }
            }
        }
        ProtocolEvent::Session {
            mid,
            display_name,
            status_message,
            picture_path,
            avatar_path,
            picture_url,
        } => {
            *state.recovery_attempts.borrow_mut() = 0;
            let result = serde_json::json!({
                "mid": mid,
                "displayName": display_name,
                "statusMessage": status_message,
                "picturePath": picture_path,
                "avatarPath": avatar_path,
                "pictureUrl": picture_url,
            });
            on_logged_in(state, &result);
        }
        ProtocolEvent::SessionFailed { error } => {
            *state.session_ready.borrow_mut() = false;
            hide_upload_progress(state);
            let msg = if error.contains("relogin_android_required") {
                crate::i18n::t("relogin_android_required")
            } else if error.contains("NOT_AUTHORIZED_DEVICE")
                || error.contains("V3_TOKEN_CLIENT_LOGGED_OUT")
                || error.contains("LOGGED_OUT")
            {
                crate::i18n::t("logged_out_hint")
            } else {
                error.clone()
            };
            show_login(state, &format!("{msg}\nTap Retry to sign in with QR."));
            state.login.retry_btn.set_visible(true);
            *state.discord_session_start.borrow_mut() = None;
            sync_discord_rpc(state);
            toast(state, &msg);
        }
        ProtocolEvent::Qr { url } => {
            show_login(state, &crate::i18n::t("login_qr_hint"));
            login::show_qr_stage(&state.login);
            if let Err(e) = login::set_qr(&state.login.qr_picture, &url) {
                state.login.hint.set_text(&format!("QR URL: {url}\n({e})"));
            }
        }
        ProtocolEvent::Pin { pin } => {
            show_login(state, &crate::i18n::t("login_pin_waiting"));
            login::show_pin_stage(&state.login, &pin);
        }
        ProtocolEvent::Listening => {
            state.status.set_text(&crate::i18n::t("live"));
            sync_discord_rpc(state);
        }
        ProtocolEvent::CallIncoming {
            call_id,
            from,
            kind,
        } => {
            tracing::info!(%call_id, %from, %kind, "incoming call");
            if !calls_experimental_enabled(state) {
                // Locked: ignore ringing UI (still experimental).
                let _ = state.sidecar.call_decline();
                return;
            }
            let name = peer_display_name(state, &from);
            *state.incoming_call_from.borrow_mut() = Some(from.clone());
            ensure_call_window(
                state,
                &name,
                CallMode::Incoming(&crate::i18n::tf("call_incoming", &[("name", &name)])),
            );
            notify_call_incoming(state, &name);
        }
        ProtocolEvent::CallCanceled {
            call_id,
            from,
            reason,
        } => {
            tracing::info!(%call_id, %from, %reason, "call canceled");
            let name = peer_display_name(state, &from);
            toast(state, &crate::i18n::tf("call_canceled", &[("name", &name)]));
            let was_incoming = state.incoming_call_from.borrow().as_deref() == Some(from.as_str());
            let was_active = state.active_call_peer.borrow().as_deref() == Some(from.as_str());
            if was_incoming {
                *state.incoming_call_from.borrow_mut() = None;
            }
            if was_incoming || was_active {
                close_active_call_ui(state);
            }
        }
        ProtocolEvent::CallState {
            call_id,
            peer,
            state: call_state,
            error,
        } => {
            tracing::debug!(%call_id, %peer, state = %call_state, ?error, "call state changed");
            handle_call_state(state, &peer, &call_state, error.as_deref());
        }
        ProtocolEvent::Message(msg) => {
            let peer = if msg.mine {
                msg.to.clone()
            } else {
                msg.from.clone()
            };
            let mut preview = format!(
                "{}: {}",
                if msg.mine {
                    crate::i18n::t("you")
                } else {
                    crate::i18n::t("they")
                },
                preview_body_ui(&msg)
            );
            preview = localize_preview(&preview);

            // New friend / first message: create the sidebar row immediately.
            ensure_chat_visible(
                state,
                &ChatInfo {
                    mid: peer.clone(),
                    name: peer_display_name(state, &peer),
                    kind: if peer.starts_with('c') {
                        "group".into()
                    } else {
                        "dm".into()
                    },
                    avatar_path: None,
                    last_activity: msg.created_time,
                    unread: 0,
                    preview: preview.clone(),
                    muted: false,
                },
            );

            if let Some(label) = state.chat_previews.borrow().get(&peer) {
                label.set_text(&preview);
            }
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == peer) {
                chat.preview = preview.clone();
                if msg.created_time > chat.last_activity {
                    chat.last_activity = msg.created_time;
                }
            }
            promote_chat_to_top(state, &peer);

            let current = state.current_chat.borrow().clone();
            let viewing = current.as_deref() == Some(peer.as_str());
            let window_focused = state.window.is_active();
            if viewing {
                if state.msg_stack.visible_child_name().as_deref() != Some("list") {
                    set_msg_state(state, "list", None);
                }
                // Capture before append — layout growth can briefly look "scrolled up".
                let was_stuck = *state.stick_bottom.borrow();
                // "New" marker only when the user is scrolled up (LINE-like).
                if !msg.mine && !was_stuck {
                    ensure_new_message_separator(state);
                }
                append_message(state, &msg, true);
                if was_stuck {
                    pin_messages_to_latest(state);
                    if !msg.mine {
                        // Only suppress alerts when this chat is open AND focused.
                        if window_focused {
                            mark_chat_read(state, &peer);
                            clear_unread_badge(state, &peer);
                        } else {
                            bump_unread(state, &peer);
                            notify_incoming(state, &msg, &peer);
                        }
                    }
                } else if !msg.mine {
                    let n = state.pending_new_below.borrow().saturating_add(1);
                    *state.pending_new_below.borrow_mut() = n;
                    update_jump_banner(state);
                    bump_unread(state, &peer);
                    notify_incoming(state, &msg, &peer);
                }
            } else if !msg.mine {
                bump_unread(state, &peer);
                notify_incoming(state, &msg, &peer);
            }
        }
        ProtocolEvent::ChatUpsert { chat } => {
            let mut chat = chat;
            if !chat.preview.is_empty() {
                chat.preview = localize_preview(&chat.preview);
            }
            upsert_chat_row(state, chat);
        }
        ProtocolEvent::ReadReceipt {
            chat_mid,
            message_id,
        } => {
            apply_peer_read(state, &chat_mid, &message_id);
        }
        ProtocolEvent::Chats { chats, cached } => {
            apply_chats(state, chats, cached);
        }
        ProtocolEvent::Messages { chat_mid, messages } => {
            if state.current_chat.borrow().as_deref() == Some(chat_mid.as_str()) {
                apply_messages(state, messages);
            }
        }
        ProtocolEvent::AvatarReady { mid, avatar_path } => {
            if state.self_mid.borrow().as_deref() == Some(mid.as_str()) {
                *state.self_avatar_path.borrow_mut() = Some(avatar_path.clone());
                if std::path::Path::new(&avatar_path).exists() {
                    attach_texture_async(state.profile_avatar.clone(), avatar_path.clone(), 64);
                }
                sync_discord_rpc(state);
            }
            if let Some(img) = state.chat_avatars.borrow().get(&mid).cloned()
                && std::path::Path::new(&avatar_path).exists()
            {
                attach_texture_async(img, avatar_path.clone(), 72);
            }
            if let Some(ui) = state.friends_ui.borrow().as_ref() {
                if let Some(img) = ui.avatars.borrow().get(&mid).cloned()
                    && std::path::Path::new(&avatar_path).exists()
                {
                    attach_texture_async(img, avatar_path.clone(), 80);
                }
                if let Some(f) = ui.friends.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                    f.avatar_path = Some(avatar_path.clone());
                }
            }
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                chat.avatar_path = Some(avatar_path);
            }
        }
        ProtocolEvent::FriendsUpdated { friends } => {
            if let Some(ui) = state.friends_ui.borrow().as_ref() {
                friends::apply_friends(ui, friends);
            }
        }
        ProtocolEvent::ChatPreview {
            mid,
            preview,
            last_activity,
        } => {
            let preview = localize_preview(&preview);
            if let Some(label) = state.chat_previews.borrow().get(&mid) {
                label.set_text(&preview);
            }
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                chat.preview = preview;
                if last_activity > chat.last_activity {
                    chat.last_activity = last_activity;
                }
            }
        }
        ProtocolEvent::ChatMute { mid, muted } => {
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                chat.muted = muted;
            }
            if state.current_chat.borrow().as_deref() == Some(mid.as_str()) {
                update_mute_btn(state, muted);
            }
            toast(
                state,
                &if muted {
                    crate::i18n::t("chat_muted")
                } else {
                    crate::i18n::t("chat_unmuted")
                },
            );
        }
        ProtocolEvent::MediaReady {
            chat_mid,
            message_id,
            image_path,
            audio_path,
            file_path,
        } => {
            // Always remember hydrated paths so list rebuilds / late bubbles can attach.
            if !image_path.is_empty() && std::path::Path::new(&image_path).exists() {
                state
                    .media_ready_paths
                    .borrow_mut()
                    .insert(message_id.clone(), image_path.clone());
                refresh_notification_media(state, &message_id, &image_path);
            }

            if state.current_chat.borrow().as_deref() != Some(chat_mid.as_str()) {
                return;
            }
            if let Some(meta) = state.media_msgs.borrow_mut().get_mut(&message_id) {
                if !image_path.is_empty() {
                    meta.image_path = Some(image_path.clone());
                }
                if let Some(ap) = audio_path.as_ref() {
                    meta.audio_path = Some(ap.clone());
                }
                if let Some(fp) = file_path.as_ref() {
                    meta.file_path = Some(fp.clone());
                }
            }
            if let Some(path) = audio_path {
                if std::path::Path::new(&path).exists() {
                    attach_audio_to_slot(state, &message_id, &path);
                }
                return;
            }
            if image_path.is_empty() || !std::path::Path::new(&image_path).exists() {
                mark_media_failed(state, &message_id);
                return;
            }
            state
                .media_queue
                .borrow_mut()
                .push_back((message_id, image_path));
            pump_media_queue(state);
        }
        ProtocolEvent::MediaFailed {
            chat_mid,
            message_id,
        } => {
            if state.current_chat.borrow().as_deref() != Some(chat_mid.as_str()) {
                return;
            }
            // A later/parallel hydrate may already have the bytes on disk.
            if let Some(path) = state.media_ready_paths.borrow().get(&message_id).cloned()
                && std::path::Path::new(&path).exists()
            {
                state.media_queue.borrow_mut().push_back((message_id, path));
                pump_media_queue(state);
                return;
            }
            mark_media_failed(state, &message_id);
        }
        ProtocolEvent::Progress {
            scope,
            chat_mid,
            state: prog,
            error,
        } => {
            match scope.as_str() {
                "chats" => match prog.as_str() {
                    "loading" => {
                        state.side_spinner.set_spinning(true);
                        state.side_spinner.set_visible(true);
                        if state.chats.borrow().is_empty() {
                            set_side_state(state, "loading", None);
                        }
                        state.status.set_text(&crate::i18n::t("refreshing_chats"));
                    }
                    "ready" => {
                        state.side_spinner.set_spinning(false);
                        state.side_spinner.set_visible(false);
                        if !state.chats.borrow().is_empty() {
                            set_side_state(state, "list", None);
                        }
                    }
                    "empty" => {
                        state.side_spinner.set_spinning(false);
                        state.side_spinner.set_visible(false);
                        set_side_state(state, "empty", Some(&crate::i18n::t("no_chats")));
                        state.status.set_text(&crate::i18n::t("status_no_chats"));
                    }
                    "error" => {
                        state.side_spinner.set_spinning(false);
                        state.side_spinner.set_visible(false);
                        let msg = error.unwrap_or_else(|| "Failed to load chats".into());
                        set_side_state(state, "empty", Some(&msg));
                        toast(state, &msg);
                    }
                    _ => {}
                },
                "messages" => {
                    let current = state.current_chat.borrow().clone();
                    if chat_mid.as_deref() != current.as_deref() {
                        return;
                    }
                    match prog.as_str() {
                        "loading" => {
                            if state.message_list.first_child().is_none() {
                                set_msg_state(state, "loading", None);
                            } else {
                                state.chat_subtitle.set_text(&crate::i18n::t("refreshing"));
                            }
                        }
                        "ready" => {
                            if state.message_list.first_child().is_some() {
                                set_msg_state(state, "list", None);
                            }
                        }
                        "empty" => {
                            // Don't clobber a thread that already has rows (e.g. optimistic send).
                            if state.message_list.first_child().is_none() {
                                set_msg_state(state, "empty", Some(&crate::i18n::t("no_messages")));
                                state.chat_subtitle.set_text(&crate::i18n::t("empty"));
                            }
                        }
                        "error" => {
                            let msg = error.unwrap_or_else(|| "Failed to load messages".into());
                            set_msg_state(state, "empty", Some(&msg));
                            toast(state, &msg);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        ProtocolEvent::UploadProgress {
            chat_mid,
            progress,
            label,
            done,
        } => {
            if state.current_chat.borrow().as_deref() != Some(chat_mid.as_str()) {
                return;
            }
            if done {
                if progress >= 1.0 {
                    state.upload_bar.set_fraction(1.0);
                    let text = if label.is_empty() {
                        crate::i18n::t("media_upload_sent")
                    } else {
                        label
                    };
                    state.upload_label.set_text(&text);
                    let s = state.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(450),
                        move || {
                            hide_upload_progress(&s);
                        },
                    );
                } else {
                    hide_upload_progress(state);
                }
                return;
            }
            show_upload_progress(state, progress, &label);
        }
        ProtocolEvent::Response {
            id,
            ok,
            result,
            error,
        } => {
            let pending = state.pending.borrow_mut().remove(&id);
            if !ok {
                if matches!(pending, Some(Pending::Login)) && *state.restarting.borrow() {
                    return;
                }
                if matches!(pending, Some(Pending::Login)) {
                    show_login(
                        state,
                        error
                            .as_deref()
                            .unwrap_or("Login failed. Tap Retry to start over."),
                    );
                    login::show_qr_stage(&state.login);
                    state.login.retry_btn.set_visible(true);
                }
                if matches!(pending, Some(Pending::ListChats)) {
                    set_side_state(
                        state,
                        "empty",
                        Some(error.as_deref().unwrap_or("Failed to load chats")),
                    );
                }
                if matches!(pending, Some(Pending::FetchMessages { .. })) {
                    set_msg_state(
                        state,
                        "empty",
                        Some(error.as_deref().unwrap_or("Failed to load messages")),
                    );
                }
                if matches!(pending, Some(Pending::ListFriends))
                    && let Some(ui) = state.friends_ui.borrow().as_ref()
                {
                    ui.stack.set_visible_child_name("empty");
                    ui.empty.set_text(
                        error
                            .as_deref()
                            .unwrap_or(&crate::i18n::t("friends_load_failed")),
                    );
                }
                if let Some(Pending::Send { placeholder_id, .. }) = pending.as_ref() {
                    hide_upload_progress(state);
                    remove_pending_placeholder(state, placeholder_id);
                }
                let err = error.as_deref().unwrap_or("request failed");
                if err.contains("sticker_not_owned") || err.contains("USER_NOT_STICKER_OWNER") {
                    toast(state, &crate::i18n::t("sticker_not_owned"));
                } else {
                    toast(state, err);
                }
                return;
            }
            match pending {
                Some(Pending::Login) => on_logged_in(state, &result),
                Some(Pending::ListChats) => {
                    let chats: Vec<ChatInfo> = result
                        .get("chats")
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    let cached = result
                        .get("cached")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    apply_chats(state, chats, cached);
                }
                Some(Pending::ListFriends) => {
                    let friends: Vec<ChatInfo> = result
                        .get("friends")
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    if let Some(ui) = state.friends_ui.borrow().as_ref() {
                        friends::apply_friends(ui, friends);
                    }
                }
                Some(Pending::FetchMessages { chat_mid }) => {
                    if state.current_chat.borrow().as_deref() == Some(chat_mid.as_str()) {
                        let messages: Vec<MessageInfo> = result
                            .get("messages")
                            .cloned()
                            .and_then(|v| serde_json::from_value(v).ok())
                            .unwrap_or_default();
                        if !same_message_list(state, &chat_mid, &messages) {
                            apply_messages(state, messages);
                        }
                    }
                }
                Some(Pending::Send {
                    placeholder_id,
                    chat_mid,
                }) => {
                    remove_pending_placeholder(state, &placeholder_id);
                    if state.current_chat.borrow().as_deref() != Some(chat_mid.as_str()) {
                        return;
                    }
                    if let Ok(msg) = serde_json::from_value::<MessageInfo>(
                        result.get("message").cloned().unwrap_or_default(),
                    ) {
                        if state.msg_stack.visible_child_name().as_deref() != Some("list") {
                            set_msg_state(state, "list", None);
                        }
                        append_message(state, &msg, true);
                        pin_messages_to_latest(state);
                    }
                }
                Some(Pending::ListStickers) => {
                    fill_sticker_popover(state, &result);
                }
                Some(Pending::DownloadMedia {
                    action,
                    content_type,
                    suggest_name,
                    message_id,
                    ..
                }) => {
                    if let Some(path) = result.get("path").and_then(|v| v.as_str()) {
                        if let Some(meta) = state.media_msgs.borrow_mut().get_mut(&message_id) {
                            meta.file_path = Some(path.to_string());
                            if content_type.eq_ignore_ascii_case("image") {
                                meta.image_path = Some(path.to_string());
                            } else if content_type.eq_ignore_ascii_case("audio") {
                                meta.audio_path = Some(path.to_string());
                            }
                        }
                        finish_media_action(state, &action, path, &suggest_name, &content_type);
                    } else {
                        toast(state, &crate::i18n::t("media_download_failed"));
                    }
                }
                None => {}
            }
        }
        ProtocolEvent::Error(e) => toast(state, &e),
        ProtocolEvent::Exited(code) => {
            if !*state.restarting.borrow() {
                tracing::warn!(code, "protocol engine exited unexpectedly");
                schedule_sidecar_recovery(state);
            }
        }
    }
}

const MAX_SIDECAR_RECOVERY_ATTEMPTS: u8 = 3;

fn schedule_sidecar_recovery(state: &AppState) {
    let attempt = {
        let mut attempts = state.recovery_attempts.borrow_mut();
        if *attempts >= MAX_SIDECAR_RECOVERY_ATTEMPTS {
            let notice = libadwaita::Toast::new(&crate::i18n::t("protocol_recovery_failed"));
            notice.set_button_label(Some(&crate::i18n::t("retry")));
            notice.set_action_name(Some("app.retry-protocol"));
            notice.set_timeout(0);
            state.toast_overlay.add_toast(notice);
            state.status.set_text(&crate::i18n::t("protocol_offline"));
            return;
        }
        *attempts += 1;
        *attempts
    };
    let delay = 1_u64 << (attempt - 1);
    state.status.set_text(&crate::i18n::tf(
        "protocol_reconnecting",
        &[("attempt", &attempt.to_string())],
    ));
    let s = state.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(delay), move || {
        recover_sidecar(&s);
    });
}

fn recover_sidecar(state: &AppState) {
    match state.sidecar.recover() {
        Ok(()) => {
            state
                .status
                .set_text(&crate::i18n::t("protocol_restarting"));
        }
        Err(error) => {
            tracing::error!(%error, "protocol recovery failed");
            schedule_sidecar_recovery(state);
        }
    }
}

fn on_logged_in(state: &AppState, result: &serde_json::Value) {
    match serde_json::from_value::<Profile>(result.clone()) {
        Ok(profile) => {
            *state.session_ready.borrow_mut() = true;
            apply_self_profile(
                state,
                &profile.mid,
                &profile.display_name,
                profile.avatar_path.as_deref(),
                profile
                    .picture_url
                    .as_deref()
                    .or(profile.picture_path.as_deref()),
            );
            state.profile_label.set_tooltip_text(
                (!profile.status_message.is_empty()).then_some(profile.status_message.as_str()),
            );
            state.stack.set_visible_child_name("shell");
            // Soft transitions only after we're past the cold boot.
            if state.config.borrow().animations {
                state
                    .stack
                    .set_transition_type(gtk::StackTransitionType::Crossfade);
                state.stack.set_transition_duration(180);
            } else {
                state
                    .stack
                    .set_transition_type(gtk::StackTransitionType::None);
                state.stack.set_transition_duration(0);
            }
            state.status.set_text(&crate::i18n::t("loading_chats"));
            set_side_state(state, "loading", None);
            set_msg_state(state, "idle", None);
            if let Ok(id) = state.sidecar.list_chats() {
                state.pending.borrow_mut().insert(id, Pending::ListChats);
            }
            // If chats were already painted from a warm cache event, open last chat now.
            maybe_restore_last_chat(state);
            sync_discord_rpc(state);
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("bad_profile", &[("error", &e.to_string())]),
        ),
    }
}

fn saved_auth_exists(data_dir: &std::path::Path) -> bool {
    let path = data_dir.join("auth-token.txt");
    match std::fs::read_to_string(&path) {
        Ok(s) => !s.trim().is_empty(),
        Err(_) => false,
    }
}

fn show_shell_restoring(state: &AppState) {
    state.stack.set_visible_child_name("shell");
    state.status.set_text(&crate::i18n::t("restoring"));
    state.side_spinner.set_spinning(true);
    state.side_spinner.set_visible(true);
    if state.chats.borrow().is_empty() {
        set_side_state(state, "loading", None);
    }
}

fn show_login(state: &AppState, hint: &str) {
    state.stack.set_visible_child_name("login");
    login::show_qr_stage(&state.login);
    state.login.hint.set_text(hint);
    sync_discord_rpc(state);
}

fn apply_chats(state: &AppState, mut chats: Vec<ChatInfo>, cached: bool) {
    clear_list(&state.chat_list);
    state.chat_avatars.borrow_mut().clear();
    state.chat_previews.borrow_mut().clear();
    state.chat_unread_badges.borrow_mut().clear();

    for chat in &mut chats {
        if !chat.preview.is_empty() {
            chat.preview = localize_preview(&chat.preview);
        }
    }

    for chat in &chats {
        let (row, avatar, preview, badge) = build_chat_row(chat, *state.sidebar_compact.borrow());
        state
            .chat_avatars
            .borrow_mut()
            .insert(chat.mid.clone(), avatar);
        state
            .chat_previews
            .borrow_mut()
            .insert(chat.mid.clone(), preview);
        state
            .chat_unread_badges
            .borrow_mut()
            .insert(chat.mid.clone(), badge);
        state.chat_list.append(&row);
    }

    let n = chats.len();
    *state.chats.borrow_mut() = chats;

    if n == 0 {
        set_side_state(state, "empty", Some(&crate::i18n::t("no_chats")));
        state.status.set_text(&crate::i18n::t("status_no_chats"));
    } else {
        set_side_state(state, "list", None);
        let n_str = n.to_string();
        let status = if cached {
            crate::i18n::tf("status_chats_cache", &[("n", &n_str)])
        } else {
            crate::i18n::tf("status_chats_latest", &[("n", &n_str)])
        };
        state.status.set_text(&status);
        // After every rebuild, keep the open chat highlighted in the sidebar.
        select_sidebar_chat(state, state.current_chat.borrow().as_deref());
        maybe_restore_last_chat(state);
    }
    refresh_tray_menu(state);
}

fn select_sidebar_chat(state: &AppState, mid: Option<&str>) {
    let Some(mid) = mid else {
        return;
    };
    let Some(pos) = state.chats.borrow().iter().position(|c| c.mid == mid) else {
        return;
    };
    if let Some(row) = state.chat_list.row_at_index(pos as i32) {
        state.chat_list.select_row(Some(&row));
        // Ensure the selected row is scrolled into view.
        row.grab_focus();
    }
}

/// Ensure a sidebar row exists for this mid (first message from a new friend).
fn ensure_chat_visible(state: &AppState, chat: &ChatInfo) {
    if state.chats.borrow().iter().any(|c| c.mid == chat.mid) {
        return;
    }
    upsert_chat_row(state, chat.clone());
}

fn upsert_chat_row(state: &AppState, chat: ChatInfo) {
    let mid = chat.mid.clone();
    let existed = state.chats.borrow().iter().any(|c| c.mid == mid);

    if existed {
        if let Some(cur) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
            if !chat.name.is_empty() && chat.name != mid {
                cur.name = chat.name.clone();
            }
            if !chat.kind.is_empty() {
                cur.kind = chat.kind.clone();
            }
            if !chat.preview.is_empty() {
                cur.preview = chat.preview.clone();
            }
            if chat.last_activity > cur.last_activity {
                cur.last_activity = chat.last_activity;
            }
            if let Some(path) = chat.avatar_path.clone() {
                cur.avatar_path = Some(path);
            }
        }
        if let Some(label) = state.chat_previews.borrow().get(&mid)
            && !chat.preview.is_empty()
        {
            label.set_text(&chat.preview);
        }
        // Refresh visible name on the row: rebuild is heavy; update via child labels if needed.
        // Promote to top so new activity is obvious.
        promote_chat_to_top(state, &mid);
        // Update name label on the row (first heading label in the row).
        if let Some(pos) = state.chats.borrow().iter().position(|c| c.mid == mid)
            && let Some(row) = state.chat_list.row_at_index(pos as i32)
        {
            update_chat_row_name(&row, &chat.name);
            if let Some(path) = chat.avatar_path.as_deref()
                && std::path::Path::new(path).exists()
                && let Some(img) = state.chat_avatars.borrow().get(&mid).cloned()
            {
                attach_texture_async(img, path.to_string(), 72);
            }
        }
        return;
    }

    let (row, avatar, preview, badge) = build_chat_row(&chat, *state.sidebar_compact.borrow());
    state.chat_avatars.borrow_mut().insert(mid.clone(), avatar);
    state
        .chat_previews
        .borrow_mut()
        .insert(mid.clone(), preview);
    state
        .chat_unread_badges
        .borrow_mut()
        .insert(mid.clone(), badge);
    state.chats.borrow_mut().insert(0, chat);
    state.chat_list.prepend(&row);
    set_side_state(state, "list", None);
    let n = state.chats.borrow().len();
    state.status.set_text(&crate::i18n::tf(
        "status_chats_live",
        &[("n", &n.to_string())],
    ));
    refresh_tray_menu(state);
}

fn promote_chat_to_top(state: &AppState, mid: &str) {
    let pos = {
        let chats = state.chats.borrow();
        chats.iter().position(|c| c.mid == mid)
    };
    let Some(pos) = pos else { return };
    if pos == 0 {
        return;
    }
    {
        let mut chats = state.chats.borrow_mut();
        let chat = chats.remove(pos);
        chats.insert(0, chat);
    }
    if let Some(row) = state.chat_list.row_at_index(pos as i32) {
        state.chat_list.remove(&row);
        state.chat_list.prepend(&row);
        if state.config.borrow().animations {
            row.add_css_class("line-chat-bump");
            let row_c = row.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(340), move || {
                row_c.remove_css_class("line-chat-bump");
            });
        }
    }
    // Keep selection on the open chat after row reorder.
    select_sidebar_chat(state, state.current_chat.borrow().as_deref());
    refresh_tray_menu(state);
}

fn update_chat_row_name(row: &gtk::ListBoxRow, name: &str) {
    if name.is_empty() {
        return;
    }
    row.set_tooltip_text(Some(name));
    // row > box > overlay + text_col > top > name label
    let Some(outer) = row.child() else { return };
    let Ok(box_) = outer.downcast::<gtk::Box>() else {
        return;
    };
    let mut child = box_.first_child();
    while let Some(w) = child {
        if w.css_classes().iter().any(|c| c == "line-chat-text")
            && let Ok(inner) = w.clone().downcast::<gtk::Box>()
            && let Some(top) = inner.first_child()
            && let Ok(top_box) = top.downcast::<gtk::Box>()
            && let Some(name_w) = top_box.first_child()
            && let Ok(label) = name_w.downcast::<gtk::Label>()
            && label.css_classes().iter().any(|c| c == "line-chat-name")
        {
            label.set_text(name);
            return;
        }
        child = w.next_sibling();
    }
}

fn maybe_restore_last_chat(state: &AppState) {
    if *state.restored_last_chat.borrow() {
        return;
    }
    if state.current_chat.borrow().is_some() {
        *state.restored_last_chat.borrow_mut() = true;
        return;
    }
    if !*state.session_ready.borrow() {
        return;
    }
    let last = state.config.borrow().last_chat_mid.clone();
    if last.is_empty() {
        *state.restored_last_chat.borrow_mut() = true;
        return;
    }
    let chats = state.chats.borrow();
    let Some(chat) = chats.iter().find(|c| c.mid == last).cloned() else {
        return; // keep trying until the mid appears in a later refresh
    };
    drop(chats);
    *state.restored_last_chat.borrow_mut() = true;
    open_chat(state, &chat);
    select_sidebar_chat(state, Some(chat.mid.as_str()));
}

fn build_chat_row(
    chat: &ChatInfo,
    compact: bool,
) -> (gtk::ListBoxRow, gtk::Picture, gtk::Label, gtk::Label) {
    let row = gtk::ListBoxRow::new();
    row.set_tooltip_text(Some(&chat.name));
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(if compact { 0 } else { 12 })
        .margin_start(if compact { 0 } else { 8 })
        .margin_end(if compact { 0 } else { 8 })
        .margin_top(8)
        .margin_bottom(8)
        .hexpand(!compact)
        .halign(if compact {
            gtk::Align::Center
        } else {
            gtk::Align::Fill
        })
        .css_classes(["line-chat-row"])
        .build();

    let avatar_px = if compact {
        AVATAR_COMPACT_PX
    } else {
        AVATAR_CHAT_PX
    };
    let avatar_frame = gtk::Box::builder()
        .width_request(avatar_px)
        .height_request(avatar_px)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-avatar-frame"])
        .build();
    let avatar = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(avatar_px)
        .height_request(avatar_px)
        .css_classes(["line-avatar"])
        .build();
    if let Some(path) = chat.avatar_path.as_deref()
        && std::path::Path::new(path).exists()
    {
        attach_texture_async(avatar.clone(), path.to_string(), avatar_px * 2);
    }
    avatar_frame.append(&avatar);

    let avatar_overlay = gtk::Overlay::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-avatar-overlay"])
        .build();
    avatar_overlay.set_child(Some(&avatar_frame));

    let badge = gtk::Label::builder()
        .label(if chat.unread > 0 {
            chat.unread.to_string()
        } else {
            String::new()
        })
        .css_classes(["line-unread", "line-unread-overlay"])
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .visible(chat.unread > 0)
        .build();
    avatar_overlay.add_overlay(&badge);

    let text_col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .visible(!compact)
        .css_classes(["line-chat-text"])
        .build();

    let top = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let name = gtk::Label::builder()
        .label(&chat.name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["line-chat-name"])
        .build();
    let time = gtk::Label::builder()
        .label(format_activity(chat.last_activity))
        .css_classes(["dim-label", "caption"])
        .build();
    top.append(&name);
    top.append(&time);

    let bottom = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let preview_text = if chat.preview.is_empty() {
        if chat.last_activity > 0 {
            crate::i18n::t("loading_last")
        } else {
            crate::i18n::t("no_recent")
        }
    } else {
        localize_preview(&chat.preview)
    };
    let preview = gtk::Label::builder()
        .label(&preview_text)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["line-chat-preview"])
        .build();
    bottom.append(&preview);

    text_col.append(&top);
    text_col.append(&bottom);
    box_.append(&avatar_overlay);
    box_.append(&text_col);
    row.set_child(Some(&box_));
    (row, avatar, preview, badge)
}

fn message_list_fingerprint(messages: &[MessageInfo]) -> (usize, String, String) {
    let first = messages.first().map(|m| m.id.clone()).unwrap_or_default();
    let last = messages.last().map(|m| m.id.clone()).unwrap_or_default();
    (messages.len(), first, last)
}

fn same_message_list(state: &AppState, chat_mid: &str, messages: &[MessageInfo]) -> bool {
    let (len, first, last) = message_list_fingerprint(messages);
    matches!(
        state.msg_list_fp.borrow().as_ref(),
        Some((mid, l, f, la)) if mid == chat_mid && *l == len && f == &first && la == &last
    )
}

fn apply_messages(state: &AppState, mut messages: Vec<MessageInfo>) {
    clear_messages(state);
    state.media_queue.borrow_mut().clear();
    *state.stick_bottom.borrow_mut() = true;
    messages.sort_by_key(|m| m.created_time);
    if let Some(mid) = state.current_chat.borrow().clone() {
        let (len, first, last) = message_list_fingerprint(&messages);
        *state.msg_list_fp.borrow_mut() = Some((mid, len, first, last));
    } else {
        *state.msg_list_fp.borrow_mut() = None;
    }
    if messages.is_empty() {
        set_msg_state(state, "empty", Some(&crate::i18n::t("no_messages")));
        state.chat_subtitle.set_text(&crate::i18n::t("empty"));
        return;
    }

    // Append in idle chunks so image-heavy threads don't freeze the window.
    let total = messages.len();
    set_msg_state(state, "list", None);
    state.chat_subtitle.set_text(&crate::i18n::tf(
        "messages_count",
        &[("n", &total.to_string())],
    ));

    let last_incoming = messages
        .iter()
        .rev()
        .find(|m| !m.mine)
        .map(|m| m.id.clone());
    if let Some(id) = last_incoming.clone() {
        *state.last_incoming_id.borrow_mut() = Some(id);
    }

    let chat_mid = state.current_chat.borrow().clone();
    let state_c = state.clone();
    let batch = Rc::new(RefCell::new(messages));
    glib::idle_add_local(move || {
        let mut left = batch.borrow_mut();
        let take = left.len().min(14);
        if take == 0 {
            pin_messages_to_latest(&state_c);
            if let Some(mid) = chat_mid.as_deref() {
                mark_chat_read(&state_c, mid);
                clear_unread_badge(&state_c, mid);
            }
            return glib::ControlFlow::Break;
        }
        let chunk: Vec<_> = left.drain(0..take).collect();
        drop(left);
        for msg in &chunk {
            append_message(&state_c, msg, false);
        }
        // Stay on the latest message while rows are still being inserted.
        scroll_messages_to_end(&state_c);
        glib::ControlFlow::Continue
    });
}

fn append_message(state: &AppState, msg: &MessageInfo, live: bool) {
    if !msg.id.is_empty() && !state.seen_msg_ids.borrow_mut().insert(msg.id.clone()) {
        return;
    }

    // Prefer a path that already finished hydrating (race with list rebuild).
    let ready_path = {
        let missing = msg
            .image_path
            .as_deref()
            .map(|p| !std::path::Path::new(p).exists())
            .unwrap_or(true);
        if missing {
            state
                .media_ready_paths
                .borrow()
                .get(&msg.id)
                .filter(|p| std::path::Path::new(p).exists())
                .cloned()
        } else {
            None
        }
    };
    let msg_owned;
    let msg = if let Some(path) = ready_path {
        msg_owned = MessageInfo {
            image_path: Some(path),
            ..msg.clone()
        };
        &msg_owned
    } else {
        msg
    };

    let list = &state.message_list;

    // Date separator like mobile LINE (Today / Yesterday / Fri, Jul 24).
    if msg.created_time > 0 {
        let day = day_key(msg.created_time);
        let need_sep = state.last_msg_day.borrow().as_deref() != Some(day.as_str());
        if need_sep {
            *state.last_msg_day.borrow_mut() = Some(day);
            let sep_row = gtk::ListBoxRow::builder()
                .selectable(false)
                .activatable(false)
                .css_classes(["line-day-sep-row"])
                .build();
            let sep = gtk::Label::builder()
                .label(format_day_separator(msg.created_time))
                .halign(gtk::Align::Center)
                .css_classes(["line-day-sep"])
                .build();
            sep_row.set_child(Some(&sep));
            list.append(&sep_row);
        }
    }

    let row = gtk::ListBoxRow::new();
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .valign(gtk::Align::End)
        .build();

    let is_sticker = msg.content_type.eq_ignore_ascii_case("sticker");
    let is_flex = msg.content_type.eq_ignore_ascii_case("flex") || msg.flex.is_some();
    let is_pending = msg.id.starts_with("pending-");
    let bubble = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(if msg.mine {
            gtk::Align::End
        } else {
            gtk::Align::Start
        })
        .css_classes(if is_sticker {
            vec!["line-bubble", "line-bubble-sticker"]
        } else if is_flex {
            vec!["line-bubble", "line-bubble-in"]
        } else {
            vec![
                "line-bubble",
                if msg.mine {
                    "line-bubble-out"
                } else {
                    "line-bubble-in"
                },
            ]
        })
        .build();
    if is_pending {
        bubble.add_css_class("line-bubble-pending");
        outer.add_css_class("line-msg-pending");
    }

    let has_image = msg
        .image_path
        .as_deref()
        .map(|p| std::path::Path::new(p).exists())
        .unwrap_or(false);
    let waiting_visual = (msg.content_type.eq_ignore_ascii_case("image")
        || msg.content_type.eq_ignore_ascii_case("sticker")
        || msg.content_type.eq_ignore_ascii_case("video"))
        && !has_image;
    let is_audio = msg.content_type.eq_ignore_ascii_case("audio");
    let is_file = msg.content_type.eq_ignore_ascii_case("file");
    let is_video = msg.content_type.eq_ignore_ascii_case("video");
    let is_image = msg.content_type.eq_ignore_ascii_case("image");
    let has_audio = msg
        .audio_path
        .as_deref()
        .map(|p| std::path::Path::new(p).exists())
        .unwrap_or(false);

    if let Some(flex) = msg.flex.as_ref() {
        append_flex_card(state, &bubble, msg, flex);
    } else if is_audio {
        append_voice_card(state, &bubble, msg, has_audio);
    } else if is_file {
        append_file_card(state, &bubble, msg);
    } else {
        let show_text = !msg.text.is_empty()
            && msg.text != "[Image]"
            && msg.text != "[Video]"
            && msg.text != "[Sticker]"
            && !is_sticker;

        if show_text {
            let label = gtk::Label::builder()
                .label(&msg.text)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .selectable(true)
                .max_width_chars(42)
                .css_classes(["line-bubble-text"])
                .build();
            bubble.append(&label);
            append_link_chips(state, &bubble, &msg.text);
        } else if !has_image && !waiting_visual {
            let fallback = if msg.content_type.is_empty() {
                "(message)".to_string()
            } else {
                format!("({})", msg.content_type.to_lowercase())
            };
            let label = gtk::Label::builder()
                .label(&fallback)
                .xalign(0.0)
                .css_classes(["dim-label"])
                .build();
            bubble.append(&label);
        }
    }

    if has_image {
        if let Some(path) = msg.image_path.as_deref() {
            let pic = make_media_picture_placeholder(is_sticker);
            let animate = is_sticker && state.config.borrow().animations;
            attach_texture_async_anim(
                pic.clone(),
                path.to_string(),
                if is_sticker { 128 } else { 320 },
                animate,
            );
            if !is_sticker && (is_image || is_video) {
                if is_video {
                    let overlay = wrap_video_thumb(state, &pic, msg);
                    bubble.append(&overlay);
                } else {
                    wire_media_open_click(state, &pic, msg, "image");
                    pic.set_tooltip_text(Some(&crate::i18n::t("media_open_image")));
                    bubble.append(&pic);
                }
            } else {
                bubble.append(&pic);
            }
        }
    } else if waiting_visual {
        if is_video {
            append_video_placeholder(state, &bubble, msg, false);
        } else {
            let placeholder = gtk::Label::builder()
                .label(if is_sticker {
                    crate::i18n::t("loading_sticker")
                } else {
                    crate::i18n::t("loading_image")
                })
                .xalign(0.0)
                .css_classes(["dim-label", "line-media-placeholder"])
                .build();
            bubble.append(&placeholder);
        }
    }

    if !msg.id.is_empty() && message_tracks_media(msg) {
        state
            .media_slots
            .borrow_mut()
            .insert(msg.id.clone(), bubble.clone());
        state
            .media_msgs
            .borrow_mut()
            .insert(msg.id.clone(), msg.clone());
    }

    if !msg.mine && !msg.id.is_empty() {
        *state.last_incoming_id.borrow_mut() = Some(msg.id.clone());
    }
    if !msg.id.is_empty() && msg.created_time > 0 {
        state
            .msg_created
            .borrow_mut()
            .insert(msg.id.clone(), msg.created_time);
    }

    let time_txt = format_msg_time(msg.created_time);

    if msg.mine {
        let col = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .halign(gtk::Align::End)
            .hexpand(true)
            .build();
        col.append(&bubble);

        let read = state
            .current_chat
            .borrow()
            .as_ref()
            .and_then(|mid| state.read_upto.borrow().get(mid).cloned())
            .map(|upto| msg_id_le(&msg.id, &upto))
            .unwrap_or(false);
        let status = gtk::Label::builder()
            .label(format_outgoing_status(read, msg.created_time))
            .halign(gtk::Align::End)
            .tooltip_text(if read {
                crate::i18n::t("status_read")
            } else {
                crate::i18n::t("status_sent")
            })
            .css_classes(if read {
                vec!["line-msg-status", "line-msg-status-read"]
            } else {
                vec!["line-msg-status"]
            })
            .build();
        if !msg.id.is_empty() {
            state
                .receipt_slots
                .borrow_mut()
                .insert(msg.id.clone(), status.clone());
        }
        col.append(&status);
        outer.append(&gtk::Box::builder().hexpand(true).build());
        outer.append(&col);
    } else {
        // Incoming: bubble + time on the right (mobile style).
        let time = gtk::Label::builder()
            .label(&time_txt)
            .valign(gtk::Align::End)
            .css_classes(["line-msg-time"])
            .build();
        outer.append(&bubble);
        outer.append(&time);
        outer.append(&gtk::Box::builder().hexpand(true).build());
    }

    row.set_child(Some(&outer));
    if is_pending {
        row.add_css_class("line-msg-pending-row");
        state
            .pending_rows
            .borrow_mut()
            .insert(msg.id.clone(), row.clone());
    }
    if live && state.config.borrow().animations {
        row.add_css_class("line-msg-enter");
        if msg.mine {
            row.add_css_class("line-msg-out");
        }
        let row_c = row.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(260), move || {
            row_c.remove_css_class("line-msg-enter");
            row_c.remove_css_class("line-msg-out");
        });
    }
    list.append(&row);
}

fn format_outgoing_status(read: bool, created_ms: i64) -> String {
    let t = format_msg_time(created_ms);
    if read {
        // Mobile LINE style: "Read 4:34 PM" (+ ✓✓ cue)
        if t.is_empty() {
            format!("✓✓ {}", crate::i18n::t("status_read"))
        } else {
            format!("✓✓ {} {}", crate::i18n::t("status_read"), t)
        }
    } else if t.is_empty() {
        "✓".into()
    } else {
        format!("✓ {t}")
    }
}

fn day_key(ts_ms: i64) -> String {
    let secs = if ts_ms > 1_000_000_000_000 {
        ts_ms / 1000
    } else {
        ts_ms
    };
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

fn format_day_separator(ts_ms: i64) -> String {
    let secs = if ts_ms > 1_000_000_000_000 {
        ts_ms / 1000
    } else {
        ts_ms
    };
    let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) else {
        return String::new();
    };
    let local = dt.with_timezone(&chrono::Local);
    let today = chrono::Local::now().date_naive();
    let day = local.date_naive();
    if day == today {
        crate::i18n::t("day_today")
    } else if day == today - chrono::Duration::days(1) {
        crate::i18n::t("day_yesterday")
    } else {
        local.format("%a, %b %-d").to_string()
    }
}

fn format_msg_time(ts_ms: i64) -> String {
    if ts_ms <= 0 {
        return String::new();
    }
    let secs = if ts_ms > 1_000_000_000_000 {
        ts_ms / 1000
    } else {
        ts_ms
    };
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%-I:%M %p")
                .to_string()
        })
        .unwrap_or_default()
}

fn append_voice_card(state: &AppState, bubble: &gtk::Box, msg: &MessageInfo, has_audio: bool) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["line-voice-card"])
        .build();
    let play = gtk::Button::builder()
        .icon_name(if has_audio {
            "media-playback-start-symbolic"
        } else {
            "content-loading-symbolic"
        })
        .sensitive(has_audio)
        .css_classes(["circular", "line-voice-play"])
        .tooltip_text(crate::i18n::t("play_voice"))
        .build();

    let peaks = Rc::new(RefCell::new(placeholder_peaks(36)));
    let progress = Rc::new(RefCell::new(0.0_f32));
    let mine = msg.mine;
    let wave = gtk::DrawingArea::builder()
        .hexpand(true)
        .content_width(160)
        .content_height(32)
        .css_classes(["line-voice-wave"])
        .build();
    {
        let peaks_draw = peaks.clone();
        let progress_draw = progress.clone();
        wave.set_draw_func(move |area, cr, width, height| {
            draw_waveform(
                area,
                cr,
                width as f64,
                height as f64,
                &peaks_draw.borrow(),
                mine,
                Some(*progress_draw.borrow()),
            );
        });
    }

    let duration_ms = if let Some(ms) = msg.duration_ms.filter(|v| *v > 0) {
        ms as u64
    } else if has_audio {
        msg.audio_path
            .as_deref()
            .and_then(|p| ffprobe_duration_ms(std::path::Path::new(p)))
            .unwrap_or(0)
    } else {
        0
    };
    let dur_txt = if duration_ms > 0 {
        format_voice_duration(duration_ms)
    } else if has_audio {
        "--:--".into()
    } else {
        crate::i18n::t("loading_voice")
    };
    let dur = gtk::Label::builder()
        .label(&dur_txt)
        .css_classes(["line-voice-dur"])
        .build();

    if has_audio && let Some(path) = msg.audio_path.clone() {
        let s = state.clone();
        let play_btn = play.clone();
        let wave_btn = wave.clone();
        let dur_btn = dur.clone();
        let progress_btn = progress.clone();
        let msg_id = msg.id.clone();
        let total_label = dur_txt.clone();
        play.connect_clicked(move |_| {
            toggle_voice_playback(VoicePlaybackRequest {
                state: &s,
                msg_id: &msg_id,
                path: &path,
                play_btn: &play_btn,
                wave: &wave_btn,
                duration_label: &dur_btn,
                progress: progress_btn.clone(),
                duration_ms,
                total_label: &total_label,
            });
        });
    }

    let dl = gtk::Button::builder()
        .icon_name("folder-download-symbolic")
        .css_classes(["flat", "circular"])
        .tooltip_text(crate::i18n::t("media_download"))
        .build();
    {
        let s = state.clone();
        let msg = msg.clone();
        dl.connect_clicked(move |_| {
            request_media_download(&s, &msg, "save_dialog");
        });
    }

    row.append(&play);
    row.append(&wave);
    row.append(&dur);
    row.append(&dl);
    bubble.append(&row);

    if has_audio && let Some(path) = msg.audio_path.clone() {
        let wave2 = wave.clone();
        let peaks2 = peaks.clone();
        let (tx, rx) = async_channel::bounded::<Vec<f32>>(1);
        std::thread::spawn(move || {
            let extracted = extract_audio_peaks(std::path::Path::new(&path), 36)
                .unwrap_or_else(|| placeholder_peaks(36));
            let _ = tx.send_blocking(extracted);
        });
        glib::spawn_future_local(async move {
            if let Ok(extracted) = rx.recv().await {
                *peaks2.borrow_mut() = extracted;
                wave2.queue_draw();
            }
        });
    }
}

struct VoicePlaybackRequest<'a> {
    state: &'a AppState,
    msg_id: &'a str,
    path: &'a str,
    play_btn: &'a gtk::Button,
    wave: &'a gtk::DrawingArea,
    duration_label: &'a gtk::Label,
    progress: Rc<RefCell<f32>>,
    duration_ms: u64,
    total_label: &'a str,
}

fn toggle_voice_playback(request: VoicePlaybackRequest<'_>) {
    let VoicePlaybackRequest {
        state,
        msg_id,
        path,
        play_btn,
        wave,
        duration_label,
        progress,
        duration_ms,
        total_label,
    } = request;
    // Second press on the same bubble stops playback.
    // Drop the borrow before stop_voice_playback (it needs borrow_mut).
    let same_bubble = state
        .voice_playback
        .borrow()
        .as_ref()
        .is_some_and(|cur| cur.msg_id == msg_id);
    if same_bubble {
        stop_voice_playback(state);
        return;
    }
    stop_voice_playback(state);

    let child = match spawn_audio_player(path, &state.config.borrow().audio_output, 1.0) {
        Ok(c) => c,
        Err(e) => {
            toast(
                state,
                &crate::i18n::tf("voice_play_failed", &[("error", &e)]),
            );
            return;
        }
    };

    play_btn.set_icon_name("media-playback-stop-symbolic");
    play_btn.set_tooltip_text(Some(&crate::i18n::t("stop_voice")));
    play_btn.add_css_class("playing");
    wave.add_css_class("playing");
    *progress.borrow_mut() = 0.0;
    wave.queue_draw();

    let playback = VoicePlayback {
        child,
        msg_id: msg_id.to_string(),
        play_btn: play_btn.clone(),
        wave: wave.clone(),
        dur: duration_label.clone(),
        duration_ms: duration_ms.max(500),
        started: std::time::Instant::now(),
        progress: progress.clone(),
        total_label: total_label.to_string(),
        tick: None,
    };
    *state.voice_playback.borrow_mut() = Some(playback);

    let s = state.clone();
    let tick = glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let mut slot = s.voice_playback.borrow_mut();
        let Some(pb) = slot.as_mut() else {
            return glib::ControlFlow::Break;
        };
        // Finished?
        match pb.child.try_wait() {
            Ok(Some(_)) => {
                drop(slot);
                stop_voice_playback(&s);
                return glib::ControlFlow::Break;
            }
            Err(_) => {
                drop(slot);
                stop_voice_playback(&s);
                return glib::ControlFlow::Break;
            }
            Ok(None) => {}
        }
        let elapsed = pb.started.elapsed().as_millis() as u64;
        let frac = (elapsed as f32 / pb.duration_ms as f32).clamp(0.0, 1.0);
        *pb.progress.borrow_mut() = frac;
        pb.wave.queue_draw();
        let remain = pb.duration_ms.saturating_sub(elapsed);
        pb.dur.set_text(&format_voice_duration(remain.max(1)));
        if frac >= 1.0 {
            // Duration reached but process still alive — keep waiting for exit.
        }
        glib::ControlFlow::Continue
    });
    if let Some(pb) = state.voice_playback.borrow_mut().as_mut() {
        pb.tick = Some(tick);
    }
}

fn stop_voice_playback(state: &AppState) {
    let Some(mut pb) = state.voice_playback.borrow_mut().take() else {
        return;
    };
    if let Some(tick) = pb.tick.take() {
        tick.remove();
    }
    let _ = pb.child.kill();
    let _ = pb.child.wait();
    *pb.progress.borrow_mut() = 0.0;
    pb.play_btn.set_icon_name("media-playback-start-symbolic");
    pb.play_btn
        .set_tooltip_text(Some(&crate::i18n::t("play_voice")));
    pb.play_btn.remove_css_class("playing");
    pb.wave.remove_css_class("playing");
    pb.wave.queue_draw();
    pb.dur.set_text(&pb.total_label);
}

fn append_file_card(state: &AppState, bubble: &gtk::Box, msg: &MessageInfo) {
    let name = msg
        .file_name
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if msg.text.is_empty() || msg.text == "[File]" {
                crate::i18n::t("preview_file")
            } else {
                msg.text.clone()
            }
        });
    let kind = viewer_kind_for("FILE", "", &name);
    let icon_name = match kind {
        ViewerKind::Pdf => "application-pdf-symbolic",
        ViewerKind::Text => "text-x-generic-symbolic",
        ViewerKind::Image => "image-x-generic-symbolic",
        ViewerKind::Video => "video-x-generic-symbolic",
        ViewerKind::Audio => "audio-x-generic-symbolic",
        ViewerKind::Generic => "folder-download-symbolic",
    };
    let row = gtk::Button::builder()
        .css_classes(["flat", "line-file-card"])
        .tooltip_text(crate::i18n::t("media_open_file"))
        .halign(gtk::Align::Fill)
        .build();
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(22);
    let label = gtk::Label::builder()
        .label(&name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(28)
        .hexpand(true)
        .css_classes(["line-bubble-text"])
        .build();
    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    inner.append(&icon);
    inner.append(&label);
    inner.append(&chevron);
    row.set_child(Some(&inner));
    let s = state.clone();
    let msg = msg.clone();
    row.connect_clicked(move |_| {
        request_media_download(&s, &msg, "open_viewer");
    });
    bubble.append(&row);
}

fn format_voice_duration(ms: u64) -> String {
    let secs = ((ms + 500) / 1000).max(1);
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

fn placeholder_peaks(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            0.18 + 0.12 * ((t * std::f32::consts::PI * 4.0).sin()).abs()
        })
        .collect()
}

fn draw_waveform(
    area: &gtk::DrawingArea,
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    peaks: &[f32],
    mine: bool,
    progress: Option<f32>,
) {
    if width <= 1.0 || height <= 1.0 || peaks.is_empty() {
        return;
    }
    // Named theme colors still require StyleContext::lookup_color (GTK < 4.10 path).
    #[allow(deprecated)]
    let accent = area
        .style_context()
        .lookup_color("accent_bg_color")
        .or_else(|| area.style_context().lookup_color("accent_color"));
    let muted = area.color();
    let (base_r, base_g, base_b) = if mine {
        match accent {
            Some(c) => (
                f64::from(c.red()),
                f64::from(c.green()),
                f64::from(c.blue()),
            ),
            None => (
                f64::from(muted.red()),
                f64::from(muted.green()),
                f64::from(muted.blue()),
            ),
        }
    } else {
        (
            f64::from(muted.red()),
            f64::from(muted.green()),
            f64::from(muted.blue()),
        )
    };
    let n = peaks.len() as f64;
    let gap = 2.0_f64.min((width / n) * 0.35).max(1.0);
    // Spread bars across the full width (do not cap bar width).
    let bar_w = ((width - gap * (n - 1.0)) / n).max(1.0);
    let mid = height * 0.5;
    let max_h = (height * 0.42).max(3.0);
    let prog = progress.unwrap_or(-1.0);
    for (i, p) in peaks.iter().enumerate() {
        let amp = (*p).clamp(0.08, 1.0) as f64;
        let h = max_h * amp;
        let x = i as f64 * (bar_w + gap);
        let t = (i as f32 + 0.5) / peaks.len() as f32;
        let played = prog >= 0.0 && t <= prog;
        let alpha = if played {
            1.0
        } else if prog >= 0.0 {
            0.35
        } else if mine {
            0.95
        } else {
            0.75
        };
        cr.set_source_rgba(base_r, base_g, base_b, alpha);
        cr.rectangle(x, mid - h, bar_w, h * 2.0);
        let _ = cr.fill();
    }
}

/// How many live spectrum bars fit the record-wave width (~3px bar + gap).
fn spectrum_bar_count(width: f64) -> usize {
    let slot = 5.0_f64;
    ((width / slot).round() as usize).clamp(32, 160)
}

fn extract_audio_peaks(path: &std::path::Path, bars: usize) -> Option<Vec<f32>> {
    if bars == 0 || !path.exists() {
        return None;
    }
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            path.to_str()?,
            "-ac",
            "1",
            "-ar",
            "8000",
            "-f",
            "s16le",
            "-",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.len() < 4 {
        return None;
    }
    let pcm = out.stdout;
    let n = pcm.len() / 2;
    if n == 0 {
        return None;
    }
    let mut peaks = vec![0.0f32; bars];
    for i in 0..n {
        let s = i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]) as f32 / 32768.0;
        let bi = (i * bars) / n;
        if bi < bars {
            peaks[bi] = peaks[bi].max(s.abs());
        }
    }
    let max = peaks.iter().cloned().fold(0.05f32, f32::max);
    Some(
        peaks
            .into_iter()
            .map(|p| (p / max).clamp(0.08, 1.0))
            .collect(),
    )
}

fn wav_tail_level(path: &std::path::Path) -> f32 {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return 0.08;
    };
    let Ok(meta) = file.metadata() else {
        return 0.08;
    };
    let len = meta.len() as usize;
    if len < 44 + 512 {
        return 0.08;
    }
    let bytes = (1024 * 2).min(len.saturating_sub(44));
    let start = (len - bytes) as u64;
    if file.seek(SeekFrom::Start(start)).is_err() {
        return 0.08;
    }
    let mut buf = vec![0u8; bytes];
    if file.read_exact(&mut buf).is_err() {
        return 0.08;
    }
    let samples = buf.len() / 2;
    if samples == 0 {
        return 0.08;
    }
    let mut sum = 0.0f64;
    for i in 0..samples {
        let s = i16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]) as f64 / 32768.0;
        sum += s * s;
    }
    let rms = (sum / samples as f64).sqrt() as f32;
    (rms * 4.5).clamp(0.08, 1.0)
}

fn wire_record_wave(state: &AppState) {
    let levels = state.recording_levels.clone();
    state
        .record_wave
        .set_draw_func(move |area, cr, width, height| {
            draw_waveform(
                area,
                cr,
                width as f64,
                height as f64,
                &levels.borrow(),
                true,
                None,
            );
        });

    // Custom DrawingAreas do not auto-repaint when the color scheme flips.
    let root = state.window.clone().upcast::<gtk::Widget>();
    let record_wave = state.record_wave.clone();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        record_wave.queue_draw();
        queue_draw_voice_waves(&root);
    });
}

fn queue_draw_voice_waves(widget: &gtk::Widget) {
    if let Ok(area) = widget.clone().downcast::<gtk::DrawingArea>()
        && (area.has_css_class("line-voice-wave") || area.has_css_class("line-record-wave"))
    {
        area.queue_draw();
    }
    let mut child = widget.first_child();
    while let Some(w) = child {
        queue_draw_voice_waves(&w);
        child = w.next_sibling();
    }
}

fn attach_audio_to_slot(state: &AppState, message_id: &str, path: &str) {
    let Some(bubble) = state.media_slots.borrow().get(message_id).cloned() else {
        return;
    };
    while let Some(child) = bubble.first_child() {
        bubble.remove(&child);
    }
    let duration_ms = ffprobe_duration_ms(std::path::Path::new(path)).map(|v| v as i64);
    let msg = MessageInfo {
        id: message_id.to_string(),
        text: "Voice message".into(),
        from: String::new(),
        to: String::new(),
        mine: true,
        created_time: 0,
        content_type: "AUDIO".into(),
        image_path: None,
        audio_path: Some(path.to_string()),
        file_name: None,
        file_path: Some(path.to_string()),
        duration_ms,
        flex: None,
    };
    append_voice_card(state, &bubble, &msg, true);
}

fn play_audio_file(path: &str, output_sink: &str) -> Result<(), String> {
    spawn_audio_player(path, output_sink, 1.0).map(|mut child| {
        // Detach: notifications / one-shot viewers don't track the process.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    })
}

fn spawn_audio_player(
    path: &str,
    output_sink: &str,
    gain: f64,
) -> Result<std::process::Child, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() < 256 {
        return Err("file too small / corrupt".into());
    }
    let sink = if output_sink.is_empty() || output_sink == "default" {
        None
    } else {
        Some(output_sink.to_string())
    };
    let gain = gain.clamp(0.0, 2.5);
    let mpv_vol = (gain * 100.0).round().clamp(0.0, 250.0);

    let mut mpv = std::process::Command::new("mpv");
    mpv.args(["--no-video", "--really-quiet"]);
    mpv.arg(format!("--volume={mpv_vol}"));
    if let Some(ref s) = sink {
        mpv.arg(format!("--audio-device=pulse/{s}"));
    }
    mpv.arg(path);
    mpv.stdout(std::process::Stdio::null());
    mpv.stderr(std::process::Stdio::null());
    if let Ok(child) = mpv.spawn() {
        return Ok(child);
    }

    let mut ff = std::process::Command::new("ffplay");
    ff.args(["-nodisp", "-autoexit", "-loglevel", "quiet"]);
    if gain != 1.0 {
        ff.args(["-af", &format!("volume={gain}")]);
    }
    if let Some(ref s) = sink {
        ff.args(["-audiodevice", s]);
    }
    ff.arg(path);
    ff.stdout(std::process::Stdio::null());
    ff.stderr(std::process::Stdio::null());
    if let Ok(child) = ff.spawn() {
        return Ok(child);
    }

    if (gain - 1.0).abs() < 0.01 {
        let mut paplay = std::process::Command::new("paplay");
        if let Some(ref s) = sink {
            paplay.args(["--device", s]);
        }
        paplay.arg(path);
        paplay.stdout(std::process::Stdio::null());
        paplay.stderr(std::process::Stdio::null());
        if let Ok(child) = paplay.spawn() {
            return Ok(child);
        }
    }

    Err("no audio player (mpv/ffplay)".into())
}

fn suggest_media_name(msg: &MessageInfo) -> String {
    if let Some(n) = msg.file_name.as_deref().filter(|s| !s.is_empty()) {
        return n.to_string();
    }
    let ct = msg.content_type.to_ascii_uppercase();
    match ct.as_str() {
        "IMAGE" => format!("{}.jpg", msg.id),
        "VIDEO" => format!("{}.mp4", msg.id),
        "AUDIO" => format!("{}.m4a", msg.id),
        "FILE" if !msg.text.is_empty() && msg.text != "[File]" => msg.text.clone(),
        _ => format!("line-{}", msg.id),
    }
}

fn is_thumb_media_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains(".thumb.") || lower.ends_with(".thumb.jpg") || lower.ends_with(".thumb.png")
}

fn is_full_media_path(path: &str) -> bool {
    path.to_ascii_lowercase().contains(".full.")
}

fn image_looks_low_res(path: &str) -> bool {
    if is_full_media_path(path) {
        return false;
    }
    if is_thumb_media_path(path) {
        return true;
    }
    match gdk_pixbuf::Pixbuf::from_file(path) {
        Ok(pb) => pb.width() <= 512 && pb.height() <= 512,
        Err(_) => false,
    }
}

fn full_image_candidate(path: &str) -> Option<String> {
    if is_full_media_path(path) && std::path::Path::new(path).exists() {
        return Some(path.to_string());
    }
    if !is_thumb_media_path(path)
        && std::path::Path::new(path).exists()
        && !image_looks_low_res(path)
    {
        return Some(path.to_string());
    }
    // Try sibling .full. next to a polluted preview or thumb.
    let p = std::path::Path::new(path);
    let parent = p.parent()?;
    let name = p.file_name()?.to_str()?;
    let id = name.split('.').next().filter(|s| !s.is_empty())?;
    for ext in ["jpg", "jpeg", "png", "webp", "gif"] {
        let cand = parent.join(format!("{id}.full.{ext}"));
        if cand.is_file() {
            return Some(cand.to_string_lossy().to_string());
        }
    }
    if is_thumb_media_path(path) {
        for ext in ["jpg", "jpeg", "png", "webp", "gif"] {
            let cand = parent.join(format!("{id}.{ext}"));
            if cand.is_file() && !image_looks_low_res(&cand.to_string_lossy()) {
                return Some(cand.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn local_media_path(msg: &MessageInfo, for_viewer: bool) -> Option<String> {
    let ct = msg.content_type.to_ascii_uppercase();
    if ct == "AUDIO" {
        return msg
            .audio_path
            .as_ref()
            .or(msg.file_path.as_ref())
            .filter(|p| std::path::Path::new(p).exists())
            .cloned();
    }
    if ct == "VIDEO" {
        if let Some(p) = msg.file_path.as_ref().filter(|p| {
            std::path::Path::new(p).exists() && p.to_ascii_lowercase().ends_with(".mp4")
        }) {
            return Some(p.clone());
        }
        // Viewer needs full video; thumb alone is not enough.
        if for_viewer {
            return None;
        }
    }
    if let Some(p) = msg
        .file_path
        .as_ref()
        .filter(|p| std::path::Path::new(p).exists() && !is_thumb_media_path(p))
        && !(for_viewer && ct == "IMAGE" && image_looks_low_res(p))
    {
        return Some(p.clone());
    }
    if ct == "IMAGE" {
        if for_viewer {
            // Never open the chat thumbnail / OBS preview in the viewer.
            if let Some(p) = msg.file_path.as_ref().and_then(|p| full_image_candidate(p)) {
                return Some(p);
            }
            if let Some(p) = msg
                .image_path
                .as_ref()
                .and_then(|p| full_image_candidate(p))
            {
                return Some(p);
            }
            return None;
        }
        if let Some(p) = msg
            .image_path
            .as_ref()
            .filter(|p| std::path::Path::new(p).exists())
        {
            return Some(p.clone());
        }
    } else if !for_viewer
        && let Some(p) = msg
            .image_path
            .as_ref()
            .filter(|p| std::path::Path::new(p).exists())
    {
        return Some(p.clone());
    }
    None
}

fn request_media_download(state: &AppState, msg: &MessageInfo, action: &str) {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if msg.id.is_empty() {
        toast(state, &crate::i18n::t("media_download_failed"));
        return;
    }
    // Prefer freshest paths from hydrate / download cache.
    let msg = state
        .media_msgs
        .borrow()
        .get(&msg.id)
        .cloned()
        .unwrap_or_else(|| msg.clone());
    let for_viewer = action == "open_viewer";
    if let Some(path) = local_media_path(&msg, for_viewer) {
        finish_media_action(
            state,
            action,
            &path,
            &suggest_media_name(&msg),
            &msg.content_type,
        );
        return;
    }
    state.status.set_text(&crate::i18n::t("media_downloading"));
    match state.sidecar.download_media(&chat_mid, &msg.id) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::DownloadMedia {
                    message_id: msg.id.clone(),
                    action: action.to_string(),
                    content_type: msg.content_type.clone(),
                    suggest_name: suggest_media_name(&msg),
                },
            );
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("media_download_failed_err", &[("error", &e.to_string())]),
        ),
    }
}

fn finish_media_action(
    state: &AppState,
    action: &str,
    path: &str,
    suggest_name: &str,
    content_type: &str,
) {
    match action {
        "open_viewer" => open_media_viewer(state, path, content_type, suggest_name),
        "save_dialog" => save_media_as(state, path, suggest_name, content_type),
        "play_audio" => {
            if let Err(e) = play_audio_file(path, &state.config.borrow().audio_output) {
                toast(
                    state,
                    &crate::i18n::tf("voice_play_failed", &[("error", &e)]),
                );
            }
        }
        _ => save_media_as(state, path, suggest_name, content_type),
    }
}

fn copy_media_to_dest(state: &AppState, src: &str, dest: &std::path::Path) {
    match std::fs::copy(src, dest) {
        Ok(_) => {
            toast(
                state,
                &crate::i18n::tf("media_saved", &[("path", &dest.display().to_string())]),
            );
            state.status.set_text(&crate::i18n::t("media_saved_status"));
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("media_download_failed_err", &[("error", &e.to_string())]),
        ),
    }
}

fn save_media_as(state: &AppState, path: &str, suggest_name: &str, content_type: &str) {
    let cfg = state.config.borrow().clone();
    let dest_dir = cfg.download_dir_for(content_type);
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        toast(
            state,
            &crate::i18n::tf("media_download_failed_err", &[("error", &e.to_string())]),
        );
        return;
    }

    if !cfg.download_ask_every_time {
        let dest = downloads::unique_download_dest(&dest_dir, suggest_name);
        copy_media_to_dest(state, path, &dest);
        return;
    }

    let filter = gtk::FileFilter::new();
    filter.set_name(Some("All files"));
    filter.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(crate::i18n::t("media_save_as"))
        .modal(true)
        .initial_name(suggest_name)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    dialog.set_initial_folder(Some(&gio::File::for_path(&dest_dir)));
    let s = state.clone();
    let src = path.to_string();
    dialog.save(
        Some(&state.window),
        None::<&gio::Cancellable>,
        move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(dest) = file.path() else {
                toast(&s, &crate::i18n::t("media_download_failed"));
                return;
            };
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            copy_media_to_dest(&s, &src, &dest);
        },
    );
}

fn wire_media_open_click(state: &AppState, pic: &gtk::Picture, msg: &MessageInfo, _kind: &str) {
    wire_media_open_click_widget(state, pic.upcast_ref::<gtk::Widget>(), msg);
}

fn open_media_viewer(state: &AppState, path: &str, content_type: &str, suggest_name: &str) {
    let kind = viewer_kind_for(content_type, path, suggest_name);
    let (def_w, def_h) = match kind {
        ViewerKind::Video => (1100, 720),
        ViewerKind::Image => (1080, 760),
        ViewerKind::Pdf | ViewerKind::Text => (920, 700),
        _ => (720, 480),
    };

    let win = gtk::Window::builder()
        .transient_for(&state.window)
        .modal(true)
        .title(crate::i18n::t("media_viewer_title"))
        .default_width(def_w)
        .default_height(def_h)
        .css_classes(["line-media-viewer"])
        .build();

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .css_classes(["line-media-viewer-root"])
        .build();

    let bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_start(12)
        .margin_end(12)
        .margin_top(8)
        .margin_bottom(8)
        .css_classes(["line-media-viewer-bar"])
        .build();
    let title = gtk::Label::builder()
        .label(suggest_name)
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["heading"])
        .build();

    let open_ext = gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(crate::i18n::t("media_open_externally"))
        .css_classes(["flat", "circular"])
        .build();
    let dl = gtk::Button::builder()
        .icon_name("folder-download-symbolic")
        .tooltip_text(crate::i18n::t("media_download"))
        .css_classes(["flat", "circular"])
        .build();
    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text(crate::i18n::t("media_close"))
        .css_classes(["flat", "circular"])
        .build();
    bar.append(&title);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .css_classes(["line-media-viewer-body"])
        .build();

    match kind {
        ViewerKind::Video => {
            append_gpu_video_viewer(&body, path);
        }
        ViewerKind::Image => {
            append_image_viewer(state, &bar, &body, path, &win);
        }
        ViewerKind::Pdf => {
            append_pdf_viewer(&body, path, suggest_name);
        }
        ViewerKind::Text => {
            append_text_viewer(&body, path);
        }
        ViewerKind::Audio => {
            append_audio_viewer(state, &body, path);
        }
        ViewerKind::Generic => {
            append_generic_file_viewer(&body, path, suggest_name);
        }
    }

    bar.append(&open_ext);
    bar.append(&dl);
    bar.append(&close);
    root.append(&bar);
    root.append(&body);
    win.set_child(Some(&root));

    {
        let w = win.clone();
        close.connect_clicked(move |_| w.close());
    }
    {
        let path = path.to_string();
        open_ext.connect_clicked(move |_| {
            open_path_externally(&path);
        });
    }
    {
        let s = state.clone();
        let path = path.to_string();
        let name = suggest_name.to_string();
        let ct = content_type.to_string();
        dl.connect_clicked(move |_| {
            save_media_as(&s, &path, &name, &ct);
        });
    }

    let controller = gtk::EventControllerKey::new();
    let w = win.clone();
    controller.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(controller);
    win.present();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewerKind {
    Image,
    Video,
    Pdf,
    Text,
    Audio,
    Generic,
}

fn viewer_kind_for(content_type: &str, path: &str, name: &str) -> ViewerKind {
    let ct = content_type.to_ascii_uppercase();
    let lower = format!(
        "{} {}",
        path.to_ascii_lowercase(),
        name.to_ascii_lowercase()
    );
    if ct == "VIDEO"
        || lower.contains(".mp4")
        || lower.contains(".webm")
        || lower.contains(".mkv")
        || lower.contains(".mov")
        || lower.contains(".m4v")
    {
        return ViewerKind::Video;
    }
    if ct == "IMAGE"
        || lower.contains(".jpg")
        || lower.contains(".jpeg")
        || lower.contains(".png")
        || lower.contains(".webp")
        || lower.contains(".gif")
        || lower.contains(".bmp")
    {
        return ViewerKind::Image;
    }
    if ct == "AUDIO"
        || lower.contains(".m4a")
        || lower.contains(".mp3")
        || lower.contains(".aac")
        || lower.contains(".ogg")
        || lower.contains(".wav")
        || lower.contains(".flac")
    {
        return ViewerKind::Audio;
    }
    if lower.contains(".pdf") {
        return ViewerKind::Pdf;
    }
    if lower.contains(".txt")
        || lower.contains(".md")
        || lower.contains(".log")
        || lower.contains(".json")
        || lower.contains(".csv")
        || lower.contains(".xml")
        || lower.contains(".yaml")
        || lower.contains(".yml")
        || lower.contains(".toml")
        || lower.contains(".rs")
        || lower.contains(".py")
        || lower.contains(".js")
        || lower.contains(".ts")
        || lower.contains(".css")
        || lower.contains(".html")
        || lower.contains(".c")
        || lower.contains(".h")
        || lower.contains(".go")
    {
        return ViewerKind::Text;
    }
    // Sniff text files without extension.
    if (ct == "FILE" || ct.is_empty()) && looks_like_text_file(path) {
        return ViewerKind::Text;
    }
    ViewerKind::Generic
}

fn looks_like_text_file(path: &str) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut buf = [0u8; 512];
    let Ok(n) = f.read(&mut buf) else {
        return false;
    };
    if n == 0 {
        return false;
    }
    let sample = &buf[..n];
    if sample.contains(&0) {
        return false;
    }
    let textish = sample
        .iter()
        .filter(|b| b.is_ascii_graphic() || b.is_ascii_whitespace())
        .count();
    textish * 100 / n >= 90
}

fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = n as f64;
    if v >= GB {
        format!("{:.1} GB", v / GB)
    } else if v >= MB {
        format!("{:.1} MB", v / MB)
    } else if v >= KB {
        format!("{:.0} KB", v / KB)
    } else {
        format!("{n} B")
    }
}

fn open_path_externally(path: &str) {
    let uri = format!("file://{}", path);
    if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_ok() {
        return;
    }
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// Prefer GPU decode + offloaded composition for in-app video.
fn append_gpu_video_viewer(body: &gtk::Box, path: &str) {
    let file = gio::File::for_path(path);
    let stream = gtk::MediaFile::for_file(&file);
    stream.set_loop(false);

    let video = gtk::Video::for_media_stream(Some(&stream));
    video.set_autoplay(true);
    video.set_loop(false);
    video.set_hexpand(true);
    video.set_vexpand(true);
    video.add_css_class("line-media-viewer-video");
    // GTK 4.14+: keep decoded frames on the GPU instead of round-tripping through CPU.
    video.set_graphics_offload(gtk::GraphicsOffloadEnabled::Enabled);

    let offload = gtk::GraphicsOffload::new(Some(&video));
    offload.set_hexpand(true);
    offload.set_vexpand(true);
    offload.add_css_class("line-media-viewer-video");

    body.append(&offload);
    stream.play();
}

fn append_image_viewer(
    state: &AppState,
    bar: &gtk::Box,
    body: &gtk::Box,
    path: &str,
    win: &gtk::Window,
) {
    #[derive(Clone)]
    struct Stroke {
        color: (f64, f64, f64, f64),
        width: f64,
        points: Vec<(f64, f64)>,
    }

    let Ok(base_pixbuf) = gdk_pixbuf::Pixbuf::from_file(path) else {
        let label = gtk::Label::builder()
            .label(crate::i18n::t("media_unsupported_preview"))
            .css_classes(["dim-label"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        body.append(&label);
        return;
    };
    let nat_w = base_pixbuf.width().max(1);
    let nat_h = base_pixbuf.height().max(1);

    // View transform: image pixel (ix,iy) -> widget (ox + ix*zoom, oy + iy*zoom).
    let zoom = Rc::new(RefCell::new(1.0_f64));
    let offset = Rc::new(RefCell::new((0.0_f64, 0.0_f64)));
    let fitted = Rc::new(RefCell::new(false));
    let fast_paint = Rc::new(RefCell::new(false));
    let pixbuf = Rc::new(base_pixbuf);
    // Reused viewport-sized buffer so pan/zoom never reallocates every frame.
    let scratch = Rc::new(RefCell::new(None::<gdk_pixbuf::Pixbuf>));
    let strokes = Rc::new(RefCell::new(Vec::<Stroke>::new()));
    let draw_mode = Rc::new(RefCell::new(false));
    let brush_color = Rc::new(RefCell::new((0.90_f64, 0.22, 0.21, 1.0)));
    let brush_width = Rc::new(RefCell::new(6.0_f64));
    let active_stroke = Rc::new(RefCell::new(None::<Stroke>));
    let pan_origin = Rc::new(RefCell::new((0.0_f64, 0.0)));

    let zoom_out = gtk::Button::builder()
        .icon_name("zoom-out-symbolic")
        .tooltip_text(crate::i18n::t("media_zoom_out"))
        .css_classes(["flat", "circular"])
        .build();
    let zoom_reset = gtk::Button::builder()
        .icon_name("zoom-fit-best-symbolic")
        .tooltip_text(crate::i18n::t("media_zoom_reset"))
        .css_classes(["flat", "circular"])
        .build();
    let zoom_in = gtk::Button::builder()
        .icon_name("zoom-in-symbolic")
        .tooltip_text(crate::i18n::t("media_zoom_in"))
        .css_classes(["flat", "circular"])
        .build();
    bar.append(&zoom_out);
    bar.append(&zoom_reset);
    bar.append(&zoom_in);

    let draw_btn = gtk::ToggleButton::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text(crate::i18n::t("media_draw"))
        .css_classes(["flat", "circular"])
        .build();
    let undo_btn = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text(crate::i18n::t("media_draw_undo"))
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .build();
    let clear_btn = gtk::Button::builder()
        .icon_name("edit-clear-all-symbolic")
        .tooltip_text(crate::i18n::t("media_draw_clear"))
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .build();
    let send_btn = gtk::Button::builder()
        .icon_name("mail-send-symbolic")
        .tooltip_text(crate::i18n::t("media_draw_send"))
        .css_classes(["flat", "circular", "suggested-action"])
        .build();
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&draw_btn);
    for (r, g, b, class, tip) in [
        (0.90, 0.22, 0.21, "line-draw-c-red", "Red"),
        (0.26, 0.63, 0.28, "line-draw-c-green", "Green"),
        (0.12, 0.53, 0.90, "line-draw-c-blue", "Blue"),
        (0.99, 0.85, 0.21, "line-draw-c-yellow", "Yellow"),
        (1.0, 1.0, 1.0, "line-draw-c-white", "White"),
        (0.13, 0.13, 0.13, "line-draw-c-black", "Black"),
    ] {
        let btn = gtk::Button::builder()
            .label("●")
            .tooltip_text(tip)
            .css_classes(["flat", "circular", "line-draw-swatch", class])
            .build();
        let brush_color = brush_color.clone();
        let draw_btn = draw_btn.clone();
        btn.connect_clicked(move |_| {
            *brush_color.borrow_mut() = (r, g, b, 1.0);
            if !draw_btn.is_active() {
                draw_btn.set_active(true);
            }
        });
        bar.append(&btn);
    }
    bar.append(&undo_btn);
    bar.append(&clear_btn);
    bar.append(&send_btn);

    let area = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .css_classes(["line-media-viewer-image"])
        .build();
    area.set_cursor_from_name(Some("grab"));
    area.set_draw_func({
        let pixbuf = pixbuf.clone();
        let scratch = scratch.clone();
        let zoom = zoom.clone();
        let offset = offset.clone();
        let fast_paint = fast_paint.clone();
        let strokes = strokes.clone();
        let active_stroke = active_stroke.clone();
        move |_area, cr, width, height| {
            if width <= 0 || height <= 0 {
                return;
            }
            // Clear is handled by GTK/CSS background; only composite the scaled viewport.
            let z = (*zoom.borrow()).max(0.01);
            let (ox, oy) = *offset.borrow();
            let interp = if *fast_paint.borrow() {
                gdk_pixbuf::InterpType::Nearest
            } else {
                gdk_pixbuf::InterpType::Bilinear
            };

            {
                let mut slot = scratch.borrow_mut();
                let needs = slot
                    .as_ref()
                    .map(|p| p.width() != width || p.height() != height)
                    .unwrap_or(true);
                if needs {
                    *slot = gdk_pixbuf::Pixbuf::new(
                        gdk_pixbuf::Colorspace::Rgb,
                        true,
                        8,
                        width,
                        height,
                    );
                }
                if let Some(dest) = slot.as_ref() {
                    // Transparent edges so the themed DrawingArea background shows through.
                    dest.fill(0);
                    pixbuf.scale(dest, 0, 0, width, height, ox, oy, z, z, interp);
                    gdk::prelude::GdkCairoContextExt::set_source_pixbuf(cr, dest, 0.0, 0.0);
                    let _ = cr.paint();
                }
            }

            // Strokes in image space → widget space.
            let paint_stroke = |cr: &gtk::cairo::Context, stroke: &Stroke| {
                if stroke.points.is_empty() {
                    return;
                }
                let (r, g, b, a) = stroke.color;
                cr.set_source_rgba(r, g, b, a);
                cr.set_line_width((stroke.width * z).max(1.0));
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_line_join(gtk::cairo::LineJoin::Round);
                let (x0, y0) = stroke.points[0];
                cr.move_to(ox + x0 * z, oy + y0 * z);
                for &(x, y) in stroke.points.iter().skip(1) {
                    cr.line_to(ox + x * z, oy + y * z);
                }
                if stroke.points.len() == 1 {
                    cr.line_to(ox + x0 * z + 0.01, oy + y0 * z);
                }
                let _ = cr.stroke();
            };
            for stroke in strokes.borrow().iter() {
                paint_stroke(cr, stroke);
            }
            if let Some(stroke) = active_stroke.borrow().as_ref() {
                paint_stroke(cr, stroke);
            }
        }
    });

    let fit_to_area = {
        let zoom = zoom.clone();
        let offset = offset.clone();
        let area = area.clone();
        Rc::new(move || {
            let aw = area.width().max(1) as f64;
            let ah = area.height().max(1) as f64;
            let z = (aw / nat_w as f64).min(ah / nat_h as f64).clamp(0.05, 1.0);
            *zoom.borrow_mut() = z;
            *offset.borrow_mut() = ((aw - nat_w as f64 * z) * 0.5, (ah - nat_h as f64 * z) * 0.5);
            area.queue_draw();
        })
    };

    let zoom_at = {
        let zoom = zoom.clone();
        let offset = offset.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        Rc::new(move |factor: f64, pivot_x: f64, pivot_y: f64, fast: bool| {
            let old = (*zoom.borrow()).max(0.01);
            let new_z = (old * factor).clamp(0.05, 16.0);
            if (new_z - old).abs() < f64::EPSILON {
                return;
            }
            let (ox, oy) = *offset.borrow();
            // Keep the image point under the pivot fixed.
            let ox2 = pivot_x - (pivot_x - ox) * (new_z / old);
            let oy2 = pivot_y - (pivot_y - oy) * (new_z / old);
            *zoom.borrow_mut() = new_z;
            *offset.borrow_mut() = (ox2, oy2);
            *fast_paint.borrow_mut() = fast;
            area.queue_draw();
        })
    };

    {
        let area = area.clone();
        let fitted = fitted.clone();
        let fit_to_area = fit_to_area.clone();
        area.connect_resize(move |_, _, _| {
            if !*fitted.borrow() {
                *fitted.borrow_mut() = true;
                fit_to_area();
            }
        });
    }

    {
        let zoom_at = zoom_at.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        zoom_in.connect_clicked(move |_| {
            let cx = area.width() as f64 * 0.5;
            let cy = area.height() as f64 * 0.5;
            zoom_at(1.25, cx, cy, false);
            *fast_paint.borrow_mut() = false;
            area.queue_draw();
        });
    }
    {
        let zoom_at = zoom_at.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        zoom_out.connect_clicked(move |_| {
            let cx = area.width() as f64 * 0.5;
            let cy = area.height() as f64 * 0.5;
            zoom_at(1.0 / 1.25, cx, cy, false);
            *fast_paint.borrow_mut() = false;
            area.queue_draw();
        });
    }
    {
        let fit_to_area = fit_to_area.clone();
        let fast_paint = fast_paint.clone();
        zoom_reset.connect_clicked(move |_| {
            *fast_paint.borrow_mut() = false;
            fit_to_area();
        });
    }

    let refresh_edit_btns = {
        let undo_btn = undo_btn.clone();
        let clear_btn = clear_btn.clone();
        let strokes = strokes.clone();
        Rc::new(move || {
            let n = strokes.borrow().len();
            undo_btn.set_sensitive(n > 0);
            clear_btn.set_sensitive(n > 0);
        })
    };

    {
        let draw_mode = draw_mode.clone();
        let area = area.clone();
        draw_btn.connect_toggled(move |btn| {
            let on = btn.is_active();
            *draw_mode.borrow_mut() = on;
            area.set_cursor_from_name(Some(if on { "crosshair" } else { "grab" }));
        });
    }
    {
        let strokes = strokes.clone();
        let area = area.clone();
        let refresh = refresh_edit_btns.clone();
        undo_btn.connect_clicked(move |_| {
            strokes.borrow_mut().pop();
            refresh();
            area.queue_draw();
        });
    }
    {
        let strokes = strokes.clone();
        let area = area.clone();
        let refresh = refresh_edit_btns.clone();
        clear_btn.connect_clicked(move |_| {
            strokes.borrow_mut().clear();
            refresh();
            area.queue_draw();
        });
    }
    {
        let s = state.clone();
        let win = win.clone();
        let pixbuf = pixbuf.clone();
        let strokes = strokes.clone();
        let src_path = path.to_string();
        send_btn.connect_clicked(move |_| {
            let out = if strokes.borrow().is_empty() {
                std::path::PathBuf::from(&src_path)
            } else {
                let packed: Vec<_> = strokes
                    .borrow()
                    .iter()
                    .map(|st| (st.color, st.width, st.points.clone()))
                    .collect();
                match export_annotated_image(pixbuf.as_ref(), &packed, &s.data_dir) {
                    Ok(p) => p,
                    Err(e) => {
                        toast(
                            &s,
                            &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
                        );
                        return;
                    }
                }
            };
            if send_local_image_file(&s, &out) {
                win.close();
            }
        });
    }

    // Scroll: wheel pans; Ctrl+wheel zooms toward pointer.
    let scroll_ctl = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
    );
    {
        let zoom_at = zoom_at.clone();
        let offset = offset.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        scroll_ctl.connect_scroll(move |ctl, dx, dy| {
            let state = ctl.current_event_state();
            if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                let px = area.width() as f64 * 0.5;
                let py = area.height() as f64 * 0.5;
                let factor = if dy < 0.0 {
                    1.15
                } else if dy > 0.0 {
                    1.0 / 1.15
                } else {
                    return glib::Propagation::Stop;
                };
                zoom_at(factor, px, py, true);
                let fast_paint = fast_paint.clone();
                let area = area.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                    *fast_paint.borrow_mut() = false;
                    area.queue_draw();
                });
                return glib::Propagation::Stop;
            }
            // Pan with wheel.
            let mut off = offset.borrow_mut();
            off.0 -= dx * 48.0;
            off.1 -= dy * 48.0;
            drop(off);
            area.queue_draw();
            glib::Propagation::Stop
        });
    }
    area.add_controller(scroll_ctl);

    // Drag: pan when browsing, draw when edit mode is on.
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    {
        let draw_mode = draw_mode.clone();
        let pan_origin = pan_origin.clone();
        let offset = offset.clone();
        let active_stroke = active_stroke.clone();
        let brush_color = brush_color.clone();
        let brush_width = brush_width.clone();
        let zoom = zoom.clone();
        let area = area.clone();
        let fast_paint = fast_paint.clone();
        drag.connect_drag_begin(move |gesture, x, y| {
            if *draw_mode.borrow() {
                let z = (*zoom.borrow()).max(0.01);
                let (ox, oy) = *offset.borrow();
                let color = *brush_color.borrow();
                let width = (*brush_width.borrow() / z).max(1.0);
                *active_stroke.borrow_mut() = Some(Stroke {
                    color,
                    width,
                    points: vec![((x - ox) / z, (y - oy) / z)],
                });
                area.set_cursor_from_name(Some("crosshair"));
            } else {
                *pan_origin.borrow_mut() = *offset.borrow();
                *fast_paint.borrow_mut() = true;
                area.set_cursor_from_name(Some("grabbing"));
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
    }
    {
        let draw_mode = draw_mode.clone();
        let pan_origin = pan_origin.clone();
        let offset = offset.clone();
        let active_stroke = active_stroke.clone();
        let zoom = zoom.clone();
        let area = area.clone();
        drag.connect_drag_update(move |gesture, _x, _y| {
            let Some((dx, dy)) = gesture.offset() else {
                return;
            };
            let Some((sx, sy)) = gesture.start_point() else {
                return;
            };
            if *draw_mode.borrow() {
                let z = (*zoom.borrow()).max(0.01);
                let (ox, oy) = *offset.borrow();
                if let Some(stroke) = active_stroke.borrow_mut().as_mut() {
                    stroke.points.push(((sx + dx - ox) / z, (sy + dy - oy) / z));
                }
                area.queue_draw();
            } else {
                let (ox, oy) = *pan_origin.borrow();
                *offset.borrow_mut() = (ox + dx, oy + dy);
                area.queue_draw();
            }
        });
    }
    {
        let draw_mode = draw_mode.clone();
        let active_stroke = active_stroke.clone();
        let strokes = strokes.clone();
        let area = area.clone();
        let refresh = refresh_edit_btns.clone();
        let fast_paint = fast_paint.clone();
        drag.connect_drag_end(move |_, _x, _y| {
            if *draw_mode.borrow() {
                if let Some(stroke) = active_stroke.borrow_mut().take()
                    && !stroke.points.is_empty()
                {
                    strokes.borrow_mut().push(stroke);
                    refresh();
                }
                area.set_cursor_from_name(Some("crosshair"));
            } else {
                *fast_paint.borrow_mut() = false;
                area.set_cursor_from_name(Some("grab"));
            }
            area.queue_draw();
        });
    }
    area.add_controller(drag);

    body.append(&area);
}

type PackedStroke = ((f64, f64, f64, f64), f64, Vec<(f64, f64)>);

fn export_annotated_image(
    pixbuf: &gdk_pixbuf::Pixbuf,
    strokes: &[PackedStroke],
    data_dir: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    let w = pixbuf.width().max(1);
    let h = pixbuf.height().max(1);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h)
        .map_err(|e| anyhow::anyhow!("cairo surface: {e}"))?;
    let cr = cairo::Context::new(&surface).map_err(|e| anyhow::anyhow!("cairo: {e}"))?;
    gdk::prelude::GdkCairoContextExt::set_source_pixbuf(&cr, pixbuf, 0.0, 0.0);
    cr.paint().map_err(|e| anyhow::anyhow!("paint: {e}"))?;

    for (color, width, points) in strokes {
        if points.is_empty() {
            continue;
        }
        let (r, g, b, a) = *color;
        cr.set_source_rgba(r, g, b, a);
        cr.set_line_width(*width);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.move_to(points[0].0, points[0].1);
        for &(x, y) in points.iter().skip(1) {
            cr.line_to(x, y);
        }
        if points.len() == 1 {
            cr.line_to(points[0].0 + 0.01, points[0].1);
        }
        cr.stroke().map_err(|e| anyhow::anyhow!("stroke: {e}"))?;
    }

    let dir = data_dir.join("cache/media");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!(
        "annotated-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let mut file = std::fs::File::create(&dest)?;
    surface
        .write_to_png(&mut file)
        .map_err(|e| anyhow::anyhow!("png: {e}"))?;
    Ok(dest)
}

fn send_local_image_file(state: &AppState, path: &std::path::Path) -> bool {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return false;
    };
    if !*state.session_ready.borrow() {
        toast(state, &crate::i18n::t("still_restoring"));
        return false;
    }
    let cached = match copy_into_media_cache(state, path) {
        Ok(p) => p,
        Err(e) => {
            toast(
                state,
                &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
            );
            return false;
        }
    };
    let path_str = cached.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("image.png")
        .to_string();
    match state
        .sidecar
        .send_media(&chat_mid, &path_str, "image", None)
    {
        Ok(id) => {
            begin_optimistic_send(
                state,
                id,
                &chat_mid,
                MessageInfo {
                    id: String::new(),
                    text: "[Image]".into(),
                    from: String::new(),
                    to: chat_mid.clone(),
                    mine: true,
                    created_time: now_ms(),
                    content_type: "IMAGE".into(),
                    image_path: Some(path_str.clone()),
                    audio_path: None,
                    file_name: Some(file_name),
                    file_path: Some(path_str),
                    duration_ms: None,
                    flex: None,
                },
            );
            show_upload_progress(state, 0.02, &crate::i18n::t("media_uploading"));
            dismiss_new_marker(state);
            pin_messages_to_latest(state);
            true
        }
        Err(e) => {
            toast(
                state,
                &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
            );
            false
        }
    }
}

fn append_pdf_viewer(body: &gtk::Box, path: &str, suggest_name: &str) {
    let wrap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(16)
        .margin_end(16)
        .margin_top(12)
        .margin_bottom(12)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-media-viewer-doc"])
        .build();

    let preview_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .css_classes(["line-media-viewer-pdf-preview"])
        .build();

    // Render first page with pdftoppm when available.
    let cache = std::env::temp_dir().join(format!(
        "line-gtk-pdf-{}-preview",
        std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("doc")
    ));
    let _ = std::fs::create_dir_all(&cache);
    let out_prefix = cache.join("page");
    let rendered = std::process::Command::new("pdftoppm")
        .args([
            "-jpeg",
            "-f",
            "1",
            "-l",
            "1",
            "-r",
            "120",
            path,
            out_prefix.to_str().unwrap_or("/tmp/line-gtk-pdf"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let preview_path = cache.join("page-1.jpg");
    if rendered && preview_path.is_file() {
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        let pic = gtk::Picture::for_filename(&preview_path);
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_can_shrink(true);
        pic.set_hexpand(true);
        pic.set_vexpand(true);
        scroll.set_child(Some(&pic));
        preview_box.append(&scroll);
    } else {
        let hint = gtk::Label::builder()
            .label(crate::i18n::t("media_pdf_preview"))
            .css_classes(["title-3"])
            .build();
        preview_box.append(&hint);
    }

    let meta = gtk::Label::builder()
        .label({
            let size = std::fs::metadata(path)
                .map(|m| format_bytes(m.len()))
                .unwrap_or_else(|_| "?".into());
            format!(
                "{suggest_name} · {}",
                crate::i18n::tf("media_file_size", &[("size", &size)])
            )
        })
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();

    // Optional text extract for searchable peek.
    let text_out = std::process::Command::new("pdftotext")
        .args(["-f", "1", "-l", "2", "-layout", path, "-"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_default();
    if !text_out.trim().is_empty() {
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .min_content_height(140)
            .css_classes(["line-media-viewer-text-scroll"])
            .build();
        let tv = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .monospace(true)
            .css_classes(["line-media-viewer-text"])
            .build();
        let buf = tv.buffer();
        let snippet: String = text_out.chars().take(4_000).collect();
        buf.set_text(&snippet);
        scroll.set_child(Some(&tv));
        wrap.append(&preview_box);
        wrap.append(&meta);
        wrap.append(&scroll);
    } else {
        wrap.append(&preview_box);
        wrap.append(&meta);
    }

    let open_btn = gtk::Button::builder()
        .label(crate::i18n::t("media_open_externally"))
        .halign(gtk::Align::Start)
        .css_classes(["suggested-action"])
        .build();
    let path_owned = path.to_string();
    open_btn.connect_clicked(move |_| open_path_externally(&path_owned));
    wrap.append(&open_btn);
    body.append(&wrap);
}

fn append_text_viewer(body: &gtk::Box, path: &str) {
    let scroll = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-media-viewer-text-scroll"])
        .build();
    let tv = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .monospace(true)
        .left_margin(14)
        .right_margin(14)
        .top_margin(12)
        .bottom_margin(12)
        .css_classes(["line-media-viewer-text"])
        .build();
    let buf = tv.buffer();
    const LIMIT: u64 = 1_500_000;
    let meta = std::fs::metadata(path).ok();
    let truncated = meta.map(|m| m.len() > LIMIT).unwrap_or(false);
    match std::fs::read(path) {
        Ok(bytes) => {
            let slice = if bytes.len() as u64 > LIMIT {
                &bytes[..LIMIT as usize]
            } else {
                &bytes[..]
            };
            let text = String::from_utf8_lossy(slice);
            if truncated {
                buf.set_text(&format!(
                    "{}\n\n{}",
                    crate::i18n::t("media_text_truncated"),
                    text
                ));
            } else {
                buf.set_text(&text);
            }
        }
        Err(e) => {
            buf.set_text(&crate::i18n::tf(
                "file_read_failed",
                &[("error", &e.to_string())],
            ));
        }
    }
    scroll.set_child(Some(&tv));
    body.append(&scroll);
}

fn append_audio_viewer(state: &AppState, body: &gtk::Box, path: &str) {
    let wrap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-media-viewer-audio"])
        .build();
    let icon = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    icon.set_pixel_size(72);
    let size = std::fs::metadata(path)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".into());
    let label = gtk::Label::builder()
        .label(crate::i18n::tf("media_file_size", &[("size", &size)]))
        .css_classes(["dim-label"])
        .build();
    let play = gtk::Button::builder()
        .label(crate::i18n::t("play_voice"))
        .css_classes(["suggested-action", "pill"])
        .build();
    let cfg = state.config.clone();
    let toast_state = state.clone();
    let path = path.to_string();
    play.connect_clicked(move |_| {
        if let Err(e) = play_audio_file(&path, &cfg.borrow().audio_output) {
            toast(
                &toast_state,
                &crate::i18n::tf("voice_play_failed", &[("error", &e)]),
            );
        }
    });
    wrap.append(&icon);
    wrap.append(&label);
    wrap.append(&play);
    body.append(&wrap);
}

fn append_generic_file_viewer(body: &gtk::Box, path: &str, suggest_name: &str) {
    let wrap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .margin_start(24)
        .margin_end(24)
        .css_classes(["line-media-viewer-generic"])
        .build();
    let icon = gtk::Image::from_icon_name("folder-download-symbolic");
    icon.set_pixel_size(64);
    let name = gtk::Label::builder()
        .label(suggest_name)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["title-3"])
        .build();
    let size = std::fs::metadata(path)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".into());
    let meta = gtk::Label::builder()
        .label(crate::i18n::tf("media_file_size", &[("size", &size)]))
        .css_classes(["dim-label"])
        .build();
    let hint = gtk::Label::builder()
        .label(crate::i18n::t("media_unsupported_preview"))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label"])
        .build();
    let open_btn = gtk::Button::builder()
        .label(crate::i18n::t("media_open_externally"))
        .css_classes(["suggested-action"])
        .build();
    let path_owned = path.to_string();
    open_btn.connect_clicked(move |_| open_path_externally(&path_owned));
    wrap.append(&icon);
    wrap.append(&name);
    wrap.append(&meta);
    wrap.append(&hint);
    wrap.append(&open_btn);
    body.append(&wrap);
}

fn pulse_device_list(kind: &str) -> Vec<(String, String)> {
    // kind: "sources" | "sinks"
    let out = std::process::Command::new("pactl")
        .args(["list", "short", kind])
        .output()
        .ok();
    let Some(out) = out else {
        return vec![("default".into(), "Default".into())];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut list = vec![("default".into(), "Default".into())];
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 {
            continue;
        }
        let name = cols[1].to_string();
        // Skip monitors as record sources unless nothing else.
        if kind == "sources" && name.contains(".monitor") {
            continue;
        }
        let label = name.clone();
        list.push((name, label));
    }
    list
}

pub fn pulse_sources() -> Vec<(String, String)> {
    pulse_device_list("sources")
}

pub fn pulse_sinks() -> Vec<(String, String)> {
    pulse_device_list("sinks")
}

fn guess_media_o_type(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "gif" => "gif",
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "heic" => "image",
        "mp4" | "webm" | "mov" | "mkv" | "m4v" => "video",
        "m4a" | "mp3" | "wav" | "ogg" | "oga" | "aac" | "flac" => "audio",
        _ => "file",
    }
}

fn ffprobe_duration_ms(path: &std::path::Path) -> Option<u64> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path.to_str()?,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let secs: f64 = text.trim().parse().ok()?;
    if !secs.is_finite() || secs <= 0.0 {
        return None;
    }
    Some((secs * 1000.0).round() as u64)
}

fn copy_into_media_cache(state: &AppState, src: &std::path::Path) -> anyhow::Result<PathBuf> {
    let dir = state.data_dir.join("cache/media");
    std::fs::create_dir_all(&dir)?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("bin");
    let name = format!(
        "out-{}-{}.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        src.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(40)
            .collect::<String>(),
        ext
    );
    let dest = dir.join(name);
    std::fs::copy(src, &dest)?;
    Ok(dest)
}

fn open_sticker_picker(state: &AppState) {
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

fn fill_sticker_popover(state: &AppState, result: &serde_json::Value) {
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

fn send_sticker_now(
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

fn pick_and_send_media(state: &AppState) {
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
    dialog.open(
        Some(&state.window),
        None::<&gio::Cancellable>,
        move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file.path() else {
                toast(
                    &s,
                    &crate::i18n::tf("media_send_failed", &[("error", "no path")]),
                );
                return;
            };
            send_local_media_path(&s, path);
        },
    );
}

/// Ctrl+V attachment: files (URI list / FileList) or a clipboard image.
/// Returns true when paste was handled so text paste is suppressed.
fn try_paste_clipboard_attachment(state: &AppState) -> bool {
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
                        let mut any = false;
                        for file in list.files() {
                            if let Some(path) = file.path()
                                && path.is_file()
                            {
                                send_local_media_path(&s, path);
                                any = true;
                            }
                        }
                        if !any {
                            paste_clipboard_uri_list(&s);
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
                Ok(path) => send_local_media_path(&s, path),
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

fn paste_clipboard_uri_list(state: &AppState) {
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
            for line in buf.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let file = gio::File::for_uri(line);
                if let Some(path) = file.path()
                    && path.is_file()
                {
                    send_local_media_path(&s, path);
                }
            }
        },
    );
}

fn paste_clipboard_image_bytes(state: &AppState) {
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
                Ok(path) => send_local_media_path(&s, path),
                Err(e) => toast(
                    &s,
                    &crate::i18n::tf("media_send_failed", &[("error", &e.to_string())]),
                ),
            }
        },
    );
}

fn save_clipboard_texture_png(state: &AppState, tex: &gdk::Texture) -> anyhow::Result<PathBuf> {
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

fn write_clipboard_bytes(state: &AppState, bytes: &[u8], ext: &str) -> anyhow::Result<PathBuf> {
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

fn send_local_media_path(state: &AppState, path: PathBuf) {
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

fn start_voice_record(state: &AppState) {
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

fn stop_record_tick(state: &AppState) {
    if let Some(id) = state.recording_tick.borrow_mut().take() {
        id.remove();
    }
}

fn start_record_tick(state: &AppState) {
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

fn stop_ffmpeg_recording(state: &AppState) -> Option<std::time::Instant> {
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
fn finish_voice_record(state: &AppState, send: bool) {
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

fn filter_chats(state: &AppState, query: &str) {
    let q = query.trim().to_lowercase();
    let chats = state.chats.borrow();
    let mut idx = 0;
    while let Some(row) = state.chat_list.row_at_index(idx) {
        let visible = if q.is_empty() {
            true
        } else {
            chats
                .get(idx as usize)
                .map(|c| {
                    c.name.to_lowercase().contains(&q) || c.preview.to_lowercase().contains(&q)
                })
                .unwrap_or(true)
        };
        row.set_visible(visible);
        idx += 1;
    }
}

fn append_flex_card(
    state: &AppState,
    bubble: &gtk::Box,
    msg: &MessageInfo,
    flex: &crate::protocol::FlexInfo,
) {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["line-flex-card"])
        .build();

    let title = if flex.alt_text.is_empty() {
        msg.text.clone()
    } else {
        flex.alt_text.clone()
    };
    if !title.is_empty() {
        card.append(
            &gtk::Label::builder()
                .label(&title)
                .xalign(0.0)
                .wrap(true)
                .max_width_chars(40)
                .css_classes(["line-flex-title"])
                .build(),
        );
    }
    for t in flex.texts.iter().take(6) {
        if *t == title {
            continue;
        }
        card.append(
            &gtk::Label::builder()
                .label(t)
                .xalign(0.0)
                .wrap(true)
                .max_width_chars(40)
                .css_classes(["dim-label"])
                .build(),
        );
    }

    for action in flex.actions.iter().take(12) {
        let btn = gtk::Button::builder()
            .label(&action.label)
            .hexpand(true)
            .css_classes(["line-flex-action"])
            .build();
        let state = state.clone();
        let action = action.clone();
        let msg_id = msg.id.clone();
        let chat = state.current_chat.borrow().clone().unwrap_or_default();
        btn.connect_clicked(move |_| {
            handle_flex_action(&state, &chat, &msg_id, &action);
        });
        card.append(&btn);
    }

    bubble.append(&card);
}

fn handle_flex_action(state: &AppState, chat_mid: &str, message_id: &str, action: &FlexAction) {
    let kind = action.kind.to_ascii_lowercase();
    if (kind == "uri" || kind == "url")
        && let Some(uri) = action.uri.as_deref()
    {
        open_uri(uri);
        return;
    }
    if kind == "message" {
        let text = action.data.clone().unwrap_or_else(|| action.label.clone());
        if let Err(e) = state.sidecar.send_message(chat_mid, &text) {
            toast(
                state,
                &crate::i18n::tf("send_failed", &[("error", &e.to_string())]),
            );
        }
        return;
    }
    // postback / default
    let data = action.data.clone().unwrap_or_else(|| action.label.clone());
    match state
        .sidecar
        .send_postback(chat_mid, message_id, &data, action.uri.as_deref())
    {
        Ok(_) => toast(
            state,
            &crate::i18n::tf("action_sent", &[("label", &action.label)]),
        ),
        Err(e) => toast(
            state,
            &crate::i18n::tf("action_failed", &[("error", &e.to_string())]),
        ),
    }
}

fn append_link_chips(_state: &AppState, bubble: &gtk::Box, text: &str) {
    for url in extract_urls(text).into_iter().take(4) {
        if youtube_id(&url).is_some() {
            let box_ = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .css_classes(["line-flex-card"])
                .build();
            box_.append(
                &gtk::Label::builder()
                    .label(crate::i18n::t("youtube_video"))
                    .xalign(0.0)
                    .css_classes(["line-flex-title"])
                    .build(),
            );
            let btn = gtk::LinkButton::builder()
                .label(crate::i18n::t("open_play_browser"))
                .uri(&url)
                .css_classes(["line-link-chip"])
                .build();
            box_.append(&btn);
            bubble.append(&box_);
            continue;
        }
        let btn = gtk::LinkButton::builder()
            .label(if url.len() > 48 {
                format!("{}…", &url[..45])
            } else {
                url.clone()
            })
            .uri(&url)
            .css_classes(["line-link-chip"])
            .build();
        bubble.append(&btn);
    }
}

fn mark_media_failed(state: &AppState, message_id: &str) {
    let Some(bubble) = state.media_slots.borrow().get(message_id).cloned() else {
        return;
    };
    let msg = state.media_msgs.borrow().get(message_id).cloned();
    let is_video = msg
        .as_ref()
        .map(|m| m.content_type.eq_ignore_ascii_case("video"))
        .unwrap_or(false);
    // Don't clobber a voice card with "image unavailable".
    let mut child = bubble.first_child();
    while let Some(w) = child {
        let next = w.next_sibling();
        if w.css_classes().iter().any(|c| c == "line-voice-card") {
            // Keep voice UI; just mark play unavailable text if needed.
            let fail = gtk::Label::builder()
                .label(crate::i18n::t("voice_unavailable"))
                .xalign(0.0)
                .css_classes(["dim-label", "line-media-failed"])
                .build();
            bubble.append(&fail);
            return;
        }
        child = next;
    }
    let mut child = bubble.first_child();
    while let Some(w) = child {
        let next = w.next_sibling();
        let drop = w.css_classes().iter().any(|c| {
            c == "line-media-placeholder"
                || c == "line-bubble-image"
                || c == "line-media-failed"
                || c == "line-video-wrap"
                || c == "line-video-placeholder"
        }) || w.downcast_ref::<gtk::Picture>().is_some()
            || w.downcast_ref::<gtk::Overlay>().is_some();
        if drop {
            bubble.remove(&w);
        }
        child = next;
    }
    if is_video && let Some(msg) = msg.as_ref() {
        append_video_placeholder(state, &bubble, msg, true);
        return;
    }
    let label = gtk::Label::builder()
        .label(crate::i18n::t("image_unavailable"))
        .xalign(0.0)
        .css_classes(["dim-label", "line-media-failed"])
        .build();
    bubble.append(&label);
}

fn make_media_picture_placeholder(sticker: bool) -> gtk::Picture {
    let (w, h) = if sticker { (128, 128) } else { (220, 160) };
    gtk::Picture::builder()
        .content_fit(if sticker {
            gtk::ContentFit::Contain
        } else {
            gtk::ContentFit::Cover
        })
        .can_shrink(true)
        .width_request(w)
        .height_request(h)
        .css_classes(if sticker {
            ["line-sticker-image"]
        } else {
            ["line-bubble-image"]
        })
        .build()
}

fn wrap_video_thumb(state: &AppState, pic: &gtk::Picture, msg: &MessageInfo) -> gtk::Overlay {
    pic.add_css_class("line-video-thumb");
    pic.set_tooltip_text(Some(&crate::i18n::t("media_open_video")));
    let overlay = gtk::Overlay::builder()
        .child(pic)
        .css_classes(["line-video-wrap"])
        .build();
    let badge = gtk::Image::builder()
        .icon_name("media-playback-start-symbolic")
        .pixel_size(28)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-video-play-badge"])
        .build();
    overlay.add_overlay(&badge);
    wire_media_open_click_widget(state, overlay.upcast_ref::<gtk::Widget>(), msg);
    overlay
}

fn append_video_placeholder(state: &AppState, bubble: &gtk::Box, msg: &MessageInfo, failed: bool) {
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .width_request(180)
        .height_request(110)
        .css_classes(if failed {
            vec!["line-video-placeholder", "line-media-failed"]
        } else {
            vec!["line-video-placeholder", "line-media-placeholder"]
        })
        .build();
    let icon = gtk::Image::builder()
        .icon_name(if failed {
            "media-playback-start-symbolic"
        } else {
            "content-loading-symbolic"
        })
        .pixel_size(28)
        .css_classes(["line-video-play-badge"])
        .build();
    let label = gtk::Label::builder()
        .label(if failed {
            crate::i18n::t("video_unavailable")
        } else {
            crate::i18n::t("media_open_video")
        })
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label"])
        .build();
    box_.append(&icon);
    box_.append(&label);
    wire_media_open_click_widget(state, box_.upcast_ref::<gtk::Widget>(), msg);
    bubble.append(&box_);
}

fn show_upload_progress(state: &AppState, progress: f64, label: &str) {
    let frac = progress.clamp(0.0, 1.0);
    state.upload_bar.set_fraction(frac);
    let text = if label.is_empty() {
        crate::i18n::t("media_uploading")
    } else {
        label.to_string()
    };
    state.upload_label.set_text(&text);
    state.upload_revealer.set_reveal_child(true);
    state.status.set_text(&text);
}

fn hide_upload_progress(state: &AppState) {
    state.upload_revealer.set_reveal_child(false);
    state.upload_bar.set_fraction(0.0);
    state.upload_label.set_text("");
}

fn wire_media_open_click_widget(state: &AppState, widget: &gtk::Widget, msg: &MessageInfo) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let s = state.clone();
    let msg = msg.clone();
    click.connect_released(move |_, _n, _x, _y| {
        request_media_download(&s, &msg, "open_viewer");
    });
    widget.add_controller(click);
    widget.set_cursor_from_name(Some("pointer"));
}

fn raw_frame_to_pixbuf(frame: &crate::sticker_anim::RawFrame) -> Option<Pixbuf> {
    let bytes = glib::Bytes::from_owned(frame.rgba.clone());
    Some(Pixbuf::from_bytes(
        &bytes,
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        frame.width,
        frame.height,
        frame.width * 4,
    ))
}

fn apply_frames_to_picture(
    picture: &gtk::Picture,
    frames: crate::sticker_anim::AnimFrames,
    animate: bool,
) {
    let Some(first) = frames.frames.first().and_then(raw_frame_to_pixbuf) else {
        return;
    };
    picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&first)));
    if !animate || frames.frames.len() <= 1 {
        return;
    }

    let max_plays = frames.plays;
    let pixbufs: Vec<(Pixbuf, u32)> = frames
        .frames
        .iter()
        .filter_map(|f| {
            let pb = raw_frame_to_pixbuf(f)?;
            Some((pb, f.delay_ms))
        })
        .collect();
    if pixbufs.len() <= 1 {
        return;
    }

    let frames = std::rc::Rc::new(pixbufs);
    let idx = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let completed_plays = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let picture = picture.clone();

    fn schedule(
        picture: gtk::Picture,
        frames: std::rc::Rc<Vec<(Pixbuf, u32)>>,
        idx: std::rc::Rc<std::cell::Cell<usize>>,
        completed_plays: std::rc::Rc<std::cell::Cell<u32>>,
        max_plays: u32,
    ) {
        let delay = frames
            .get(idx.get())
            .map(|(_, d)| (*d as u64).max(20))
            .unwrap_or(50);
        let picture_weak = picture.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
            let Some(picture) = picture_weak.upgrade() else {
                return;
            };
            if picture.parent().is_none() {
                return;
            }
            let next = (idx.get() + 1) % frames.len();
            if next == 0 {
                let completed = completed_plays.get().saturating_add(1);
                completed_plays.set(completed);
                if max_plays > 0 && completed >= max_plays {
                    return;
                }
            }
            idx.set(next);
            if let Some((pb, _)) = frames.get(next) {
                picture.set_paintable(Some(&gdk::Texture::for_pixbuf(pb)));
            }
            schedule(picture, frames, idx, completed_plays, max_plays);
        });
    }

    schedule(picture, frames, idx, completed_plays, max_plays);
}

fn attach_texture_async(picture: gtk::Picture, path: String, max_px: i32) {
    attach_texture_async_anim(picture, path, max_px, false);
}

fn attach_texture_async_anim(picture: gtk::Picture, path: String, max_px: i32, animate: bool) {
    let (tx, rx) = async_channel::bounded::<Option<crate::sticker_anim::AnimFrames>>(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(crate::sticker_anim::load_scaled(&path, max_px, animate));
    });
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Some(frames)) => apply_frames_to_picture(&picture, frames, animate),
            _ => {
                if let Some(parent) = picture.parent()
                    && let Some(box_) = parent.downcast_ref::<gtk::Box>()
                {
                    box_.remove(&picture);
                    let label = gtk::Label::builder()
                        .label(crate::i18n::t("image_unavailable"))
                        .xalign(0.0)
                        .css_classes(["dim-label", "line-media-failed"])
                        .build();
                    box_.append(&label);
                }
            }
        }
    });
}

fn pump_media_queue(state: &AppState) {
    if *state.media_pumping.borrow() {
        return;
    }
    let next = state.media_queue.borrow_mut().pop_front();
    let Some((message_id, image_path)) = next else {
        return;
    };
    if !std::path::Path::new(&image_path).exists() {
        mark_media_failed(state, &message_id);
        let state = state.clone();
        glib::idle_add_local_once(move || pump_media_queue(&state));
        return;
    }

    let Some(bubble) = state.media_slots.borrow().get(&message_id).cloned() else {
        // Bubble not mounted yet (list rebuild / idle chunk). Keep the path so
        // append_message can attach it when the row appears; do not spin the queue.
        state
            .media_ready_paths
            .borrow_mut()
            .insert(message_id, image_path);
        let state = state.clone();
        glib::idle_add_local_once(move || pump_media_queue(&state));
        return;
    };

    *state.media_pumping.borrow_mut() = true;
    let is_sticker = bubble
        .css_classes()
        .iter()
        .any(|c| c == "line-bubble-sticker");

    // Keep the "Loading…" label until decode succeeds.
    let max_px = if is_sticker { 128 } else { 320 };
    let animate = is_sticker && state.config.borrow().animations;
    let state2 = state.clone();
    let bubble2 = bubble.clone();
    let message_id2 = message_id.clone();

    let (tx, rx) = async_channel::bounded::<Option<crate::sticker_anim::AnimFrames>>(1);
    let image_path2 = image_path.clone();
    std::thread::spawn(move || {
        let loaded =
            crate::sticker_anim::load_scaled(&image_path2, max_px, animate).or_else(|| {
                // Brief retry — avoids rare races right after atomic rename.
                std::thread::sleep(std::time::Duration::from_millis(30));
                crate::sticker_anim::load_scaled(&image_path2, max_px, animate)
            });
        let _ = tx.send_blocking(loaded);
    });
    glib::spawn_future_local(async move {
        let frames = rx.recv().await.ok().flatten();
        if let Some(frames) = frames {
            let mut child = bubble2.first_child();
            while let Some(w) = child {
                let next_w = w.next_sibling();
                if w.css_classes().iter().any(|c| {
                    c == "line-media-placeholder"
                        || c == "line-media-failed"
                        || c == "line-bubble-image"
                        || c == "line-video-wrap"
                        || c == "line-video-placeholder"
                }) || w.downcast_ref::<gtk::Picture>().is_some()
                    || w.downcast_ref::<gtk::Overlay>().is_some()
                {
                    bubble2.remove(&w);
                }
                child = next_w;
            }
            let pic = make_media_picture_placeholder(is_sticker);
            apply_frames_to_picture(&pic, frames, animate);
            let msg = state2.media_msgs.borrow().get(&message_id2).cloned();
            let is_video = msg
                .as_ref()
                .map(|m| m.content_type.eq_ignore_ascii_case("video"))
                .unwrap_or(false);
            let is_image = msg
                .as_ref()
                .map(|m| m.content_type.eq_ignore_ascii_case("image"))
                .unwrap_or(false);
            if !is_sticker && is_video {
                if let Some(msg) = msg.as_ref() {
                    let overlay = wrap_video_thumb(&state2, &pic, msg);
                    bubble2.append(&overlay);
                } else {
                    bubble2.append(&pic);
                }
            } else {
                bubble2.append(&pic);
                if !is_sticker
                    && is_image
                    && let Some(msg) = msg.as_ref()
                {
                    wire_media_open_click(&state2, &pic, msg, "image");
                    pic.set_tooltip_text(Some(&crate::i18n::t("media_open_image")));
                }
            }
            if *state2.stick_bottom.borrow() {
                scroll_messages_to_end(&state2);
            }
        } else {
            mark_media_failed(&state2, &message_id2);
        }
        *state2.media_pumping.borrow_mut() = false;
        let state3 = state2.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(12), move || {
            pump_media_queue(&state3);
        });
    });
}

fn clear_messages(state: &AppState) {
    stop_voice_playback(state);
    state.message_list.clear();
    state.media_slots.borrow_mut().clear();
    state.media_msgs.borrow_mut().clear();
    state.receipt_slots.borrow_mut().clear();
    state.msg_created.borrow_mut().clear();
    state.pending_rows.borrow_mut().clear();
    *state.last_msg_day.borrow_mut() = None;
    state.seen_msg_ids.borrow_mut().clear();
    *state.last_incoming_id.borrow_mut() = None;
    state.media_queue.borrow_mut().clear();
    *state.media_pumping.borrow_mut() = false;
    *state.new_sep_row.borrow_mut() = None;
    *state.pending_new_below.borrow_mut() = 0;
    state.jump_banner.set_reveal_child(false);
    // Keep media_ready_paths across rebuilds so late slots can still attach.
}

fn message_tracks_media(msg: &MessageInfo) -> bool {
    let ct = msg.content_type.to_ascii_uppercase();
    matches!(
        ct.as_str(),
        "IMAGE" | "VIDEO" | "AUDIO" | "FILE" | "STICKER"
    ) || msg.image_path.is_some()
        || msg.audio_path.is_some()
        || msg.file_path.is_some()
        || msg.flex.is_some()
}

fn set_side_state(state: &AppState, name: &str, empty_text: Option<&str>) {
    if let Some(text) = empty_text {
        state.side_empty.set_text(text);
    }
    if name == "loading" {
        state.side_spinner.set_spinning(true);
        state.side_spinner.set_visible(true);
    }
    state.side_stack.set_visible_child_name(name);
}

fn set_msg_state(state: &AppState, name: &str, empty_text: Option<&str>) {
    if let Some(text) = empty_text {
        state.msg_empty.set_text(text);
    }
    if name == "loading" {
        state.msg_spinner.set_spinning(true);
    }
    state.msg_stack.set_visible_child_name(name);
}

fn preview_body_ui(msg: &MessageInfo) -> String {
    let ct = msg.content_type.to_ascii_uppercase();
    match ct.as_str() {
        "IMAGE" => crate::i18n::t("preview_photo"),
        "VIDEO" => crate::i18n::t("preview_video"),
        "STICKER" => crate::i18n::t("preview_sticker"),
        "AUDIO" => crate::i18n::t("voice_message"),
        "FILE" => crate::i18n::t("preview_file"),
        "FLEX" => {
            let t = msg.text.trim();
            if t.is_empty() {
                crate::i18n::t("preview_flex")
            } else {
                t.to_string()
            }
        }
        _ => {
            let t = msg.text.trim();
            if t.is_empty() {
                if ct.is_empty() {
                    crate::i18n::t("preview_message")
                } else {
                    ct.to_ascii_lowercase()
                }
            } else if t.chars().count() > 64 {
                format!("{}…", t.chars().take(63).collect::<String>())
            } else {
                t.to_string()
            }
        }
    }
}

/// Rewrite cached English (or mismatched) preview prefixes into the active UI language.
fn localize_preview(preview: &str) -> String {
    let mut s = preview.to_string();
    let pairs = [
        ("You: ", "you"),
        ("They: ", "they"),
        ("คุณ: ", "you"),
        ("เขา: ", "they"),
    ];
    for (prefix, key) in pairs {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = format!("{}: {}", crate::i18n::t(key), rest);
            break;
        }
    }
    // Localized media placeholders that may appear after the who-prefix.
    let replacements = [
        ("Photo", "preview_photo"),
        ("Video", "preview_video"),
        ("Sticker", "preview_sticker"),
        ("Voice message", "voice_message"),
        ("File", "preview_file"),
        ("Flex message", "preview_flex"),
        ("Message", "preview_message"),
        ("Tap to open", "tap_open"),
        ("Loading last message…", "loading_last"),
        ("Loading last message...", "loading_last"),
        ("No recent messages", "no_recent"),
        ("รูปภาพ", "preview_photo"),
        ("วิดีโอ", "preview_video"),
        ("สติกเกอร์", "preview_sticker"),
        ("ข้อความเสียง", "voice_message"),
        ("ไฟล์", "preview_file"),
        ("ข้อความ Flex", "preview_flex"),
        ("ข้อความ", "preview_message"),
        ("แตะเพื่อเปิด", "tap_open"),
        ("กำลังโหลดข้อความล่าสุด…", "loading_last"),
        ("ไม่มีข้อความล่าสุด", "no_recent"),
    ];
    if let Some((who, body)) = s.split_once(": ") {
        let mut body = body.to_string();
        for (from, key) in replacements {
            if body == from {
                body = crate::i18n::t(key);
                break;
            }
        }
        return format!("{who}: {body}");
    }
    for (from, key) in replacements {
        if s == from {
            return crate::i18n::t(key);
        }
    }
    s
}

fn snap_adj_to_bottom(adj: &gtk::Adjustment) {
    let target = (adj.upper() - adj.page_size()).max(0.0);
    if (adj.value() - target).abs() > 0.5 {
        adj.set_value(target);
    }
}

fn scroll_last_row_into_view(state: &AppState) {
    state.message_list.scroll_to_end();
    snap_adj_to_bottom(&state.message_scroll.vadjustment());
}

fn scroll_messages_to_end(state: &AppState) {
    if !*state.stick_bottom.borrow() {
        return;
    }
    // Supersede any previous pin chain — overlapping clears were leaving stick=false
    // and stopping one message short of the latest.
    let pin_gen = {
        let mut g = state.scroll_pin_gen.borrow_mut();
        *g = g.saturating_add(1);
        *g
    };
    *state.scroll_pinning.borrow_mut() = true;
    state.message_list.queue_allocate();

    let state_c = state.clone();
    let step = Rc::new(RefCell::new(None::<Rc<dyn Fn(u32)>>));
    let step_fn = {
        let step = step.clone();
        let state_c = state_c.clone();
        Rc::new(move |attempt: u32| {
            if *state_c.scroll_pin_gen.borrow() != pin_gen {
                return;
            }
            if !*state_c.stick_bottom.borrow() {
                *state_c.scroll_pinning.borrow_mut() = false;
                return;
            }
            *state_c.scroll_pinning.borrow_mut() = true;
            scroll_last_row_into_view(&state_c);
            snap_adj_to_bottom(&state_c.message_scroll.vadjustment());

            let adj = state_c.message_scroll.vadjustment();
            let target = (adj.upper() - adj.page_size()).max(0.0);
            let at_bottom = adj.value() + 1.0 >= target;
            // Keep pinning across delayed ListBox allocation (~0.5–0.7s).
            // Require a few consecutive at-bottom frames so we don't stop one row short.
            if attempt >= 48 {
                *state_c.scroll_pinning.borrow_mut() = false;
                if *state_c.stick_bottom.borrow() {
                    *state_c.scroll_pinning.borrow_mut() = true;
                    scroll_last_row_into_view(&state_c);
                    snap_adj_to_bottom(&adj);
                    *state_c.scroll_pinning.borrow_mut() = false;
                }
                return;
            }
            if at_bottom && attempt >= 12 {
                // One more settle pass on the next ticks, then release.
                if attempt >= 18 {
                    *state_c.scroll_pinning.borrow_mut() = false;
                    return;
                }
            }
            let step = step.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(16), move || {
                if let Some(f) = step.borrow().as_ref() {
                    f(attempt + 1);
                }
            });
        }) as Rc<dyn Fn(u32)>
    };
    *step.borrow_mut() = Some(step_fn.clone());
    step_fn(0);
}

fn pin_messages_to_latest(state: &AppState) {
    *state.stick_bottom.borrow_mut() = true;
    scroll_messages_to_end(state);
}

fn state_chat_open(state: &AppState) -> bool {
    state.current_chat.borrow().is_some()
        && state.msg_stack.visible_child_name().as_deref() == Some("list")
}

fn ensure_new_message_separator(state: &AppState) {
    if state.new_sep_row.borrow().is_some() {
        return;
    }
    let sep_row = gtk::ListBoxRow::builder()
        .selectable(false)
        .activatable(false)
        .css_classes(["line-new-sep-row"])
        .build();
    let row_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .css_classes(["line-new-sep"])
        .build();
    let line = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(["line-new-sep-line"])
        .build();
    line.set_height_request(1);
    let badge = gtk::Label::builder()
        .label(crate::i18n::t("new_messages_badge"))
        .css_classes(["line-new-sep-badge"])
        .valign(gtk::Align::Center)
        .build();
    row_box.append(&line);
    row_box.append(&badge);
    sep_row.set_child(Some(&row_box));
    state.message_list.append(&sep_row);
    *state.new_sep_row.borrow_mut() = Some(sep_row);
}

fn dismiss_new_marker(state: &AppState) {
    if let Some(row) = state.new_sep_row.borrow_mut().take() {
        state.message_list.remove(&row);
    }
    *state.pending_new_below.borrow_mut() = 0;
    update_jump_banner(state);
}

fn update_jump_banner(state: &AppState) {
    let n = *state.pending_new_below.borrow();
    let show = n > 0 && !*state.stick_bottom.borrow();
    if show {
        let label = if n == 1 {
            crate::i18n::t("jump_new_one")
        } else {
            crate::i18n::tf("jump_new_n", &[("n", &n.to_string())])
        };
        state.jump_banner_label.set_text(&label);
    }
    state.jump_banner.set_reveal_child(show);
}

fn jump_to_latest(state: &AppState) {
    *state.pending_new_below.borrow_mut() = 0;
    update_jump_banner(state);
    pin_messages_to_latest(state);
    if let Some(mid) = state.current_chat.borrow().clone() {
        mark_chat_read(state, &mid);
        clear_unread_badge(state, &mid);
    }
}

fn format_activity(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let secs = ts / 1000;
    let now = chrono::Utc::now().timestamp();
    let diff = (now - secs).max(0);
    if diff < 60 {
        "now".into()
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else if diff < 86400 * 7 {
        format!("{}d", diff / 86400)
    } else {
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|d| d.format("%m/%d").to_string())
            .unwrap_or_default()
    }
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn msg_id_le(a: &str, b: &str) -> bool {
    match (a.parse::<u128>(), b.parse::<u128>()) {
        (Ok(x), Ok(y)) => x <= y,
        _ => a <= b,
    }
}

fn mark_chat_read(state: &AppState, chat_mid: &str) {
    if !state.config.borrow().auto_mark_read {
        return;
    }
    let Some(last_id) = state.last_incoming_id.borrow().clone() else {
        return;
    };
    if last_id.is_empty() {
        return;
    }
    let _ = state.sidecar.mark_read(chat_mid, &last_id);
}

fn clear_unread_badge(state: &AppState, chat_mid: &str) {
    if let Some(chat) = state
        .chats
        .borrow_mut()
        .iter_mut()
        .find(|c| c.mid == chat_mid)
    {
        chat.unread = 0;
    }
    if let Some(badge) = state.chat_unread_badges.borrow().get(chat_mid) {
        badge.set_text("");
        badge.set_visible(false);
    }
    if state.current_chat.borrow().as_deref() == Some(chat_mid) {
        state
            .chat_subtitle
            .set_text(&crate::i18n::t("conversation"));
    }
    refresh_tray_menu(state);
}

fn bump_unread(state: &AppState, chat_mid: &str) {
    let n = if let Some(chat) = state
        .chats
        .borrow_mut()
        .iter_mut()
        .find(|c| c.mid == chat_mid)
    {
        chat.unread = chat.unread.saturating_add(1);
        chat.unread
    } else {
        1
    };
    if let Some(badge) = state.chat_unread_badges.borrow().get(chat_mid) {
        badge.set_text(&n.to_string());
        badge.set_visible(true);
    }
    refresh_tray_menu(state);
}

fn update_mute_btn(state: &AppState, muted: bool) {
    if muted {
        state.mute_btn.set_icon_name("audio-volume-muted-symbolic");
        state
            .mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("unmute_chat")));
    } else {
        state.mute_btn.set_icon_name("audio-volume-high-symbolic");
        state
            .mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("mute_chat")));
    }
}

fn toggle_chat_mute(state: &AppState) {
    let Some(mid) = state.current_chat.borrow().clone() else {
        return;
    };
    if !mid.starts_with('u') {
        toast(state, &crate::i18n::t("mute_dm_only"));
        return;
    }
    let currently = state
        .chats
        .borrow()
        .iter()
        .find(|c| c.mid == mid)
        .map(|c| c.muted)
        .unwrap_or(false);
    let next = !currently;
    match state.sidecar.mute_chat(&mid, next) {
        Ok(_) => {
            if let Some(chat) = state.chats.borrow_mut().iter_mut().find(|c| c.mid == mid) {
                chat.muted = next;
            }
            update_mute_btn(state, next);
        }
        Err(e) => toast(
            state,
            &crate::i18n::tf("mute_failed", &[("error", &e.to_string())]),
        ),
    }
}

fn apply_peer_read(state: &AppState, chat_mid: &str, message_id: &str) {
    if chat_mid.is_empty() || message_id.is_empty() {
        return;
    }
    let prev = state.read_upto.borrow().get(chat_mid).cloned();
    if let Some(p) = prev.as_deref()
        && msg_id_le(message_id, p)
    {
        return;
    }
    state
        .read_upto
        .borrow_mut()
        .insert(chat_mid.to_string(), message_id.to_string());

    if state.current_chat.borrow().as_deref() != Some(chat_mid) {
        return;
    }
    let times = state.msg_created.borrow().clone();
    for (id, label) in state.receipt_slots.borrow().iter() {
        if msg_id_le(id, message_id) {
            let ts = times.get(id).copied().unwrap_or(0);
            label.set_text(&format_outgoing_status(true, ts));
            label.set_tooltip_text(Some(&crate::i18n::t("status_read")));
            label.add_css_class("line-msg-status-read");
        }
    }
}

fn toast(state: &AppState, msg: &str) {
    // Soften noisy restore / mark-read style failures.
    let lower = msg.to_ascii_lowercase();
    if lower.contains("not_logged_in") || lower.contains("e2eeversion") {
        eprintln!("[soft] {msg}");
        return;
    }
    state.toast_overlay.add_toast(libadwaita::Toast::new(msg));
}

fn peer_display_name(state: &AppState, mid: &str) -> String {
    state
        .chats
        .borrow()
        .iter()
        .find(|c| c.mid == mid)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| mid.to_string())
}

fn refresh_call_controls(state: &AppState) {
    let unlocked = state.config.borrow().experimental_calls;
    if !unlocked {
        state.call_btn.set_visible(false);
        state.call_btn.set_sensitive(false);
        state.call_btn.remove_css_class("line-call-locked");
        state.call_btn.set_tooltip_text(None);
        return;
    }
    state.call_btn.set_visible(true);
    state.call_btn.remove_css_class("line-call-locked");
    let can_call = state
        .current_chat
        .borrow()
        .as_ref()
        .and_then(|mid| {
            state
                .chats
                .borrow()
                .iter()
                .find(|c| &c.mid == mid)
                .map(|c| c.mid.starts_with('u') && c.kind != "bot")
        })
        .unwrap_or(false);
    state.call_btn.set_sensitive(can_call);
    state.call_btn.set_tooltip_text(Some(&if can_call {
        format!(
            "{}: {}",
            crate::i18n::t("voice_call"),
            crate::i18n::t("voice_call_note")
        )
    } else {
        crate::i18n::t("voice_call_unavailable")
    }));
}

fn calls_experimental_enabled(state: &AppState) -> bool {
    state.config.borrow().experimental_calls
}

fn call_error_toast(state: &AppState, err: &str) {
    let msg = if err.contains("relogin_android_required") {
        crate::i18n::t("relogin_android_required")
    } else {
        crate::i18n::tf("call_failed", &[("error", err)])
    };
    toast(state, &msg);
}

fn start_voice_call(state: &AppState) {
    if !calls_experimental_enabled(state) {
        toast(state, &crate::i18n::t("call_experimental_toast"));
        return;
    }
    let Some(mid) = state.current_chat.borrow().clone() else {
        toast(state, &crate::i18n::t("select_chat_first"));
        return;
    };
    if !mid.starts_with('u') {
        toast(state, &crate::i18n::t("voice_call_unavailable"));
        return;
    }
    if state.active_call_peer.borrow().is_some() || state.call_ui.borrow().is_some() {
        toast(state, &crate::i18n::t("call_already_active"));
        return;
    }
    match state.sidecar.call_start(
        &mid,
        &state.config.borrow().audio_input,
        &state.config.borrow().audio_output,
        state.config.borrow().call_mic_volume,
        state.config.borrow().call_spk_volume,
    ) {
        Ok(_) => {
            *state.active_call_peer.borrow_mut() = Some(mid.clone());
            let name = peer_display_name(state, &mid);
            ensure_call_window(
                state,
                &name,
                CallMode::Outgoing(&crate::i18n::tf("call_calling", &[("name", &name)])),
            );
        }
        Err(e) => call_error_toast(state, &e.to_string()),
    }
}

fn answer_voice_call(state: &AppState) {
    if !calls_experimental_enabled(state) {
        toast(state, &crate::i18n::t("call_experimental_toast"));
        decline_voice_call(state);
        return;
    }
    let Some(from) = state.incoming_call_from.borrow().clone() else {
        toast(state, &crate::i18n::t("call_no_incoming"));
        return;
    };
    if state.active_call_peer.borrow().is_some() {
        toast(state, &crate::i18n::t("call_already_active"));
        return;
    }
    match state.sidecar.call_answer(
        &state.config.borrow().audio_input,
        &state.config.borrow().audio_output,
        state.config.borrow().call_mic_volume,
        state.config.borrow().call_spk_volume,
    ) {
        Ok(_) => {
            *state.incoming_call_from.borrow_mut() = None;
            *state.active_call_peer.borrow_mut() = Some(from.clone());
            let name = peer_display_name(state, &from);
            ensure_call_window(
                state,
                &name,
                CallMode::Outgoing(&crate::i18n::tf("call_answering", &[("name", &name)])),
            );
        }
        Err(e) => call_error_toast(state, &e.to_string()),
    }
}

fn decline_voice_call(state: &AppState) {
    let _ = state.sidecar.call_decline();
    *state.incoming_call_from.borrow_mut() = None;
    close_active_call_ui(state);
    toast(state, &crate::i18n::t("call_declined"));
}

fn end_voice_call(state: &AppState) {
    let _ = state.sidecar.call_end();
    *state.incoming_call_from.borrow_mut() = None;
    close_active_call_ui(state);
}

enum CallMode<'a> {
    Outgoing(&'a str),
    Incoming(&'a str),
    Connected(&'a str),
}

fn ensure_call_window(state: &AppState, peer_name: &str, mode: CallMode<'_>) {
    if state.call_ui.borrow().is_none() {
        let (mic_v, spk_v) = {
            let cfg = state.config.borrow();
            (cfg.call_mic_volume, cfg.call_spk_volume)
        };
        let ui = call_window::open_call_window(&state.app, &state.window, peer_name, mic_v, spk_v);
        {
            let s = state.clone();
            ui.hangup_btn.connect_clicked(move |_| end_voice_call(&s));
        }
        {
            let s = state.clone();
            ui.answer_btn
                .connect_clicked(move |_| answer_voice_call(&s));
        }
        {
            let s = state.clone();
            ui.decline_btn
                .connect_clicked(move |_| decline_voice_call(&s));
        }
        {
            let s = state.clone();
            ui.mute_btn.connect_toggled(move |btn| {
                let muted = btn.is_active();
                *s.call_mic_muted.borrow_mut() = muted;
                let _ = s
                    .sidecar
                    .call_set_audio(muted, *s.call_deafened.borrow(), None, None);
                if let Some(ui) = s.call_ui.borrow().as_ref() {
                    call_window::update_mute_visual(ui, muted);
                }
            });
        }
        {
            let s = state.clone();
            ui.deafen_btn.connect_toggled(move |btn| {
                let deafened = btn.is_active();
                *s.call_deafened.borrow_mut() = deafened;
                let _ = s
                    .sidecar
                    .call_set_audio(*s.call_mic_muted.borrow(), deafened, None, None);
                if let Some(ui) = s.call_ui.borrow().as_ref() {
                    call_window::update_deafen_visual(ui, deafened);
                }
            });
        }
        {
            let s = state.clone();
            ui.mic_vol.connect_value_changed(move |scale| {
                let g = scale.value();
                s.config.borrow_mut().call_mic_volume = g;
                s.config.borrow().save(&s.data_dir);
                let _ = s.sidecar.call_set_audio(
                    *s.call_mic_muted.borrow(),
                    *s.call_deafened.borrow(),
                    Some(g),
                    None,
                );
            });
        }
        {
            let s = state.clone();
            ui.spk_vol.connect_value_changed(move |scale| {
                let g = scale.value();
                s.config.borrow_mut().call_spk_volume = g;
                s.config.borrow().save(&s.data_dir);
                let _ = s.sidecar.call_set_audio(
                    *s.call_mic_muted.borrow(),
                    *s.call_deafened.borrow(),
                    None,
                    Some(g),
                );
            });
        }
        {
            let s = state.clone();
            call_window::wire_close_hangup(&ui, move || end_voice_call(&s));
        }
        *state.call_mic_muted.borrow_mut() = false;
        *state.call_deafened.borrow_mut() = false;
        *state.call_ui.borrow_mut() = Some(ui);
    }

    if let Some(ui) = state.call_ui.borrow().as_ref() {
        ui.peer_label.set_text(peer_name);
        match mode {
            CallMode::Outgoing(status) => call_window::set_outgoing_mode(ui, status),
            CallMode::Incoming(status) => call_window::set_incoming_mode(ui, status),
            CallMode::Connected(status) => call_window::set_connected(ui, status),
        }
        ui.window.present();
    }
}

fn close_active_call_ui(state: &AppState) {
    *state.active_call_peer.borrow_mut() = None;
    *state.call_mic_muted.borrow_mut() = false;
    *state.call_deafened.borrow_mut() = false;
    if let Some(ui) = state.call_ui.borrow_mut().take() {
        call_window::close_call_window(&ui);
    }
}

fn handle_call_state(state: &AppState, peer: &str, call_state: &str, error: Option<&str>) {
    if !calls_experimental_enabled(state)
        && matches!(
            call_state,
            "ringing" | "connecting" | "connected" | "acquiring"
        )
    {
        let _ = state.sidecar.call_end();
        close_active_call_ui(state);
        return;
    }
    let name = peer_display_name(state, peer);
    match call_state {
        "acquiring" | "ringing" | "starting" => {
            *state.active_call_peer.borrow_mut() = Some(peer.to_string());
            ensure_call_window(
                state,
                &name,
                CallMode::Outgoing(&crate::i18n::tf("call_calling", &[("name", &name)])),
            );
        }
        "connected" => {
            *state.active_call_peer.borrow_mut() = Some(peer.to_string());
            ensure_call_window(
                state,
                &name,
                CallMode::Connected(&crate::i18n::tf("call_connected", &[("name", &name)])),
            );
        }
        "ended" => {
            close_active_call_ui(state);
            toast(state, &crate::i18n::t("call_ended"));
        }
        "failed" => {
            close_active_call_ui(state);
            let err = error.unwrap_or("call failed");
            // Only nudge QR re-login for actual desktop-session problems — not Opus/media bugs.
            if err.contains("relogin_android_required") || err.contains("DESKTOPWIN") {
                call_error_toast(state, err);
            } else {
                toast(state, &crate::i18n::tf("call_failed", &[("error", err)]));
            }
        }
        _ => {}
    }
}

fn notify_call_incoming(state: &AppState, name: &str) {
    if !notifications_allowed(state) {
        return;
    }
    let n = gio::Notification::new(&crate::i18n::t("voice_call"));
    n.set_body(Some(&crate::i18n::tf("call_incoming", &[("name", name)])));
    n.set_priority(gio::NotificationPriority::Urgent);
    state.app.send_notification(Some("line-gtk-call"), &n);
}
