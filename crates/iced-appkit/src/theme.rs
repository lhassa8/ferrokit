//! An Iced theme built from the system's colours (risk 8): background, text and
//! accent follow System Settings, including the accent colour.

use std::cell::RefCell;

use appkit_host::Rgba;
use iced::theme::Palette;
use iced::{Color, Theme};

thread_local! {
    static CACHE: RefCell<Option<((u64, bool), Theme)>> = const { RefCell::new(None) };
}

fn color(c: Rgba) -> Color {
    Color::from_rgba(c.r, c.g, c.b, c.a)
}

/// The system palette as an Iced theme, for the window's current appearance.
/// Cheap to call every frame: rebuilt only when the appearance or the system
/// colours change. Call from your `theme` function.
pub fn system() -> Theme {
    let dark = appkit_host::is_dark();
    let key = (appkit_host::appearance_generation(), dark);
    if let Some(theme) = CACHE.with_borrow(|c| c.as_ref().filter(|(k, _)| *k == key).map(|(_, t)| t.clone())) {
        return theme;
    }
    let c = appkit_host::system_colors();
    let palette = Palette {
        background: color(c.window_background),
        text: color(c.text),
        primary: color(c.accent),
        success: color(c.green),
        warning: color(c.orange),
        danger: color(c.red),
    };
    let theme = Theme::custom(if c.dark { "System Dark" } else { "System Light" }, palette);
    CACHE.with_borrow_mut(|cache| *cache = Some((key, theme.clone())));
    theme
}
