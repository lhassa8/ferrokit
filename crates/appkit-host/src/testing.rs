//! Helpers for unattended tests: synthetic input posted into the app's own
//! event queue, and probes of AppKit state. Not for production use.

use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::NSPoint;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ViewId;

/// Table and text models alive (Rust side).
pub(crate) static LIVE_MODELS: AtomicUsize = AtomicUsize::new(0);
/// Library-defined Objective-C objects alive (data sources, delegates, text
/// views): decremented when AppKit actually deallocates them.
pub(crate) static LIVE_OBJC: AtomicUsize = AtomicUsize::new(0);

/// (models alive, library Objective-C objects alive), for leak checks.
pub fn live_objects() -> (usize, usize) {
    (LIVE_MODELS.load(Ordering::Relaxed), LIVE_OBJC.load(Ordering::Relaxed))
}
use crate::host::{mtm, with_host};

/// Posts a key-down (optionally with ⌘ / ⇧).
pub fn post_key(chars: &str, key_code: u16, command: bool, shift: bool) {
    crate::keys::post_key_down(chars, key_code, command, shift);
}

/// Posts Tab (or Shift-Tab) to the window.
pub fn post_tab(shift: bool) {
    post_key("\t", 48, false, shift);
}

/// Posts a left click at a point in host coordinates (top-left origin, points).
pub fn post_click(x: f64, y: f64) {
    let Some((window_number, location)) = with_host(|h| {
        let window = h.content.window()?;
        let p = if h.content.isFlipped() { NSPoint::new(x, y) } else { NSPoint::new(x, h.content.bounds().size.height - y) };
        Some((window.windowNumber(), h.content.convertPoint_toView(p, None)))
    })
    .flatten() else {
        return;
    };
    for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind, location, NSEventModifierFlags::empty(), 0.0, window_number, None, 0, 1, 1.0,
        );
        if let Some(event) = event {
            NSApplication::sharedApplication(mtm()).postEvent_atStart(&event, false);
        }
    }
}

/// Posts a mouse move to a point in host coordinates (hover).
pub fn post_mouse_move(x: f64, y: f64) {
    let Some((window_number, location)) = with_host(|h| {
        let window = h.content.window()?;
        let p = if h.content.isFlipped() { NSPoint::new(x, y) } else { NSPoint::new(x, h.content.bounds().size.height - y) };
        Some((window.windowNumber(), h.content.convertPoint_toView(p, None)))
    })
    .flatten() else {
        return;
    };
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::MouseMoved, location, NSEventModifierFlags::empty(), 0.0, window_number, None, 0, 0, 0.0,
    );
    if let Some(event) = event {
        NSApplication::sharedApplication(mtm()).postEvent_atStart(&event, false);
    }
}

/// Would a click at the centre of the view's visible part reach the view?
/// (AppKit's own hit test, so it reflects what a real click would do.)
pub fn click_reaches(id: ViewId) -> Option<bool> {
    with_host(|h| {
        let m = h.views.get(&id)?;
        let v = m.visible?;
        let (x, y) = (v.x + v.width / 2.0, v.y + v.height / 2.0);
        let p = if h.content.isFlipped() { NSPoint::new(x, y) } else { NSPoint::new(x, h.content.bounds().size.height - y) };
        // SAFETY: the content view is in a window, so its superview is valid.
        let p = unsafe { h.content.superview() }.map_or(p, |s| h.content.convertPoint_toView(p, Some(&s)));
        let hit = h.content.hitTest(p)?;
        Some(hit.isDescendantOf(&m.view))
    })
    .flatten()
}

/// Class name of the window's first responder ("host" for the host's view).
pub fn first_responder() -> String {
    with_host(|h| {
        let Some(r) = h.content.window().and_then(|w| w.firstResponder()) else { return "none".into() };
        if std::ptr::eq(&*r as *const _ as *const objc2_app_kit::NSView, &*h.content) {
            return "host".into();
        }
        r.class().name().to_string_lossy().into_owned()
    })
    .unwrap_or_else(|| "detached".into())
}

/// Whether the mounted view's clip container is currently shown.
pub fn is_visible(id: ViewId) -> bool {
    with_host(|h| h.views.get(&id).is_some_and(|m| m.shown()))
        .unwrap_or(false)
}

/// Forces this app's appearance (`None`: follow the system). The system
/// setting is untouched; windows' `effectiveAppearance` changes exactly as on
/// a system switch.
pub fn set_app_appearance(dark: Option<bool>) {
    let app = NSApplication::sharedApplication(mtm());
    let appearance = dark.and_then(|dark| {
        // SAFETY: framework string constants.
        let name = unsafe { if dark { objc2_app_kit::NSAppearanceNameDarkAqua } else { objc2_app_kit::NSAppearanceNameAqua } };
        objc2_app_kit::NSAppearance::appearanceNamed(name)
    });
    app.setAppearance(appearance.as_deref());
}

/// Whether a mounted view resolves to a dark appearance.
pub fn view_is_dark(id: ViewId) -> Option<bool> {
    with_host(|h| {
        let view = &h.views.get(&id)?.view;
        Some(objc2_app_kit::NSAppearanceCustomization::effectiveAppearance(&**view).name().to_string().contains("Dark"))
    })
    .flatten()
}

/// Shown views whose clip frame isn't on the device-pixel grid.
pub fn off_grid() -> Vec<ViewId> {
    with_host(|h| {
        let scale = h.content.window().map_or(1.0, |w| w.backingScaleFactor());
        let on = |v: f64| ((v * scale) - (v * scale).round()).abs() < 1e-6;
        h.views
            .iter()
            .filter(|(_, m)| m.shown())
            .filter(|(_, m)| {
                let f = m.clip_frame();
                !(on(f.origin.x) && on(f.origin.y) && on(f.size.width) && on(f.size.height))
            })
            .map(|(id, _)| *id)
            .collect()
    })
    .unwrap_or_default()
}

/// The visible part of a view as last placed (host coordinates).
pub fn visible_rect(id: ViewId) -> Option<crate::Rect> {
    with_host(|h| h.views.get(&id).and_then(|m| m.visible)).flatten()
}

/// Number of subviews of the host's content view (mounted clip containers).
pub fn subview_count() -> usize {
    with_host(|h| h.content.subviews().len()).unwrap_or(0)
}

/// (window is key, app is active), for diagnosing posted clicks.
pub fn window_state() -> (bool, bool) {
    let key = with_host(|h| h.content.window().is_some_and(|w| w.isKeyWindow())).unwrap_or(false);
    (key, NSApplication::sharedApplication(mtm()).isActive())
}

/// Makes a mounted view first responder directly through AppKit, bypassing
/// [`crate::focus`] (as when AppKit moves focus itself, e.g. on a click).
pub fn make_first_responder_raw(id: ViewId) -> bool {
    with_host(|h| {
        let (Some(m), Some(window)) = (h.views.get(&id), h.content.window()) else { return false };
        window.makeFirstResponder(Some(&crate::host::focus_target(&m.view)))
    })
    .unwrap_or(false)
}

/// Posts a key-down with explicit modifier flags (e.g. Control for Ctrl-Tab).
pub fn post_key_with(chars: &str, key_code: u16, flags: NSEventModifierFlags) {
    let Some(window_number) = with_host(|h| h.content.window().map(|w| w.windowNumber())).flatten() else { return };
    let chars = objc2_foundation::NSString::from_str(chars);
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown, NSPoint::ZERO, flags, 0.0, window_number, None, &chars, &chars, false, key_code,
    );
    if let Some(event) = event {
        NSApplication::sharedApplication(mtm()).postEvent_atStart(&event, false);
    }
}
