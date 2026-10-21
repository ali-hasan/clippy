//! Colors, fonts and spacing that make the window look like a native macOS app.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle,
    Theme, vec2,
};
use eframe::epaint::text::{FontTweak, VariationCoords};

/// The fonts macOS uses for its own interface (SF Pro and SF Mono), read from the system rather
/// than shipped with the app. Characters they lack, such as emoji, come from other system fonts.
const SYSTEM_FONT: &str = "/System/Library/Fonts/SFNS.ttf";
const SYSTEM_MONO_FONT: &str = "/System/Library/Fonts/SFNSMono.ttf";
/// Used if SF Pro cannot be read.
const BACKUP_FONT: &str = "/System/Library/Fonts/Helvetica.ttc";

/// Font family for emphasized text, such as each item's text in the list.
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color32,
    pub footer: Color32,
    pub separator: Color32,
    pub text: Color32,
    pub secondary_text: Color32,
    pub row_hover: Color32,
    pub row_selected: Color32,
    pub badge: Color32,
    pub keycap: Color32,
    pub accent: Color32,
}

impl Palette {
    pub fn of(ui: &egui::Ui) -> Self {
        if ui.visuals().dark_mode { DARK } else { LIGHT }
    }
}

const DARK: Palette = Palette {
    background: Color32::from_rgb(0x1e, 0x1e, 0x20),
    footer: Color32::from_rgb(0x19, 0x19, 0x1b),
    separator: Color32::from_rgb(0x2f, 0x2f, 0x33),
    text: Color32::from_rgb(0xf2, 0xf2, 0xf4),
    secondary_text: Color32::from_rgb(0x8e, 0x8e, 0x93),
    row_hover: Color32::from_rgb(0x27, 0x27, 0x2a),
    row_selected: Color32::from_rgb(0x30, 0x30, 0x34),
    badge: Color32::from_rgb(0x2c, 0x2c, 0x30),
    keycap: Color32::from_rgb(0x3a, 0x3a, 0x3e),
    accent: Color32::from_rgb(0x0a, 0x84, 0xff),
};

const LIGHT: Palette = Palette {
    background: Color32::from_rgb(0xfc, 0xfc, 0xfd),
    footer: Color32::from_rgb(0xf3, 0xf3, 0xf5),
    separator: Color32::from_rgb(0xe3, 0xe3, 0xe7),
    text: Color32::from_rgb(0x1d, 0x1d, 0x1f),
    secondary_text: Color32::from_rgb(0x86, 0x86, 0x8b),
    row_hover: Color32::from_rgb(0xf2, 0xf2, 0xf5),
    row_selected: Color32::from_rgb(0xe8, 0xe8, 0xed),
    badge: Color32::from_rgb(0xec, 0xec, 0xf0),
    keycap: Color32::from_rgb(0xdd, 0xdd, 0xe2),
    accent: Color32::from_rgb(0x00, 0x7a, 0xff),
};

/// Applies the fonts and both color schemes; egui follows the system's light or dark mode.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.style_mut_of(Theme::Dark, |style| apply(style, DARK));
    ctx.style_mut_of(Theme::Light, |style| apply(style, LIGHT));
}

fn install_fonts(ctx: &egui::Context) {
    let read = |paths: &[&str]| paths.iter().find_map(|path| std::fs::read(path).ok()).map(FontData::from_owned);
    let mut fonts = FontDefinitions::empty();
    if let Some(font) = read(&[SYSTEM_FONT, BACKUP_FONT]) {
        // The system font is one file holding every weight; pick two of them.
        let weight = |wght: f32| FontTweak {
            coords: VariationCoords::new([(b"wght", wght)]),
            ..Default::default()
        };
        fonts.font_data.insert("regular".into(), Arc::new(font.clone().tweak(weight(400.0))));
        fonts.font_data.insert("medium".into(), Arc::new(font.tweak(weight(510.0))));
    }
    if let Some(font) = read(&[SYSTEM_MONO_FONT]) {
        fonts.font_data.insert("mono".into(), Arc::new(font));
    }
    let loaded = |names: &[&str]| -> Vec<String> {
        names.iter().filter(|name| fonts.font_data.contains_key(**name)).map(|name| name.to_string()).collect()
    };
    let (regular, medium, mono) = (loaded(&["regular"]), loaded(&["medium"]), loaded(&["mono", "regular"]));
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(FontFamily::Name("medium".into()), medium);
    fonts.families.insert(FontFamily::Monospace, mono);
    ctx.set_fonts(fonts);
}

fn apply(style: &mut egui::Style, palette: Palette) {
    style.text_styles = [
        (TextStyle::Heading, regular(18.0)),
        (TextStyle::Body, regular(13.5)),
        (TextStyle::Button, regular(13.5)),
        (TextStyle::Small, regular(11.5)),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
    style.spacing.item_spacing = vec2(8.0, 4.0);
    style.spacing.button_padding = vec2(10.0, 5.0);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.tooltip_width = 420.0;
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    // Text in the list is for clicking, not for selecting with the mouse.
    style.interaction.selectable_labels = false;

    let visuals = &mut style.visuals;
    visuals.panel_fill = palette.background;
    visuals.extreme_bg_color = palette.background;
    visuals.weak_text_color = Some(palette.secondary_text);
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.text_cursor.stroke = Stroke::new(2.0, palette.accent);

    // Tooltips and the right-click menu.
    visuals.window_fill = palette.background;
    visuals.window_stroke = Stroke::new(1.0, palette.separator);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    let shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(50) };
    visuals.window_shadow = shadow;
    visuals.popup_shadow = shadow;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.separator);
    for state in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
        state.corner_radius = CornerRadius::same(6);
        state.expansion = 0.0;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0, palette.text);
    }
    widgets.hovered.weak_bg_fill = palette.row_selected;
    widgets.active.weak_bg_fill = palette.row_selected;
}
