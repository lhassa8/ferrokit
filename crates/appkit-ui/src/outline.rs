//! A native outline (`NSOutlineView`) over a tree from signals, and the
//! source-list sidebar built on it.
//!
//! Each item gets a stable Objective-C object, reused by id when the tree
//! changes, so AppKit keeps expansion and selection across updates.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSControlTextEditingDelegate, NSImage, NSImageView, NSOutlineView, NSOutlineViewDataSource, NSOutlineViewDelegate,
    NSScrollView, NSTableCellView, NSTableColumn, NSTableViewStyle, NSTextField, NSUserInterfaceItemIdentification, NSView,
};
use objc2_foundation::{NSIndexSet, NSInteger, NSNotification, NSPoint, NSRect, NSSize, NSString};

use crate::signal::{Effect, Signal, batch, untrack};
use crate::view::{Cx, Identifiable, Kind, Node, Size, View};

/// One item of a tree, with its children.
#[derive(Debug, Clone)]
pub struct TreeNode<T> {
    /// The item.
    pub value: T,
    /// Its children (empty for a leaf).
    pub children: Vec<TreeNode<T>>,
}

impl<T> TreeNode<T> {
    /// A leaf.
    pub fn leaf(value: T) -> Self {
        TreeNode { value, children: Vec::new() }
    }

    /// An item with children.
    pub fn branch(value: T, children: Vec<TreeNode<T>>) -> Self {
        TreeNode { value, children }
    }
}

struct Entry {
    key: u64,
    title: String,
    icon: Option<String>,
    children: Vec<usize>,
    expandable: bool,
}

#[derive(Default)]
struct State {
    entries: Vec<Entry>,
    roots: Vec<usize>,
    /// The object AppKit sees for each entry (index-aligned).
    items: Vec<Retained<OutlineItem>>,
    /// Item objects by key, kept across rebuilds (stable identity).
    by_key: HashMap<u64, Retained<OutlineItem>>,
    group_roots: bool,
    on_select: Option<Box<dyn Fn(Option<usize>)>>,
    /// Set while the outline changes selection itself (no callback).
    quiet: bool,
}

define_class!(
    // AppKit's handle for one tree item: the index of its entry.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiOutlineItem"]
    #[ivars = std::cell::Cell<usize>]
    struct OutlineItem;
);

impl OutlineItem {
    fn new(mtm: MainThreadMarker, index: usize) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(std::cell::Cell::new(index));
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    // Data source and delegate for one outline view.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiOutlineSource"]
    #[ivars = RefCell<Weak<RefCell<State>>>]
    struct Source;

    unsafe impl NSObjectProtocol for Source {}
    unsafe impl NSControlTextEditingDelegate for Source {}

    unsafe impl NSOutlineViewDataSource for Source {
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        fn count(&self, _ov: &NSOutlineView, item: Option<&AnyObject>) -> NSInteger {
            self.with(|s| match index_of(item) {
                None => s.roots.len(),
                Some(i) => s.entries.get(i).map_or(0, |e| e.children.len()),
            }) as NSInteger
        }

        #[unsafe(method_id(outlineView:child:ofItem:))]
        fn child(&self, _ov: &NSOutlineView, index: NSInteger, item: Option<&AnyObject>) -> Retained<AnyObject> {
            let state = self.ivars().borrow().upgrade().expect("outline state outlives its view");
            let s = state.borrow();
            let list = match index_of(item) {
                None => &s.roots,
                Some(i) => &s.entries[i].children,
            };
            Retained::into_super(Retained::into_super(s.items[list[index as usize]].clone()))
        }

        #[unsafe(method(outlineView:isItemExpandable:))]
        fn expandable(&self, _ov: &NSOutlineView, item: &AnyObject) -> bool {
            self.with(|s| index_of(Some(item)).and_then(|i| s.entries.get(i)).is_some_and(|e| e.expandable))
        }
    }

    unsafe impl NSOutlineViewDelegate for Source {
        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        fn view_for(&self, ov: &NSOutlineView, _column: Option<&NSTableColumn>, item: &AnyObject) -> Option<Retained<NSView>> {
            let found = self.with(|s| {
                index_of(Some(item)).and_then(|i| {
                    s.entries.get(i).map(|e| (e.title.clone(), e.icon.clone(), s.group_roots && s.roots.contains(&i)))
                })
            });
            found.map(|(title, icon, group)| cell(ov, &title, icon.as_deref(), group))
        }

        #[unsafe(method(outlineView:isGroupItem:))]
        fn is_group(&self, _ov: &NSOutlineView, item: &AnyObject) -> bool {
            self.with(|s| s.group_roots && index_of(Some(item)).is_some_and(|i| s.roots.contains(&i)))
        }

        #[unsafe(method(outlineView:shouldSelectItem:))]
        fn should_select(&self, _ov: &NSOutlineView, item: &AnyObject) -> bool {
            // Section headers of a sidebar aren't selectable.
            !self.with(|s| s.group_roots && index_of(Some(item)).is_some_and(|i| s.roots.contains(&i)))
        }

        #[unsafe(method(outlineViewSelectionDidChange:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            let Some(ov) = notification.object().and_then(|o| o.downcast::<NSOutlineView>().ok()) else { return };
            let Some(state) = self.ivars().borrow().upgrade() else { return };
            if state.borrow().quiet {
                return;
            }
            let row = ov.selectedRow();
            let index = (row >= 0).then(|| ov.itemAtRow(row)).flatten().and_then(|i| index_of(Some(&i)));
            // Take the callback out while it runs: it may update the tree.
            let cb = state.borrow_mut().on_select.take();
            if let Some(cb) = &cb {
                batch(|| cb(index));
            }
            let mut s = state.borrow_mut();
            if s.on_select.is_none() {
                s.on_select = cb;
            }
        }
    }
);

impl Source {
    fn new(mtm: MainThreadMarker, state: &Rc<RefCell<State>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(Rc::downgrade(state)));
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }

    fn with<R: Default>(&self, f: impl FnOnce(&State) -> R) -> R {
        self.ivars().borrow().upgrade().map(|s| f(&s.borrow())).unwrap_or_default()
    }
}

fn index_of(item: Option<&AnyObject>) -> Option<usize> {
    item.and_then(|i| i.downcast_ref::<OutlineItem>()).map(|i| i.ivars().get())
}

/// A row: optional SF Symbol and a title (reused by identifier).
fn cell(ov: &NSOutlineView, title: &str, icon: Option<&str>, group: bool) -> Retained<NSView> {
    let mtm = MainThreadMarker::from(ov);
    let ident = NSString::from_str(if group { "group" } else if icon.is_some() { "icon" } else { "plain" });
    // SAFETY: reuse lookup by identifier; owner may be nil.
    let reused = unsafe { ov.makeViewWithIdentifier_owner(&ident, None) }.and_then(|v| v.downcast::<NSTableCellView>().ok());
    let view = reused.unwrap_or_else(|| {
        let v = NSTableCellView::new(mtm);
        v.setIdentifier(Some(&ident));
        let text = NSTextField::labelWithString(&NSString::new(), mtm);
        let x = if icon.is_some() { 24.0 } else { 2.0 };
        text.setFrame(NSRect::new(NSPoint::new(x, 2.0), NSSize::new(200.0, 18.0)));
        text.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable);
        v.addSubview(&text);
        // SAFETY: the cell view keeps its subviews; these are weak conveniences.
        unsafe { v.setTextField(Some(&text)) };
        if icon.is_some() {
            let image = NSImageView::new(mtm);
            image.setFrame(NSRect::new(NSPoint::new(2.0, 2.0), NSSize::new(18.0, 18.0)));
            v.addSubview(&image);
            // SAFETY: as above.
            unsafe { v.setImageView(Some(&image)) };
        }
        v
    });
    // SAFETY: reading the cell's outlets on the main thread.
    if let Some(t) = unsafe { view.textField() } {
        t.setStringValue(&NSString::from_str(title));
    }
    if let (Some(name), Some(iv)) = (icon, unsafe { view.imageView() }) {
        iv.setImage(NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None).as_deref());
    }
    Retained::into_super(view)
}

fn key_of<K: Hash>(id: &K) -> u64 {
    let mut h = DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

type Roots<T> = Box<dyn Fn() -> Vec<TreeNode<T>>>;
/// An item's SF Symbol name, if any.
type IconFn<T> = dyn Fn(&T) -> Option<String>;

/// A native outline view over a tree (see the module docs).
pub struct Outline<T: Identifiable + 'static> {
    roots: Roots<T>,
    title: Rc<dyn Fn(&T) -> String>,
    icon: Option<Rc<IconFn<T>>>,
    selection: Option<Signal<Option<T::Id>>>,
    sidebar: bool,
}

impl<T: Identifiable + 'static> Outline<T> {
    /// An SF Symbol per item (or `None`).
    pub fn icon(mut self, f: impl Fn(&T) -> Option<String> + 'static) -> Self {
        self.icon = Some(Rc::new(f));
        self
    }

    /// Two-way selection by item id.
    pub fn selection(mut self, selected: &Signal<Option<T::Id>>) -> Self {
        self.selection = Some(*selected);
        self
    }
}

impl<T: Identifiable + 'static> View for Outline<T> {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        let ov = NSOutlineView::new(mtm);
        let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str("title"));
        ov.addTableColumn(&column);
        // SAFETY: the column was just added to this outline.
        unsafe { ov.setOutlineTableColumn(Some(&column)) };
        ov.setHeaderView(None);
        if self.sidebar {
            ov.setStyle(NSTableViewStyle::SourceList);
            ov.setFloatsGroupRows(false);
        }
        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&ov));
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        if self.sidebar {
            scroll.setDrawsBackground(false);
        }

        let state = Rc::new(RefCell::new(State { group_roots: self.sidebar, ..State::default() }));
        let source = Source::new(mtm, &state);
        // SAFETY: data source and delegate are weak in AppKit; the node keeps the source.
        unsafe {
            ov.setDataSource(Some(ProtocolObject::from_ref(&*source)));
            ov.setDelegate(Some(ProtocolObject::from_ref(&*source)));
        }

        // Typed ids, index-aligned with the entries.
        let ids: Rc<RefCell<Vec<T::Id>>> = Rc::default();
        if let Some(sel) = &self.selection {
            let (sel, ids) = (*sel, ids.clone());
            state.borrow_mut().on_select = Some(Box::new(move |i| {
                sel.set(i.and_then(|i| ids.borrow().get(i).cloned()));
            }));
        }

        let (mut node, _) = Node::leaf(cx, Retained::into_super(scroll), (true, true), Size { w: 160.0, h: 120.0 });
        if let Kind::Leaf(l) = &node.kind {
            l.content.set(Some(Size { w: 160.0, h: 120.0 }));
        }

        // Selects the item whose id the selection signal holds (quietly);
        // used when the signal changes and after a reload, which clears
        // AppKit's selection.
        let restore: Rc<dyn Fn()> = {
            let (st, ids, ov3, sel) = (state.clone(), ids.clone(), ov.clone(), self.selection);
            Rc::new(move || {
                let Some(sel) = &sel else { return };
                let want = sel.get();
                untrack(|| {
                    let index = want.and_then(|id| ids.borrow().iter().position(|x| *x == id));
                    let item = index.and_then(|i| st.borrow().items.get(i).cloned());
                    // SAFETY: an item object this outline returned.
                    let row = item.map_or(-1, |it| unsafe { ov3.rowForItem(Some(&it)) });
                    if ov3.selectedRow() != row {
                        st.borrow_mut().quiet = true;
                        if row >= 0 {
                            ov3.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row as usize), false);
                        } else {
                            // SAFETY: nil sender.
                            unsafe { ov3.deselectAll(None) };
                        }
                        st.borrow_mut().quiet = false;
                    }
                });
            })
        };

        // Rebuild when the tree's signals change; keep item objects by id.
        let (roots_fn, title, icon, sidebar) = (self.roots, self.title, self.icon, self.sidebar);
        let (st, ids2, ov2, restore2) = (state.clone(), ids.clone(), ov.clone(), restore.clone());
        node.own(Effect::new(move || {
            let roots = roots_fn();
            untrack(|| {
                let mut s = st.borrow_mut();
                let old = std::mem::take(&mut s.by_key);
                let (mut entries, mut items, mut keys, mut typed) = (Vec::new(), Vec::new(), HashMap::new(), Vec::new());
                fn walk<T: Identifiable>(
                    n: TreeNode<T>,
                    out: &mut Vec<Entry>,
                    typed: &mut Vec<T::Id>,
                    title: &dyn Fn(&T) -> String,
                    icon: Option<&IconFn<T>>,
                ) -> usize {
                    let index = out.len();
                    let id = n.value.id();
                    out.push(Entry {
                        key: key_of(&id),
                        title: title(&n.value),
                        icon: icon.and_then(|f| f(&n.value)),
                        children: Vec::new(),
                        expandable: !n.children.is_empty(),
                    });
                    typed.push(id);
                    let kids: Vec<usize> = n.children.into_iter().map(|c| walk(c, out, typed, title, icon)).collect();
                    out[index].children = kids;
                    index
                }
                let root_idx: Vec<usize> =
                    roots.into_iter().map(|r| walk(r, &mut entries, &mut typed, &*title, icon.as_deref())).collect();
                for (i, e) in entries.iter().enumerate() {
                    let item = old.get(&e.key).cloned().unwrap_or_else(|| OutlineItem::new(mtm, i));
                    item.ivars().set(i);
                    keys.insert(e.key, item.clone());
                    items.push(item);
                }
                s.entries = entries;
                s.roots = root_idx;
                s.items = items;
                s.by_key = keys;
                *ids2.borrow_mut() = typed;
                s.quiet = true;
                drop(s);
                ov2.reloadData();
                if sidebar {
                    // Sections start expanded.
                    let roots: Vec<_> = st.borrow().roots.iter().map(|&i| st.borrow().items[i].clone()).collect();
                    for r in roots {
                        // SAFETY: an item object this outline returned.
                        unsafe { ov2.expandItem(Some(&r)) };
                    }
                }
                st.borrow_mut().quiet = false;
                untrack(|| restore2());
            });
        }));

        // Selection signal → outline (quietly).
        if self.selection.is_some() {
            node.own(Effect::new(move || restore()));
        }
        node.keep(Retained::into_super(source));
        node.hold(state);
        node
    }
}

/// A native outline: `roots` (reading signals) gives the tree, `title` each
/// item's text.
pub fn outline<T: Identifiable + 'static>(
    roots: impl Fn() -> Vec<TreeNode<T>> + 'static,
    title: impl Fn(&T) -> String + 'static,
) -> Outline<T> {
    Outline { roots: Box::new(roots), title: Rc::new(title), icon: None, selection: None, sidebar: false }
}

/// A Mac sidebar: an outline in the source-list style whose top-level items
/// are section headers, expanded.
pub fn sidebar<T: Identifiable + 'static>(
    sections: impl Fn() -> Vec<TreeNode<T>> + 'static,
    title: impl Fn(&T) -> String + 'static,
) -> Outline<T> {
    Outline { sidebar: true, ..outline(sections, title) }
}
