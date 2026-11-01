use super::*;
use std::time::{Duration, Instant};

fn settle() {
    let context = glib::MainContext::default();
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end {
        while context.pending() && Instant::now() < end {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn descendants(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    fn visit(widget: &gtk::Widget, all: &mut Vec<gtk::Widget>) {
        all.push(widget.clone());
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            visit(&current, all);
        }
    }
    let mut all = Vec::new();
    visit(widget.as_ref(), &mut all);
    all
}

fn button(widget: &impl IsA<gtk::Widget>, text: &str) -> gtk::Button {
    descendants(widget)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| b.label().as_deref() == Some(text))
        .unwrap_or_else(|| panic!("missing button: {text}"))
}

fn context_menu(widget: &impl IsA<gtk::Widget>) -> gtk::PopoverMenu {
    let controllers = widget.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|i| {
            controllers
                .item(i)?
                .downcast::<gtk::EventControllerKey>()
                .ok()
        })
        .next()
        .unwrap();
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::Menu, &0u32, &gdk::ModifierType::empty()],
    ));
    settle();
    let menu = descendants(widget)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::PopoverMenu>().ok())
        .unwrap();
    assert!(menu.is_mapped());
    menu
}

fn screenshot(widget: &impl IsA<gtk::Widget>, path: &std::path::Path) {
    settle();
    let widget = widget.as_ref();
    assert!(widget.width() > 0 && widget.height() > 0);
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    // A theme change can invalidate the paintable until the next rendered frame.
    let end = Instant::now() + Duration::from_secs(2);
    let node = loop {
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, widget.width() as f64, widget.height() as f64);
        if let Some(node) = snapshot.to_node() {
            break node;
        }
        assert!(Instant::now() < end, "widget did not render");
        widget.queue_draw();
        settle();
    };
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize(None::<&gdk::Surface>).unwrap();
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).unwrap();
    renderer.unrealize();
}

/// Run manually in a private D-Bus session with isolated XDG directories.
#[test]
#[ignore = "requires a desktop display; uses a fixture sidecar and no LINE account"]
fn fixture_picker_and_import_dialogs() {
    libadwaita::init().unwrap();
    let root = std::env::temp_dir().join(format!("linegtk-sticker-ui-{}", std::process::id()));
    let protocol = root.join("protocol");
    let data = root.join("data");
    std::fs::create_dir_all(&protocol).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    std::os::unix::fs::symlink(assets, root.join("assets")).unwrap();
    let cfg = AppConfig {
        language: "en".into(),
        theme: "light".into(),
        tray_enabled: false,
        close_to_tray: false,
        ..AppConfig::default()
    };
    cfg.save(&data);
    let image = root.join("personal.png");
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 96, 96).unwrap();
    let cr = cairo::Context::new(&surface).unwrap();
    cr.set_source_rgb(0.03, 0.76, 0.38);
    cr.arc(48., 48., 40., 0., std::f64::consts::TAU);
    cr.fill().unwrap();
    cr.set_source_rgb(0.05, 0.12, 0.08);
    for x in [33., 63.] {
        cr.arc(x, 38., 4., 0., std::f64::consts::TAU);
        cr.fill().unwrap();
    }
    cr.set_line_width(4.);
    cr.arc(48., 48., 23., 0.2, 2.94);
    cr.stroke().unwrap();
    surface
        .write_to_png(&mut std::fs::File::create(&image).unwrap())
        .unwrap();
    let animation = root.join("animated.png");
    let gif = root.join("animated.gif");
    let first = image::open(&image).unwrap().into_rgba8();
    let mut second = first.clone();
    for pixel in second.pixels_mut() {
        if pixel[1] > pixel[0].saturating_mul(3) {
            pixel[0] = 50;
            pixel[1] = 90;
            pixel[2] = 240;
        }
    }
    let mut encoder = png::Encoder::new(std::fs::File::create(&animation).unwrap(), 96, 96);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_animated(2, 0).unwrap();
    let mut writer = encoder.write_header().unwrap();
    writer.set_frame_delay(1, 5).unwrap();
    writer.write_image_data(first.as_raw()).unwrap();
    writer.write_image_data(second.as_raw()).unwrap();
    writer.finish().unwrap();
    let mut encoder = image::codecs::gif::GifEncoder::new(std::fs::File::create(&gif).unwrap());
    encoder
        .set_repeat(image::codecs::gif::Repeat::Infinite)
        .unwrap();
    for frame in [first, second] {
        encoder
            .encode_frame(image::Frame::from_parts(
                frame,
                0,
                0,
                image::Delay::from_numer_denom_ms(200, 1),
            ))
            .unwrap();
    }
    drop(encoder);
    let script = format!(
        r#"#!/usr/bin/env python3
import json,sys
image={image:?}
animation={animation:?}
gif={gif:?}
log=image+".requests"
for line in sys.stdin:
    r=json.loads(line); m=r['method']; p=r.get('params') or {{}}
    with open(log,'a') as out: out.write(json.dumps(r)+'\n')
    if m == 'fixture_exit': break
    if m == 'inspect_sticker_pngs':
        result={{'accountMid':'u-fixture','files':[{{'index':i,'valid':True,'digest':'fixture','previewPath':image}} for i in range(len(p['paths']))]}}
    elif m == 'prepare_sticker_image':
        animated='animated' in p.get('optionKey','')
        result={{'accountMid':'u-fixture','path':gif if animated else image,'animated':animated,'originalBytes':1024,'optimizedBytes':512,'options':p.get('options') or {{'maxDimension':0,'fps':15,'colors':128}}}}
    elif m == 'set_sticker_optimization':
        result={{'accountMid':'u-fixture'}}
    elif m == 'import_sticker_pngs':
        result={{'accountMid':'u-fixture','added':len(p['paths']),'errors':[]}}
    elif m == 'preview_sticker_pack':
        result={{'accountMid':'u-fixture','packageId':'2','name':'A creator’s sticker pack','count':40,'previews':[image]*4}}
    elif m == 'sticker_owned_pack':
        result={{'id':'2','name':'Owned creator pack','stickers':[{{'stickerId':'10','packageId':'2','imagePath':image}}]}}
    else: result={{}}
    print(json.dumps({{'id':r['id'],'ok':True,'result':result}}),flush=True)
"#,
        image = image.to_string_lossy(),
        animation = animation.to_string_lossy(),
        gif = gif.to_string_lossy()
    );
    let binary = protocol.join("line-gtk-protocol");
    std::fs::write(&binary, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let app = Application::builder()
        .application_id("dev.linegtk.StickerFixture")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let state = crate::ui::build_ui(&app, root.clone(), data.clone(), false).unwrap();
    state.sticker_popover.set_autohide(false);
    *state.self_mid.borrow_mut() = Some("u-fixture".into());
    *state.current_chat.borrow_mut() = Some("c-fixture".into());
    *state.session_ready.borrow_mut() = true;
    state.stack.set_visible_child_name("shell");
    state.conversation.set_visible(true);
    settle();
    let local: Vec<Value> = (0..61).map(|i| json!({ "stickerId": format!("item-{i}"), "packageId": "personal", "imagePath": image.to_string_lossy(), "optionKey": format!("fixture/personal/{i}"), "personal": true })).collect();
    let packs = json!([
        {"id":"personal","name":"My PNGs","source":"local_png","imageOnly":true,"stickers":local},
        {"id":"store-2","name":"Downloaded creator pack","source":"line_store","imageOnly":true,"sourceUrl":"https://store.line.me/stickershop/product/2/en","importPack":"2","incomplete":true,"stickers":[{"stickerId":"10","packageId":"store-2","imagePath":animation.to_string_lossy(),"optionKey":"fixture/animated/10","animated":true,"optimization":{"maxDimension":128,"fps":10,"colors":32}}]},
        {"id":"2","name":"Owned creator pack","lazy":true,"stickers":[]}
    ]);
    fill(
        &state,
        &json!({"accountMid":"u-fixture","packs":packs,"ownedPackages":1}),
    );
    state.sticker_popover.popup();
    settle();
    let grid = descendants(&state.sticker_popover)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::FlowBox>().ok())
        .unwrap();
    let count = |g: &gtk::FlowBox| {
        let mut n = 0;
        let mut c = g.first_child();
        while let Some(w) = c {
            n += 1;
            c = w.next_sibling();
        }
        n
    };
    assert_eq!(count(&grid), 60);
    screenshot(&state.sticker_popover, &root.join("picker-light.png"));
    button(&state.sticker_popover, "Next").emit_clicked();
    settle();
    let grid = descendants(&state.sticker_popover)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::FlowBox>().ok())
        .unwrap();
    assert_eq!(count(&grid), 1);
    let tabs = descendants(&state.sticker_popover)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::StackSwitcher>().ok())
        .unwrap();
    let tabs = tabs.stack().unwrap();
    assert_eq!(tabs.pages().n_items(), 3);
    tabs.set_visible_child_name("store-2");
    settle();
    let tab = descendants(&state.sticker_popover)
        .into_iter()
        .find(|w| w.tooltip_text().as_deref() == Some("Downloaded creator pack"))
        .unwrap();
    let menu = context_menu(&tab);
    let labels: Vec<_> = descendants(&menu)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
        .map(|l| l.text().to_string())
        .collect();
    assert!(labels.iter().any(|l| l == "Retry missing images"));
    assert!(labels.iter().any(|l| l == "Rename"));
    screenshot(&menu, &root.join("pack-context-menu-light.png"));
    menu.popdown();
    settle();
    let picture = descendants(&tabs.visible_child().unwrap())
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Picture>().ok())
        .unwrap();
    let first = picture.paintable().unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while picture.paintable().as_ref() == Some(&first) && Instant::now() < end {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_ne!(
        picture.paintable().as_ref(),
        Some(&first),
        "grid preview must animate without hovering"
    );
    let send_gif = picture.parent().unwrap().downcast::<gtk::Button>().unwrap();
    let menu = context_menu(&send_gif);
    let labels: Vec<_> = descendants(&menu)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
        .map(|l| l.text().to_string())
        .collect();
    assert!(labels.iter().any(|l| l == "Optimize image"));
    assert!(labels.iter().any(|l| l == "Remove image"));
    screenshot(&menu, &root.join("image-context-menu-light.png"));
    menu.popdown();
    settle();
    screenshot(&state.sticker_popover, &root.join("creator-pack-light.png"));
    tabs.set_visible_child_name("2");
    settle();
    let visible_labels: Vec<String> = descendants(&state.sticker_popover)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Label>().ok())
        .map(|l| l.text().to_string())
        .collect();
    assert!(
        visible_labels
            .iter()
            .any(|text| text == "Sends as LINE stickers"),
        "native picker labels: {visible_labels:?}; selected={}",
        tabs.visible_child_name().unwrap()
    );
    tabs.set_visible_child_name("store-2");
    settle();
    let picture = descendants(&tabs.visible_child().unwrap())
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Picture>().ok())
        .unwrap();
    picture
        .parent()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    settle();
    assert!(
        state
            .sticker_dialogs
            .borrow()
            .iter()
            .all(|d| d.upgrade().is_none_or(|d| d.parent().is_none()))
    );
    state.sticker_popover.popdown();
    store_dialog(&state);
    settle();
    let d = state
        .sticker_dialogs
        .borrow()
        .last()
        .unwrap()
        .upgrade()
        .unwrap();
    let entry = descendants(&d)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Entry>().ok())
        .unwrap();
    entry.set_text("2");
    button(&d, "Preview pack").emit_clicked();
    settle();
    assert!(button(&d, "Download").is_sensitive());
    screenshot(&d, &root.join("store-preview-light.png"));
    close(&state);
    settle();
    png_review(
        &state,
        vec![image.to_string_lossy().to_string()],
        Rc::new(serde_json::from_value(packs).unwrap()),
        None,
    );
    settle();
    let d = state
        .sticker_dialogs
        .borrow()
        .last()
        .unwrap()
        .upgrade()
        .unwrap();
    assert!(button(&d, "Import selected").is_sensitive());
    screenshot(&d, &root.join("png-review-light.png"));
    libadwaita::StyleManager::default().set_color_scheme(libadwaita::ColorScheme::ForceDark);
    screenshot(&d, &root.join("png-review-dark.png"));
    let name = descendants(&d)
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Entry>().ok())
        .unwrap();
    name.set_text("My imported PNGs");
    button(&d, "Import selected").emit_clicked();
    settle();
    assert!(
        descendants(&d)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Label>().ok())
            .any(|l| l.text() == "Added 1 images.")
    );
    assert!(!button(&d, "Import selected").is_sensitive());
    let requests = std::fs::read_to_string(root.join("personal.png.requests")).unwrap();
    let imports: Vec<Value> = requests
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|r| r["method"] == "import_sticker_pngs")
        .collect();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0]["params"]["digests"], json!(["fixture"]));
    close(&state);
    settle();
    assert!(state.sticker_dialogs.borrow().is_empty());
    libadwaita::StyleManager::default().set_color_scheme(libadwaita::ColorScheme::ForceLight);
    let source = sticker_image_editor::Source {
        path: image.clone(),
        key: "fixture/personal/0".into(),
        personal: true,
        animated: false,
        optimized: false,
        optimization: json!({"maxDimension":0,"fps":15,"colors":128}),
    };
    optimize_dialog(&state, source);
    settle();
    let d = state
        .sticker_dialogs
        .borrow()
        .last()
        .unwrap()
        .upgrade()
        .unwrap();
    button(&d, "Preview optimized image").emit_clicked();
    settle();
    assert!(button(&d, "Save settings").is_sensitive());
    screenshot(&d, &root.join("optimize-static-light.png"));
    close(&state);
    settle();
    let source = sticker_image_editor::Source {
        path: animation,
        key: "fixture/animated/0".into(),
        personal: true,
        animated: true,
        optimized: false,
        optimization: json!({"maxDimension":0,"fps":15,"colors":128}),
    };
    optimize_dialog(&state, source);
    settle();
    let d = state
        .sticker_dialogs
        .borrow()
        .last()
        .unwrap()
        .upgrade()
        .unwrap();
    assert_eq!(
        descendants(&d)
            .iter()
            .filter(|w| w.is::<gtk::DropDown>())
            .count(),
        3
    );
    button(&d, "Small GIF").emit_clicked();
    settle();
    assert!(button(&d, "Send GIF").is_sensitive());
    screenshot(&d, &root.join("optimize-animation-light.png"));
    libadwaita::StyleManager::default().set_color_scheme(libadwaita::ColorScheme::ForceDark);
    screenshot(&d, &root.join("optimize-animation-dark.png"));
    button(&d, "Send GIF").emit_clicked();
    settle();
    let requests = std::fs::read_to_string(root.join("personal.png.requests")).unwrap();
    let previews: Vec<Value> = requests
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|r| r["method"] == "prepare_sticker_image")
        .collect();
    assert_eq!(
        previews.last().unwrap()["params"]["options"],
        json!({"maxDimension":128,"fps":10,"colors":32})
    );
    let sends: Vec<Value> = requests
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|r| r["method"] == "send_media")
        .collect();
    assert_eq!(sends.len(), 2);
    assert!(sends.iter().all(|r| r["params"]["oType"] == "gif"));
    assert!(
        !requests
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|r| r["method"] == "send_sticker")
    );
    close(&state);
    settle();
    state.window.destroy();
    let _ = state.sidecar.request("fixture_exit", None);
    println!("GTK fixture screenshots: {}", root.display());
}
