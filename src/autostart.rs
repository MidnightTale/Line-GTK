//! Launch at login.
//!
//! Native installs use an XDG autostart entry under `$XDG_CONFIG_HOME/autostart`.
//! Inside Flatpak that directory is private to the sandbox, so the Background portal
//! is asked to register the autostart instead.

use gtk::gio;
use gtk::glib;
use gtk::glib::prelude::ToVariant;
use std::path::{Path, PathBuf};

/// Command-line flag passed by the autostart entry: keep the window hidden in the tray.
pub const BACKGROUND_FLAG: &str = "--background";

const DESKTOP_FILE: &str = "dev.linegtk.LineGtk.desktop";

pub fn in_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

fn entry_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("autostart").join(DESKTOP_FILE))
}

/// Quote one `Exec` argument as the Desktop Entry spec requires.
fn exec_quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:+@,".contains(c));
    if plain {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        match c {
            // Reserved in a quoted argument; the backslash itself is doubled again
            // by the string-value escaping of the key file.
            '"' | '`' | '$' | '\\' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn exec_line(exe: &Path, root: Option<&str>, hidden: bool) -> String {
    let mut parts = Vec::new();
    // Development builds locate resources through LINE_GTK_ROOT; keep it across logins.
    if let Some(root) = root.filter(|r| !r.is_empty()) {
        parts.push("env".to_string());
        parts.push(exec_quote(&format!("LINE_GTK_ROOT={root}")));
    }
    parts.push(exec_quote(&exe.to_string_lossy()));
    if hidden {
        parts.push(BACKGROUND_FLAG.to_string());
    }
    parts.join(" ")
}

fn desktop_contents(exec: &str, name: &str, comment: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Comment={comment}\n\
         Exec={exec}\n\
         Icon=line-gtk\n\
         Terminal=false\n\
         Categories=Network;InstantMessaging;\n\
         StartupWMClass=dev.linegtk.LineGtk\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// Apply the user's choice. `hidden` only matters when `enabled`.
pub fn apply(enabled: bool, hidden: bool) -> Result<(), String> {
    if in_flatpak() {
        return apply_portal(enabled, hidden);
    }
    let path = entry_path().ok_or("no config directory")?;
    if !enabled {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let root = std::env::var("LINE_GTK_ROOT").ok();
    let exec = exec_line(&exe, root.as_deref(), hidden);
    let contents = desktop_contents(
        &exec,
        &crate::i18n::t("app_name"),
        "Native unofficial LINE client",
    );
    let dir = path.parent().ok_or("invalid autostart path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Skip the write when nothing changed so a login-time sync leaves the file alone.
    if std::fs::read_to_string(&path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

fn apply_portal(enabled: bool, hidden: bool) -> Result<(), String> {
    let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|e| e.message().to_string())?;
    let mut commandline = vec!["line-gtk".to_string()];
    if hidden {
        commandline.push(BACKGROUND_FLAG.to_string());
    }
    let options = glib::VariantDict::new(None);
    options.insert_value("autostart", &enabled.to_variant());
    options.insert_value("commandline", &commandline.to_variant());
    // Shown by the portal in its permission prompt.
    options.insert_value(
        "reason",
        &crate::i18n::t("autostart_portal_reason").to_variant(),
    );
    let params = glib::Variant::tuple_from_iter(["".to_variant(), options.end()]);
    conn.call_sync(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Background",
        "RequestBackground",
        Some(&params),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
    )
    .map(|_| ())
    .map_err(|e| e.message().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_are_not_quoted() {
        assert_eq!(exec_quote("/usr/bin/line-gtk"), "/usr/bin/line-gtk");
    }

    #[test]
    fn paths_with_spaces_and_specials_are_quoted() {
        assert_eq!(
            exec_quote("/opt/my app/line-gtk"),
            "\"/opt/my app/line-gtk\""
        );
        assert_eq!(exec_quote("/a/$b"), "\"/a/\\\\$b\"");
        assert_eq!(exec_quote("/a/100%"), "\"/a/100%%\"");
    }

    #[test]
    fn exec_line_includes_root_and_background_flag() {
        let exe = Path::new("/home/me/line-gtk/target/release/line-gtk");
        assert_eq!(
            exec_line(exe, Some("/home/me/line-gtk"), true),
            "env LINE_GTK_ROOT=/home/me/line-gtk /home/me/line-gtk/target/release/line-gtk --background"
        );
        assert_eq!(
            exec_line(Path::new("/usr/bin/line-gtk"), None, false),
            "/usr/bin/line-gtk"
        );
    }
}
