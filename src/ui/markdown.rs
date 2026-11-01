//! Native Markdown preview. Only generated, escaped Pango markup reaches GTK.
use gtk::prelude::*;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::io::Read;

const LIMIT: u64 = 256 * 1024;

pub(super) fn is_markdown(name: &str, path: &str) -> bool {
    // Prefer the original filename: cache paths may have unrelated extensions.
    let candidate = if name.is_empty() { path } else { name };
    std::path::Path::new(candidate)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("md") || s.eq_ignore_ascii_case("markdown"))
}

fn safe_link(url: &str) -> bool {
    // Attachments cannot launch local files or custom application protocols.
    reqwest::Url::parse(url).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
            || url.scheme() == "mailto"
    })
}

fn escape(text: &str) -> String {
    // XML 1.0 disallows NUL and most control characters even when escaped.
    let clean: String = text
        .chars()
        .map(|c| match c {
            '\t'
            | '\n'
            | '\r'
            | '\u{20}'..='\u{d7ff}'
            | '\u{e000}'..='\u{fffd}'
            | '\u{10000}'..='\u{10ffff}' => c,
            _ => '�',
        })
        .collect();
    gtk::glib::markup_escape_text(&clean).to_string()
}

fn render(source: &str) -> String {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut out = String::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut links = Vec::new();
    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    let scale = match level as u8 {
                        1 => "xx-large",
                        2 => "x-large",
                        3 => "large",
                        _ => "medium",
                    };
                    out.push_str(&format!("<span size=\"{scale}\"><b>"));
                }
                Tag::Strong => out.push_str("<b>"),
                Tag::Emphasis => out.push_str("<i>"),
                Tag::Strikethrough => out.push_str("<s>"),
                Tag::CodeBlock(_) => out.push_str("<tt>"),
                Tag::BlockQuote(_) => out.push_str("<i>│ "),
                Tag::List(start) => lists.push(start),
                Tag::Item => {
                    out.push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                    if let Some(Some(index)) = lists.last_mut() {
                        out.push_str(&format!("{index}. "));
                        *index = index.saturating_add(1);
                    } else {
                        out.push_str("• ");
                    }
                }
                Tag::Link { dest_url, .. } => {
                    let safe = safe_link(&dest_url);
                    links.push(safe);
                    if safe {
                        out.push_str(&format!("<a href=\"{}\">", escape(&dest_url)));
                    }
                }
                // Images show their alt text without fetching local or remote files.
                Tag::Table(_) => out.push_str("<tt>"),
                Tag::TableHead => out.push_str("<b>"),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => out.push_str("</b></span>\n\n"),
                TagEnd::Strong => out.push_str("</b>"),
                TagEnd::Emphasis => out.push_str("</i>"),
                TagEnd::Strikethrough => out.push_str("</s>"),
                TagEnd::CodeBlock => out.push_str("</tt>\n\n"),
                TagEnd::BlockQuote(_) => out.push_str("</i>\n"),
                TagEnd::Paragraph => out.push_str("\n\n"),
                TagEnd::Item => {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                TagEnd::List(_) => {
                    lists.pop();
                    out.push('\n');
                }
                TagEnd::Link => {
                    if links.pop() == Some(true) {
                        out.push_str("</a>");
                    }
                }
                TagEnd::TableCell => out.push_str("  │  "),
                TagEnd::TableRow => out.push('\n'),
                TagEnd::TableHead => out.push_str("</b>\n"),
                TagEnd::Table => out.push_str("</tt>\n\n"),
                _ => {}
            },
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                out.push_str(&escape(&text));
            }
            Event::Code(text) => out.push_str(&format!("<tt>{}</tt>", escape(&text))),
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("\n────────────\n\n"),
            Event::TaskListMarker(done) => out.push_str(if done { "☑ " } else { "☐ " }),
            _ => {}
        }
    }
    out.trim_end().to_string()
}

fn read_source(path: &str) -> std::io::Result<(String, bool)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() as u64 > LIMIT;
    bytes.truncate(LIMIT as usize);
    Ok((
        String::from_utf8_lossy(&bytes).replace('\0', "�"),
        truncated,
    ))
}

pub(super) fn append(body: &gtk::Box, path: &str) {
    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    let switcher = gtk::StackSwitcher::builder()
        .stack(&stack)
        .halign(gtk::Align::Center)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let preview = gtk::Label::builder()
        .xalign(0.0)
        .yalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .margin_start(24)
        .margin_end(24)
        .margin_top(20)
        .margin_bottom(20)
        .label(crate::i18n::t("media_markdown_loading"))
        .build();
    preview.connect_activate_link(|_, uri| {
        if safe_link(uri) {
            super::open_uri(uri);
        }
        gtk::glib::Propagation::Stop
    });
    let preview_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&preview)
        .build();
    let source_view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(14)
        .right_margin(14)
        .top_margin(12)
        .bottom_margin(12)
        .css_classes(["line-media-viewer-text"])
        .build();
    let source_scroll = gtk::ScrolledWindow::builder().child(&source_view).build();
    stack.add_titled(
        &preview_scroll,
        Some("preview"),
        &crate::i18n::t("media_markdown_preview"),
    );
    stack.add_titled(
        &source_scroll,
        Some("source"),
        &crate::i18n::t("media_markdown_source"),
    );
    body.append(&switcher);
    body.append(&stack);

    // Reading and parsing stay off the GTK main thread; reads are bounded.
    let path = path.to_string();
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = read_source(&path).map(|(text, truncated)| {
            let markup = render(&text);
            (text, markup, truncated)
        });
        let _ = tx.send_blocking(result);
    });
    let preview = preview.downgrade();
    let source_view = source_view.downgrade();
    gtk::glib::spawn_future_local(async move {
        let Ok(result) = rx.recv().await else { return };
        let (Some(preview), Some(source_view)) = (preview.upgrade(), source_view.upgrade()) else {
            return;
        };
        match result {
            Ok((text, markup, truncated)) => {
                source_view.buffer().set_text(&text);
                if truncated {
                    let notice = crate::i18n::t("media_text_truncated");
                    preview.set_markup(&format!("{}\n\n{markup}", escape(&notice)));
                    source_view
                        .buffer()
                        .set_text(&format!("{notice}\n\n{text}"));
                } else {
                    preview.set_markup(&markup);
                }
            }
            Err(error) => {
                let text = crate::i18n::tf("file_read_failed", &[("error", &error.to_string())]);
                preview.set_text(&text);
                source_view.buffer().set_text(&text);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(source: &str) -> (String, String) {
        let markup = render(source);
        // GtkLabel handles anchors before passing the remaining markup to Pango.
        let mut pango = markup.replace("</a>", "</span>");
        while let Some(start) = pango.find("<a href=") {
            let end = start + pango[start..].find('>').unwrap() + 1;
            pango.replace_range(start..end, "<span underline=\"single\">");
        }
        let (_, text, _) = gtk::pango::parse_markup(&pango, '\0').unwrap();
        (markup, text.to_string())
    }

    #[test]
    fn nested_blocks_and_untrusted_markup_stay_valid() {
        for source in [
            "> # Heading\n>\n> **bold** [link](https://example.org)",
            "- outer\n  - inner\n\n    paragraph\n\n  ```\n  <a href='file:///'>\n  ```",
            "**<span foreground='red'>injected</span>**\n\n\u{1}\u{fffe}",
            "[**bold _link_**](https://example.org/?q=\"quote\")",
        ] {
            rendered(source);
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn native_preview_loads_and_switches_to_source() {
        gtk::init().unwrap();
        let path =
            std::env::temp_dir().join(format!("line-gtk-markdown-ui-{}.md", std::process::id()));
        let source = "# Xin chào\n\n**Đậm**, _nghiêng_, [link](https://example.org).\n\n```rs\nlet x = 1;\n```";
        std::fs::write(&path, source).unwrap();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        append(&body, path.to_str().unwrap());
        let stack = body.last_child().unwrap().downcast::<gtk::Stack>().unwrap();
        let scroll = stack
            .child_by_name("preview")
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let preview = scroll
            .child()
            .unwrap()
            .downcast::<gtk::Viewport>()
            .unwrap()
            .child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let context = gtk::glib::MainContext::default();
        while !preview.text().starts_with("Xin chào") && std::time::Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(preview.text().contains("Đậm, nghiêng, link."));
        stack.set_visible_child_name("source");
        assert_eq!(stack.visible_child_name().as_deref(), Some("source"));
        let scroll = stack
            .child_by_name("source")
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let view = scroll.child().unwrap().downcast::<gtk::TextView>().unwrap();
        let buffer = view.buffer();
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
            source
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn renders_common_markdown_and_unicode_as_valid_pango() {
        let (markup, text) = rendered(
            "# Xin chào\n\n**đậm _nghiêng_** và `a < b`\n\n3. Ba\n4. Bốn\n\n> Trích dẫn\n\n```rs\nlet x = 1;\n```\n\n- [x] Xong\n\n~~cũ~~\n\n| A | B |\n|---|---|\n| 1 | 2 |\n",
        );
        assert!(markup.contains("<b>đậm <i>nghiêng</i></b>"));
        for expected in [
            "Xin chào",
            "a < b",
            "3. Ba",
            "4. Bốn",
            "Trích dẫn",
            "let x = 1;",
            "☑ Xong",
            "1  │  2",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in {text:?}");
        }
    }

    #[test]
    fn escapes_html_and_only_allows_web_and_mail_links() {
        let (markup, text) = rendered(
            "<script>alert('x')</script>\n\n[web](https://example.org/?a=1&b=2) [bad](javascript:alert) [local](file:///etc/passwd) ![alt](https://example.org/image.png)\n\nA\0B",
        );
        assert!(text.contains("<script>"));
        assert!(text.contains("alt"));
        assert!(markup.contains("href=\"https://example.org/?a=1&amp;b=2\""));
        assert!(!markup.contains("href=\"javascript:"));
        assert!(!markup.contains("href=\"file:"));
        assert!(safe_link("mailto:test@example.org"));
        assert!(!safe_link("../local.txt"));
    }

    #[test]
    fn recognizes_only_markdown_extensions_and_prefers_original_name() {
        assert!(is_markdown("README.MD", "/cache/blob"));
        assert!(is_markdown("notes.markdown", ""));
        assert!(is_markdown("", "/cache/notes.md"));
        assert!(!is_markdown("notes.md.exe", ""));
        assert!(!is_markdown("page.html", "/cache/notes.md"));
        assert!(!is_markdown("photo.png", "/cache/readme.md/data"));
    }

    #[test]
    fn bounds_large_reads_and_handles_missing_files() {
        let path =
            std::env::temp_dir().join(format!("line-gtk-markdown-test-{}", std::process::id()));
        std::fs::write(&path, vec![b'a'; LIMIT as usize + 100]).unwrap();
        let (text, truncated) = read_source(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(truncated);
        assert_eq!(text.len(), LIMIT as usize);
        assert!(read_source(path.to_str().unwrap()).is_err());
    }
}
