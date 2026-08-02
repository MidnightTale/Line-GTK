//! Decode LINE animated stickers (APNG) into RGBA frames.
//! gdk-pixbuf cannot load APNG, so we composite frames with the `png` crate.
//! Raw frames are `Send` so decoding can run off the GTK thread.
//!
//! Static images use `Pixbuf::from_file_at_scale` so JPEG/PNG are never fully
//! decoded at native resolution just to show a chat thumbnail.

use gdk_pixbuf::Pixbuf;
use std::fs::File;
use std::io::{BufReader, Read};
use std::sync::{Condvar, Mutex, OnceLock};

#[derive(Clone)]
pub struct RawFrame {
    pub rgba: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub delay_ms: u32,
}

#[derive(Clone)]
pub struct AnimFrames {
    pub frames: Vec<RawFrame>,
    /// 0 = loop forever (LINE chat UX).
    pub plays: u32,
}

/// Cap parallel image/APNG decodes so chat open cannot spike hundreds of MiB.
struct DecodeLimiter {
    active: Mutex<u32>,
    cv: Condvar,
    max: u32,
}

fn decode_limiter() -> &'static DecodeLimiter {
    static LIMITER: OnceLock<DecodeLimiter> = OnceLock::new();
    LIMITER.get_or_init(|| DecodeLimiter {
        active: Mutex::new(0),
        cv: Condvar::new(),
        max: 2,
    })
}

fn with_decode_slot<R>(f: impl FnOnce() -> R) -> R {
    let lim = decode_limiter();
    let mut guard = lim.active.lock().unwrap_or_else(|e| e.into_inner());
    while *guard >= lim.max {
        guard = lim.cv.wait(guard).unwrap_or_else(|e| e.into_inner());
    }
    *guard += 1;
    drop(guard);
    let out = f();
    let mut guard = lim.active.lock().unwrap_or_else(|e| e.into_inner());
    *guard = guard.saturating_sub(1);
    lim.cv.notify_one();
    out
}

pub fn is_apng_file(path: &str) -> bool {
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 512];
    let Ok(n) = f.read(&mut buf) else {
        return false;
    };
    if n < 24 || &buf[0..8] != b"\x89PNG\r\n\x1a\n" {
        return false;
    }
    let mut i = 8usize;
    while i + 8 <= n {
        let len = u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as usize;
        let ctype = &buf[i + 4..i + 8];
        if ctype == b"acTL" {
            return true;
        }
        if ctype == b"IDAT" || ctype == b"IEND" {
            break;
        }
        let next = i.saturating_add(12).saturating_add(len);
        if next <= i {
            break;
        }
        i = next;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes.windows(4).any(|w| w == b"acTL")
}

/// Load a chat/sticker image scaled to `max_px`.
/// When `animate` is false, APNG returns only the first composited frame.
pub fn load_scaled(path: &str, max_px: i32, animate: bool) -> Option<AnimFrames> {
    with_decode_slot(|| load_scaled_inner(path, max_px, animate))
}

/// Decode into an exact logical canvas. `cover` center-crops photos; contain
/// keeps the full image on a transparent canvas (stickers/icons). Returning
/// fixed-size frames prevents GtkPicture from re-negotiating the row size from
/// the source image's intrinsic dimensions after the async decode completes.
pub fn load_fitted(
    path: &str,
    width_px: i32,
    height_px: i32,
    cover: bool,
    animate: bool,
) -> Option<AnimFrames> {
    with_decode_slot(|| {
        let width_px = width_px.max(1);
        let height_px = height_px.max(1);
        let decode_max = Pixbuf::file_info(path)
            .map(|(_, source_w, source_h)| {
                let source_w = source_w.max(1) as f64;
                let source_h = source_h.max(1) as f64;
                let sx = width_px as f64 / source_w;
                let sy = height_px as f64 / source_h;
                let scale = if cover { sx.max(sy) } else { sx.min(sy) };
                ((source_w * scale).max(source_h * scale)).ceil() as i32
            })
            .unwrap_or_else(|| width_px.max(height_px) * 2)
            .max(width_px.max(height_px));
        let mut decoded = load_scaled_inner(path, decode_max, animate)?;
        decoded.frames = decoded
            .frames
            .into_iter()
            .filter_map(|frame| fit_frame(frame, width_px, height_px, cover))
            .collect();
        (!decoded.frames.is_empty()).then_some(decoded)
    })
}

fn load_scaled_inner(path: &str, max_px: i32, animate: bool) -> Option<AnimFrames> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() < 32 {
        return None;
    }
    // Reject HTML leftovers cached from expired bot preview URLs.
    let mut hdr = [0u8; 16];
    if let Ok(mut f) = File::open(path) {
        let _ = f.read(&mut hdr);
        let is_jpeg = hdr[0] == 0xff && hdr[1] == 0xd8 && hdr[2] == 0xff;
        let is_png = hdr[0] == 0x89 && hdr[1] == 0x50 && hdr[2] == 0x4e && hdr[3] == 0x47;
        let is_gif = hdr[0] == 0x47 && hdr[1] == 0x49 && hdr[2] == 0x46;
        let is_webp = hdr[0] == 0x52
            && hdr[1] == 0x49
            && hdr[2] == 0x46
            && hdr[3] == 0x46
            && hdr[8] == 0x57
            && hdr[9] == 0x45
            && hdr[10] == 0x42
            && hdr[11] == 0x50;
        if !(is_jpeg || is_png || is_gif || is_webp) {
            return None;
        }
    }
    if is_apng_file(path) {
        return decode_apng(path, max_px, animate);
    }
    decode_static(path, max_px)
}

fn decode_static(path: &str, max_px: i32) -> Option<AnimFrames> {
    let max = if max_px > 0 { max_px } else { 2048 };
    let pb = Pixbuf::from_file_at_scale(path, max, max, true).ok()?;
    let w = pb.width();
    let h = pb.height();
    if w <= 0 || h <= 0 {
        return None;
    }
    let rgba = pixbuf_to_rgba(&pb)?;
    Some(AnimFrames {
        frames: vec![RawFrame {
            rgba,
            width: w,
            height: h,
            delay_ms: 0,
        }],
        plays: 1,
    })
}

/// Decode a profile image into an exact, center-cropped RGBA square. Only the
/// Send-safe raw bytes leave the worker thread; GTK objects stay on GTK's thread.
pub fn load_square(path: &str, size_px: i32) -> Option<RawFrame> {
    load_fitted(path, size_px, size_px, true, false)?
        .frames
        .into_iter()
        .next()
}

fn fit_frame(frame: RawFrame, width_px: i32, height_px: i32, cover: bool) -> Option<RawFrame> {
    let source_w = frame.width.max(1) as u32;
    let source_h = frame.height.max(1) as u32;
    let target_w = width_px.max(1) as u32;
    let target_h = height_px.max(1) as u32;
    let source = image::RgbaImage::from_raw(source_w, source_h, frame.rgba)?;
    let sx = target_w as f64 / source_w as f64;
    let sy = target_h as f64 / source_h as f64;
    let scale = if cover { sx.max(sy) } else { sx.min(sy) };
    let resized_w = ((source_w as f64 * scale).round() as u32).max(1);
    let resized_h = ((source_h as f64 * scale).round() as u32).max(1);
    let resized = image::imageops::resize(
        &source,
        resized_w,
        resized_h,
        image::imageops::FilterType::Triangle,
    );
    let rgba = if cover {
        let x = resized_w.saturating_sub(target_w) / 2;
        let y = resized_h.saturating_sub(target_h) / 2;
        image::imageops::crop_imm(&resized, x, y, target_w, target_h)
            .to_image()
            .into_raw()
    } else {
        let mut canvas = image::RgbaImage::new(target_w, target_h);
        let x = target_w.saturating_sub(resized_w) / 2;
        let y = target_h.saturating_sub(resized_h) / 2;
        image::imageops::overlay(&mut canvas, &resized, i64::from(x), i64::from(y));
        canvas.into_raw()
    };
    Some(RawFrame {
        rgba,
        width: target_w as i32,
        height: target_h as i32,
        delay_ms: frame.delay_ms,
    })
}

fn pixbuf_to_rgba(pb: &Pixbuf) -> Option<Vec<u8>> {
    let w = pb.width() as usize;
    let h = pb.height() as usize;
    let n_ch = pb.n_channels() as usize;
    let stride = pb.rowstride() as usize;
    let bytes = pb.read_pixel_bytes();
    let src = bytes.as_ref();
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let row = y * stride;
        for x in 0..w {
            let s = row + x * n_ch;
            let d = (y * w + x) * 4;
            if s + n_ch.min(3) > src.len() {
                return None;
            }
            rgba[d] = src[s];
            rgba[d + 1] = src.get(s + 1).copied().unwrap_or(0);
            rgba[d + 2] = src.get(s + 2).copied().unwrap_or(0);
            rgba[d + 3] = if n_ch >= 4 {
                src.get(s + 3).copied().unwrap_or(255)
            } else {
                255
            };
        }
    }
    Some(rgba)
}

fn decode_apng(path: &str, max_px: i32, animate: bool) -> Option<AnimFrames> {
    let file = File::open(path).ok()?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    let full_w = info.width as usize;
    let full_h = info.height as usize;
    if full_w == 0 || full_h == 0 || full_w * full_h > 2048 * 2048 {
        return None;
    }
    let plays = info.animation_control.map(|a| a.num_plays).unwrap_or(0);

    let scale = if max_px > 0 {
        (max_px as f64 / full_w.max(full_h) as f64).min(1.0)
    } else {
        1.0
    };
    let tw = ((full_w as f64) * scale).round().max(1.0) as u32;
    let th = ((full_h as f64) * scale).round().max(1.0) as u32;

    let mut canvas = vec![0u8; full_w * full_h * 4];
    let mut frames: Vec<RawFrame> = Vec::new();

    while let Some(buf_size) = reader.output_buffer_size() {
        let mut buf = vec![0u8; buf_size];
        let out = match reader.next_frame(&mut buf) {
            Ok(o) => o,
            Err(_) => break,
        };
        let Some(fc) = reader.info().frame_control else {
            if frames.is_empty() {
                let rgba = expand_to_rgba(
                    &buf[..out.buffer_size()],
                    out.color_type,
                    out.width,
                    out.height,
                )?;
                let rgba = if tw != full_w as u32 || th != full_h as u32 {
                    let img = image::RgbaImage::from_raw(out.width, out.height, rgba)?;
                    image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle)
                        .into_raw()
                } else {
                    rgba
                };
                frames.push(RawFrame {
                    rgba,
                    width: tw as i32,
                    height: th as i32,
                    delay_ms: 100,
                });
            }
            break;
        };

        let fw = out.width as usize;
        let fh = out.height as usize;
        let xo = fc.x_offset as usize;
        let yo = fc.y_offset as usize;
        if xo + fw > full_w || yo + fh > full_h {
            break;
        }

        let before = match fc.dispose_op {
            png::DisposeOp::Previous => Some(canvas.clone()),
            _ => None,
        };

        let src = match out.color_type {
            png::ColorType::Rgba => buf[..out.buffer_size()].to_vec(),
            png::ColorType::Rgb => {
                let mut rgba = vec![0u8; fw * fh * 4];
                for i in 0..fw * fh {
                    rgba[i * 4] = buf[i * 3];
                    rgba[i * 4 + 1] = buf[i * 3 + 1];
                    rgba[i * 4 + 2] = buf[i * 3 + 2];
                    rgba[i * 4 + 3] = 255;
                }
                rgba
            }
            _ => continue,
        };

        match fc.blend_op {
            png::BlendOp::Source => {
                for row in 0..fh {
                    let s = row * fw * 4;
                    let d = ((yo + row) * full_w + xo) * 4;
                    canvas[d..d + fw * 4].copy_from_slice(&src[s..s + fw * 4]);
                }
            }
            png::BlendOp::Over => {
                for row in 0..fh {
                    for col in 0..fw {
                        let si = (row * fw + col) * 4;
                        let di = ((yo + row) * full_w + (xo + col)) * 4;
                        let src_a = src[si + 3] as u32;
                        if src_a == 0 {
                            continue;
                        }
                        if src_a == 255 {
                            canvas[di..di + 4].copy_from_slice(&src[si..si + 4]);
                            continue;
                        }
                        let dst_a = canvas[di + 3] as u32;
                        let out_a = src_a + (dst_a * (255 - src_a) + 127) / 255;
                        for c in 0..3 {
                            let s = src[si + c] as u32;
                            let d = canvas[di + c] as u32;
                            canvas[di + c] = ((s * src_a + d * dst_a * (255 - src_a) / 255 + 127)
                                / out_a.max(1)) as u8;
                        }
                        canvas[di + 3] = out_a as u8;
                    }
                }
            }
        }

        let delay_ms = {
            let den = if fc.delay_den == 0 {
                100u32
            } else {
                fc.delay_den as u32
            };
            ((fc.delay_num as u32) * 1000 / den).clamp(20, 5000)
        };
        // Scale immediately so we never retain a full-res frame list in RAM.
        let rgba = if tw != full_w as u32 || th != full_h as u32 {
            let img = image::RgbaImage::from_raw(full_w as u32, full_h as u32, canvas.clone())?;
            image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle).into_raw()
        } else {
            canvas.clone()
        };
        frames.push(RawFrame {
            rgba,
            width: tw as i32,
            height: th as i32,
            delay_ms,
        });

        // Static display: keep first composited frame only.
        if !animate {
            break;
        }

        match fc.dispose_op {
            png::DisposeOp::None => {}
            png::DisposeOp::Background => {
                for row in 0..fh {
                    let d = ((yo + row) * full_w + xo) * 4;
                    canvas[d..d + fw * 4].fill(0);
                }
            }
            png::DisposeOp::Previous => {
                if let Some(prev) = before {
                    canvas = prev;
                }
            }
        }
    }

    if frames.is_empty() {
        return None;
    }

    Some(AnimFrames {
        frames,
        plays: if !animate || plays == 1 { 1 } else { 0 },
    })
}

fn expand_to_rgba(
    buf: &[u8],
    color_type: png::ColorType,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let n = (width as usize).checked_mul(height as usize)?;
    match color_type {
        png::ColorType::Rgba => Some(buf[..n * 4].to_vec()),
        png::ColorType::Rgb => {
            let mut rgba = vec![0u8; n * 4];
            for i in 0..n {
                rgba[i * 4] = buf[i * 3];
                rgba[i * 4 + 1] = buf[i * 3 + 1];
                rgba[i * 4 + 2] = buf[i * 3 + 2];
                rgba[i * 4 + 3] = 255;
            }
            Some(rgba)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn striped_frame() -> RawFrame {
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for value in [10, 20, 30, 40] {
                rgba.extend_from_slice(&[value, 0, 0, 255]);
            }
        }
        RawFrame {
            rgba,
            width: 4,
            height: 2,
            delay_ms: 75,
        }
    }

    #[test]
    fn cover_produces_exact_center_cropped_canvas() {
        let frame = fit_frame(striped_frame(), 2, 2, true).unwrap();
        assert_eq!((frame.width, frame.height), (2, 2));
        assert_eq!(frame.rgba.len(), 2 * 2 * 4);
        assert_eq!(frame.rgba[0], 20);
        assert_eq!(frame.rgba[4], 30);
        assert_eq!(frame.delay_ms, 75);
    }

    #[test]
    fn contain_produces_exact_transparent_letterbox_canvas() {
        let frame = fit_frame(striped_frame(), 4, 4, false).unwrap();
        assert_eq!((frame.width, frame.height), (4, 4));
        assert_eq!(frame.rgba.len(), 4 * 4 * 4);
        assert_eq!(frame.rgba[3], 0);
        assert_eq!(frame.rgba[(4 * 4) + 3], 255);
        assert_eq!(frame.rgba[(3 * 4 * 4) + 3], 0);
    }
}
