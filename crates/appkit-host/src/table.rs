//! A real `NSTableView` (in its `NSScrollView`) backed by a typed Rust model.
//!
//! AppKit pulls cell values lazily, during its own layout and scrolling, so
//! the data lives in the model rather than in the host's view code. The model
//! is a cheap handle (`Clone`, main thread only); the native view is created
//! once and updated in place, and removed when the last handle is dropped.
//!
//! User actions (selection, activation, sorting, checkbox toggles, column
//! moves and resizes, drops, pastes) are queued on the model and announced
//! with [`crate::request_redraw`]; the host's table widget drains them.
//!
//! ⌘C copies the selected rows as tab-separated text. Dragging rows out,
//! reordering by drag, file drops and pasting are opt-in
//! ([`TableModel::set_row_drag`], [`TableModel::set_reorderable`],
//! [`TableModel::set_file_drop`], [`TableModel::set_pastes`]).

use std::cell::{Cell, Ref, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSDragOperation,
    NSDraggingInfo, NSEvent, NSLineBreakMode, NSMenuItem, NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL,
    NSPasteboardTypeString, NSPasteboardTypeTabularText, NSPasteboardWriting, NSResponder, NSScrollView, NSTableColumn,
    NSTableColumnResizingOptions, NSTableView, NSTableViewDataSource, NSTableViewDelegate, NSTableViewDropOperation,
    NSTableViewStyle, NSTextAlignment, NSTextField, NSUserInterfaceItemIdentification, NSView,
};
use objc2_foundation::{
    NSArray, NSIndexSet, NSInteger, NSMutableIndexSet, NSNotification, NSSortDescriptor, NSString, NSURL,
};

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
    /// A bar from 0 to 1 (the value is a number).
    Progress,
}

/// A small image shown before a cell's text (see [`Column::icon`]).
#[derive(Debug, Clone, PartialEq)]
pub enum CellImage {
    /// An SF Symbol by name.
    Symbol(String),
    /// The Finder's icon for a file or folder.
    FileIcon(std::path::PathBuf),
}

type IconFn<T> = Box<dyn Fn(&T) -> Option<CellImage>>;

/// A row's group (see [`TableModel::set_group_by`]).
pub type GroupKey<T> = Box<dyn Fn(&T) -> String>;

/// A column: an id (stable, used for sorting and saved state), a title, and
/// how to get the cell value from a row.
pub struct Column<T> {
    id: String,
    title: String,
    width: f64,
    min_width: f64,
    kind: Kind,
    sortable: bool,
    editable: bool,
    fit: bool,
    icon: Option<IconFn<T>>,
    value: Box<dyn Fn(&T) -> Value>,
}

impl<T> Column<T> {
    fn new(id: &str, title: &str, kind: Kind, value: Box<dyn Fn(&T) -> Value>) -> Self {
        Self { id: id.into(), title: title.into(), width: 120.0, min_width: 40.0, kind, sortable: true, editable: false, fit: false, icon: None, value }
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

    /// A progress bar from 0 to 1 (values outside are clamped). Sorted and
    /// copied as the number; VoiceOver reads it as a percentage.
    pub fn progress(id: &str, title: &str, value: impl Fn(&T) -> f64 + 'static) -> Self {
        Self::new(id, title, Kind::Progress, Box::new(move |row| Value::Number(value(row).clamp(0.0, 1.0)))).width(100.0)
    }

    /// Shows an image before each cell's text (a text or number column),
    /// e.g. a file's icon before its name. `None` leaves the space empty.
    pub fn icon(mut self, image: impl Fn(&T) -> Option<CellImage> + 'static) -> Self {
        self.icon = Some(Box::new(image));
        self
    }

    /// Sizes the column to its widest content (and its title) whenever the
    /// rows change. With many rows, the longest values are measured.
    pub fn fit(mut self) -> Self {
        self.fit = true;
        self
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

    /// Lets the user edit this column's text or number cells (double-click,
    /// or Return on the selected row's cell). Finishing an edit emits
    /// [`TableEvent::Edit`]; the model doesn't change the row: apply it with
    /// [`TableModel::update_row`]. No effect on checkbox columns.
    pub fn editable(mut self, editable: bool) -> Self {
        self.editable = editable;
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

/// Group header slots in `order` carry this bit (and the group's index).
const HEADER: usize = 1 << (usize::BITS - 1);

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
#[non_exhaustive]
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
    /// The user finished editing a cell of an [`Column::editable`] column.
    /// The model doesn't change the row: apply it with
    /// [`TableModel::update_row`].
    Edit {
        /// The row (model index).
        row: usize,
        /// The column id.
        column: String,
        /// The text the user entered.
        value: String,
    },
    /// The user dragged rows to a new place (see
    /// [`TableModel::set_reorderable`]). The model doesn't move them: apply
    /// it with [`TableModel::move_rows`].
    RowsMoved {
        /// The dragged rows (model indices).
        rows: Vec<usize>,
        /// The row they were dropped above (model index), or `None` for the end.
        before: Option<usize>,
    },
    /// Files were dropped on the table (see [`TableModel::set_file_drop`]).
    FilesDropped {
        /// The files' paths.
        paths: Vec<std::path::PathBuf>,
        /// The row they were dropped above (model index), or `None` for the end.
        before: Option<usize>,
    },
    /// The user pasted (⌘V) text (see [`TableModel::set_pastes`]); rows
    /// copied from a table arrive as tab-separated lines.
    Paste(String),
}

/// Which transfers a table takes part in (see the `set_…` methods on
/// [`TableModel`]).
#[derive(Debug, Clone, Default)]
struct Transfer {
    drag: bool,
    reorder: bool,
    /// File drops: `None` off; `Some(extensions)`, any file if empty.
    files: Option<Vec<String>>,
    pastes: bool,
}

/// The pasteboard type carrying a dragged row's model index (only read back
/// by the table it came from).
fn row_type() -> Retained<NSString> {
    NSString::from_str("dev.ferrokit.table-row")
}

/// Type-erased view of a model for the Objective-C data source.
trait Data {
    fn count(&self) -> usize;
    fn cell(&self, view_row: usize, column: &str) -> Option<(Kind, Value)>;
    fn editable(&self, column: &str) -> bool;
    fn title(&self, column: &str) -> Option<String>;
    fn model_row(&self, view_row: usize) -> Option<usize>;
    fn sort_changed(&self, sort: Option<Sort>);
    fn push(&self, event: TableEvent);
    fn quiet(&self) -> bool;
    fn transfer(&self) -> Transfer;
    fn sorted(&self) -> bool;
    /// Tab-separated text of rows (view positions), columns in `columns` order.
    fn rows_text(&self, view_rows: &[usize], columns: &[String]) -> String;
    /// The title of the group header shown at `view_row`, if it is one.
    fn group_title(&self, view_row: usize) -> Option<String>;
    /// The cell's image (an [`Column::icon`] column).
    fn icon(&self, view_row: usize, column: &str) -> Option<CellImage>;
    /// Whether the column has icons.
    fn has_icon(&self, column: &str) -> bool;
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

        #[unsafe(method_id(tableView:pasteboardWriterForRow:))]
        fn writer_for_row(&self, table: &NSTableView, row: NSInteger) -> Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>> {
            self.row_writer(table, row)
        }

        #[unsafe(method(tableView:validateDrop:proposedRow:proposedDropOperation:))]
        fn validate_drop(
            &self,
            table: &NSTableView,
            info: &ProtocolObject<dyn NSDraggingInfo>,
            row: NSInteger,
            _operation: NSTableViewDropOperation,
        ) -> NSDragOperation {
            let operation = match self.dropped(table, info) {
                Some(Dropped::Rows(_)) => NSDragOperation::Move,
                Some(Dropped::Files(_)) => NSDragOperation::Copy,
                None => return NSDragOperation::None,
            };
            // Between rows, never onto one.
            table.setDropRow_dropOperation(row, NSTableViewDropOperation::Above);
            operation
        }

        #[unsafe(method(tableView:acceptDrop:row:dropOperation:))]
        fn accept_drop(
            &self,
            table: &NSTableView,
            info: &ProtocolObject<dyn NSDraggingInfo>,
            row: NSInteger,
            _operation: NSTableViewDropOperation,
        ) -> bool {
            self.accept(table, info, row)
        }
    }

    unsafe impl NSTableViewDelegate for Source {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for(&self, table: &NSTableView, column: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSView>> {
            match column {
                Some(column) => self.cell_view(table, column, row),
                // A group row spans the table (no column).
                None => self.group_view(table, row),
            }
        }

        #[unsafe(method(tableView:isGroupRow:))]
        fn is_group_row(&self, _table: &NSTableView, row: NSInteger) -> bool {
            self.data().is_some_and(|d| d.group_title(row as usize).is_some())
        }

        #[unsafe(method(tableView:shouldSelectRow:))]
        fn should_select_row(&self, _table: &NSTableView, row: NSInteger) -> bool {
            self.data().is_none_or(|d| d.group_title(row as usize).is_none())
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

        #[unsafe(method(edit:))]
        fn edit(&self, sender: &NSTextField) {
            let Some(data) = self.data() else { return };
            let Some(table) = table_of(sender) else { return };
            let row = table.rowForView(sender);
            let Some(column) = sender.identifier().map(|i| i.to_string()) else { return };
            if let Some(row) = (row >= 0).then_some(row as usize).and_then(|r| data.model_row(r)) {
                data.push(TableEvent::Edit { row, column, value: sender.stringValue().to_string() });
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

/// What a drag carries that this table accepts.
enum Dropped {
    Rows(Vec<usize>),
    Files(Vec<std::path::PathBuf>),
}

define_class!(
    // NSTableView that also activates the selected row on Return / Enter, and
    // copies and pastes rows.
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

        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            self.copy_rows(&NSPasteboard::generalPasteboard());
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            self.paste_text(&NSPasteboard::generalPasteboard());
        }

        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let action = item.action();
            if action == Some(sel!(copy:)) {
                self.selectedRow() >= 0
            } else if action == Some(sel!(paste:)) {
                let pastes = self.source().and_then(|s| s.data()).is_some_and(|d| d.transfer().pastes);
                // SAFETY: AppKit's pasteboard type constant.
                pastes && NSPasteboard::generalPasteboard().stringForType(unsafe { NSPasteboardTypeString }).is_some()
            } else {
                true
            }
        }
    }
);

impl HostTableView {
    fn source(&self) -> Option<Retained<Source>> {
        let source = unsafe { self.dataSource() }?;
        let any: &AnyObject = source.as_ref();
        any.downcast_ref::<Source>().map(|s| s.retain())
    }

    /// Writes the selected rows to `pasteboard` as tab-separated text.
    fn copy_rows(&self, pasteboard: &NSPasteboard) -> bool {
        let Some(data) = self.source().and_then(|s| s.data()) else { return false };
        let rows = indexes(&self.selectedRowIndexes());
        if rows.is_empty() {
            return false;
        }
        let text = NSString::from_str(&data.rows_text(&rows, &column_ids(self)));
        pasteboard.clearContents();
        // SAFETY: AppKit's pasteboard type constants.
        unsafe {
            pasteboard.setString_forType(&text, NSPasteboardTypeString);
            pasteboard.setString_forType(&text, NSPasteboardTypeTabularText);
        }
        true
    }

    /// Reports `pasteboard`'s text as a paste, if the table takes pastes.
    fn paste_text(&self, pasteboard: &NSPasteboard) -> bool {
        let Some(data) = self.source().and_then(|s| s.data()) else { return false };
        // SAFETY: AppKit's pasteboard type constant.
        let text = pasteboard.stringForType(unsafe { NSPasteboardTypeString });
        match (data.transfer().pastes, text) {
            (true, Some(text)) => {
                data.push(TableEvent::Paste(text.to_string()));
                true
            }
            _ => false,
        }
    }
}

/// The table's column ids, in their current (user-arranged) order.
fn column_ids(table: &NSTableView) -> Vec<String> {
    table.tableColumns().iter().map(|c| c.identifier().to_string()).collect()
}

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

    fn row_writer(&self, table: &NSTableView, row: NSInteger) -> Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>> {
        let data = self.data()?;
        if !data.transfer().drag {
            return None;
        }
        let model = data.model_row(usize::try_from(row).ok()?)?;
        let item = NSPasteboardItem::new();
        item.setString_forType(&NSString::from_str(&model.to_string()), &row_type());
        let text = data.rows_text(&[row as usize], &column_ids(table));
        // SAFETY: AppKit's pasteboard type constant.
        item.setString_forType(&NSString::from_str(&text), unsafe { NSPasteboardTypeString });
        Some(ProtocolObject::from_retained(item))
    }

    fn accept(&self, table: &NSTableView, info: &ProtocolObject<dyn NSDraggingInfo>, row: NSInteger) -> bool {
        let Some(data) = self.data() else { return false };
        let before = usize::try_from(row).ok().and_then(|r| data.model_row(r));
        match self.dropped(table, info) {
            Some(Dropped::Rows(rows)) => data.push(TableEvent::RowsMoved { rows, before }),
            Some(Dropped::Files(paths)) => data.push(TableEvent::FilesDropped { paths, before }),
            None => return false,
        }
        true
    }

    /// What a drag over `table` carries that the table takes: its own rows
    /// (when reorderable and unsorted), or files (when it takes them).
    fn dropped(&self, table: &NSTableView, info: &ProtocolObject<dyn NSDraggingInfo>) -> Option<Dropped> {
        let data = self.data()?;
        let transfer = data.transfer();
        let items = info.draggingPasteboard().pasteboardItems()?;
        let own = info.draggingSource().is_some_and(|s| std::ptr::eq(&*s as *const AnyObject, table as *const NSTableView as *const AnyObject));
        if own {
            // Reordering a sorted table has no meaning: the sort decides.
            if !transfer.reorder || data.sorted() {
                return None;
            }
            let rows: Vec<usize> = items.iter().filter_map(|i| i.stringForType(&row_type())?.to_string().parse().ok()).collect();
            return (!rows.is_empty()).then_some(Dropped::Rows(rows));
        }
        let extensions = transfer.files?;
        let paths: Vec<std::path::PathBuf> = items
            .iter()
            // SAFETY: AppKit's pasteboard type constant.
            .filter_map(|i| i.stringForType(unsafe { NSPasteboardTypeFileURL }))
            .filter_map(|u| NSURL::URLWithString(&u)?.path().map(|p| std::path::PathBuf::from(p.to_string())))
            .collect();
        let accepted = |p: &std::path::PathBuf| {
            extensions.is_empty()
                || p.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
        };
        (!paths.is_empty() && paths.iter().all(accepted)).then_some(Dropped::Files(paths))
    }

    /// A group header: its title, in the table's group-row style.
    fn group_view(&self, table: &NSTableView, row: NSInteger) -> Option<Retained<NSView>> {
        let title = self.data()?.group_title(row as usize)?;
        let ident = NSString::from_str("appkit-host.group");
        // SAFETY: reuse lookup by identifier; owner may be nil.
        let field = match unsafe { table.makeViewWithIdentifier_owner(&ident, None) }.and_then(|v| v.downcast::<NSTextField>().ok()) {
            Some(f) => f,
            None => {
                let f = NSTextField::labelWithString(&NSString::new(), MainThreadMarker::from(self));
                f.setIdentifier(Some(&ident));
                f.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(objc2_app_kit::NSFont::smallSystemFontSize())));
                f.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
                f
            }
        };
        field.setStringValue(&NSString::from_str(&title));
        Some(Retained::into_super(Retained::into_super(field)))
    }

    fn progress_view(&self, table: &NSTableView, ident: &NSString, value: f64) -> Retained<NSView> {
        use objc2_app_kit::{NSProgressIndicator, NSProgressIndicatorStyle, NSTableCellView};
        let mtm = MainThreadMarker::from(self);
        // SAFETY: reuse lookup by identifier; owner may be nil.
        let cell = match unsafe { table.makeViewWithIdentifier_owner(ident, None) }.and_then(|v| v.downcast::<NSTableCellView>().ok()) {
            Some(c) => c,
            None => {
                self.ivars().created.set(self.ivars().created.get() + 1);
                let c = NSTableCellView::new(mtm);
                c.setIdentifier(Some(ident));
                let bar = NSProgressIndicator::new(mtm);
                bar.setStyle(NSProgressIndicatorStyle::Bar);
                bar.setIndeterminate(false);
                bar.setMinValue(0.0);
                bar.setMaxValue(1.0);
                bar.setControlSize(objc2_app_kit::NSControlSize::Small);
                bar.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable | objc2_app_kit::NSAutoresizingMaskOptions::ViewMinYMargin | objc2_app_kit::NSAutoresizingMaskOptions::ViewMaxYMargin);
                bar.setFrame(objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(2.0, 2.0), objc2_foundation::NSSize::new(96.0, 12.0)));
                c.addSubview(&bar);
                if let Some(title) = self.data().and_then(|d| d.title(&ident.to_string())) {
                    // SAFETY: NSAccessibility's label setter.
                    let () = unsafe { msg_send![&*bar, setAccessibilityLabel: &*NSString::from_str(&title)] };
                }
                c
            }
        };
        if let Some(bar) = cell.subviews().iter().find_map(|v| v.downcast::<objc2_app_kit::NSProgressIndicator>().ok()) {
            bar.setDoubleValue(value);
        }
        Retained::into_super(cell)
    }

    fn cell_view(&self, table: &NSTableView, column: &NSTableColumn, row: NSInteger) -> Option<Retained<NSView>> {
        let ident = column.identifier();
        let (kind, value) = self.data()?.cell(row as usize, &ident.to_string())?;
        let mtm = MainThreadMarker::from(self);
        if kind == Kind::Progress {
            let v = if let Value::Number(n) = value { n } else { 0.0 };
            return Some(self.progress_view(table, &ident, v));
        }
        if self.data().is_some_and(|d| d.has_icon(&ident.to_string())) {
            return Some(self.icon_view(table, &ident, kind, &value, row));
        }
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
                if self.data().is_some_and(|d| d.editable(&ident.to_string())) {
                    // A borderless editable label: NSTableView starts editing
                    // on double-click (or a click on the selected row), and
                    // the action fires when editing ends.
                    f.setEditable(true);
                    f.setSelectable(true);
                    // SAFETY: target is this source, which outlives the table's cells.
                    unsafe {
                        f.setTarget(Some(self));
                        f.setAction(Some(sel!(edit:)));
                    }
                }
                f
            }
        };
        // Set on reuse too: a column's kind can change (`set_columns`).
        field.setAlignment(if let Kind::Number { .. } = kind { NSTextAlignment::Right } else { NSTextAlignment::Natural });
        field.setStringValue(&NSString::from_str(&display(kind, &value)));
        Some(Retained::into_super(Retained::into_super(field)))
    }

    /// A cell with an image before its text (an icon column).
    fn icon_view(&self, table: &NSTableView, ident: &NSString, kind: Kind, value: &Value, row: NSInteger) -> Retained<NSView> {
        use objc2_app_kit::{NSImageView, NSTableCellView};
        let mtm = MainThreadMarker::from(self);
        // SAFETY: reuse lookup by identifier; owner may be nil.
        let cell = match unsafe { table.makeViewWithIdentifier_owner(ident, None) }.and_then(|v| v.downcast::<NSTableCellView>().ok()) {
            Some(c) => c,
            None => {
                self.ivars().created.set(self.ivars().created.get() + 1);
                let c = NSTableCellView::new(mtm);
                c.setIdentifier(Some(ident));
                let image = NSImageView::new(mtm);
                image.setFrame(objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(2.0, 0.0), objc2_foundation::NSSize::new(16.0, 16.0)));
                image.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewMinYMargin | objc2_app_kit::NSAutoresizingMaskOptions::ViewMaxYMargin);
                let text = NSTextField::labelWithString(&NSString::new(), mtm);
                text.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                text.setFrame(objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(22.0, 0.0), objc2_foundation::NSSize::new(100.0, 17.0)));
                text.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable | objc2_app_kit::NSAutoresizingMaskOptions::ViewMinYMargin | objc2_app_kit::NSAutoresizingMaskOptions::ViewMaxYMargin);
                if let Kind::Number { .. } = kind {
                    text.setAlignment(NSTextAlignment::Right);
                }
                c.addSubview(&image);
                c.addSubview(&text);
                // SAFETY: weak outlets; both views are the cell's subviews,
                // which keeps them alive.
                unsafe {
                    c.setImageView(Some(&image));
                    c.setTextField(Some(&text));
                }
                c
            }
        };
        let image = self.data().and_then(|d| d.icon(row as usize, &ident.to_string())).and_then(|i| match i {
            CellImage::Symbol(name) => objc2_app_kit::NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(&name), None),
            CellImage::FileIcon(path) => Some(objc2_app_kit::NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()))),
        });
        // SAFETY: the outlets set above (subviews of the cell).
        if let Some(iv) = unsafe { cell.imageView() } {
            iv.setImage(image.as_deref());
        }
        // SAFETY: as above.
        if let Some(tf) = unsafe { cell.textField() } {
            tf.setStringValue(&NSString::from_str(&display(kind, value)));
        }
        Retained::into_super(cell)
    }
}

struct Inner<T: 'static> {
    id: ViewId,
    columns: RefCell<Vec<Column<T>>>,
    rows: RefCell<Vec<T>>,
    /// View position → model index (identity when unsorted); a group
    /// header's slot is `HEADER | group`.
    order: RefCell<Vec<usize>>,
    /// Groups' titles (see `set_group_by`).
    groups: RefCell<Vec<String>>,
    group_by: RefCell<Option<GroupKey<T>>>,
    sort: RefCell<Option<Sort>>,
    sorts_in_place: Cell<bool>,
    events: RefCell<VecDeque<TableEvent>>,
    /// Set while the model changes the selection itself (no event).
    quiet: Cell<bool>,
    mounted: Cell<bool>,
    transfer: RefCell<Transfer>,
    scroll: Retained<NSScrollView>,
    table: Retained<HostTableView>,
    _source: Retained<Source>,
}

impl<T: 'static> Data for Inner<T> {
    fn count(&self) -> usize {
        self.order.borrow().len()
    }

    fn cell(&self, view_row: usize, column: &str) -> Option<(Kind, Value)> {
        let columns = self.columns.borrow();
        let column = columns.iter().find(|c| c.id == column)?;
        let index = self.model_row(view_row)?;
        let rows = self.rows.borrow();
        Some((column.kind, (column.value)(rows.get(index)?)))
    }

    fn model_row(&self, view_row: usize) -> Option<usize> {
        self.order.borrow().get(view_row).copied().filter(|m| m & HEADER == 0)
    }

    fn group_title(&self, view_row: usize) -> Option<String> {
        let slot = *self.order.borrow().get(view_row)?;
        (slot & HEADER != 0).then(|| self.groups.borrow().get(slot & !HEADER).cloned()).flatten()
    }

    fn icon(&self, view_row: usize, column: &str) -> Option<CellImage> {
        let columns = self.columns.borrow();
        let column = columns.iter().find(|c| c.id == column)?;
        let index = self.model_row(view_row)?;
        let rows = self.rows.borrow();
        column.icon.as_ref().and_then(|f| f(rows.get(index)?))
    }

    fn has_icon(&self, column: &str) -> bool {
        self.columns.borrow().iter().any(|c| c.id == column && c.icon.is_some())
    }

    fn editable(&self, column: &str) -> bool {
        self.columns.borrow().iter().any(|c| c.id == column && c.editable && c.kind != Kind::Check)
    }

    fn title(&self, column: &str) -> Option<String> {
        self.columns.borrow().iter().find(|c| c.id == column).map(|c| c.title.clone())
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

    fn transfer(&self) -> Transfer {
        self.transfer.borrow().clone()
    }

    fn sorted(&self) -> bool {
        self.sort.borrow().is_some()
    }

    fn rows_text(&self, view_rows: &[usize], columns: &[String]) -> String {
        let all = self.columns.borrow();
        let columns: Vec<&Column<T>> = columns.iter().filter_map(|id| all.iter().find(|c| &c.id == id)).collect();
        let order = self.order.borrow();
        let rows = self.rows.borrow();
        let clean = |s: String| s.replace(['\t', '\n', '\r'], " ");
        view_rows
            .iter()
            .filter_map(|&r| rows.get(*order.get(r)?))
            .map(|row| columns.iter().map(|c| clean(display(c.kind, &(c.value)(row)))).collect::<Vec<_>>().join("\t"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl<T: 'static> Inner<T> {
    /// Recomputes `order` from the current sort. Keys are computed once per
    /// row (not per comparison) and the sort is stable.
    fn compute_order(&self) {
        let rows = self.rows.borrow();
        let mut order: Vec<usize> = (0..rows.len()).collect();
        let sort = self.sort.borrow().clone();
        let columns = self.columns.borrow();
        if let Some(column) = sort.as_ref().and_then(|s| columns.iter().find(|c| c.id == s.column)) {
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
        // Groups: rows gathered by key (groups in order of first appearance
        // in the model, rows in the sort's order within each), each run
        // after a header slot.
        let mut groups = Vec::new();
        if let Some(key) = self.group_by.borrow().as_ref() {
            let keys: Vec<String> = rows.iter().map(key).collect();
            let mut index_of: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for k in &keys {
                if !index_of.contains_key(k.as_str()) {
                    index_of.insert(k, groups.len());
                    groups.push(k.clone());
                }
            }
            let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
            for &m in &order {
                buckets[index_of[keys[m].as_str()]].push(m);
            }
            order = buckets.into_iter().enumerate().flat_map(|(g, rows)| std::iter::once(HEADER | g).chain(rows)).collect();
        }
        drop(rows);
        *self.groups.borrow_mut() = groups;
        *self.order.borrow_mut() = order;
        self.fit_columns();
    }

    /// Sizes `fit` columns to their widest content: the title and, with many
    /// rows, the 64 longest values by character count (all of them below
    /// 2,000 rows), measured in the table's font.
    fn fit_columns(&self) {
        let rows = self.rows.borrow();
        for column in self.columns.borrow().iter().filter(|c| c.fit) {
            let Some(native) = self.table.tableColumnWithIdentifier(&NSString::from_str(&column.id)) else { continue };
            let mut texts: Vec<String> = rows.iter().map(|r| display(column.kind, &(column.value)(r))).collect();
            if texts.len() > 2_000 {
                texts.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
                texts.truncate(64);
            }
            let font = objc2_app_kit::NSFont::systemFontOfSize(objc2_app_kit::NSFont::systemFontSize());
            let attrs = objc2_foundation::NSDictionary::from_retained_objects(
                // SAFETY: AppKit's font attribute key.
                &[unsafe { objc2_app_kit::NSFontAttributeName }],
                &[Retained::into_super(Retained::into_super(font)) as Retained<AnyObject>],
            );
            // SAFETY: measuring a string with font attributes.
            let width = |s: &str| unsafe { objc2_app_kit::NSStringDrawing::sizeWithAttributes(&*NSString::from_str(s), Some(&attrs)).width };
            let widest = texts.iter().map(|t| width(t)).fold(width(&column.title) + 16.0, f64::max);
            let icon = if column.icon.is_some() { 22.0 } else { 0.0 };
            native.setWidth((widest + icon + 12.0).ceil().max(column.min_width));
        }
    }

    fn selection_model_rows(&self) -> Vec<usize> {
        let order = self.order.borrow();
        indexes(&self.table.selectedRowIndexes()).into_iter().filter_map(|r| order.get(r).copied()).filter(|m| m & HEADER == 0).collect()
    }

    fn select_model_rows(&self, rows: &[usize]) {
        let order = self.order.borrow();
        let mut position = vec![usize::MAX; self.rows.borrow().len()];
        for (view, &model) in order.iter().enumerate() {
            if model & HEADER == 0 {
                position[model] = view;
            }
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

/// Adds a native column for each of `columns`.
fn add_columns<T>(table: &NSTableView, columns: &[Column<T>]) {
    for c in columns {
        let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm()), &NSString::from_str(&c.id));
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
        add_columns(&table, &columns);
        let scroll = NSScrollView::new(mtm);
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDocumentView(Some(&table));

        let inner = Rc::new(Inner {
            id,
            columns: RefCell::new(columns),
            rows: RefCell::new(Vec::new()),
            order: RefCell::new(Vec::new()),
            groups: RefCell::new(Vec::new()),
            group_by: RefCell::new(None),
            sort: RefCell::new(None),
            sorts_in_place: Cell::new(true),
            events: RefCell::new(VecDeque::new()),
            quiet: Cell::new(false),
            mounted: Cell::new(false),
            transfer: RefCell::new(Transfer::default()),
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

    /// Replaces the columns, keeping the same native table (a query
    /// browser's results change shape with every query). The sort is
    /// cleared. Call [`set_rows`](Self::set_rows) after it when the rows
    /// change too: nothing is drawn in between.
    pub fn set_columns(&self, columns: Vec<Column<T>>) {
        let table = &self.inner.table;
        *self.inner.sort.borrow_mut() = None;
        self.inner.quiet.set(true);
        table.setSortDescriptors(&NSArray::new());
        let old: Vec<_> = table.tableColumns().iter().collect();
        for column in old {
            table.removeTableColumn(&column);
        }
        add_columns(table, &columns);
        *self.inner.columns.borrow_mut() = columns;
        self.inner.compute_order();
        table.reloadData();
        self.inner.quiet.set(false);
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
        let columns = NSIndexSet::indexSetWithIndexesInRange(objc2_foundation::NSRange::new(0, self.inner.columns.borrow().len()));
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

    /// Appends rows in one step, for live data (a log tail, a feed): when
    /// unsorted, the new rows are added to the end and AppKit only learns
    /// the new count (cost independent of the rows already there; the
    /// selection is kept). When sorted, the order is recomputed.
    pub fn append(&self, rows: Vec<T>) {
        if rows.is_empty() {
            return;
        }
        let start = self.len();
        self.inner.rows.borrow_mut().extend(rows);
        let grouped = self.inner.group_by.borrow().is_some();
        if grouped || (self.inner.sort.borrow().is_some() && self.inner.sorts_in_place.get()) {
            self.inner.resort_keeping_selection();
            return;
        }
        let end = self.len();
        self.inner.order.borrow_mut().extend(start..end);
        self.inner.table.noteNumberOfRowsChanged();
    }

    /// Scrolls so the last row is visible.
    pub fn scroll_to_end(&self) {
        let n = self.inner.order.borrow().len();
        if n > 0 {
            self.inner.table.scrollRowToVisible((n - 1) as NSInteger);
        }
    }

    /// Whether the table is scrolled to its last row (within one row), e.g.
    /// to keep following new rows only while the user hasn't scrolled up.
    pub fn is_at_end(&self) -> bool {
        let table = &self.inner.table;
        let visible = table.visibleRect();
        let bottom = visible.origin.y + visible.size.height;
        bottom >= table.bounds().size.height - table.rowHeight() - table.intercellSpacing().height - 1.0
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

    /// Moves `rows` (model indices) to just above the row `before` (or to
    /// the end), keeping their order, as a drop reported by
    /// [`TableEvent::RowsMoved`] asks. Returns their new model indices; they
    /// end up selected.
    pub fn move_rows(&self, rows: &[usize], before: Option<usize>) -> Vec<usize> {
        let mut moving: Vec<usize> = rows.iter().copied().filter(|&r| r < self.len()).collect();
        moving.sort_unstable();
        moving.dedup();
        let at = {
            let mut all = self.inner.rows.borrow_mut();
            let end = before.unwrap_or(all.len());
            // Rows staying put above the drop point.
            let at = (0..end.min(all.len())).filter(|r| moving.binary_search(r).is_err()).count();
            let mut taken = Vec::with_capacity(moving.len());
            for &r in moving.iter().rev() {
                taken.push(all.remove(r));
            }
            for (i, row) in taken.into_iter().rev().enumerate() {
                all.insert(at + i, row);
            }
            at
        };
        let moved: Vec<usize> = (at..at + moving.len()).collect();
        self.inner.compute_order();
        self.inner.table.reloadData();
        self.inner.select_model_rows(&moved);
        moved
    }

    /// Lets rows be dragged out: each carries its cells as tab-separated
    /// text (for other apps), and its model index for
    /// [`set_reorderable`](Self::set_reorderable). Off by default (a drag
    /// then extends the selection).
    pub fn set_row_drag(&self, on: bool) {
        self.inner.transfer.borrow_mut().drag = on;
        self.inner.table.setDraggingSourceOperationMask_forLocal(NSDragOperation::Copy, false);
        self.register_drag_types();
    }

    /// Lets the user drag rows to a new place (also turns on
    /// [`set_row_drag`](Self::set_row_drag)); each drop is reported as
    /// [`TableEvent::RowsMoved`]. Refused while the table is sorted.
    pub fn set_reorderable(&self, on: bool) {
        if on {
            self.set_row_drag(true);
            self.inner.table.setDraggingSourceOperationMask_forLocal(NSDragOperation::Move, true);
        }
        self.inner.transfer.borrow_mut().reorder = on;
        self.register_drag_types();
    }

    /// Accepts files dropped from the Finder: `Some(&["csv", "txt"])` for
    /// those extensions, `Some(&[])` for any file, `None` to stop. Each drop
    /// is reported as [`TableEvent::FilesDropped`].
    pub fn set_file_drop(&self, extensions: Option<&[&str]>) {
        self.inner.transfer.borrow_mut().files = extensions.map(|e| e.iter().map(|s| s.to_string()).collect());
        self.register_drag_types();
    }

    /// Reports ⌘V on the table as [`TableEvent::Paste`] (and enables Paste
    /// in the Edit menu while there is text to paste). Off by default.
    pub fn set_pastes(&self, on: bool) {
        self.inner.transfer.borrow_mut().pastes = on;
    }

    fn register_drag_types(&self) {
        let t = self.inner.transfer.borrow();
        let mut types = Vec::new();
        if t.reorder {
            types.push(row_type());
        }
        if t.files.is_some() {
            // SAFETY: AppKit's pasteboard type constant.
            types.push(NSString::from_str(&unsafe { NSPasteboardTypeFileURL }.to_string()));
        }
        self.inner.table.unregisterDraggedTypes();
        if !types.is_empty() {
            self.inner.table.registerForDraggedTypes(&NSArray::from_retained_slice(&types));
        }
    }

    /// The selected rows as tab-separated text (columns in their current
    /// order, one line per row, as ⌘C copies them).
    pub fn selected_text(&self) -> String {
        let rows = indexes(&self.inner.table.selectedRowIndexes());
        self.inner.rows_text(&rows, &column_ids(&self.inner.table))
    }

    /// Copies the selected rows to `pasteboard` (what ⌘C does with the
    /// general pasteboard). False if nothing is selected.
    pub fn copy_to(&self, pasteboard: &NSPasteboard) -> bool {
        self.inner.table.copy_rows(pasteboard)
    }

    /// Pastes `pasteboard`'s text (what ⌘V does with the general
    /// pasteboard): queues [`TableEvent::Paste`] if the table takes pastes.
    pub fn paste_from(&self, pasteboard: &NSPasteboard) -> bool {
        self.inner.table.paste_text(pasteboard)
    }

    /// The model index of the row shown at `position` (0 = top row as
    /// currently sorted); `None` past the end or for a group header.
    pub fn row_at(&self, position: usize) -> Option<usize> {
        self.inner.model_row(position)
    }

    /// Groups the rows under headers by `key` (e.g. a date or a category):
    /// groups appear in the order their first row has in the model, rows
    /// keep the sort's order within each group, and each group starts with
    /// a header row showing the key (full width, not selectable, floating
    /// while its rows scroll). `None` ungroups. Row indices in events stay
    /// model indices.
    pub fn set_group_by(&self, key: Option<GroupKey<T>>) {
        *self.inner.group_by.borrow_mut() = key;
        self.inner.table.setFloatsGroupRows(true);
        self.inner.resort_keeping_selection();
    }

    /// The groups' titles, in the order shown (empty when not grouped).
    pub fn groups(&self) -> Vec<String> {
        self.inner.groups.borrow().clone()
    }

    /// The number of rows shown, group headers included.
    pub fn shown_rows(&self) -> usize {
        self.inner.order.borrow().len()
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

    /// The `NSScrollView` holding the table, for hosts that place it
    /// themselves (no [`crate::attach`]ed window).
    pub fn scroll_view(&self) -> Retained<NSScrollView> {
        self.inner.scroll.clone()
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
