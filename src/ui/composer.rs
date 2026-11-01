use gtk::prelude::*;
use gtk::{gdk, glib};

// Keep text and cursor operations in character offsets, as mention spans expect.
pub(super) trait ComposerExt {
    fn text(&self) -> glib::GString;
    fn set_text(&self, text: &str);
    fn position(&self) -> i32;
    fn set_position(&self, position: i32);
}

impl ComposerExt for gtk::TextView {
    fn text(&self) -> glib::GString {
        let buffer = self.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), true)
    }

    fn set_text(&self, text: &str) {
        self.buffer().set_text(text);
    }

    fn position(&self) -> i32 {
        self.buffer().cursor_position()
    }

    fn set_position(&self, position: i32) {
        let buffer = self.buffer();
        let position = if position < 0 {
            buffer.char_count()
        } else {
            position.min(buffer.char_count())
        };
        buffer.place_cursor(&buffer.iter_at_offset(position));
    }
}

pub(super) fn wire_send(
    view: &gtk::TextView,
    send: impl Fn() + 'static,
) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    let weak = view.downgrade();
    keys.connect_key_pressed(move |controller, key, _, modifiers| {
        if !matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
            || modifiers.intersects(
                gdk::ModifierType::SHIFT_MASK
                    | gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            )
        {
            return glib::Propagation::Proceed;
        }
        // Let the input method commit a composition without sending the message.
        if let Some(view) = weak.upgrade()
            && let Some(event) = controller.current_event()
            && view.im_context_filter_keypress(event)
        {
            return glib::Propagation::Stop;
        }
        send();
        glib::Propagation::Stop
    });
    view.add_controller(keys.clone());
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn multiline_composer_preserves_text_and_only_sends_on_enter() {
        gtk::init().unwrap();
        let view = gtk::TextView::new();
        let sent = Rc::new(Cell::new(0));
        let count = sent.clone();
        let keys = wire_send(&view, move || count.set(count.get() + 1));
        for key in [gdk::Key::Return, gdk::Key::KP_Enter] {
            let handled = keys.emit_by_name::<bool>(
                "key-pressed",
                &[&key, &0u32, &gdk::ModifierType::SHIFT_MASK],
            );
            assert!(!handled, "Shift+Enter must reach GTK's newline binding");
            assert_eq!(sent.get(), 0);
        }

        // GTK's insertion action retains line breaks and Unicode cursor offsets.
        view.set_text("Hi 👋");
        view.set_position(-1);
        view.emit_insert_at_cursor("\nNext line");
        assert_eq!(view.text(), "Hi 👋\nNext line");
        assert_eq!(view.position(), 14);
        view.set_position(5);
        view.emit_insert_at_cursor("中");
        assert_eq!(view.text(), "Hi 👋\n中Next line");

        for (index, key) in [gdk::Key::Return, gdk::Key::KP_Enter]
            .into_iter()
            .enumerate()
        {
            assert!(
                keys.emit_by_name::<bool>(
                    "key-pressed",
                    &[&key, &0u32, &gdk::ModifierType::empty()],
                )
            );
            assert_eq!(sent.get(), index + 1);
        }
        view.set_text("");
        assert!(view.text().is_empty());
    }
}
