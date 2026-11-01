use super::*;

/// Fraction of the limit kept when old messages are cleared.
const KEEP_NUM: u64 = 1;
const KEEP_DEN: u64 = 2;

/// Messages to keep after "clear old": half the limit, or half the current
/// count when the limit is unlimited or already raised past it.
pub(super) fn keep_after_clear(count: u64, limit: u64) -> u64 {
    let basis = if limit == 0 { count } else { limit.min(count) };
    (basis * KEEP_NUM / KEEP_DEN).max(1)
}

pub(super) fn warn(state: &AppState, chat_mid: &str, count: u64, limit: u64, full: bool) {
    let name = peer_display_name(state, chat_mid);
    let heading = crate::i18n::t(if full {
        "cache_limit_full"
    } else {
        "cache_limit_near"
    });
    let body = crate::i18n::tf(
        "cache_limit_body",
        &[
            ("chat", &name),
            ("count", &count.to_string()),
            ("limit", &limit.to_string()),
        ],
    );
    let dialog = libadwaita::AlertDialog::new(Some(&heading), Some(&body));
    dialog.add_response("later", &crate::i18n::t("cache_limit_later"));
    dialog.add_response("clear", &crate::i18n::t("cache_limit_clear"));
    dialog.add_response("raise", &crate::i18n::t("cache_limit_raise"));
    dialog.set_default_response(Some("later"));
    dialog.set_close_response("later");
    dialog.set_response_appearance("clear", libadwaita::ResponseAppearance::Destructive);
    dialog.set_response_appearance("raise", libadwaita::ResponseAppearance::Suggested);
    let s = state.clone();
    let chat_mid = chat_mid.to_string();
    dialog.connect_response(None, move |_, response| match response {
        "raise" => {
            let mut cfg = s.config.borrow_mut();
            cfg.message_cache_limit =
                crate::config::next_message_cache_limit(cfg.message_cache_limit);
            cfg.save(&s.data_dir);
            let label = limit_label(cfg.message_cache_limit);
            drop(cfg);
            toast(
                &s,
                &crate::i18n::tf("cache_limit_raised", &[("limit", &label)]),
            );
        }
        "clear" => {
            let keep = keep_after_clear(count, limit);
            match s.sidecar.request(
                "trim_message_cache",
                Some(serde_json::json!({ "chatMid": chat_mid, "keep": keep })),
            ) {
                Ok(_) => toast(
                    &s,
                    &crate::i18n::tf("cache_limit_cleared", &[("keep", &keep.to_string())]),
                ),
                Err(_) => toast(&s, &crate::i18n::t("cache_limit_clear_failed")),
            }
        }
        _ => {}
    });
    dialog.present(Some(&state.window));
}

pub(super) fn limit_label(limit: u32) -> String {
    if limit == 0 {
        crate::i18n::t("cache_limit_unlimited")
    } else {
        limit.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_half_of_the_limit_but_never_more_than_exists() {
        assert_eq!(keep_after_clear(2000, 2000), 1000);
        assert_eq!(keep_after_clear(1800, 2000), 900);
        assert_eq!(keep_after_clear(300, 2000), 150);
        assert_eq!(keep_after_clear(5000, 0), 2500);
        assert_eq!(keep_after_clear(1, 2000), 1);
    }
}
