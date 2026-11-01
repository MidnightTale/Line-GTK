use super::*;
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Preview {
    title: String,
    description: String,
    image: Option<Vec<u8>>,
}

pub(super) fn append(bubble: &gtk::Box, text: &str) {
    for url in extract_urls(text).into_iter().take(4) {
        let Ok(parsed) = reqwest::Url::parse(&url) else {
            continue;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            continue;
        }
        let domain = parsed.host_str().unwrap_or(&url).to_string();
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .halign(gtk::Align::Start)
            .css_classes(["line-link-preview"])
            .build();
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .width_request(320)
            .height_request(180)
            .visible(false)
            .css_classes(["line-link-preview-image"])
            .build();
        let thumbnail_link = gtk::LinkButton::builder()
            .uri(&url)
            .halign(gtk::Align::Start)
            .visible(false)
            .css_classes(["flat", "line-link-preview-thumbnail"])
            .build();
        thumbnail_link.set_child(Some(&picture));
        card.append(&thumbnail_link);
        let domain_label = gtk::Label::builder()
            .label(&domain)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["line-link-preview-domain"])
            .build();
        card.append(&domain_label);
        let title = gtk::Label::builder()
            .label(if youtube_id(&url).is_some() {
                crate::i18n::t("youtube_video")
            } else {
                domain.clone()
            })
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(42)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["line-link-preview-title"])
            .build();
        card.append(&title);
        let description = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(42)
            .css_classes(["line-link-preview-description"])
            .build();
        card.append(&description);

        let actions = gtk::Box::builder()
            .spacing(8)
            .css_classes(["line-link-preview-actions"])
            .build();
        let load = gtk::Button::builder()
            .label(crate::i18n::t("link_preview_retry"))
            .visible(false)
            .css_classes(["flat"])
            .build();
        let open = gtk::LinkButton::builder()
            .label(crate::i18n::t("link_preview_open"))
            .uri(&url)
            .css_classes(["flat"])
            .build();
        actions.append(&load);
        actions.append(&open);
        card.append(&actions);
        bubble.append(&card);

        let title_c = title.clone();
        let description_c = description.clone();
        let picture_c = picture.clone();
        let thumbnail_link_c = thumbnail_link.clone();
        load.connect_clicked(move |button| {
            button.set_sensitive(false);
            button.set_label(&crate::i18n::t("link_preview_loading"));
            description_c.set_text(&crate::i18n::t("link_preview_loading"));
            let (tx, rx) = async_channel::bounded(1);
            let url = url.clone();
            let domain = domain.clone();
            queue_preview(url, domain, tx);
            let button = button.clone();
            let title = title_c.clone();
            let description = description_c.clone();
            let picture = picture_c.clone();
            let thumbnail_link = thumbnail_link_c.clone();
            glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok(preview)) => {
                        picture.set_alternative_text(Some(&preview.title));
                        title.set_text(&preview.title);
                        description.set_text(&compact_text(&preview.description, 160));
                        description.set_visible(!preview.description.is_empty());
                        if let Some(bytes) = preview.image
                            && let Ok(pixbuf) = Pixbuf::from_read(Cursor::new(bytes))
                        {
                            picture.set_size_request(pixbuf.width(), pixbuf.height());
                            title.set_max_width_chars(if pixbuf.height() > pixbuf.width() {
                                26
                            } else {
                                42
                            });
                            picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf)));
                            picture.set_visible(true);
                            thumbnail_link.set_visible(true);
                            description.set_visible(false);
                        }
                        button.set_visible(picture.paintable().is_none());
                        button.set_label(&crate::i18n::t("link_preview_retry"));
                        button.set_sensitive(true);
                    }
                    _ => {
                        description.set_visible(true);
                        description.set_text(&crate::i18n::t("link_preview_unavailable"));
                        button.set_label(&crate::i18n::t("link_preview_retry"));
                        button.set_sensitive(true);
                        button.set_visible(true);
                    }
                }
            });
        });
        // Mapping happens when the message enters the widget tree. Start only once,
        // including when GTK remaps the card after scrolling or switching tabs.
        let started = std::cell::Cell::new(false);
        let load_c = load.clone();
        card.connect_map(move |_| {
            if !started.replace(true) {
                description.set_text(&crate::i18n::t("link_preview_loading"));
                load_c.emit_clicked();
            }
        });
    }
}

type PreviewJob = (String, String, async_channel::Sender<Result<Preview, ()>>);

fn queue_preview(url: String, domain: String, reply: async_channel::Sender<Result<Preview, ()>>) {
    static QUEUE: OnceLock<async_channel::Sender<PreviewJob>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = async_channel::bounded::<PreviewJob>(256);
        // Bound network concurrency while keeping all I/O off the GTK thread.
        let cache = Arc::new(Mutex::new(HashMap::<String, (Instant, Preview)>::new()));
        for _ in 0..4 {
            let rx = rx.clone();
            let cache = cache.clone();
            std::thread::spawn(move || {
                while let Ok((url, domain, reply)) = rx.recv_blocking() {
                    if reply.is_closed() {
                        continue;
                    }
                    let cached = cache.lock().unwrap().get(&url).cloned();
                    let result = match cached {
                        Some((saved, preview)) if saved.elapsed() < Duration::from_secs(900) => {
                            Ok(preview)
                        }
                        _ => fetch(&url, &domain),
                    };
                    if let Ok(preview) = &result
                        && preview.image.is_some()
                    {
                        let mut cache = cache.lock().unwrap();
                        if cache.len() >= 128 {
                            cache.clear();
                        }
                        cache.insert(url, (Instant::now(), preview.clone()));
                    }
                    let _ = reply.send_blocking(result);
                }
            });
        }
        tx
    });
    // A full queue drops the reply sender, so the card exposes Retry.
    let _ = queue.try_send((url, domain, reply));
}

fn fetch(url: &str, domain: &str) -> Result<Preview, ()> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("Line-GTK/0.1 (link preview)")
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|_| ())?;
    let parsed = reqwest::Url::parse(url).map_err(|_| ())?;
    if let Some(preview) = fetch_oembed(&client, &parsed) {
        return Ok(preview);
    }
    // YouTube pages can require consent or JavaScript; their public thumbnail
    // still works when page metadata and oEmbed are unavailable.
    if let Some(id) = youtube_id(url) {
        let thumbnail = reqwest::Url::parse(&format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))
            .map_err(|_| ())?;
        if let Some(image) = fetch_image(&client, &thumbnail) {
            return Ok(Preview {
                title: crate::i18n::t("youtube_video"),
                description: String::new(),
                image: Some(image),
            });
        }
    }
    let response = client.get(url).send().map_err(|_| ())?;
    let page_url = response.url().clone();
    // TikTok short share links redirect to the canonical video URL required by
    // oEmbed. Try that even when the page itself returns a challenge/error.
    if page_url != parsed
        && let Some(preview) = fetch_oembed(&client, &page_url)
    {
        return Ok(preview);
    }
    let response = response.error_for_status().map_err(|_| ())?;
    let html_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.contains("text/html") || value.contains("application/xhtml+xml")
        });
    if !html_type {
        return Err(());
    }
    // Metadata is in the page head. Read a bounded prefix rather than rejecting
    // large video pages just because their entire document exceeds the limit.
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    let html = String::from_utf8_lossy(&bytes);
    let title = meta(&html, "og:title")
        .or_else(|| meta(&html, "twitter:title"))
        .or_else(|| page_title(&html))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| domain.to_string());
    let description = meta(&html, "og:description")
        .or_else(|| meta(&html, "description"))
        .or_else(|| meta(&html, "twitter:description"))
        .unwrap_or_default();
    let image = meta(&html, "og:image:secure_url")
        .or_else(|| meta(&html, "og:image"))
        .or_else(|| meta(&html, "twitter:image"))
        .or_else(|| meta(&html, "twitter:image:src"))
        .and_then(|url| page_url.join(&url).ok())
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && (page_url.scheme() != "https" || url.scheme() == "https")
        })
        .and_then(|url| fetch_image(&client, &url));
    Ok(Preview {
        title: compact_text(&title, 100),
        description: truncate(&description, 280),
        image,
    })
}

fn oembed_url(url: &reqwest::Url) -> Option<reqwest::Url> {
    let host = url.host_str()?;
    let endpoint = if youtube_id(url.as_str()).is_some() {
        "https://www.youtube.com/oembed"
    } else if host == "tiktok.com" || host.ends_with(".tiktok.com") {
        "https://www.tiktok.com/oembed"
    } else {
        return None;
    };
    let mut endpoint = reqwest::Url::parse(endpoint).ok()?;
    endpoint
        .query_pairs_mut()
        .append_pair("url", url.as_str())
        .append_pair("format", "json");
    Some(endpoint)
}

fn fetch_oembed(client: &reqwest::blocking::Client, url: &reqwest::Url) -> Option<Preview> {
    let endpoint = oembed_url(url)?;
    let response = client.get(endpoint).send().ok()?.error_for_status().ok()?;
    let mut bytes = Vec::new();
    response.take(64 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let data: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let thumbnail = reqwest::Url::parse(data.get("thumbnail_url")?.as_str()?).ok()?;
    if thumbnail.scheme() != "https" {
        return None;
    }
    Some(Preview {
        title: compact_text(data.get("title")?.as_str()?, 100),
        description: truncate(
            data.get("author_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            280,
        ),
        image: Some(fetch_image(client, &thumbnail)?),
    })
}

fn fetch_image(client: &reqwest::blocking::Client, url: &reqwest::Url) -> Option<Vec<u8>> {
    let response = client
        .get(url.clone())
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    let image_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("image/"));
    if !image_type
        || response
            .content_length()
            .is_some_and(|size| size > 2 * 1024 * 1024)
    {
        return None;
    }
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 2 * 1024 * 1024 {
        return None;
    }
    let reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width as u64 * height as u64 > 12_000_000
    {
        return None;
    }
    let thumbnail = image::load_from_memory(&bytes).ok()?.thumbnail(320, 360);
    let mut png = Cursor::new(Vec::new());
    thumbnail.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

fn meta(html: &str, key: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(offset) = lower[from..].find("<meta") {
        let start = from + offset;
        let end = tag_end(html, start)?;
        let tag = &html[start..=end];
        if attribute(tag, "property")
            .or_else(|| attribute(tag, "name"))
            .is_some_and(|name| name.eq_ignore_ascii_case(key))
        {
            return attribute(tag, "content").map(|value| entities(&value));
        }
        from = end + 1;
    }
    None
}

fn tag_end(html: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, byte) in html.as_bytes()[start..].iter().copied().enumerate() {
        match (quote, byte) {
            (None, b'\'' | b'"') => quote = Some(byte),
            (Some(current), byte) if current == byte => quote = None,
            (None, b'>') => return Some(start + offset),
            _ => {}
        }
    }
    None
}

fn page_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let content = lower[start..].find('>')? + start + 1;
    let end = lower[content..].find("</title>")? + content;
    Some(entities(html[content..end].trim()))
}

fn attribute(tag: &str, wanted: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
            i += 1;
        }
        if start == i {
            break;
        }
        let name = &tag[start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = bytes
            .get(i)
            .copied()
            .filter(|byte| *byte == b'"' || *byte == b'\'');
        if quote.is_some() {
            i += 1;
        }
        let value_start = i;
        while i < bytes.len()
            && match quote {
                Some(q) => bytes[i] != q,
                None => !bytes[i].is_ascii_whitespace() && bytes[i] != b'>',
            }
        {
            i += 1;
        }
        let value = &tag[value_start..i];
        if quote.is_some() && i < bytes.len() {
            i += 1;
        }
        if name.eq_ignore_ascii_case(wanted) {
            return Some(value.to_string());
        }
    }
    None
}

fn entities(value: &str) -> String {
    let mut result = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let decoded = rest.find(';').filter(|end| *end <= 12).and_then(|end| {
            let entity = &rest[..end];
            let character = match entity {
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                    .or_else(|| {
                        entity
                            .strip_prefix('#')
                            .and_then(|digits| digits.parse().ok())
                    })
                    .and_then(char::from_u32),
            }?;
            Some((end, character))
        });
        if let Some((end, character)) = decoded {
            result.push(character);
            rest = &rest[end + 1..];
        } else {
            result.push('&');
        }
    }
    result.push_str(rest);
    result.trim().to_string()
}

fn compact_text(value: &str, max: usize) -> String {
    truncate(&value.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

fn truncate(value: &str, max: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn social_metadata_decodes_numeric_entities_and_keeps_invalid_ones() {
        assert_eq!(
            entities("4,8&#xa0;tri&#x1ec7;u l&#x1b0;&#x1ee3;t xem"),
            "4,8\u{a0}triệu lượt xem"
        );
        assert_eq!(
            entities("I wasn&#x2019;t &#128578; &#39;ready&#39;"),
            "I wasn’t 🙂 'ready'"
        );
        assert_eq!(
            entities("&amp;#39; &#x110000; &#xD800; &unknown; &end"),
            "&#39; &#x110000; &#xD800; &unknown; &end"
        );
        assert_eq!(
            compact_text("  Have you...\n\nAd | Promotion. ", 100),
            "Have you... Ad | Promotion."
        );
    }

    #[test]
    fn youtube_video_urls_use_valid_ids_and_trusted_hosts() {
        for url in [
            "https://youtu.be/jNQXAC9IVRw?si=share",
            "https://www.youtube.com/watch?v=jNQXAC9IVRw&t=1",
            "https://youtube.com/shorts/jNQXAC9IVRw",
            "https://m.youtube.com/live/jNQXAC9IVRw",
            "https://www.youtube.com/embed/jNQXAC9IVRw",
        ] {
            assert_eq!(youtube_id(url).as_deref(), Some("jNQXAC9IVRw"));
            let endpoint = oembed_url(&reqwest::Url::parse(url).unwrap()).unwrap();
            assert_eq!(endpoint.host_str(), Some("www.youtube.com"));
            assert_eq!(
                endpoint
                    .query_pairs()
                    .find(|(key, _)| key == "url")
                    .unwrap()
                    .1,
                url
            );
        }
        for url in [
            "https://youtube.com.evil.example/watch?v=jNQXAC9IVRw",
            "https://example.com/youtu.be/jNQXAC9IVRw",
            "https://youtube.com/watch?v=../image",
            "https://youtube.com/watch?v=short",
            "https://youtube.com/playlist?list=abc",
        ] {
            assert!(youtube_id(url).is_none(), "{url}");
            assert!(
                oembed_url(&reqwest::Url::parse(url).unwrap()).is_none(),
                "{url}"
            );
        }
    }

    #[test]
    fn tiktok_oembed_accepts_share_urls_and_rejects_lookalike_hosts() {
        for url in [
            "https://www.tiktok.com/@creator/video/123456789",
            "https://vm.tiktok.com/share/",
            "https://vt.tiktok.com/share/",
        ] {
            let endpoint = oembed_url(&reqwest::Url::parse(url).unwrap()).unwrap();
            assert_eq!(endpoint.host_str(), Some("www.tiktok.com"));
        }
        for url in [
            "https://tiktok.com.evil.example/video/123",
            "https://faketiktok.com/video/123",
            "https://www.facebook.com/reel/123",
        ] {
            assert!(oembed_url(&reqwest::Url::parse(url).unwrap()).is_none());
        }
    }

    #[test]
    fn large_video_pages_still_load_relative_open_graph_thumbnails() {
        use std::io::Write;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut image_bytes = Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(640, 360)
                .write_to(&mut image_bytes, image::ImageFormat::Png)
                .unwrap();
            let html = format!(
                r#"<head><meta property="og:title" content="Video &amp; title">
                <meta property="og:image" content="/thumbnail.png"></head>{}"#,
                " ".repeat(300 * 1024)
            );
            for (content_type, body) in [
                ("text/html", html.into_bytes()),
                ("image/png", image_bytes.into_inner()),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).unwrap() > 0);
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let preview = fetch(&url, "localhost").unwrap();
        assert_eq!(preview.title, "Video & title");
        let thumbnail = image::load_from_memory(&preview.image.unwrap()).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (320, 180));
        server.join().unwrap();
    }

    #[test]
    fn reads_open_graph_metadata_with_attribute_order_and_quotes() {
        let html = r#"<meta content='A &amp; B' property='og:title'>
            <meta name="description" content="Short description"><title>Fallback</title>"#;
        assert_eq!(meta(html, "og:title").as_deref(), Some("A & B"));
        assert_eq!(
            meta(html, "description").as_deref(),
            Some("Short description")
        );
        assert_eq!(page_title(html).as_deref(), Some("Fallback"));
        assert_eq!(
            meta(r#"<meta property="og:title" content="A > B">"#, "og:title").as_deref(),
            Some("A > B")
        );
    }
}
