//! Views over `ferrokit-core`'s models: the native table and the text editor. The model owns the AppKit view and the data; the view places
//! it and turns the model's events into callbacks.

use ferrokit_core::kept::{UiCell, UiLocal};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use ferrokit_core::table::{ColumnState, Sort, TableEvent, TableModel};
use ferrokit_core::text::{TextEvent, TextModel, TextStyle};
use objc2::rc::Retained;

use crate::signal::{Effect, Signal, batch, untrack};
use crate::view::{Cx, Node, Size, View};

type Drain = Rc<dyn Fn()>;

thread_local! {
    /// Event drains of the models currently in a window (owned by their nodes).
    static DRAINS: UiCell<Vec<Weak<dyn Fn()>>> = const { UiCell::new(Vec::new()) };
}

/// Delivers queued model events (called on the run-loop pass after
/// `ferrokit-core` asks for a redraw).
/// Delivers editors' and tables' pending changes now: before a menu item,
/// a button or a command acts, so it sees what was just typed (typing and
/// a shortcut can arrive in the same run-loop turn, before the next pass
/// would deliver them).
pub(crate) fn flush() {
    if !DRAINS.with_borrow(Vec::is_empty) {
        drain_all();
    }
}

pub(crate) fn drain_all() {
    ferrokit_core::begin_frame();
    let live: Vec<Drain> = DRAINS.with_borrow_mut(|d| {
        d.retain(|w| w.strong_count() > 0);
        d.iter().filter_map(Weak::upgrade).collect()
    });
    batch(|| live.iter().for_each(|f| f()));
}

fn register(drain: &Drain) {
    DRAINS.with_borrow_mut(|d| d.push(Rc::downgrade(drain)));
}

type Callback<A> = Option<Box<dyn Fn(A)>>;
/// A row-menu item: its title and the action given the row.
type RowAction = (String, Box<dyn Fn(usize)>);

/// A native `NSTableView` showing a [`TableModel`]: millions of rows,
/// sorting, column resizing, type-select and VoiceOver come from AppKit.
pub struct Table<T: 'static> {
    model: Rc<TableModel<T>>,
    on_select: Callback<Vec<usize>>,
    on_activate: Callback<usize>,
    on_sort: Callback<Sort>,
    on_toggle: Callback<(usize, String, bool)>,
    on_edit: Callback<(usize, String, String)>,
    on_columns: Callback<Vec<ColumnState>>,
    on_move: Callback<(Vec<usize>, Option<usize>)>,
    on_files: Callback<(Vec<PathBuf>, Option<usize>)>,
    file_types: Vec<String>,
    on_paste: Callback<String>,
    row_drag: bool,
    row_menu: Vec<RowAction>,
}

impl<T: 'static> Table<T> {
    /// A right-click menu for rows: each action gets the clicked row (model
    /// index), or the selected row when invoked another way.
    pub fn row_menu(mut self, title: &str, action: impl Fn(usize) + 'static) -> Self {
        self.row_menu.push((title.into(), Box::new(action)));
        self
    }

    /// The selected rows (model indices) changed.
    pub fn on_select(mut self, f: impl Fn(Vec<usize>) + 'static) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }

    /// A row was double-clicked, or Return pressed on it.
    pub fn on_activate(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_activate = Some(Box::new(f));
        self
    }

    /// The user sorted by a column (rows are already shown in the new order).
    pub fn on_sort(mut self, f: impl Fn(Sort) + 'static) -> Self {
        self.on_sort = Some(Box::new(f));
        self
    }

    /// A checkbox cell was clicked: (row, column id, new value).
    pub fn on_toggle(mut self, f: impl Fn(usize, String, bool) + 'static) -> Self {
        self.on_toggle = Some(Box::new(move |(r, c, v)| f(r, c, v)));
        self
    }

    /// An editable cell was edited: (row, column id, new text). Apply it to
    /// the row with [`TableModel::update_row`].
    pub fn on_edit(mut self, f: impl Fn(usize, String, String) + 'static) -> Self {
        self.on_edit = Some(Box::new(move |(r, c, v)| f(r, c, v)));
        self
    }

    /// Rows can be dragged to other apps (as tab-separated text). ⌘C copies
    /// the selected rows the same way, always.
    pub fn row_drag(mut self) -> Self {
        self.row_drag = true;
        self
    }

    /// The user can drag rows to a new place: `f` gets the dragged rows and
    /// the row they were dropped above (`None`: the end), as model indices.
    /// Apply it with [`TableModel::move_rows`]. Not while the table is sorted.
    pub fn reorderable(mut self, f: impl Fn(Vec<usize>, Option<usize>) + 'static) -> Self {
        self.on_move = Some(Box::new(move |(rows, before)| f(rows, before)));
        self
    }

    /// Files with these extensions (any file if empty) can be dropped on the
    /// table: `f` gets their paths and the row they were dropped above.
    pub fn on_files_dropped(mut self, extensions: &[&str], f: impl Fn(Vec<PathBuf>, Option<usize>) + 'static) -> Self {
        self.file_types = extensions.iter().map(|e| e.to_string()).collect();
        self.on_files = Some(Box::new(move |(paths, before)| f(paths, before)));
        self
    }

    /// ⌘V on the table: `f` gets the pasted text (rows copied from a table
    /// arrive as tab-separated lines).
    pub fn on_paste(mut self, f: impl Fn(String) + 'static) -> Self {
        self.on_paste = Some(Box::new(f));
        self
    }

    /// Columns were moved or resized (save them to restore later).
    pub fn on_columns_changed(mut self, f: impl Fn(Vec<ColumnState>) + 'static) -> Self {
        self.on_columns = Some(Box::new(f));
        self
    }
}

impl<T: 'static> View for Table<T> {
    fn build(mut self, cx: &mut Cx) -> Node {
        let scroll = self.model.scroll_view();
        let (mut node, _) = Node::leaf(cx, Retained::into_super(scroll), (true, true), Size { w: 200.0, h: 120.0 });
        if let crate::view::Kind::Leaf(l) = &node.kind {
            // A scroll view has no useful fitting size: it takes what it's given.
            l.content.set(Some(Size { w: 200.0, h: 120.0 }));
        }
        if !self.row_menu.is_empty() {
            let tv = self.model.table_view();
            let items = std::mem::take(&mut self.row_menu)
                .into_iter()
                .map(|(title, action)| {
                    let (tv, model) = (tv.clone(), self.model.clone());
                    crate::chrome::menu_item(&title, move || {
                        let clicked = tv.clickedRow();
                        let view_row = if clicked >= 0 { clicked } else { tv.selectedRow() };
                        if let Some(row) = usize::try_from(view_row).ok().and_then(|r| model.row_at(r)) {
                            action(row);
                        }
                    })
                })
                .collect();
            let (menu, targets) = crate::chrome::build_menu(cx.mtm, items);
            // SAFETY: the node keeps the menu's targets alive.
            unsafe { tv.setMenu(Some(&menu)) };
            targets.into_iter().for_each(|t| node.keep(t));
        }
        if self.row_drag {
            self.model.set_row_drag(true);
        }
        if self.on_move.is_some() {
            self.model.set_reorderable(true);
        }
        if self.on_files.is_some() {
            let types: Vec<&str> = self.file_types.iter().map(String::as_str).collect();
            self.model.set_file_drop(Some(&types));
        }
        if self.on_paste.is_some() {
            self.model.set_pastes(true);
        }
        let model = self.model.clone();
        let Table { on_select, on_activate, on_sort, on_toggle, on_edit, on_columns, on_move, on_files, on_paste, .. } = self;
        let drain: Drain = Rc::new(move || {
            for event in model.drain_events() {
                match event {
                    TableEvent::Selection(rows) => on_select.iter().for_each(|f| f(rows.clone())),
                    TableEvent::Activate(row) => on_activate.iter().for_each(|f| f(row)),
                    TableEvent::Sort(sort) => on_sort.iter().for_each(|f| f(sort.clone())),
                    TableEvent::Toggle { row, column, value } => on_toggle.iter().for_each(|f| f((row, column.clone(), value))),
                    TableEvent::Edit { row, column, value } => on_edit.iter().for_each(|f| f((row, column.clone(), value.clone()))),
                    TableEvent::ColumnsChanged => on_columns.iter().for_each(|f| f(model.column_state())),
                    TableEvent::RowsMoved { rows, before } => on_move.iter().for_each(|f| f((rows.clone(), before))),
                    TableEvent::FilesDropped { paths, before } => on_files.iter().for_each(|f| f((paths.clone(), before))),
                    TableEvent::Paste(text) => on_paste.iter().for_each(|f| f(text.clone())),
                    _ => {}
                }
            }
        });
        register(&drain);
        node.hold(drain);
        node
    }
}

/// A native table showing `model` (see [`Table`]).
pub fn table<T: 'static>(model: &Rc<TableModel<T>>) -> Table<T> {
    Table {
        model: model.clone(),
        on_select: None,
        on_activate: None,
        on_sort: None,
        on_toggle: None,
        on_edit: None,
        on_columns: None,
        on_move: None,
        on_files: None,
        file_types: Vec::new(),
        on_paste: None,
        row_drag: false,
        row_menu: Vec::new(),
    }
}

/// A native plain-text editor (`NSTextView` in a scroll view) showing a
/// [`TextModel`]: input methods, spell checking, find, undo, text services
/// and VoiceOver come from AppKit.
pub struct TextEditor {
    model: TextModel,
    bound: Option<Signal<String>>,
    on_change: Callback<String>,
    on_selection: Callback<std::ops::Range<usize>>,
    on_composition: Callback<Option<String>>,
    shared_undo: bool,
    highlight: Option<Rc<Highlight>>,
    code: bool,
    rich: Option<Rich>,
}

/// A rich text editor's settings (see [`TextEditor::rich`]).
#[derive(Default)]
struct Rich {
    ruler: bool,
    format_bar: bool,
    rtf: Option<Signal<Vec<u8>>>,
}

type Highlight = dyn Fn(&str) -> Vec<(std::ops::Range<usize>, TextStyle)>;

impl TextEditor {
    /// Records edits in the window's undo history, with its text fields and
    /// app-level changes ([`undoable`](crate::undoable)), instead of the
    /// editor's own. Then ⌘Z anywhere in the window undoes the latest
    /// change, wherever it was made.
    pub fn shared_undo(mut self) -> Self {
        self.shared_undo = true;
        self
    }

    /// Binds the text two ways to `text`: edits set it (not while an input
    /// method is composing); setting it replaces the editor's text (clearing
    /// undo, like [`TextModel::set_text`]) unless it already matches.
    pub fn bind(mut self, text: &Signal<String>) -> Self {
        self.bound = Some(*text);
        self
    }

    /// The text changed (typing, paste, undo, a committed composition).
    pub fn on_change(mut self, f: impl Fn(String) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// The selection or caret moved (UTF-8 byte range).
    pub fn on_selection(mut self, f: impl Fn(std::ops::Range<usize>) + 'static) -> Self {
        self.on_selection = Some(Box::new(f));
        self
    }

    /// An input method is composing (`Some(marked text)`) or stopped (`None`).
    pub fn on_composition(mut self, f: impl Fn(Option<String>) + 'static) -> Self {
        self.on_composition = Some(Box::new(f));
        self
    }

    /// Styles the text: `f` gets the text and returns runs (UTF-8 byte
    /// ranges) with their [`TextStyle`]s. It runs after every edit, before
    /// it's drawn, and again when signals it reads change (a search term to
    /// mark, say). See [`TextModel::set_highlighter`].
    pub fn highlight(mut self, f: impl Fn(&str) -> Vec<(std::ops::Range<usize>, TextStyle)> + 'static) -> Self {
        self.highlight = Some(Rc::new(f));
        self
    }

    /// Syntax highlighting with `syntax` (e.g. [`Syntax::sql`](crate::Syntax::sql)),
    /// for code: see [`code`](Self::code).
    pub fn syntax(self, syntax: crate::Syntax) -> Self {
        self.code().highlight(move |text| syntax.highlight(text))
    }

    /// Rich text: the user styles it (fonts, bold, colours, alignment,
    /// lists, pictures) with ⌘B and friends, the font panel and
    /// [`format_menu`](crate::format_menu); see [`TextModel::set_rich`].
    /// Read and load it as RTF or HTML on the model, or bind it with
    /// [`bind_rtf`](Self::bind_rtf).
    pub fn rich(mut self) -> Self {
        self.rich.get_or_insert_with(Rich::default);
        self
    }

    /// Rich text with the ruler shown (margins, tabs, alignment, lists).
    pub fn ruler(mut self) -> Self {
        self.rich.get_or_insert_with(Rich::default).ruler = true;
        self
    }

    /// Rich text with TextEdit's format bar above it (font, size, colours,
    /// bold, italic, alignment, spacing, lists).
    pub fn format_bar(mut self) -> Self {
        self.rich.get_or_insert_with(Rich::default).format_bar = true;
        self
    }

    /// Rich text bound two ways to `rtf`, its RTF (what a rich document
    /// saves): edits, styling ones too, set it; setting it loads it
    /// (clearing undo) unless it's what the editor shows.
    pub fn bind_rtf(mut self, rtf: &Signal<Vec<u8>>) -> Self {
        self.rich.get_or_insert_with(Rich::default).rtf = Some(*rtf);
        self
    }

    /// An editor for code: quotes and dashes stay as typed (no smart
    /// quotes), and no text replacement, autocorrection or spell checking
    /// ([`TextModel::set_code_input`]). [`syntax`](Self::syntax) implies it.
    pub fn code(mut self) -> Self {
        self.code = true;
        self
    }
}

impl View for TextEditor {
    fn build(self, cx: &mut Cx) -> Node {
        let scroll = self.model.scroll_view();
        let (mut node, _) = Node::leaf(cx, Retained::into_super(scroll), (true, true), Size { w: 200.0, h: 80.0 });
        if let crate::view::Kind::Leaf(l) = &node.kind {
            l.content.set(Some(Size { w: 200.0, h: 80.0 }));
        }
        if self.shared_undo {
            self.model.share_undo(crate::undo::manager_for(&cx.container).as_deref());
        }
        if self.code {
            self.model.set_code_input(true);
        }
        let rtf = self.rich.as_ref().and_then(|r| r.rtf);
        if let Some(rich) = &self.rich {
            self.model.set_rich(true);
            self.model.set_ruler_visible(rich.ruler);
            self.model.set_format_bar(rich.format_bar);
        }
        if let Some(rtf) = rtf {
            // Signal → editor (skipped when it's what the editor shows).
            let model = self.model.clone();
            node.own(Effect::new(move || {
                let want = rtf.get();
                untrack(|| {
                    if model.rtf() != want && !model.set_rtf(&want) && want.is_empty() {
                        model.set_text("");
                    }
                });
            }));
        }
        if let Some(f) = self.highlight.clone() {
            let g = f.clone();
            self.model.set_highlighter(Some(move |text: &str| g(text)));
            // Restyle when signals the highlighter reads change.
            let model = self.model.clone();
            node.own(Effect::new(move || {
                let _ = f(&untrack(|| model.text()));
                untrack(|| model.restyle());
            }));
        }
        if let Some(bound) = &self.bound {
            // Signal → editor (skipped when equal: the editor is the source).
            let (bound, model) = (*bound, self.model.clone());
            node.own(Effect::new(move || {
                let want = bound.get();
                untrack(|| {
                    if model.text() != want {
                        model.set_text(&want);
                    }
                });
            }));
        }
        let model = self.model.clone();
        let TextEditor { bound, on_change, on_selection, on_composition, .. } = self;
        let drain: Drain = Rc::new(move || {
            for event in model.drain_events() {
                match event {
                    TextEvent::Changed => {
                        if let Some(r) = &rtf {
                            r.set_if_changed(model.rtf());
                        }
                        let text = model.text();
                        if let Some(b) = &bound {
                            b.set_if_changed(text.clone());
                        }
                        on_change.iter().for_each(|f| f(text.clone()));
                    }
                    TextEvent::Selection(r) => on_selection.iter().for_each(|f| f(r.clone())),
                    TextEvent::Composition(c) => on_composition.iter().for_each(|f| f(c.clone())),
                }
            }
        });
        register(&drain);
        node.hold(drain);
        node
    }
}

/// A native text editor showing `model` (see [`TextEditor`]).
pub fn text_editor(model: &TextModel) -> TextEditor {
    TextEditor { model: model.clone(), bound: None, on_change: None, on_selection: None, on_composition: None, shared_undo: false, highlight: None, code: false, rich: None }
}
