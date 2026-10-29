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

// ---- drag and drop, without a mouse ----

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSDragOperation, NSDraggingInfo, NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypeString,
    NSPasteboardWriting, NSTableView, NSTableViewDropOperation,
};
use objc2_foundation::{NSArray, NSInteger, NSString, NSURL};

struct DragIvars {
    pasteboard: Retained<NSPasteboard>,
    source: Option<Retained<AnyObject>>,
}

define_class!(
    // The parts of a drag a table's data source reads: its pasteboard and
    // its source.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitHostTestDrag"]
    #[ivars = DragIvars]
    struct TestDrag;

    unsafe impl NSObjectProtocol for TestDrag {}

    unsafe impl NSDraggingInfo for TestDrag {
        #[unsafe(method_id(draggingPasteboard))]
        fn dragging_pasteboard(&self) -> Retained<NSPasteboard> {
            self.ivars().pasteboard.clone()
        }

        #[unsafe(method_id(draggingSource))]
        fn dragging_source(&self) -> Option<Retained<AnyObject>> {
            self.ivars().source.clone()
        }
    }
);

/// A fresh private pasteboard (never the general one); release it with
/// [`release_pasteboard`].
pub fn private_pasteboard() -> Retained<NSPasteboard> {
    NSPasteboard::pasteboardWithUniqueName()
}

/// Releases a pasteboard made by [`private_pasteboard`].
pub fn release_pasteboard(pasteboard: &NSPasteboard) {
    // SAFETY: frees a uniquely named pasteboard's resources.
    let () = unsafe { msg_send![pasteboard, releaseGlobally] };
}

/// Drops whatever is on `pasteboard` onto `table` above view row `row`, as
/// AppKit does at the end of a drag: the data source validates the drop,
/// then accepts it. `from_table` makes the table itself the drag's source.
/// Returns whether the drop was accepted.
pub fn drop_on_table(table: &NSTableView, pasteboard: &NSPasteboard, row: usize, from_table: bool) -> bool {
    let source: Option<Retained<AnyObject>> = from_table.then(|| {
        let any: &AnyObject = table.as_ref();
        any.retain()
    });
    let drag = TestDrag::alloc(mtm()).set_ivars(DragIvars { pasteboard: pasteboard.retain(), source });
    // SAFETY: NSObject's designated initialiser.
    let drag: Retained<TestDrag> = unsafe { msg_send![super(drag), init] };
    let info: &ProtocolObject<dyn NSDraggingInfo> = ProtocolObject::from_ref(&*drag);
    let Some(ds) = (unsafe { table.dataSource() }) else { return false };
    let row = row as NSInteger;
    // SAFETY: the table's own data-source methods, with a drag stand-in that
    // answers what they read.
    unsafe {
        let op: NSDragOperation = msg_send![&*ds, tableView: table, validateDrop: info, proposedRow: row, proposedDropOperation: NSTableViewDropOperation::Above];
        if op == NSDragOperation::None {
            return false;
        }
        msg_send![&*ds, tableView: table, acceptDrop: info, row: row, dropOperation: NSTableViewDropOperation::Above]
    }
}

/// Starts a drag of `table`'s view rows `rows` the way AppKit does (asking
/// the data source for each row's pasteboard writer) and puts them on a
/// private pasteboard. `None` if the table doesn't let rows be dragged.
pub fn drag_rows(table: &NSTableView, rows: &[usize]) -> Option<Retained<NSPasteboard>> {
    let ds = unsafe { table.dataSource() }?;
    let mut writers: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = Vec::new();
    for &r in rows {
        // SAFETY: the data source's tableView:pasteboardWriterForRow:.
        let w: Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
            unsafe { msg_send![&*ds, tableView: table, pasteboardWriterForRow: r as NSInteger] };
        writers.push(w?);
    }
    let pasteboard = private_pasteboard();
    pasteboard.clearContents();
    pasteboard.writeObjects(&NSArray::from_retained_slice(&writers));
    Some(pasteboard)
}

/// A private pasteboard holding file URLs, as a Finder drag carries them.
pub fn files_pasteboard(paths: &[&std::path::Path]) -> Retained<NSPasteboard> {
    let pasteboard = private_pasteboard();
    pasteboard.clearContents();
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = paths
        .iter()
        .map(|p| {
            let item = NSPasteboardItem::new();
            let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
            if let Some(s) = url.absoluteString() {
                // SAFETY: AppKit's pasteboard type constant.
                item.setString_forType(&s, unsafe { NSPasteboardTypeFileURL });
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    pasteboard.writeObjects(&NSArray::from_retained_slice(&items));
    pasteboard
}

/// The plain text on `pasteboard`.
pub fn pasteboard_text(pasteboard: &NSPasteboard) -> Option<String> {
    // SAFETY: AppKit's pasteboard type constant.
    pasteboard.stringForType(unsafe { NSPasteboardTypeString }).map(|s| s.to_string())
}

/// Puts `text` on `pasteboard`.
pub fn set_pasteboard_text(pasteboard: &NSPasteboard, text: &str) {
    pasteboard.clearContents();
    // SAFETY: AppKit's pasteboard type constant.
    pasteboard.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
}
