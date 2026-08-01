use crate::config::{apply_animations, apply_font, apply_theme, AppConfig};
use crate::i18n;
use crate::sidecar::Sidecar;
use gtk::gio;
use gtk::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub struct SettingsDeps {
    pub sidecar: Rc<Sidecar>,
    pub data_dir: PathBuf,
    pub config: Rc<RefCell<AppConfig>>,
    pub on_logout: Rc<dyn Fn()>,
    pub on_lang: Rc<dyn Fn()>,
    pub on_animations: Rc<dyn Fn(bool)>,
    pub on_experimental_calls: Rc<dyn Fn(bool)>,
    pub on_tray_settings: Rc<dyn Fn()>,
    pub on_discord_rpc: Rc<dyn Fn()>,
    pub toast: Rc<dyn Fn(&str)>,
}

pub fn open_settings(parent: &impl IsA<gtk::Window>, deps: SettingsDeps) {
    let win = libadwaita::PreferencesWindow::builder()
        .transient_for(parent)
        .title(i18n::t("settings"))
        .search_enabled(true)
        .build();

    // ---- Appearance ----
    let appearance = libadwaita::PreferencesPage::builder()
        .title(i18n::t("settings_appearance"))
        .icon_name("preferences-desktop-appearance-symbolic")
        .build();
    let look = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("settings_look"))
        .build();

    let theme_row = libadwaita::ComboRow::builder()
        .title(i18n::t("theme"))
        .subtitle(i18n::t("theme_subtitle"))
        .build();
    let themes = gtk::StringList::new(&[
        &i18n::t("theme_system"),
        &i18n::t("theme_dark"),
        &i18n::t("theme_light"),
    ]);
    theme_row.set_model(Some(&themes));
    theme_row.set_selected(match deps.config.borrow().theme.as_str() {
        "dark" => 1,
        "light" => 2,
        _ => 0,
    });
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        theme_row.connect_selected_notify(move |row| {
            let theme = match row.selected() {
                1 => "dark",
                2 => "light",
                _ => "system",
            };
            cfg.borrow_mut().theme = theme.into();
            cfg.borrow().save(&data_dir);
            apply_theme(theme);
        });
    }
    look.add(&theme_row);

    let lang_row = libadwaita::ComboRow::builder()
        .title(i18n::t("language"))
        .subtitle(i18n::t("language_subtitle"))
        .build();
    let langs = gtk::StringList::new(&[&i18n::t("lang_thai"), &i18n::t("lang_english")]);
    lang_row.set_model(Some(&langs));
    lang_row.set_selected(match deps.config.borrow().language.as_str() {
        "en" | "eng" => 1,
        _ => 0,
    });
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let on_lang = deps.on_lang.clone();
        lang_row.connect_selected_notify(move |row| {
            let language = if row.selected() == 1 { "en" } else { "th" };
            cfg.borrow_mut().language = language.into();
            cfg.borrow().save(&data_dir);
            on_lang();
        });
    }
    look.add(&lang_row);

    let anim_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("animations"))
        .subtitle(i18n::t("animations_subtitle"))
        .active(deps.config.borrow().animations)
        .build();
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let on_animations = deps.on_animations.clone();
        anim_row.connect_active_notify(move |row| {
            let on = row.is_active();
            cfg.borrow_mut().animations = on;
            cfg.borrow().save(&data_dir);
            apply_animations(on);
            on_animations(on);
        });
    }
    look.add(&anim_row);

    let desktop_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("settings_desktop"))
        .description(i18n::t("settings_desktop_desc"))
        .build();
    let tray_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("tray_enabled"))
        .subtitle(i18n::t("tray_enabled_subtitle"))
        .active(deps.config.borrow().tray_enabled)
        .build();
    let close_tray_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("close_to_tray"))
        .subtitle(i18n::t("close_to_tray_subtitle"))
        .active(deps.config.borrow().close_to_tray)
        .sensitive(deps.config.borrow().tray_enabled)
        .build();
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let on_tray = deps.on_tray_settings.clone();
        let close_tray_row = close_tray_row.clone();
        tray_row.connect_active_notify(move |row| {
            let on = row.is_active();
            cfg.borrow_mut().tray_enabled = on;
            cfg.borrow().save(&data_dir);
            close_tray_row.set_sensitive(on);
            on_tray();
        });
    }
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let on_tray = deps.on_tray_settings.clone();
        close_tray_row.connect_active_notify(move |row| {
            if !cfg.borrow().tray_enabled && row.is_active() {
                return;
            }
            cfg.borrow_mut().close_to_tray = row.is_active();
            cfg.borrow().save(&data_dir);
            on_tray();
        });
    }
    desktop_grp.add(&tray_row);
    desktop_grp.add(&close_tray_row);

    let discord_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("settings_discord"))
        .description(i18n::t("settings_discord_desc"))
        .build();
    let discord_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("discord_rpc"))
        .subtitle(i18n::t("discord_rpc_subtitle"))
        .active(deps.config.borrow().discord_rpc)
        .build();
    let discord_chat_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("discord_rpc_show_chat"))
        .subtitle(i18n::t("discord_rpc_show_chat_subtitle"))
        .active(deps.config.borrow().discord_rpc_show_chat)
        .sensitive(deps.config.borrow().discord_rpc)
        .build();
    let discord_avatar_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("discord_rpc_show_avatar"))
        .subtitle(i18n::t("discord_rpc_show_avatar_subtitle"))
        .active(deps.config.borrow().discord_rpc_show_avatar)
        .sensitive(deps.config.borrow().discord_rpc)
        .build();
    let discord_name_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("discord_rpc_show_name"))
        .subtitle(i18n::t("discord_rpc_show_name_subtitle"))
        .active(deps.config.borrow().discord_rpc_show_name)
        .sensitive(deps.config.borrow().discord_rpc)
        .build();
    let id_initial = {
        let c = deps.config.borrow();
        let custom = c.discord_rpc_client_id.trim();
        if custom.is_empty() {
            crate::discord_rpc::DEFAULT_APP_ID.to_string()
        } else {
            custom.to_string()
        }
    };
    let discord_id_row = libadwaita::EntryRow::builder()
        .title(i18n::t("discord_rpc_client_id"))
        .text(&id_initial)
        .build();
    discord_id_row.set_show_apply_button(true);
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let on_discord = deps.on_discord_rpc.clone();
        let discord_chat_row = discord_chat_row.clone();
        let discord_avatar_row = discord_avatar_row.clone();
        let discord_name_row = discord_name_row.clone();
        discord_row.connect_active_notify(move |row| {
            let on = row.is_active();
            cfg.borrow_mut().discord_rpc = on;
            cfg.borrow().save(&data_dir);
            discord_chat_row.set_sensitive(on);
            discord_avatar_row.set_sensitive(on);
            discord_name_row.set_sensitive(on);
            on_discord();
        });
    }
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let on_discord = deps.on_discord_rpc.clone();
        discord_chat_row.connect_active_notify(move |row| {
            if !cfg.borrow().discord_rpc {
                return;
            }
            cfg.borrow_mut().discord_rpc_show_chat = row.is_active();
            cfg.borrow().save(&data_dir);
            on_discord();
        });
    }
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let on_discord = deps.on_discord_rpc.clone();
        discord_avatar_row.connect_active_notify(move |row| {
            if !cfg.borrow().discord_rpc {
                return;
            }
            cfg.borrow_mut().discord_rpc_show_avatar = row.is_active();
            cfg.borrow().save(&data_dir);
            on_discord();
        });
    }
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let on_discord = deps.on_discord_rpc.clone();
        discord_name_row.connect_active_notify(move |row| {
            if !cfg.borrow().discord_rpc {
                return;
            }
            cfg.borrow_mut().discord_rpc_show_name = row.is_active();
            cfg.borrow().save(&data_dir);
            on_discord();
        });
    }
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let on_discord = deps.on_discord_rpc.clone();
        let toast = deps.toast.clone();
        discord_id_row.connect_apply(move |row| {
            let id = row
                .text()
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>();
            let stored = if id == crate::discord_rpc::DEFAULT_APP_ID {
                String::new()
            } else {
                id.clone()
            };
            row.set_text(if stored.is_empty() {
                crate::discord_rpc::DEFAULT_APP_ID
            } else {
                &id
            });
            cfg.borrow_mut().discord_rpc_client_id = stored;
            cfg.borrow().save(&data_dir);
            on_discord();
            toast(&i18n::t("discord_rpc_saved"));
        });
    }
    discord_grp.add(&discord_row);
    discord_grp.add(&discord_chat_row);
    discord_grp.add(&discord_avatar_row);
    discord_grp.add(&discord_name_row);
    discord_grp.add(&discord_id_row);

    appearance.add(&look);
    appearance.add(&desktop_grp);
    appearance.add(&discord_grp);

    let font_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("features_font"))
        .description(i18n::t("features_font_desc"))
        .build();
    let font_entry = libadwaita::EntryRow::builder()
        .title(i18n::t("font_family"))
        .text(deps.config.borrow().font_family.as_str())
        .build();
    font_entry.set_show_apply_button(true);
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let toast = deps.toast.clone();
        font_entry.connect_apply(move |row| {
            let family = row.text().trim().to_string();
            cfg.borrow_mut().font_family = family.clone();
            let scale = cfg.borrow().font_scale;
            cfg.borrow().save(&data_dir);
            apply_font(&family, scale);
            if family.is_empty() {
                toast(&i18n::t("font_family_default"));
            } else {
                toast(&i18n::tf("font_family_set", &[("name", &family)]));
            }
        });
    }
    font_grp.add(&font_entry);

    let scale_row = libadwaita::ActionRow::builder()
        .title(i18n::t("ui_scale"))
        .subtitle(i18n::t("ui_scale_subtitle"))
        .activatable(false)
        .build();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 60.0, 140.0, 5.0);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_digits(0);
    scale.set_hexpand(true);
    scale.set_width_request(200);
    scale.set_valign(gtk::Align::Center);
    scale.add_mark(100.0, gtk::PositionType::Bottom, Some("100%"));
    scale.set_value((deps.config.borrow().font_scale * 100.0).clamp(60.0, 140.0));
    scale.set_format_value_func(|_, v| format!("{v:.0}%"));
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        scale.connect_value_changed(move |s| {
            let v = (s.value() / 100.0).clamp(0.60, 1.40);
            cfg.borrow_mut().font_scale = v;
            let family = cfg.borrow().font_family.clone();
            cfg.borrow().save(&data_dir);
            apply_font(&family, v);
        });
    }
    scale_row.add_suffix(&scale);
    font_grp.add(&scale_row);
    appearance.add(&font_grp);
    win.add(&appearance);

    // ---- Features ----
    let features = libadwaita::PreferencesPage::builder()
        .title(i18n::t("features"))
        .icon_name("applications-utilities-symbolic")
        .build();
    let chat_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("features_chat"))
        .description(i18n::t("features_chat_desc"))
        .build();
    let notif_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("notifications"))
        .subtitle(i18n::t("notifications_subtitle"))
        .active(deps.config.borrow().notifications)
        .build();
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        notif_row.connect_active_notify(move |row| {
            cfg.borrow_mut().notifications = row.is_active();
            cfg.borrow().save(&data_dir);
        });
    }
    chat_grp.add(&notif_row);

    let sound_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("notification_sound"))
        .subtitle(i18n::t("notification_sound_subtitle"))
        .active(deps.config.borrow().notification_sound)
        .build();
    chat_grp.add(&sound_row);

    let sound_vol_row = libadwaita::ActionRow::builder()
        .title(i18n::t("notification_sound_vol"))
        .subtitle(i18n::t("notification_sound_vol_subtitle"))
        .sensitive(deps.config.borrow().notification_sound)
        .build();
    let sound_vol = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 2.0, 0.05);
    sound_vol.set_value(
        deps.config
            .borrow()
            .notification_sound_volume
            .clamp(0.0, 2.0),
    );
    sound_vol.set_width_request(160);
    sound_vol.set_draw_value(true);
    sound_vol.set_digits(2);
    sound_vol.set_valign(gtk::Align::Center);
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        sound_vol.connect_value_changed(move |scale| {
            cfg.borrow_mut().notification_sound_volume = scale.value();
            cfg.borrow().save(&data_dir);
        });
    }
    sound_vol_row.add_suffix(&sound_vol);
    chat_grp.add(&sound_vol_row);
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let sound_vol_row = sound_vol_row.clone();
        sound_row.connect_active_notify(move |row| {
            let on = row.is_active();
            cfg.borrow_mut().notification_sound = on;
            cfg.borrow().save(&data_dir);
            sound_vol_row.set_sensitive(on);
        });
    }

    let mark_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("auto_mark_read"))
        .subtitle(i18n::t("auto_mark_read_subtitle"))
        .active(deps.config.borrow().auto_mark_read)
        .build();
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        mark_row.connect_active_notify(move |row| {
            cfg.borrow_mut().auto_mark_read = row.is_active();
            cfg.borrow().save(&data_dir);
        });
    }
    chat_grp.add(&mark_row);

    let audio_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("settings_audio"))
        .description(i18n::t("settings_audio_desc"))
        .build();

    let inputs = crate::ui::pulse_sources();
    let input_labels: Vec<&str> = inputs.iter().map(|(_, l)| l.as_str()).collect();
    let input_row = libadwaita::ComboRow::builder()
        .title(i18n::t("audio_input"))
        .subtitle(i18n::t("audio_input_subtitle"))
        .build();
    input_row.set_model(Some(&gtk::StringList::new(&input_labels)));
    {
        let cur = deps.config.borrow().audio_input.clone();
        let idx = inputs
            .iter()
            .position(|(n, _)| n == &cur || (cur.is_empty() && n == "default"))
            .unwrap_or(0);
        input_row.set_selected(idx as u32);
    }
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let inputs = inputs.clone();
        input_row.connect_selected_notify(move |row| {
            let i = row.selected() as usize;
            if let Some((name, _)) = inputs.get(i) {
                cfg.borrow_mut().audio_input = if name == "default" {
                    String::new()
                } else {
                    name.clone()
                };
                cfg.borrow().save(&data_dir);
            }
        });
    }
    audio_grp.add(&input_row);

    let outputs = crate::ui::pulse_sinks();
    let output_labels: Vec<&str> = outputs.iter().map(|(_, l)| l.as_str()).collect();
    let output_row = libadwaita::ComboRow::builder()
        .title(i18n::t("audio_output"))
        .subtitle(i18n::t("audio_output_subtitle"))
        .build();
    output_row.set_model(Some(&gtk::StringList::new(&output_labels)));
    {
        let cur = deps.config.borrow().audio_output.clone();
        let idx = outputs
            .iter()
            .position(|(n, _)| n == &cur || (cur.is_empty() && n == "default"))
            .unwrap_or(0);
        output_row.set_selected(idx as u32);
    }
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let outputs = outputs.clone();
        output_row.connect_selected_notify(move |row| {
            let i = row.selected() as usize;
            if let Some((name, _)) = outputs.get(i) {
                cfg.borrow_mut().audio_output = if name == "default" {
                    String::new()
                } else {
                    name.clone()
                };
                cfg.borrow().save(&data_dir);
            }
        });
    }
    audio_grp.add(&output_row);

    features.add(&chat_grp);
    features.add(&audio_grp);

    // ---- Experimental (voice calls) ----
    let exp_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("settings_experimental"))
        .description(i18n::t("settings_experimental_desc"))
        .build();
    let exp_calls_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("experimental_calls"))
        .subtitle(i18n::t("experimental_calls_subtitle"))
        .active(deps.config.borrow().experimental_calls)
        .build();

    let calls_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("settings_calls"))
        .description(i18n::t("settings_calls_desc"))
        .build();
    let mic_vol_row = libadwaita::ActionRow::builder()
        .title(i18n::t("call_mic_vol"))
        .subtitle(i18n::t("call_mic_vol_subtitle"))
        .build();
    let mic_vol = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 2.0, 0.05);
    mic_vol.set_value(deps.config.borrow().call_mic_volume.clamp(0.0, 2.5));
    mic_vol.set_width_request(160);
    mic_vol.set_draw_value(true);
    mic_vol.set_digits(2);
    mic_vol.set_valign(gtk::Align::Center);
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        mic_vol.connect_value_changed(move |scale| {
            cfg.borrow_mut().call_mic_volume = scale.value();
            cfg.borrow().save(&data_dir);
        });
    }
    mic_vol_row.add_suffix(&mic_vol);
    calls_grp.add(&mic_vol_row);

    let spk_vol_row = libadwaita::ActionRow::builder()
        .title(i18n::t("call_spk_vol"))
        .subtitle(i18n::t("call_spk_vol_subtitle"))
        .build();
    let spk_vol = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 2.0, 0.05);
    spk_vol.set_value(deps.config.borrow().call_spk_volume.clamp(0.0, 2.5));
    spk_vol.set_width_request(160);
    spk_vol.set_draw_value(true);
    spk_vol.set_digits(2);
    spk_vol.set_valign(gtk::Align::Center);
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        spk_vol.connect_value_changed(move |scale| {
            cfg.borrow_mut().call_spk_volume = scale.value();
            cfg.borrow().save(&data_dir);
        });
    }
    spk_vol_row.add_suffix(&spk_vol);
    calls_grp.add(&spk_vol_row);

    let calls_note = libadwaita::ActionRow::builder()
        .title(i18n::t("voice_call"))
        .subtitle(i18n::t("voice_call_note"))
        .build();
    calls_grp.add(&calls_note);

    let calls_unlocked = deps.config.borrow().experimental_calls;
    calls_grp.set_sensitive(calls_unlocked);
    calls_grp.set_visible(calls_unlocked);

    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let sidecar = deps.sidecar.clone();
        let on_exp = deps.on_experimental_calls.clone();
        let toast = deps.toast.clone();
        let calls_grp = calls_grp.clone();
        let settings_win = win.clone();
        let suppressing = Rc::new(std::cell::Cell::new(false));
        let dialog_open = Rc::new(std::cell::Cell::new(false));
        exp_calls_row.connect_active_notify(move |row| {
            if suppressing.get() || dialog_open.get() {
                return;
            }
            let want_on = row.is_active();
            let already = cfg.borrow().experimental_calls;
            if want_on == already {
                return;
            }

            // Keep the switch on the current saved value until confirmed.
            suppressing.set(true);
            row.set_active(already);
            suppressing.set(false);

            let (heading, body, confirm_id, confirm_label) = if want_on {
                (
                    i18n::t("exp_calls_warn_heading"),
                    i18n::t("exp_calls_warn_body"),
                    "enable",
                    i18n::t("exp_calls_warn_enable"),
                )
            } else {
                (
                    i18n::t("exp_calls_disable_heading"),
                    i18n::t("exp_calls_disable_body"),
                    "disable",
                    i18n::t("exp_calls_disable_confirm"),
                )
            };

            let dialog = libadwaita::AlertDialog::new(Some(&heading), Some(&body));
            dialog.add_response("cancel", &i18n::t("exp_calls_warn_cancel"));
            dialog.add_response(confirm_id, &confirm_label);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            dialog.set_response_appearance(
                confirm_id,
                libadwaita::ResponseAppearance::Destructive,
            );

            let cfg = cfg.clone();
            let data_dir = data_dir.clone();
            let sidecar = sidecar.clone();
            let on_exp = on_exp.clone();
            let toast = toast.clone();
            let calls_grp = calls_grp.clone();
            let row = row.clone();
            let settings_win_close = settings_win.clone();
            let settings_win_present = settings_win.clone();
            let suppressing = suppressing.clone();
            let dialog_open = dialog_open.clone();
            let confirm_id = confirm_id.to_string();
            dialog_open.set(true);
            dialog.connect_response(None, move |_dialog, response| {
                dialog_open.set(false);
                if response != confirm_id {
                    return;
                }
                suppressing.set(true);
                row.set_active(want_on);
                suppressing.set(false);
                cfg.borrow_mut().experimental_calls = want_on;
                cfg.borrow().save(&data_dir);
                calls_grp.set_sensitive(want_on);
                calls_grp.set_visible(want_on);
                let _ = sidecar.logout();
                toast(&i18n::t("exp_calls_relogin_toast"));
                on_exp(want_on);
                settings_win_close.close();
            });
            // Show the warning on the Settings window itself.
            dialog.present(Some(&settings_win_present));
        });
    }
    exp_grp.add(&exp_calls_row);
    features.add(&exp_grp);
    features.add(&calls_grp);
    win.add(&features);

    // ---- Downloads ----
    let downloads_page = libadwaita::PreferencesPage::builder()
        .title(i18n::t("settings_downloads"))
        .icon_name("folder-download-symbolic")
        .build();
    let download_opts = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("settings_downloads"))
        .description(i18n::t("settings_downloads_desc"))
        .build();
    let ask_row = libadwaita::SwitchRow::builder()
        .title(i18n::t("download_ask_every_time"))
        .subtitle(i18n::t("download_ask_every_time_subtitle"))
        .active(deps.config.borrow().download_ask_every_time)
        .build();
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        ask_row.connect_active_notify(move |row| {
            cfg.borrow_mut().download_ask_every_time = row.is_active();
            cfg.borrow().save(&data_dir);
        });
    }
    download_opts.add(&ask_row);
    downloads_page.add(&download_opts);

    let folder_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("settings_downloads"))
        .build();

    let make_folder_row = |title_key: &str, kind: &'static str, configured: String| {
        let row = libadwaita::ActionRow::builder()
            .title(i18n::t(title_key))
            .subtitle(AppConfig::download_dir_display(&configured))
            .build();
        let choose = gtk::Button::builder()
            .label(i18n::t("download_folder_choose"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let reset = gtk::Button::builder()
            .label(i18n::t("download_folder_reset"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let toast = deps.toast.clone();
        let parent = win.clone();
        let row_choose = row.clone();
        choose.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title(i18n::t("download_folder_choose"))
                .modal(true)
                .build();
            let initial = cfg.borrow().download_dir_for(kind);
            dialog.set_initial_folder(Some(&gio::File::for_path(initial)));
            let cfg = cfg.clone();
            let data_dir = data_dir.clone();
            let toast = toast.clone();
            let row = row_choose.clone();
            dialog.select_folder(
                Some(&parent),
                None::<&gio::Cancellable>,
                move |result| {
                    let Ok(file) = result else {
                        return;
                    };
                    let Some(path) = file.path() else {
                        return;
                    };
                    let path_str = path.display().to_string();
                    cfg.borrow_mut().set_download_dir_for(kind, path_str.clone());
                    cfg.borrow().save(&data_dir);
                    row.set_subtitle(&path_str);
                    toast(&i18n::t("download_folder_saved"));
                },
            );
        });
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let toast = deps.toast.clone();
        let row_reset = row.clone();
        reset.connect_clicked(move |_| {
            cfg.borrow_mut().set_download_dir_for(kind, String::new());
            cfg.borrow().save(&data_dir);
            row_reset.set_subtitle(&AppConfig::download_dir_display(""));
            toast(&i18n::t("download_folder_saved"));
        });
        row.add_suffix(&reset);
        row.add_suffix(&choose);
        row
    };

    {
        let cfg = deps.config.borrow();
        folder_grp.add(&make_folder_row(
            "download_folder_image",
            "IMAGE",
            cfg.download_dir_image.clone(),
        ));
        folder_grp.add(&make_folder_row(
            "download_folder_video",
            "VIDEO",
            cfg.download_dir_video.clone(),
        ));
        folder_grp.add(&make_folder_row(
            "download_folder_audio",
            "AUDIO",
            cfg.download_dir_audio.clone(),
        ));
        folder_grp.add(&make_folder_row(
            "download_folder_file",
            "FILE",
            cfg.download_dir_file.clone(),
        ));
    }
    downloads_page.add(&folder_grp);
    win.add(&downloads_page);

    // ---- Cache ----
    let cache_page = libadwaita::PreferencesPage::builder()
        .title(i18n::t("settings_cache"))
        .icon_name("drive-harddisk-symbolic")
        .build();
    let cache_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("clear_cache"))
        .description(i18n::t("clear_cache_desc"))
        .build();

    let retention_grp = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("cache_retention"))
        .description(i18n::t("cache_retention_desc"))
        .build();
    let retention_row = libadwaita::ComboRow::builder()
        .title(i18n::t("cache_retention"))
        .subtitle(i18n::t("cache_retention_subtitle"))
        .build();
    let retention_model = gtk::StringList::new(&[
        &i18n::t("cache_ret_smart"),
        &i18n::t("cache_ret_day"),
        &i18n::t("cache_ret_week"),
        &i18n::t("cache_ret_month"),
        &i18n::t("cache_ret_forever"),
    ]);
    retention_row.set_model(Some(&retention_model));
    retention_row.set_selected(match deps.config.borrow().cache_retention.as_str() {
        "day" => 1,
        "week" => 2,
        "month" => 3,
        "forever" => 4,
        _ => 0,
    });
    {
        let cfg = deps.config.clone();
        let data_dir = deps.data_dir.clone();
        let toast = deps.toast.clone();
        retention_row.connect_selected_notify(move |row| {
            let retention = match row.selected() {
                1 => "day",
                2 => "week",
                3 => "month",
                4 => "forever",
                _ => "smart",
            };
            cfg.borrow_mut().cache_retention = retention.into();
            cfg.borrow().save(&data_dir);
            toast(&i18n::tf("cache_retention_saved", &[("mode", retention)]));
        });
    }
    retention_grp.add(&retention_row);
    cache_page.add(&retention_grp);

    let make_clear = |title_key: &str, kind: &str| {
        let row = libadwaita::ActionRow::builder()
            .title(i18n::t(title_key))
            .build();
        let btn = gtk::Button::builder()
            .label(i18n::t("clear"))
            .valign(gtk::Align::Center)
            .css_classes(["destructive-action"])
            .build();
        let sidecar = deps.sidecar.clone();
        let toast = deps.toast.clone();
        let kind = kind.to_string();
        btn.connect_clicked(move |_| match sidecar.clear_cache(&kind) {
            Ok(_) => toast(&i18n::tf("cleared_cache", &[("kind", &kind)])),
            Err(e) => toast(&i18n::tf("clear_failed", &[("error", &e.to_string())])),
        });
        row.add_suffix(&btn);
        row.set_activatable_widget(Some(&btn));
        row
    };

    cache_grp.add(&make_clear("cache_media", "media"));
    cache_grp.add(&make_clear("cache_stickers", "stickers"));
    cache_grp.add(&make_clear("cache_avatars", "avatars"));
    cache_grp.add(&make_clear("cache_messages", "messages"));
    cache_grp.add(&make_clear("cache_chats", "chats"));
    cache_grp.add(&make_clear("cache_all", "all"));
    cache_page.add(&cache_grp);
    win.add(&cache_page);

    // ---- Account ----
    let account = libadwaita::PreferencesPage::builder()
        .title(i18n::t("settings_account"))
        .icon_name("avatar-default-symbolic")
        .build();
    let session = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("session"))
        .build();
    let logout_row = libadwaita::ActionRow::builder()
        .title(i18n::t("logout"))
        .subtitle(i18n::t("logout_subtitle"))
        .build();
    let logout_btn = gtk::Button::builder()
        .label(i18n::t("logout"))
        .valign(gtk::Align::Center)
        .css_classes(["destructive-action"])
        .build();
    {
        let sidecar = deps.sidecar.clone();
        let toast = deps.toast.clone();
        let on_logout = deps.on_logout.clone();
        let win2 = win.clone();
        logout_btn.connect_clicked(move |_| {
            let _ = sidecar.logout();
            toast(&i18n::t("logged_out"));
            on_logout();
            win2.close();
        });
    }
    logout_row.add_suffix(&logout_btn);
    session.add(&logout_row);

    let about = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("about"))
        .description(i18n::t("about_desc"))
        .build();
    about.add(
        &libadwaita::ActionRow::builder()
            .title(i18n::t("about_version"))
            .subtitle(i18n::t("about_version_value"))
            .build(),
    );
    {
        let row = libadwaita::ActionRow::builder()
            .title(i18n::t("about_maintainer"))
            .subtitle(i18n::t("about_maintainer_name"))
            .activatable(true)
            .build();
        row.add_suffix(
            &gtk::Image::from_icon_name("go-next-symbolic"),
        );
        row.connect_activated(|_| {
            let _ = gio::AppInfo::launch_default_for_uri(
                "https://github.com/MidnightTale",
                None::<&gio::AppLaunchContext>,
            );
        });
        about.add(&row);
    }
    {
        let row = libadwaita::ActionRow::builder()
            .title(i18n::t("about_source"))
            .subtitle(i18n::t("about_source_url"))
            .activatable(true)
            .build();
        row.add_suffix(
            &gtk::Image::from_icon_name("go-next-symbolic"),
        );
        row.connect_activated(|_| {
            let _ = gio::AppInfo::launch_default_for_uri(
                "https://github.com/MidnightTale/line-gtk",
                None::<&gio::AppLaunchContext>,
            );
        });
        about.add(&row);
    }

    let stack = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("about_stack"))
        .build();
    stack.add(
        &libadwaita::ActionRow::builder()
            .title(i18n::t("about_stack_ui"))
            .build(),
    );
    stack.add(
        &libadwaita::ActionRow::builder()
            .title(i18n::t("about_stack_protocol"))
            .build(),
    );
    stack.add(
        &libadwaita::ActionRow::builder()
            .title(i18n::t("about_stack_runtime"))
            .build(),
    );

    let credits = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("about_credits"))
        .description(i18n::t("about_credits_desc"))
        .build();

    let license = libadwaita::PreferencesGroup::builder()
        .title(i18n::t("about_license"))
        .description(i18n::t("about_license_desc"))
        .build();
    license.add(
        &libadwaita::ActionRow::builder()
            .title(i18n::t("about_license_name"))
            .subtitle(i18n::t("about_license_spdx"))
            .build(),
    );

    account.add(&session);
    account.add(&about);
    account.add(&stack);
    account.add(&credits);
    account.add(&license);
    win.add(&account);

    win.present();
}
