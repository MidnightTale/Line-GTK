use crate::i18n;
use crate::protocol::ChatInfo;
use crate::sidecar::Sidecar;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct FriendsUi {
    pub window: gtk::Window,
    pub list: gtk::ListBox,
    pub stack: gtk::Stack,
    pub empty: gtk::Label,
    pub search: gtk::SearchEntry,
    pub avatars: Rc<RefCell<HashMap<String, gtk::Picture>>>,
    pub friends: Rc<RefCell<Vec<ChatInfo>>>,
    /// Parallel to list rows: None = section header, Some(mid) = friend.
    pub row_mids: Rc<RefCell<Vec<Option<String>>>>,
}

pub struct FriendsDeps {
    pub sidecar: Rc<Sidecar>,
    pub toast: Rc<dyn Fn(&str)>,
    pub on_open: Rc<dyn Fn(ChatInfo)>,
    pub request_list: Rc<dyn Fn()>,
}

const THAI_CONSONANTS: &str = "กขฃคฅฆงจฉชซฌญฎฏฐฑฒณดตถทธนบปผฝพฟภมยรลวศษสหฬอฮ";

pub fn open_friends(parent: &impl IsA<gtk::Window>, deps: FriendsDeps) -> FriendsUi {
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .default_width(420)
        .default_height(560)
        .title(i18n::t("friends"))
        .build();

    let header = libadwaita::HeaderBar::builder()
        .title_widget(
            &gtk::Label::builder()
                .label(i18n::t("friends"))
                .css_classes(["heading"])
                .build(),
        )
        .build();

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();
    root.append(&header);

    let search = gtk::SearchEntry::builder()
        .placeholder_text(i18n::t("friends_search"))
        .hexpand(true)
        .margin_start(16)
        .margin_end(16)
        .margin_top(12)
        .margin_bottom(8)
        .css_classes(["line-search"])
        .build();

    let stack = gtk::Stack::builder().vexpand(true).hexpand(true).build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["line-friend-list"])
        .build();
    scroll.set_child(Some(&list));

    let loading = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    let spin = gtk::Spinner::builder().spinning(true).build();
    spin.set_size_request(28, 28);
    loading.append(&spin);
    loading.append(
        &gtk::Label::builder()
            .label(i18n::t("loading_friends"))
            .css_classes(["dim-label"])
            .build(),
    );

    let empty = gtk::Label::builder()
        .label(i18n::t("no_friends"))
        .css_classes(["dim-label", "title-4"])
        .justify(gtk::Justification::Center)
        .build();

    stack.add_named(&loading, Some("loading"));
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&empty, Some("empty"));
    stack.set_visible_child_name("loading");

    let add_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(16)
        .margin_end(16)
        .margin_top(12)
        .margin_bottom(16)
        .css_classes(["line-friend-add"])
        .build();
    add_box.append(
        &gtk::Label::builder()
            .label(i18n::t("add_friend"))
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    let add_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let id_entry = gtk::Entry::builder()
        .placeholder_text(i18n::t("line_id"))
        .hexpand(true)
        .build();
    let add_btn = gtk::Button::builder()
        .label(i18n::t("add"))
        .css_classes(["suggested-action"])
        .build();
    add_row.append(&id_entry);
    add_row.append(&add_btn);
    add_box.append(&add_row);

    {
        let sidecar = deps.sidecar.clone();
        let toast = deps.toast.clone();
        let id_entry2 = id_entry.clone();
        let request_list = deps.request_list.clone();
        add_btn.connect_clicked(move |_| {
            let userid = id_entry2.text().trim().to_string();
            if userid.is_empty() {
                toast(&i18n::t("enter_line_id"));
                return;
            }
            match sidecar.add_friend(&userid) {
                Ok(_) => {
                    toast(&i18n::t("friend_ok"));
                    id_entry2.set_text("");
                    request_list();
                }
                Err(e) => toast(&i18n::tf("friend_failed", &[("error", &e.to_string())])),
            }
        });
    }

    root.append(&search);
    root.append(&stack);
    root.append(&add_box);
    window.set_child(Some(&root));

    // Plain GtkWindow does not close on Escape; SearchEntry/Entry also eat Esc.
    // Capture-phase handler makes Esc always dismiss this modal.
    {
        let window2 = window.clone();
        let esc = gtk::EventControllerKey::new();
        esc.set_propagation_phase(gtk::PropagationPhase::Capture);
        esc.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                window2.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(esc);
    }

    let friends = Rc::new(RefCell::new(Vec::<ChatInfo>::new()));
    let avatars = Rc::new(RefCell::new(HashMap::<String, gtk::Picture>::new()));
    let row_mids = Rc::new(RefCell::new(Vec::<Option<String>>::new()));

    {
        let list2 = list.clone();
        let friends2 = friends.clone();
        let row_mids2 = row_mids.clone();
        search.connect_search_changed(move |entry| {
            filter_friend_rows_inner(&list2, &friends2, &row_mids2, &entry.text());
        });
    }

    {
        let on_open = deps.on_open.clone();
        let friends2 = friends.clone();
        let row_mids2 = row_mids.clone();
        let window2 = window.clone();
        list.connect_row_activated(move |_, row| {
            let idx = row.index();
            if idx < 0 {
                return;
            }
            let mid = row_mids2.borrow().get(idx as usize).and_then(|m| m.clone());
            let Some(mid) = mid else { return };
            if let Some(friend) = friends2.borrow().iter().find(|f| f.mid == mid).cloned() {
                on_open(friend);
                window2.close();
            }
        });
    }

    window.present();

    FriendsUi {
        window,
        list,
        stack,
        empty,
        search,
        avatars,
        friends,
        row_mids,
    }
}

pub fn apply_friends(ui: &FriendsUi, mut friends: Vec<ChatInfo>) {
    friends.sort_by(|a, b| {
        friend_sort_key(&a.name)
            .cmp(&friend_sort_key(&b.name))
            .then_with(|| {
                a.name
                    .to_lowercase()
                    .cmp(&b.name.to_lowercase())
                    .then_with(|| a.mid.cmp(&b.mid))
            })
    });
    *ui.friends.borrow_mut() = friends.clone();
    ui.avatars.borrow_mut().clear();
    ui.row_mids.borrow_mut().clear();
    while let Some(row) = ui.list.row_at_index(0) {
        ui.list.remove(&row);
    }

    if friends.is_empty() {
        ui.stack.set_visible_child_name("empty");
        return;
    }
    ui.stack.set_visible_child_name("list");

    let mut last_bucket: Option<String> = None;
    for friend in &friends {
        let bucket = letter_bucket(&friend.name);
        if last_bucket.as_deref() != Some(bucket.as_str()) {
            append_section_header(ui, &bucket);
            last_bucket = Some(bucket);
        }
        append_friend_row(ui, friend);
    }

    filter_friend_rows(ui);
}

fn append_section_header(ui: &FriendsUi, letter: &str) {
    let row = gtk::ListBoxRow::builder()
        .selectable(false)
        .activatable(false)
        .css_classes(["line-friend-section-row"])
        .build();
    let label = gtk::Label::builder()
        .label(letter)
        .xalign(0.0)
        .css_classes(["line-friend-section"])
        .build();
    row.set_child(Some(&label));
    ui.list.append(&row);
    ui.row_mids.borrow_mut().push(None);
}

fn append_friend_row(ui: &FriendsUi, friend: &ChatInfo) {
    let row = gtk::ListBoxRow::builder()
        .css_classes(["line-friend-row"])
        .build();
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(8)
        .margin_bottom(8)
        .build();

    let avatar_frame = gtk::Box::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["line-avatar-frame", "line-avatar-sm"])
        .build();
    let avatar = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .width_request(40)
        .height_request(40)
        .css_classes(["line-avatar", "line-avatar-sm"])
        .build();
    if let Some(path) = friend.avatar_path.as_deref()
        && std::path::Path::new(path).exists()
        && let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_file_at_scale(path, 80, 80, true)
    {
        let tex = gtk::gdk::Texture::for_pixbuf(&pixbuf);
        avatar.set_paintable(Some(&tex));
    }
    avatar_frame.append(&avatar);
    ui.avatars.borrow_mut().insert(friend.mid.clone(), avatar);

    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    text.append(
        &gtk::Label::builder()
            .label(&friend.name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build(),
    );
    let sub = if friend.kind.eq_ignore_ascii_case("bot") {
        i18n::t("friend_bot")
    } else {
        i18n::t("friend_tap_chat")
    };
    text.append(
        &gtk::Label::builder()
            .label(&sub)
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build(),
    );

    box_.append(&avatar_frame);
    box_.append(&text);
    row.set_child(Some(&box_));
    ui.list.append(&row);
    ui.row_mids.borrow_mut().push(Some(friend.mid.clone()));
}

fn filter_friend_rows(ui: &FriendsUi) {
    filter_friend_rows_inner(&ui.list, &ui.friends, &ui.row_mids, &ui.search.text());
}

fn filter_friend_rows_inner(
    list: &gtk::ListBox,
    friends: &Rc<RefCell<Vec<ChatInfo>>>,
    row_mids: &Rc<RefCell<Vec<Option<String>>>>,
    query: &str,
) {
    let q = query.to_lowercase();
    let mids = row_mids.borrow();
    let friends = friends.borrow();

    // First pass: visibility of friend rows.
    let mut visible_friend = vec![false; mids.len()];
    for (i, mid) in mids.iter().enumerate() {
        let Some(mid) = mid else { continue };
        let show = q.is_empty()
            || friends.iter().any(|f| {
                f.mid == *mid
                    && (f.name.to_lowercase().contains(&q) || f.mid.to_lowercase().contains(&q))
            });
        visible_friend[i] = show;
        if let Some(row) = list.row_at_index(i as i32) {
            row.set_visible(show);
        }
    }

    // Second pass: show a section header if any following friends (until next header) match.
    let mut i = 0;
    while i < mids.len() {
        if mids[i].is_some() {
            i += 1;
            continue;
        }
        let mut any = false;
        let mut j = i + 1;
        while j < mids.len() && mids[j].is_some() {
            if visible_friend[j] {
                any = true;
            }
            j += 1;
        }
        if let Some(row) = list.row_at_index(i as i32) {
            row.set_visible(q.is_empty() || any);
        }
        i = j;
    }
}

/// Sort: Thai ก–ฮ, then A–Z, then digits/#, then other.
fn friend_sort_key(name: &str) -> (u8, u32, String) {
    let bucket = letter_bucket(name);
    let (group, ord) = bucket_ord(&bucket);
    (group, ord, name.to_lowercase())
}

fn bucket_ord(bucket: &str) -> (u8, u32) {
    if bucket.len() == 1 {
        let ch = bucket.chars().next().unwrap();
        if let Some(pos) = THAI_CONSONANTS.chars().position(|c| c == ch) {
            return (0, pos as u32);
        }
        if ch.is_ascii_uppercase() {
            return (1, (ch as u32) - ('A' as u32));
        }
    }
    if bucket == "#" {
        return (2, 0);
    }
    (3, 0)
}

fn letter_bucket(name: &str) -> String {
    let Some(ch) = first_index_char(name) else {
        return "#".into();
    };
    if THAI_CONSONANTS.contains(ch) {
        return ch.to_string();
    }
    // Thai leading vowels → following consonant when possible.
    if "เแโใไ".contains(ch) {
        if let Some(cons) = following_thai_consonant(name) {
            return cons.to_string();
        }
        return ch.to_string();
    }
    // Other Thai letters (vowels / tones used as first glyph) → #
    if is_thai(ch) {
        return "#".into();
    }
    if ch.is_ascii_alphabetic() {
        return ch.to_ascii_uppercase().to_string();
    }
    if ch.is_ascii_digit() {
        return "#".into();
    }
    "#".into()
}

fn first_index_char(name: &str) -> Option<char> {
    for ch in name.chars() {
        if ch.is_whitespace() {
            continue;
        }
        // Skip common emoji / symbol ranges used as name prefixes.
        if is_decorative(ch) {
            continue;
        }
        return Some(ch);
    }
    name.chars().find(|c| !c.is_whitespace())
}

fn following_thai_consonant(name: &str) -> Option<char> {
    let mut seen_lead = false;
    for ch in name.chars() {
        if ch.is_whitespace() || is_decorative(ch) {
            continue;
        }
        if !seen_lead {
            if "เแโใไ".contains(ch) {
                seen_lead = true;
                continue;
            }
            return None;
        }
        if THAI_CONSONANTS.contains(ch) {
            return Some(ch);
        }
        if is_thai(ch) {
            continue;
        }
        break;
    }
    None
}

fn is_thai(ch: char) -> bool {
    ('\u{0E00}'..='\u{0E7F}').contains(&ch)
}

fn is_decorative(ch: char) -> bool {
    matches!(
        ch,
        '\u{2600}'..='\u{27BF}'
            | '\u{1F300}'..='\u{1FAFF}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{200D}'
            | '\u{20E3}'
            | '•' | '·' | '|' | '~' | '-' | '_' | '.'
    ) || ch.is_ascii_punctuation()
}

pub fn set_friends_loading(ui: &FriendsUi) {
    ui.stack.set_visible_child_name("loading");
}
