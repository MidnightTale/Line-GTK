use super::*;
use serde_json::{Value, json};

pub(super) struct FoodPollUi {
    window: gtk::Window,
    account: String,
    mid: String,
    flagged: gtk::CheckButton,
    address: gtk::Entry,
    title: gtk::Entry,
    time: gtk::Entry,
    days: Vec<gtk::CheckButton>,
    count: gtk::SpinButton,
    key: gtk::PasswordEntry,
    status: gtk::Label,
    output: gtk::TextView,
    save: gtk::Button,
    preview: gtk::Button,
    busy: std::cell::Cell<bool>,
}

pub(super) fn wire(state: &AppState) {
    let action = gio::SimpleAction::new("food-poll", Some(glib::VariantTy::STRING));
    let s = state.clone();
    action.connect_activate(move |_, value| {
        if let Some(mid) = value.and_then(|v| v.str()) {
            open(&s, mid);
        }
    });
    state.window.add_action(&action);
}

pub(super) fn close(state: &AppState) {
    let ui = state.food_poll_ui.borrow_mut().take();
    if let Some(ui) = ui {
        ui.window.close();
    }
}

fn field(root: &gtk::Box, title: &str, widget: &impl IsA<gtk::Widget>) {
    let label = gtk::Label::builder().label(title).xalign(0.0).build();
    label.set_mnemonic_widget(Some(widget));
    root.append(&label);
    root.append(widget);
}

fn open(state: &AppState, mid: &str) {
    if !*state.session_ready.borrow() {
        return;
    }
    let Some(account) = state.self_mid.borrow().clone() else {
        return;
    };
    let Some(chat) = state
        .chats
        .borrow()
        .iter()
        .find(|c| c.mid == mid && c.kind == "group")
        .cloned()
    else {
        return;
    };
    close(state);
    let window = gtk::Window::builder()
        .title(format!("Poll đồ ăn · {}", chat.name))
        .transient_for(&state.window)
        .modal(true)
        .default_width(620)
        .default_height(720)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    let note = gtk::Label::builder().label("Chuẩn bị poll từ quán ăn trên Google Maps trong bán kính 5 km. Khoảng cách đường chim bay; không xác nhận giao hàng hay giờ mở cửa.").wrap(true).xalign(0.0).build();
    root.append(&note);
    let flagged = gtk::CheckButton::with_label("Đánh dấu nhóm để tạo poll đồ ăn");
    root.append(&flagged);
    let title = gtk::Entry::new();
    field(&root, "Tiêu đề poll", &title);
    let address = gtk::Entry::new();
    field(&root, "Địa chỉ trung tâm (bán kính cố định 5 km)", &address);
    let time = gtk::Entry::new();
    field(&root, "Giờ dự kiến (HH:mm · Asia/Ho_Chi_Minh)", &time);
    let day_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let days: Vec<_> = ["CN", "T2", "T3", "T4", "T5", "T6", "T7"]
        .iter()
        .map(|name| {
            let button = gtk::CheckButton::with_label(name);
            day_box.append(&button);
            button
        })
        .collect();
    field(&root, "Ngày trong tuần", &day_box);
    let count = gtk::SpinButton::with_range(2.0, 10.0, 1.0);
    field(&root, "Số quán tối đa trong poll", &count);
    let key = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    field(
        &root,
        "Google Maps API key (để trống để giữ key đã lưu)",
        &key,
    );
    let help = gtk::Label::builder().label("Key dùng chung cho các nhóm của tài khoản này. Cần bật Geocoding API và Places API (New) trong Google Cloud; yêu cầu có thể phát sinh phí.").wrap(true).xalign(0.0).build();
    root.append(&help);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let save = gtk::Button::with_label("Lưu cấu hình nhóm");
    let preview = gtk::Button::with_label("Tìm quán và xem bản nháp");
    actions.append(&save);
    actions.append(&preview);
    root.append(&actions);
    let status = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .build();
    root.append(&status);
    let transport = gtk::Label::builder().label("Lịch tự gửi chưa bật: đang chờ chọn bình chọn qua tin nhắn hay poll gốc LINE. Lưu cấu hình và xem nháp không gửi vào nhóm.").wrap(true).xalign(0.0).build();
    root.append(&transport);
    let output = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .height_request(180)
        .build();
    root.append(&output);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&root)
        .build();
    window.set_child(Some(&scroll));
    let ui = Rc::new(FoodPollUi {
        window,
        account,
        mid: mid.into(),
        flagged,
        address,
        title,
        time,
        days,
        count,
        key,
        status,
        output,
        save,
        preview,
        busy: std::cell::Cell::new(false),
    });
    *state.food_poll_ui.borrow_mut() = Some(ui.clone());
    let s = state.clone();
    let weak = Rc::downgrade(&ui);
    ui.window.connect_close_request(move |_| {
        if let Some(ui) = weak.upgrade() {
            let is_current = s
                .food_poll_ui
                .borrow()
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &ui));
            if is_current {
                s.food_poll_ui.borrow_mut().take();
            }
        }
        glib::Propagation::Proceed
    });
    for (button, method) in [
        (&ui.save, "food_poll_save"),
        (&ui.preview, "food_poll_preview"),
    ] {
        let s = state.clone();
        let weak = Rc::downgrade(&ui);
        button.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                send(&s, &ui, method);
            }
        });
    }
    ui.window.present();
    send(state, &ui, "food_poll_read");
}

fn config(ui: &FoodPollUi) -> Value {
    json!({"flagged":ui.flagged.is_active(), "address":ui.address.text().as_str(), "title":ui.title.text().as_str(), "time":ui.time.text().as_str(), "weekdays":ui.days.iter().enumerate().filter_map(|(i,b)|b.is_active().then_some(i)).collect::<Vec<_>>(), "optionCount":ui.count.value_as_int()})
}

fn busy(ui: &FoodPollUi, value: bool) {
    ui.busy.set(value);
    ui.save.set_sensitive(!value);
    ui.preview.set_sensitive(!value);
    for field in [&ui.address, &ui.title, &ui.time] {
        field.set_sensitive(!value);
    }
    ui.key.set_sensitive(!value);
    ui.flagged.set_sensitive(!value);
    ui.count.set_sensitive(!value);
    for day in &ui.days {
        day.set_sensitive(!value);
    }
}

fn send(state: &AppState, ui: &Rc<FoodPollUi>, method: &'static str) {
    if ui.busy.get() {
        return;
    }
    if state.self_mid.borrow().as_deref() != Some(&ui.account) || !*state.session_ready.borrow() {
        ui.status
            .set_text("Phiên đăng nhập đã thay đổi. Hãy mở lại cấu hình.");
        return;
    }
    let mut params = json!({"chatMid":ui.mid, "config":config(ui)});
    if !ui.key.text().is_empty() {
        params["apiKey"] = json!(ui.key.text().as_str());
    }
    busy(ui, true);
    ui.status.set_text("Đang xử lý…");
    if method == "food_poll_preview" {
        ui.output.buffer().set_text("");
    }
    match state.sidecar.request(method, Some(params)) {
        Ok(id) => {
            state.pending.borrow_mut().insert(
                id,
                Pending::FoodPoll {
                    ui: Rc::downgrade(ui),
                    method,
                },
            );
        }
        Err(_) => {
            busy(ui, false);
            ui.status.set_text("Không kết nối được dịch vụ LINE.");
        }
    }
}

pub(super) fn response(
    state: &AppState,
    ui: &FoodPollUi,
    method: &str,
    ok: bool,
    result: &Value,
    error: Option<&str>,
) {
    if state.self_mid.borrow().as_deref() != Some(&ui.account) {
        return;
    }
    busy(ui, false);
    if !ok {
        ui.status
            .set_text(error.unwrap_or("Không xử lý được yêu cầu."));
        return;
    }
    if method == "food_poll_preview" {
        ui.output
            .buffer()
            .set_text(result["text"].as_str().unwrap_or_default());
        ui.status.set_text(&format!(
            "Bản nháp · trung tâm: {} · nguồn Google Maps",
            result["resolvedAddress"].as_str().unwrap_or_default()
        ));
    } else {
        let c = &result["config"];
        ui.flagged
            .set_active(c["flagged"].as_bool().unwrap_or(false));
        ui.title.set_text(c["title"].as_str().unwrap_or_default());
        ui.address
            .set_text(c["address"].as_str().unwrap_or_default());
        ui.time.set_text(c["time"].as_str().unwrap_or("10:00"));
        ui.count.set_value(c["optionCount"].as_f64().unwrap_or(6.0));
        for (day, button) in ui.days.iter().enumerate() {
            button.set_active(
                c["weekdays"]
                    .as_array()
                    .is_some_and(|days| days.contains(&json!(day))),
            );
        }
        ui.key.set_text("");
        ui.status.set_text(if method == "food_poll_save" {
            "Đã lưu cấu hình nhóm. Lịch tự gửi chưa bật."
        } else if result["hasApiKey"].as_bool() == Some(true) {
            "Đã có API key lưu cho tài khoản này."
        } else {
            "Nhập Google Maps API key để tìm quán thật."
        });
    }
}
