//! Any `NSView` as a Ferrokit view: [`native`] wraps a view Ferrokit doesn't
//! have (a map, a 3D scene, a third-party view, the app's own subclass), and
//! it takes part in layout, identity, updates in place, focus and teardown
//! like Ferrokit's own views.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::NSView;

use crate::signal::{Effect, untrack};
use crate::view::{Cx, LeafHandle, Node, Size, View};

type Make<V> = Box<dyn FnOnce(MainThreadMarker) -> Retained<V>>;
type Measure<V> = Rc<dyn Fn(&V) -> Size>;
type Update<V> = Box<dyn FnMut(&V)>;
type Hook<V> = Box<dyn FnOnce(&V)>;

/// An `NSView` of the app's choosing, wrapped as a Ferrokit view (see
/// [`native`]).
#[must_use = "a view does nothing until it's in a window"]
pub struct NativeView<V: 'static> {
    make: Make<V>,
    updates: Vec<Update<V>>,
    measure: Option<Measure<V>>,
    flex: (bool, bool),
    keep: Vec<Rc<dyn Any>>,
    on_remove: Vec<Hook<V>>,
}

/// Wraps an `NSView` Ferrokit doesn't have: `make` builds it, once, when the
/// view is built (on the main thread; `mtm` is the marker AppKit's
/// initialisers take). Any view works: AppKit's, another framework's
/// (`MKMapView`, `SCNView`), or the app's own subclass made with
/// `define_class!`.
///
/// It's a leaf in the layout like Ferrokit's own views: it fills the room
/// it gets (see [`flexible`](NativeView::flexible)), its ideal size is its
/// `fittingSize` or what [`measure`](NativeView::measure) says, and the
/// usual modifiers apply (`.min_height(..)`, `.padding(..)`,
/// `.accessibility_label(..)`, `.hidden(..)`, `.id(..)`…). Change it in
/// place with [`update`](NativeView::update); it's released when it leaves
/// the tree or its window closes.
///
/// ```no_run
/// use ferrokit::*;
/// use objc2_app_kit::NSLevelIndicator;
///
/// let level = Signal::new(3.0);
/// let view = native(|mtm| NSLevelIndicator::new(mtm))
///     .update(move |meter| meter.setDoubleValue(level.get()))
///     .measure(|_| Size { w: 120.0, h: 18.0 })
///     .flexible(false, false)
///     .accessibility_label("Signal strength");
/// # let _ = view;
/// ```
///
/// Focus and Tab order are AppKit's: a view that accepts first responder is
/// in the window's key view loop where its frame puts it. Accessibility is
/// the view's own (AppKit's views have theirs; a custom view adopts
/// `NSAccessibility` or takes `.accessibility_label`).
pub fn native<V>(make: impl FnOnce(MainThreadMarker) -> Retained<V> + 'static) -> NativeView<V>
where
    V: Message + AsRef<NSView> + 'static,
{
    NativeView { make: Box::new(make), updates: Vec::new(), measure: None, flex: (true, true), keep: Vec::new(), on_remove: Vec::new() }
}

impl<V: Message + AsRef<NSView> + 'static> NativeView<V> {
    /// Changes the view in place: `f` runs as an effect, now and again
    /// whenever a signal it read changes. After each run the view is
    /// measured again (its neighbours move if its size changed) and redrawn
    /// (`setNeedsDisplay`). Each `update` is an effect of its own, so
    /// unrelated properties don't re-apply together.
    pub fn update(mut self, f: impl FnMut(&V) + 'static) -> Self {
        self.updates.push(Box::new(f));
        self
    }

    /// The view's ideal size, asked when it's built and after each
    /// [`update`](Self::update). Without it, the view's `fittingSize` (its
    /// Auto Layout size, which is zero for a plain `NSView`).
    pub fn measure(mut self, f: impl Fn(&V) -> Size + 'static) -> Self {
        self.measure = Some(Rc::new(f));
        self
    }

    /// Whether the view stretches across (`horizontal`) and down
    /// (`vertical`) to fill the room it gets. Both by default (a map, a
    /// drawing); `(false, false)` keeps it at its measured size, like a
    /// control.
    pub fn flexible(mut self, horizontal: bool, vertical: bool) -> Self {
        self.flex = (horizontal, vertical);
        self
    }

    /// Keeps `object` alive as long as the view: a delegate, a target, an
    /// observer (AppKit holds delegates weakly).
    pub fn keep<T: Message + 'static>(mut self, object: Retained<T>) -> Self {
        self.keep.push(Rc::new(object));
        self
    }

    /// Runs `f` once the view has left its window (removed from the tree,
    /// or its window closed), after its updates have stopped: stop a timer,
    /// end an observation, clear a delegate.
    pub fn on_remove(mut self, f: impl FnOnce(&V) + 'static) -> Self {
        self.on_remove.push(Box::new(f));
        self
    }
}

/// Runs the `on_remove` hooks when the node lets it go (after its effects
/// stopped and its view left the window).
struct Removal<V: Message + 'static> {
    view: Retained<V>,
    hooks: RefCell<Vec<Hook<V>>>,
}

impl<V: Message + 'static> Drop for Removal<V> {
    fn drop(&mut self) {
        for f in self.hooks.take() {
            crate::failure::guard("a native view's on_remove", (), || f(&self.view));
        }
    }
}

impl<V: Message + AsRef<NSView> + 'static> View for NativeView<V> {
    fn build(self, cx: &mut Cx) -> Node {
        let view = (self.make)(cx.mtm);
        let ns: Retained<NSView> = (*view).as_ref().retain();
        let (mut node, handle) = Node::leaf(cx, ns.clone(), self.flex, Size::default());
        // Measured now, and again after each update: the size the layout uses.
        let measure = self.measure;
        let handle: Rc<LeafHandle> = Rc::new(handle);
        let remeasure: Rc<dyn Fn()> = {
            let (view, handle) = (view.clone(), handle.clone());
            Rc::new(move || {
                handle.content.set(measure.as_ref().map(|m| untrack(|| m(&view))));
                handle.meta.mark();
            })
        };
        remeasure();
        for mut f in self.updates {
            let (v, ns, remeasure) = (view.clone(), ns.clone(), remeasure.clone());
            node.own(Effect::new(move || {
                f(&v);
                ns.setNeedsDisplay(true);
                remeasure();
            }));
        }
        for k in self.keep {
            node.hold(k);
        }
        if !self.on_remove.is_empty() {
            node.hold(Rc::new(Removal { view, hooks: RefCell::new(self.on_remove) }));
        }
        node
    }
}
