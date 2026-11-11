//! Real input for checks: clicks and keystrokes that take the path a user's
//! do, not AppKit's entry points (`performClick`, `selectRowIndexes`).
//!
//! Two kinds:
//! - **Event-queue input** (`click`, `type_text`, `key`…): mouse and key
//!   events posted to the app's own event queue. AppKit dispatches them as
//!   it dispatches the window server's: `sendEvent`, hit-testing, key
//!   equivalents through the menus, the first responder, the field editor
//!   and text input. They need the app active and the window key (see
//!   [`ready`]); no permission.
//! - **Window-server input** (`server_click`, `server_type`): `CGEvent`s
//!   posted to this process, through the window server like a real
//!   keyboard and mouse, without moving the pointer. They need the process
//!   to be trusted for Accessibility (System Settings › Privacy & Security
//!   › Accessibility, for the terminal that runs the checks):
//!   [`server_available`] says whether it is. Checks skip them, and say so,
//!   when it isn't.
//!
//! Events are delivered on the run loop's next turns: check their effect a
//! moment later.

use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow};
use objc2_foundation::{NSPoint, NSProcessInfo, NSString};

/// A modifier key held with [`key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mod {
    /// ⌘
    Command,
    /// ⇧
    Shift,
    /// ⌥
    Option,
    /// ⌃
    Control,
}

/// A key for [`key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A character key: a letter, digit or punctuation.
    Char(char),
    /// Return.
    Return,
    /// Tab.
    Tab,
    /// Escape.
    Escape,
    /// Delete (backspace).
    Delete,
    /// The arrow keys.
    Left,
    /// →
    Right,
    /// ↑
    Up,
    /// ↓
    Down,
}

/// The ANSI key code of a character (what `keyCode` reports), 0 if unknown.
fn code_of(c: char) -> u16 {
    const KEYS: &str = "asdfhgzxcv\0bqweryt123465=97-80]ou[ip\0lj'k;\\,/nm.";
    if c == ' ' {
        return 49;
    }
    KEYS.chars().position(|k| k == c.to_ascii_lowercase()).map_or(0, |i| i as u16)
}

impl Key {
    /// (key code, characters, whether it's a function key).
    fn parts(self) -> (u16, String, bool) {
        let f = |c: u32| char::from_u32(c).unwrap().to_string();
        match self {
            Key::Char(c) => (code_of(c), c.to_string(), false),
            Key::Return => (36, "\r".into(), false),
            Key::Tab => (48, "\t".into(), false),
            Key::Escape => (53, "\u{1b}".into(), false),
            Key::Delete => (51, "\u{7f}".into(), false),
            Key::Left => (123, f(0xF702), true),
            Key::Right => (124, f(0xF703), true),
            Key::Down => (125, f(0xF701), true),
            Key::Up => (126, f(0xF700), true),
        }
    }
}

fn flags(mods: &[Mod]) -> NSEventModifierFlags {
    let mut f = NSEventModifierFlags::empty();
    for m in mods {
        f |= match m {
            Mod::Command => NSEventModifierFlags::Command,
            Mod::Shift => NSEventModifierFlags::Shift,
            Mod::Option => NSEventModifierFlags::Option,
            Mod::Control => NSEventModifierFlags::Control,
        };
    }
    f
}

fn now() -> f64 {
    NSProcessInfo::processInfo().systemUptime()
}

fn post(event: Option<Retained<NSEvent>>) {
    if let Some(e) = event {
        NSApplication::sharedApplication(crate::mtm()).postEvent_atStart(&e, false);
    }
}

/// Brings the app to the front and makes `window` key, so input reaches it.
/// True once both hold (it can take a run-loop turn: call it, wait a
/// moment, and call it again before sending input).
///
/// It stays false while the screen is locked, or when macOS doesn't let
/// the app take the front from the one in use: nothing is wrong with the
/// app then, and a check prints `SKIP` instead of sending input.
pub fn ready(window: &NSWindow) -> bool {
    let app = NSApplication::sharedApplication(crate::mtm());
    if !app.isActive() {
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
    if !window.isKeyWindow() {
        window.makeKeyAndOrderFront(None);
    }
    app.isActive() && window.isKeyWindow()
}

/// The point `x, y` points from `view`'s top-left corner, in its window's
/// coordinates.
fn in_window(view: &NSView, x: f64, y: f64) -> NSPoint {
    let b = view.bounds();
    let y = if view.isFlipped() { y } else { b.size.height - y };
    view.convertPoint_toView(NSPoint::new(b.origin.x + x, b.origin.y + y), None)
}

fn mouse(window: &NSWindow, kind: NSEventType, at: NSPoint, clicks: isize, mods: &[Mod]) {
    post(NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        at,
        flags(mods),
        now(),
        window.windowNumber(),
        None,
        0,
        clicks,
        1.0,
    ));
}

/// Clicks `view` at `x, y` points from its top-left corner, holding `mods`.
pub fn click_at_with(view: &NSView, x: f64, y: f64, mods: &[Mod]) -> bool {
    let Some(window) = view.window() else { return false };
    let at = in_window(view, x, y);
    mouse(&window, NSEventType::LeftMouseDown, at, 1, mods);
    mouse(&window, NSEventType::LeftMouseUp, at, 1, mods);
    true
}

/// Clicks `view` at `x, y` points from its top-left corner.
pub fn click_at(view: &NSView, x: f64, y: f64) -> bool {
    click_at_with(view, x, y, &[])
}

/// Clicks the middle of `view`, as the user would: the window hit-tests
/// the point, so whatever is on top there gets the click.
pub fn click(view: &NSView) -> bool {
    let b = view.bounds();
    click_at(view, b.size.width / 2.0, b.size.height / 2.0)
}

/// Double-clicks the middle of `view`.
pub fn double_click(view: &NSView) -> bool {
    let Some(window) = view.window() else { return false };
    let b = view.bounds();
    let at = in_window(view, b.size.width / 2.0, b.size.height / 2.0);
    for clicks in [1, 2] {
        mouse(&window, NSEventType::LeftMouseDown, at, clicks, &[]);
        mouse(&window, NSEventType::LeftMouseUp, at, clicks, &[]);
    }
    true
}

/// Presses `key` with `mods` in `window` (to its first responder, or the
/// menus' key equivalents when ⌘ is held).
pub fn key(window: &NSWindow, key: Key, mods: &[Mod]) {
    let (code, chars, function) = key.parts();
    let mut f = flags(mods);
    if function {
        f |= NSEventModifierFlags::Function | NSEventModifierFlags::NumericPad;
    }
    let s = NSString::from_str(&chars);
    for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
        post(NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            kind,
            NSPoint::ZERO,
            f,
            now(),
            window.windowNumber(),
            None,
            &s,
            &s,
            false,
            code,
        ));
    }
}

/// Types `text` into `window`'s first responder, a key at a time (capitals
/// with ⇧, line breaks as Return).
pub fn type_text(window: &NSWindow, text: &str) {
    for c in text.chars() {
        match c {
            '\n' => key(window, Key::Return, &[]),
            '\t' => key(window, Key::Tab, &[]),
            c if c.is_uppercase() => key(window, Key::Char(c), &[Mod::Shift]),
            c => key(window, Key::Char(c), &[]),
        }
    }
}

// ---- through the window server ----

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

/// Whether window-server input can be posted: this process is trusted for
/// Accessibility. (Asking for it is the user's to do, in System Settings.)
pub fn server_available() -> bool {
    // SAFETY: a plain query with no side effects (it doesn't prompt).
    unsafe { AXIsProcessTrusted() != 0 }
}

fn pid() -> i32 {
    std::process::id() as i32
}

/// Clicks the middle of `view` through the window server (see the module
/// docs); false if that isn't available.
pub fn server_click(view: &NSView) -> bool {
    use objc2_core_graphics::{CGEvent, CGEventType, CGMouseButton};
    let Some(window) = view.window().filter(|_| server_available()) else { return false };
    let b = view.bounds();
    let p = window.convertPointToScreen(in_window(view, b.size.width / 2.0, b.size.height / 2.0));
    // Global coordinates have their origin at the top left of the main screen.
    let top = objc2_app_kit::NSScreen::screens(crate::mtm()).firstObject().map_or(0.0, |s| s.frame().size.height);
    let at = objc2_core_foundation::CGPoint::new(p.x, top - p.y);
    for kind in [CGEventType::LeftMouseDown, CGEventType::LeftMouseUp] {
        if let Some(e) = CGEvent::new_mouse_event(None, kind, at, CGMouseButton::Left) {
            CGEvent::post_to_pid(pid(), Some(&e));
        }
    }
    true
}

/// Types `text` through the window server (see the module docs); false if
/// that isn't available.
pub fn server_type(text: &str) -> bool {
    use objc2_core_graphics::CGEvent;
    if !server_available() {
        return false;
    }
    for c in text.chars() {
        let units: Vec<u16> = c.to_string().encode_utf16().collect();
        for down in [true, false] {
            if let Some(e) = CGEvent::new_keyboard_event(None, code_of(c), down) {
                // SAFETY: a UTF-16 buffer of the given length.
                unsafe { CGEvent::keyboard_set_unicode_string(Some(&e), units.len() as _, units.as_ptr()) };
                CGEvent::post_to_pid(pid(), Some(&e));
            }
        }
    }
    true
}
