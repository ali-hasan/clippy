//! The history window: a search box above the list of copied text and images.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Align, Align2, ColorImage, CornerRadius, Galley, Key, Layout, Margin, Modifiers, Painter, Pos2, Rect,
    RichText, Sense, Stroke, TextureHandle, TextureOptions, pos2, vec2,
};
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

use crate::HOTKEY_LABEL;
use crate::apps::{SourceApp, SourceApps};
use crate::clipboard;
use crate::history::{self, Content, History, Item, ItemId, Thumbnail};
use crate::theme::{self, Palette};

/// Most characters shown for an item in the list, and in the tooltip with its full text.
const PREVIEW_CHARS: usize = 200;
const TOOLTIP_CHARS: usize = 2000;

const ROW_HEIGHT: f32 = 46.0;
const HEADER_MARGIN: Margin = Margin::symmetric(16, 14);
const FOOTER_MARGIN: Margin = Margin::symmetric(14, 8);

/// State used by the window, the menu bar menu, the hotkey and the clipboard watcher.
pub struct Shared {
    pub history: Mutex<History>,
    /// Tells the window to reload its list from the database.
    pub history_changed: AtomicBool,
    /// Tells the window it was just opened, so it starts with an empty search.
    pub just_opened: AtomicBool,
}

impl Shared {
    pub fn new(history: History) -> Self {
        Self {
            history: Mutex::new(history),
            history_changed: AtomicBool::new(true),
            just_opened: AtomicBool::new(true),
        }
    }
}

/// Opens the window, or hides it if it is already in front.
pub fn toggle_window(shared: &Shared, ctx: &egui::Context) {
    let Some(app) = ns_app() else { return };
    if app.isActive() && !app.isHidden() {
        app.hide(None);
    } else {
        open_window(shared, ctx);
    }
}

pub fn open_window(shared: &Shared, ctx: &egui::Context) {
    let Some(app) = ns_app() else { return };
    shared.just_opened.store(true, Ordering::Relaxed);
    app.unhide(None);
    // The newer `activate()` may be ignored when triggered by a global hotkey.
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    ctx.request_repaint();
}

/// Hides the whole app rather than just the window, so macOS returns focus to the app the user
/// came from, ready for ⌘V.
fn hide_window() {
    if let Some(app) = ns_app() {
        app.hide(None);
    }
}

/// The running macOS application, or `None` off the main thread, where AppKit must not be used.
fn ns_app() -> Option<objc2::rc::Retained<NSApplication>> {
    MainThreadMarker::new().map(NSApplication::sharedApplication)
}

pub struct ClippyApp {
    shared: Arc<Shared>,
    view: View,
    /// The menu bar icon and the hotkey stop working once these are dropped.
    _keep_alive: Box<dyn std::any::Any>,
}

impl ClippyApp {
    pub fn new(shared: Arc<Shared>, keep_alive: Box<dyn std::any::Any>) -> Self {
        Self { shared, view: View::default(), _keep_alive: keep_alive }
    }

    fn reload(&mut self) {
        let items = self.shared.history.lock().unwrap().items();
        self.view.set_items(items.unwrap_or_else(|err| {
            eprintln!("clippy: could not read history: {err}");
            Vec::new()
        }));
    }
}

impl eframe::App for ClippyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.shared.history_changed.swap(false, Ordering::Relaxed) {
            self.reload();
        }
        if self.shared.just_opened.swap(false, Ordering::Relaxed) {
            self.view.start_over();
        }
        // A request to close the window only hides it; quitting is in the menu bar menu.
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            hide_window();
        }

        match self.view.show(ui, history::now_millis()) {
            Some(Action::CopyText(text)) => {
                clipboard::write_text(&text);
                hide_window();
            }
            Some(Action::CopyImage(id)) => {
                let png = self.shared.history.lock().unwrap().image_png(id);
                match png {
                    Ok(png) => {
                        clipboard::write_png(&png);
                        hide_window();
                    }
                    Err(err) => eprintln!("clippy: could not load image: {err}"),
                }
            }
            Some(Action::Delete(id)) => {
                if let Err(err) = self.shared.history.lock().unwrap().delete(id) {
                    eprintln!("clippy: could not delete item: {err}");
                }
                self.reload();
            }
            Some(Action::Close) => hide_window(),
            None => {}
        }
    }
}

/// What the user asked for in the window.
pub enum Action {
    CopyText(String),
    /// Copies the image with this id; the window only has its thumbnail.
    CopyImage(i64),
    Delete(ItemId),
    Close,
}

struct Entry {
    item: Item,
    preview: String,
    size: String,
}

impl Entry {
    fn new(item: Item) -> Self {
        let (preview, size) = match &item.content {
            Content::Text { text, .. } => (preview(text), size_label(text)),
            Content::Image { width, height, .. } => ("Image".to_string(), format!("{width} × {height}")),
        };
        Self { item, preview, size }
    }

    /// What the search looks in; images can be found by typing "image".
    fn search_text(&self) -> &str {
        match &self.item.content {
            Content::Text { text, .. } => text,
            Content::Image { .. } => "image",
        }
    }

    fn copy_action(&self) -> Action {
        match &self.item.content {
            Content::Text { text, .. } => Action::CopyText(text.clone()),
            Content::Image { id, .. } => Action::CopyImage(*id),
        }
    }
}

/// Draws the window's contents. It never touches the clipboard or the database itself, so it
/// can be drawn in tests.
#[derive(Default)]
pub struct View {
    entries: Vec<Entry>,
    /// Positions in `entries` that match the search, in the order shown.
    matching: Vec<usize>,
    query: String,
    /// Row within `matching` that Enter copies.
    selected: usize,
    apps: SourceApps,
    /// Image thumbnails ready to draw, by image id.
    thumbnails: HashMap<i64, TextureHandle>,
}

impl View {
    pub fn set_items(&mut self, items: Vec<Item>) {
        self.entries = items.into_iter().map(Entry::new).collect();
        let shown: Vec<ItemId> = self.entries.iter().map(|entry| entry.item.id()).collect();
        self.thumbnails.retain(|id, _| shown.contains(&ItemId::Image(*id)));
        self.refilter();
    }

    /// Empties the search and selects the newest item, as each time the window opens.
    pub fn start_over(&mut self) {
        self.query.clear();
        self.selected = 0;
        self.refilter();
    }

    fn refilter(&mut self) {
        let words = search_words(&self.query);
        self.matching = (0..self.entries.len())
            .filter(|&i| matches(self.entries[i].search_text(), &words))
            .collect();
        self.selected = self.selected.min(self.matching.len().saturating_sub(1));
    }


    pub fn show(&mut self, ui: &mut egui::Ui, now_ms: i64) -> Option<Action> {
        let palette = Palette::of(ui);
        let mut action = None;
        let selection_moved = self.handle_keys(ui.ctx(), &mut action);

        egui::Panel::top("search")
            .frame(egui::Frame::NONE.fill(palette.background).inner_margin(HEADER_MARGIN))
            .show(ui, |ui| {
                drag_window_from_background(ui, HEADER_MARGIN);
                self.search_bar(ui, palette);
            });
        egui::Panel::bottom("footer")
            .frame(egui::Frame::NONE.fill(palette.footer).inner_margin(FOOTER_MARGIN))
            .show(ui, |ui| {
                drag_window_from_background(ui, FOOTER_MARGIN);
                self.footer(ui, palette);
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(palette.background).inner_margin(Margin::same(8)))
            .show(ui, |ui| {
                if self.matching.is_empty() {
                    self.empty_state(ui, palette);
                } else {
                    self.list(ui, palette, now_ms, selection_moved, &mut action);
                }
            });
        action
    }

    /// Taken before the search box sees them, so the arrows move through the list and Enter or
    /// Escape don't take the cursor out of the search box. Returns whether the selection moved.
    fn handle_keys(&mut self, ctx: &egui::Context, action: &mut Option<Action>) -> bool {
        let mut moved = false;
        ctx.input_mut(|input| {
            if input.consume_key(Modifiers::NONE, Key::ArrowDown) && self.selected + 1 < self.matching.len() {
                self.selected += 1;
                moved = true;
            }
            if input.consume_key(Modifiers::NONE, Key::ArrowUp) && self.selected > 0 {
                self.selected -= 1;
                moved = true;
            }
            if input.consume_key(Modifiers::NONE, Key::Enter) {
                *action = self.matching.get(self.selected).map(|&i| self.entries[i].copy_action());
            }
            if input.consume_key(Modifiers::NONE, Key::Escape) {
                if self.query.is_empty() {
                    *action = Some(Action::Close);
                } else {
                    self.query.clear();
                    self.refilter();
                }
            }
        });
        moved
    }

    fn search_bar(&mut self, ui: &mut egui::Ui, palette: Palette) {
        ui.horizontal(|ui| {
            let (icon, _) = ui.allocate_exact_size(vec2(20.0, 24.0), Sense::hover());
            paint_magnifier(ui.painter(), icon.center(), 18.0, palette.secondary_text);
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(RichText::new("Search clipboard history").color(palette.secondary_text))
                    .font(theme::regular(17.0))
                    .text_color(palette.text)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
            );
            search.request_focus();
            if search.changed() {
                self.selected = 0;
                self.refilter();
            }
        });
    }

    fn list(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        now_ms: i64,
        selection_moved: bool,
        action: &mut Option<Action>,
    ) {
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (row, &index) in self.matching.iter().enumerate() {
                let entry = &self.entries[index];
                let selected = row == self.selected;
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
                if selection_moved && selected {
                    response.scroll_to_me(None);
                }
                // Icons and thumbnails are prepared only for rows on screen, so a long history
                // doesn't load them all at once.
                let visible = ui.is_rect_visible(rect);
                let thumbnail = match &entry.item.content {
                    Content::Image { id, thumbnail, .. } if visible => {
                        Some(self.thumbnails.entry(*id).or_insert_with(|| load_thumbnail(&ctx, *id, thumbnail)).clone())
                    }
                    _ => None,
                };
                if visible {
                    let app = entry.item.source_app.as_deref().and_then(|id| self.apps.get(&ctx, id));
                    let age = age_label(entry.item.copied_at, now_ms);
                    let details = match app {
                        Some(app) => format!("{} · {age} · {}", app.name, entry.size),
                        None => format!("{age} · {}", entry.size),
                    };
                    let look = RowLook { selected, hovered: response.hovered(), palette };
                    paint_row(ui, rect, entry, app, thumbnail.as_ref(), &details, look);
                }
                if response.clicked() {
                    *action = Some(entry.copy_action());
                }
                response.context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        *action = Some(Action::Delete(entry.item.id()));
                    }
                });
                response.on_hover_ui(|ui| match (&entry.item.content, &thumbnail) {
                    // Thumbnails have twice the pixels needed, for Retina screens.
                    (Content::Image { .. }, Some(texture)) => {
                        ui.add(egui::Image::new((texture.id(), texture.size_vec2() / 2.0)).corner_radius(6));
                    }
                    (Content::Text { text, .. }, _) => {
                        ui.label(truncate_chars(text, TOOLTIP_CHARS));
                    }
                    _ => {}
                });
            }
        });
    }

    fn footer(&self, ui: &mut egui::Ui, palette: Palette) {
        ui.horizontal(|ui| {
            let count = plural(self.entries.len(), "item");
            ui.label(RichText::new(count).size(11.5).color(palette.secondary_text));
            // Laid out from the right edge, so the last hint is added first.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for (key, label) in [("esc", "Close"), ("⏎", "Copy")] {
                    ui.label(RichText::new(label).size(11.5).color(palette.secondary_text));
                    keycap(ui, key, palette);
                    ui.add_space(10.0);
                }
            });
        });
    }

    fn empty_state(&self, ui: &mut egui::Ui, palette: Palette) {
        let nothing_copied = self.entries.is_empty();
        let (title, subtitle) = if nothing_copied {
            ("Nothing copied yet", format!("Copy some text, then press {HOTKEY_LABEL} to find it here."))
        } else {
            ("No matches", "Try other words.".to_string())
        };
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.28).max(12.0));
            let (icon, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
            ui.painter().rect_filled(icon, CornerRadius::same(12), palette.badge);
            if nothing_copied {
                paint_clipboard(ui.painter(), icon.center(), 30.0, palette.secondary_text);
            } else {
                paint_magnifier(ui.painter(), icon.center(), 28.0, palette.secondary_text);
            }
            ui.add_space(12.0);
            ui.label(RichText::new(title).font(theme::medium(15.0)).color(palette.text));
            ui.label(RichText::new(subtitle).size(12.5).color(palette.secondary_text));
        });
    }
}

/// The window has no title bar, so dragging an empty part of the header or footer moves it.
fn drag_window_from_background(ui: &mut egui::Ui, margin: Margin) {
    let area = ui.max_rect() + margin;
    let response = ui.interact(area, ui.scope_id().with("drag window"), Sense::drag());
    if response.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

fn load_thumbnail(ctx: &egui::Context, id: i64, thumbnail: &Thumbnail) -> TextureHandle {
    let size = [thumbnail.width as usize, thumbnail.height as usize];
    let image = ColorImage::from_rgba_premultiplied(size, &thumbnail.pixels);
    ctx.load_texture(format!("image {id}"), image, TextureOptions::LINEAR)
}

struct RowLook {
    selected: bool,
    hovered: bool,
    palette: Palette,
}

/// One list row: the icon of the app it was copied from (or a small badge when that is unknown),
/// the text on one line with where and when it was copied below it, and for images a thumbnail.
fn paint_row(
    ui: &egui::Ui,
    rect: Rect,
    entry: &Entry,
    app: Option<&SourceApp>,
    thumbnail: Option<&TextureHandle>,
    details: &str,
    look: RowLook,
) {
    let RowLook { selected, hovered, palette } = look;
    let painter = ui.painter();
    let fill = match (selected, hovered) {
        (true, _) => palette.row_selected,
        (false, true) => palette.row_hover,
        (false, false) => egui::Color32::TRANSPARENT,
    };
    painter.rect_filled(rect, CornerRadius::same(8), fill);

    let badge = Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), vec2(28.0, 28.0));
    if let Some(app) = app {
        // App icons have a transparent margin of their own, so they are drawn a little larger.
        let whole_image = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(app.icon.id(), badge.expand(2.0), whole_image, egui::Color32::WHITE);
    } else {
        let (badge_fill, badge_mark) =
            if selected { (palette.accent, egui::Color32::WHITE) } else { (palette.badge, palette.secondary_text) };
        painter.rect_filled(badge, CornerRadius::same(7), badge_fill);
        match entry.item.content {
            Content::Text { .. } => {
                painter.text(badge.center(), Align2::CENTER_CENTER, "Aa", theme::medium(11.0), badge_mark);
            }
            Content::Image { .. } => paint_picture(painter, badge.center(), badge_mark),
        }
    }

    let text_left = badge.right() + 12.0;
    let mut text_right = rect.right() - 12.0;
    if selected {
        let key = keycap_galley(painter, "⏎", palette);
        let size = keycap_size(&key);
        let cap = Rect::from_min_size(pos2(text_right - size.x, rect.center().y - size.y / 2.0), size);
        paint_keycap(painter, cap, key, palette);
        text_right = cap.left() - 10.0;
    }
    if let Some(texture) = thumbnail {
        // Fits the thumbnail in a small box on the right, without enlarging tiny images.
        let pixels = texture.size_vec2();
        let scale = (56.0 / pixels.x).min(34.0 / pixels.y).min(1.0);
        let size = pixels * scale;
        let area = Rect::from_min_size(pos2(text_right - size.x, rect.center().y - size.y / 2.0), size);
        egui::Image::new((texture.id(), size)).corner_radius(4).paint_at(ui, area);
        painter.rect_stroke(area, CornerRadius::same(4), Stroke::new(1.0, palette.separator), egui::StrokeKind::Inside);
        text_right = area.left() - 12.0;
    }
    let width = (text_right - text_left).max(0.0);
    let title = one_line(painter, &entry.preview, theme::medium(13.5), palette.text, width);
    painter.galley(pos2(text_left, rect.top() + 7.0), title, palette.text);
    let details = one_line(painter, details, theme::regular(11.5), palette.secondary_text, width);
    painter.galley(pos2(text_left, rect.top() + 26.0), details, palette.secondary_text);
}

/// Lays out text on a single line, ending in "…" when it is wider than `width`.
fn one_line(painter: &Painter, text: &str, font: egui::FontId, color: egui::Color32, width: f32) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

fn keycap(ui: &mut egui::Ui, key: &str, palette: Palette) {
    let galley = keycap_galley(ui.painter(), key, palette);
    let (rect, _) = ui.allocate_exact_size(keycap_size(&galley), Sense::hover());
    paint_keycap(ui.painter(), rect, galley, palette);
}

fn keycap_galley(painter: &Painter, key: &str, palette: Palette) -> Arc<Galley> {
    painter.layout_no_wrap(key.to_string(), theme::medium(10.5), palette.secondary_text)
}

fn keycap_size(galley: &Galley) -> egui::Vec2 {
    vec2((galley.size().x + 10.0).max(20.0), 18.0)
}

fn paint_keycap(painter: &Painter, rect: Rect, galley: Arc<Galley>, palette: Palette) {
    painter.rect_filled(rect, CornerRadius::same(4), palette.keycap);
    painter.galley(rect.center() - galley.size() / 2.0, galley, palette.secondary_text);
}

/// A small framed landscape, the badge for an image when its app is unknown.
fn paint_picture(painter: &Painter, center: Pos2, color: egui::Color32) {
    let frame = Rect::from_center_size(center, vec2(16.0, 12.0));
    painter.rect_stroke(frame, CornerRadius::same(2), Stroke::new(1.4, color), egui::StrokeKind::Middle);
    painter.circle_filled(frame.left_top() + vec2(11.5, 3.5), 1.5, color);
    let ground = frame.left_bottom() + vec2(2.0, -2.0);
    let hill = vec![ground, ground + vec2(4.5, -5.5), ground + vec2(9.0, 0.0)];
    painter.add(egui::Shape::convex_polygon(hill, color, Stroke::NONE));
}

fn paint_magnifier(painter: &Painter, center: Pos2, size: f32, color: egui::Color32) {
    let stroke = Stroke::new(size * 0.09, color);
    let lens = center - vec2(size, size) * 0.08;
    let radius = size * 0.3;
    painter.circle_stroke(lens, radius, stroke);
    let handle_start = lens + vec2(radius, radius) * std::f32::consts::FRAC_1_SQRT_2;
    painter.line_segment([handle_start, center + vec2(size, size) * 0.36], stroke);
}

fn paint_clipboard(painter: &Painter, center: Pos2, size: f32, color: egui::Color32) {
    let stroke = Stroke::new(size * 0.07, color);
    let board = Rect::from_center_size(center + vec2(0.0, size * 0.04), vec2(size * 0.64, size * 0.8));
    painter.rect_stroke(board, CornerRadius::same((size * 0.1) as u8), stroke, egui::StrokeKind::Middle);
    let clip = Rect::from_center_size(pos2(center.x, board.top()), vec2(size * 0.32, size * 0.16));
    painter.rect_filled(clip, CornerRadius::same((size * 0.05) as u8), color);
    for line in 0..3 {
        let y = board.top() + size * (0.26 + 0.15 * line as f32);
        painter.line_segment([pos2(center.x - size * 0.16, y), pos2(center.x + size * 0.16, y)], stroke);
    }
}

/// The list shows each item on one line, with line breaks and runs of spaces as single spaces.
fn preview(text: &str) -> String {
    // Only the start of a long copy can ever be shown, so skip collapsing the rest of it.
    let start = truncate_chars(text, PREVIEW_CHARS * 4);
    truncate_chars(&start.split_whitespace().collect::<Vec<_>>().join(" "), PREVIEW_CHARS)
}

/// How long a copy is, shown under it in the list.
fn size_label(text: &str) -> String {
    let lines = text.lines().count();
    if lines > 1 { plural(lines, "line") } else { plural(text.trim().chars().count(), "character") }
}

/// When an item was copied, in words, such as "5 min ago" or "Yesterday".
fn age_label(copied_at_ms: i64, now_ms: i64) -> String {
    let minutes = (now_ms - copied_at_ms).max(0) / 60_000;
    match minutes {
        0 => "Just now".to_string(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} hr ago", minutes / 60),
        1440..2880 => "Yesterday".to_string(),
        _ => format!("{} days ago", minutes / 1440),
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

fn search_words(query: &str) -> Vec<String> {
    query.to_lowercase().split_whitespace().map(String::from).collect()
}

/// An item matches when it contains every search word, in any order and ignoring case.
fn matches(text: &str, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let text = text.to_lowercase();
    words.iter().all(|word| text.contains(word.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;

    fn search(text: &str, query: &str) -> bool {
        matches(text, &search_words(query))
    }

    #[test]
    fn search_ignores_case() {
        assert!(search("Hello World", "hello"));
    }

    #[test]
    fn search_finds_items_containing_all_words_in_any_order() {
        assert!(search("deploy the api to staging", "staging api"));
        assert!(!search("deploy the api to staging", "staging web"));
    }

    #[test]
    fn empty_search_shows_every_item() {
        assert!(search("anything", "   "));
    }

    #[test]
    fn list_shows_multi_line_text_on_one_line() {
        assert_eq!(preview("first line\n\n  second\tline"), "first line second line");
    }

    #[test]
    fn list_cuts_long_text_short_with_an_ellipsis() {
        let shown = preview(&"a".repeat(PREVIEW_CHARS + 50));
        assert_eq!(shown.chars().count(), PREVIEW_CHARS + 1);
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn cutting_text_short_never_splits_a_multi_byte_character() {
        assert_eq!(truncate_chars("héllo wörld", 7), "héllo w…");
    }

    #[test]
    fn copy_time_reads_like_a_person_would_say_it() {
        let now = 30 * DAY;
        assert_eq!(age_label(now - 20_000, now), "Just now");
        assert_eq!(age_label(now - 5 * MINUTE, now), "5 min ago");
        assert_eq!(age_label(now - 3 * HOUR, now), "3 hr ago");
        assert_eq!(age_label(now - 30 * HOUR, now), "Yesterday");
        assert_eq!(age_label(now - 4 * DAY, now), "4 days ago");
    }

    #[test]
    fn copy_size_counts_lines_for_multi_line_text_and_characters_otherwise() {
        assert_eq!(size_label("one\ntwo\nthree"), "3 lines");
        assert_eq!(size_label("hello\n"), "5 characters");
        assert_eq!(size_label("a"), "1 character");
    }

    /// A made-up picture for image items: an orange-to-purple sky above dark ground.
    fn sunset_thumbnail() -> Thumbnail {
        let (width, height) = (256u32, 160u32);
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for _ in 0..width {
                let t = y as f32 / height as f32;
                let [r, g, b] = if t < 0.7 { [255.0 - 120.0 * t, 140.0 - 120.0 * t, 60.0 + 140.0 * t] } else { [40.0, 30.0, 50.0] };
                pixels.extend([r as u8, g as u8, b as u8, 255]);
            }
        }
        Thumbnail { width, height, pixels }
    }

    /// Saves pictures of the window to target/ui-preview/ for checking the design by eye. It
    /// needs a GPU, so it only runs when asked for: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn saves_pictures_of_the_window_in_light_and_dark_mode() {
        let now = 30 * DAY;
        let item = |id, text: &str, age, app: Option<&str>| Item {
            content: Content::Text { id, text: text.to_string() },
            copied_at: now - age,
            source_app: app.map(String::from),
        };
        let image = |id, age, app: Option<&str>| Item {
            content: Content::Image { id, width: 2880, height: 1800, thumbnail: sunset_thumbnail() },
            copied_at: now - age,
            source_app: app.map(String::from),
        };
        // Apple's own apps, so the pictures show real icons on any Mac; two items have no app.
        let items = vec![
            item(1, "https://github.com/emilk/egui/releases", 20_000, Some("com.apple.Safari")),
            image(1, 2 * MINUTE, Some("com.apple.Preview")),
            item(2, "cargo build --release && ./scripts/bundle.sh", 5 * MINUTE, Some("com.apple.Terminal")),
            item(3, "Hi team,\nthe release is out, notes are in the wiki.\nThanks!", 3 * HOUR, Some("com.apple.mail")),
            item(4, "Shipped it 🎉 — 发布完成 ✅", 20 * HOUR, Some("com.apple.Notes")),
            image(2, 2 * DAY, None),
            item(5, "SELECT id, text FROM items ORDER BY copied_at DESC LIMIT 1000;", 4 * DAY, None),
        ];
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ui-preview");
        std::fs::create_dir_all(&folder).unwrap();
        for (theme, name) in [(egui::Theme::Dark, "dark"), (egui::Theme::Light, "light")] {
            for (suffix, items) in [("", items.clone()), ("-empty", Vec::new())] {
                let mut view = View::default();
                view.set_items(items);
                let mut harness = egui_kittest::Harness::builder()
                    .with_size([560.0, 520.0])
                    .with_pixels_per_point(2.0)
                    .with_theme(theme)
                    // The app also looks up system fonts for characters SF Pro lacks, such as emoji.
                    .with_context_setup(|ctx| {
                        theme::install(ctx);
                        ctx.add_font_provider(std::sync::Arc::new(egui_system_font_provider::SystemFontProvider::new()));
                    })
                    .wgpu()
                    .build_ui_state(|ui, view: &mut View| _ = view.show(ui, now), view);
                harness.run_steps(3);
                let picture = harness.render().unwrap();
                picture.save(folder.join(format!("{name}{suffix}.png"))).unwrap();
            }
        }
    }
}
