//! Names and icons of the apps that text was copied from.

use std::collections::HashMap;

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSFileManager, NSString};

use crate::images;

/// Icons are drawn at about 30 points; this many pixels keeps them sharp on Retina screens.
const ICON_PIXELS: usize = 64;

pub struct SourceApp {
    pub name: String,
    pub icon: TextureHandle,
}

/// Looks each app up once and remembers it, including apps that could not be found.
#[derive(Default)]
pub struct SourceApps {
    known: HashMap<String, Option<SourceApp>>,
}

impl SourceApps {
    /// The app with this bundle identifier, or `None` if it is no longer installed.
    pub fn get(&mut self, ctx: &egui::Context, bundle_id: &str) -> Option<&SourceApp> {
        self.known
            .entry(bundle_id.to_string())
            .or_insert_with(|| {
                let (name, icon) = autoreleasepool(|_| look_up(bundle_id))?;
                Some(SourceApp { name, icon: ctx.load_texture(bundle_id, icon, TextureOptions::LINEAR) })
            })
            .as_ref()
    }
}

fn look_up(bundle_id: &str) -> Option<(String, ColorImage)> {
    let workspace = NSWorkspace::sharedWorkspace();
    let path = workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id))?.path()?;
    let name = NSFileManager::defaultManager().displayNameAtPath(&path).to_string();
    let name = name.strip_suffix(".app").unwrap_or(&name).to_string();
    let pixels = images::draw_into_pixels(&workspace.iconForFile(&path), ICON_PIXELS, ICON_PIXELS)?;
    Some((name, ColorImage::from_rgba_premultiplied([ICON_PIXELS, ICON_PIXELS], &pixels)))
}
