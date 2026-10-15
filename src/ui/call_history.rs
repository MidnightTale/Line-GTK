use crate::protocol::{CallHistory, MessageInfo};
use gtk::prelude::*;

struct HistoryCard {
    title_key: &'static str,
    icon_name: &'static str,
    duration: Option<String>,
    missed: bool,
}

fn legacy_duration(text: &str) -> Option<i64> {
    let (duration, result) = text
        .trim()
        .strip_prefix("Call History : ")?
        .split_once(" millisecs, Result: ")?;
    if !result.chars().all(|c| c.is_ascii_digit()) || result.is_empty() {
        return None;
    }
    duration.parse::<i64>().ok().filter(|value| *value >= 0)
}

fn card_data(message: &MessageInfo) -> Option<HistoryCard> {
    if !message.content_type.eq_ignore_ascii_case("CALL") {
        return None;
    }
    let fallback = CallHistory {
        video: false,
        status: "unknown".into(),
        duration_ms: message
            .duration_ms
            .filter(|value| *value >= 0)
            .or_else(|| legacy_duration(&message.text)),
    };
    let history = message.call_history.as_ref().unwrap_or(&fallback);
    let duration = history.duration_ms.filter(|value| *value > 0).map(|value| {
        let seconds = value / 1000;
        let minutes = seconds / 60;
        if minutes >= 60 {
            format!("{}:{:02}:{:02}", minutes / 60, minutes % 60, seconds % 60)
        } else {
            format!("{}:{:02}", minutes, seconds % 60)
        }
    });
    // 時間ゼロや未知の数値コードから、応答の有無を推測しない。
    let unanswered = duration.is_none() && history.status == "unanswered";
    let title_key = match (history.video, message.mine, unanswered) {
        (false, false, false) => "call_history_incoming",
        (false, true, false) => "call_history_outgoing",
        (false, false, true) => "call_history_missed",
        (false, true, true) => "call_history_unanswered",
        (true, false, false) => "call_history_video_incoming",
        (true, true, false) => "call_history_video_outgoing",
        (true, false, true) => "call_history_video_missed",
        (true, true, true) => "call_history_video_unanswered",
    };
    let missed = unanswered && !message.mine;
    let icon_name = if history.video {
        "camera-video-symbolic"
    } else if missed {
        "call-missed-symbolic"
    } else if message.mine {
        "call-outgoing-symbolic"
    } else {
        "call-incoming-symbolic"
    };
    Some(HistoryCard {
        title_key,
        icon_name,
        duration,
        missed,
    })
}

pub(super) fn history_preview(message: &MessageInfo) -> Option<String> {
    let card = card_data(message)?;
    let title = crate::i18n::t(card.title_key);
    Some(match card.duration {
        Some(duration) => format!("{title} · {duration}"),
        None => title,
    })
}

pub(super) fn append_history_card(bubble: &gtk::Box, message: &MessageInfo) {
    let Some(card) = card_data(message) else {
        return;
    };
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(14)
        .css_classes(["line-history-card"])
        .build();
    if card.missed {
        row.add_css_class("line-history-missed");
    }
    let icon = gtk::Image::builder()
        .icon_name(card.icon_name)
        .pixel_size(28)
        .valign(gtk::Align::Center)
        .css_classes(["line-history-icon"])
        .build();
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .valign(gtk::Align::Center)
        .build();
    text.append(
        &gtk::Label::builder()
            .label(crate::i18n::t(card.title_key))
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(24)
            .selectable(true)
            .css_classes(["line-history-title"])
            .build(),
    );
    if let Some(duration) = card.duration {
        text.append(
            &gtk::Label::builder()
                .label(crate::i18n::tf(
                    "call_history_duration",
                    &[("duration", &duration)],
                ))
                .xalign(0.0)
                .selectable(true)
                .css_classes(["line-history-duration"])
                .build(),
        );
    }
    row.append(&icon);
    row.append(&text);
    bubble.append(&row);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(value: serde_json::Value) -> MessageInfo {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn old_incoming_and_outgoing_records_keep_direction_and_duration() {
        let mut incoming = message(serde_json::json!({
            "contentType": "CALL", "text": "Call History : 136000 millisecs, Result: 16"
        }));
        let card = card_data(&incoming).unwrap();
        assert_eq!(card.title_key, "call_history_incoming");
        assert_eq!(card.icon_name, "call-incoming-symbolic");
        assert_eq!(card.duration.as_deref(), Some("2:16"));
        incoming.mine = true;
        assert_eq!(
            card_data(&incoming).unwrap().title_key,
            "call_history_outgoing"
        );
    }

    #[test]
    fn unknown_zero_duration_is_not_assumed_to_be_missed() {
        let incoming = message(serde_json::json!({
            "contentType": "CALL", "text": "Call History : 0 millisecs, Result: 77", "durationMs": 0
        }));
        let card = card_data(&incoming).unwrap();
        assert_eq!(card.title_key, "call_history_incoming");
        assert!(!card.missed);
        assert!(card.duration.is_none());
    }

    #[test]
    fn explicit_missed_video_and_long_calls_have_specific_presentations() {
        let missed = message(serde_json::json!({
            "contentType": "CALL", "callHistory": { "video": true, "status": "unanswered", "durationMs": 0 }
        }));
        let card = card_data(&missed).unwrap();
        assert_eq!(card.title_key, "call_history_video_missed");
        assert!(card.missed);
        let completed = message(serde_json::json!({
            "contentType": "CALL", "durationMs": 3723000
        }));
        assert_eq!(
            card_data(&completed).unwrap().duration.as_deref(),
            Some("1:02:03")
        );
    }

    #[test]
    fn regular_text_is_not_converted_into_call_history() {
        let text = message(serde_json::json!({
            "contentType": "NONE", "text": "Call History : 136000 millisecs, Result: 16"
        }));
        assert!(card_data(&text).is_none());
    }
}
