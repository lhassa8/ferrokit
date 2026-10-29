//! Undo: every window has one `NSUndoManager`, shared by its text fields,
//! app-level changes ([`undoable`]) and editors that opt in
//! ([`TextEditor::shared_undo`](crate::TextEditor::shared_undo)). ⌘Z and
//! ⇧⌘Z reach it through the responder chain; the Edit menu shows the
//! action's name ("Undo Rename").

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSWindow, NSWindowDelegate};
use objc2_foundation::{NSNotification, NSString, NSUndoManager};

use crate::signal::{Signal, batch};

define_class!(
    // Each window's delegate: gives it the undo manager appkit-ui made for
    // it, and tears the window down when it closes.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiWindowDelegate"]
    #[ivars = Retained<NSUndoManager>]
    pub(crate) struct WindowDelegate;

    unsafe impl NSObjectProtocol for WindowDelegate {}

    unsafe impl NSWindowDelegate for WindowDelegate {
        #[unsafe(method_id(windowWillReturnUndoManager:))]
        fn undo_manager(&self, _window: &NSWindow) -> Option<Retained<NSUndoManager>> {
            Some(self.ivars().clone())
        }

        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, notification: &NSNotification) {
            if let Some(window) = notification.object().and_then(|o| o.downcast::<NSWindow>().ok()) {
                crate::window_will_close(&window);
            }
        }
    }
);

thread_local! {
    /// The undo manager of the window whose views are being built (its
    /// content isn't in the window yet).
    static BUILDING: RefCell<Option<Retained<NSUndoManager>>> = const { RefCell::new(None) };
}

/// Gives `window` its own undo manager (kept by the returned delegate) and
/// returns both.
pub(crate) fn install(mtm: MainThreadMarker, window: &NSWindow) -> (Retained<NSObject>, Retained<NSUndoManager>) {
    let manager = NSUndoManager::new(mtm);
    let delegate = WindowDelegate::alloc(mtm).set_ivars(manager.clone());
    // SAFETY: NSObject's designated initialiser.
    let delegate: Retained<WindowDelegate> = unsafe { msg_send![super(delegate), init] };
    // Weak in AppKit; the caller keeps the delegate.
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    (Retained::into_super(delegate), manager)
}

/// Runs `f` (building a window's views) with `manager` as the window's.
pub(crate) fn building<R>(manager: &Retained<NSUndoManager>, f: impl FnOnce() -> R) -> R {
    let previous = BUILDING.with_borrow_mut(|b| b.replace(manager.clone()));
    let r = f();
    BUILDING.with_borrow_mut(|b| *b = previous);
    r
}

/// The undo manager of the window `view` is in, or is being built for.
pub(crate) fn manager_for(view: &objc2_app_kit::NSView) -> Option<Retained<NSUndoManager>> {
    view.window().and_then(|w| w.undoManager()).or_else(|| BUILDING.with_borrow(Clone::clone))
}

struct Step {
    title: String,
    /// Undoes the change.
    back: Rc<dyn Fn()>,
    /// Does it again.
    forth: Rc<dyn Fn()>,
    /// Where it's recorded (weak: the manager keeps the step).
    manager: Weak<NSUndoManager>,
}

define_class!(
    // One recorded change; registered as both target and argument, so the
    // undo manager keeps it alive.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiUndoStep"]
    #[ivars = Step]
    struct UndoStep;

    impl UndoStep {
        #[unsafe(method(apply:))]
        fn apply(&self, _arg: Option<&AnyObject>) {
            let step = self.ivars();
            batch(|| (step.back)());
            // Registered while undoing, this lands on the redo stack (and
            // vice versa).
            if let Some(manager) = step.manager.load() {
                register(&manager, &step.title, step.forth.clone(), step.back.clone());
            }
        }
    }
);

fn register(manager: &Retained<NSUndoManager>, title: &str, back: Rc<dyn Fn()>, forth: Rc<dyn Fn()>) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let step = UndoStep::alloc(mtm).set_ivars(Step { title: title.into(), back, forth, manager: Weak::from_retained(manager) });
    // SAFETY: NSObject's designated initialiser.
    let step: Retained<UndoStep> = unsafe { msg_send![super(step), init] };
    // SAFETY: the step responds to apply:; passing it as the argument makes
    // the undo manager retain it (targets aren't retained).
    unsafe { manager.registerUndoWithTarget_selector_object(&step, sel!(apply:), Some(&step)) };
    manager.setActionName(&NSString::from_str(title));
}

/// The window app-level changes go to: the key window, else the main
/// window (none while the app is inactive), else the last-opened visible one.
fn target_window() -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(MainThreadMarker::new().expect("main thread"));
    app.keyWindow()
        .or_else(|| app.mainWindow())
        .or_else(|| crate::windows().into_iter().rev().find(|w| w.isVisible()))
}

/// Makes a change the user can undo: runs `apply` now (in a [`batch`]) and
/// records `revert` in the frontmost window's undo history under `title`
/// ("Rename", "Delete Row"…). ⌘Z runs `revert`, ⇧⌘Z runs `apply` again.
pub fn undoable(title: &str, apply: impl Fn() + 'static, revert: impl Fn() + 'static) {
    match target_window() {
        Some(w) => undoable_in(&w, title, apply, revert),
        None => batch(apply),
    }
}

/// As [`undoable`], in `window`'s undo history.
pub fn undoable_in(window: &NSWindow, title: &str, apply: impl Fn() + 'static, revert: impl Fn() + 'static) {
    batch(&apply);
    if let Some(manager) = window.undoManager() {
        register(&manager, title, Rc::new(revert), Rc::new(apply));
    }
}

/// Sets `signal` to `value` as an undoable change named `title` (see
/// [`undoable`]). Nothing is recorded if the value is unchanged.
pub fn set_undoable<T: Clone + PartialEq + 'static>(title: &str, signal: &Signal<T>, value: T) {
    let old = signal.get_untracked();
    if old == value {
        return;
    }
    let (s1, s2) = (signal.clone(), signal.clone());
    undoable(title, move || s1.set(value.clone()), move || s2.set(old.clone()));
}
