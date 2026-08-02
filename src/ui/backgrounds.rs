use super::*;
use crate::config::{ChatBackgroundConfig, ensure_private_dir};

const BACKGROUND_CLASSES: &[&str] = &[
    "line-chat-bg-default",
    "line-chat-bg-mint",
    "line-chat-bg-ocean",
    "line-chat-bg-dusk",
    "line-chat-bg-graphite",
];

const BACKGROUND_CHOICES: &[(&str, &str, &str)] = &[
    (
        "default",
        "chat_background_default",
        "line-bg-swatch-default",
    ),
    ("mint", "chat_background_mint", "line-bg-swatch-mint"),
    ("ocean", "chat_background_ocean", "line-bg-swatch-ocean"),
    ("dusk", "chat_background_dusk", "line-bg-swatch-dusk"),
    (
        "graphite",
        "chat_background_graphite",
        "line-bg-swatch-graphite",
    ),
];

pub(super) fn wire_chat_background_actions(state: &AppState) {
    let state = state.clone();
    state.background_btn.clone().connect_clicked(move |_| {
        fill_background_popover(&state);
        state.background_popover.popup();
    });
}

pub(super) fn apply_chat_background(state: &AppState, chat_mid: &str) {
    for class in BACKGROUND_CLASSES {
        state.message_background_layer.remove_css_class(class);
    }
    state
        .message_background_layer
        .add_css_class("line-chat-bg-default");
    state
        .message_background_picture
        .set_paintable(None::<&gdk::Paintable>);
    state.message_background_picture.set_visible(false);

    let setting = state
        .config
        .borrow()
        .chat_backgrounds
        .get(chat_mid)
        .cloned()
        .unwrap_or_default();
    if setting.preset == "custom" && std::path::Path::new(&setting.image_path).is_file() {
        state.message_background_picture.set_visible(true);
        load_custom_background(state, chat_mid.to_string(), setting.image_path.clone());
    } else if setting.preset != "default" {
        state
            .message_background_layer
            .remove_css_class("line-chat-bg-default");
        state
            .message_background_layer
            .add_css_class(&format!("line-chat-bg-{}", setting.preset));
    }
}

fn load_custom_background(state: &AppState, chat_mid: String, path: String) {
    let (tx, rx) = async_channel::bounded::<Option<crate::sticker_anim::AnimFrames>>(1);
    let load_path = path.clone();
    std::thread::spawn(move || {
        let _ = tx.send_blocking(crate::sticker_anim::load_scaled(&load_path, 1600, false));
    });
    let state = state.clone();
    glib::spawn_future_local(async move {
        let loaded = rx.recv().await;
        let still_selected = state.current_chat.borrow().as_deref() == Some(chat_mid.as_str())
            && state
                .config
                .borrow()
                .chat_backgrounds
                .get(&chat_mid)
                .is_some_and(|background| {
                    background.preset == "custom" && background.image_path == path
                });
        if !still_selected {
            return;
        }
        match loaded {
            Ok(Some(frames)) => {
                apply_frames_to_picture(&state.message_background_picture, frames, false)
            }
            _ => {
                state.message_background_picture.set_visible(false);
                toast(&state, &crate::i18n::t("chat_background_failed"));
            }
        }
    });
}

fn fill_background_popover(state: &AppState) {
    let Some(chat_mid) = state.current_chat.borrow().clone() else {
        return;
    };
    let chat_name = state
        .chats
        .borrow()
        .iter()
        .find(|chat| chat.mid == chat_mid)
        .map(|chat| chat.name.clone())
        .unwrap_or_else(|| chat_mid.clone());
    let selected = state
        .config
        .borrow()
        .chat_backgrounds
        .get(&chat_mid)
        .cloned()
        .unwrap_or_default();

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .css_classes(["line-chat-background-menu"])
        .build();
    root.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("chat_background"))
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    root.append(
        &gtk::Label::builder()
            .label(chat_name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "dim-label"])
            .build(),
    );

    let choices = gtk::Grid::builder()
        .column_spacing(8)
        .row_spacing(8)
        .column_homogeneous(true)
        .build();
    for (index, (preset, label_key, swatch_class)) in BACKGROUND_CHOICES.iter().enumerate() {
        let selected_now = selected.preset == *preset;
        let button = background_choice_button(label_key, swatch_class, selected_now);
        let state = state.clone();
        let chat_mid = chat_mid.clone();
        let preset = (*preset).to_string();
        button.connect_clicked(move |_| {
            set_background_preset(&state, &chat_mid, &preset);
            state.background_popover.popdown();
        });
        choices.attach(&button, (index % 2) as i32, (index / 2) as i32, 1, 1);
    }
    root.append(&choices);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let custom = gtk::Button::builder()
        .halign(gtk::Align::Fill)
        .css_classes(if selected.preset == "custom" {
            vec!["flat", "line-chat-background-custom", "suggested-action"]
        } else {
            vec!["flat", "line-chat-background-custom"]
        })
        .build();
    let custom_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    custom_content.append(&gtk::Image::from_icon_name("folder-pictures-symbolic"));
    custom_content.append(
        &gtk::Label::builder()
            .label(crate::i18n::t("chat_background_choose_image"))
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    if selected.preset == "custom" {
        custom_content.append(&gtk::Image::from_icon_name("object-select-symbolic"));
    }
    custom.set_child(Some(&custom_content));
    {
        let state = state.clone();
        let chat_mid = chat_mid.clone();
        custom.connect_clicked(move |_| {
            state.background_popover.popdown();
            choose_custom_background(&state, &chat_mid);
        });
    }
    root.append(&custom);
    state.background_popover.set_child(Some(&root));
}

fn background_choice_button(label_key: &str, swatch_class: &str, selected: bool) -> gtk::Button {
    let button = gtk::Button::builder()
        .css_classes(if selected {
            vec![
                "flat",
                "line-chat-bg-choice",
                "line-chat-bg-choice-selected",
            ]
        } else {
            vec!["flat", "line-chat-bg-choice"]
        })
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    content.append(
        &gtk::Box::builder()
            .width_request(36)
            .height_request(28)
            .css_classes(["line-chat-bg-swatch", swatch_class])
            .build(),
    );
    content.append(
        &gtk::Label::builder()
            .label(crate::i18n::t(label_key))
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    content.append(
        &gtk::Image::builder()
            .icon_name("object-select-symbolic")
            .visible(selected)
            .build(),
    );
    button.set_child(Some(&content));
    button
}

fn set_background_preset(state: &AppState, chat_mid: &str, preset: &str) {
    {
        let mut config = state.config.borrow_mut();
        if preset == "default" {
            config.chat_backgrounds.remove(chat_mid);
        } else {
            config.chat_backgrounds.insert(
                chat_mid.to_string(),
                ChatBackgroundConfig {
                    preset: preset.to_string(),
                    image_path: String::new(),
                },
            );
        }
        config.save(&state.data_dir);
    }
    if state.current_chat.borrow().as_deref() == Some(chat_mid) {
        apply_chat_background(state, chat_mid);
    }
}

fn choose_custom_background(state: &AppState, chat_mid: &str) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&crate::i18n::t("chat_background_images")));
    filter.add_mime_type("image/png");
    filter.add_mime_type("image/jpeg");
    filter.add_mime_type("image/webp");
    filter.add_mime_type("image/gif");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(crate::i18n::t("chat_background_choose_image"))
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    let state = state.clone();
    let window = state.window.clone();
    let chat_mid = chat_mid.to_string();
    dialog.open(Some(&window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(source) = file.path() else {
            toast(&state, &crate::i18n::t("chat_background_failed"));
            return;
        };
        match cache_background_image(&state, &chat_mid, &source) {
            Ok(path) => {
                {
                    let mut config = state.config.borrow_mut();
                    config.chat_backgrounds.insert(
                        chat_mid.clone(),
                        ChatBackgroundConfig {
                            preset: "custom".into(),
                            image_path: path.to_string_lossy().to_string(),
                        },
                    );
                    config.save(&state.data_dir);
                }
                if state.current_chat.borrow().as_deref() == Some(chat_mid.as_str()) {
                    apply_chat_background(&state, &chat_mid);
                }
                toast(&state, &crate::i18n::t("chat_background_saved"));
            }
            Err(error) => toast(
                &state,
                &crate::i18n::tf(
                    "chat_background_save_failed",
                    &[("error", &error.to_string())],
                ),
            ),
        }
    });
}

fn cache_background_image(
    state: &AppState,
    chat_mid: &str,
    source: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    let metadata = std::fs::metadata(source)?;
    anyhow::ensure!(metadata.is_file(), "not a file");
    anyhow::ensure!(
        metadata.len() <= 32 * 1024 * 1024,
        "image is larger than 32 MB"
    );
    anyhow::ensure!(
        gdk_pixbuf::Pixbuf::file_info(source).is_some(),
        "unsupported image"
    );
    let dir = state.data_dir.join("backgrounds");
    ensure_private_dir(&dir)?;
    let safe_mid: String = chat_mid
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(80)
        .collect();
    anyhow::ensure!(!safe_mid.is_empty(), "invalid chat id");
    let destination = dir.join(format!("{safe_mid}.image"));
    let temporary = dir.join(format!(".{safe_mid}.image.tmp"));
    std::fs::copy(source, &temporary)?;
    std::fs::rename(&temporary, &destination)?;
    Ok(destination)
}
