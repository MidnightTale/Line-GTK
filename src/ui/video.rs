use super::*;
use std::cell::Cell;

type Refresh = Rc<dyn Fn(&gtk::MediaFile)>;

pub(super) fn append(body: &gtk::Box, path: &str, win: &gtk::Window) {
    let stream = gtk::MediaFile::new();
    let picture = gtk::Picture::builder()
        .paintable(&stream)
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .hexpand(true)
        .vexpand(true)
        .alternative_text(crate::i18n::t("preview_video"))
        .accessible_role(gtk::AccessibleRole::Img)
        .css_classes(["line-media-viewer-video"])
        .build();
    let surface: Rc<RefCell<Option<gdk::Surface>>> = Rc::default();
    let s = stream.clone();
    let surface_c = surface.clone();
    picture.connect_realize(move |picture| {
        if let Some(native) = picture.native().and_then(|native| native.surface()) {
            s.realize(&native);
            *surface_c.borrow_mut() = Some(native);
        }
    });
    let s = stream.clone();
    picture.connect_unrealize(move |_| {
        if let Some(surface) = surface.borrow_mut().take() {
            s.unrealize(&surface);
        }
    });

    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    stack.add_named(&picture, Some("video"));
    let loading = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let spinner = gtk::Spinner::builder()
        .spinning(true)
        .width_request(32)
        .height_request(32)
        .build();
    loading.append(&spinner);
    loading.append(&gtk::Label::new(Some(&crate::i18n::t("video_loading"))));
    stack.add_named(&loading, Some("loading"));
    let error = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .margin_start(24)
        .margin_end(24)
        .build();
    error.append(
        &gtk::Image::builder()
            .icon_name("dialog-warning-symbolic")
            .pixel_size(40)
            .build(),
    );
    error.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("video_play_failed"))
            .wrap(true)
            .max_width_chars(50)
            .justify(gtk::Justification::Center)
            .build(),
    );
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let retry = gtk::Button::with_label(&crate::i18n::t("retry"));
    let external = gtk::Button::with_label(&crate::i18n::t("media_open_externally"));
    actions.append(&retry);
    actions.append(&external);
    error.append(&actions);
    stack.add_named(&error, Some("error"));
    stack.set_visible_child_name("loading");
    body.append(&stack);

    let controls = gtk::Box::builder()
        .spacing(10)
        .margin_start(12)
        .margin_end(12)
        .margin_top(8)
        .margin_bottom(12)
        .css_classes(["line-video-controls"])
        .build();
    let play = gtk::Button::builder()
        .icon_name("media-playback-start-symbolic")
        .tooltip_text(crate::i18n::t("video_play"))
        .css_classes(["flat", "circular"])
        .build();
    let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
    seek.set_hexpand(true);
    seek.set_draw_value(false);
    seek.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "video_seek",
    ))]);
    let time = gtk::Label::builder()
        .label("0:00 / 0:00")
        .width_chars(15)
        .build();
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    volume.set_draw_value(false);
    volume.set_width_request(80);
    volume.set_value(1.0);
    volume.set_tooltip_text(Some(&crate::i18n::t("video_volume")));
    volume.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(
        "video_volume",
    ))]);
    controls.append(&play);
    controls.append(&seek);
    controls.append(&time);
    controls.append(&gtk::Image::from_icon_name("audio-volume-high-symbolic"));
    controls.append(&volume);
    body.append(&controls);
    let changing = Rc::new(Cell::new(false));
    let weak_play = play.downgrade();
    let weak_seek = seek.downgrade();
    let weak_time = time.downgrade();
    let weak_volume = volume.downgrade();
    let weak_stack = stack.downgrade();
    let weak_spinner = spinner.downgrade();
    let changing_c = changing.clone();
    let refresh: Refresh = Rc::new(move |stream| {
        let ready = stream.is_prepared() && stream.has_video() && stream.error().is_none();
        let failed = stream.error().is_some() || (stream.is_prepared() && !stream.has_video());
        if let Some(stack) = weak_stack.upgrade() {
            stack.set_visible_child_name(if failed {
                "error"
            } else if ready {
                "video"
            } else {
                "loading"
            });
        }
        if let Some(spinner) = weak_spinner.upgrade() {
            spinner.set_spinning(!ready && !failed);
        }
        if let Some(play) = weak_play.upgrade() {
            play.set_sensitive(ready);
            let (icon, key) = if stream.is_ended() {
                ("media-playback-start-symbolic", "video_replay")
            } else if stream.is_playing() {
                ("media-playback-pause-symbolic", "video_pause")
            } else {
                ("media-playback-start-symbolic", "video_play")
            };
            play.set_icon_name(icon);
            play.set_tooltip_text(Some(&crate::i18n::t(key)));
            play.update_property(&[gtk::accessible::Property::Label(&crate::i18n::t(key))]);
        }
        let duration = stream.duration().max(0);
        let timestamp = stream.timestamp().clamp(0, duration.max(0));
        if let Some(time) = weak_time.upgrade() {
            time.set_text(&format!("{} / {}", clock(timestamp), clock(duration)));
        }
        if let Some(seek) = weak_seek.upgrade() {
            changing_c.set(true);
            seek.set_range(0.0, (duration as f64 / 1_000_000.0).max(1.0));
            seek.set_value(timestamp as f64 / 1_000_000.0);
            seek.set_sensitive(ready && stream.is_seekable() && duration > 0);
            changing_c.set(false);
        }
        if let Some(volume) = weak_volume.upgrade() {
            volume.set_sensitive(ready && stream.has_audio());
        }
    });
    for property in [
        "prepared",
        "error",
        "playing",
        "ended",
        "duration",
        "timestamp",
        "seekable",
        "has-video",
        "has-audio",
    ] {
        let refresh = refresh.clone();
        stream.connect_notify_local(Some(property), move |stream, _| refresh(stream));
    }
    let s = stream.clone();
    play.connect_clicked(move |_| {
        if s.is_ended() {
            s.seek(0);
            s.play();
        } else if s.is_playing() {
            s.pause();
        } else {
            s.play();
        }
    });
    let s = stream.clone();
    seek.connect_value_changed(move |seek| {
        if !changing.get() && s.is_seekable() && s.duration() > 0 {
            s.seek((seek.value() * 1_000_000.0).round() as i64);
        }
    });
    let s = stream.clone();
    volume.connect_value_changed(move |volume| s.set_volume(volume.value()));
    let s = stream.clone();
    let original = gio::File::for_path(path);
    retry.connect_clicked(move |_| {
        s.pause();
        // GStreamer実装はファイルを開き直してもエラー状態を残す場合がある。
        if s.is_prepared() {
            s.stream_unprepared();
        }
        s.set_file(Some(&original));
        s.play();
    });
    let external_path = path.to_string();
    external.connect_clicked(move |_| open_path_externally(&external_path));
    // 閉じたあとに音声やデコーダーを動かし続けない。
    let s = stream.clone();
    win.connect_unmap(move |_| {
        s.pause();
        s.clear();
    });
    stream.connect_error_notify(|stream| {
        if let Some(error) = stream.error() {
            tracing::warn!(%error, "動画を再生できませんでした");
        }
    });
    refresh(&stream);
    stream.set_file(Some(&gio::File::for_path(path)));
    stream.play();
}

fn clock(microseconds: i64) -> String {
    let seconds = microseconds.max(0) / 1_000_000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::clock;
    #[test]
    fn elapsed_time_supports_long_videos_and_unknown_duration() {
        assert_eq!(clock(-1), "0:00");
        assert_eq!(clock(65_999_999), "1:05");
        assert_eq!(clock(3_661_000_000), "1:01:01");
    }
}
