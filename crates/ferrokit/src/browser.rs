//! A column browser (`NSBrowser`), as Finder's column view: a tree shown
//! one level per column, the selected item's children in the next.

use std::cell::RefCell;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBrowser, NSBrowserColumnResizingType, NSBrowserDelegate, NSImage};
use objc2_foundation::{NSIndexPath, NSInteger, NSNumber, NSRect, NSString};

use crate::outline::TreeNode;
use crate::signal::{Effect, Signal, untrack};
use crate::view::{Cx, Identifiable, Node, Size, View};

/// An item as the delegate shows it; index 0 is the (unseen) root.
#[derive(Default)]
struct Entry {
    title: String,
    icon: Option<String>,
    children: Vec<usize>,
    /// Has children: shown with an arrow, opening a column.
    branch: bool,
}

define_class!(
    // The browser's item-based data source: items are their indices in
    // the flattened tree, as NSNumbers.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitBrowserDelegate"]
    #[ivars = RefCell<Vec<Entry>>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSBrowserDelegate for Delegate {
        #[unsafe(method_id(rootItemForBrowser:))]
        fn root(&self, _browser: &NSBrowser) -> Option<Retained<AnyObject>> {
            Some(item(0))
        }

        #[unsafe(method(browser:numberOfChildrenOfItem:))]
        fn count(&self, _browser: &NSBrowser, item: Option<&AnyObject>) -> NSInteger {
            self.ivars().borrow().get(index(item)).map_or(0, |e| e.children.len() as NSInteger)
        }

        #[unsafe(method_id(browser:child:ofItem:))]
        fn child(&self, _browser: &NSBrowser, i: NSInteger, of: Option<&AnyObject>) -> Retained<AnyObject> {
            let child = self.ivars().borrow().get(index(of)).and_then(|e| e.children.get(i as usize).copied()).unwrap_or(0);
            item(child)
        }

        #[unsafe(method(browser:isLeafItem:))]
        fn is_leaf(&self, _browser: &NSBrowser, item: Option<&AnyObject>) -> bool {
            !self.ivars().borrow().get(index(item)).is_some_and(|e| e.branch)
        }

        // The title, after the item's symbol (as a text attachment: the
        // browser's cells on macOS 27 are text cells, with no image).
        #[unsafe(method_id(browser:objectValueForItem:))]
        fn value(&self, _browser: &NSBrowser, item: Option<&AnyObject>) -> Option<Retained<AnyObject>> {
            let (title, icon) = self.ivars().borrow().get(index(item)).map(|e| (e.title.clone(), e.icon.clone())).unwrap_or_default();
            Some(match icon.and_then(|i| symbol(&i)) {
                Some(image) => Retained::into_super(Retained::into_super(titled(&image, &title))),
                None => Retained::into_super(Retained::into_super(NSString::from_str(&title))),
            })
        }

        // Typing selects by title, not by the symbol before it.
        #[unsafe(method_id(browser:typeSelectStringForRow:inColumn:))]
        fn type_select(&self, browser: &NSBrowser, row: NSInteger, column: NSInteger) -> Option<Retained<NSString>> {
            // SAFETY: NSBrowser's item for a shown row.
            let item: Option<Retained<AnyObject>> = unsafe { msg_send![browser, itemAtRow: row, inColumn: column] };
            let title = self.ivars().borrow().get(index(item.as_deref())).map(|e| e.title.clone()).unwrap_or_default();
            Some(NSString::from_str(&title))
        }
    }
);

/// What clicking and double-clicking do.
struct Actions {
    click: Box<dyn Fn()>,
    open: Option<Box<dyn Fn()>>,
}

define_class!(
    // The browser's target: its action (a click) and double action.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitBrowserTarget"]
    #[ivars = Actions]
    struct Target;

    impl Target {
        #[unsafe(method(click:))]
        fn click(&self, _sender: Option<&AnyObject>) {
            crate::failure::entry("a browser's click", (), || (self.ivars().click)());
        }

        #[unsafe(method(open:))]
        fn open(&self, _sender: Option<&AnyObject>) {
            if let Some(f) = &self.ivars().open {
                crate::failure::entry("a browser's double-click", (), f);
            }
        }
    }
);

/// The symbol `name`, centred in a box of one width, so the titles after
/// it line up (as Finder's icons do); a template, following Dark Mode.
fn symbol(name: &str) -> Option<Retained<NSImage>> {
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let sym = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
    let (w, h) = (20.0, 15.0);
    let s = sym.size();
    let scale = (h / s.height.max(1.0)).min(w / s.width.max(1.0)).min(1.0);
    let (dw, dh) = (s.width * scale, s.height * scale);
    let block = block2::RcBlock::new(move |_rect: NSRect| {
        sym.drawInRect(NSRect::new(NSPoint::new((w - dw) / 2.0, (h - dh) / 2.0), NSSize::new(dw, dh)));
        objc2::runtime::Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(w, h), false, &block);
    image.setTemplate(true);
    Some(image)
}

/// `image`, then `title`, in the browser's font.
fn titled(image: &NSImage, title: &str) -> Retained<objc2_foundation::NSAttributedString> {
    use objc2_app_kit::{NSAttributedStringAttachmentConveniences, NSFont, NSFontAttributeName, NSTextAttachment};
    let font = NSFont::systemFontOfSize(NSFont::systemFontSize());
    let attachment = NSTextAttachment::new();
    attachment.setImage(Some(image));
    // Down by the font's descent: centred on the text, not on its baseline.
    let size = image.size();
    attachment.setBounds(objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(0.0, font.descender() + 0.5), size));
    let out = objc2_foundation::NSMutableAttributedString::new();
    out.appendAttributedString(&objc2_foundation::NSAttributedString::attributedStringWithAttachment(&attachment));
    out.appendAttributedString(&objc2_foundation::NSAttributedString::from_nsstring(&NSString::from_str(&format!("  {title}"))));
    // SAFETY: the font attribute takes an NSFont.
    unsafe { out.addAttribute_value_range(NSFontAttributeName, &font, objc2_foundation::NSRange::new(0, out.length())) };
    Retained::into_super(out)
}

fn item(i: usize) -> Retained<AnyObject> {
    Retained::into_super(Retained::into_super(Retained::into_super(NSNumber::new_usize(i))))
}

fn index(item: Option<&AnyObject>) -> usize {
    item.and_then(|i| i.downcast_ref::<NSNumber>()).map_or(0, |n| n.as_usize())
}

type IconFn<T> = dyn Fn(&T) -> Option<String>;
type ActivateFn<T> = dyn Fn(&T);

/// A column browser (see [`column_browser`]).
pub struct ColumnBrowser<T: Identifiable + 'static> {
    roots: Box<dyn Fn() -> Vec<TreeNode<T>>>,
    title: Rc<dyn Fn(&T) -> String>,
    icon: Option<Rc<IconFn<T>>>,
    selection: Option<Signal<Vec<T::Id>>>,
    activate: Option<Rc<ActivateFn<T>>>,
    column_width: f64,
}

impl<T: Identifiable + 'static> ColumnBrowser<T> {
    /// An SF Symbol per item (or `None`).
    pub fn icon(mut self, f: impl Fn(&T) -> Option<String> + 'static) -> Self {
        self.icon = Some(Rc::new(f));
        self
    }

    /// The selected item's path, bound two ways: the ids from the first
    /// column's item to the selected one (empty for none). Setting it
    /// opens the columns down to that item.
    pub fn selection(mut self, path: &Signal<Vec<T::Id>>) -> Self {
        self.selection = Some(*path);
        self
    }

    /// Called with an item the user double-clicks (opening a file, say).
    pub fn on_activate(mut self, f: impl Fn(&T) + 'static) -> Self {
        self.activate = Some(Rc::new(f));
        self
    }

    /// The columns' width at first, in points (200 unless set); the user
    /// can resize them.
    pub fn column_width(mut self, points: f64) -> Self {
        self.column_width = points;
        self
    }
}

/// The flattened tree: entries for the delegate, and each one's value and
/// id hash for paths and callbacks.
struct Flat<T> {
    values: Vec<Option<T>>,
    keys: Vec<u64>,
}

fn key<I: Hash>(id: &I) -> u64 {
    let mut h = DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

fn flatten<T: Identifiable>(roots: Vec<TreeNode<T>>, title: &dyn Fn(&T) -> String, icon: Option<&IconFn<T>>) -> (Vec<Entry>, Flat<T>) {
    let mut entries = vec![Entry { branch: true, ..Entry::default() }];
    let mut flat = Flat { values: vec![None], keys: vec![0] };
    fn walk<T: Identifiable>(node: TreeNode<T>, parent: usize, entries: &mut Vec<Entry>, flat: &mut Flat<T>, title: &dyn Fn(&T) -> String, icon: Option<&IconFn<T>>) {
        let i = entries.len();
        entries.push(Entry { title: title(&node.value), icon: icon.and_then(|f| f(&node.value)), children: Vec::new(), branch: !node.children.is_empty() });
        flat.keys.push(key(&node.value.id()));
        flat.values.push(Some(node.value));
        entries[parent].children.push(i);
        for c in node.children {
            walk(c, i, entries, flat, title, icon);
        }
    }
    for r in roots {
        walk(r, 0, &mut entries, &mut flat, title, icon);
    }
    (entries, flat)
}

/// The rows from the first column down to the item `path` names, as far
/// as it's found.
fn rows_for(entries: &[Entry], keys: &[u64], path: &[u64]) -> Vec<usize> {
    let mut rows = Vec::new();
    let mut at = 0;
    for k in path {
        let Some((row, &child)) = entries[at].children.iter().enumerate().find(|(_, c)| keys[**c] == *k) else { break };
        rows.push(row);
        at = child;
    }
    rows
}

/// The entries along `rows` from the root.
fn entries_for(entries: &[Entry], rows: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = 0;
    for &r in rows {
        let Some(&c) = entries[at].children.get(r) else { break };
        out.push(c);
        at = c;
    }
    out
}

fn index_path(rows: &[usize]) -> Option<Retained<NSIndexPath>> {
    let (first, rest) = rows.split_first()?;
    Some(rest.iter().fold(NSIndexPath::indexPathWithIndex(*first), |p, r| p.indexPathByAddingIndex(*r)))
}

fn rows_of(path: Option<Retained<NSIndexPath>>) -> Vec<usize> {
    path.map(|p| (0..p.length()).map(|i| p.indexAtPosition(i)).collect()).unwrap_or_default()
}

impl<T: Identifiable + 'static> View for ColumnBrowser<T> {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        // SAFETY: NSView's designated initialiser.
        let browser: Retained<NSBrowser> = unsafe { msg_send![NSBrowser::alloc(mtm), initWithFrame: NSRect::ZERO] };
        browser.setTitled(false);
        browser.setSeparatesColumns(true);
        browser.setHasHorizontalScroller(true);
        browser.setAutohidesScroller(true);
        browser.setTakesTitleFromPreviousColumn(false);
        browser.setAllowsEmptySelection(true);
        browser.setColumnResizingType(NSBrowserColumnResizingType::UserColumnResizing);
        browser.setMinColumnWidth(self.column_width);
        let delegate = Delegate::alloc(mtm).set_ivars(RefCell::new(vec![Entry::default()]));
        // SAFETY: NSObject's designated initialiser.
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        // Weak in AppKit: the node keeps it.
        browser.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let view = Retained::into_super(Retained::into_super(browser.clone()));
        let (mut node, _) = Node::leaf(cx, view, (true, true), Size { w: self.column_width * 2.0, h: 160.0 });
        node.keep(Retained::into_super(delegate.clone()));

        let flat: Rc<RefCell<Flat<T>>> = Rc::new(RefCell::new(Flat { values: vec![None], keys: vec![0] }));
        let selection = self.selection;
        // The path shown now, as id hashes.
        let shown_keys = {
            let (b, d, f) = (browser.clone(), delegate.clone(), flat.clone());
            move || {
                let entries = d.ivars().borrow();
                let keys = &f.borrow().keys;
                entries_for(&entries, &rows_of(b.selectionIndexPath())).into_iter().map(|e| keys[e]).collect::<Vec<u64>>()
            }
        };
        let shown_keys = Rc::new(shown_keys);
        let select = {
            let (b, d, f) = (browser.clone(), delegate.clone(), flat.clone());
            move |path: &[u64]| {
                let rows = rows_for(&d.ivars().borrow(), &f.borrow().keys, path);
                // A column at a time: AppKit checks each row against the
                // columns as they are, so a column is chosen once the one
                // before has loaded it.
                if rows.is_empty() {
                    b.setSelectionIndexPath(None);
                }
                for n in 1..=rows.len() {
                    b.setSelectionIndexPath(index_path(&rows[..n]).as_deref());
                }
            }
        };
        let select = Rc::new(select);

        // The tree: flattened again when its signals change; the columns
        // reload, keeping the selected path where it still exists.
        let (roots, title, icon) = (self.roots, self.title, self.icon);
        let (b, d, f, keys_now, sel) = (browser.clone(), delegate.clone(), flat.clone(), shown_keys.clone(), select.clone());
        node.own(Effect::new(move || {
            let tree = roots();
            untrack(|| {
                let kept = keys_now();
                let (entries, values) = flatten(tree, &*title, icon.as_deref());
                *d.ivars().borrow_mut() = entries;
                *f.borrow_mut() = values;
                b.loadColumnZero();
                let want = match selection {
                    Some(s) => s.get().iter().map(key).collect(),
                    None => kept,
                };
                sel(&want);
            });
        }));

        // The user's clicks set the path; double-clicks activate.
        let (d, f, keys_now) = (delegate.clone(), flat.clone(), shown_keys.clone());
        let click = move || {
            let Some(s) = selection else { return };
            let picked = keys_now();
            let ids: Vec<T::Id> = {
                let entries = d.ivars().borrow();
                let flat = f.borrow();
                entries_for(&entries, &rows_for(&entries, &flat.keys, &picked)).into_iter().filter_map(|e| flat.values[e].as_ref().map(|v| v.id())).collect()
            };
            if untrack(|| s.with(|cur| cur.iter().map(key).collect::<Vec<_>>())) != picked {
                s.set(ids);
            }
        };
        let open = self.activate.map(|activate| {
            let (b, d, f) = (browser.clone(), delegate.clone(), flat.clone());
            Box::new(move || {
                let last = entries_for(&d.ivars().borrow(), &rows_of(b.selectionIndexPath())).last().copied();
                // Inside a batch: what `activate` changes takes effect after
                // this borrow ends.
                let flat = f.borrow();
                if let Some(Some(v)) = last.and_then(|e| flat.values.get(e)) {
                    activate(v);
                }
            }) as Box<dyn Fn()>
        });
        let has_open = open.is_some();
        let target = Target::alloc(mtm).set_ivars(Actions { click: Box::new(click), open });
        // SAFETY: NSObject's designated initialiser.
        let target: Retained<Target> = unsafe { msg_send![super(target), init] };
        // SAFETY: the target answers both selectors; the node keeps it.
        unsafe {
            browser.setTarget(Some(&target));
            browser.setAction(Some(objc2::sel!(click:)));
            if has_open {
                browser.setDoubleAction(Some(objc2::sel!(open:)));
            }
        }
        node.keep(Retained::into_super(target));
        // The path set from code opens those columns.
        if let Some(s) = selection {
            let (keys_now, sel) = (shown_keys, select);
            node.own(Effect::new(move || {
                let want: Vec<u64> = s.get().iter().map(key).collect();
                untrack(|| {
                    if keys_now() != want {
                        sel(&want);
                    }
                });
            }));
        }
        node
    }
}

/// A column browser, as Finder's column view: `roots` (reading signals)
/// gives the tree, `title` each item's text; the first column shows the
/// roots, and selecting an item with children shows them in the next.
/// Arrow keys move through it, type-select finds an item, and VoiceOver
/// reads it as a browser. Name it with
/// [`accessibility_label`](crate::ViewExt::accessibility_label).
///
/// ```no_run
/// use ferrokit::*;
///
/// #[derive(Clone)]
/// struct Place(String);
/// impl Identifiable for Place {
///     type Id = String;
///     fn id(&self) -> String { self.0.clone() }
/// }
/// let p = |s: &str| Place(s.into());
/// let path = Signal::new(Vec::<String>::new());
/// let places = column_browser(move || vec![
///     TreeNode::branch(p("Norway"), vec![TreeNode::branch(p("Lofoten"), vec![TreeNode::leaf(p("Reine")), TreeNode::leaf(p("Å"))])]),
///     TreeNode::leaf(p("Sweden")),
/// ], |place: &Place| place.0.clone())
/// .selection(&path)
/// .accessibility_label("Places");
/// # let _ = places;
/// ```
pub fn column_browser<T: Identifiable + 'static>(roots: impl Fn() -> Vec<TreeNode<T>> + 'static, title: impl Fn(&T) -> String + 'static) -> ColumnBrowser<T> {
    ColumnBrowser { roots: Box::new(roots), title: Rc::new(title), icon: None, selection: None, activate: None, column_width: 200.0 }
}
