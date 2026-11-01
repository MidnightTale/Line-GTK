use super::*;
use crate::protocol::LineEmoji;

#[derive(Clone)]
pub(super) struct EmojiText {
    view: gtk::TextView,
    text: String,
    emojis: Vec<LineEmoji>,
    mentions: Vec<crate::protocol::Mention>,
}

impl EmojiText {
    pub(super) fn is_inside(&self, ancestor: &gtk::Widget) -> bool {
        self.view.is_ancestor(ancestor)
    }
}

fn ranges(text: &str, emojis: &[LineEmoji]) -> Vec<(usize, usize, String)> {
    let mut boundaries: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
    boundaries.push(text.len());
    let mut previous = 0;
    emojis
        .iter()
        .take(100)
        .filter_map(|emoji| {
            if emoji.start < previous || emoji.end <= emoji.start {
                return None;
            }
            let (&start, &end) = (boundaries.get(emoji.start)?, boundaries.get(emoji.end)?);
            previous = emoji.end;
            Some((start, end, emoji.key.clone()))
        })
        .collect()
}

fn shifts(
    textures: &HashMap<String, gdk::Texture>,
    text: &str,
    emojis: &[LineEmoji],
) -> Vec<(usize, usize)> {
    ranges(text, emojis)
        .into_iter()
        .filter(|(_, _, key)| textures.contains_key(key))
        .map(|(start, end, _)| {
            (
                text[..end].chars().count(),
                text[start..end].chars().count() - 1,
            )
        })
        .collect()
}

/// Maps a buffer offset (emoji images count as one char) back to a message text offset.
fn original_offset(buffer_offset: usize, shifts: &[(usize, usize)]) -> usize {
    let mut shifted = 0;
    for (end, n) in shifts {
        if buffer_offset >= end.saturating_sub(shifted + n) {
            shifted += n;
        } else {
            break;
        }
    }
    buffer_offset + shifted
}

fn mention_in_view(
    view: &gtk::TextView,
    textures: &HashMap<String, gdk::Texture>,
    content: &EmojiText,
    x: f64,
    y: f64,
) -> Option<String> {
    let (bx, by) = view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
    let iter = view.iter_at_location(bx, by)?;
    let offset = original_offset(
        iter.offset().max(0) as usize,
        &shifts(textures, &content.text, &content.emojis),
    );
    mentions::mention_at(&content.mentions, offset).map(str::to_string)
}

fn wire_click(state: &AppState, content: &EmojiText) {
    let click = gtk::GestureClick::builder()
        .button(1)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let press = Rc::new(std::cell::Cell::new((0.0, 0.0)));
    {
        let press = press.clone();
        click.connect_pressed(move |gesture, _, x, y| {
            // Conversation text is not a selection surface. Claim the press
            // before TextView starts selecting or moves its insertion cursor.
            gesture.set_state(gtk::EventSequenceState::Claimed);
            press.set((x, y));
        });
    }
    {
        let (state, content) = (state.clone(), content.clone());
        click.connect_released(move |gesture, n, x, y| {
            let (px, py) = press.get();
            if n != 1 || (x - px).abs() > 4.0 || (y - py).abs() > 4.0 {
                return;
            }
            let textures = state.emoji_textures.borrow();
            if let Some(mid) = mention_in_view(&content.view, &textures, &content, x, y) {
                drop(textures);
                gesture.set_state(gtk::EventSequenceState::Claimed);
                user_profile::open(&state, &mid);
            }
        });
    }
    content.view.add_controller(click);
    let motion = gtk::EventControllerMotion::new();
    {
        let (textures, content) = (state.emoji_textures.clone(), content.clone());
        motion.connect_motion(move |_, x, y| {
            let over = mention_in_view(&content.view, &textures.borrow(), &content, x, y).is_some();
            content
                .view
                .set_cursor_from_name(Some(if over { "pointer" } else { "default" }));
        });
    }
    content.view.add_controller(motion);
}

fn render(textures: &HashMap<String, gdk::Texture>, content: &EmojiText) {
    let buffer = content.view.buffer();
    buffer.set_text("");
    let mut iter = buffer.end_iter();
    let mut previous = 0;
    let mut measured = String::new();
    let mut images = 0;
    let mut shifts: Vec<(usize, usize)> = Vec::new();
    for (start, end, key) in ranges(&content.text, &content.emojis) {
        buffer.insert(&mut iter, &content.text[previous..start]);
        measured.push_str(&content.text[previous..start]);
        if let Some(texture) = textures.get(&key) {
            buffer.insert_paintable(&mut iter, texture);
            shifts.push((
                content.text[..end].chars().count(),
                content.text[start..end].chars().count() - 1,
            ));
            measured.push('　');
            images += 1;
        } else {
            // 取得失敗時も代替テキストを残し、本文を欠落させない。
            buffer.insert(&mut iter, &content.text[start..end]);
            measured.push_str(&content.text[start..end]);
        }
        previous = end;
    }
    buffer.insert(&mut iter, &content.text[previous..]);
    measured.push_str(&content.text[previous..]);
    let tag = buffer.tag_table().lookup("mention").unwrap_or_else(|| {
        let tag = gtk::TextTag::builder()
            .name("mention")
            .weight(700)
            .underline(gtk::pango::Underline::Single)
            .build();
        buffer.tag_table().add(&tag);
        tag
    });
    tag.set_foreground_rgba(Some(&mentions::color(&content.view)));
    for mention in &content.mentions {
        let adjust = |offset: usize| {
            offset.saturating_sub(
                shifts
                    .iter()
                    .filter(|(end, _)| *end <= offset)
                    .map(|(_, n)| *n)
                    .sum::<usize>(),
            )
        };
        if mention.start < mention.end && mention.end <= content.text.chars().count() {
            buffer.apply_tag(
                &tag,
                &buffer.iter_at_offset(adjust(mention.start) as i32),
                &buffer.iter_at_offset(adjust(mention.end) as i32),
            );
        }
    }
    let layout = content.view.create_pango_layout(Some(&measured));
    let width = (layout.pixel_size().0 + images * 14).clamp(28, 340);
    content.view.set_width_request(width);
}

pub(super) fn append(state: &AppState, bubble: &gtk::Box, message: &MessageInfo) {
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .focusable(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .css_classes(["line-bubble-text", "line-emoji-text"])
        .build();
    view.set_cursor_from_name(Some("default"));
    view.update_property(&[gtk::accessible::Property::Label(&message.text)]);
    let content = EmojiText {
        view: view.clone(),
        text: message.text.clone(),
        emojis: message.emojis.clone(),
        mentions: message.mentions.clone(),
    };
    render(&state.emoji_textures.borrow(), &content);
    // 親の日本語フォントが適用された後に幅を測り直す。
    let textures = state.emoji_textures.clone();
    let weak = view.downgrade();
    let text = message.text.clone();
    let emojis = message.emojis.clone();
    let mentions = message.mentions.clone();
    view.connect_map(move |_| {
        if let Some(view) = weak.upgrade() {
            render(
                &textures.borrow(),
                &EmojiText {
                    view,
                    text: text.clone(),
                    emojis: emojis.clone(),
                    mentions: mentions.clone(),
                },
            );
        }
    });
    wire_click(state, &content);
    state.emoji_views.borrow_mut().push(content);
    bubble.append(&view);
}

pub(super) fn ready(state: &AppState, key: String, path: String) {
    if state.emoji_textures.borrow().contains_key(&key) {
        return;
    }
    let Ok(pixbuf) = Pixbuf::from_file_at_scale(path, 28, 28, true) else {
        return;
    };
    state
        .emoji_textures
        .borrow_mut()
        .insert(key.clone(), gdk::Texture::for_pixbuf(&pixbuf));
    for content in state.emoji_views.borrow().iter() {
        if content.emojis.iter().any(|emoji| emoji.key == key) {
            render(&state.emoji_textures.borrow(), content);
        }
    }
    reactions::emoji_ready(state, &key);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buffer_offsets_map_back_past_emoji_images() {
        // "a(smile)b@c": emoji occupies chars 1..8 and renders as one image char.
        let shifts = vec![(8, 6)];
        assert_eq!(original_offset(0, &shifts), 0);
        assert_eq!(original_offset(1, &shifts), 1);
        assert_eq!(original_offset(2, &shifts), 8);
        assert_eq!(original_offset(4, &shifts), 10);
    }
    #[test]
    fn emoji_ranges_keep_unicode_and_reject_overlapping_or_invalid_offsets() {
        let text = "😀前(顔)後";
        let emojis = vec![
            LineEmoji {
                start: 2,
                end: 5,
                key: "face".into(),
            },
            LineEmoji {
                start: 3,
                end: 6,
                key: "overlap".into(),
            },
            LineEmoji {
                start: 6,
                end: 999,
                key: "outside".into(),
            },
        ];
        let ranges = ranges(text, &emojis);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&text[..ranges[0].0], "😀前");
        assert_eq!(&text[ranges[0].0..ranges[0].1], "(顔)");
        assert_eq!(&text[ranges[0].1..], "後");
    }
}
