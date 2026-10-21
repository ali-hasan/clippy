mod app;
mod apps;
mod clipboard;
mod history;
mod images;
mod theme;
mod tray;

use std::error::Error;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::MenuEvent;
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

use app::{ClippyApp, Shared};
use clipboard::CopiedContent;
use history::History;

/// Shortcut that opens and closes the window from any app. While Clippy runs, other apps no
/// longer receive ⌘⌥C; to choose another, change it here and in the label.
fn hotkey() -> HotKey {
    HotKey::new(Some(Modifiers::META | Modifiers::ALT), Code::KeyC)
}
const HOTKEY_LABEL: &str = "⌘⌥C";

fn main() -> Result<(), Box<dyn Error>> {
    let database = database_path()?;
    let history = History::open(&database)?;
    // Only the current user may read the copied text.
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600))?;
    let shared = Arc::new(Shared::new(history));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Clippy")
            .with_inner_size([560.0, 520.0])
            .with_min_inner_size([380.0, 300.0])
            // No visible title bar or window buttons: the window is just the search box and list.
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_titlebar_buttons_shown(false),
        centered: true,
        // Lives in the menu bar only: no Dock icon and no entry in the ⌘Tab switcher.
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_activation_policy(ActivationPolicy::Accessory);
        })),
        ..Default::default()
    };

    eframe::run_native(
        "Clippy",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            theme::install(&ctx);
            // The menu bar icon and the hotkey need the running event loop, which starts here.
            let menu_bar = tray::build()?;
            let hotkeys = GlobalHotKeyManager::new()?;
            hotkeys.register(hotkey())?;

            let (watcher_shared, watcher_ctx) = (shared.clone(), ctx.clone());
            clipboard::watch(move |copied| {
                let (now, app) = (history::now_millis(), copied.source_app.as_deref());
                let store = watcher_shared.history.lock().unwrap();
                let saved = match &copied.content {
                    CopiedContent::Text(text) => store.add(text, now, app),
                    CopiedContent::Image(image) => store.add_image(image, now, app),
                };
                drop(store);
                if let Err(err) = saved {
                    eprintln!("clippy: could not save copied text: {err}");
                }
                watcher_shared.history_changed.store(true, Ordering::Relaxed);
                watcher_ctx.request_repaint();
            });

            let (hotkey_shared, hotkey_ctx) = (shared.clone(), ctx.clone());
            GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
                if event.state == HotKeyState::Pressed {
                    app::toggle_window(&hotkey_shared, &hotkey_ctx);
                }
            }));

            let (menu_shared, menu_ctx) = (shared.clone(), ctx.clone());
            let (open_id, clear_id) = (menu_bar.open_id.clone(), menu_bar.clear_id.clone());
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if event.id == open_id {
                    app::open_window(&menu_shared, &menu_ctx);
                } else if event.id == clear_id {
                    if let Err(err) = menu_shared.history.lock().unwrap().clear() {
                        eprintln!("clippy: could not clear history: {err}");
                    }
                    menu_shared.history_changed.store(true, Ordering::Relaxed);
                    menu_ctx.request_repaint();
                }
            }));

            Ok(Box::new(ClippyApp::new(shared, Box::new((menu_bar.icon, hotkeys)))))
        }),
    )?;
    Ok(())
}

/// History is kept in ~/Library/Application Support/clippy/history.db. Setting CLIPPY_DB uses
/// another file instead, for example to try the app without touching the real history.
fn database_path() -> std::io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("CLIPPY_DB") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or_else(|| std::io::Error::other("HOME is not set"))?;
    let dir = PathBuf::from(home).join("Library/Application Support/clippy");
    fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    Ok(dir.join("history.db"))
}
