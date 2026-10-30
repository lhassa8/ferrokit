//! Popovers: content in a bubble pointing at a view (`NSPopover`), shown
//! while a signal is true. See [`ViewExt::popover`](crate::ViewExt::popover).

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
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

pub(crate) struct PopoverSpec {
    pub(crate) shown: Signal<bool>,
    pub(crate) content: Rc<dyn Fn() -> AnyView>,
    pub(crate) edge: Edge,
}

struct State {
    shown: Signal<bool>,
    /// The content's root while it's shown.
    root: RefCell<Option<Retained<NSView>>>,
}

impl State {
    /// Drops the content (its views, effects and signals) and clears the
    /// signal: the popover closed (Escape, a click outside, or the signal).
    fn closed(&self) {
        if let Some(root) = self.root.borrow_mut().take() {
            crate::drop_hosted(&root);
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
        #[unsafe(method(popoverDidClose:))]
        fn did_close(&self, _notification: &NSNotification) {
            self.ivars().closed();
        }
    }
);

/// The popover of one view: made when first shown, reused after.
struct Popover {
    popover: Retained<NSPopover>,
    delegate: Retained<Delegate>,
}

impl Drop for Popover {
    fn drop(&mut self) {
        // The anchor's node is going: close without animation and free the
        // content now.
        if self.popover.isShown() {
            self.popover.close();
        }
        if let Some(root) = self.delegate.ivars().root.borrow_mut().take() {
            crate::drop_hosted(&root);
        }
        // The signal reads false again, on the next pass: not now, as this
        // can run while the app's state is being dropped at exit (when the
        // next pass never comes). Freed signals ignore the set.
        let shown = self.delegate.ivars().shown;
        crate::later(move || shown.set_if_changed(false));
        // AppKit holds the delegate weakly: detach it before it goes.
        self.popover.setDelegate(None);
    }
}

/// Shows `spec`'s content in a popover on `anchor` while its signal is true.
pub(crate) fn attach(node: &mut Node, anchor: Retained<NSView>, spec: PopoverSpec) {
    let mtm = crate::mtm();
    let popover = NSPopover::new(mtm);
    popover.setBehavior(NSPopoverBehavior::Transient);
    popover.setAnimates(true);
    let delegate = Delegate::alloc(mtm).set_ivars(State { shown: spec.shown, root: RefCell::new(None) });
    // SAFETY: NSObject's designated initialiser.
    let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
    popover.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let state = Rc::new(Popover { popover, delegate });
    let (s, shown, content, edge) = (Rc::downgrade(&state), spec.shown, spec.content, spec.edge);
    node.own(Effect::new(move || {
        let want = shown.get();
        let Some(p) = s.upgrade() else { return };
        untrack(|| {
            if want && !p.popover.isShown() {
                show(&p, &anchor, &*content, edge);
            } else if !want && p.popover.isShown() {
                // SAFETY: a nil sender.
                unsafe { p.popover.performClose(None) };
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
                && !p.popover.isShown()
            {
                // The effect shows it: nudge it by re-setting the signal.
                let shown = p.delegate.ivars().shown;
                shown.set(false);
                shown.set(true);
            }
        });
        return;
    }
    let (root, size) = crate::host_view(content());
    let controller = NSViewController::new(crate::mtm());
    controller.setView(&root);
    p.popover.setContentViewController(Some(&controller));
    p.popover.setContentSize(size);
    *p.delegate.ivars().root.borrow_mut() = Some(root);
    let flipped = anchor.isFlipped();
    let side = match (edge, flipped) {
        (Edge::Below, false) | (Edge::Above, true) => NSRectEdge::MinY,
        (Edge::Below, true) | (Edge::Above, false) => NSRectEdge::MaxY,
        (Edge::Leading, _) => NSRectEdge::MinX,
        (Edge::Trailing, _) => NSRectEdge::MaxX,
    };
    p.popover.showRelativeToRect_ofView_preferredEdge(anchor.bounds(), anchor, side);
}
