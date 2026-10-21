//! The menu bar icon and its menu.

use objc2::rc::autoreleasepool;
use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::images;

/// The SF Symbol shown in the menu bar.
const SYMBOL: &str = "list.clipboard";

pub struct MenuBar {
    /// The icon disappears from the menu bar when this is dropped.
    pub icon: TrayIcon,
    pub open_id: MenuId,
    pub clear_id: MenuId,
}

pub fn build() -> Result<MenuBar, Box<dyn std::error::Error + Send + Sync>> {
    let open = MenuItem::new("Open Clippy", true, None);
    let clear = MenuItem::new("Clear History", true, None);
    let menu = Menu::with_items(&[
        &open,
        &PredefinedMenuItem::separator(),
        &clear,
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::quit(Some("Quit Clippy")),
    ])?;
    let icon = TrayIconBuilder::new()
        // A template icon is recolored by macOS to suit light and dark menu bars.
        .with_icon_templated(menu_bar_icon(SYMBOL).ok_or("could not draw the menu bar icon")?)
        .with_tooltip("Clippy")
        .with_menu(Box::new(menu))
        .build()?;
    Ok(MenuBar {
        icon,
        open_id: open.id().clone(),
        clear_id: clear.id().clone(),
    })
}

/// Draws an SF Symbol sized to fill the menu bar's height, with twice the pixels so it stays
/// sharp on Retina screens.
fn menu_bar_icon(symbol: &str) -> Option<Icon> {
    autoreleasepool(|_| {
        let image = images::symbol(symbol, 15.0)?;
        let size = image.size();
        let (width, height) = ((size.width * 2.0).round() as usize, (size.height * 2.0).round() as usize);
        let pixels = images::draw_into_pixels(&image, width, height)?;
        Icon::from_rgba(pixels, width as u32, height as u32).ok()
    })
}
