use std::path::{Path, PathBuf};

pub fn unique_download_dest(dir: &Path, suggested_name: &str) -> PathBuf {
    let safe = sanitize_filename(suggested_name);
    let candidate = dir.join(&safe);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(&safe)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = Path::new(&safe)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    for sequence in 1..10_000 {
        let alternative = dir.join(format!("{stem}-{sequence}{extension}"));
        if !alternative.exists() {
            return alternative;
        }
    }
    dir.join(format!("{stem}-dup{extension}"))
}

pub fn sanitize_filename(suggested_name: &str) -> String {
    let safe = suggested_name
        .chars()
        .map(|character| {
            if matches!(character, '/' | '\\' | '\0') || character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let safe = safe.trim().trim_matches('.');
    if safe.is_empty() || matches!(safe, "." | "..") {
        "download.bin".into()
    } else {
        safe.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_paths_and_control_characters() {
        assert_eq!(sanitize_filename("../../secret\n.txt"), "_.._secret_.txt");
        assert_eq!(sanitize_filename(".."), "download.bin");
        assert_eq!(sanitize_filename(""), "download.bin");
    }

    #[test]
    fn chooses_a_non_overwriting_destination() {
        let dir =
            std::env::temp_dir().join(format!("line-gtk-download-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("photo.jpg"), b"existing").unwrap();
        assert_eq!(
            unique_download_dest(&dir, "photo.jpg"),
            dir.join("photo-1.jpg")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
