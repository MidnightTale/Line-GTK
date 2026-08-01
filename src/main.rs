mod config;
mod desktop_notify;
mod discord_rpc;
mod i18n;
mod protocol;
mod sidecar;
mod sticker_anim;
mod tray;
mod ui;

use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Prefer GPU video decoders (NVDEC / Vulkan / VA-API) for GtkVideo / GStreamer.
    prefer_gstreamer_hw_decoders();

    let repo_root = discover_repo_root()?;
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("line-gtk");
    std::fs::create_dir_all(&data_dir)?;

    tracing::info!(?repo_root, ?data_dir, "starting LINE GTK");
    ui::run(repo_root, data_dir)
}

fn prefer_gstreamer_hw_decoders() {
    const RANK: &str = "\
nvh264dec:MAX,nvh265dec:MAX,nvav1dec:MAX,nvvp9dec:MAX,nvvp8dec:MAX,\
vulkanh264dec:MAX,vulkanh265dec:MAX,vulkanav1dec:MAX,vulkanvp9dec:MAX,\
vah264dec:MAX,vah265dec:MAX,vavp9dec:MAX,vaav1dec:MAX";
    if env::var_os("GST_PLUGIN_FEATURE_RANK").is_none() {
        // SAFETY: process startup, before GTK/GStreamer init.
        unsafe {
            env::set_var("GST_PLUGIN_FEATURE_RANK", RANK);
        }
    }
}

fn discover_repo_root() -> Result<PathBuf> {
    if let Ok(p) = env::var("LINE_GTK_ROOT") {
        let root = PathBuf::from(p);
        if root.join("protocol/src/main.ts").exists() {
            return Ok(root);
        }
        anyhow::bail!(
            "LINE_GTK_ROOT={} missing protocol/src/main.ts",
            root.display()
        );
    }

    let exe = env::current_exe().context("current_exe")?;
    let mut candidates = vec![
        // System package layout (AUR / distro install).
        PathBuf::from("/usr/share/line-gtk"),
        PathBuf::from("/usr/local/share/line-gtk"),
        env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        exe.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
    ];
    if let Some(p) = exe.parent().and_then(|p| p.parent()) {
        // target/release -> repo root (dev builds)
        candidates.push(p.parent().unwrap_or(p).to_path_buf());
        candidates.push(p.to_path_buf());
    }
    for c in candidates {
        if c.join("protocol/src/main.ts").exists() {
            return Ok(c);
        }
    }
    anyhow::bail!(
        "could not find LINE GTK resources (protocol/src/main.ts); set LINE_GTK_ROOT"
    )
}
