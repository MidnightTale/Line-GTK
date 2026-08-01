use crate::sidecar::Sidecar;
use gtk::prelude::*;
use libadwaita::prelude::*;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;

pub fn open(parent: &impl IsA<gtk::Window>, sidecar: Rc<Sidecar>, data_dir: &Path) {
    let window = libadwaita::PreferencesWindow::builder()
        .transient_for(parent)
        .title(crate::i18n::t("diagnostics"))
        .search_enabled(false)
        .default_width(640)
        .default_height(640)
        .build();
    let page = libadwaita::PreferencesPage::builder()
        .title(crate::i18n::t("diagnostics"))
        .icon_name("utilities-system-monitor-symbolic")
        .build();

    let status = sidecar.status();
    let engine = libadwaita::PreferencesGroup::builder()
        .title(crate::i18n::t("diagnostics_engine"))
        .build();
    let running_label = crate::i18n::t(if status.running {
        "diagnostics_running"
    } else {
        "diagnostics_stopped"
    });
    engine.add(&row(&crate::i18n::t("diagnostics_status"), &running_label));
    engine.add(&row(&crate::i18n::t("diagnostics_runtime"), status.runtime));
    engine.add(&row(
        &crate::i18n::t("diagnostics_pid"),
        &status
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "—".into()),
    ));
    engine.add(&row(
        &crate::i18n::t("diagnostics_pending"),
        &status.pending_requests.to_string(),
    ));
    let restart = gtk::Button::builder()
        .label(crate::i18n::t("diagnostics_restart"))
        .halign(gtk::Align::Start)
        .css_classes(["suggested-action"])
        .build();
    {
        let sidecar = sidecar.clone();
        restart.connect_clicked(move |button| {
            button.set_sensitive(false);
            let label = if sidecar.recover().is_ok() {
                crate::i18n::t("diagnostics_restarted")
            } else {
                crate::i18n::t("diagnostics_restart_failed")
            };
            button.set_label(&label);
        });
    }
    engine.add(&restart);
    page.add(&engine);

    let dependencies = libadwaita::PreferencesGroup::builder()
        .title(crate::i18n::t("diagnostics_dependencies"))
        .build();
    for program in ["ffmpeg", "ffprobe", "pdftoppm", "pdftotext"] {
        dependencies.add(&row(program, &program_version(program)));
    }
    dependencies.add(&row(
        "GTK",
        &format!(
            "{}.{}.{}",
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version()
        ),
    ));
    dependencies.add(&row(
        "Libadwaita",
        &format!(
            "{}.{}.{}",
            libadwaita::major_version(),
            libadwaita::minor_version(),
            libadwaita::micro_version()
        ),
    ));
    page.add(&dependencies);

    let codecs = libadwaita::PreferencesGroup::builder()
        .title(crate::i18n::t("diagnostics_codecs"))
        .description(crate::i18n::t("diagnostics_codecs_desc"))
        .build();
    let decoder_list = command_output("ffmpeg", &["-hide_banner", "-decoders"]);
    for (label, needles) in [
        ("H.264", &[" h264 ", " h264_v4l2m2m "][..]),
        ("H.265/HEVC", &[" hevc ", " hevc_v4l2m2m "][..]),
        ("VP9", &[" vp9 ", " libvpx-vp9 "][..]),
        ("AV1", &[" av1 ", " libdav1d "][..]),
        ("Opus", &[" opus ", " libopus "][..]),
    ] {
        let available = decoder_list
            .as_deref()
            .map(|list| needles.iter().any(|needle| list.contains(needle)))
            .unwrap_or(false);
        let availability = crate::i18n::t(if available {
            "diagnostics_available"
        } else {
            "diagnostics_unavailable"
        });
        codecs.add(&row(label, &availability));
    }
    page.add(&codecs);

    let paths = libadwaita::PreferencesGroup::builder()
        .title(crate::i18n::t("diagnostics_paths"))
        .build();
    paths.add(&row(
        &crate::i18n::t("diagnostics_data"),
        &data_dir.display().to_string(),
    ));
    paths.add(&row(
        &crate::i18n::t("diagnostics_log"),
        &data_dir.join("line-gtk.log").display().to_string(),
    ));
    page.add(&paths);

    window.add(&page);
    window.present();
}

fn row(title: &str, subtitle: &str) -> libadwaita::ActionRow {
    libadwaita::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .subtitle_selectable(true)
        .build()
}

fn program_version(program: &str) -> String {
    command_output(program, &["--version"])
        .and_then(|output| output.lines().next().map(str::to_string))
        .unwrap_or_else(|| crate::i18n::t("diagnostics_not_found"))
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some(text)
}
