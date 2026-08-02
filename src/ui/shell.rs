use gtk::prelude::*;

pub struct ShellWidgets {
    pub page: gtk::Box,
    pub chat_list: gtk::ListBox,
    pub message_list: super::virtual_list::VirtualMessageList,
    pub message_scroll: gtk::ScrolledWindow,
    pub composer: gtk::Entry,
    pub composer_row: gtk::Box,
    pub composer_stack: gtk::Stack,
    pub record_cancel_btn: gtk::Button,
    pub record_send_btn: gtk::Button,
    pub record_timer: gtk::Label,
    pub record_wave: gtk::DrawingArea,
    pub upload_revealer: gtk::Revealer,
    pub upload_bar: gtk::ProgressBar,
    pub upload_label: gtk::Label,
    pub conversation: gtk::Box,
    pub send_btn: gtk::Button,
    pub status: gtk::Label,
    pub profile_label: gtk::Label,
    pub profile_avatar: gtk::Picture,
    pub brand_label: gtk::Label,
    pub brand_icon: gtk::Image,
    pub chat_title: gtk::Label,
    pub chat_subtitle: gtk::Label,
    pub side_stack: gtk::Stack,
    pub side_spinner: gtk::Spinner,
    pub side_empty: gtk::Label,
    pub side_load_label: gtk::Label,
    pub msg_stack: gtk::Stack,
    pub msg_spinner: gtk::Spinner,
    pub msg_empty: gtk::Label,
    pub msg_load_label: gtk::Label,
    pub msg_idle: gtk::Label,
    pub settings_btn: gtk::Button,
    pub friends_btn: gtk::Button,
    pub side_title: gtk::Label,
    pub search_entry: gtk::SearchEntry,
    pub compact_search_btn: gtk::Button,
    pub compact_search_entry: gtk::SearchEntry,
    pub mic_btn: gtk::Button,
    pub attach_btn: gtk::Button,
    pub sticker_btn: gtk::Button,
    pub call_btn: gtk::Button,
    pub mute_btn: gtk::Button,
    pub pin_btn: gtk::Button,
    pub album_btn: gtk::Button,
    pub jump_banner: gtk::Revealer,
    pub jump_banner_btn: gtk::Button,
    pub jump_banner_label: gtk::Label,
    pub sticker_popover: gtk::Popover,
    pub sidebar: gtk::Box,
    pub sidebar_paned: gtk::Paned,
    pub side_header: gtk::Box,
}

pub fn build_shell_page() -> ShellWidgets {
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-shell"])
        .build();

    let header = libadwaita::HeaderBar::new();

    let brand_icon = gtk::Image::builder()
        .icon_name("line-gtk")
        .pixel_size(22)
        .css_classes(["line-app-brand-icon"])
        .build();
    let brand_label = gtk::Label::builder()
        .label("LINE GTK")
        .css_classes(["heading", "line-app-brand-label"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let brand = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .valign(gtk::Align::Center)
        .css_classes(["line-app-brand"])
        .build();
    brand.append(&brand_icon);
    brand.append(&brand_label);
    header.pack_start(&brand);

    let profile_avatar = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(26)
        .height_request(26)
        .visible(false)
        .css_classes(["line-title-avatar"])
        .build();
    profile_avatar.set_overflow(gtk::Overflow::Hidden);
    let profile_label = gtk::Label::builder()
        .label("LINE GTK")
        .css_classes(["heading", "line-title-name"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let profile_title = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .css_classes(["line-title-profile"])
        .build();
    profile_title.append(&profile_avatar);
    profile_title.append(&profile_label);
    header.set_title_widget(Some(&profile_title));

    let status = gtk::Label::builder()
        .label("…")
        .css_classes(["dim-label", "caption"])
        .margin_end(10)
        .build();
    header.pack_end(&status);

    let settings_btn = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Settings")
        .css_classes(["flat"])
        .build();
    let friends_btn = gtk::Button::builder()
        .icon_name("system-users-symbolic")
        .tooltip_text("Friends")
        .css_classes(["flat"])
        .build();
    header.pack_end(&settings_btn);
    header.pack_end(&friends_btn);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .vexpand(true)
        .build();

    // ---- Sidebar ----
    let sidebar = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(false)
        .vexpand(true)
        .css_classes(["line-sidebar"])
        .build();
    sidebar.set_size_request(80, -1);

    let side_header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(16)
        .margin_end(16)
        .margin_top(12)
        .margin_bottom(8)
        .css_classes(["line-side-header"])
        .build();
    let side_title = gtk::Label::builder()
        .label(crate::i18n::t("chats"))
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["title-3", "line-side-title"])
        .build();
    let side_spinner = gtk::Spinner::new();
    side_header.append(&side_title);
    side_header.append(&side_spinner);

    let compact_search_btn = gtk::Button::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Search")
        .css_classes(["flat", "circular", "line-compact-search-btn"])
        .halign(gtk::Align::Center)
        .margin_top(8)
        .margin_bottom(4)
        .visible(false)
        .build();
    let compact_search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search")
        .width_request(200)
        .build();
    let compact_search_popover = gtk::Popover::builder()
        .autohide(true)
        .has_arrow(true)
        .position(gtk::PositionType::Bottom)
        .build();
    compact_search_popover.set_child(Some(&compact_search_entry));
    compact_search_popover.set_parent(&compact_search_btn);
    {
        let pop = compact_search_popover.clone();
        let entry = compact_search_entry.clone();
        compact_search_btn.connect_clicked(move |_| {
            pop.popup();
            entry.grab_focus();
        });
    }

    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search")
        .hexpand(true)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(8)
        .css_classes(["line-search"])
        .build();

    let side_stack = gtk::Stack::builder()
        .vexpand(true)
        .hexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(160)
        .build();

    let chat_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .build();
    let chat_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["line-chat-list"])
        .vexpand(true)
        .build();
    chat_scroll.set_child(Some(&chat_list));

    let side_loading = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    let side_load_spin = gtk::Spinner::builder().spinning(true).build();
    side_load_spin.set_size_request(28, 28);
    let side_load_label = gtk::Label::builder()
        .label(crate::i18n::t("loading_chats"))
        .css_classes(["dim-label"])
        .build();
    side_loading.append(&side_load_spin);
    side_loading.append(&side_load_label);

    let side_empty = gtk::Label::builder()
        .label(crate::i18n::t("no_chats"))
        .css_classes(["dim-label", "title-4"])
        .justify(gtk::Justification::Center)
        .build();

    side_stack.add_named(&side_loading, Some("loading"));
    side_stack.add_named(&chat_scroll, Some("list"));
    side_stack.add_named(&side_empty, Some("empty"));
    side_stack.set_visible_child_name("loading");

    sidebar.append(&side_header);
    sidebar.append(&compact_search_btn);
    sidebar.append(&search_entry);
    sidebar.append(&side_stack);

    // ---- Conversation ----
    let conversation = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-conversation"])
        .build();

    let chat_bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(16)
        .margin_end(16)
        .margin_top(12)
        .margin_bottom(12)
        .css_classes(["line-chat-bar"])
        .build();
    let title_col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    let chat_title = gtk::Label::builder()
        .label(crate::i18n::t("select_chat"))
        .xalign(0.0)
        .css_classes(["title-3"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let chat_subtitle = gtk::Label::builder()
        .label(crate::i18n::t("pick_chat"))
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build();
    title_col.append(&chat_title);
    title_col.append(&chat_subtitle);
    let call_btn = gtk::Button::builder()
        .icon_name("call-start-symbolic")
        .tooltip_text("Voice call")
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .visible(false)
        .build();
    let mute_btn = gtk::Button::builder()
        .icon_name("audio-volume-high-symbolic")
        .tooltip_text("Mute chat")
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .build();
    let pin_btn = gtk::Button::builder()
        .icon_name("non-starred-symbolic")
        .tooltip_text("Pin chat")
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .build();
    let album_btn = gtk::Button::builder()
        .icon_name("folder-pictures-symbolic")
        .tooltip_text("Chat album")
        .css_classes(["flat", "circular"])
        .sensitive(false)
        .build();
    chat_bar.append(&title_col);
    chat_bar.append(&album_btn);
    chat_bar.append(&pin_btn);
    chat_bar.append(&mute_btn);
    chat_bar.append(&call_btn);

    let msg_stack = gtk::Stack::builder()
        .vexpand(true)
        .hexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(160)
        .build();

    let message_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .css_classes(["line-msg-scroll"])
        .build();
    let message_list = super::virtual_list::VirtualMessageList::new();
    message_scroll.set_child(Some(message_list.widget()));

    let msg_loading = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    let msg_spinner = gtk::Spinner::builder().spinning(true).build();
    msg_spinner.set_size_request(32, 32);
    let msg_load_label = gtk::Label::builder()
        .label(crate::i18n::t("loading_messages"))
        .css_classes(["dim-label"])
        .build();
    msg_loading.append(&msg_spinner);
    msg_loading.append(&msg_load_label);

    let msg_empty = gtk::Label::builder()
        .label(crate::i18n::t("no_messages"))
        .css_classes(["dim-label", "title-4"])
        .justify(gtk::Justification::Center)
        .build();

    let msg_idle = gtk::Label::builder()
        .label(crate::i18n::t("select_chat_start"))
        .css_classes(["dim-label", "title-4"])
        .justify(gtk::Justification::Center)
        .build();

    msg_stack.add_named(&msg_idle, Some("idle"));
    msg_stack.add_named(&msg_loading, Some("loading"));
    msg_stack.add_named(&message_scroll, Some("list"));
    msg_stack.add_named(&msg_empty, Some("empty"));
    msg_stack.set_visible_child_name("idle");

    let jump_banner = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .transition_duration(180)
        .reveal_child(false)
        .css_classes(["line-jump-banner-revealer"])
        .build();
    let jump_banner_btn = gtk::Button::builder()
        .css_classes(["pill", "line-jump-banner"])
        .halign(gtk::Align::Center)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let jump_inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::Center)
        .build();
    let jump_banner_label = gtk::Label::builder()
        .label(crate::i18n::t("new_messages_badge"))
        .css_classes(["caption"])
        .build();
    let jump_icon = gtk::Image::from_icon_name("go-down-symbolic");
    jump_inner.append(&jump_banner_label);
    jump_inner.append(&jump_icon);
    jump_banner_btn.set_child(Some(&jump_inner));
    jump_banner.set_child(Some(&jump_banner_btn));

    let composer_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .valign(gtk::Align::Center)
        .css_classes(["line-composer"])
        .build();
    let composer = gtk::Entry::builder()
        .placeholder_text("Type a message…")
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(["line-composer-entry"])
        .build();
    let attach_btn = gtk::Button::builder()
        .icon_name("mail-attachment-symbolic")
        .tooltip_text("Attach file")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular", "line-composer-icon"])
        .build();
    let sticker_btn = gtk::Button::builder()
        .icon_name("face-smile-symbolic")
        .tooltip_text("Stickers")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular", "line-composer-icon"])
        .build();
    let mic_btn = gtk::Button::builder()
        .icon_name("audio-input-microphone-symbolic")
        .tooltip_text("Voice message")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular", "line-composer-icon"])
        .build();
    let send_btn = gtk::Button::builder()
        .label(crate::i18n::t("send"))
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill", "line-send-btn"])
        .build();
    composer_row.append(&attach_btn);
    composer_row.append(&sticker_btn);
    composer_row.append(&mic_btn);
    composer_row.append(&composer);
    composer_row.append(&send_btn);

    // Recording overlay: cancel | live wave | timer | send (replaces composer; no toast)
    let record_bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .valign(gtk::Align::Center)
        .css_classes(["line-composer", "line-record-bar"])
        .build();
    let record_cancel_btn = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Cancel recording")
        .valign(gtk::Align::Center)
        .css_classes([
            "flat",
            "circular",
            "destructive-action",
            "line-record-cancel",
        ])
        .build();
    let record_dot = gtk::Label::builder()
        .label("●")
        .valign(gtk::Align::Center)
        .css_classes(["line-record-dot"])
        .build();
    let record_wave = gtk::DrawingArea::builder()
        .hexpand(true)
        .hexpand_set(true)
        .content_width(240)
        .content_height(28)
        .valign(gtk::Align::Center)
        .css_classes(["line-record-wave"])
        .build();
    let record_timer = gtk::Label::builder()
        .label("0:00")
        .valign(gtk::Align::Center)
        .css_classes(["line-record-timer"])
        .width_chars(5)
        .build();
    let record_send_btn = gtk::Button::builder()
        .icon_name("mail-send-symbolic")
        .tooltip_text("Send voice message")
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action", "circular", "line-record-send"])
        .build();
    record_bar.append(&record_cancel_btn);
    record_bar.append(&record_dot);
    record_bar.append(&record_wave);
    record_bar.append(&record_timer);
    record_bar.append(&record_send_btn);

    let composer_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .vhomogeneous(false)
        .hhomogeneous(true)
        .margin_start(10)
        .margin_end(10)
        .margin_top(4)
        .margin_bottom(6)
        .css_classes(["line-composer-stack"])
        .build();
    composer_stack.add_named(&composer_row, Some("compose"));
    composer_stack.add_named(&record_bar, Some("record"));
    composer_stack.set_visible_child_name("compose");

    let upload_label = gtk::Label::builder()
        .label("")
        .xalign(0.0)
        .css_classes(["caption", "line-upload-label"])
        .build();
    let upload_bar = gtk::ProgressBar::builder()
        .show_text(false)
        .fraction(0.0)
        .hexpand(true)
        .css_classes(["line-upload-bar"])
        .build();
    let upload_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_start(12)
        .margin_end(12)
        .margin_top(4)
        .css_classes(["line-upload-box"])
        .build();
    upload_box.append(&upload_label);
    upload_box.append(&upload_bar);
    let upload_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .reveal_child(false)
        .child(&upload_box)
        .css_classes(["line-upload-revealer"])
        .build();

    let sticker_popover = gtk::Popover::builder()
        .autohide(true)
        .has_arrow(true)
        .position(gtk::PositionType::Top)
        .css_classes(["line-sticker-popover"])
        .build();
    sticker_popover.set_parent(&sticker_btn);

    conversation.append(&chat_bar);
    conversation.append(&msg_stack);
    conversation.append(&jump_banner);
    conversation.append(&upload_revealer);
    conversation.append(&composer_stack);

    let sidebar_paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .vexpand(true)
        .wide_handle(false)
        .resize_start_child(false)
        .shrink_start_child(false)
        .resize_end_child(true)
        .shrink_end_child(true)
        .css_classes(["line-sidebar-paned"])
        .build();
    sidebar_paned.set_start_child(Some(&sidebar));
    sidebar_paned.set_end_child(Some(&conversation));
    sidebar_paned.set_position(320);

    body.append(&sidebar_paned);
    page.append(&header);
    page.append(&body);

    ShellWidgets {
        page,
        chat_list,
        message_list,
        message_scroll,
        composer,
        composer_row,
        composer_stack,
        record_cancel_btn,
        record_send_btn,
        record_timer,
        record_wave,
        upload_revealer,
        upload_bar,
        upload_label,
        conversation,
        send_btn,
        status,
        profile_label,
        profile_avatar,
        brand_label,
        brand_icon,
        chat_title,
        chat_subtitle,
        side_stack,
        side_spinner,
        side_empty,
        side_load_label,
        msg_stack,
        msg_spinner,
        msg_empty,
        msg_load_label,
        msg_idle,
        settings_btn,
        friends_btn,
        side_title,
        search_entry,
        compact_search_btn,
        compact_search_entry,
        mic_btn,
        attach_btn,
        sticker_btn,
        call_btn,
        mute_btn,
        pin_btn,
        album_btn,
        jump_banner,
        jump_banner_btn,
        jump_banner_label,
        sticker_popover,
        sidebar,
        sidebar_paned,
        side_header,
    }
}

pub fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        r#"
        .line-shell {
            background: @window_bg_color;
        }
        .line-app-brand {
            margin-start: 4px;
        }
        .line-app-brand-label {
            font-weight: 600;
        }
        .line-app-brand-icon {
            margin-end: 0;
        }
        .line-sidebar {
            background: alpha(@window_bg_color, 0.96);
            min-width: 80px;
        }
        .line-sidebar-paned > separator {
            background: alpha(@borders, 0.40);
            min-width: 1px;
            min-height: 1px;
        }

        /* Compact sidebar: select the avatar frame, not a full-width pill. */
        .line-sidebar-compact .line-chat-list row {
            margin: 2px 0;
            border-radius: 0;
            padding: 0;
            background: transparent;
        }
        .line-sidebar-compact .line-chat-list row:hover,
        .line-sidebar-compact .line-chat-list row:selected {
            box-shadow: none;
            background: transparent;
        }
        .line-sidebar-compact .line-chat-list row:hover .line-avatar-frame {
            box-shadow: 0 0 0 2px alpha(@accent_bg_color, 0.55);
        }
        .line-sidebar-compact .line-chat-list row:selected .line-avatar-frame {
            box-shadow: 0 0 0 2px @accent_bg_color;
            background: alpha(@accent_bg_color, 0.28);
        }
        .line-sidebar-compact .line-chat-row {
            margin: 0;
            padding: 0;
        }
        .line-sidebar-compact .line-avatar-overlay {
            margin: 0;
        }
        .line-sidebar-compact .line-avatar,
        .line-sidebar-compact .line-avatar-frame {
            min-width: 36px;
            min-height: 36px;
        }

        .line-conversation {
            background:
              linear-gradient(
                180deg,
                alpha(@accent_bg_color, 0.05) 0%,
                transparent 120px
              ),
              @window_bg_color;
        }
        .line-chat-bar {
            background: transparent;
            border-bottom: 1px solid alpha(@borders, 0.35);
        }
        .line-chat-list { background: transparent; }
        .line-chat-list row {
            margin: 2px 8px;
            border-radius: 12px;
            padding: 0;
        }
        .line-chat-list row:selected {
            background: alpha(@accent_bg_color, 0.28);
            box-shadow: none;
        }
        .line-chat-list row:selected:hover {
            background: alpha(@accent_bg_color, 0.34);
        }
        .line-chat-list row:hover {
            background: alpha(@accent_bg_color, 0.14);
        }

        /* Avatar tiers: list/chat 48 default; friends override to 40; compact 36. */
        .line-title-profile {
            padding: 0 4px;
        }
        .line-title-avatar {
            border-radius: 999px;
            min-width: 26px;
            min-height: 26px;
            background: alpha(@borders, 0.35);
        }
        .line-title-name {
            font-weight: 600;
        }
        .line-avatar-frame {
            border-radius: 999px;
            min-width: 48px;
            min-height: 48px;
            background: alpha(@borders, 0.35);
            box-shadow: 0 1px 2px alpha(@window_fg_color, 0.12);
        }
        .line-avatar {
            border-radius: 999px;
        }
        .line-avatar-sm,
        .line-avatar-sm.line-avatar,
        .line-avatar-frame.line-avatar-sm {
            min-width: 40px;
            min-height: 40px;
        }
        .line-search {
            border-radius: 12px;
            margin-bottom: 4px;
        }
        .line-call-locked {
            opacity: 0.38;
        }
        .line-call-window {
            background: @window_bg_color;
        }
        .line-call-avatar {
            border-radius: 999px;
            min-width: 96px;
            min-height: 96px;
            background: alpha(@accent_bg_color, 0.35);
            font-size: 2.4em;
            font-weight: 700;
        }
        .line-call-ctl {
            min-width: 52px;
            min-height: 52px;
        }
        .line-call-hangup, .line-call-answer {
            min-width: 64px;
            min-height: 64px;
        }
        .line-call-vol-row {
            margin-top: 4px;
        }
        .line-voice-card {
            min-width: 180px;
            max-width: min(340px, 78%);
            padding: 2px 0;
        }
        .line-voice-play {
            min-width: 36px;
            min-height: 36px;
        }
        .line-voice-play.playing {
            color: @accent_color;
        }
        .line-voice-wave {
            min-width: 120px;
            min-height: 32px;
        }
        .line-voice-wave.playing {
            opacity: 1;
        }
        .line-voice-dur {
            opacity: 0.85;
            font-size: 0.85em;
            font-feature-settings: "tnum";
        }
        .line-upload-revealer {
            padding-bottom: 2px;
        }
        .line-upload-box {
            padding: 6px 4px 2px 4px;
        }
        .line-upload-label {
            opacity: 0.85;
        }
        .line-upload-bar {
            min-height: 6px;
        }
        .line-video-thumb {
            border-radius: 12px;
        }
        .line-video-play-badge {
            min-width: 48px;
            min-height: 48px;
            border-radius: 999px;
            padding: 10px;
            background: alpha(@window_fg_color, 0.55);
            color: @window_bg_color;
        }
        .line-video-placeholder {
            min-width: 180px;
            min-height: 110px;
            border-radius: 12px;
            background: alpha(@accent_bg_color, 0.18);
            padding: 16px;
        }
        .line-file-card {
            padding: 6px 4px;
            border-radius: 10px;
            min-width: 180px;
        }
        .line-file-card:hover {
            background: alpha(@accent_bg_color, 0.14);
        }
        .line-media-viewer-root {
            background: @window_bg_color;
        }
        .line-media-review-window,
        .line-media-review {
            background: @window_bg_color;
        }
        .line-media-review-grid {
            padding: 16px;
            background: @view_bg_color;
        }
        .line-media-review-card {
            padding: 8px;
            border-radius: 12px;
            background: alpha(@card_bg_color, 0.96);
            border: 1px solid alpha(@borders, 0.45);
        }
        .line-media-review-preview {
            border-radius: 9px;
            background: alpha(@borders, 0.20);
        }
        .line-media-review-thumb {
            border-radius: 9px;
        }
        .line-media-review-file-icon {
            padding: 32px;
        }
        .line-media-review-remove {
            min-width: 30px;
            min-height: 30px;
            margin: 6px;
            color: @window_bg_color;
            background: alpha(@window_fg_color, 0.76);
            box-shadow: 0 1px 3px alpha(@window_fg_color, 0.28);
        }
        .line-media-review-remove:hover {
            background: alpha(@window_fg_color, 0.92);
        }
        .line-media-review-footer {
            border-top: 1px solid alpha(@borders, 0.38);
        }
        .line-media-viewer-bar {
            background: alpha(@headerbar_bg_color, 0.96);
            border-bottom: 1px solid alpha(@borders, 0.45);
        }
        .line-media-viewer-body {
            background: @window_bg_color;
            padding: 0;
        }
        .line-media-viewer-scroll {
            background: @window_bg_color;
        }
        .line-media-viewer-image {
            background: @window_bg_color;
        }
        .line-draw-swatch {
            min-width: 28px;
            min-height: 28px;
            font-size: 1.1em;
            padding: 0;
        }
        .line-draw-c-red { color: #e53935; }
        .line-draw-c-green { color: #43a047; }
        .line-draw-c-blue { color: #1e88e5; }
        .line-draw-c-yellow { color: #fdd835; }
        .line-draw-c-white { color: #f5f5f5; }
        .line-draw-c-black { color: #212121; }
        .line-media-viewer-video {
            background: @window_bg_color;
        }
        .line-media-viewer-doc {
            background: alpha(@card_bg_color, 0.92);
        }
        .line-media-viewer-pdf-preview {
            min-height: 280px;
            background: @view_bg_color;
            border-radius: 8px;
        }
        .line-media-viewer-text-scroll {
            background: @card_bg_color;
            border-radius: 8px;
        }
        .line-media-viewer-text {
            background: @card_bg_color;
            color: @view_fg_color;
            font-size: 0.95em;
        }
        .line-media-viewer-audio, .line-media-viewer-generic {
            padding: 24px;
        }
        .line-msg-pending, .line-msg-pending-row {
            opacity: 0.45;
        }
        .line-bubble-pending {
            opacity: 0.92;
        }
        .line-record-bar {
            min-height: 0;
            padding: 4px 8px;
            border-radius: 16px;
            background: alpha(@destructive_bg_color, 0.14);
            border: 1px solid alpha(@destructive_bg_color, 0.35);
        }
        .line-record-dot {
            color: @destructive_bg_color;
            font-size: 0.85em;
            padding: 0 2px;
        }
        .line-record-timer {
            font-weight: 700;
            font-feature-settings: "tnum";
            min-width: 3.2em;
            color: @destructive_bg_color;
        }
        .line-record-wave {
            min-height: 28px;
            min-width: 120px;
            border-radius: 10px;
            background: alpha(@window_bg_color, 0.55);
        }
        .line-record-send {
            min-width: 42px;
            min-height: 42px;
        }
        .line-record-cancel {
            min-width: 42px;
            min-height: 42px;
        }
        .line-chat-name { font-weight: 650; }
        .line-chat-pinned { color: @accent_color; opacity: 0.9; }
        .line-chat-preview { opacity: 0.72; font-size: 0.85em; }
        .line-album-thumb-btn {
            padding: 0;
            border-radius: 14px;
        }
        .line-album-thumb {
            min-width: 160px;
            min-height: 160px;
            border-radius: 14px;
            background: alpha(@borders, 0.22);
        }
        .line-bubble-image {
            border-radius: 12px;
            max-width: min(420px, 72%);
        }
        .line-bubble-sticker {
            background: transparent;
            background-color: transparent;
            border: none;
            box-shadow: none;
            padding: 0;
            margin: 0;
            max-width: min(160px, 56%);
        }
        .line-bubble-sticker .line-bubble-image,
        .line-sticker-image {
            background: transparent;
            background-color: transparent;
            border: none;
            box-shadow: none;
            border-radius: 0;
            max-width: 160px;
        }
        .line-flex-card {
            border-radius: 12px;
            padding: 8px 12px;
            background: alpha(@card_bg_color, 0.98);
            border: 1px solid alpha(@borders, 0.55);
            min-width: 180px;
            max-width: min(420px, 72%);
        }
        .line-flex-title { font-weight: 700; }
        .line-flex-action { margin-top: 4px; }
        .line-link-chip { margin-top: 4px; }
        .line-unread {
            background: @accent_bg_color;
            color: @accent_fg_color;
            border-radius: 999px;
            min-width: 18px;
            padding: 1px 6px;
            font-size: 0.75em;
            font-weight: 700;
        }
        .line-unread-overlay {
            margin: 0 0 2px 2px;
            min-width: 16px;
            padding: 0 4px;
            font-size: 0.7em;
        }
        .line-msg-scroll {
            background: transparent;
        }
        .line-msg-list { background: transparent; }
        .line-msg-list row {
            background: transparent;
            margin: 2px 4px;
            padding: 0;
        }
        .line-bubble {
            border-radius: 16px;
            padding: 8px 12px;
            box-shadow: 0 1px 2px alpha(@window_fg_color, 0.08);
            max-width: min(480px, 72%);
        }
        .line-bubble-in {
            background: alpha(@card_bg_color, 0.97);
            border: 1px solid alpha(@borders, 0.42);
        }
        .line-bubble-out {
            background: alpha(@accent_bg_color, 0.42);
            border: 1px solid alpha(@accent_bg_color, 0.18);
        }
        .line-bubble-text { font-size: 0.98em; }
        .line-msg-status {
            font-size: 0.72em;
            opacity: 0.65;
            margin-top: 1px;
        }
        .line-msg-status-read {
            opacity: 0.95;
            color: @accent_color;
        }
        .line-msg-time {
            font-size: 0.70em;
            opacity: 0.55;
            margin-bottom: 4px;
            margin-left: 4px;
            margin-right: 4px;
        }
        .line-message-reactions {
            margin-top: 2px;
        }
        .line-message-reaction-chip {
            min-height: 24px;
            padding: 1px 7px;
            border-radius: 999px;
            font-size: 0.78em;
            background: alpha(@card_bg_color, 0.78);
            border: 1px solid alpha(@borders, 0.42);
        }
        .line-message-reaction-mine {
            color: @accent_color;
            background: alpha(@accent_bg_color, 0.18);
            border-color: alpha(@accent_bg_color, 0.48);
        }
        .line-message-reaction-choice {
            min-width: 38px;
            min-height: 38px;
            padding: 2px;
            font-size: 1.2em;
        }
        .line-message-reaction-choice:hover {
            background: alpha(@accent_bg_color, 0.16);
        }
        .line-message-unsend {
            color: @destructive_color;
        }
        .line-msg-sender-name {
            font-size: 0.78em;
            font-weight: 650;
            opacity: 0.78;
            margin-left: 6px;
        }
        .line-msg-avatar-btn {
            min-width: 38px;
            min-height: 38px;
            padding: 2px;
            margin-top: 18px;
        }
        .line-msg-avatar,
        .line-sender-profile-avatar {
            border-radius: 999px;
            background: alpha(@borders, 0.25);
        }
        .line-msg-avatar-btn:hover {
            background: alpha(@accent_bg_color, 0.16);
        }
        .line-sender-profile {
            min-width: 280px;
        }
        .line-sender-profile-name {
            font-weight: 750;
        }
        .line-sender-profile-status {
            padding: 4px 12px;
        }
        .line-day-sep-row {
            margin-top: 12px;
            margin-bottom: 8px;
        }
        .line-day-sep {
            font-size: 0.78em;
            opacity: 0.7;
            padding: 4px 12px;
            border-radius: 999px;
            background: alpha(@borders, 0.35);
        }
        .line-new-sep-row {
            margin-top: 12px;
            margin-bottom: 8px;
            padding: 0 2px;
        }
        .line-new-sep-line {
            min-height: 1px;
            background: alpha(@borders, 0.55);
        }
        .line-new-sep-badge {
            font-size: 0.72em;
            font-weight: 700;
            letter-spacing: 0.02em;
            padding: 2px 10px;
            border-radius: 999px;
            background: alpha(@accent_bg_color, 0.28);
            color: @accent_color;
            margin-left: 8px;
        }
        .line-jump-banner {
            padding: 4px 16px;
            min-height: 28px;
            font-weight: 600;
        }
        .line-jump-banner-revealer {
            padding-top: 2px;
        }
        .line-friend-list {
            background: transparent;
        }
        .line-friend-list row {
            border-radius: 12px;
            margin: 2px 8px;
        }
        .line-friend-list row:hover {
            background: alpha(@accent_bg_color, 0.12);
        }
        .line-friend-section-row {
            margin-top: 8px;
            margin-bottom: 0;
            background: transparent;
        }
        .line-friend-section-row:hover {
            background: transparent;
        }
        .line-friend-section {
            font-size: 0.78em;
            font-weight: 700;
            letter-spacing: 0.04em;
            opacity: 0.72;
            padding: 8px 16px 2px 16px;
            color: @accent_color;
        }
        .line-friend-add {
            background: alpha(@card_bg_color, 0.45);
            border-top: 1px solid alpha(@borders, 0.35);
            padding-top: 4px;
        }
        .line-composer {
            background: alpha(@window_bg_color, 0.94);
            border-top: 1px solid alpha(@borders, 0.4);
            padding: 0;
            min-height: 0;
        }
        .line-composer-stack {
            min-height: 0;
        }
        .line-composer-icon {
            min-width: 34px;
            min-height: 34px;
            padding: 0;
        }
        .line-composer-entry {
            border-radius: 18px;
            min-height: 34px;
            padding-top: 0;
            padding-bottom: 0;
            padding-left: 12px;
            padding-right: 12px;
        }
        .line-send-btn {
            min-height: 34px;
            padding: 0 14px;
        }
        .line-composer-narrow {
            spacing: 4px;
        }
        .line-composer-narrow .line-send-btn {
            min-width: 34px;
            min-height: 32px;
            padding: 0 6px;
        }
        .line-composer-narrow .line-composer-entry {
            min-height: 32px;
            padding-left: 10px;
            padding-right: 10px;
        }
        .line-sticker-popover {
            padding: 0;
        }
        .line-sticker-chooser {
            min-width: 280px;
            min-height: 320px;
        }
        .line-sticker-pack-title {
            font-size: 0.95em;
            font-weight: 650;
        }
        .line-sticker-pages {
            min-height: 240px;
        }
        .line-sticker-scroll {
            min-width: 260px;
            min-height: 220px;
            padding: 8px;
        }
        .line-sticker-grid {
            margin: 2px;
        }
        .line-sticker-cell {
            padding: 8px;
            border-radius: 12px;
        }
        .line-sticker-cell:hover {
            background: alpha(@accent_bg_color, 0.18);
        }
        .line-sticker-thumb {
            min-width: 64px;
            min-height: 64px;
        }
        .line-sticker-tabs-scroll {
            min-height: 52px;
            background: alpha(@card_bg_color, 0.55);
        }
        .line-sticker-tabs {
            padding: 2px 0;
        }
        .line-sticker-tab {
            min-width: 40px;
            min-height: 40px;
            padding: 4px;
            border-radius: 12px;
        }
        .line-sticker-tab:checked {
            background: alpha(@accent_bg_color, 0.28);
            box-shadow: inset 0 -2px 0 @accent_bg_color;
        }
        .line-sticker-tab-icon {
            min-width: 28px;
            min-height: 28px;
        }
        .line-login-page {
            background:
              linear-gradient(
                165deg,
                alpha(@accent_bg_color, 0.28) 0%,
                alpha(@accent_bg_color, 0.10) 28%,
                alpha(@window_bg_color, 0.92) 58%,
                @window_bg_color 100%
              );
        }
        .line-login-content {
            background: transparent;
        }
        .line-login-brand {
            letter-spacing: 0.02em;
        }
        .line-login-hint {
            font-size: 1.05em;
            opacity: 0.9;
        }
        .line-login-card {
            background: alpha(@card_bg_color, 0.92);
            border: 1px solid alpha(@borders, 0.45);
            border-radius: 16px;
            padding: 16px;
            box-shadow: 0 2px 8px alpha(@window_fg_color, 0.12);
        }
        .line-login-qr {
            border-radius: 8px;
        }
        "#,
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
