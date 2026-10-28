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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSButton, NSTextField, NSView};
use objc2_foundation::NSString;

use crate::signal::{Effect, batch, untrack};

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

    fn build_node(self, cx: &mut Cx) -> Node {
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
    pub(crate) container: Retained<NSView>,
    pub(crate) mtm: MainThreadMarker,
}

impl Cx {
    pub(crate) fn new(container: Retained<NSView>) -> Self {
        Cx { container, mtm: MainThreadMarker::new().expect("main thread") }
    }

    fn child(&self) -> Cx {
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
    /// Something in this subtree changed since the last layout.
    pub(crate) dirty: Cell<bool>,
}

impl Meta {
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
    pub(crate) view: Retained<NSView>,
    pub(crate) flex: (bool, bool),
    pub(crate) min: Size,
    /// Cached `fittingSize`.
    pub(crate) content: Rc<Cell<Option<Size>>>,
}

/// What an effect needs to report that a leaf's content changed.
pub(crate) struct LeafHandle {
    content: Rc<Cell<Option<Size>>>,
    meta: Rc<Meta>,
}

pub(crate) enum Kind {
    Leaf(Leaf),
    Stack { axis: Axis, spacing: f64, align: Option<Align>, children: Vec<Node> },
    Grid { spacing: (f64, f64), rows: Vec<Vec<Node>> },
    Spacer,
    /// Children that change over time; laid out inline in the parent stack.
    Group(Rc<RefCell<Vec<Node>>>),
}

/// A built view: native views, effects and helper objects it owns.
pub struct Node {
    pub(crate) kind: Kind,
    pub(crate) id: Option<u64>,
    pub(crate) padding: f64,
    pub(crate) fixed: (Option<f64>, Option<f64>),
    /// Forced flexibility (`fill_width` / `fill_height`).
    pub(crate) fill: (bool, bool),
    pub(crate) meta: Rc<Meta>,
    effects: Vec<Effect>,
    keep: Vec<Retained<NSObject>>,
}

impl Node {
    pub(crate) fn new(kind: Kind) -> Node {
        LIVE_NODES.with(|n| n.set(n.get() + 1));
        let meta = Rc::new(Meta::default());
        meta.dirty.set(true);
        let node = Node {
            kind,
            id: None,
            padding: 0.0,
            fixed: (None, None),
            fill: (false, false),
            meta,
            effects: Vec::new(),
            keep: Vec::new(),
        };
        for c in node.children_static() {
            c.meta.set_parent(&node.meta);
        }
        node
    }

    /// Children owned directly (not through a group).
    fn children_static(&self) -> Vec<&Node> {
        match &self.kind {
            Kind::Stack { children, .. } => children.iter().collect(),
            Kind::Grid { rows, .. } => rows.iter().flatten().collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn leaf(cx: &Cx, view: Retained<NSView>, flex: (bool, bool), min: Size) -> (Node, LeafHandle) {
        cx.container.addSubview(&view);
        let content = Rc::new(Cell::new(None));
        invalidate_layout();
        let node = Node::new(Kind::Leaf(Leaf { view, flex, min, content: content.clone() }));
        let handle = LeafHandle { content, meta: node.meta.clone() };
        (node, handle)
    }

    pub(crate) fn own(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    pub(crate) fn keep(&mut self, object: Retained<NSObject>) {
        self.keep.push(object);
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        LIVE_NODES.with(|n| n.set(n.get() - 1));
        // Effects stop first (dropping them), then the native view leaves.
        self.effects.clear();
        if let Kind::Leaf(l) = &self.kind {
            l.view.removeFromSuperview();
            if let Some(parent) = self.meta.parent.borrow().upgrade() {
                parent.mark();
            }
            invalidate_layout();
        }
    }
}

/// Modifiers every view has.
pub trait ViewExt: View + Sized {
    /// Inset on all sides, in points.
    fn padding(self, points: f64) -> Modified<Self> {
        Modified::new(self).padding(points)
    }

    /// A fixed width.
    fn width(self, points: f64) -> Modified<Self> {
        Modified::new(self).width(points)
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
}

impl<V: View> ViewExt for V {}

/// A view with modifiers applied (see [`ViewExt`]).
pub struct Modified<V> {
    view: V,
    padding: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    fill: (bool, bool),
}

impl<V: View> Modified<V> {
    fn new(view: V) -> Self {
        Modified { view, padding: None, width: None, height: None, fill: (false, false) }
    }

    /// Takes all the width its container offers.
    pub fn fill_width(mut self) -> Self {
        self.fill.0 = true;
        self
    }

    /// Takes all the height its container offers.
    pub fn fill_height(mut self) -> Self {
        self.fill.1 = true;
        self
    }

    /// Inset on all sides, in points.
    pub fn padding(mut self, points: f64) -> Self {
        self.padding = Some(points);
        self
    }

    /// A fixed width.
    pub fn width(mut self, points: f64) -> Self {
        self.width = Some(points);
        self
    }

    /// A fixed height.
    pub fn height(mut self, points: f64) -> Self {
        self.height = Some(points);
        self
    }
}

impl<V: View> View for Modified<V> {
    fn build(self, cx: &mut Cx) -> Node {
        let mut node = self.view.build(cx);
        if let Some(p) = self.padding {
            node.padding = p;
        }
        node.fixed = (self.width.or(node.fixed.0), self.height.or(node.fixed.1));
        node.fill = (node.fill.0 || self.fill.0, node.fill.1 || self.fill.1);
        node
    }
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

/// Rows of cells in aligned columns: each column is as wide as its widest
/// cell, each row as tall as its tallest. Columns (rows) with a flexible cell
/// share the extra space.
pub struct Grid {
    spacing: (f64, f64),
    rows: Vec<Vec<AnyView>>,
}

impl Grid {
    /// Space between columns and between rows (default 8, 8).
    pub fn spacing(mut self, columns: f64, rows: f64) -> Self {
        self.spacing = (columns, rows);
        self
    }
}

impl View for Grid {
    fn build(self, cx: &mut Cx) -> Node {
        let rows = self.rows.into_iter().map(|r| r.into_iter().map(|c| c.build_node(&mut cx.child())).collect()).collect();
        Node::new(Kind::Grid { spacing: self.spacing, rows })
    }
}

/// A grid from rows of views (rows may be shorter than others).
pub fn grid(rows: impl IntoIterator<Item = impl Views>) -> Grid {
    Grid { spacing: (8.0, 8.0), rows: rows.into_iter().map(Views::into_views).collect() }
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

/// After a content change: re-measure, and mark the leaf (and its
/// ancestors) for layout only if its size changed.
pub(crate) fn remeasure(view: &NSView, leaf: &LeafHandle) {
    let s = view.fittingSize();
    let new = Size { w: s.width, h: s.height };
    if leaf.content.get() != Some(new) {
        leaf.content.set(Some(new));
        leaf.meta.mark();
    }
}

/// A static label.
pub struct Text(String);

impl View for Text {
    fn build(self, cx: &mut Cx) -> Node {
        let field = NSTextField::labelWithString(&NSString::from_str(&self.0), cx.mtm);
        Node::leaf(cx, Retained::into_super(Retained::into_super(field)), (false, false), Size::default()).0
    }
}

/// A static label.
pub fn text(s: impl Into<String>) -> Text {
    Text(s.into())
}

/// A label computed from signals; updates in place.
pub struct Label(Box<dyn Fn() -> String>);

impl View for Label {
    fn build(self, cx: &mut Cx) -> Node {
        let field = NSTextField::labelWithString(&NSString::from_str(""), cx.mtm);
        let view: Retained<NSView> = Retained::into_super(Retained::into_super(field.clone()));
        let (mut node, handle) = Node::leaf(cx, view.clone(), (false, false), Size::default());
        let f = self.0;
        node.own(Effect::new(move || {
            field.setStringValue(&NSString::from_str(&f()));
            remeasure(&view, &handle);
        }));
        node
    }
}

/// A label computed from signals; updates in place.
pub fn label(f: impl Fn() -> String + 'static) -> Label {
    Label(Box::new(f))
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiAction"]
    #[ivars = Box<dyn Fn()>]
    pub(crate) struct ActionTarget;

    impl ActionTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, _sender: Option<&AnyObject>) {
            batch(|| (self.ivars())());
        }
    }
);

impl ActionTarget {
    pub(crate) fn new(mtm: MainThreadMarker, f: impl Fn() + 'static) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Box::new(f) as Box<dyn Fn()>);
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// A push button.
pub struct Button {
    title: String,
    action: Box<dyn Fn()>,
    disabled: Option<Box<dyn Fn() -> bool>>,
    default_action: bool,
}

impl Button {
    /// Disables the button while `f` (reading signals) returns true.
    pub fn disabled(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Some(Box::new(f));
        self
    }

    /// Makes this the window's default button: blue, and Return presses it.
    pub fn default_action(mut self) -> Self {
        self.default_action = true;
        self
    }
}

impl View for Button {
    fn build(self, cx: &mut Cx) -> Node {
        let target = ActionTarget::new(cx.mtm, self.action);
        // SAFETY: the target responds to `fire:` and the node keeps it alive.
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(&self.title), Some(&target), Some(sel!(fire:)), cx.mtm)
        };
        if self.default_action {
            button.setKeyEquivalent(&NSString::from_str("\r"));
        }
        let (mut node, _) =
            Node::leaf(cx, Retained::into_super(Retained::into_super(button.clone())), (false, false), Size::default());
        if let Some(disabled) = self.disabled {
            node.own(Effect::new(move || button.setEnabled(!disabled())));
        }
        node.keep(Retained::into_super(target));
        node
    }
}

/// A push button; `action` runs inside a batch.
pub fn button(title: &str, action: impl Fn() + 'static) -> Button {
    Button { title: title.into(), action: Box::new(action), disabled: None, default_action: false }
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
            let view = f();
            untrack(|| {
                let current = children.borrow().first().and_then(|n| n.id);
                if view.id.is_some() && view.id == current {
                    return;
                }
                let new = view.build_node(&mut Cx::new(container.clone()));
                new.meta.set_parent(&meta);
                // The old node (and its subtree) is dropped here: teardown.
                *children.borrow_mut() = vec![new];
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
                let mut old: HashMap<T::Id, Node> =
                    keys.borrow_mut().drain(..).zip(children.borrow_mut().drain(..)).collect();
                let mut new_keys = Vec::with_capacity(items.len());
                let mut new_nodes = Vec::with_capacity(items.len());
                for item in items {
                    let key = item.id();
                    let n = match old.remove(&key) {
                        Some(n) => n,
                        None => {
                            let n = AnyView::new(row(item)).build_node(&mut Cx::new(container.clone()));
                            n.meta.set_parent(&meta);
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
