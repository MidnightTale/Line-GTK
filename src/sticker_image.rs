use anyhow::{Context, Result, ensure};
use gdk_pixbuf::{Colorspace, InterpType, Pixbuf};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

pub const CANVAS: i32 = 384;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageSize {
    Small,
    Medium,
    #[default]
    Large,
    Original,
}

impl ImageSize {
    pub fn from_selection(index: u32) -> Self {
        match index {
            0 => Self::Small,
            1 => Self::Medium,
            3 => Self::Original,
            _ => Self::Large,
        }
    }

    pub fn selection(self) -> u32 {
        match self {
            Self::Small => 0,
            Self::Medium => 1,
            Self::Large => 2,
            Self::Original => 3,
        }
    }

    pub fn pixels(self) -> Option<i32> {
        match self {
            Self::Small => Some(96),
            Self::Medium => Some(144),
            Self::Large => Some(224),
            Self::Original => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ImageOptions {
    pub size: ImageSize,
    pub remove_background: bool,
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            size: ImageSize::Large,
            remove_background: true,
        }
    }
}

fn read_options(data_dir: &Path) -> Result<BTreeMap<String, serde_json::Value>> {
    match std::fs::read(data_dir.join("sticker-image-options.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error.into()),
    }
}

// Downloaded stickers use their globally unique sticker ID as the file name.
#[cfg(test)]
fn image_key(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
pub fn options_for(data_dir: &Path, image: &Path) -> ImageOptions {
    options_for_key(data_dir, &image_key(image), ImageOptions::default())
}

pub fn options_for_key(data_dir: &Path, key: &str, fallback: ImageOptions) -> ImageOptions {
    read_options(data_dir)
        .ok()
        .and_then(|mut saved| saved.remove(key))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or(fallback)
}

/// Adopt legacy numeric sticker keys once, preserving newer per-item choices.
pub fn migrate_options(
    data_dir: &Path,
    aliases: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    let saved = read_options(data_dir)?;
    for (key, old) in aliases {
        if saved.contains_key(key) {
            continue;
        }
        if let Some(value) = old.as_str().and_then(|old| saved.get(old))
            && let Ok(options) = serde_json::from_value(value.clone())
        {
            remember_options_key(data_dir, key, options)?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub fn remember_options(data_dir: &Path, image: &Path, options: ImageOptions) -> Result<()> {
    remember_options_key(data_dir, &image_key(image), options)
}

pub fn remember_options_key(data_dir: &Path, key: &str, options: ImageOptions) -> Result<()> {
    let mut saved = read_options(data_dir)?;
    saved.insert(key.to_string(), serde_json::to_value(options)?);
    crate::config::ensure_private_dir(data_dir)?;
    let temporary = data_dir.join(format!(".sticker-image-options.{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(&saved)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, data_dir.join("sticker-image-options.json"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Remove only a nearly uniform, opaque background reachable from the edges.
/// Enclosed areas of the same colour belong to the artwork and are preserved.
fn remove_edge_background(rgba: &mut [u8], width: usize, height: usize) -> bool {
    let count = width * height;
    if width < 3 || height < 3 || rgba.len() != count * 4 {
        return false;
    }
    let corners = [0, width - 1, count - width, count - 1];
    let background = [rgba[0], rgba[1], rgba[2]];
    let matches = |index: usize| {
        let pixel = &rgba[index * 4..index * 4 + 4];
        pixel[3] >= 250 && (0..3).all(|channel| pixel[channel].abs_diff(background[channel]) <= 18)
    };
    if !corners.iter().all(|&index| matches(index)) {
        return false;
    }
    let mut seen = vec![false; count];
    let mut queue = Vec::new();
    let mut visit = |index: usize, queue: &mut Vec<usize>| {
        if !seen[index] && matches(index) {
            seen[index] = true;
            queue.push(index);
        }
    };
    for x in 0..width {
        visit(x, &mut queue);
        visit(count - width + x, &mut queue);
    }
    for y in 0..height {
        visit(y * width, &mut queue);
        visit(y * width + width - 1, &mut queue);
    }
    let mut cursor = 0;
    while cursor < queue.len() {
        let index = queue[cursor];
        cursor += 1;
        if index % width > 0 {
            visit(index - 1, &mut queue);
        }
        if index % width + 1 < width {
            visit(index + 1, &mut queue);
        }
        if index >= width {
            visit(index - width, &mut queue);
        }
        if index + width < count {
            visit(index + width, &mut queue);
        }
    }
    // Avoid turning an almost uniformly coloured image into an empty sticker.
    if queue.is_empty() || queue.len() * 100 > count * 98 {
        return false;
    }
    for index in queue {
        rgba[index * 4 + 3] = 0;
    }
    true
}

#[derive(Clone)]
pub struct PreparedImage {
    pub pixbuf: Pixbuf,
    pub background_removed: bool,
}

pub fn prepare(path: &Path, size: Option<i32>, remove_background: bool) -> Result<PreparedImage> {
    // Probe size before allocating the decoded pixel buffer.
    let (_, probe_width, probe_height) =
        Pixbuf::file_info(path).context("could not inspect image")?;
    ensure!(
        probe_width > 0 && probe_height > 0 && probe_width <= 2048 && probe_height <= 2048,
        "sticker image is too large to edit"
    );
    let source = Pixbuf::from_file(path)?;
    let width = source.width();
    let height = source.height();
    ensure!(
        width <= 2048 && height <= 2048,
        "sticker image is too large to edit"
    );
    let channels = source.n_channels() as usize;
    let stride = source.rowstride() as usize;
    ensure!(channels == 3 || channels == 4, "unsupported image channels");
    let bytes = source.read_pixel_bytes();
    let mut rgba = vec![0; width as usize * height as usize * 4];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let input = y * stride + x * channels;
            let output = (y * width as usize + x) * 4;
            rgba[output..output + 3].copy_from_slice(&bytes[input..input + 3]);
            rgba[output + 3] = if channels == 4 { bytes[input + 3] } else { 255 };
        }
    }
    let background_removed =
        remove_background && remove_edge_background(&mut rgba, width as usize, height as usize);
    let image = Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba),
        Colorspace::Rgb,
        true,
        8,
        width,
        height,
        width * 4,
    );
    let Some(size) = size else {
        return Ok(PreparedImage {
            pixbuf: image,
            background_removed,
        });
    };
    ensure!((32..=CANVAS).contains(&size), "invalid sticker image size");
    // Crop empty margins first so the size control measures the visible artwork.
    let pixels = image.read_pixel_bytes();
    let mut bounds = (width, height, -1, -1);
    for y in 0..height {
        for x in 0..width {
            if pixels[((y * width + x) * 4 + 3) as usize] > 8 {
                bounds.0 = bounds.0.min(x);
                bounds.1 = bounds.1.min(y);
                bounds.2 = bounds.2.max(x);
                bounds.3 = bounds.3.max(y);
            }
        }
    }
    ensure!(bounds.2 >= bounds.0, "image has no visible pixels");
    let crop = image.new_subpixbuf(
        bounds.0,
        bounds.1,
        bounds.2 - bounds.0 + 1,
        bounds.3 - bounds.1 + 1,
    );
    let ratio = f64::from(size) / f64::from(crop.width().max(crop.height()));
    let scaled_width = (f64::from(crop.width()) * ratio).round().max(1.0) as i32;
    let scaled_height = (f64::from(crop.height()) * ratio).round().max(1.0) as i32;
    let scaled = crop
        .scale_simple(scaled_width, scaled_height, InterpType::Bilinear)
        .context("could not resize sticker image")?;
    let canvas = Pixbuf::new(Colorspace::Rgb, true, 8, CANVAS, CANVAS)
        .context("could not create transparent canvas")?;
    canvas.fill(0);
    scaled.copy_area(
        0,
        0,
        scaled_width,
        scaled_height,
        &canvas,
        (CANVAS - scaled_width) / 2,
        (CANVAS - scaled_height) / 2,
    );
    Ok(PreparedImage {
        pixbuf: canvas,
        background_removed,
    })
}

pub fn save(image: &PreparedImage, path: &Path) -> Result<()> {
    image.pixbuf.savev(path, "png", &[])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_settings_are_independent_and_migration_does_not_overwrite_them() {
        let directory =
            std::env::temp_dir().join(format!("line-gtk-library-settings-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let original = ImageOptions {
            size: ImageSize::Original,
            remove_background: false,
        };
        let small = ImageOptions {
            size: ImageSize::Small,
            remove_background: true,
        };
        remember_options_key(&directory, "10.png", small).unwrap();
        remember_options_key(&directory, "account/pack-a/item", original).unwrap();
        let aliases =
            serde_json::json!({ "account/pack-a/item": "10.png", "account/pack-b/item": "10.png" });
        migrate_options(&directory, aliases.as_object().unwrap()).unwrap();
        assert_eq!(
            options_for_key(&directory, "account/pack-a/item", small),
            original
        );
        assert_eq!(
            options_for_key(&directory, "account/pack-b/item", original),
            small
        );
        migrate_options(&directory, aliases.as_object().unwrap()).unwrap();
        assert_eq!(
            options_for_key(&directory, "account/pack-a/item", small),
            original
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn last_options_persist_per_image_and_new_images_default_to_large_with_removal() {
        let directory =
            std::env::temp_dir().join(format!("line-gtk-sticker-options-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let first = Path::new("cache/stickers/100.png");
        let second = Path::new("cache/stickers/200.png");
        assert_eq!(
            options_for(&directory, first),
            ImageOptions {
                size: ImageSize::Large,
                remove_background: true
            }
        );
        let custom = ImageOptions {
            size: ImageSize::Small,
            remove_background: false,
        };
        remember_options(&directory, first, custom).unwrap();
        assert_eq!(options_for(&directory, first), custom);
        assert_eq!(options_for(&directory, second), ImageOptions::default());
        let original = ImageOptions {
            size: ImageSize::Original,
            remove_background: false,
        };
        remember_options(&directory, second, original).unwrap();
        remember_options(
            &directory,
            first,
            ImageOptions {
                size: ImageSize::Medium,
                ..custom
            },
        )
        .unwrap();
        assert_eq!(options_for(&directory, first).size, ImageSize::Medium);
        assert_eq!(options_for(&directory, second), original);
        // Identity follows the sticker ID, independently of its cache directory.
        assert_eq!(
            options_for(&directory, Path::new("new-cache/200.png")),
            original
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(directory.join("sticker-image-options.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_saved_image_options_do_not_replace_other_images_preferences() {
        let directory = std::env::temp_dir().join(format!(
            "line-gtk-sticker-options-invalid-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let saved = directory.join("sticker-image-options.json");
        std::fs::write(&saved, br#"{"100.png":{"size":"unknown"},"200.png":{"size":"original","remove_background":false}}"#).unwrap();
        assert_eq!(
            options_for(&directory, Path::new("100.png")),
            ImageOptions::default()
        );
        assert_eq!(
            options_for(&directory, Path::new("200.png")).size,
            ImageSize::Original
        );
        remember_options(&directory, Path::new("300.png"), ImageOptions::default()).unwrap();
        assert_eq!(
            options_for(&directory, Path::new("200.png")).size,
            ImageSize::Original
        );
        std::fs::write(&saved, b"invalid json").unwrap();
        assert_eq!(
            options_for(&directory, Path::new("100.png")),
            ImageOptions::default()
        );
        assert!(
            remember_options(&directory, Path::new("100.png"), ImageOptions::default()).is_err()
        );
        assert_eq!(std::fs::read(&saved).unwrap(), b"invalid json");
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn sample() -> Vec<u8> {
        let mut pixels = vec![255; 9 * 9 * 4];
        // A black outline around a white interior, on a white background.
        for y in 2..7 {
            for x in 2..7 {
                if x == 2 || x == 6 || y == 2 || y == 6 {
                    let i = (y * 9 + x) * 4;
                    pixels[i..i + 3].fill(0);
                }
            }
        }
        pixels
    }

    #[test]
    fn removes_edge_white_without_erasing_enclosed_white_or_black_artwork() {
        let mut pixels = sample();
        assert!(remove_edge_background(&mut pixels, 9, 9));
        assert_eq!(pixels[3], 0);
        assert_eq!(pixels[(4 * 9 + 4) * 4 + 3], 255);
        assert_eq!(
            &pixels[(2 * 9 + 2) * 4..(2 * 9 + 2) * 4 + 4],
            &[0, 0, 0, 255]
        );
    }

    #[test]
    fn preserves_existing_transparency_and_mismatched_corner_backgrounds() {
        for corner in [[255, 255, 255, 0], [20, 40, 60, 255]] {
            let mut pixels = sample();
            pixels[..4].copy_from_slice(&corner);
            let original = pixels.clone();
            assert!(!remove_edge_background(&mut pixels, 9, 9));
            assert_eq!(pixels, original);
        }
    }

    #[test]
    fn does_not_erase_a_uniform_image_and_keeps_partial_alpha_inside_artwork() {
        let mut uniform = vec![255; 9 * 9 * 4];
        assert!(!remove_edge_background(&mut uniform, 9, 9));
        assert!(uniform.iter().all(|&value| value == 255));
        let mut pixels = sample();
        pixels[(4 * 9 + 4) * 4 + 3] = 100;
        assert!(remove_edge_background(&mut pixels, 9, 9));
        assert_eq!(pixels[(4 * 9 + 4) * 4 + 3], 100);
    }

    #[test]
    fn prepared_png_has_transparent_padding_correct_size_and_preserves_original() {
        static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "line-gtk-sticker-image-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("source.png");
        let output = directory.join("edited.png");
        let source = Pixbuf::new(Colorspace::Rgb, false, 8, 20, 10).unwrap();
        source.fill(0x203040ff);
        source.savev(&input, "png", &[]).unwrap();
        let original = std::fs::read(&input).unwrap();
        for size in [96, 144, 224] {
            let prepared = prepare(&input, Some(size), false).unwrap();
            assert_eq!(
                (prepared.pixbuf.width(), prepared.pixbuf.height()),
                (CANVAS, CANVAS)
            );
            assert!(!prepared.background_removed);
            save(&prepared, &output).unwrap();
            let saved = Pixbuf::from_file(&output).unwrap();
            assert!(saved.has_alpha());
            let bytes = saved.read_pixel_bytes();
            assert_eq!(bytes[3], 0);
            let alpha = |x, y| bytes[(y * saved.rowstride() + x * 4 + 3) as usize];
            let left = (CANVAS - size) / 2;
            let top = (CANVAS - size / 2) / 2;
            assert_eq!(alpha(left, top), 255);
            assert_eq!(alpha(left - 1, top), 0);
            assert_eq!(alpha(left + size, top), 0);
            assert_eq!(alpha(left, top - 1), 0);
            assert_eq!(alpha(left, top + size / 2), 0);
        }
        let unchanged = prepare(&input, None, false).unwrap();
        assert_eq!(
            (unchanged.pixbuf.width(), unchanged.pixbuf.height()),
            (20, 10)
        );
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert!(prepare(&input, Some(CANVAS + 1), false).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
