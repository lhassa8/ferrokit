//! A native grid of items (`NSCollectionView`): galleries, file browsers,
//! dashboards. Only the items on screen have views; cells are made once
//! and refilled as they scroll in, so a grid of 100,000 items costs about
//! as much as the screenful you see.
//!
//! Each item shows a fixed template: an image (an SF Symbol, a file's icon
//! or a colour swatch), a title and an optional subtitle. Items are keyed by
//! id, so the selection follows them when the list changes.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSIndexPathNSCollectionViewAdditions,
    NSBox, NSBoxType, NSCollectionView, NSCollectionViewDataSource, NSCollectionViewDelegate, NSCollectionViewFlowLayout, NSCollectionViewItem,
    NSColor, NSEvent, NSImage, NSImageScaling, NSImageView, NSScrollView, NSTextAlignment, NSTextField, NSView, NSViewController, NSWorkspace,
};
use objc2_foundation::{NSEdgeInsets, NSIndexPath, NSInteger, NSPoint, NSRect, NSSet, NSSize, NSString};

use crate::controls::Color;
use crate::signal::{Effect, Signal, batch, untrack};
use crate::view::{Cx, Identifiable, Kind, Node, Size, View};

/// What an item's image is.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemImage {
    /// An SF Symbol by name.
    Symbol(String),
    /// The Finder's icon for a file or folder.
    FileIcon(PathBuf),
    /// A swatch of a colour.
    Color(Color),
    /// No image.
    None,
}

/// What a collection shows for one item.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectionItem {
    /// The name under the image (and what VoiceOver reads).
    pub title: String,
    /// A second, smaller line.
    pub subtitle: Option<String>,
    /// The image.
    pub image: ItemImage,
}

impl CollectionItem {
    /// An item with a title and an image.
    pub fn new(title: impl Into<String>, image: ItemImage) -> Self {
        CollectionItem { title: title.into(), subtitle: None, image }
    }

    /// Adds a second line.
    pub fn subtitle(mut self, text: impl Into<String>) -> Self {
        self.subtitle = Some(text.into());
        self
    }
}

#[derive(Default)]
struct State {
    entries: Vec<CollectionItem>,
    /// Model indices → (selected now) called with the selected indices.
    on_select: Option<Box<dyn Fn(Vec<usize>)>>,
    on_activate: Option<Box<dyn Fn(usize)>>,
    /// Set while the collection changes its selection itself.
    quiet: bool,
}

thread_local! {
    /// Cells made so far, over all collections (a diagnostic: they're reused).
    static CELLS_MADE: Cell<u64> = const { Cell::new(0) };
}

/// Collection cells made since launch: stays near a screenful however many
/// items scroll past (diagnostic).
pub fn collection_cells_made() -> u64 {
    CELLS_MADE.with(Cell::get)
}

const CELL_ID: &str = "appkit-ui.cell";

struct CellParts {
    highlight: Retained<NSBox>,
    swatch: Retained<NSBox>,
    image: Retained<NSImageView>,
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
    /// The item's model index, and where to send double-clicks.
    index: Cell<usize>,
    state: RefCell<Weak<RefCell<State>>>,
}

define_class!(
    // A cell's view: lays out its parts and turns double-clicks into
    // activation.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiCellView"]
    #[ivars = CellParts]
    struct CellView;

    impl CellView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let p = self.ivars();
            let b = self.bounds().size;
            let line = 17.0;
            let lines = if p.subtitle.isHidden() { 1.0 } else { 2.0 };
            let image_h = (b.height - 8.0 - lines * line - 4.0).max(0.0);
            p.highlight.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), b));
            let art = NSRect::new(NSPoint::new(6.0, 4.0), NSSize::new((b.width - 12.0).max(0.0), image_h));
            p.image.setFrame(art);
            p.swatch.setFrame(art);
            p.title.setFrame(NSRect::new(NSPoint::new(2.0, 4.0 + image_h + 2.0), NSSize::new((b.width - 4.0).max(0.0), line)));
            p.subtitle.setFrame(NSRect::new(NSPoint::new(2.0, 4.0 + image_h + 2.0 + line), NSSize::new((b.width - 4.0).max(0.0), line)));
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            // SAFETY: the superclass handles the click (selection) first.
            let () = unsafe { msg_send![super(self), mouseDown: event] };
            if event.clickCount() == 2 {
                activate(&self.ivars().state.borrow(), self.ivars().index.get());
            }
        }
    }
);

fn activate(state: &Weak<RefCell<State>>, index: usize) {
    let Some(state) = state.upgrade() else { return };
    // Take the callback out while it runs: it may change the items.
    let f = state.borrow_mut().on_activate.take();
    if let Some(f) = &f {
        batch(|| f(index));
    }
    if state.borrow().on_activate.is_none() {
        state.borrow_mut().on_activate = f;
    }
}

fn label(mtm: MainThreadMarker, small: bool) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    l.setAlignment(NSTextAlignment::Center);
    l.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
    if small {
        l.setTextColor(Some(&NSColor::secondaryLabelColor()));
        l.setFont(Some(&objc2_app_kit::NSFont::systemFontOfSize(objc2_app_kit::NSFont::smallSystemFontSize())));
    }
    l
}

fn filled_box(mtm: MainThreadMarker, radius: f64) -> Retained<NSBox> {
    let b = NSBox::new(mtm);
    b.setBoxType(NSBoxType::Custom);
    b.setBorderWidth(0.0);
    b.setCornerRadius(radius);
    b.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
    b.setFillColor(&NSColor::clearColor());
    b
}

define_class!(
    // One reusable cell: built once in loadView, refilled per item.
    #[unsafe(super(NSCollectionViewItem, NSViewController))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiCollectionCell"]
    struct GridCell;

    impl GridCell {
        #[unsafe(method(loadView))]
        fn load_view(&self) {
            let mtm = MainThreadMarker::new().expect("main thread");
            CELLS_MADE.with(|c| c.set(c.get() + 1));
            let parts = CellParts {
                highlight: filled_box(mtm, 8.0),
                swatch: filled_box(mtm, 6.0),
                image: NSImageView::new(mtm),
                title: label(mtm, false),
                subtitle: label(mtm, true),
                index: Cell::new(0),
                state: RefCell::new(Weak::new()),
            };
            parts.image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            let view = CellView::alloc(mtm).set_ivars(parts);
            // SAFETY: NSView's designated initialiser.
            let view: Retained<CellView> = unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
            let p = view.ivars();
            for v in [&**p.highlight as &NSView, &p.swatch, &p.image, &p.title, &p.subtitle] {
                view.addSubview(v);
            }
            // VoiceOver reads the cell as one element named by its title.
            view.setAccessibilityElement(true);
            view.setAccessibilityRole(Some(unsafe { objc2_app_kit::NSAccessibilityImageRole }));
            // Its parts are hidden (on their cells too: macOS 27 reads
            // cell-based controls through the cell).
            for v in [&**p.image as &NSView, &p.title, &p.subtitle, &p.swatch, &p.highlight] {
                <crate::platform::Native as crate::platform::Platform>::set_accessibility_element(v, false);
            }
            self.setView(&view);
        }

        #[unsafe(method(setSelected:))]
        fn set_selected(&self, selected: bool) {
            // SAFETY: NSCollectionViewItem's own setter first.
            let () = unsafe { msg_send![super(self), setSelected: selected] };
            if let Some(v) = self.cell_view() {
                let fill = if selected { NSColor::selectedContentBackgroundColor() } else { NSColor::clearColor() };
                v.ivars().highlight.setFillColor(&fill);
                let text = if selected { NSColor::alternateSelectedControlTextColor() } else { NSColor::labelColor() };
                v.ivars().title.setTextColor(Some(&text));
            }
        }
    }
);

impl GridCell {
    fn cell_view(&self) -> Option<Retained<CellView>> {
        self.view().downcast::<CellView>().ok()
    }

    fn fill(&self, item: &CollectionItem, index: usize, state: &Weak<RefCell<State>>) {
        let Some(v) = self.cell_view() else { return };
        let p = v.ivars();
        p.index.set(index);
        *p.state.borrow_mut() = state.clone();
        p.title.setStringValue(&NSString::from_str(&item.title));
        p.subtitle.setStringValue(&NSString::from_str(item.subtitle.as_deref().unwrap_or("")));
        p.subtitle.setHidden(item.subtitle.is_none());
        let (image, swatch) = match &item.image {
            ItemImage::Symbol(name) => (NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None), None),
            ItemImage::FileIcon(path) => (Some(NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()))), None),
            ItemImage::Color(c) => (None, Some(NSColor::colorWithSRGBRed_green_blue_alpha(c.r, c.g, c.b, c.a))),
            ItemImage::None => (None, None),
        };
        p.image.setImage(image.as_deref());
        p.image.setHidden(image.is_none());
        p.swatch.setHidden(swatch.is_none());
        if let Some(c) = &swatch {
            p.swatch.setFillColor(c);
        }
        let name = match &item.subtitle {
            Some(s) => format!("{}, {s}", item.title),
            None => item.title.clone(),
        };
        v.setAccessibilityLabel(Some(&NSString::from_str(&name)));
        v.setNeedsLayout(true);
    }
}

define_class!(
    // The grid: Return activates the selected item.
    #[unsafe(super(NSCollectionView))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiCollectionView"]
    #[ivars = RefCell<Weak<RefCell<State>>>]
    struct GridView;

    impl GridView {
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let ret = matches!(event.keyCode(), 36 | 76);
            let first = self.selectionIndexPaths().iter().map(|p| p.item() as usize).min();
            match (ret, first) {
                (true, Some(i)) => activate(&self.ivars().borrow(), i),
                // SAFETY: the superclass handles every other key (arrows move the selection).
                _ => unsafe { msg_send![super(self), keyDown: event] },
            }
        }
    }
);

define_class!(
    // Data source and delegate of one grid.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiCollectionSource"]
    #[ivars = Weak<RefCell<State>>]
    struct GridSource;

    unsafe impl NSObjectProtocol for GridSource {}

    unsafe impl NSCollectionViewDataSource for GridSource {
        #[unsafe(method(collectionView:numberOfItemsInSection:))]
        fn count(&self, _cv: &NSCollectionView, _section: NSInteger) -> NSInteger {
            self.ivars().upgrade().map_or(0, |s| s.borrow().entries.len()) as NSInteger
        }

        #[unsafe(method_id(collectionView:itemForRepresentedObjectAtIndexPath:))]
        fn item(&self, cv: &NSCollectionView, path: &NSIndexPath) -> Retained<NSCollectionViewItem> {
            let made = cv.makeItemWithIdentifier_forIndexPath(&NSString::from_str(CELL_ID), path);
            if let (Ok(cell), Some(state)) = (made.clone().downcast::<GridCell>(), self.ivars().upgrade()) {
                let index = path.item() as usize;
                if let Some(entry) = state.borrow().entries.get(index) {
                    cell.fill(entry, index, self.ivars());
                }
            }
            made
        }
    }

    unsafe impl NSCollectionViewDelegate for GridSource {
        #[unsafe(method(collectionView:didSelectItemsAtIndexPaths:))]
        fn did_select(&self, cv: &NSCollectionView, _paths: &NSSet<NSIndexPath>) {
            self.selection_changed(cv);
        }

        #[unsafe(method(collectionView:didDeselectItemsAtIndexPaths:))]
        fn did_deselect(&self, cv: &NSCollectionView, _paths: &NSSet<NSIndexPath>) {
            self.selection_changed(cv);
        }
    }
);

impl GridSource {
    fn selection_changed(&self, cv: &NSCollectionView) {
        let Some(state) = self.ivars().upgrade() else { return };
        if state.borrow().quiet {
            return;
        }
        let mut rows: Vec<usize> = cv.selectionIndexPaths().iter().map(|p| p.item() as usize).collect();
        rows.sort_unstable();
        let s = state.borrow();
        if let Some(f) = &s.on_select {
            batch(|| f(rows));
        }
    }
}

type Describe<T> = Rc<dyn Fn(&T) -> CollectionItem>;

/// A grid of items (see [`collection`]).
pub struct Collection<T: Identifiable + 'static> {
    items: Box<dyn Fn() -> Vec<T>>,
    describe: Describe<T>,
    item_size: (f64, f64),
    selection: Option<Signal<Vec<T::Id>>>,
    multiple: bool,
    on_activate: Option<Rc<dyn Fn(T)>>,
}

impl<T: Identifiable + Clone + 'static> Collection<T> {
    /// The size of each item, in points (default 120 × 110).
    pub fn item_size(mut self, width: f64, height: f64) -> Self {
        self.item_size = (width, height);
        self
    }

    /// Two-way selection by item id (one item; see [`multiple`](Self::multiple)).
    pub fn selection(mut self, selected: &Signal<Vec<T::Id>>) -> Self {
        self.selection = Some(selected.clone());
        self
    }

    /// Lets the user select several items (⌘-click, ⇧-click, drag).
    pub fn multiple(mut self) -> Self {
        self.multiple = true;
        self
    }

    /// Runs `f` with an item when it's double-clicked, or Return is pressed
    /// on it.
    pub fn on_activate(mut self, f: impl Fn(T) + 'static) -> Self {
        self.on_activate = Some(Rc::new(f));
        self
    }
}

impl<T: Identifiable + Clone + 'static> View for Collection<T> {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        let state = Rc::new(RefCell::new(State::default()));
        let grid = GridView::alloc(mtm).set_ivars(RefCell::new(Rc::downgrade(&state)));
        // SAFETY: NSView's designated initialiser.
        let grid: Retained<GridView> = unsafe { msg_send![super(grid), initWithFrame: NSRect::ZERO] };
        let cv: Retained<NSCollectionView> = Retained::into_super(grid);
        let layout = NSCollectionViewFlowLayout::new(mtm);
        layout.setItemSize(NSSize::new(self.item_size.0, self.item_size.1));
        layout.setMinimumInteritemSpacing(8.0);
        layout.setMinimumLineSpacing(8.0);
        layout.setSectionInset(NSEdgeInsets { top: 12.0, left: 12.0, bottom: 12.0, right: 12.0 });
        cv.setCollectionViewLayout(Some(&layout));
        // SAFETY: GridCell is an NSCollectionViewItem subclass.
        unsafe { cv.registerClass_forItemWithIdentifier(Some(GridCell::class()), &NSString::from_str(CELL_ID)) };
        cv.setSelectable(true);
        cv.setAllowsMultipleSelection(self.multiple);
        cv.setAllowsEmptySelection(true);
        cv.setBackgroundColors(Some(&objc2_foundation::NSArray::from_retained_slice(&[NSColor::controlBackgroundColor()])));

        let source = GridSource::alloc(mtm).set_ivars(Rc::downgrade(&state));
        // SAFETY: NSObject's designated initialiser.
        let source: Retained<GridSource> = unsafe { msg_send![super(source), init] };
        // Data source and delegate are weak in AppKit; the node keeps the source.
        cv.setDataSource(Some(ProtocolObject::from_ref(&*source)));
        cv.setDelegate(Some(ProtocolObject::from_ref(&*source)));
        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&cv));
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);

        // The items (typed) and their ids, index-aligned with the entries.
        let values: Rc<RefCell<Vec<T>>> = Rc::default();
        if let Some(sel) = &self.selection {
            let (sel, values) = (sel.clone(), values.clone());
            state.borrow_mut().on_select = Some(Box::new(move |rows| {
                let v = values.borrow();
                sel.set(rows.iter().filter_map(|&i| v.get(i).map(Identifiable::id)).collect());
            }));
        }
        if let Some(f) = self.on_activate.clone() {
            let values = values.clone();
            state.borrow_mut().on_activate = Some(Box::new(move |i| {
                let item = values.borrow().get(i).cloned();
                if let Some(item) = item {
                    f(item);
                }
            }));
        }

        let (mut node, _) = Node::leaf(cx, Retained::into_super(scroll), (true, true), Size { w: 240.0, h: 200.0 });
        if let Kind::Leaf(l) = &node.kind {
            l.content.set(Some(Size { w: 240.0, h: 200.0 }));
        }

        // Selects the items whose ids the signal holds (quietly): when the
        // signal changes, and after a reload (which clears the selection).
        let restore: Rc<dyn Fn()> = {
            let (st, values, cv, sel) = (state.clone(), values.clone(), cv.clone(), self.selection.clone());
            Rc::new(move || {
                let Some(sel) = &sel else { return };
                let want = sel.get();
                untrack(|| {
                    let rows: Vec<usize> = values.borrow().iter().enumerate().filter(|(_, v)| want.contains(&v.id())).map(|(i, _)| i).collect();
                    let mut now: Vec<usize> = cv.selectionIndexPaths().iter().map(|p| p.item() as usize).collect();
                    now.sort_unstable();
                    if now != rows {
                        st.borrow_mut().quiet = true;
                        let paths: Vec<Retained<NSIndexPath>> = rows.iter().map(|&i| NSIndexPath::indexPathForItem_inSection(i as NSInteger, 0)).collect();
                        let set = NSSet::from_retained_slice(&paths);
                        cv.setSelectionIndexPaths(&set);
                        st.borrow_mut().quiet = false;
                    }
                });
            })
        };

        // Reload when the items' signals change.
        let (items_fn, describe) = (self.items, self.describe);
        let (st, vals, cv2, restore2) = (state.clone(), values.clone(), cv.clone(), restore.clone());
        node.own(Effect::new(move || {
            let items = items_fn();
            untrack(|| {
                let entries: Vec<CollectionItem> = items.iter().map(|i| describe(i)).collect();
                *vals.borrow_mut() = items;
                st.borrow_mut().entries = entries;
                st.borrow_mut().quiet = true;
                cv2.reloadData();
                st.borrow_mut().quiet = false;
                restore2();
            });
        }));
        if self.selection.is_some() {
            node.own(Effect::new(move || restore()));
        }
        node.keep(Retained::into_super(source));
        node.hold(state);
        node
    }
}

/// A native grid: `items` (reading signals) gives the items, `describe`
/// what each shows (see [`CollectionItem`]).
pub fn collection<T: Identifiable + Clone + 'static>(items: impl Fn() -> Vec<T> + 'static, describe: impl Fn(&T) -> CollectionItem + 'static) -> Collection<T> {
    Collection { items: Box::new(items), describe: Rc::new(describe), item_size: (120.0, 110.0), selection: None, multiple: false, on_activate: None }
}
