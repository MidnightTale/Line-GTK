use gtk::prelude::*;

pub struct ShellWidgets {
    pub page: gtk::Box,
    pub chat_list: gtk::ListBox,
    pub message_list: super::virtual_list::VirtualMessageList,
    pub message_scroll: gtk::ScrolledWindow,
    pub composer: gtk::TextView,
    pub composer_placeholder: gtk::Label,
    pub composer_emoji: gtk::Button,
    pub attachment_preview: gtk::Box,
    pub reply_preview: gtk::Revealer,
    pub reply_title: gtk::Label,
    pub reply_text: gtk::Label,
    pub reply_cancel: gtk::Button,
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
    pub read_pause_switch: gtk::Switch,
    pub read_pause_label: gtk::Label,
    pub read_pause_box: gtk::Box,
    pub read_once_btn: gtk::Button,
    pub last_read_btn: gtk::Button,
    pub profile_label: gtk::Label,
    pub profile_avatar: gtk::Picture,
    pub brand_label: gtk::Label,
    pub brand_icon: gtk::Image,
    pub chat_header: gtk::Box,
    pub chat_avatar_frame: gtk::Box,
    pub chat_title: gtk::Label,
    pub chat_subtitle: gtk::Label,
    pub sync_spinner: gtk::Spinner,
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
    pub album_btn: gtk::Button,
    pub message_search_btn: gtk::Button,
    pub shared_media_btn: gtk::Button,
    pub poll_create_btn: gtk::Button,
    pub side_title: gtk::Label,
    pub search_entry: gtk::SearchEntry,
    pub chat_filter_bar: gtk::Box,
    pub chat_filter_btn: gtk::MenuButton,
    pub chat_filter_label: gtk::Label,
    pub compact_filter_btn: gtk::MenuButton,
    pub compact_search_btn: gtk::Button,
    pub compact_search_entry: gtk::SearchEntry,
    pub mic_btn: gtk::Button,
    pub attach_btn: gtk::Button,
    pub sticker_btn: gtk::Button,
    pub call_btn: gtk::Button,
    pub mute_btn: gtk::Button,
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
    // KDE may put the application icon in the window controls. Keep our
    // brand icon only when the desktop layout does not already show one.
    header
        .settings()
        .bind_property("gtk-decoration-layout", &brand_icon, "visible")
        .transform_to(|_, layout: Option<String>| {
            Some(
                !layout.is_some_and(|layout| {
                    layout.split([',', ':']).any(|control| control == "icon")
                }),
            )
        })
        .sync_create()
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

    let read_pause_switch = gtk::Switch::builder().valign(gtk::Align::Center).build();
    let read_pause_label = gtk::Label::builder().css_classes(["caption"]).build();
    let read_pause_box = gtk::Box::builder()
        .spacing(8)
        .valign(gtk::Align::Center)
        .css_classes(["line-read-pause"])
        .build();
    read_pause_box.append(&gtk::Image::from_icon_name("view-conceal-symbolic"));
    read_pause_box.append(&read_pause_label);
    read_pause_box.append(&read_pause_switch);
    header.pack_end(&read_pause_box);

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

    let chat_filter_bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(8)
        .build();
    let filter_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let filter_icon = gtk::Image::from_icon_name("view-filter-symbolic");
    filter_icon.set_accessible_role(gtk::AccessibleRole::Presentation);
    filter_content.append(&filter_icon);
    let chat_filter_label = gtk::Label::builder()
        .label(crate::i18n::t("chat_filter_all"))
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .build();
    filter_content.append(&chat_filter_label);
    let filter_arrow = gtk::Image::from_icon_name("pan-down-symbolic");
    filter_arrow.set_accessible_role(gtk::AccessibleRole::Presentation);
    filter_content.append(&filter_arrow);
    let chat_filter_btn = gtk::MenuButton::builder()
        .child(&filter_content)
        .hexpand(true)
        .css_classes(["flat", "line-chat-filter"])
        .build();
    chat_filter_bar.append(&chat_filter_btn);
    let compact_filter_btn = gtk::MenuButton::builder()
        .icon_name("view-list-symbolic")
        .css_classes(["flat", "circular"])
        .halign(gtk::Align::Center)
        .visible(false)
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
    sidebar.append(&chat_filter_bar);
    sidebar.append(&compact_filter_btn);
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
    // Small sign that the chat is refreshing; the conversation stays usable.
    let sync_spinner = gtk::Spinner::builder()
        .visible(false)
        .valign(gtk::Align::Center)
        .build();
    sync_spinner.set_size_request(10, 10);
    let subtitle_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    subtitle_row.append(&chat_subtitle);
    subtitle_row.append(&sync_spinner);
    title_col.append(&chat_title);
    title_col.append(&subtitle_row);
    let chat_avatar_frame = gtk::Box::builder()
        .valign(gtk::Align::Center)
        .visible(false)
        .css_classes(["line-avatar-frame", "line-avatar-xs"])
        .build();
    let chat_header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["line-chat-header"])
        .build();
    chat_header.append(&chat_avatar_frame);
    chat_header.append(&title_col);
    let chat_bar_spacer = gtk::Box::builder().hexpand(true).build();
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
    chat_bar.append(&chat_header);
    chat_bar.append(&chat_bar_spacer);
    let read_once_btn = gtk::Button::builder()
        .label(crate::i18n::t("mark_read_once"))
        .tooltip_text(crate::i18n::t("mark_read_once_tooltip"))
        .css_classes(["flat"])
        .visible(false)
        .build();
    chat_bar.append(&read_once_btn);
    let last_read_btn = gtk::Button::builder()
        .icon_name("go-jump-symbolic")
        .tooltip_text(crate::i18n::t("jump_last_read"))
        .css_classes(["flat", "circular"])
        .visible(false)
        .build();
    chat_bar.append(&last_read_btn);
    let album_btn = gtk::Button::builder()
        .icon_name("folder-pictures-symbolic")
        .label(crate::i18n::t("albums"))
        .tooltip_text(crate::i18n::t("albums"))
        .css_classes(["flat"])
        .build();
    chat_bar.append(&album_btn);
    let message_search_btn = gtk::Button::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text(crate::i18n::t("message_search"))
        .css_classes(["flat", "circular"])
        .build();
    chat_bar.append(&message_search_btn);
    let shared_media_btn = gtk::Button::builder()
        .label(crate::i18n::t("shared_media"))
        .sensitive(false)
        .css_classes(["flat"])
        .build();
    let conversation_menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(crate::i18n::t("shared_media"))
        .css_classes(["flat"])
        .build();
    let poll_create_btn = gtk::Button::builder()
        .label(crate::i18n::t("poll_create"))
        .sensitive(false)
        .css_classes(["flat"])
        .build();
    let menu_items = gtk::Box::new(gtk::Orientation::Vertical, 2);
    menu_items.append(&shared_media_btn);
    menu_items.append(&poll_create_btn);
    let menu = gtk::Popover::new();
    menu.set_child(Some(&menu_items));
    conversation_menu.set_popover(Some(&menu));
    for button in [&shared_media_btn, &poll_create_btn] {
        let popover = menu.clone();
        button.connect_clicked(move |_| popover.popdown());
    }
    chat_bar.append(&conversation_menu);
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
    let composer = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(7)
        .bottom_margin(7)
        .hexpand(true)
        .css_classes(["line-composer-text"])
        .build();
    let composer_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(34)
        .max_content_height(160)
        .propagate_natural_height(true)
        .hexpand(true)
        .child(&composer)
        .build();
    let composer_placeholder = gtk::Label::builder()
        .label(crate::i18n::t("type_message"))
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .margin_top(7)
        .can_target(false)
        .css_classes(["dim-label"])
        .build();
    let composer_overlay = gtk::Overlay::new();
    composer_overlay.set_child(Some(&composer_scroll));
    composer_overlay.add_overlay(&composer_placeholder);
    let placeholder = composer_placeholder.clone();
    composer.buffer().connect_changed(move |buffer| {
        placeholder.set_visible(buffer.char_count() == 0);
    });
    let composer_emoji = gtk::Button::builder()
        .icon_name("face-smile-symbolic")
        .tooltip_text(crate::i18n::t("insert_emoji"))
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    let weak = composer.downgrade();
    composer_emoji.connect_clicked(move |_| {
        if let Some(composer) = weak.upgrade() {
            composer.grab_focus();
            composer.emit_insert_emoji();
        }
    });
    let composer_field = gtk::Box::builder()
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(["entry", "line-composer-entry"])
        .build();
    composer_field.append(&composer_overlay);
    composer_field.append(&composer_emoji);
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
    composer_row.append(&composer_field);
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
    let reply_title = gtk::Label::builder()
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(40)
        .xalign(0.0)
        .css_classes(["heading", "caption"])
        .build();
    let reply_text = gtk::Label::builder()
        .xalign(0.0)
        .single_line_mode(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .hexpand(true)
        .build();
    let reply_column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();
    reply_column.append(&reply_title);
    reply_column.append(&reply_text);
    let reply_cancel = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular", "line-reply-cancel"])
        .build();
    let reply_bar = gtk::Box::builder()
        .spacing(10)
        .margin_start(12)
        .margin_end(12)
        .margin_top(4)
        .margin_bottom(4)
        .css_classes(["line-reply-preview"])
        .build();
    let reply_icon = gtk::Image::builder()
        .icon_name("mail-reply-sender-symbolic")
        .pixel_size(20)
        .css_classes(["line-reply-preview-icon"])
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    reply_bar.append(&reply_icon);
    reply_bar.append(&reply_column);
    reply_bar.append(&reply_cancel);
    let reply_preview = gtk::Revealer::builder()
        .child(&reply_bar)
        .reveal_child(false)
        .build();
    conversation.append(&reply_preview);
    let attachment_preview = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_start(12)
        .margin_end(12)
        .margin_top(4)
        .visible(false)
        .build();
    conversation.append(&attachment_preview);
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
        read_once_btn,
        last_read_btn,
        page,
        chat_list,
        message_list,
        message_scroll,
        composer,
        composer_placeholder,
        composer_emoji,
        attachment_preview,
        reply_preview,
        reply_title,
        reply_text,
        reply_cancel,
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
        read_pause_switch,
        read_pause_label,
        read_pause_box,
        profile_label,
        profile_avatar,
        brand_label,
        brand_icon,
        chat_header,
        chat_avatar_frame,
        chat_title,
        chat_subtitle,
        sync_spinner,
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
        album_btn,
        message_search_btn,
        shared_media_btn,
        poll_create_btn,
        side_title,
        search_entry,
        chat_filter_bar,
        chat_filter_btn,
        chat_filter_label,
        compact_filter_btn,
        compact_search_btn,
        compact_search_entry,
        mic_btn,
        attach_btn,
        sticker_btn,
        call_btn,
        mute_btn,
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
.line-read-pause { padding: 4px 8px; border-radius: 10px; }
.line-read-pause.line-read-paused { background: alpha(@accent_bg_color, 0.16); color: @accent_color; }
.line-chat-read-paused, .line-chat-pinned, .line-chat-blocked { background: @window_bg_color; color: @accent_color; border-radius: 999px; padding: 2px; }
.line-reply-quote-button { padding: 0; border-radius: 8px; }
.line-reply-quote { border-left: 3px solid @accent_bg_color; padding: 8px 10px; background: alpha(@accent_bg_color, 0.10); border-radius: 8px; }
.line-reply-author, .line-reply-preview-icon { color: @accent_color; }
.line-reply-excerpt { font-size: 0.9em; }
.line-reply-preview { border-left: 3px solid @accent_bg_color; padding: 10px 12px; background: alpha(@accent_bg_color, 0.08); border-radius: 10px; }
.line-reply-cancel { min-width: 32px; min-height: 32px; padding: 0; }
.line-message-actions { border-radius: 999px; background: alpha(@card_bg_color, 0.65); border: 1px solid alpha(@borders, 0.45); padding: 2px; }
.line-message-row .line-message-actions { opacity: 0; transition: opacity 120ms ease-in-out; }
.line-message-row:hover .line-message-actions,
.line-message-row:focus-within .line-message-actions { opacity: 1; }
.line-message-action { min-width: 30px; min-height: 30px; padding: 0; color: @window_fg_color; }
.line-message-action:hover, .line-message-action:focus-visible { color: @accent_color; background: alpha(@accent_bg_color, 0.12); }
.line-album-photo { border-radius: 12px; padding: 4px; }
.line-emoji-text, .line-emoji-text text { background: transparent; color: inherit; padding: 0; }
.line-reaction-strip { margin: 0; }
.line-reaction-chip { min-height: 24px; padding: 0 4px; border-radius: 6px; background: transparent; border: none; box-shadow: none; font-size: 0.85em; }
.line-reaction-chip:hover { background: alpha(@accent_bg_color, 0.12); }
.line-reaction-count { font-weight: 600; }
.line-reaction-chip.line-reaction-mine { background: alpha(@accent_bg_color, 0.18); }
.line-reaction-details { min-width: 24px; min-height: 24px; padding: 0; border: none; box-shadow: none; }
.line-reaction-tab { min-width: 36px; min-height: 36px; padding: 2px; border-radius: 8px; border-bottom: 2px solid transparent; }
.line-reaction-tab.line-reaction-selected { background: alpha(@accent_bg_color, 0.16); border-bottom-color: @accent_bg_color; }
.line-reaction-option { min-width: 40px; min-height: 40px; padding: 2px; border-radius: 10px; }
.line-reaction-option.line-reaction-selected { background: alpha(@accent_bg_color, 0.24); }
.line-reaction-check { color: @accent_fg_color; background: @accent_bg_color; border-radius: 999px; padding: 1px; }

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
        .line-chat-header-link {
            border-radius: 12px;
            padding: 2px 8px 2px 4px;
        }
        .line-chat-header-link:hover { background: alpha(@window_fg_color, 0.07); }
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
            overflow: hidden;
            min-width: 26px;
            min-height: 26px;
            background: alpha(@borders, 0.35);
        }
        .line-title-name {
            font-weight: 600;
        }
        .line-avatar-frame {
            border-radius: 999px;
            overflow: hidden;
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
        .line-chat-filter > button { padding: 8px 10px; border-radius: 10px; }
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
            overflow: hidden;
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
        .line-history-card {
            min-width: 196px;
            padding: 6px 2px;
        }
        .line-history-icon {
            background: alpha(@accent_bg_color, 0.14);
            color: @accent_color;
            border-radius: 999px;
            padding: 12px;
        }
        .line-history-title { font-weight: 700; }
        .line-history-duration {
            opacity: 0.8;
            font-feature-settings: "tnum";
        }
        .line-history-missed .line-history-icon,
        .line-history-missed .line-history-title { color: @error_color; }
        .line-history-missed .line-history-icon {
            background: alpha(@error_color, 0.10);
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
        .line-media-viewer-bar {
            background: alpha(@headerbar_bg_color, 0.96);
            color: @headerbar_fg_color;
            padding: 8px 12px;
            border-bottom: 1px solid alpha(@borders, 0.45);
        }
        .line-media-viewer-bar button {
            min-width: 30px;
            min-height: 30px;
            border-radius: 8px;
        }
        .line-media-viewer-bar button.flat {
            color: inherit;
        }
        .line-media-viewer-visual,
        .line-media-viewer-visual .line-media-viewer-body,
        .line-media-viewer-visual .line-media-viewer-image,
        .line-media-viewer-visual .line-media-viewer-video {
            background: #14171d;
            color: #f5f7fb;
        }
        .line-media-viewer-visual .line-media-viewer-bar {
            background: #242832;
            color: #f5f7fb;
            border-bottom: 1px solid #353b46;
        }
        .line-media-viewer-visual button.flat:hover {
            background: rgba(255, 255, 255, 0.10);
        }
        .line-media-viewer-visual button.flat:checked {
            background: #3b4960;
        }
        .line-media-viewer-zoom {
            background: rgba(0, 0, 0, 0.20);
            border-radius: 10px;
            padding: 2px;
        }
        .line-media-draw-tools {
            background: #1d222b;
            color: #f5f7fb;
            padding: 10px 16px;
            border-bottom: 1px solid #353b46;
        }
        .line-media-draw-tools button.flat {
            color: #f5f7fb;
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
            padding: 0;
            border: 1px solid rgba(255, 255, 255, 0.35);
            box-shadow: none;
        }
        .line-draw-swatch:checked {
            outline: 2px solid #99bdff;
            outline-offset: 2px;
        }
        .line-draw-c-red { background: #e53935; }
        .line-draw-c-green { background: #43a047; }
        .line-draw-c-blue { background: #1e88e5; }
        .line-draw-c-yellow { background: #fdd835; }
        .line-draw-c-white { background: #f5f5f5; }
        .line-draw-c-black { background: #212121; }
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
        .line-chat-preview { opacity: 0.72; font-size: 0.85em; }
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
        .line-poll-choice progressbar trough, .line-poll-choice progressbar progress { min-height: 4px; }
        .line-link-preview { border-radius: 10px; padding: 8px; background: alpha(@card_bg_color, 0.75); border: 1px solid alpha(@borders, 0.5); max-width: 360px; }
        .line-link-preview-thumbnail { padding: 0; margin: 0; border-radius: 6px; }
        .line-link-preview-title { font-weight: 700; }
        .line-link-preview-description { font-size: 0.9em; opacity: 0.8; }
        .line-link-preview-domain { font-size: 0.8em; opacity: 0.7; }
        .line-link-preview-actions { margin-top: 4px; }
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
        .line-msg-list row, .line-message-row {
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
            font-size: 0.78em;
            opacity: 0.65;
            margin-top: 1px;
        }
        .line-msg-status-read {
            opacity: 1;
            font-weight: 600;
            color: @accent_color;
        }
        .line-message-sender {
            font-size: 0.82em;
            font-weight: 600;
            color: alpha(@window_fg_color, 0.72);
        }
        .line-message-sender-link:hover { text-decoration: underline; }
        .line-avatar-xs,
        .line-avatar-xs.line-avatar,
        .line-avatar-frame.line-avatar-xs {
            min-width: 32px;
            min-height: 32px;
        }
        .line-msg-time {
            font-size: 0.70em;
            opacity: 0.55;
            margin-bottom: 4px;
            margin-left: 4px;
            margin-right: 4px;
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
        .line-reply-target {
            background-color: alpha(@accent_bg_color, 0.18);
            border-radius: 12px;
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
        .line-composer-text, .line-composer-text text {
            background: transparent;
        }
        .line-composer-entry:focus-within {
            outline: 2px solid @accent_color;
            outline-offset: -2px;
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
        .line-sticker-hover-stage {
            background: #eef0f3;
            border-radius: 12px;
            padding: 8px;
        }
        .line-sticker-hover-stage label {
            color: #33363b;
            padding: 16px;
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
