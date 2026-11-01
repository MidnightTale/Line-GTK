//! Native card for LINE's own poll messages (the Flex bubble that links to the
//! poll LIFF app). Reads and votes through the sidecar instead of the browser.

use super::*;
use crate::protocol::FlexInfo;
use serde_json::Value;
use std::cell::Cell;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const POLL_LIFF_ID: &str = "1477715170-Pl2JnXpR";
const CACHE_FRESH: Duration = Duration::from_secs(20);
const CACHE_LIMIT: usize = 256;
/// Cards drawn from the startup cache can outlive the login by many seconds.
const RETRY_EVERY: Duration = Duration::from_secs(2);
const MAX_RETRIES: u8 = 30;

thread_local! {
    static CACHE: RefCell<HashMap<(String, String), (Instant, Value)>> =
        RefCell::new(HashMap::new());
}

pub(super) struct PollCard {
    account: String,
    chat_mid: String,
    poll_id: String,
    title: gtk::Label,
    meta: gtk::Label,
    choices: gtk::Box,
    vote: gtk::Button,
    status: gtk::Label,
    selected: RefCell<BTreeSet<String>>,
    multiple: Cell<bool>,
    closed: Cell<bool>,
    busy: Cell<bool>,
    rebuilding: Cell<bool>,
    retries: Cell<u8>,
}

/// The poll link inside a Flex message, if this is a LINE poll card.
pub(super) fn poll_uri(flex: &FlexInfo) -> Option<&str> {
    flex.actions
        .iter()
        .filter_map(|a| a.uri.as_deref())
        .find(|uri| uri.contains(POLL_LIFF_ID) && poll_id(uri).is_some())
}

fn poll_id(uri: &str) -> Option<String> {
    let rest = uri.split("/view/").nth(1)?;
    let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!id.is_empty()).then_some(id)
}

fn date_label(ms: i64) -> String {
    glib::DateTime::from_unix_local(ms / 1000)
        .ok()
        .and_then(|d| d.format("%d/%m/%Y").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

pub(super) fn append_card(state: &AppState, bubble: &gtk::Box, flex: &FlexInfo, uri: &str) -> bool {
    let Some(poll_id) = poll_id(uri) else {
        return false;
    };
    let account = state.self_mid.borrow().clone().unwrap_or_default();
    let chat_mid = state.current_chat.borrow().clone().unwrap_or_default();
    if account.is_empty() || chat_mid.is_empty() {
        return false;
    }

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["line-flex-card", "line-poll-card"])
        .build();
    let title = gtk::Label::builder()
        .label(&flex.alt_text)
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(40)
        .css_classes(["line-flex-title"])
        .build();
    let meta = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(40)
        .css_classes(["dim-label", "caption"])
        .build();
    let choices = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    let status = gtk::Label::builder()
        .label(crate::i18n::t("poll_loading"))
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(40)
        .css_classes(["dim-label", "caption"])
        .build();
    let vote = gtk::Button::builder()
        .label(crate::i18n::t("poll_vote"))
        .hexpand(true)
        .sensitive(false)
        .css_classes(["suggested-action", "line-flex-action"])
        .build();
    let browser = gtk::Button::builder()
        .label(crate::i18n::t("poll_open_browser"))
        .css_classes(["flat", "caption"])
        .halign(gtk::Align::Start)
        .build();
    {
        let uri = uri.to_string();
        browser.connect_clicked(move |_| open_uri(&uri));
    }

    root.append(&title);
    root.append(&meta);
    root.append(&choices);
    root.append(&vote);
    root.append(&status);
    root.append(&browser);
    bubble.append(&root);

    let card = Rc::new(PollCard {
        account,
        chat_mid,
        poll_id,
        title,
        meta,
        choices,
        vote,
        status,
        selected: RefCell::new(BTreeSet::new()),
        multiple: Cell::new(false),
        closed: Cell::new(false),
        busy: Cell::new(false),
        rebuilding: Cell::new(false),
        retries: Cell::new(0),
    });
    // The card owns child widgets only; the root keeps the card alive until
    // the message row is destroyed, so no reference cycle outlives the row.
    {
        let keep = card.clone();
        root.connect_destroy(move |_| {
            let _ = &keep;
        });
    }
    {
        let state = state.clone();
        let weak = Rc::downgrade(&card);
        card.vote.connect_clicked(move |_| {
            if let Some(card) = weak.upgrade() {
                submit(&state, &card);
            }
        });
    }

    let key = (card.account.clone(), card.poll_id.clone());
    let cached = CACHE.with(|c| c.borrow().get(&key).cloned());
    match cached {
        Some((at, poll)) => {
            render(&card, &poll);
            if at.elapsed() >= CACHE_FRESH {
                fetch(state, &card);
            }
        }
        None => fetch(state, &card),
    }
    true
}

fn fetch(state: &AppState, card: &Rc<PollCard>) {
    // Before the login finishes the sidecar has no session; wait instead of
    // showing a transient "not logged in" error.
    if !*state.session_ready.borrow() {
        schedule_retry(state, card);
        return;
    }
    match state.sidecar.poll_get(&card.chat_mid, &card.poll_id) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::Poll {
                    card: Rc::downgrade(card),
                    vote: false,
                },
            );
        }
        Err(e) => card.status.set_text(&failed("poll_failed", &e.to_string())),
    }
}

/// Re-check shortly; stops when the row is gone or after a bounded wait.
fn schedule_retry(state: &AppState, card: &Rc<PollCard>) {
    if card.retries.get() >= MAX_RETRIES {
        card.status
            .set_text(&failed("poll_failed", "not_logged_in"));
        return;
    }
    card.retries.set(card.retries.get() + 1);
    let state = state.clone();
    let weak = Rc::downgrade(card);
    glib::timeout_add_local_once(RETRY_EVERY, move || {
        if let Some(card) = weak.upgrade() {
            fetch(&state, &card);
        }
    });
}

fn submit(state: &AppState, card: &Rc<PollCard>) {
    if card.busy.get() || card.closed.get() || card.selected.borrow().is_empty() {
        return;
    }
    let ids: Vec<String> = card.selected.borrow().iter().cloned().collect();
    card.busy.set(true);
    card.vote.set_sensitive(false);
    card.status.set_text(&crate::i18n::t("poll_sending"));
    match state.sidecar.poll_vote(&card.chat_mid, &card.poll_id, &ids) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::Poll {
                    card: Rc::downgrade(card),
                    vote: true,
                },
            );
        }
        Err(e) => {
            card.busy.set(false);
            refresh_vote(card);
            card.status
                .set_text(&failed("poll_vote_failed", &e.to_string()));
        }
    }
}

pub(super) fn response(
    state: &AppState,
    card: &std::rc::Weak<PollCard>,
    vote: bool,
    ok: bool,
    result: &Value,
    error: Option<&str>,
) {
    let Some(card) = card.upgrade() else {
        return;
    };
    // A card built for another account must never show or send this result.
    if state.self_mid.borrow().as_deref() != Some(card.account.as_str()) {
        return;
    }
    card.busy.set(false);
    if !ok && !vote && error.is_some_and(is_not_logged_in) {
        schedule_retry(state, &card);
        return;
    }
    if !ok {
        let key = if vote {
            "poll_vote_failed"
        } else {
            "poll_failed"
        };
        card.status
            .set_text(&failed(key, error.unwrap_or("unknown")));
        refresh_vote(&card);
        return;
    }
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(
            (card.account.clone(), card.poll_id.clone()),
            (Instant::now(), result.clone()),
        );
    });
    render(&card, result);
    if vote {
        card.status.set_text(&crate::i18n::t("poll_voted"));
    }
}

fn is_not_logged_in(error: &str) -> bool {
    error.contains("not_logged_in")
}

fn failed(key: &str, error: &str) -> String {
    crate::i18n::tf(key, &[("error", error)])
}

fn refresh_vote(card: &PollCard) {
    card.vote.set_sensitive(
        !card.busy.get() && !card.closed.get() && !card.selected.borrow().is_empty(),
    );
}

fn render(card: &Rc<PollCard>, poll: &Value) {
    let title = poll["title"].as_str().unwrap_or_default();
    if !title.is_empty() {
        card.title.set_text(title);
    }
    let multiple = poll["multiple"].as_bool().unwrap_or(false);
    let closed = poll["closed"].as_bool().unwrap_or(false);
    card.multiple.set(multiple);
    card.closed.set(closed);

    let voters = poll["voterCount"].as_u64().unwrap_or(0);
    let mut meta = vec![crate::i18n::tf(
        "poll_votes",
        &[("count", &voters.to_string())],
    )];
    if poll["anonymous"].as_bool().unwrap_or(false) {
        meta.push(crate::i18n::t("poll_anonymous"));
    }
    if multiple {
        meta.push(crate::i18n::t("poll_multiple"));
    }
    if closed {
        meta.push(crate::i18n::t("poll_closed"));
    } else if let Some(ms) = poll["closeDate"].as_i64() {
        meta.push(crate::i18n::tf("poll_closes", &[("date", &date_label(ms))]));
    }
    card.meta.set_text(&meta.join(" · "));

    let items = poll["choices"].as_array().cloned().unwrap_or_default();
    *card.selected.borrow_mut() = items
        .iter()
        .filter(|c| c["votedByMe"].as_bool().unwrap_or(false))
        .filter_map(|c| c["id"].as_str().map(str::to_string))
        .collect();

    card.rebuilding.set(true);
    while let Some(child) = card.choices.first_child() {
        card.choices.remove(&child);
    }
    let mut group: Option<gtk::CheckButton> = None;
    for item in &items {
        let Some(id) = item["id"].as_str().map(str::to_string) else {
            continue;
        };
        let votes = item["voterCount"].as_u64().unwrap_or(0);
        let check = gtk::CheckButton::builder()
            .label(item["text"].as_str().unwrap_or_default())
            .hexpand(true)
            .sensitive(!closed)
            .active(card.selected.borrow().contains(&id))
            .build();
        if !multiple {
            match &group {
                Some(first) => check.set_group(Some(first)),
                None => group = Some(check.clone()),
            }
        }
        let weak = Rc::downgrade(card);
        check.connect_toggled(move |check| {
            let Some(card) = weak.upgrade() else {
                return;
            };
            if card.rebuilding.get() {
                return;
            }
            {
                let mut selected = card.selected.borrow_mut();
                if check.is_active() {
                    if !card.multiple.get() {
                        selected.clear();
                    }
                    selected.insert(id.clone());
                } else {
                    selected.remove(&id);
                }
            }
            refresh_vote(&card);
        });
        let count = gtk::Label::builder()
            .label(votes.to_string())
            .css_classes(["dim-label"])
            .build();
        let top = gtk::Box::builder().spacing(8).build();
        top.append(&check);
        top.append(&count);
        let bar = gtk::ProgressBar::builder()
            .fraction(if voters == 0 {
                0.0
            } else {
                (votes as f64 / voters as f64).clamp(0.0, 1.0)
            })
            .build();
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .css_classes(["line-poll-choice"])
            .build();
        row.append(&top);
        row.append(&bar);
        card.choices.append(&row);
    }
    card.rebuilding.set(false);

    card.vote.set_visible(!closed);
    card.vote.set_label(&crate::i18n::t(
        if poll["votedByMe"].as_bool().unwrap_or(false) {
            "poll_revote"
        } else {
            "poll_vote"
        },
    ));
    card.status.set_text("");
    refresh_vote(card);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::FlexAction;

    fn action(uri: Option<&str>) -> FlexAction {
        FlexAction {
            label: "x".into(),
            kind: "uri".into(),
            data: None,
            uri: uri.map(str::to_string),
        }
    }

    fn flex(uris: &[Option<&str>]) -> FlexInfo {
        FlexInfo {
            alt_text: String::new(),
            texts: Vec::new(),
            actions: uris.iter().map(|u| action(*u)).collect(),
        }
    }

    #[test]
    fn extracts_the_poll_id_from_the_view_link() {
        assert_eq!(
            poll_id("https://liff.line.me/1477715170-Pl2JnXpR/view/7702851912688516885?chatId=C1")
                .as_deref(),
            Some("7702851912688516885")
        );
        assert_eq!(
            poll_id("https://liff.line.me/1477715170-Pl2JnXpR?chatId=C1"),
            None
        );
        assert_eq!(poll_id("https://example.com/view/abc"), None);
    }

    #[test]
    fn a_missing_session_is_retried_rather_than_shown() {
        assert!(is_not_logged_in("not_logged_in"));
        assert!(is_not_logged_in("Error: not_logged_in"));
        assert!(!is_not_logged_in("Poll is closed"));
    }

    #[test]
    fn only_poll_links_with_an_id_make_a_native_card() {
        let poll = "https://liff.line.me/1477715170-Pl2JnXpR/view/42?chatId=C1";
        let list = "https://liff.line.me/1477715170-Pl2JnXpR?chatId=C1";
        let other = "https://liff.line.me/1111111111-Other/view/42";
        assert_eq!(poll_uri(&flex(&[Some(list), Some(poll)])), Some(poll));
        assert_eq!(poll_uri(&flex(&[Some(list)])), None);
        assert_eq!(poll_uri(&flex(&[Some(other), None])), None);
    }
}
