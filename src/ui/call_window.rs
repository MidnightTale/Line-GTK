use gtk::glib;
use gtk::prelude::*;
use libadwaita::ApplicationWindow;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

pub struct CallUi {
    pub window: ApplicationWindow,
    pub peer_label: gtk::Label,
    pub status_label: gtk::Label,
    pub timer_label: gtk::Label,
    pub mute_btn: gtk::ToggleButton,
    pub deafen_btn: gtk::ToggleButton,
    pub hangup_btn: gtk::Button,
    pub answer_btn: gtk::Button,
    pub decline_btn: gtk::Button,
    pub mic_vol: gtk::Scale,
    pub spk_vol: gtk::Scale,
    pub out_box: gtk::Box,
    pub in_box: gtk::Box,
    pub connected_at: Rc<RefCell<Option<Instant>>>,
    pub tick: Rc<RefCell<Option<glib::SourceId>>>,
    hangup_guard: Rc<RefCell<bool>>,
}

pub fn open_call_window(
    app: &impl IsA<gtk::Application>,
    parent: &ApplicationWindow,
    peer_name: &str,
    mic_vol: f64,
    spk_vol: f64,
) -> CallUi {
    let window = ApplicationWindow::builder()
        .application(app)
        .title(crate::i18n::t("voice_call"))
        .default_width(380)
        .default_height(560)
        .resizable(false)
        .build();
    window.set_transient_for(Some(parent));
    window.set_modal(false);

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-call-window"])
        .build();

    let avatar = gtk::Label::builder()
        .label(
            peer_name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "?".into()),
        )
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-call-avatar"])
        .build();
    avatar.set_size_request(96, 96);

    let peer_label = gtk::Label::builder()
        .label(peer_name)
        .halign(gtk::Align::Center)
        .css_classes(["title-1"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();

    let status_label = gtk::Label::builder()
        .label(crate::i18n::t("call_calling_short"))
        .halign(gtk::Align::Center)
        .css_classes(["dim-label", "title-4"])
        .build();

    let timer_label = gtk::Label::builder()
        .label("00:00")
        .halign(gtk::Align::Center)
        .css_classes(["dim-label", "caption"])
        .visible(false)
        .build();

    let vol_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(8)
        .hexpand(true)
        .width_request(260)
        .build();

    let mic_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .homogeneous(false)
        .css_classes(["line-call-vol-row"])
        .build();
    let mic_lab = gtk::Label::builder()
        .label(crate::i18n::t("call_mic_vol"))
        .xalign(0.0)
        .width_request(64)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption"])
        .build();
    let mic_vol_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 2.0, 0.05);
    mic_vol_scale.set_value(mic_vol.clamp(0.0, 2.5));
    mic_vol_scale.set_hexpand(true);
    mic_vol_scale.set_draw_value(true);
    mic_vol_scale.set_value_pos(gtk::PositionType::Right);
    mic_vol_scale.set_digits(2);
    mic_row.append(&mic_lab);
    mic_row.append(&mic_vol_scale);

    let spk_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .homogeneous(false)
        .css_classes(["line-call-vol-row"])
        .build();
    let spk_lab = gtk::Label::builder()
        .label(crate::i18n::t("call_spk_vol"))
        .xalign(0.0)
        .width_request(64)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption"])
        .build();
    let spk_vol_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 2.0, 0.05);
    spk_vol_scale.set_value(spk_vol.clamp(0.0, 2.5));
    spk_vol_scale.set_hexpand(true);
    spk_vol_scale.set_draw_value(true);
    spk_vol_scale.set_value_pos(gtk::PositionType::Right);
    spk_vol_scale.set_digits(2);
    spk_row.append(&spk_lab);
    spk_row.append(&spk_vol_scale);

    vol_box.append(&mic_row);
    vol_box.append(&spk_row);

    let out_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .halign(gtk::Align::Center)
        .margin_top(16)
        .build();

    let mute_btn = gtk::ToggleButton::builder()
        .icon_name("microphone-sensitivity-high-symbolic")
        .tooltip_text(crate::i18n::t("call_mute_mic"))
        .css_classes(["circular", "line-call-ctl"])
        .build();
    mute_btn.set_size_request(52, 52);

    let deafen_btn = gtk::ToggleButton::builder()
        .icon_name("audio-volume-high-symbolic")
        .tooltip_text(crate::i18n::t("call_deafen"))
        .css_classes(["circular", "line-call-ctl"])
        .build();
    deafen_btn.set_size_request(52, 52);

    let hangup_btn = gtk::Button::builder()
        .icon_name("call-stop-symbolic")
        .tooltip_text(crate::i18n::t("call_hangup"))
        .css_classes(["circular", "destructive-action", "line-call-hangup"])
        .build();
    hangup_btn.set_size_request(64, 64);

    out_box.append(&mute_btn);
    out_box.append(&hangup_btn);
    out_box.append(&deafen_btn);

    let in_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .halign(gtk::Align::Center)
        .margin_top(16)
        .visible(false)
        .build();

    let decline_btn = gtk::Button::builder()
        .icon_name("call-stop-symbolic")
        .tooltip_text(crate::i18n::t("call_decline"))
        .css_classes(["circular", "destructive-action", "line-call-hangup"])
        .build();
    decline_btn.set_size_request(64, 64);

    let answer_btn = gtk::Button::builder()
        .icon_name("call-start-symbolic")
        .tooltip_text(crate::i18n::t("call_answer"))
        .css_classes(["circular", "suggested-action", "line-call-answer"])
        .build();
    answer_btn.set_size_request(64, 64);

    in_box.append(&decline_btn);
    in_box.append(&answer_btn);

    root.append(&avatar);
    root.append(&peer_label);
    root.append(&status_label);
    root.append(&timer_label);
    root.append(&vol_box);
    root.append(&out_box);
    root.append(&in_box);

    window.set_content(Some(&root));
    window.present();

    CallUi {
        window,
        peer_label,
        status_label,
        timer_label,
        mute_btn,
        deafen_btn,
        hangup_btn,
        answer_btn,
        decline_btn,
        mic_vol: mic_vol_scale,
        spk_vol: spk_vol_scale,
        out_box,
        in_box,
        connected_at: Rc::new(RefCell::new(None)),
        tick: Rc::new(RefCell::new(None)),
        hangup_guard: Rc::new(RefCell::new(false)),
    }
}

pub fn set_outgoing_mode(ui: &CallUi, status: &str) {
    ui.status_label.set_text(status);
    ui.out_box.set_visible(true);
    ui.in_box.set_visible(false);
}

pub fn set_incoming_mode(ui: &CallUi, status: &str) {
    ui.status_label.set_text(status);
    ui.out_box.set_visible(false);
    ui.in_box.set_visible(true);
}

pub fn set_connected(ui: &CallUi, status: &str) {
    ui.status_label.set_text(status);
    ui.out_box.set_visible(true);
    ui.in_box.set_visible(false);
    *ui.connected_at.borrow_mut() = Some(Instant::now());
    ui.timer_label.set_visible(true);
    start_timer(ui);
}

fn start_timer(ui: &CallUi) {
    stop_timer(ui);
    let connected_at = ui.connected_at.clone();
    let timer_label = ui.timer_label.clone();
    let id = glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        let Some(started) = *connected_at.borrow() else {
            return glib::ControlFlow::Break;
        };
        let secs = started.elapsed().as_secs();
        timer_label.set_text(&format!("{:02}:{:02}", secs / 60, secs % 60));
        glib::ControlFlow::Continue
    });
    *ui.tick.borrow_mut() = Some(id);
}

pub fn stop_timer(ui: &CallUi) {
    if let Some(id) = ui.tick.borrow_mut().take() {
        id.remove();
    }
}

pub fn close_call_window(ui: &CallUi) {
    *ui.hangup_guard.borrow_mut() = true;
    stop_timer(ui);
    ui.window.close();
}

pub fn update_mute_visual(ui: &CallUi, muted: bool) {
    if ui.mute_btn.is_active() != muted {
        ui.mute_btn.set_active(muted);
    }
    if muted {
        ui.mute_btn
            .set_icon_name("microphone-sensitivity-muted-symbolic");
        ui.mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_unmute_mic")));
    } else {
        ui.mute_btn
            .set_icon_name("microphone-sensitivity-high-symbolic");
        ui.mute_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_mute_mic")));
    }
}

pub fn update_deafen_visual(ui: &CallUi, deafened: bool) {
    if ui.deafen_btn.is_active() != deafened {
        ui.deafen_btn.set_active(deafened);
    }
    if deafened {
        ui.deafen_btn.set_icon_name("audio-volume-muted-symbolic");
        ui.deafen_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_undeafen")));
    } else {
        ui.deafen_btn.set_icon_name("audio-volume-high-symbolic");
        ui.deafen_btn
            .set_tooltip_text(Some(&crate::i18n::t("call_deafen")));
    }
}

pub fn wire_close_hangup(ui: &CallUi, hangup: impl Fn() + 'static) {
    let guard = ui.hangup_guard.clone();
    ui.window.connect_close_request(move |_| {
        if !*guard.borrow() {
            hangup();
        }
        glib::Propagation::Proceed
    });
}
