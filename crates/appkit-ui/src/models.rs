//! Views over `appkit-host`'s models: the native table and the text editor. The model owns the AppKit view and the data; the view places
//! it and turns the model's events into callbacks.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use appkit_host::table::{ColumnState, Sort, TableEvent, TableModel};
use appkit_host::text::{TextEvent, TextModel};
use objc2::rc::Retained;

use crate::signal::{Effect, Signal, batch, untrack};
use crate::view::{Cx, Node, Size, View};

type Drain = Rc<dyn Fn()>;

thread_local! {
    /// Event drains of the models currently in a window (owned by their nodes).
    static DRAINS: RefCell<Vec<Weak<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
}

/// Delivers queued model events (called on the run-loop pass after
/// `appkit-host` asks for a redraw).
pub(crate) fn drain_all() {
    appkit_host::begin_frame();
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
}

impl<T: 'static> Table<T> {
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

    /// Columns were moved or resized (save them to restore later).
    pub fn on_columns_changed(mut self, f: impl Fn(Vec<ColumnState>) + 'static) -> Self {
        self.on_columns = Some(Box::new(f));
        self
    }
}

impl<T: 'static> View for Table<T> {
    fn build(self, cx: &mut Cx) -> Node {
        let scroll = self.model.scroll_view();
        let (mut node, _) = Node::leaf(cx, Retained::into_super(scroll), (true, true), Size { w: 200.0, h: 120.0 });
        if let crate::view::Kind::Leaf(l) = &node.kind {
            // A scroll view has no useful fitting size: it takes what it's given.
            l.content.set(Some(Size { w: 200.0, h: 120.0 }));
        }
        let model = self.model.clone();
        let Table { on_select, on_activate, on_sort, on_toggle, on_edit, on_columns, .. } = self;
        let drain: Drain = Rc::new(move || {
            for event in model.drain_events() {
                match event {
                    TableEvent::Selection(rows) => on_select.iter().for_each(|f| f(rows.clone())),
                    TableEvent::Activate(row) => on_activate.iter().for_each(|f| f(row)),
                    TableEvent::Sort(sort) => on_sort.iter().for_each(|f| f(sort.clone())),
                    TableEvent::Toggle { row, column, value } => on_toggle.iter().for_each(|f| f((row, column.clone(), value))),
                    TableEvent::Edit { row, column, value } => on_edit.iter().for_each(|f| f((row, column.clone(), value.clone()))),
                    TableEvent::ColumnsChanged => on_columns.iter().for_each(|f| f(model.column_state())),
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
}

impl TextEditor {
    /// Binds the text two ways to `text`: edits set it (not while an input
    /// method is composing); setting it replaces the editor's text (clearing
    /// undo, like [`TextModel::set_text`]) unless it already matches.
    pub fn bind(mut self, text: &Signal<String>) -> Self {
        self.bound = Some(text.clone());
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
}

impl View for TextEditor {
    fn build(self, cx: &mut Cx) -> Node {
        let scroll = self.model.scroll_view();
        let (mut node, _) = Node::leaf(cx, Retained::into_super(scroll), (true, true), Size { w: 200.0, h: 80.0 });
        if let crate::view::Kind::Leaf(l) = &node.kind {
            l.content.set(Some(Size { w: 200.0, h: 80.0 }));
        }
        if let Some(bound) = &self.bound {
            // Signal → editor (skipped when equal: the editor is the source).
            let (bound, model) = (bound.clone(), self.model.clone());
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
    TextEditor { model: model.clone(), bound: None, on_change: None, on_selection: None, on_composition: None }
}
