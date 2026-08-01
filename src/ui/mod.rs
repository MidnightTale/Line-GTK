mod call_window;
mod calls;
mod chats;
mod composer_media;
mod diagnostics;
mod downloads;
mod events;
mod friends;
mod login;
mod media;
mod messages;
mod notifications;
mod settings;
mod shell;
mod state;
mod virtual_list;

use crate::config::{AppConfig, apply_animations, apply_font, apply_theme};
use crate::protocol::{ChatInfo, FlexAction, MessageInfo, Profile, ProtocolEvent};
use crate::sidecar::Sidecar;
use anyhow::Result;
use calls::*;
use chats::*;
use composer_media::*;
use events::*;
use gdk_pixbuf::Pixbuf;
use gtk::gdk;
use gtk::prelude::*;
use gtk::{gio, glib};
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow};
use media::*;
use messages::*;
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
