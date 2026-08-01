use anyhow::{anyhow, Result};
use gdk_pixbuf::Pixbuf;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use qrcode::EcLevel;
use qrcode::QrCode;
use std::io::Cursor;

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
        .label("QR code with LINE mobile")
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
        .vhomogeneous(true)
        .hhomogeneous(true)
        .build();

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
        .label("Enter this PIN on your phone")
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
        .label("QR code with LINE mobile")
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(40)
        .css_classes(["line-login-hint"])
        .build();

    let retry_btn = gtk::Button::builder()
        .label("Retry QR login")
        .css_classes(["pill", "suggested-action"])
        .halign(gtk::Align::Center)
        .visible(false)
        .build();

    let note = gtk::Label::builder()
        .label("Unofficial. LINE ToS risk applies. Prefer a secondary session.")
        .css_classes(["caption", "dim-label"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(48)
        .build();

    content.append(&title);
    content.append(&subtitle);
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
    }
}

pub fn apply_login_language(w: &LoginWidgets) {
    w.subtitle.set_text(&crate::i18n::t("login_subtitle"));
    w.hint.set_text(&crate::i18n::t("login_qr_hint"));
    w.pin_caption.set_text(&crate::i18n::t("login_pin_caption"));
    w.retry_btn.set_label(&crate::i18n::t("login_retry"));
    w.note.set_text(&crate::i18n::t("login_note"));
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
