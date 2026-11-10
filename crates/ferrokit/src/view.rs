//! Views and the node tree.
//!
//! A [`View`] is a plain value describing UI. Building it creates the native
//! views once, plus the [`Effect`]s that keep them up to date; the result is a
//! [`Node`]. Bodies never re-run, so a node's place in the tree is its
//! identity. The parts that do change shape — [`dynamic`] and [`for_each`] —
//! keep existing nodes where they can: by `.id(..)` and by item key.
//!
//! Dropping a node is the teardown: its native views leave the window and its
//! effects stop.

use ferrokit_core::kept::{UiCell, UiLocal};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::{Rc, Weak};

use crate::platform::{MainThread, Native, NativeObject, NativeScroll, NativeSplit, NativeView, Platform};
use crate::signal::{Effect, Owner, Signal, untrack};

/// Something that can be shown. Build-once: [`View::build`] creates native
/// views and effects; it is never called again for the same place.
pub trait View: 'static {
    /// Creates the node (native views are added to `cx`'s container).
    fn build(self, cx: &mut Cx) -> Node;

    /// Gives the view an identity: a [`dynamic`] region keeps the existing
    /// node instead of rebuilding when the new view has the same id.
    fn id(self, id: impl Hash) -> AnyView
    where
        Self: Sized,
    {
        let mut h = DefaultHasher::new();
        id.hash(&mut h);
        AnyView { id: Some(h.finish()), build: Box::new(move |cx| self.build(cx)) }
    }
}

/// A type-erased view (any [`View`] converts into one).
pub struct AnyView {
    id: Option<u64>,
    build: Box<dyn FnOnce(&mut Cx) -> Node>,
}

impl AnyView {
    /// Erases `view`.
    pub fn new(view: impl View) -> Self {
        AnyView { id: None, build: Box::new(move |cx| view.build(cx)) }
    }

    pub(crate) fn build_node(self, cx: &mut Cx) -> Node {
        let mut node = (self.build)(cx);
        node.id = self.id;
        node
    }
}

impl View for AnyView {
    fn build(self, cx: &mut Cx) -> Node {
        self.build_node(cx)
    }
}

/// Where a view is being built: the native view its leaves are added to.
pub struct Cx {
    pub(crate) container: NativeView,
    pub(crate) mtm: MainThread,
}

impl Cx {
    pub(crate) fn new(container: NativeView) -> Self {
        Cx { container, mtm: MainThread::new().expect("main thread") }
    }

    pub(crate) fn child(&self) -> Cx {
        Cx { container: self.container.clone(), mtm: self.mtm }
    }
}

thread_local! {
    pub(crate) static LAYOUT_DIRTY: Cell<bool> = const { Cell::new(false) };
    static LIVE_NODES: Cell<usize> = const { Cell::new(0) };
}

/// Marks the window layout as stale; it runs after the current batch.
pub(crate) fn invalidate_layout() {
    LAYOUT_DIRTY.with(|d| d.set(true));
}

/// Per-node layout state, shared with the effects that change the node.
#[derive(Default)]
pub(crate) struct Meta {
    parent: RefCell<Weak<Meta>>,
    /// Cached ideal size (padding and fixed sizes included).
    pub(crate) ideal: Cell<Option<Size>>,
    /// The rectangle the node was last placed in.
    pub(crate) rect: Cell<Option<[f64; 4]>>,
    /// The width it was mirrored within when last placed, right to left
    /// (0 left to right).
    pub(crate) mirror: Cell<f64>,
    /// Something in this subtree changed since the last layout.
    pub(crate) dirty: Cell<bool>,
    /// Commands this node handles and values it publishes to the focus.
    pub(crate) responds: RefCell<crate::commands::Responds>,
    /// Hidden by [`ViewExt::hidden`]: left out of layout, its views hidden.
    pub(crate) hidden: Cell<bool>,
    /// This node's own views are hidden because it, or a view above it,
    /// is: shown again when none is. (A view the app hid itself stays so.)
    hides: Cell<bool>,
    /// Disabled by [`ViewExt::disabled`], or by the control itself: its
    /// controls, and those of the views inside it, are disabled.
    pub(crate) disabled: Cell<bool>,
}

impl Meta {
    pub(crate) fn parent(&self) -> Option<Rc<Meta>> {
        self.parent.borrow().upgrade()
    }

    /// Whether this node or a view above it is hidden.
    pub(crate) fn hidden_here(self: &Rc<Self>) -> bool {
        self.any_up(|m| m.hidden.get())
    }

    /// Whether this node or a view above it is disabled.
    pub(crate) fn disabled_here(self: &Rc<Self>) -> bool {
        self.any_up(|m| m.disabled.get())
    }

    fn any_up(self: &Rc<Self>, f: impl Fn(&Meta) -> bool) -> bool {
        let mut up = Some(self.clone());
        while let Some(m) = up {
            if f(&m) {
                return true;
            }
            up = m.parent();
        }
        false
    }

    pub(crate) fn set_parent(&self, parent: &Rc<Meta>) {
        *self.parent.borrow_mut() = Rc::downgrade(parent);
    }

    /// Marks this node and its ancestors for layout (their cached ideal
    /// sizes are dropped); the layout runs after the current batch.
    pub(crate) fn mark(self: &Rc<Self>) {
        let mut cur = Some(self.clone());
        while let Some(m) = cur {
            m.ideal.set(None);
            if m.dirty.replace(true) {
                break; // ancestors are already marked
            }
            cur = m.parent.borrow().upgrade();
        }
        invalidate_layout();
    }
}

/// Alignment across a stack's axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Leading (vertical stacks) or top (horizontal stacks).
    Start,
    /// Centred.
    Center,
    /// Trailing or bottom.
    End,
}

/// Number of live nodes (diagnostic, for teardown checks).
pub fn live_nodes() -> usize {
    LIVE_NODES.with(Cell::get)
}

/// Stack direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Children left to right.
    Horizontal,
    /// Children top to bottom.
    Vertical,
}

/// A size in points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

pub(crate) struct Leaf {
    pub(crate) view: NativeView,
    pub(crate) flex: (bool, bool),
    pub(crate) min: Size,
    /// Cached `fittingSize`.
    pub(crate) content: Rc<Cell<Option<Size>>>,
    /// Flexible only across its parent stack's axis (dividers).
    pub(crate) cross_only: bool,
}

/// What an effect needs to report that a leaf's content changed.
pub(crate) struct LeafHandle {
    pub(crate) content: Rc<Cell<Option<Size>>>,
    pub(crate) meta: Rc<Meta>,
}

pub(crate) enum Kind {
    Leaf(Leaf),
    Stack { axis: Axis, spacing: f64, align: Option<Align>, children: Vec<Node> },
    Grid { spacing: (f64, f64), rows: Vec<Vec<Node>>, align: Vec<Align> },
    Spacer,
    /// Children on top of one another, each aligned in the same rectangle.
    Layers { align: (Align, Align), children: Vec<Node> },
    /// Children that change over time; laid out inline in the parent stack.
    Group(Rc<RefCell<Vec<Node>>>),
    /// Content inside a scroll container's (flipped) document view.
    Scroll { scroll: NativeScroll, doc: NativeView, content: Box<Node> },
    /// Panes of a split container, each laid out in its pane's bounds.
    Split {
        split: NativeSplit,
        panes: Vec<(NativeView, Node)>,
        /// The first divider's position to set, and whether it was given by
        /// the app (counted from below a title bar it's under) or restored.
        position: Cell<Option<(f64, bool)>>,
        /// The last pane's initial size, applied at the first layout.
        trailing: Cell<Option<f64>>,
        /// Panes top to bottom (dividers across) instead of side by side.
        stacked: bool,
    },
}

/// What [`Node::each_leaf`] calls: a leaf's view, its measured size and its
/// node's metadata.
pub(crate) type LeafFn<'a> = dyn FnMut(&NativeView, &Cell<Option<Size>>, &Rc<Meta>) + 'a;

/// A built view: native views, effects and helper objects it owns.
pub struct Node {
    pub(crate) kind: Kind,
    pub(crate) id: Option<u64>,
    /// Inset (horizontal, vertical) on each side.
    pub(crate) padding: (f64, f64),
    pub(crate) fixed: (Option<f64>, Option<f64>),
    /// At least this wide and tall (`min_width` / `min_height`).
    pub(crate) min: (Option<f64>, Option<f64>),
    /// Can be narrower than its ideal width when its row is short of room
    /// (truncating text).
    pub(crate) shrinks: bool,
    /// Forced flexibility (`fill_width` / `fill_height`).
    pub(crate) fill: (bool, bool),
    pub(crate) meta: Rc<Meta>,
    /// Views behind the node's own (a surface), framed to its rectangle.
    pub(crate) decor: Vec<NativeView>,
    /// How it leaves when removed inside an animated change.
    pub(crate) transition: Option<crate::Transition>,
    /// Its matched-geometry id (see [`ViewExt::matched_geometry`]).
    pub(crate) matched: Option<Rc<str>>,
    effects: Vec<Effect>,
    keep: Vec<NativeObject>,
    held: Vec<Rc<dyn std::any::Any>>,
    /// Frees the signals made while building this subtree (dropped last).
    owner: Option<Owner>,
}

impl Node {
    pub(crate) fn new(kind: Kind) -> Node {
        LIVE_NODES.with(|n| n.set(n.get() + 1));
        let meta = Rc::new(Meta::default());
        meta.dirty.set(true);
        let node = Node {
            kind,
            id: None,
            padding: (0.0, 0.0),
            fixed: (None, None),
            min: (None, None),
            shrinks: false,
            fill: (false, false),
            meta,
            decor: Vec::new(),
            transition: None,
            matched: None,
            effects: Vec::new(),
            keep: Vec::new(),
            held: Vec::new(),
            owner: None,
        };
        for c in node.children_static() {
            c.meta.set_parent(&node.meta);
        }
        node
    }

    /// Children owned directly (not through a group).
    fn children_static(&self) -> Vec<&Node> {
        match &self.kind {
            Kind::Stack { children, .. } | Kind::Layers { children, .. } => children.iter().collect(),
            Kind::Grid { rows, .. } => rows.iter().flatten().collect(),
            Kind::Scroll { content, .. } => vec![&**content],
            Kind::Split { panes, .. } => panes.iter().map(|(_, n)| n).collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn leaf(cx: &Cx, view: NativeView, flex: (bool, bool), min: Size) -> (Node, LeafHandle) {
        Native::add_subview(&cx.container, &view);
        let content = Rc::new(Cell::new(None));
        invalidate_layout();
        let key = Native::view_key(&view);
        let node = Node::new(Kind::Leaf(Leaf { view, flex, min, content: content.clone(), cross_only: false }));
        crate::commands::register_leaf(key, &node.meta);
        let handle = LeafHandle { content, meta: node.meta.clone() };
        (node, handle)
    }

    /// Builds `view` with a new owner, which the node keeps: signals made
    /// while building (and by its effects) are freed with the node.
    pub(crate) fn build_owned(view: AnyView, cx: &mut Cx) -> Node {
        let owner = Owner::new();
        let mut node = owner.with(|| view.build_node(cx));
        node.owner = Some(owner);
        node
    }

    pub(crate) fn own(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    pub(crate) fn keep(&mut self, object: NativeObject) {
        self.keep.push(object);
    }

    /// The native views of this subtree's leaves (and scroll and split views).
    pub(crate) fn leaf_views(&self, out: &mut Vec<NativeView>) {
        match &self.kind {
            Kind::Leaf(l) => out.push(l.view.clone()),
            Kind::Scroll { scroll, .. } => out.push(Native::scroll_as_view(scroll)),
            Kind::Split { panes, .. } => panes.iter().for_each(|(_, n)| n.leaf_views(out)),
            Kind::Stack { children, .. } | Kind::Layers { children, .. } => children.iter().for_each(|c| c.leaf_views(out)),
            Kind::Grid { rows, .. } => rows.iter().flatten().for_each(|c| c.leaf_views(out)),
            Kind::Group(g) => g.borrow().iter().for_each(|c| c.leaf_views(out)),
            Kind::Spacer => {}
        }
    }

    /// Calls `f` with each leaf of this subtree (not inside scroll views or
    /// splits: those are panes of their own) and its measured size.
    pub(crate) fn each_leaf(&self, f: &mut LeafFn<'_>) {
        match &self.kind {
            Kind::Leaf(l) => f(&l.view, &l.content, &self.meta),
            Kind::Stack { children, .. } | Kind::Layers { children, .. } => children.iter().for_each(|c| c.each_leaf(f)),
            Kind::Grid { rows, .. } => rows.iter().flatten().for_each(|c| c.each_leaf(f)),
            Kind::Group(g) => g.borrow().iter().for_each(|c| c.each_leaf(f)),
            Kind::Scroll { .. } | Kind::Split { .. } | Kind::Spacer => {}
        }
    }

    /// Every native view of this subtree: its leaves and their surfaces.
    pub(crate) fn all_views(&self) -> Vec<NativeView> {
        let mut out = self.decor.clone();
        match &self.kind {
            Kind::Leaf(l) => out.push(l.view.clone()),
            Kind::Scroll { scroll, .. } => out.push(Native::scroll_as_view(scroll)),
            Kind::Split { panes, .. } => panes.iter().for_each(|(_, n)| out.extend(n.all_views())),
            Kind::Stack { children, .. } | Kind::Layers { children, .. } => children.iter().for_each(|c| out.extend(c.all_views())),
            Kind::Grid { rows, .. } => rows.iter().flatten().for_each(|c| out.extend(c.all_views())),
            Kind::Group(g) => g.borrow().iter().for_each(|c| out.extend(c.all_views())),
            Kind::Spacer => {}
        }
        out
    }

    /// This node's children, as they are now.
    fn each_child_now(&self, f: &mut dyn FnMut(&Node)) {
        match &self.kind {
            Kind::Group(g) => g.borrow().iter().for_each(f),
            _ => self.children_static().into_iter().for_each(f),
        }
    }

    /// Hides or shows this subtree's views, given whether a view above it
    /// is hidden: each node's views are hidden while it or a view above it
    /// is, whatever order they were hidden in.
    pub(crate) fn apply_hidden(&self, above: bool) {
        let hide = above || self.meta.hidden.get();
        let mut own = self.decor.clone();
        match &self.kind {
            Kind::Leaf(l) => own.push(l.view.clone()),
            Kind::Scroll { scroll, .. } => own.push(Native::scroll_as_view(scroll)),
            _ => {}
        }
        if hide {
            for v in own.iter().filter(|v| !v.isHidden()) {
                v.setHidden(true);
                self.meta.hides.set(true);
            }
        } else if self.meta.hides.replace(false) {
            own.iter().for_each(|v| v.setHidden(false));
        }
        self.each_child_now(&mut |c| c.apply_hidden(hide));
    }

    /// Disables or enables this subtree's controls, given whether a view
    /// above it is disabled.
    pub(crate) fn apply_disabled(&self, above: bool) {
        let disable = above || self.meta.disabled.get();
        if let Kind::Leaf(l) = &self.kind
            && let Some(control) = l.view.downcast_ref::<objc2_app_kit::NSControl>()
        {
            control.setEnabled(!disable);
        }
        self.each_child_now(&mut |c| c.apply_disabled(disable));
    }

    /// Keeps a Rust value alive for as long as the node.
    pub(crate) fn hold<V: ?Sized + 'static>(&mut self, value: Rc<V>) {
        self.held.push(Rc::new(value));
    }
}

thread_local! {
    /// The views of a node leaving with a transition: collected instead of
    /// removed, to go when the transition ends.
    static LEAVING: UiCell<Option<Vec<NativeView>>> = const { UiCell::new(None) };
}

/// Removes `view` from its window, or keeps it for the leaving node's
/// transition.
fn remove_view(view: &NativeView) {
    let kept = LEAVING.try_with(|l| l.borrow_mut().as_mut().map(|l| l.push(view.clone())).is_some()).unwrap_or(false);
    if !kept {
        Native::remove(view);
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        LIVE_NODES.with(|n| n.set(n.get() - 1));
        // Leaving inside an animated change: where it was, for the view with
        // its id that comes in.
        if let Some(id) = &self.matched {
            crate::motion::matched_leaving(id, &self.all_views());
        }
        // Removed inside an animated change: its views (and its subtree's)
        // stay for its transition, then go.
        let leaving = self.transition.filter(|_| crate::motion::animating() && LEAVING.try_with(|l| l.borrow().is_none()).unwrap_or(false));
        if leaving.is_some() {
            LEAVING.with_borrow_mut(|l| *l = Some(Vec::new()));
        }
        // Effects stop first (dropping them), then the native view leaves.
        self.effects.clear();
        for d in &self.decor {
            remove_view(d);
        }
        if let Kind::Scroll { scroll, .. } = &self.kind {
            remove_view(&Native::scroll_as_view(scroll));
        }
        if let Kind::Split { split, .. } = &self.kind {
            remove_view(&Native::split_as_view(split));
        }
        if let Kind::Leaf(l) = &self.kind {
            crate::commands::unregister_leaf(Native::view_key(&l.view));
            remove_view(&l.view);
            if let Some(parent) = self.meta.parent.borrow().upgrade() {
                parent.mark();
            }
            invalidate_layout();
        }
        if let Some(transition) = leaving {
            // The subtree goes now, so its views are collected too.
            drop(std::mem::replace(&mut self.kind, Kind::Spacer));
            let views = LEAVING.with_borrow_mut(Option::take).unwrap_or_default();
            crate::motion::exit(views, transition);
            invalidate_layout();
        }
    }
}

/// Modifiers every view has.
pub trait ViewExt: View + Sized {
    /// Handles `command` while the focus is in this view (see
    /// [`Command`](crate::Command)).
    fn on_command(self, command: crate::Command, action: impl Fn() + 'static) -> Modified<Self> {
        Modified::new(self).on_command(command, action)
    }

    /// Handles `command`, enabled only while `enabled` returns true.
    fn on_command_when(self, command: crate::Command, enabled: impl Fn() -> bool + 'static, action: impl Fn() + 'static) -> Modified<Self> {
        Modified::new(self).on_command_when(command, enabled, action)
    }

    /// Publishes `value` under `name` to the focus (see
    /// [`focused_value`](crate::focused_value)).
    fn focused_value<T: Clone + 'static>(self, name: &'static str, value: T) -> Modified<Self> {
        Modified::new(self).focused_value(name, value)
    }

    /// Inset on all sides, in points.
    fn padding(self, points: f64) -> Modified<Self> {
        Modified::new(self).padding(points)
    }

    /// Inset `horizontal` points left and right and `vertical` above and
    /// below: a row's `.padding_xy(10.0, 7.0)`, as SwiftUI's
    /// `.padding(.horizontal, 10)` and `.padding(.vertical, 7)`.
    fn padding_xy(self, horizontal: f64, vertical: f64) -> Modified<Self> {
        Modified::new(self).padding_xy(horizontal, vertical)
    }

    /// A fixed width.
    fn width(self, points: f64) -> Modified<Self> {
        Modified::new(self).width(points)
    }

    /// At least `points` wide (SwiftUI's `frame(minWidth:)`).
    fn min_width(self, points: f64) -> Modified<Self> {
        Modified::new(self).min_width(points)
    }

    /// At least `points` tall (SwiftUI's `frame(minHeight:)`).
    fn min_height(self, points: f64) -> Modified<Self> {
        Modified::new(self).min_height(points)
    }

    /// A fixed height.
    fn height(self, points: f64) -> Modified<Self> {
        Modified::new(self).height(points)
    }

    /// A fixed size.
    fn frame(self, width: f64, height: f64) -> Modified<Self> {
        Modified::new(self).width(width).height(height)
    }

    /// Takes all the width its container offers.
    fn fill_width(self) -> Modified<Self> {
        Modified::new(self).fill_width()
    }

    /// Takes all the height its container offers.
    fn fill_height(self) -> Modified<Self> {
        Modified::new(self).fill_height()
    }

    /// Shows `content` in a popover pointing at this view while `shown` is
    /// true (see [`Modified::popover`]).
    fn popover<P: View>(self, shown: &Signal<bool>, content: impl Fn() -> P + 'static) -> Modified<Self> {
        Modified::new(self).popover(shown, content)
    }

    /// A right-click menu on this view's native views.
    fn context_menu(self, items: Vec<crate::chrome::MenuItem>) -> Modified<Self> {
        Modified::new(self).context_menu(items)
    }

    /// The name VoiceOver reads for this view's native views ("Name",
    /// "Volume"), for controls whose visible label is a separate view or
    /// that have none (a field after a `text("Name:")`, an icon button).
    fn accessibility_label(self, label: &str) -> Modified<Self> {
        Modified::new(self).accessibility_label(label)
    }

    /// A background behind the view, its padding included: with
    /// [`corner_radius`](Self::corner_radius) and [`shadow`](Self::shadow), a
    /// card. `Color::QuaternaryFill` is the system's grouped-card colour.
    /// A closure computes it from signals (a selected row, a status); it
    /// changes smoothly inside [`animated`](crate::animated).
    fn background(self, color: impl crate::IntoValue<crate::Color>) -> Modified<Self> {
        Modified::new(self).background(color)
    }

    /// Rounds the corners of the view's background and border, in points.
    fn corner_radius(self, points: f64) -> Modified<Self> {
        Modified::new(self).corner_radius(points)
    }

    /// A border `width` points wide around the view (inside its padding).
    fn border(self, color: crate::Color, width: f64) -> Modified<Self> {
        Modified::new(self).border(color, width)
    }

    /// A drop shadow under the view's background (give it a background).
    fn shadow(self, shadow: crate::Shadow) -> Modified<Self> {
        Modified::new(self).shadow(shadow)
    }

    /// The view at `opacity` (0 transparent, 1 opaque), all of it, or a
    /// closure computing it from signals; it fades inside
    /// [`animated`](crate::animated).
    fn opacity(self, opacity: impl crate::IntoValue<f64>) -> Modified<Self> {
        Modified::new(self).opacity(opacity)
    }

    /// The font of the text inside the view (all of it, when it's a row or
    /// a card): a text style or a custom font. Works after any modifier.
    fn font(self, font: crate::Font) -> Modified<Self> {
        Modified::new(self).font(font)
    }

    /// The colour of the text and symbols inside the view, or a closure
    /// computing it from signals. Works after any modifier.
    fn color(self, color: impl crate::IntoValue<crate::Color>) -> Modified<Self> {
        Modified::new(self).color(color)
    }

    /// What VoiceOver reads as the view's value, after its name: a card's
    /// state ("done"), a gauge's level. A closure keeps it current.
    fn accessibility_value(self, value: impl crate::IntoValue<String>) -> Modified<Self> {
        Modified::new(self).accessibility_value(value)
    }

    /// A system material behind the view, its padding included: a
    /// translucent sidebar, header or panel that blurs what's behind it.
    /// [`corner_radius`](Self::corner_radius) rounds it.
    fn material(self, material: crate::Material) -> Modified<Self> {
        Modified::new(self).material(material)
    }

    /// Liquid Glass behind the view, its padding included (macOS 26; a
    /// translucent material on macOS 15). [`corner_radius`](Self::corner_radius)
    /// rounds it.
    fn glass(self, glass: crate::Glass) -> Modified<Self> {
        Modified::new(self).glass(glass)
    }

    /// A background following signals (now [`background`](Self::background)
    /// takes a closure).
    #[deprecated(since = "0.4.0", note = "`background` takes a closure: `.background(move || ..)`")]
    fn background_when(self, color: impl Fn() -> crate::Color + 'static) -> Modified<Self> {
        Modified::new(self).background(color)
    }

    /// Opacity following signals (now [`opacity`](Self::opacity) takes a
    /// closure).
    #[deprecated(since = "0.4.0", note = "`opacity` takes a closure: `.opacity(move || ..)`")]
    fn opacity_when(self, opacity: impl Fn() -> f64 + 'static) -> Modified<Self> {
        Modified::new(self).opacity(opacity)
    }

    /// How the view comes and goes when it's added or removed (by
    /// [`dynamic`] or [`for_each`]) inside [`animated`](crate::animated).
    fn transition(self, transition: crate::Transition) -> Modified<Self> {
        Modified::new(self).transition(transition)
    }

    /// Matched geometry (SwiftUI's `matchedGeometryEffect`): when, in one
    /// [`animated`](crate::animated) change, a view with this `id` goes
    /// and another with it comes (a thumbnail replaced by its detail, a
    /// selection marker moving), the new one starts where and as large as
    /// the old one was, and moves to its place.
    fn matched_geometry(self, id: &str) -> Modified<Self> {
        Modified::new(self).matched_geometry(id)
    }

    /// Calls `action` with `true` when the pointer moves over the view and
    /// `false` when it leaves (while the app is active).
    fn on_hover(self, action: impl Fn(bool) + 'static) -> Modified<Self> {
        Modified::new(self).on_hover(action)
    }

    /// A background of `color` while the pointer is over the view, fading
    /// in and out: for rows and cards that can be clicked.
    fn hover_highlight(self, color: crate::Color) -> Modified<Self> {
        Modified::new(self).hover_highlight(color)
    }

    /// A tooltip: `text` appears in a small box when the pointer rests on
    /// the view (SwiftUI's `help`). VoiceOver reads it as the view's help.
    fn tooltip(self, text: &str) -> Modified<Self> {
        Modified::new(self).tooltip(text)
    }

    /// Hides the view while `when()` is true, following the signals it
    /// reads: its native views stay built (no rebuild, the state inside
    /// kept) but aren't shown, and it takes no room in its stack, as if it
    /// weren't there. For a panel shown in one mode only, rather than a
    /// [`dynamic`] that rebuilds it each time. VoiceOver and Tab skip it.
    fn hidden(self, when: impl crate::IntoValue<bool>) -> Modified<Self> {
        Modified::new(self).hidden(when)
    }

    /// Disables the view's controls while `when()` is true (a picker that
    /// only matters once a checkbox is on, a form while it's saving),
    /// following the signals it reads. SwiftUI's `disabled`.
    fn disabled(self, when: impl crate::IntoValue<bool>) -> Modified<Self> {
        Modified::new(self).disabled(when)
    }

    /// Makes the whole view a button: a click on it calls `action`, it
    /// dims while pressed, and VoiceOver reads it as a button named by its
    /// text. For rows, cards and tiles; controls inside it don't get
    /// clicks.
    fn on_click(self, action: impl Fn() + 'static) -> Modified<Self> {
        Modified::new(self).on_click(action)
    }
}

impl<V: View> ViewExt for V {}

/// A view with modifiers applied (see [`ViewExt`]). Layout and look
/// modifiers apply in the order written, as SwiftUI's do:
/// `.frame(18.0, 18.0).padding(7.0)` is an 18-point view with 7 points
/// around it, and `.padding(16.0).background(..)` puts the background under
/// the padding too, while `.background(..).padding(16.0)` doesn't.
pub struct Modified<V> {
    view: V,
    ops: Vec<Op>,
    menu: Option<Vec<crate::chrome::MenuItem>>,
    label: Option<String>,
    responds: crate::commands::Responds,
    popover: Option<crate::popover::PopoverSpec>,
    opacity: Option<f64>,
    opacity_when: Option<Box<dyn Fn() -> f64>>,
    transition: Option<crate::Transition>,
    matched: Option<Rc<str>>,
    hover: Option<Rc<dyn Fn(bool)>>,
    click: Option<Rc<dyn Fn()>>,
    tooltip: Option<String>,
    disabled: Option<crate::Value<bool>>,
    hidden: Option<crate::Value<bool>>,
    font: Option<crate::Font>,
    color: Option<crate::Value<crate::Color>>,
    ax_value: Option<crate::Value<String>>,
}

/// A layout or look modifier, in the order written.
enum Op {
    Padding(f64, f64),
    Width(f64),
    MinWidth(f64),
    MinHeight(f64),
    Height(f64),
    FillWidth,
    FillHeight,
    Background(crate::Color),
    Radius(f64),
    Border(crate::Color, f64),
    Shadow(crate::Shadow),
    Backdrop(crate::material::Backdrop),
    BackgroundWhen(Rc<dyn Fn() -> crate::Color>),
    HoverHighlight(crate::Color),
}

impl<V: View> Modified<V> {
    fn new(view: V) -> Self {
        Modified { view, ops: Vec::new(), menu: None, label: None, responds: Default::default(), popover: None, opacity: None, opacity_when: None, transition: None, matched: None, hover: None, click: None, tooltip: None, disabled: None, hidden: None, font: None, color: None, ax_value: None }
    }

    /// Shows `content` in a popover pointing at this view while `shown` is
    /// true; closing it (Escape, a click outside) sets `shown` to false.
    /// The content is built each time it opens and freed when it closes.
    pub fn popover<P: View>(self, shown: &Signal<bool>, content: impl Fn() -> P + 'static) -> Self {
        self.popover_on(crate::Edge::Below, shown, content)
    }

    /// [`popover`](Self::popover) on a given side of the view.
    pub fn popover_on<P: View>(mut self, edge: crate::Edge, shown: &Signal<bool>, content: impl Fn() -> P + 'static) -> Self {
        self.popover = Some(crate::popover::PopoverSpec { shown: *shown, content: Rc::new(move || AnyView::new(content())), edge });
        self
    }

    /// Handles `command` while the focus is in this view (or nothing nearer
    /// the focus handles it). See [`Command`](crate::Command).
    pub fn on_command(mut self, command: crate::Command, action: impl Fn() + 'static) -> Self {
        self.responds.commands.push((command, crate::commands::Handler::new(action, None)));
        self
    }

    /// Handles `command` like [`on_command`](Self::on_command), enabled only
    /// while `enabled` returns true (a disabled handler still answers: the
    /// menu item is greyed out, and handlers further out aren't asked).
    pub fn on_command_when(mut self, command: crate::Command, enabled: impl Fn() -> bool + 'static, action: impl Fn() + 'static) -> Self {
        self.responds.commands.push((command, crate::commands::Handler::new(action, Some(Box::new(enabled)))));
        self
    }

    /// Publishes `value` under `name` while the focus is in this view, for
    /// [`focused_value`](crate::focused_value) (a signal, typically: the
    /// list's selection, the editor's document).
    pub fn focused_value<T: Clone + 'static>(mut self, name: &'static str, value: T) -> Self {
        self.responds.values.push((name, Rc::new(value)));
        self
    }

    /// The name VoiceOver reads for this view's native views.
    pub fn accessibility_label(mut self, label: &str) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A right-click menu on this view's native views.
    pub fn context_menu(mut self, items: Vec<crate::chrome::MenuItem>) -> Self {
        self.menu = Some(items);
        self
    }

    /// Takes all the width its container offers.
    pub fn fill_width(mut self) -> Self {
        self.ops.push(Op::FillWidth);
        self
    }

    /// Takes all the height its container offers.
    pub fn fill_height(mut self) -> Self {
        self.ops.push(Op::FillHeight);
        self
    }

    /// Inset on all sides, in points.
    pub fn padding(mut self, points: f64) -> Self {
        self.ops.push(Op::Padding(points, points));
        self
    }

    /// Inset left and right, and above and below (see [`ViewExt::padding_xy`]).
    pub fn padding_xy(mut self, horizontal: f64, vertical: f64) -> Self {
        self.ops.push(Op::Padding(horizontal, vertical));
        self
    }

    /// A fixed width.
    pub fn width(mut self, points: f64) -> Self {
        self.ops.push(Op::Width(points));
        self
    }

    /// A minimum width (see [`ViewExt::min_width`]).
    pub fn min_width(mut self, points: f64) -> Self {
        self.ops.push(Op::MinWidth(points));
        self
    }

    /// A minimum height (see [`ViewExt::min_height`]).
    pub fn min_height(mut self, points: f64) -> Self {
        self.ops.push(Op::MinHeight(points));
        self
    }

    /// A fixed height.
    pub fn height(mut self, points: f64) -> Self {
        self.ops.push(Op::Height(points));
        self
    }

    /// A background behind the view (see [`ViewExt::background`]).
    pub fn background(mut self, color: impl crate::IntoValue<crate::Color>) -> Self {
        self.ops.push(match color.into_value() {
            crate::Value::Fixed(c) => Op::Background(c),
            crate::Value::Computed(f) => Op::BackgroundWhen(f),
        });
        self
    }

    /// Rounded corners for the background and border.
    pub fn corner_radius(mut self, points: f64) -> Self {
        self.ops.push(Op::Radius(points));
        self
    }

    /// A border around the view.
    pub fn border(mut self, color: crate::Color, width: f64) -> Self {
        self.ops.push(Op::Border(color, width));
        self
    }

    /// A drop shadow under the view's background.
    pub fn shadow(mut self, shadow: crate::Shadow) -> Self {
        self.ops.push(Op::Shadow(shadow));
        self
    }

    /// The view at `opacity` (see [`ViewExt::opacity`]).
    pub fn opacity(mut self, opacity: impl crate::IntoValue<f64>) -> Self {
        match opacity.into_value() {
            crate::Value::Fixed(o) => self.opacity = Some(o),
            crate::Value::Computed(f) => self.opacity_when = Some(Box::new(move || f())),
        }
        self
    }

    /// The font of the text inside (see [`ViewExt::font`]).
    pub fn font(mut self, font: crate::Font) -> Self {
        self.font = Some(font);
        self
    }

    /// The colour of the text and symbols inside (see [`ViewExt::color`]).
    pub fn color(mut self, color: impl crate::IntoValue<crate::Color>) -> Self {
        self.color = Some(color.into_value());
        self
    }

    /// What VoiceOver reads as the value (see [`ViewExt::accessibility_value`]).
    pub fn accessibility_value(mut self, value: impl crate::IntoValue<String>) -> Self {
        self.ax_value = Some(value.into_value());
        self
    }

    /// A system material behind the view (see [`ViewExt::material`]).
    pub fn material(mut self, material: crate::Material) -> Self {
        self.ops.push(Op::Backdrop(crate::material::Backdrop::Material(material)));
        self
    }

    /// Liquid Glass behind the view (see [`ViewExt::glass`]).
    pub fn glass(mut self, glass: crate::Glass) -> Self {
        self.ops.push(Op::Backdrop(crate::material::Backdrop::Glass(glass)));
        self
    }

    /// A background following signals (now `background` takes a closure).
    #[deprecated(since = "0.4.0", note = "`background` takes a closure: `.background(move || ..)`")]
    pub fn background_when(self, color: impl Fn() -> crate::Color + 'static) -> Self {
        self.background(color)
    }

    /// Opacity following signals (now `opacity` takes a closure).
    #[deprecated(since = "0.4.0", note = "`opacity` takes a closure: `.opacity(move || ..)`")]
    pub fn opacity_when(self, opacity: impl Fn() -> f64 + 'static) -> Self {
        self.opacity(opacity)
    }

    /// Matched geometry (see [`ViewExt::matched_geometry`]).
    pub fn matched_geometry(mut self, id: &str) -> Self {
        self.matched = Some(id.into());
        self
    }

    /// How the view comes and goes (see [`ViewExt::transition`]).
    pub fn transition(mut self, transition: crate::Transition) -> Self {
        self.transition = Some(transition);
        self
    }

    /// Hover in and out (see [`ViewExt::on_hover`]).
    pub fn on_hover(mut self, action: impl Fn(bool) + 'static) -> Self {
        self.hover = Some(Rc::new(action));
        self
    }

    /// A background while hovered (see [`ViewExt::hover_highlight`]).
    pub fn hover_highlight(mut self, color: crate::Color) -> Self {
        self.ops.push(Op::HoverHighlight(color));
        self
    }

    /// A tooltip (see [`ViewExt::tooltip`]).
    pub fn tooltip(mut self, text: &str) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Disabled controls (see [`ViewExt::disabled`]).
    pub fn disabled(mut self, when: impl crate::IntoValue<bool>) -> Self {
        self.disabled = Some(when.into_value());
        self
    }

    /// Hidden while `when()` is true (see [`ViewExt::hidden`]).
    pub fn hidden(mut self, when: impl crate::IntoValue<bool>) -> Self {
        self.hidden = Some(when.into_value());
        self
    }

    /// The view as a button (see [`ViewExt::on_click`]).
    pub fn on_click(mut self, action: impl Fn() + 'static) -> Self {
        self.click = Some(Rc::new(action));
        self
    }
}

impl<V: View> View for Modified<V> {
    fn build(self, cx: &mut Cx) -> Node {
        let mut node = self.view.build(cx);
        // A hover highlight follows a signal the hover handler sets.
        let mut hover = self.hover;
        let hovered = self.ops.iter().any(|o| matches!(o, Op::HoverHighlight(_))).then(|| {
            let hovered = crate::Signal::new(false);
            let user = hover.take();
            hover = Some(Rc::new(move |on: bool| {
                crate::animated(|| hovered.set(on));
                if let Some(f) = &user {
                    f(on);
                }
            }));
            hovered
        });
        // Look modifiers written one after another make one surface.
        let mut look = crate::surface::Look::default();
        let mut has_look = false;
        let mut look_when: Option<Rc<dyn Fn() -> crate::Color>> = None;
        for op in self.ops {
            let is_look = matches!(op, Op::Background(_) | Op::Radius(_) | Op::Border(..) | Op::Shadow(_) | Op::Backdrop(_) | Op::BackgroundWhen(_) | Op::HoverHighlight(_));
            if !is_look && has_look {
                node = add_surface(node, std::mem::take(&mut look), cx);
                follow_background(&mut node, look_when.take());
                has_look = false;
            }
            match op {
                Op::Padding(px, py) => {
                    if node.padding != (0.0, 0.0) || node.fixed != (None, None) || !node.decor.is_empty() {
                        node = wrap(node);
                    }
                    node.padding = (px, py);
                }
                Op::MinWidth(w) => {
                    if node.min.0.is_some() {
                        node = wrap(node);
                    }
                    node.min.0 = Some(w);
                }
                Op::MinHeight(h) => {
                    if node.min.1.is_some() {
                        node = wrap(node);
                    }
                    node.min.1 = Some(h);
                }
                Op::Width(w) => {
                    if node.fixed.0.is_some() {
                        node = wrap(node);
                    }
                    node.fixed.0 = Some(w);
                }
                Op::Height(h) => {
                    if node.fixed.1.is_some() {
                        node = wrap(node);
                    }
                    node.fixed.1 = Some(h);
                }
                Op::FillWidth => node.fill.0 = true,
                Op::FillHeight => node.fill.1 = true,
                Op::Background(c) => look.background = Some(c),
                Op::Radius(r) => look.radius = r,
                Op::Border(c, w) => look.border = Some((c, w)),
                Op::Shadow(s) => look.shadow = Some(s),
                Op::Backdrop(b) => look.backdrop = Some(b),
                Op::BackgroundWhen(f) => {
                    look.background = Some(crate::signal::untrack(|| f()));
                    look_when = Some(f);
                }
                Op::HoverHighlight(c) => {
                    let hovered = hovered.expect("made above");
                    // Over the view's own background (a selected row's): the
                    // highlight shows only where that's clear.
                    let base: Option<Rc<dyn Fn() -> crate::Color>> = look_when.take().or_else(|| look.background.map(|b| Rc::new(move || b) as Rc<dyn Fn() -> crate::Color>));
                    let f: Rc<dyn Fn() -> crate::Color> = Rc::new(move || {
                        let under = base.as_ref().map_or(crate::Color::Clear, |b| b());
                        if hovered.get() && under == crate::Color::Clear { c } else { under }
                    });
                    look.background = Some(crate::signal::untrack(|| f()));
                    look_when = Some(f);
                }
            }
            has_look |= is_look;
        }
        if has_look {
            node = add_surface(node, look, cx);
            follow_background(&mut node, look_when);
        }
        if let Some(alpha) = self.opacity {
            for v in node.all_views() {
                v.setAlphaValue(alpha);
            }
        }
        if let Some(f) = self.opacity_when {
            let views = node.all_views();
            let first = Cell::new(true);
            node.own(Effect::new(move || {
                let alpha = f();
                let views = views.clone();
                let set = move || views.iter().for_each(|v| v.setAlphaValue(alpha));
                if first.replace(false) { set() } else { crate::motion::apply(set) }
            }));
        }
        if hover.is_some() || self.click.is_some() {
            let views = node.all_views();
            let interaction = crate::interact::Interaction::new(cx.mtm, hover, self.click.clone());
            if self.click.is_some() {
                let mut leaves = Vec::new();
                node.leaf_views(&mut leaves);
                let texts = leaves.into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).collect();
                interaction.set_views(views, self.label.clone(), texts);
            }
            let interaction: NativeView = objc2::rc::Retained::into_super(interaction);
            Native::add_subview(&cx.container, &interaction);
            node.decor.push(interaction);
            node.meta.mark();
        }
        if let Some(font) = &self.font {
            // Text inside takes the font, and is measured again.
            let ns = font.ns();
            node.each_leaf(&mut |v, content, meta| {
                if let Some(field) = v.downcast_ref::<objc2_app_kit::NSTextField>() {
                    field.setFont(Some(&ns));
                    content.set(None);
                    meta.mark();
                }
            });
        }
        if let Some(color) = self.color {
            let mut fields = Vec::new();
            let mut symbols = Vec::new();
            node.each_leaf(&mut |v, _, _| {
                if let Ok(f) = v.clone().downcast::<objc2_app_kit::NSTextField>() {
                    fields.push(f);
                } else if let Ok(i) = v.clone().downcast::<objc2_app_kit::NSImageView>() {
                    symbols.push(i);
                }
            });
            let set = move |c: crate::Color| {
                let ns = c.ns();
                fields.iter().for_each(|f| f.setTextColor(Some(&ns)));
                symbols.iter().for_each(|i| i.setContentTintColor(Some(&ns)));
            };
            match color {
                crate::Value::Fixed(c) => set(c),
                color => node.own(Effect::new(move || set(color.get()))),
            }
        }
        if let Some(value) = self.ax_value {
            // On a clickable view's button, else on its native views.
            let mut targets: Vec<NativeView> = node.decor.iter().filter(|d| d.class().name().to_string_lossy() == "FerrokitInteraction").cloned().collect();
            if targets.is_empty() {
                node.leaf_views(&mut targets);
            }
            node.own(Effect::new(move || {
                let v = objc2_foundation::NSString::from_str(&value.get());
                for t in &targets {
                    // SAFETY: an NSString is a valid accessibility value.
                    unsafe { objc2_app_kit::NSAccessibility::setAccessibilityValue(&**t, Some(&v)) };
                }
            }));
        }
        if let Some(tip) = &self.tooltip {
            let tip = objc2_foundation::NSString::from_str(tip);
            let mut leaves = Vec::new();
            node.leaf_views(&mut leaves);
            // The leaves, and a clickable view's interaction over them.
            for v in leaves.iter().chain(node.decor.iter().filter(|d| d.class().name().to_string_lossy() == "FerrokitInteraction")) {
                v.setToolTip(Some(&tip));
            }
        }
        if let Some(id) = self.matched {
            crate::motion::matched_entering(&id, node.all_views());
            node.matched = Some(id);
        }
        if let Some(t) = self.transition {
            node.transition = Some(t);
            if crate::motion::animating() {
                crate::motion::enter(node.all_views(), t, self.opacity.unwrap_or(1.0));
            }
        }
        if let Some(items) = self.menu {
            let mut leaves = Vec::new();
            node.leaf_views(&mut leaves);
            for t in Native::set_context_menu(cx.mtm, &leaves, items) {
                node.keep(t);
            }
        }
        // A clickable view's label names its button (the interaction view),
        // not each view inside it.
        if let Some(label) = self.label.as_ref().filter(|_| self.click.is_none()) {
            let mut leaves = Vec::new();
            node.leaf_views(&mut leaves);
            for v in leaves {
                Native::set_accessibility_label(&v, label);
            }
        }
        {
            let mut r = node.meta.responds.borrow_mut();
            r.commands.extend(self.responds.commands);
            r.values.extend(self.responds.values);
        }
        if let Some(spec) = self.popover {
            // It points at the view's first native view (a button, a field).
            let mut leaves = Vec::new();
            node.leaf_views(&mut leaves);
            if let Some(anchor) = leaves.into_iter().next() {
                crate::popover::attach(&mut node, anchor, spec);
            }
        }
        if let Some(when) = self.disabled {
            node = disable_when(node, when);
        }
        if let Some(when) = self.hidden {
            node = hide_when(node, when);
        }
        node
    }
}

/// `node` in a group of its own that [`ViewExt::hidden`] hides while
/// `when()` is true. The views are found each time (a [`dynamic`] inside
/// may have replaced them).
fn hide_when(node: Node, when: crate::Value<bool>) -> Node {
    let children = Rc::new(RefCell::new(Vec::new()));
    let mut group = Node::new(Kind::Group(children.clone()));
    node.meta.set_parent(&group.meta);
    children.borrow_mut().push(node);
    let meta = group.meta.clone();
    let first = Cell::new(true);
    group.own(Effect::new(move || {
        let hide = when.get();
        if meta.hidden.get() == hide && !first.get() {
            return;
        }
        let initial = first.replace(false);
        if !initial || hide {
            set_hidden(&meta, &children, hide);
        }
    }));
    group
}

/// `node` in a group of its own whose controls [`ViewExt::disabled`]
/// disables while `when()` is true, with those of the views inside it,
/// whenever they were built.
fn disable_when(node: Node, when: crate::Value<bool>) -> Node {
    let children = Rc::new(RefCell::new(Vec::new()));
    let mut group = Node::new(Kind::Group(children.clone()));
    node.meta.set_parent(&group.meta);
    children.borrow_mut().push(node);
    let meta = group.meta.clone();
    let first = Cell::new(true);
    group.own(Effect::new(move || {
        let disable = when.get();
        let initial = first.replace(false);
        if meta.disabled.replace(disable) == disable && !initial {
            return;
        }
        if !initial || disable {
            let above = meta.parent().is_some_and(|p| p.disabled_here());
            children.borrow().iter().for_each(|c| c.apply_disabled(above || disable));
        }
    }));
    group
}

/// Sets a control's own disabled state (a button's `.disabled(..)`): it is
/// enabled only if no view above it is disabled either.
pub(crate) fn set_control_disabled(meta: &Rc<Meta>, control: &objc2_app_kit::NSControl, disabled: bool) {
    meta.disabled.set(disabled);
    control.setEnabled(!meta.disabled_here());
}

/// A branch built into `parent` (a `dynamic`'s new branch, a `for_each`'s
/// new row) takes the state of the views above it: hidden while one of
/// them is, its controls disabled while one of them is.
fn inherit_state(parent: &Rc<Meta>, new: &Node) {
    if parent.hidden_here() {
        new.apply_hidden(true);
    }
    if parent.disabled_here() {
        new.apply_disabled(true);
    }
}

fn set_hidden(meta: &Rc<Meta>, children: &RefCell<Vec<Node>>, hide: bool) {
    meta.hidden.set(hide);
    let above = meta.parent().is_some_and(|p| p.hidden_here());
    children.borrow().iter().for_each(|c| c.apply_hidden(above || hide));
    // Laid out again (it was skipped while hidden, so it may still be dirty).
    meta.dirty.set(false);
    meta.mark();
    invalidate_layout();
}

/// Makes `node`'s last surface follow `color` (a dynamic background).
fn follow_background(node: &mut Node, color: Option<Rc<dyn Fn() -> crate::Color>>) {
    let Some(color) = color else { return };
    let Some(surface) = node.decor.iter().rev().find_map(|d| d.clone().downcast::<crate::surface::Surface>().ok()) else { return };
    let first = Cell::new(true);
    node.own(Effect::new(move || {
        let c = color();
        if first.replace(false) {
            return; // the surface starts with it
        }
        let surface = surface.clone();
        crate::motion::apply(move || surface.set_background(c));
    }));
}

/// `node` inside a box of its own, so a modifier can go around what it
/// already has (padding outside a frame or a background, a second frame).
/// The box is laid out at `node`'s size and centres it when larger.
fn wrap(node: Node) -> Node {
    Node::new(Kind::Stack { axis: Axis::Vertical, spacing: 0.0, align: Some(Align::Center), children: vec![node] })
}

/// A surface behind `node`'s native views, framed to its rectangle (a
/// node that already has one goes in a box first). A material or glass in
/// the look goes under the surface.
fn add_surface(node: Node, mut look: crate::surface::Look, cx: &Cx) -> Node {
    let mut node = if node.decor.is_empty() { node } else { wrap(node) };
    let mut layers = Vec::new();
    if let Some(b) = look.backdrop.take() {
        layers.push(crate::material::backdrop(cx.mtm, b, look.radius));
    }
    if look != (crate::surface::Look { radius: look.radius, ..Default::default() }) || layers.is_empty() {
        layers.push(objc2::rc::Retained::into_super(crate::surface::Surface::new(cx.mtm, look)));
    }
    // Below the lowest of the node's views in the container (they were
    // added as it was built), bottom layer first.
    let views = node.all_views();
    let subviews = cx.container.subviews();
    let lowest = subviews.iter().find(|s| views.iter().any(|v| objc2::rc::Retained::as_ptr(v) == objc2::rc::Retained::as_ptr(s)));
    for layer in layers {
        match &lowest {
            Some(lowest) => cx.container.addSubview_positioned_relativeTo(&layer, objc2_app_kit::NSWindowOrderingMode::Below, Some(lowest)),
            None => Native::add_subview(&cx.container, &layer),
        }
        node.decor.push(layer);
    }
    node.meta.mark();
    node
}

/// A list of child views: a tuple of views, or a `Vec<AnyView>`.
pub trait Views {
    /// Erases the children, in order.
    fn into_views(self) -> Vec<AnyView>;
}

impl Views for Vec<AnyView> {
    fn into_views(self) -> Vec<AnyView> {
        self
    }
}

/// One view alone is a list of one: `vstack(for_each(..))`,
/// `scroll(vstack(text("Notes")))`.
impl<V: View> Views for V {
    fn into_views(self) -> Vec<AnyView> {
        vec![AnyView::new(self)]
    }
}

macro_rules! tuple_views {
    ($($v:ident),+) => {
        impl<$($v: View),+> Views for ($($v,)+) {
            #[allow(non_snake_case)]
            fn into_views(self) -> Vec<AnyView> {
                let ($($v,)+) = self;
                vec![$(AnyView::new($v)),+]
            }
        }
    };
}
tuple_views!(A);
tuple_views!(A, B);
tuple_views!(A, B, C);
tuple_views!(A, B, C, D);
tuple_views!(A, B, C, D, E);
tuple_views!(A, B, C, D, E, F);
tuple_views!(A, B, C, D, E, F, G);
tuple_views!(A, B, C, D, E, F, G, H);
tuple_views!(A, B, C, D, E, F, G, H, I);
tuple_views!(A, B, C, D, E, F, G, H, I, J);

/// A stack of views along one axis.
pub struct Stack {
    axis: Axis,
    spacing: f64,
    align: Option<Align>,
    children: Vec<AnyView>,
}

impl Stack {
    /// Alignment across the axis (default: leading for vertical stacks,
    /// centred for horizontal ones).
    pub fn alignment(mut self, align: Align) -> Self {
        self.align = Some(align);
        self
    }

    /// Space between children, in points (default 8).
    pub fn spacing(mut self, points: f64) -> Self {
        self.spacing = points;
        self
    }
}

impl View for Stack {
    fn build(self, cx: &mut Cx) -> Node {
        let children = self.children.into_iter().map(|c| c.build_node(&mut cx.child())).collect();
        Node::new(Kind::Stack { axis: self.axis, spacing: self.spacing, align: self.align, children })
    }
}

/// Children top to bottom.
pub fn vstack(children: impl Views) -> Stack {
    Stack { axis: Axis::Vertical, spacing: 8.0, align: None, children: children.into_views() }
}

/// Children left to right.
pub fn hstack(children: impl Views) -> Stack {
    Stack { axis: Axis::Horizontal, spacing: 8.0, align: None, children: children.into_views() }
}

/// Children on top of one another (see [`zstack`]).
pub struct Layers {
    align: (Align, Align),
    children: Vec<AnyView>,
}

impl Layers {
    /// Where children smaller than the stack sit, across and down (default
    /// centred): `Align::Start` is leading or top, `Align::End` trailing or
    /// bottom.
    pub fn alignment(mut self, horizontal: Align, vertical: Align) -> Self {
        self.align = (horizontal, vertical);
        self
    }
}

impl View for Layers {
    fn build(self, cx: &mut Cx) -> Node {
        let children = self.children.into_iter().map(|c| c.build_node(&mut cx.child())).collect();
        Node::new(Kind::Layers { align: self.align, children })
    }
}

/// Children on top of one another, the first at the back: text or a glass
/// panel over a picture, a badge over an icon. The stack is as large as its
/// largest child. Children that fill (a picture with `.fill_width()` and
/// `.fill_height()`) take its whole size; the others keep their own and are
/// centred, or placed by [`alignment`](Layers::alignment).
///
/// ```no_run
/// use ferrokit::*;
///
/// let hero = zstack((
///     image_file("cover.jpg").fill().fill_width().fill_height(),
///     text("Summer").font(Font::large_title()).padding(16.0).glass(Glass::Regular).corner_radius(14.0).padding(20.0),
/// ))
/// .alignment(Align::Start, Align::End)
/// .height(260.0);
/// # let _ = hero;
/// ```
pub fn zstack(children: impl Views) -> Layers {
    Layers { align: (Align::Center, Align::Center), children: children.into_views() }
}

/// Rows of cells in aligned columns: each column is as wide as its widest
/// cell, each row as tall as its tallest. Columns (rows) with a flexible cell
/// share the extra space.
pub struct Grid {
    spacing: (f64, f64),
    rows: Vec<Vec<AnyView>>,
    align: Vec<Align>,
}

impl Grid {
    /// Horizontal alignment of the cells in `column` (default leading).
    /// Mac forms right-align their label column: `.align_column(0, Align::End)`.
    pub fn align_column(mut self, column: usize, align: Align) -> Self {
        if self.align.len() <= column {
            self.align.resize(column + 1, Align::Start);
        }
        self.align[column] = align;
        self
    }

    /// Space between columns and between rows (default 8, 8).
    pub fn spacing(mut self, columns: f64, rows: f64) -> Self {
        self.spacing = (columns, rows);
        self
    }
}

impl View for Grid {
    fn build(self, cx: &mut Cx) -> Node {
        let rows = self.rows.into_iter().map(|r| r.into_iter().map(|c| c.build_node(&mut cx.child())).collect()).collect();
        Node::new(Kind::Grid { spacing: self.spacing, rows, align: self.align })
    }
}

/// A grid from rows of views (rows may be shorter than others).
pub fn grid(rows: impl IntoIterator<Item = impl Views>) -> Grid {
    Grid { spacing: (8.0, 8.0), rows: rows.into_iter().map(Views::into_views).collect(), align: Vec::new() }
}



/// Runs a side effect for as long as this place in the tree exists; takes
/// no space. For work that follows signals but shows nothing (applying a
/// setting, saving a value).
pub struct Watch(Box<dyn FnMut()>);

impl View for Watch {
    fn build(self, _cx: &mut Cx) -> Node {
        let mut node = Node::new(Kind::Group(Rc::default()));
        node.own(Effect::new(self.0));
        node
    }
}

/// A side effect owned by the view tree (see [`Watch`]).
pub fn watch(f: impl FnMut() + 'static) -> Watch {
    Watch(Box::new(f))
}

/// Flexible empty space along its stack's axis.
pub struct Spacer;

impl View for Spacer {
    fn build(self, _cx: &mut Cx) -> Node {
        Node::new(Kind::Spacer)
    }
}

/// Flexible empty space along its stack's axis.
pub fn spacer() -> Spacer {
    Spacer
}


/// A region whose content is rebuilt when the signals `f` reads change. If
/// the new view has the same `.id(..)` as the current one, the current node
/// (and its native views) is kept.
pub struct Dynamic(Box<dyn Fn() -> AnyView>);

impl View for Dynamic {
    fn build(self, cx: &mut Cx) -> Node {
        let children: Rc<RefCell<Vec<Node>>> = Rc::default();
        let mut node = Node::new(Kind::Group(children.clone()));
        let meta = node.meta.clone();
        let container = cx.container.clone();
        let f = self.0;
        node.own(Effect::new(move || {
            // Signals the branch makes belong to it (freed when it's replaced).
            let owner = Owner::new();
            let view = owner.with(&f);
            untrack(|| {
                let current = children.borrow().first().and_then(|n| n.id);
                if view.id.is_some() && view.id == current {
                    return;
                }
                let mut new = owner.with(|| view.build_node(&mut Cx::new(container.clone())));
                new.owner = Some(owner);
                new.meta.set_parent(&meta);
                inherit_state(&meta, &new);
                // The old node (and its subtree) is dropped here, outside
                // the borrow: teardown.
                let old = std::mem::replace(&mut *children.borrow_mut(), vec![new]);
                drop(old);
                meta.mark();
            });
        }));
        node
    }
}

/// A region rebuilt when the signals `f` reads change (see [`Dynamic`]).
pub fn dynamic<V: View>(f: impl Fn() -> V + 'static) -> Dynamic {
    Dynamic(Box::new(move || {
        let v = f();
        // Keep an id the view already carries.
        let any: Box<dyn std::any::Any> = Box::new(v);
        match any.downcast::<AnyView>() {
            Ok(a) => *a,
            Err(v) => AnyView::new(*v.downcast::<V>().expect("view type")),
        }
    }))
}

/// Items with a stable identity, for [`for_each`].
pub trait Identifiable {
    /// The identity type.
    type Id: Hash + Eq + Clone + 'static;
    /// This item's identity.
    fn id(&self) -> Self::Id;
}

/// One view per item. When the items change, items whose id is still present
/// keep their node (and native views); new ones are built; removed ones are
/// torn down. Order follows the items.
pub struct ForEach(Box<dyn FnOnce(&mut Cx) -> Node>);

impl View for ForEach {
    fn build(self, cx: &mut Cx) -> Node {
        (self.0)(cx)
    }
}

/// One view per item from `items` (re-run when the signals it reads change).
pub fn for_each<T, V>(items: impl Fn() -> Vec<T> + 'static, row: impl Fn(T) -> V + 'static) -> ForEach
where
    T: Identifiable + 'static,
    V: View,
{
    ForEach(Box::new(move |cx| {
        let children: Rc<RefCell<Vec<Node>>> = Rc::default();
        let keys: Rc<RefCell<Vec<T::Id>>> = Rc::default();
        let mut node = Node::new(Kind::Group(children.clone()));
        let meta = node.meta.clone();
        let container = cx.container.clone();
        node.own(Effect::new(move || {
            let items = items();
            untrack(|| {
                // Taken out first: a node dropped here (two items with one
                // id) is torn down outside the borrows.
                let old_keys: Vec<T::Id> = keys.borrow_mut().drain(..).collect();
                let old_nodes: Vec<Node> = children.borrow_mut().drain(..).collect();
                let mut old: HashMap<T::Id, Node> = old_keys.into_iter().zip(old_nodes).collect();
                let mut new_keys = Vec::with_capacity(items.len());
                let mut new_nodes = Vec::with_capacity(items.len());
                for item in items {
                    let key = item.id();
                    let n = match old.remove(&key) {
                        Some(n) => n,
                        None => {
                            // Signals the row's closure makes belong to
                            // the row, as those its views make do.
                            let owner = Owner::new();
                            let mut n = owner.with(|| AnyView::new(row(item)).build_node(&mut Cx::new(container.clone())));
                            n.owner = Some(owner);
                            n.meta.set_parent(&meta);
                            inherit_state(&meta, &n);
                            n
                        }
                    };
                    new_keys.push(key);
                    new_nodes.push(n);
                }
                drop(old); // teardown of removed items
                *keys.borrow_mut() = new_keys;
                *children.borrow_mut() = new_nodes;
                meta.mark();
            });
        }));
        node
    }))
}
