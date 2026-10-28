//! A real `NSTableView` (in its `NSScrollView`) backed by a typed Rust model.
//!
//! AppKit pulls cell values lazily, during its own layout and scrolling, so
//! the data lives in the model rather than in the host's view code. The model
//! is a cheap handle (`Clone`, main thread only); the native view is created
//! once and updated in place, and removed when the last handle is dropped.
//!
//! User actions (selection, activation, sorting, checkbox toggles, column
//! moves and resizes) are queued on the model and announced with
//! [`crate::request_redraw`]; the host's table widget drains them.

use std::cell::{Cell, Ref, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSEvent,
    NSLineBreakMode, NSResponder, NSScrollView, NSTableColumn, NSTableColumnResizingOptions, NSTableView,
    NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle, NSTextAlignment, NSTextField,
    NSUserInterfaceItemIdentification, NSView,
};
use objc2_foundation::{NSArray, NSIndexSet, NSInteger, NSMutableIndexSet, NSNotification, NSSortDescriptor, NSString};

use crate::ViewId;
use crate::host::mtm;

/// A cell's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Text, shown as is and sorted in natural order.
    Text(String),
    /// A number, right-aligned and sorted numerically.
    Number(f64),
    /// A checkbox.
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Number { decimals: usize },
    Check,
}

/// A column: an id (stable, used for sorting and saved state), a title, and
/// how to get the cell value from a row.
pub struct Column<T> {
    id: String,
    title: String,
    width: f64,
    min_width: f64,
    kind: Kind,
    sortable: bool,
    value: Box<dyn Fn(&T) -> Value>,
}

impl<T> Column<T> {
    fn new(id: &str, title: &str, kind: Kind, value: Box<dyn Fn(&T) -> Value>) -> Self {
        Self { id: id.into(), title: title.into(), width: 120.0, min_width: 40.0, kind, sortable: true, value }
    }

    /// A text column.
    pub fn text(id: &str, title: &str, value: impl Fn(&T) -> String + 'static) -> Self {
        Self::new(id, title, Kind::Text, Box::new(move |row| Value::Text(value(row))))
    }

    /// A right-aligned number column, sorted numerically.
    pub fn number(id: &str, title: &str, value: impl Fn(&T) -> f64 + 'static) -> Self {
        Self::new(id, title, Kind::Number { decimals: 0 }, Box::new(move |row| Value::Number(value(row))))
    }

    /// A checkbox column. Clicking a checkbox emits [`TableEvent::Toggle`];
    /// the model doesn't change the row itself.
    pub fn check(id: &str, title: &str, value: impl Fn(&T) -> bool + 'static) -> Self {
        Self::new(id, title, Kind::Check, Box::new(move |row| Value::Bool(value(row)))).width(60.0)
    }

    /// Initial width in points (default 120).
    pub fn width(mut self, width: f64) -> Self {
        self.width = width;
        self
    }

    /// Smallest width the user can drag the column to.
    pub fn min_width(mut self, min_width: f64) -> Self {
        self.min_width = min_width;
        self
    }

    /// Decimal places shown for a number column (default 0).
    pub fn decimals(mut self, decimals: usize) -> Self {
        if let Kind::Number { .. } = self.kind {
            self.kind = Kind::Number { decimals };
        }
        self
    }

    /// Whether a header click sorts by this column (default true).
    pub fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        self
    }
}

fn display(kind: Kind, value: &Value) -> String {
    match (kind, value) {
        (_, Value::Text(s)) => s.clone(),
        (Kind::Number { decimals }, Value::Number(n)) => format!("{n:.decimals$}"),
        (_, Value::Number(n)) => n.to_string(),
        (_, Value::Bool(b)) => if *b { "Yes".into() } else { "No".into() },
    }
}

/// The table's sort: a column id and a direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sort {
    /// The column id.
    pub column: String,
    /// Ascending (A→Z, 0→9) or descending.
    pub ascending: bool,
}

/// A column's place and width, as the user arranged them (for saving).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnState {
    /// The column id.
    pub id: String,
    /// Width in points.
    pub width: f64,
}

/// Something the user did in the table. Row indices are model indices
/// (positions in the rows given to [`TableModel::set_rows`]), whatever the sort.
#[derive(Debug, Clone, PartialEq)]
pub enum TableEvent {
    /// The selected rows changed.
    Selection(Vec<usize>),
    /// Double-click or Return on a row.
    Activate(usize),
    /// The user clicked a column header. With built-in sorting on (the
    /// default) the rows are already shown in the new order.
    Sort(Sort),
    /// A checkbox cell was clicked. The model doesn't change the row: apply
    /// it with [`TableModel::update_row`].
    Toggle {
        /// The row (model index).
        row: usize,
        /// The column id.
        column: String,
        /// The checkbox's new state.
        value: bool,
    },
    /// Columns were moved or resized; read [`TableModel::column_state`].
    ColumnsChanged,
}

/// Type-erased view of a model for the Objective-C data source.
trait Data {
    fn count(&self) -> usize;
    fn cell(&self, view_row: usize, column: &str) -> Option<(Kind, Value)>;
    fn title(&self, column: &str) -> Option<String>;
    fn model_row(&self, view_row: usize) -> Option<usize>;
    fn sort_changed(&self, sort: Option<Sort>);
    fn push(&self, event: TableEvent);
    fn quiet(&self) -> bool;
}

struct SourceIvars {
    data: RefCell<Option<Weak<dyn Data>>>,
    /// Cell views created (not reused), for diagnostics.
    created: Cell<usize>,
}

define_class!(
    // Data source, delegate and action target for one table.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitHostTableSource"]
    #[ivars = SourceIvars]
    struct Source;

    unsafe impl NSObjectProtocol for Source {}
    unsafe impl NSControlTextEditingDelegate for Source {}

    unsafe impl NSTableViewDataSource for Source {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            self.data().map_or(0, |d| d.count() as NSInteger)
        }

        #[unsafe(method(tableView:sortDescriptorsDidChange:))]
        fn sort_descriptors_did_change(&self, table: &NSTableView, _old: &NSArray<NSSortDescriptor>) {
            let sort = table.sortDescriptors().firstObject().and_then(|d| {
                Some(Sort { column: d.key()?.to_string(), ascending: d.ascending() })
            });
            if let Some(data) = self.data() {
                data.sort_changed(sort);
            }
        }
    }

    unsafe impl NSTableViewDelegate for Source {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for(&self, table: &NSTableView, column: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSView>> {
            column.and_then(|column| self.cell_view(table, column, row))
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            let Some(data) = self.data() else { return };
            if data.quiet() {
                return;
            }
            let Some(table) = notification.object().and_then(|o| o.downcast::<NSTableView>().ok()) else { return };
            let rows = indexes(&table.selectedRowIndexes()).into_iter().filter_map(|r| data.model_row(r)).collect();
            data.push(TableEvent::Selection(rows));
        }

        #[unsafe(method_id(tableView:typeSelectStringForTableColumn:row:))]
        fn type_select_string(&self, _table: &NSTableView, column: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSString>> {
            self.cell_text(column, row)
        }

        #[unsafe(method(tableViewColumnDidMove:))]
        fn column_did_move(&self, _notification: &NSNotification) {
            if let Some(data) = self.data() {
                data.push(TableEvent::ColumnsChanged);
            }
        }

        #[unsafe(method(tableViewColumnDidResize:))]
        fn column_did_resize(&self, _notification: &NSNotification) {
            if let Some(data) = self.data() {
                data.push(TableEvent::ColumnsChanged);
            }
        }
    }

    impl Source {
        #[unsafe(method(activate:))]
        fn activate(&self, sender: &NSTableView) {
            let Some(data) = self.data() else { return };
            let row = sender.clickedRow();
            if let Some(row) = (row >= 0).then_some(row as usize).and_then(|r| data.model_row(r)) {
                data.push(TableEvent::Activate(row));
            }
        }

        #[unsafe(method(toggle:))]
        fn toggle(&self, sender: &NSButton) {
            let Some(data) = self.data() else { return };
            let Some(table) = table_of(sender) else { return };
            let row = table.rowForView(sender);
            let Some(column) = sender.identifier().map(|i| i.to_string()) else { return };
            if let Some(row) = (row >= 0).then_some(row as usize).and_then(|r| data.model_row(r)) {
                data.push(TableEvent::Toggle { row, column, value: sender.state() == NSControlStateValueOn });
            }
        }
    }
);

define_class!(
    // NSTableView that also activates the selected row on Return / Enter.
    #[unsafe(super(NSTableView, NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitHostTableView"]
    struct HostTableView;

    impl HostTableView {
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            // 36: Return, 76: keypad Enter.
            if matches!(event.keyCode(), 36 | 76) {
                let row = self.selectedRow();
                let source = unsafe { self.dataSource() };
                let source = source.and_then(|s| {
                    let any: &AnyObject = s.as_ref();
                    any.downcast_ref::<Source>().map(|s| s.retain())
                });
                if let (true, Some(source)) = (row >= 0, source) {
                    if let Some(data) = source.data()
                        && let Some(row) = data.model_row(row as usize)
                    {
                        data.push(TableEvent::Activate(row));
                    }
                    return;
                }
            }
            let () = unsafe { msg_send![super(self), keyDown: event] };
        }
    }
);

use objc2::Message as _;

fn table_of(view: &NSView) -> Option<Retained<NSTableView>> {
    // SAFETY: walking up a live view hierarchy on the main thread.
    let mut current = unsafe { view.superview() };
    while let Some(v) = current {
        if let Ok(table) = v.clone().downcast::<NSTableView>() {
            return Some(table);
        }
        current = unsafe { v.superview() };
    }
    None
}

fn indexes(set: &NSIndexSet) -> Vec<usize> {
    let mut out = Vec::with_capacity(set.count());
    let mut i = set.firstIndex();
    while i != usize::MAX && out.len() < set.count() {
        out.push(i);
        i = set.indexGreaterThanIndex(i);
    }
    out
}

impl Source {
    fn cell_text(&self, column: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSString>> {
        let (kind, value) = self.data()?.cell(row as usize, &column?.identifier().to_string())?;
        Some(NSString::from_str(&display(kind, &value)))
    }

    fn data(&self) -> Option<Rc<dyn Data>> {
        self.ivars().data.borrow().as_ref().and_then(Weak::upgrade)
    }

    fn cell_view(&self, table: &NSTableView, column: &NSTableColumn, row: NSInteger) -> Option<Retained<NSView>> {
        let ident = column.identifier();
        let (kind, value) = self.data()?.cell(row as usize, &ident.to_string())?;
        let mtm = MainThreadMarker::from(self);
        // SAFETY: reuse lookup by identifier; owner may be nil.
        let reused = unsafe { table.makeViewWithIdentifier_owner(&ident, None) };
        if kind == Kind::Check {
            let button = match reused.and_then(|v| v.downcast::<NSButton>().ok()) {
                Some(b) => b,
                None => {
                    self.ivars().created.set(self.ivars().created.get() + 1);
                    // SAFETY: target is this source, which outlives the table's cells.
                    let b = unsafe { NSButton::checkboxWithTitle_target_action(&NSString::new(), Some(self), Some(sel!(toggle:)), mtm) };
                    b.setIdentifier(Some(&ident));
                    // An untitled checkbox reads as just "checkbox": title it after
                    // its column and show only the box.
                    if let Some(title) = self.data().and_then(|d| d.title(&ident.to_string())) {
                        let title = NSString::from_str(&title);
                        b.setTitle(&title);
                        b.setImagePosition(objc2_app_kit::NSCellImagePosition::ImageOnly);
                        // Image-only buttons report an empty AX title; label
                        // the button and its cell (which serves accessibility).
                        // SAFETY: NSAccessibility setters taking a string.
                        unsafe {
                            let () = msg_send![&*b, setAccessibilityLabel: &*title];
                            let cell: Option<Retained<AnyObject>> = msg_send![&*b, cell];
                            if let Some(cell) = cell {
                                let () = msg_send![&*cell, setAccessibilityLabel: &*title];
                            }
                        }
                    }
                    b
                }
            };
            button.setState(if value == Value::Bool(true) { NSControlStateValueOn } else { NSControlStateValueOff });
            return Some(Retained::into_super(Retained::into_super(button)));
        }
        let field = match reused.and_then(|v| v.downcast::<NSTextField>().ok()) {
            Some(f) => f,
            None => {
                self.ivars().created.set(self.ivars().created.get() + 1);
                let f = NSTextField::labelWithString(&NSString::new(), mtm);
                f.setIdentifier(Some(&ident));
                f.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                if let Kind::Number { .. } = kind {
                    f.setAlignment(NSTextAlignment::Right);
                }
                f
            }
        };
        field.setStringValue(&NSString::from_str(&display(kind, &value)));
        Some(Retained::into_super(Retained::into_super(field)))
    }
}

struct Inner<T: 'static> {
    id: ViewId,
    columns: Vec<Column<T>>,
    rows: RefCell<Vec<T>>,
    /// View position → model index (identity when unsorted).
    order: RefCell<Vec<usize>>,
    sort: RefCell<Option<Sort>>,
    sorts_in_place: Cell<bool>,
    events: RefCell<VecDeque<TableEvent>>,
    /// Set while the model changes the selection itself (no event).
    quiet: Cell<bool>,
    mounted: Cell<bool>,
    scroll: Retained<NSScrollView>,
    table: Retained<HostTableView>,
    _source: Retained<Source>,
}

impl<T: 'static> Data for Inner<T> {
    fn count(&self) -> usize {
        self.order.borrow().len()
    }

    fn cell(&self, view_row: usize, column: &str) -> Option<(Kind, Value)> {
        let column = self.columns.iter().find(|c| c.id == column)?;
        let index = *self.order.borrow().get(view_row)?;
        let rows = self.rows.borrow();
        Some((column.kind, (column.value)(rows.get(index)?)))
    }

    fn model_row(&self, view_row: usize) -> Option<usize> {
        self.order.borrow().get(view_row).copied()
    }

    fn title(&self, column: &str) -> Option<String> {
        self.columns.iter().find(|c| c.id == column).map(|c| c.title.clone())
    }

    fn sort_changed(&self, sort: Option<Sort>) {
        *self.sort.borrow_mut() = sort.clone();
        if self.sorts_in_place.get() {
            self.resort_keeping_selection();
        }
        if let Some(sort) = sort {
            self.push(TableEvent::Sort(sort));
        }
    }

    fn push(&self, event: TableEvent) {
        let mut events = self.events.borrow_mut();
        // Column resizes arrive continuously while dragging: keep one.
        if event == TableEvent::ColumnsChanged && events.back() == Some(&TableEvent::ColumnsChanged) {
            return;
        }
        events.push_back(event);
        drop(events);
        crate::request_redraw();
    }

    fn quiet(&self) -> bool {
        self.quiet.get()
    }
}

impl<T: 'static> Inner<T> {
    /// Recomputes `order` from the current sort. Keys are computed once per
    /// row (not per comparison) and the sort is stable.
    fn compute_order(&self) {
        let rows = self.rows.borrow();
        let mut order: Vec<usize> = (0..rows.len()).collect();
        let sort = self.sort.borrow().clone();
        if let Some(column) = sort.as_ref().and_then(|s| self.columns.iter().find(|c| c.id == s.column)) {
            // One key per row, built once: text becomes a natural-order key
            // compared bytewise (comparing with case folding on every
            // comparison took 160 ms for 100k rows).
            enum Key {
                Number(f64),
                Text(String),
            }
            let keys: Vec<Key> = rows
                .iter()
                .map(|r| match (column.value)(r) {
                    Value::Number(n) => Key::Number(n),
                    Value::Bool(b) => Key::Number(b as u8 as f64),
                    Value::Text(t) => Key::Text(natural_key(&t)),
                })
                .collect();
            order.sort_by(|a, b| match (&keys[*a], &keys[*b]) {
                (Key::Number(x), Key::Number(y)) => x.total_cmp(y),
                (Key::Text(x), Key::Text(y)) => x.cmp(y),
                (Key::Number(_), Key::Text(_)) => std::cmp::Ordering::Less,
                (Key::Text(_), Key::Number(_)) => std::cmp::Ordering::Greater,
            });
            if sort.is_some_and(|s| !s.ascending) {
                order.reverse();
            }
        }
        *self.order.borrow_mut() = order;
    }

    fn selection_model_rows(&self) -> Vec<usize> {
        let order = self.order.borrow();
        indexes(&self.table.selectedRowIndexes()).into_iter().filter_map(|r| order.get(r).copied()).collect()
    }

    fn select_model_rows(&self, rows: &[usize]) {
        let order = self.order.borrow();
        let mut position = vec![usize::MAX; self.rows.borrow().len()];
        for (view, &model) in order.iter().enumerate() {
            position[model] = view;
        }
        drop(order);
        let set = NSMutableIndexSet::new();
        for &r in rows {
            if let Some(&p) = position.get(r).filter(|p| **p != usize::MAX) {
                set.addIndex(p);
            }
        }
        self.quiet.set(true);
        self.table.selectRowIndexes_byExtendingSelection(&set, false);
        self.quiet.set(false);
    }

    fn resort_keeping_selection(&self) {
        let selected = self.selection_model_rows();
        self.compute_order();
        self.table.reloadData();
        self.select_model_rows(&selected);
    }
}

impl<T: 'static> Drop for Inner<T> {
    fn drop(&mut self) {
        // AppKit holds these weakly: detach them before the source goes, in
        // case the table outlives the model (e.g. in an autorelease pool).
        // SAFETY: clearing to nil is always valid.
        unsafe {
            self.table.setDataSource(None);
            self.table.setDelegate(None);
            self.table.setTarget(None);
            self.table.setDoubleAction(None);
        }
        if self.mounted.get() {
            crate::unmount(self.id);
        }
        crate::testing::LIVE_MODELS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for SourceIvars {
    fn drop(&mut self) {
        crate::testing::LIVE_OBJC.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A sort key giving natural order under plain byte comparison:
/// case-insensitive, digit runs compared as numbers ("Item 9" < "Item 10"),
/// like the Finder. Digit runs become their length (as a char) then the
/// digits without leading zeros, so longer numbers sort later.
fn natural_key(s: &str) -> String {
    let mut key = String::with_capacity(s.len() + 4);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            let mut digits = String::from(c);
            while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                digits.push(*d);
                chars.next();
            }
            let trimmed = digits.trim_start_matches('0');
            // \u{1} sorts before any printable character.
            key.push('\u{1}');
            key.push(char::from_u32(0x30 + trimmed.len() as u32).unwrap_or('~'));
            key.push_str(trimmed);
        } else {
            key.extend(c.to_lowercase());
        }
    }
    key
}

/// Natural order for text (reference implementation for the key's tests).
#[cfg(test)]
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek(), b.peek()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.peek().filter(|c| c.is_ascii_digit()) {
                        n.push(*c);
                        it.next();
                    }
                    n
                };
                let (x, y) = (take(&mut a), take(&mut b));
                let (xt, yt) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let o = xt.len().cmp(&yt.len()).then_with(|| xt.cmp(yt));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// A native table backed by rows of `T`. See the module docs.
pub struct TableModel<T: 'static> {
    inner: Rc<Inner<T>>,
}

impl<T: 'static> Clone for TableModel<T> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<T: 'static> std::fmt::Debug for TableModel<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TableModel({:?}, {} rows)", self.inner.id, self.inner.rows.borrow().len())
    }
}

impl<T: 'static> TableModel<T> {
    /// Creates the native table (not yet in a window: it's added the first
    /// time the host draws it, see [`TableModel::ensure_mounted`]).
    pub fn new(id: ViewId, columns: Vec<Column<T>>) -> Self {
        let mtm = mtm();
        crate::testing::LIVE_OBJC.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let source = Source::alloc(mtm).set_ivars(SourceIvars { data: RefCell::new(None), created: Cell::new(0) });
        // SAFETY: plain NSObject init.
        let source: Retained<Source> = unsafe { msg_send![super(source), init] };

        let table: Retained<HostTableView> = unsafe { msg_send![HostTableView::alloc(mtm), init] };
        table.setStyle(NSTableViewStyle::FullWidth);
        table.setUsesAlternatingRowBackgroundColors(true);
        table.setAllowsMultipleSelection(true);
        table.setAllowsColumnReordering(true);
        table.setAllowsColumnResizing(true);
        for c in &columns {
            let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str(&c.id));
            column.setTitle(&NSString::from_str(&c.title));
            column.setWidth(c.width);
            column.setMinWidth(c.min_width);
            column.setResizingMask(NSTableColumnResizingOptions::UserResizingMask);
            if c.sortable {
                let descriptor = NSSortDescriptor::sortDescriptorWithKey_ascending(Some(&NSString::from_str(&c.id)), true);
                column.setSortDescriptorPrototype(Some(&descriptor));
            }
            table.addTableColumn(&column);
        }
        let scroll = NSScrollView::new(mtm);
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDocumentView(Some(&table));

        let inner = Rc::new(Inner {
            id,
            columns,
            rows: RefCell::new(Vec::new()),
            order: RefCell::new(Vec::new()),
            sort: RefCell::new(None),
            sorts_in_place: Cell::new(true),
            events: RefCell::new(VecDeque::new()),
            quiet: Cell::new(false),
            mounted: Cell::new(false),
            scroll,
            table: table.clone(),
            _source: source.clone(),
        });
        crate::testing::LIVE_MODELS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let weak: Weak<dyn Data> = Rc::downgrade(&inner) as Weak<dyn Data>;
        *source.ivars().data.borrow_mut() = Some(weak);
        // SAFETY: the source is kept alive by the model (`_source`), and the
        // model removes the table when dropped.
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(&*source)));
            table.setDelegate(Some(ProtocolObject::from_ref(&*source)));
            table.setTarget(Some(&source));
            table.setDoubleAction(Some(sel!(activate:)));
        }
        Self { inner }
    }

    /// The view id the table was created with.
    pub fn id(&self) -> ViewId {
        self.inner.id
    }

    /// Adds the table to the attached window if it isn't yet. Called by the
    /// host widget before placing it.
    pub fn ensure_mounted(&self) {
        if !self.inner.mounted.get() && crate::is_attached() {
            crate::mount(self.inner.id, Retained::into_super(self.inner.scroll.clone()));
            self.inner.mounted.set(true);
        }
    }

    /// Replaces all rows (the current sort is applied). Clears the selection.
    pub fn set_rows(&self, rows: Vec<T>) {
        *self.inner.rows.borrow_mut() = rows;
        self.inner.compute_order();
        self.inner.quiet.set(true);
        self.inner.table.reloadData();
        // SAFETY: nil sender.
        unsafe { self.inner.table.deselectAll(None) };
        self.inner.quiet.set(false);
    }

    /// The rows, in model order (unsorted).
    pub fn rows(&self) -> Ref<'_, Vec<T>> {
        self.inner.rows.borrow()
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.inner.rows.borrow().len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Replaces one row. Redraws only that row unless the table is sorted
    /// (then the order is recomputed; the selection is kept).
    pub fn update_row(&self, index: usize, row: T) {
        if let Some(slot) = self.inner.rows.borrow_mut().get_mut(index) {
            *slot = row;
        } else {
            return;
        }
        if self.inner.sort.borrow().is_some() && self.inner.sorts_in_place.get() {
            self.inner.resort_keeping_selection();
            return;
        }
        let Some(view_row) = self.inner.order.borrow().iter().position(|&m| m == index) else { return };
        let columns = NSIndexSet::indexSetWithIndexesInRange(objc2_foundation::NSRange::new(0, self.inner.columns.len()));
        self.inner.table.reloadDataForRowIndexes_columnIndexes(&NSIndexSet::indexSetWithIndex(view_row), &columns);
    }

    /// Inserts a row at a model index (the selection is kept).
    pub fn insert(&self, index: usize, row: T) {
        let selected = self.inner.selection_model_rows();
        self.inner.rows.borrow_mut().insert(index, row);
        let selected: Vec<usize> = selected.into_iter().map(|r| if r >= index { r + 1 } else { r }).collect();
        self.inner.compute_order();
        self.inner.table.reloadData();
        self.inner.select_model_rows(&selected);
    }

    /// Appends a row.
    pub fn push(&self, row: T) {
        let len = self.len();
        self.insert(len, row);
    }

    /// Removes a row (the selection is kept for the other rows).
    pub fn remove(&self, index: usize) -> Option<T> {
        if index >= self.len() {
            return None;
        }
        let selected = self.inner.selection_model_rows();
        let row = self.inner.rows.borrow_mut().remove(index);
        let selected: Vec<usize> =
            selected.into_iter().filter(|&r| r != index).map(|r| if r > index { r - 1 } else { r }).collect();
        self.inner.compute_order();
        self.inner.table.reloadData();
        self.inner.select_model_rows(&selected);
        Some(row)
    }

    /// The model index of the row shown at `position` (0 = top row as
    /// currently sorted).
    pub fn row_at(&self, position: usize) -> Option<usize> {
        self.inner.order.borrow().get(position).copied()
    }

    /// Selected rows (model indices, ascending in view order).
    pub fn selection(&self) -> Vec<usize> {
        self.inner.selection_model_rows()
    }

    /// Selects rows by model index (emits no event).
    pub fn select(&self, rows: &[usize]) {
        self.inner.select_model_rows(rows);
        if let Some(&first) = rows.first()
            && let Some(view_row) = self.inner.order.borrow().iter().position(|&m| m == first)
        {
            self.inner.table.scrollRowToVisible(view_row as NSInteger);
        }
    }

    /// The current sort, if any.
    pub fn sort(&self) -> Option<Sort> {
        self.inner.sort.borrow().clone()
    }

    /// Sets the sort (shown in the header) and, with built-in sorting, applies it.
    pub fn set_sort(&self, sort: Option<Sort>) {
        let descriptors = match &sort {
            Some(s) => NSArray::from_retained_slice(&[NSSortDescriptor::sortDescriptorWithKey_ascending(
                Some(&NSString::from_str(&s.column)),
                s.ascending,
            )]),
            None => NSArray::new(),
        };
        *self.inner.sort.borrow_mut() = sort;
        // setSortDescriptors calls back sortDescriptorsDidChange; quiet the event.
        let events_before = self.inner.events.borrow().len();
        self.inner.table.setSortDescriptors(&descriptors);
        self.inner.events.borrow_mut().truncate(events_before);
        if self.inner.sorts_in_place.get() {
            self.inner.resort_keeping_selection();
        }
    }

    /// Built-in sorting (default on): header clicks reorder the rows here.
    /// Turn it off to sort yourself on [`TableEvent::Sort`] (e.g. in a database)
    /// and call [`TableModel::set_rows`].
    pub fn set_sorts_in_place(&self, on: bool) {
        self.inner.sorts_in_place.set(on);
    }

    /// Columns in their current order, with widths.
    pub fn column_state(&self) -> Vec<ColumnState> {
        self.inner
            .table
            .tableColumns()
            .iter()
            .map(|c| ColumnState { id: c.identifier().to_string(), width: c.width() })
            .collect()
    }

    /// Restores a saved arrangement (unknown ids are ignored).
    pub fn set_column_state(&self, state: &[ColumnState]) {
        let table = &self.inner.table;
        for (target, s) in state.iter().enumerate() {
            let index = table.columnWithIdentifier(&NSString::from_str(&s.id));
            if index < 0 {
                continue;
            }
            if let Some(column) = table.tableColumns().iter().nth(index as usize) {
                column.setWidth(s.width);
            }
            if index as usize != target && target < table.numberOfColumns() as usize {
                table.moveColumn_toColumn(index, target as NSInteger);
            }
        }
        // moveColumn fires the delegate; this was the program, not the user.
        self.inner.events.borrow_mut().retain(|e| *e != TableEvent::ColumnsChanged);
    }

    /// Takes the queued user events (for the host widget).
    pub fn drain_events(&self) -> Vec<TableEvent> {
        self.inner.events.borrow_mut().drain(..).collect()
    }

    /// Diagnostics: cell views created so far (reuse keeps this near the
    /// number of visible cells).
    pub fn cells_created(&self) -> usize {
        self.inner._source.ivars().created.get()
    }

    /// The underlying `NSTableView` (escape hatch).
    pub fn table_view(&self) -> Retained<NSTableView> {
        Retained::into_super(self.inner.table.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::natural_cmp;
    use std::cmp::Ordering::*;

    #[test]
    fn key_matches_natural_order() {
        use super::natural_key;
        let words = ["Item 10", "item 9", "Item 009", "apple", "Banana", "x", "x1", "a2b", "a10b", "Zo", "zoe 2"];
        for a in words {
            for b in words {
                let by_key = natural_key(a).cmp(&natural_key(b));
                let by_cmp = natural_cmp(a, b);
                if by_cmp != std::cmp::Ordering::Equal {
                    assert_eq!(by_key, by_cmp, "{a:?} vs {b:?}");
                }
            }
        }
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("Item 9", "Item 10"), Less);
        assert_eq!(natural_cmp("apple", "Banana"), Less);
        assert_eq!(natural_cmp("a2b", "a02b"), Equal);
        assert_eq!(natural_cmp("x", "x1"), Less);
    }
}
