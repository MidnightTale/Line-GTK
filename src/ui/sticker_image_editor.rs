use super::*;
use crate::sticker_image::{ImageOptions, ImageSize};

fn send_prepared(
    state: &AppState,
    image: &crate::sticker_image::PreparedImage,
    source: &Source,
) -> bool {
    let result = (|| -> anyhow::Result<PathBuf> {
        let directory = state.data_dir.join("cache/media");
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("sticker-image-{}.png", now_ms()));
        crate::sticker_image::save(image, &path)?;
        Ok(path)
    })();
    match result {
        Ok(path) => {
            if source.optimized {
                sticker_library::send_optimized(state, source, Some(path));
                true
            } else {
                try_send_local_media_path(state, path)
            }
        }
        Err(error) => {
            toast(
                state,
                &crate::i18n::tf("media_send_failed", &[("error", &error.to_string())]),
            );
            false
        }
    }
}

#[derive(Clone)]
pub(super) struct Source {
    pub path: PathBuf,
    pub key: String,
    pub personal: bool,
    pub animated: bool,
    pub optimized: bool,
    pub optimization: serde_json::Value,
}

impl Source {
    fn options(&self, state: &AppState) -> ImageOptions {
        let fallback = if self.personal {
            ImageOptions {
                size: ImageSize::Original,
                remove_background: false,
            }
        } else {
            ImageOptions::default()
        };
        crate::sticker_image::options_for_key(&state.data_dir, &self.key, fallback)
    }
}

pub(super) fn send_last(state: &AppState, source: &Source) {
    if source.animated {
        sticker_library::send_optimized(state, source, None);
        return;
    }
    let path = &source.path;
    let options = source.options(state);
    if options.size == ImageSize::Original && !options.remove_background {
        if source.optimized {
            sticker_library::send_optimized(state, source, None);
        } else {
            try_send_local_media_path(state, path.clone());
        }
        return;
    }
    match crate::sticker_image::prepare(path, options.size.pixels(), options.remove_background) {
        Ok(image) => {
            send_prepared(state, &image, source);
        }
        Err(error) => toast(
            state,
            &crate::i18n::tf("media_send_failed", &[("error", &error.to_string())]),
        ),
    }
}

pub(super) fn optimization_input(
    state: &AppState,
    source: &Source,
) -> anyhow::Result<Option<PathBuf>> {
    if source.animated {
        return Ok(None);
    }
    let options = source.options(state);
    if options.size == ImageSize::Original && !options.remove_background {
        return Ok(None);
    }
    let image = crate::sticker_image::prepare(
        &source.path,
        options.size.pixels(),
        options.remove_background,
    )?;
    let directory = state.data_dir.join("cache/media");
    crate::config::ensure_private_dir(&directory)?;
    let path = directory.join(format!("sticker-image-{}.png", now_ms()));
    crate::sticker_image::save(&image, &path)?;
    Ok(Some(path))
}

pub(super) fn open(state: &AppState, source: Source) {
    if source.animated {
        sticker_library::optimize_dialog(state, source);
        return;
    }
    let scope = composer_drafts::scope(state);
    let options = source.options(state);
    // An editor keeps its own snapshot so library removal cannot invalidate it.
    let path = match copy_into_media_cache(state, &source.path) {
        Ok(path) => path,
        Err(error) => {
            toast(state, &error.to_string());
            return;
        }
    };
    let dialog = libadwaita::Dialog::builder()
        .title(crate::i18n::t("sticker_send_as_image"))
        .content_width(400)
        .content_height(560)
        .build();
    let toolbar = libadwaita::ToolbarView::new();
    toolbar.add_top_bar(&libadwaita::HeaderBar::new());
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(18)
        .margin_end(18)
        .margin_top(12)
        .margin_bottom(18)
        .build();
    let preview = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .vexpand(true)
        .build();
    preview.set_size_request(240, 240);
    preview.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "sticker_image_preview",
    ))]);
    // A checkerboard makes remaining transparency visible before sending.
    let checkerboard = gtk::DrawingArea::builder()
        .vexpand(true)
        .hexpand(true)
        .build();
    checkerboard.set_draw_func(|_, cr, width, height| {
        for y in (0..height).step_by(12) {
            for x in (0..width).step_by(12) {
                let shade = if (x / 12 + y / 12) % 2 == 0 {
                    1.0
                } else {
                    0.85
                };
                cr.set_source_rgb(shade, shade, shade);
                cr.rectangle(f64::from(x), f64::from(y), 12.0, 12.0);
                let _ = cr.fill();
            }
        }
    });
    let overlay = gtk::Overlay::builder().child(&checkerboard).build();
    overlay.add_overlay(&preview);
    overlay.set_measure_overlay(&preview, true);
    let frame = gtk::Frame::builder().child(&overlay).build();
    body.append(&frame);
    let size_label = gtk::Label::builder()
        .label(crate::i18n::t("sticker_image_size"))
        .xalign(0.0)
        .build();
    let sizes: Vec<String> = [
        "sticker_image_small",
        "sticker_image_medium",
        "sticker_image_large",
        "sticker_image_original",
    ]
    .iter()
    .map(|key| crate::i18n::t(key))
    .collect();
    let size = gtk::DropDown::from_strings(&sizes.iter().map(String::as_str).collect::<Vec<_>>());
    size.set_selected(options.size.selection());
    size.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "sticker_image_size",
    ))]);
    size_label.set_mnemonic_widget(Some(&size));
    body.append(&size_label);
    body.append(&size);
    let remove = gtk::CheckButton::with_label(&crate::i18n::t("sticker_image_remove_bg"));
    remove.set_active(options.remove_background);
    body.append(&remove);
    let hint = gtk::Label::builder()
        .label(crate::i18n::t("sticker_image_edit_hint"))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["caption", "dim-label"])
        .build();
    body.append(&hint);
    let status = gtk::Label::builder().xalign(0.0).wrap(true).build();
    body.append(&status);
    let send = gtk::Button::builder()
        .label(crate::i18n::t("send"))
        .css_classes(["suggested-action"])
        .halign(gtk::Align::End)
        .build();
    let actions = gtk::Box::builder()
        .margin_start(18)
        .margin_end(18)
        .margin_top(8)
        .margin_bottom(18)
        .halign(gtk::Align::End)
        .build();
    actions.append(&send);
    toolbar.add_bottom_bar(&actions);
    let prepared = Rc::new(RefCell::new(None));
    let update: Rc<dyn Fn()> = {
        let path = path.clone();
        let size = size.downgrade();
        let remove = remove.downgrade();
        let preview = preview.downgrade();
        let prepared = prepared.clone();
        let status = status.downgrade();
        let send = send.downgrade();
        Rc::new(move || {
            let (Some(size), Some(remove), Some(preview), Some(status), Some(send)) = (
                size.upgrade(),
                remove.upgrade(),
                preview.upgrade(),
                status.upgrade(),
                send.upgrade(),
            ) else {
                return;
            };
            let pixels = ImageSize::from_selection(size.selected()).pixels();
            match crate::sticker_image::prepare(&path, pixels, remove.is_active()) {
                Ok(image) => {
                    preview.set_paintable(Some(&gdk::Texture::for_pixbuf(&image.pixbuf)));
                    status.set_text(&if remove.is_active() && !image.background_removed {
                        crate::i18n::t("sticker_image_bg_unchanged")
                    } else {
                        String::new()
                    });
                    *prepared.borrow_mut() = Some(image);
                    send.set_sensitive(true);
                }
                Err(error) => {
                    *prepared.borrow_mut() = None;
                    preview.set_paintable(None::<&gdk::Texture>);
                    status.set_text(&crate::i18n::tf(
                        "media_send_failed",
                        &[("error", &error.to_string())],
                    ));
                    send.set_sensitive(false);
                }
            }
        })
    };
    let update_c = update.clone();
    size.connect_selected_notify(move |_| update_c());
    let update_c = update.clone();
    remove.connect_toggled(move |_| update_c());
    update();

    let s = state.clone();
    let weak_dialog = dialog.downgrade();
    send.connect_clicked(move |_| {
        // An editor opened in one conversation must never send to another.
        if composer_drafts::scope(&s) != scope {
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.close();
            }
            return;
        }
        let Some(image) = prepared.borrow().clone() else {
            return;
        };
        if send_prepared(&s, &image, &source) {
            let options = ImageOptions {
                size: ImageSize::from_selection(size.selected()),
                remove_background: remove.is_active(),
            };
            if let Err(error) =
                crate::sticker_image::remember_options_key(&s.data_dir, &source.key, options)
            {
                tracing::warn!(%error, "could not save sticker image options");
            }
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.close();
            }
        }
    });
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&body)
        .build();
    toolbar.set_content(Some(&scroll));
    dialog.set_child(Some(&toolbar));
    sticker_library::track_dialog(state, &dialog);
    dialog.present(Some(&state.window));
}
