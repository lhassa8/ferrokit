//! Keyboard focus and shortcuts across the boundary (risk 4).
//!
//! One local `NSEvent` monitor sees every key-down and mouse-down first:
//! - Tab / Shift-Tab become [`Event::FocusStep`]: the host owns the focus order
//!   across both worlds. A view registered with [`set_accepts_tab`] keeps plain
//!   Tab (a text editor inserts it); Ctrl-Tab still moves on.
//! - ⌘A/C/V/X/Z/⇧⌘Z go to the focused native view's responder chain. With the
//!   host focused and the Edit menu installed, they go straight to the host's
//!   view, so the menu's key equivalents don't swallow them.
//! - A click outside every native view hands focus back to the host view.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSControl, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem, NSView,
};
use objc2_core_foundation::{CFRunLoop, kCFRunLoopCommonModes};
use objc2_foundation::{NSPoint, NSString};

use crate::events::{Event, emit};
use crate::host::{focus_target, mtm, with_host};
use crate::ViewId;

const TAB: u16 = 48;

thread_local! {
    static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    static ACCEPTS_TAB: RefCell<HashSet<ViewId>> = RefCell::new(HashSet::new());
    static LAST_FOCUS: Cell<Option<ViewId>> = const { Cell::new(None) };
    static CHECK_QUEUED: Cell<bool> = const { Cell::new(false) };
    static EDIT_MENU: Cell<bool> = const { Cell::new(false) };
    static TARGET: RefCell<Option<Retained<Target>>> = const { RefCell::new(None) };
}

/// Lets a view keep plain Tab (e.g. a text editor). Ctrl-Tab leaves it.
pub fn set_accepts_tab(id: ViewId, accepts: bool) {
    ACCEPTS_TAB.with_borrow_mut(|s| if accepts { s.insert(id) } else { s.remove(&id) });
}

/// Makes a mounted view the first responder.
pub fn focus(id: ViewId) -> bool {
    let done = with_host(|h| {
        let (Some(m), Some(window)) = (h.views.get(&id), h.content.window()) else { return false };
        window.makeFirstResponder(Some(&focus_target(&m.view)))
    })
    .unwrap_or(false);
    queue_focus_check();
    done
}

/// Gives keyboard focus back to the host's view.
pub fn focus_host() -> bool {
    let done = with_host(|h| h.content.window().is_some_and(|w| w.makeFirstResponder(Some(&h.content)))).unwrap_or(false);
    queue_focus_check();
    done
}

/// The mounted view containing the first responder, if any.
pub fn focused_view() -> Option<ViewId> {
    with_host(|h| {
        let responder = h.content.window()?.firstResponder()?;
        let view = responder.downcast::<NSView>().ok()?;
        h.views.iter().find(|(_, m)| view.isDescendantOf(&m.view)).map(|(id, _)| *id)
    })
    .flatten()
}

/// Emits `FocusChanged` once the current event has been handled, if focus moved.
fn queue_focus_check() {
    if CHECK_QUEUED.with(|q| q.replace(true)) {
        return;
    }
    let block = RcBlock::new(|| {
        CHECK_QUEUED.with(|q| q.set(false));
        let now = focused_view();
        if LAST_FOCUS.with(|l| l.replace(now)) != now {
            emit(Event::FocusChanged(now));
        }
    });
    let main = CFRunLoop::main().expect("main run loop");
    // SAFETY: runs on the main run loop; touches main-thread state only.
    unsafe { main.perform_block(kCFRunLoopCommonModes.map(|m| m.as_ref()), Some(&block)) };
    main.wake_up();
}

pub(crate) fn install() {
    if MONITOR.with_borrow(Option::is_some) {
        return;
    }
    let block = RcBlock::new(|event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit passes a valid event for the duration of the call.
        let e = unsafe { event.as_ref() };
        let keep = match e.r#type() {
            NSEventType::KeyDown => on_key(e),
            NSEventType::LeftMouseDown | NSEventType::RightMouseDown => {
                on_mouse_down(e);
                true
            }
            _ => true,
        };
        if keep { event.as_ptr() } else { std::ptr::null_mut() }
    });
    // SAFETY: the handler returns the event or nil, as the API requires.
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::KeyDown | NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown,
            &block,
        )
    };
    MONITOR.with_borrow_mut(|m| *m = monitor);
}

/// Returns false to consume the event.
fn on_key(e: &NSEvent) -> bool {
    let flags = e.modifierFlags();
    let focused = focused_view();
    if e.keyCode() == TAB && !flags.contains(NSEventModifierFlags::Command) {
        let ctrl = flags.contains(NSEventModifierFlags::Control);
        if !ctrl && focused.is_some_and(|id| ACCEPTS_TAB.with_borrow(|s| s.contains(&id))) {
            return true;
        }
        emit(Event::FocusStep { forward: !flags.contains(NSEventModifierFlags::Shift) });
        return false;
    }
    let command = flags.contains(NSEventModifierFlags::Command);
    let Some(action) = command.then(|| edit_action(e.keyCode(), flags.contains(NSEventModifierFlags::Shift))).flatten() else {
        return true;
    };
    if focused.is_some() {
        return !perform_on_first_responder(action);
    }
    if EDIT_MENU.with(Cell::get) {
        // Host focused: deliver to the host's view before the menu's key
        // equivalent can take it.
        with_host(|h| h.content.keyDown(e));
        return false;
    }
    true
}

fn on_mouse_down(e: &NSEvent) {
    // A click outside every native view gives focus back to the host's view
    // (AppKit leaves the old first responder in place otherwise).
    let outside = with_host(|h| {
        let Some(window) = h.content.window() else { return false };
        if e.window(mtm()).is_none_or(|w| w != window) {
            return false;
        }
        // hitTest: takes a point in the receiver's superview coordinates.
        let p = e.locationInWindow();
        // SAFETY: the content view is in a window, so its superview is valid.
        let p = unsafe { h.content.superview() }.map_or(p, |s| s.convertPoint_fromView(p, None));
        let hit = h.content.hitTest(p);
        !h.views.values().any(|m| hit.as_ref().is_some_and(|v| v.isDescendantOf(&m.view)))
    })
    .unwrap_or(false);
    if outside && focused_view().is_some() {
        focus_host();
    }
    queue_focus_check();
}

fn edit_action(key_code: u16, shift: bool) -> Option<Sel> {
    Some(match (key_code, shift) {
        (0, false) => sel!(selectAll:),
        (7, false) => sel!(cut:),
        (8, false) => sel!(copy:),
        (9, false) => sel!(paste:),
        (6, false) => sel!(undo:),
        (6, true) => sel!(redo:),
        _ => return None,
    })
}

fn key_for(action: Sel) -> Option<(&'static str, u16, bool)> {
    [
        (sel!(selectAll:), ("a", 0, false)),
        (sel!(cut:), ("x", 7, false)),
        (sel!(copy:), ("c", 8, false)),
        (sel!(paste:), ("v", 9, false)),
        (sel!(undo:), ("z", 6, false)),
        (sel!(redo:), ("z", 6, true)),
    ]
    .into_iter()
    .find(|(s, _)| *s == action)
    .map(|(_, k)| k)
}

fn perform_on_first_responder(action: Sel) -> bool {
    with_host(|h| h.content.window()?.firstResponder())
        .flatten()
        // SAFETY: a standard responder action with a nil sender.
        .is_some_and(|r| unsafe { r.tryToPerform_with(action, None) })
}

define_class!(
    // Target for the Edit menu and for native controls' actions.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitHostTarget"]
    struct Target;

    impl Target {
        #[unsafe(method(editAction:))]
        fn edit_action(&self, sender: &NSMenuItem) {
            let Some(action) = EDIT_ACTIONS.get(sender.tag() as usize).copied() else { return };
            if focused_view().is_some() {
                perform_on_first_responder(action());
            } else if let Some((chars, code, shift)) = key_for(action()) {
                // Chosen with the mouse while the host has focus: send the
                // host the shortcut it would have received from the keyboard.
                post_key_down(chars, code, true, shift);
            }
        }

        #[unsafe(method(controlAction:))]
        fn control_action(&self, sender: &AnyObject) {
            // SAFETY: senders are NSControls and NSMenuItems, which both have -tag.
            let tag: isize = unsafe { msg_send![sender, tag] };
            emit(Event::Action(ViewId(tag as u64)));
        }

        #[unsafe(method(validateMenuItem:))]
        fn validate(&self, _item: &NSMenuItem) -> bool {
            true
        }
    }
);

const EDIT_ACTIONS: [fn() -> Sel; 6] =
    [|| sel!(undo:), || sel!(redo:), || sel!(cut:), || sel!(copy:), || sel!(paste:), || sel!(selectAll:)];

fn target() -> Retained<Target> {
    TARGET.with_borrow_mut(|t| {
        t.get_or_insert_with(|| {
            let this = Target::alloc(mtm()).set_ivars(());
            unsafe { msg_send![super(this), init] }
        })
        .clone()
    })
}

/// Sends `control`'s action to the event sink as `Event::Action(id)`.
pub fn set_action(control: &NSControl, id: ViewId) {
    let target = target();
    control.setTag(id.0 as isize);
    // SAFETY: the target lives for the rest of the program (kept in TARGET).
    unsafe {
        control.setTarget(Some(&target));
        control.setAction(Some(sel!(controlAction:)));
    }
}

/// Sends a menu item's action to the event sink as `Event::Action(id)`.
pub fn set_menu_action(item: &NSMenuItem, id: ViewId) {
    let target = target();
    item.setTag(id.0 as isize);
    // SAFETY: the target lives for the rest of the program (kept in TARGET).
    unsafe {
        item.setTarget(Some(&target));
        item.setAction(Some(sel!(controlAction:)));
    }
}

/// Adds a standard Edit menu (Undo, Redo, Cut, Copy, Paste, Select All) with
/// the usual shortcuts, routed to whichever side has focus.
pub fn install_edit_menu() {
    if EDIT_MENU.with(|e| e.replace(true)) {
        return;
    }
    let mtm = mtm();
    let app = NSApplication::sharedApplication(mtm);
    let Some(main_menu) = app.mainMenu() else { return };
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    let target = target();
    for (tag, (title, key)) in
        [("Undo", "z"), ("Redo", "Z"), ("Cut", "x"), ("Copy", "c"), ("Paste", "v"), ("Select All", "a")].into_iter().enumerate()
    {
        // SAFETY: plain menu item construction; target kept alive in TARGET.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(sel!(editAction:)),
                &NSString::from_str(key),
            )
        };
        item.setTag(tag as isize);
        unsafe { item.setTarget(Some(&target)) };
        menu.addItem(&item);
        if tag == 1 {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
    }
    let top = NSMenuItem::new(mtm);
    top.setSubmenu(Some(&menu));
    main_menu.insertItem_atIndex(&top, 1.min(main_menu.numberOfItems()));
}

pub(crate) fn post_key_down(chars: &str, key_code: u16, command: bool, shift: bool) {
    let mtm = mtm();
    let Some(window_number) = with_host(|h| h.content.window().map(|w| w.windowNumber())).flatten() else { return };
    let mut flags = NSEventModifierFlags::empty();
    if command {
        flags |= NSEventModifierFlags::Command;
    }
    if shift {
        flags |= NSEventModifierFlags::Shift;
    }
    let chars = NSString::from_str(chars);
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown, NSPoint::ZERO, flags, 0.0, window_number, None, &chars, &chars, false, key_code,
    );
    if let Some(event) = event {
        NSApplication::sharedApplication(mtm).postEvent_atStart(&event, false);
    }
}
