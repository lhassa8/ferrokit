//! Popovers: content in a bubble pointing at a view (`NSPopover`), shown
//! while a signal is true. See [`ViewExt::popover`](crate::ViewExt::popover).

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{NSPopover, NSPopoverBehavior, NSPopoverDelegate, NSView, NSViewController};
use objc2_foundation::{NSNotification, NSRectEdge};

use crate::signal::{Effect, Signal, untrack};
use crate::view::{AnyView, Node};

/// Which side of its view a popover appears on (it moves to the other side
/// when there isn't room).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Edge {
    /// Below the view.
    #[default]
    Below,
    /// Above the view.
    Above,
    /// Before the view (left, in left-to-right languages).
    Leading,
    /// After the view.
    Trailing,
}

/// Whether `window` is a popover's (AppKit's private window class).
pub(crate) fn is_popover_window(window: &objc2_app_kit::NSWindow) -> bool {
    use objc2::runtime::NSObjectProtocol;
    window.class().name().to_string_lossy().contains("Popover") && window.respondsToSelector(objc2::sel!(cancelOperation:))
}

pub(crate) struct PopoverSpec {
    pub(crate) shown: Signal<bool>,
    pub(crate) content: Rc<dyn Fn() -> AnyView>,
    pub(crate) edge: Edge,
}

struct State {
    shown: Signal<bool>,
    /// The popover while it's open. A new one each time: AppKit can keep
    /// counting a closed one as shown for a while (in an inactive app),
    /// and then won't show it again.
    current: RefCell<Option<Retained<NSPopover>>>,
    /// The content's root while it's shown.
    root: RefCell<Option<Retained<NSView>>>,
}

impl State {
    /// Drops the content (its views, effects and signals) and clears the
    /// signal: the popover closed (Escape, a click outside, or the signal).
    fn closed(&self) {
        self.closing();
        if let Some(root) = self.root.borrow_mut().take() {
            crate::drop_hosted(&root);
        }
    }

    /// The popover started to close: the signal reads false and a new one
    /// can open. Its content stays until it has animated out.
    fn closing(&self) {
        if let Some(p) = self.current.borrow_mut().take() {
            p.setDelegate(None);
        }
        self.shown.set_if_changed(false);
    }
}

define_class!(
    // Hears the popover close however it closes.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiPopoverDelegate"]
    #[ivars = State]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSPopoverDelegate for Delegate {
        // Will-close, not did-close: an animated close in an app that isn't
        // active may never report that it finished.
        #[unsafe(method(popoverWillClose:))]
        fn will_close(&self, notification: &NSNotification) {
            // Only the open one (a popover closed earlier reports late).
            let ours = self.ivars().current.borrow().as_ref().is_some_and(|p| {
                notification.object().is_some_and(|o| std::ptr::eq(&*o as *const _ as *const u8, &**p as *const _ as *const u8))
            });
            if ours {
                self.ivars().closing();
                // Free the content once it's off screen, unless it has
                // been freed (its view went) or replaced (opened again).
                let Some(root) = self.ivars().root.borrow().clone() else { return };
                let me = self.retain();
                crate::spawn(
                    || std::thread::sleep(std::time::Duration::from_millis(500)),
                    move |()| {
                        let state = me.ivars();
                        let same = state.root.borrow().as_ref().is_some_and(|r| Retained::as_ptr(r) == Retained::as_ptr(&root));
                        if same && state.current.borrow().is_none() {
                            state.root.borrow_mut().take();
                            crate::drop_hosted(&root);
                        }
                    },
                );
            }
        }
    }
);

/// The popover of one view.
struct Popover {
    delegate: Retained<Delegate>,
}

impl Drop for Popover {
    fn drop(&mut self) {
        // The anchor's node is going: close without animation and free the
        // content now.
        let state = self.delegate.ivars();
        if let Some(p) = state.current.borrow_mut().take() {
            p.setDelegate(None);
            p.close();
        }
        if let Some(root) = state.root.borrow_mut().take() {
            crate::drop_hosted(&root);
        }
        // The signal reads false again, on the next pass: not now, as this
        // can run while the app's state is being dropped at exit (when the
        // next pass never comes). Freed signals ignore the set.
        let shown = state.shown;
        crate::later(move || shown.set_if_changed(false));
    }
}

/// Shows `spec`'s content in a popover on `anchor` while its signal is true.
pub(crate) fn attach(node: &mut Node, anchor: Retained<NSView>, spec: PopoverSpec) {
    let mtm = crate::mtm();
    let delegate = Delegate::alloc(mtm).set_ivars(State { shown: spec.shown, current: RefCell::new(None), root: RefCell::new(None) });
    // SAFETY: NSObject's designated initialiser.
    let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
    let state = Rc::new(Popover { delegate });
    let (s, shown, content, edge) = (Rc::downgrade(&state), spec.shown, spec.content, spec.edge);
    node.own(Effect::new(move || {
        let want = shown.get();
        let Some(p) = s.upgrade() else { return };
        untrack(|| {
            let open = p.delegate.ivars().current.borrow().clone();
            match (want, open) {
                (true, None) => show(&p, &anchor, &*content, edge),
                // Closed by the app: at once, and cleaned up here (an
                // animated close may never report back).
                (false, Some(popover)) => {
                    p.delegate.ivars().closed();
                    popover.close();
                }
                _ => {}
            }
        });
    }));
    node.hold(state);
}

fn show(p: &Rc<Popover>, anchor: &Retained<NSView>, content: &dyn Fn() -> AnyView, edge: Edge) {
    if anchor.window().is_none() {
        // Not on screen yet (shown while its window is being built): try
        // once the window is up.
        let (p, anchor) = (Rc::downgrade(p), anchor.clone());
        crate::later(move || {
            if let Some(p) = p.upgrade()
                && anchor.window().is_some()
                && p.delegate.ivars().shown.try_get() == Some(true)
                && p.delegate.ivars().current.borrow().is_none()
            {
                // The effect shows it: nudge it by re-setting the signal.
                let shown = p.delegate.ivars().shown;
                shown.set(false);
                shown.set(true);
            }
        });
        return;
    }
    // The last opening's content, if it's still animating out.
    if let Some(old) = p.delegate.ivars().root.borrow_mut().take() {
        crate::drop_hosted(&old);
    }
    let (root, size) = crate::host_view(content());
    let mtm = crate::mtm();
    let popover = NSPopover::new(mtm);
    popover.setBehavior(NSPopoverBehavior::Transient);
    popover.setAnimates(true);
    popover.setDelegate(Some(ProtocolObject::from_ref(&*p.delegate)));
    let controller = NSViewController::new(mtm);
    controller.setView(&root);
    popover.setContentViewController(Some(&controller));
    popover.setContentSize(size);
    *p.delegate.ivars().root.borrow_mut() = Some(root);
    *p.delegate.ivars().current.borrow_mut() = Some(popover.clone());
    let flipped = anchor.isFlipped();
    let side = match (edge, flipped) {
        (Edge::Below, false) | (Edge::Above, true) => NSRectEdge::MinY,
        (Edge::Below, true) | (Edge::Above, false) => NSRectEdge::MaxY,
        (Edge::Leading, _) => NSRectEdge::MinX,
        (Edge::Trailing, _) => NSRectEdge::MaxX,
    };
    popover.showRelativeToRect_ofView_preferredEdge(anchor.bounds(), anchor, side);
}
