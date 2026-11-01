use super::*;
use std::time::{Duration, Instant};

fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(1200);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires a desktop display; uses a fixture sidecar and no LINE account"]
fn jump_keeps_entry_position_after_read_ack_and_loads_older_messages() {
    use std::os::unix::fs::PermissionsExt;
    libadwaita::init().unwrap();
    let root = std::env::temp_dir().join(format!("linegtk-read-ui-{}", std::process::id()));
    let protocol = root.join("protocol");
    let data = root.join("data");
    std::fs::create_dir_all(&protocol).unwrap();
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    std::os::unix::fs::symlink(assets, root.join("assets")).unwrap();
    let mut cfg = AppConfig {
        language: "en".into(),
        animations: false,
        tray_enabled: false,
        close_to_tray: false,
        ..AppConfig::default()
    };
    cfg.set_last_read_message(&data, "u-fixture", "c-fixture", "40")
        .unwrap();
    let binary = protocol.join("line-gtk-protocol");
    std::fs::write(&binary, r#"#!/usr/bin/env python3
import json, sys
def msg(i):
    return {'id':str(i),'from':'u-friend','to':'c-fixture','text':f'Message {i}',
            'createdTime':1700000000000+i*1000,'mine':False,'contentType':'TEXT'}
for line in sys.stdin:
    r=json.loads(line); m=r['method']; p=r.get('params') or {}
    if m=='fixture_exit': break
    if m=='fetch_messages': result={'messages':[msg(i) for i in range(61,101)]}
    elif m=='fetch_older_messages':
        end=int(p['beforeId'])
        result={'chatMid':'c-fixture','messages':[msg(i) for i in range(max(1,end-50),end)],'hasMore':end>51}
    elif m=='mark_read': result={'marked':True,**p}
    else: result={}
    print(json.dumps({'id':r['id'],'ok':True,'result':result}),flush=True)
"#).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let app = Application::builder()
        .application_id("dev.linegtk.ReadPositionFixture")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let state = crate::ui::build_ui(&app, root.clone(), data.clone(), false).unwrap();
    *state.self_mid.borrow_mut() = Some("u-fixture".into());
    *state.session_ready.borrow_mut() = true;
    state.stack.set_visible_child_name("shell");
    let chat: ChatInfo = serde_json::from_value(serde_json::json!({
        "mid":"c-fixture", "name":"Read position fixture", "kind":"group", "unread":60,
    }))
    .unwrap();
    state.chats.borrow_mut().push(chat.clone());
    open_chat(&state, &chat);
    settle();
    assert!(state.last_read_btn.is_visible());
    assert!(state.last_read_btn.is_sensitive());
    assert_eq!(
        state.last_read_position.borrow().target.as_deref(),
        Some("40")
    );
    assert_eq!(
        AppConfig::load(&data).last_read_message("u-fixture", "c-fixture"),
        Some("100")
    );

    // This is the same signal a pointer click or keyboard activation emits.
    state.last_read_btn.emit_clicked();
    settle();
    assert!(state.seen_msg_ids.borrow().contains("40"));
    assert!(!*state.stick_bottom.borrow());
    assert!(
        state
            .message_rows
            .borrow()
            .get("40")
            .is_some_and(|row| row.has_css_class("line-reply-target"))
    );
    assert_eq!(
        state.last_read_position.borrow().target.as_deref(),
        Some("40")
    );

    let paintable = gtk::WidgetPaintable::new(Some(&state.window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        state.window.width() as f64,
        state.window.height() as f64,
    );
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize(None::<&gdk::Surface>).unwrap();
    let texture = renderer.render_texture(snapshot.to_node().unwrap(), None);
    texture.save_to_png(root.join("last-read.png")).unwrap();
    renderer.unrealize();

    *state.self_mid.borrow_mut() = Some("u-other".into());
    refresh(&state);
    assert!(!state.last_read_btn.is_visible());
    record(&state, "u-fixture", "c-fixture", "200");
    assert_eq!(
        AppConfig::load(&data).last_read_message("u-fixture", "c-fixture"),
        Some("100")
    );
    state.window.destroy();
    let _ = state.sidecar.request("fixture_exit", None);
    println!(
        "Read position screenshot: {}",
        root.join("last-read.png").display()
    );
}
