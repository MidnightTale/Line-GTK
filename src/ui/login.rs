use anyhow::{Result, anyhow};
use gdk_pixbuf::Pixbuf;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use qrcode::EcLevel;
use qrcode::QrCode;
use std::cell::{Cell, RefCell};
use std::io::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMethod {
    Qr,
    Email,
}

pub struct LoginWidgets {
    /// Full-bleed root (owns the page gradient).
    pub page: gtk::Box,
    pub stage: gtk::Stack,
    pub qr_picture: gtk::Picture,
    pub pin_label: gtk::Label,
    pub hint: gtk::Label,
    pub retry_btn: gtk::Button,
    pub subtitle: gtk::Label,
    pub pin_caption: gtk::Label,
    pub note: gtk::Label,
    pub qr_method_btn: gtk::Button,
    pub email_method_btn: gtk::Button,
    pub email_entry: gtk::Entry,
    pub password_entry: gtk::PasswordEntry,
    pub email_submit_btn: gtk::Button,
    pub password_note: gtk::Label,
    pub spinner: gtk::Spinner,
    pub method: Cell<LoginMethod>,
    pub busy: Cell<bool>,
    pub error: RefCell<Option<String>>,
}

pub fn build_login_page() -> LoginWidgets {
    // Root fills the window so the gradient covers the whole page.
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .css_classes(["line-login-page"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .margin_top(40)
        .margin_bottom(40)
        .margin_start(40)
        .margin_end(40)
        .css_classes(["line-login-content"])
        .build();

    let title = gtk::Label::builder()
        .label("LINE GTK")
        .css_classes(["title-1", "line-login-brand"])
        .build();

    let subtitle = gtk::Label::builder()
        .label(crate::i18n::t("login_subtitle"))
        .css_classes(["dim-label", "line-login-subtitle"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .visible(false)
        .build();

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-login-card"])
        .build();

    let stage = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vhomogeneous(false)
        .hhomogeneous(false)
        .build();

    let methods = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::Center)
        .build();
    let qr_method_btn = gtk::Button::with_label(&crate::i18n::t("login_method_qr"));
    let email_method_btn = gtk::Button::with_label(&crate::i18n::t("login_method_email"));
    qr_method_btn.add_css_class("suggested-action");
    methods.append(&qr_method_btn);
    methods.append(&email_method_btn);

    let email_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .width_request(300)
        .build();
    let email_entry = gtk::Entry::builder()
        .placeholder_text(crate::i18n::t("login_email_address"))
        .input_purpose(gtk::InputPurpose::Email)
        .tooltip_text(crate::i18n::t("login_email_address"))
        .build();
    let password_entry = gtk::PasswordEntry::builder()
        .placeholder_text(crate::i18n::t("login_password"))
        .tooltip_text(crate::i18n::t("login_password"))
        .show_peek_icon(true)
        .build();
    let email_submit_btn = gtk::Button::builder()
        .label(crate::i18n::t("login_email_submit"))
        .css_classes(["suggested-action", "pill"])
        .build();
    let password_note = gtk::Label::builder()
        .label(crate::i18n::t("login_password_note"))
        .wrap(true)
        .max_width_chars(32)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label", "caption"])
        .build();
    let spinner = gtk::Spinner::builder()
        .halign(gtk::Align::Center)
        .visible(false)
        .build();
    email_box.append(&email_entry);
    email_box.append(&password_entry);
    email_box.append(&email_submit_btn);
    email_box.append(&spinner);
    email_box.append(&password_note);
    stage.add_named(&email_box, Some("email"));

    let qr_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();

    let qr_picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .width_request(280)
        .height_request(280)
        .css_classes(["line-login-qr"])
        .build();
    qr_box.append(&qr_picture);
    stage.add_named(&qr_box, Some("qr"));

    let pin_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .width_request(280)
        .height_request(280)
        .build();

    let pin_caption = gtk::Label::builder()
        .label(crate::i18n::t("login_pin_caption"))
        .css_classes(["title-3"])
        .justify(gtk::Justification::Center)
        .wrap(true)
        .build();

    let pin_label = gtk::Label::builder()
        .label("----")
        .justify(gtk::Justification::Center)
        .selectable(true)
        .build();
    pin_label.set_markup("<span size='92000' weight='heavy' letter_spacing='8000'>----</span>");

    pin_box.append(&pin_caption);
    pin_box.append(&pin_label);
    stage.add_named(&pin_box, Some("pin"));
    stage.set_visible_child_name("qr");

    card.append(&stage);

    let hint = gtk::Label::builder()
        .label(crate::i18n::t("login_qr_hint"))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(40)
        .css_classes(["line-login-hint"])
        .build();

    let retry_btn = gtk::Button::builder()
        .label(crate::i18n::t("login_retry"))
        .css_classes(["pill", "suggested-action"])
        .halign(gtk::Align::Center)
        .visible(false)
        .build();

    let note = gtk::Label::builder()
        .label(crate::i18n::t("login_note"))
        .css_classes(["caption", "dim-label"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(48)
        .build();

    content.append(&title);
    content.append(&subtitle);
    content.append(&methods);
    content.append(&card);
    content.append(&hint);
    content.append(&retry_btn);
    content.append(&note);
    page.append(&content);

    LoginWidgets {
        page,
        stage,
        qr_picture,
        pin_label,
        hint,
        retry_btn,
        subtitle,
        pin_caption,
        note,
        qr_method_btn,
        email_method_btn,
        email_entry,
        password_entry,
        email_submit_btn,
        password_note,
        spinner,
        method: Cell::new(LoginMethod::Qr),
        busy: Cell::new(false),
        error: RefCell::new(None),
    }
}

pub fn apply_login_language(w: &LoginWidgets) {
    w.subtitle.set_text(&crate::i18n::t("login_subtitle"));
    w.hint.set_text(&method_hint(w));
    w.pin_caption.set_text(&crate::i18n::t("login_pin_caption"));
    w.retry_btn
        .set_label(&crate::i18n::t(if w.method.get() == LoginMethod::Email {
            "login_email_retry"
        } else {
            "login_retry"
        }));
    w.note.set_text(&crate::i18n::t("login_note"));
    w.qr_method_btn
        .set_label(&crate::i18n::t("login_method_qr"));
    w.email_method_btn
        .set_label(&crate::i18n::t("login_method_email"));
    w.email_entry
        .set_placeholder_text(Some(&crate::i18n::t("login_email_address")));
    w.email_entry
        .set_tooltip_text(Some(&crate::i18n::t("login_email_address")));
    w.password_entry
        .set_placeholder_text(Some(&crate::i18n::t("login_password")));
    w.password_entry
        .set_tooltip_text(Some(&crate::i18n::t("login_password")));
    w.email_submit_btn
        .set_label(&crate::i18n::t(if w.busy.get() {
            "login_email_signing_in"
        } else {
            "login_email_submit"
        }));
    w.password_note
        .set_text(&crate::i18n::t("login_password_note"));
}

pub fn method_hint(w: &LoginWidgets) -> String {
    w.error.borrow().clone().unwrap_or_else(|| {
        crate::i18n::t(if w.method.get() == LoginMethod::Email {
            "login_email_hint"
        } else {
            "login_qr_hint"
        })
    })
}

pub fn set_method(w: &LoginWidgets, method: LoginMethod) {
    w.method.set(method);
    w.password_entry.set_text("");
    *w.error.borrow_mut() = None;
    w.qr_method_btn.remove_css_class("suggested-action");
    w.email_method_btn.remove_css_class("suggested-action");
    match method {
        LoginMethod::Qr => w.qr_method_btn.add_css_class("suggested-action"),
        LoginMethod::Email => w.email_method_btn.add_css_class("suggested-action"),
    }
    set_busy(w, false);
    show_method_stage(w);
    apply_login_language(w);
}

pub fn set_busy(w: &LoginWidgets, busy: bool) {
    w.busy.set(busy);
    w.email_entry.set_sensitive(!busy);
    w.password_entry.set_sensitive(!busy);
    w.email_submit_btn.set_sensitive(!busy);
    w.email_submit_btn.set_label(&crate::i18n::t(if busy {
        "login_email_signing_in"
    } else {
        "login_email_submit"
    }));
    w.spinner.set_visible(busy);
    w.spinner.set_spinning(busy);
}

pub fn show_method_stage(w: &LoginWidgets) {
    match w.method.get() {
        LoginMethod::Qr => show_qr_stage(w),
        LoginMethod::Email => {
            w.stage.set_visible_child_name("email");
            w.retry_btn.set_visible(false);
            w.hint.set_text(&method_hint(w));
        }
    }
}

pub fn show_failure(w: &LoginWidgets, message: &str) {
    *w.error.borrow_mut() = Some(message.into());
    w.password_entry.set_text("");
    set_busy(w, false);
    show_method_stage(w);
    w.hint.set_text(message);
    w.retry_btn.set_visible(w.method.get() == LoginMethod::Qr);
}

pub fn show_qr_stage(w: &LoginWidgets) {
    w.stage.set_visible_child_name("qr");
    w.retry_btn.set_visible(false);
    w.hint.set_text(&crate::i18n::t("login_qr_hint"));
    w.pin_label
        .set_markup("<span size='92000' weight='heavy' letter_spacing='8000'>----</span>");
}

pub fn show_pin_stage(w: &LoginWidgets, pin: &str) {
    let safe = glib::markup_escape_text(pin);
    w.pin_label.set_markup(&format!(
        "<span size='92000' weight='heavy' letter_spacing='8000'>{safe}</span>"
    ));
    w.stage.set_visible_child_name("pin");
    w.retry_btn.set_visible(true);
    w.retry_btn
        .set_label(&crate::i18n::t(if w.method.get() == LoginMethod::Email {
            "login_email_retry"
        } else {
            "login_retry"
        }));
    w.hint.set_text(&crate::i18n::t("login_pin_waiting"));
}

pub fn set_qr(picture: &gtk::Picture, url: &str) -> Result<()> {
    let code = QrCode::with_error_correction_level(url.as_bytes(), EcLevel::M)
        .map_err(|e| anyhow!("qr encode: {e}"))?;
    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(280, 280)
        .build();

    let mut png = Vec::new();
    image::DynamicImage::ImageLuma8(image)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| anyhow!("png encode: {e}"))?;

    let bytes = gtk::glib::Bytes::from(&png[..]);
    let stream = gtk::gio::MemoryInputStream::from_bytes(&bytes);
    let pixbuf = Pixbuf::from_stream(&stream, gtk::gio::Cancellable::NONE)
        .map_err(|e| anyhow!("pixbuf: {e}"))?;
    let texture = gdk::Texture::for_pixbuf(&pixbuf);
    picture.set_paintable(Some(&texture));
    Ok(())
}
