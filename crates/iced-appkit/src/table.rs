//! `table(&model)`: a real `NSTableView` in the Iced layout.

use iced::advanced::image;
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{Clipboard, Renderer, Shell, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

pub use appkit_host::table::{Column, ColumnState, Sort, TableEvent, TableModel, Value};

use crate::NativeSlot;

/// A native table showing `model`. Focusable (Tab) by default.
pub fn table<'a, T: 'static, Message>(model: &TableModel<T>) -> Table<'a, T, Message> {
    Table {
        model: model.clone(),
        slot: NativeSlot::new(model.id(), Length::Fill, Length::Fill).focusable(true),
        on_select: None,
        on_activate: None,
        on_sort: None,
        on_toggle: None,
        on_columns: None,
    }
}

type Callback<'a, A, Message> = Option<Box<dyn Fn(A) -> Message + 'a>>;

/// The table widget; see [`table`].
pub struct Table<'a, T: 'static, Message> {
    model: TableModel<T>,
    slot: NativeSlot,
    on_select: Callback<'a, Vec<usize>, Message>,
    on_activate: Callback<'a, usize, Message>,
    on_sort: Callback<'a, Sort, Message>,
    on_toggle: Callback<'a, (usize, String, bool), Message>,
    on_columns: Callback<'a, Vec<ColumnState>, Message>,
}

impl<'a, T: 'static, Message> Table<'a, T, Message> {
    /// Width in the Iced layout (default: fill).
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.slot = self.slot.width(width);
        self
    }

    /// Height in the Iced layout (default: fill).
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.slot = self.slot.height(height);
        self
    }

    /// See [`NativeSlot::in_scrollable`].
    pub fn in_scrollable(mut self, bounds: Option<Rectangle>, reported_offset: Vector) -> Self {
        self.slot = self.slot.in_scrollable(bounds, reported_offset);
        self
    }

    /// Selected rows changed (model indices).
    pub fn on_select(mut self, f: impl Fn(Vec<usize>) -> Message + 'a) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }

    /// A row was double-clicked or Return was pressed on it.
    pub fn on_activate(mut self, f: impl Fn(usize) -> Message + 'a) -> Self {
        self.on_activate = Some(Box::new(f));
        self
    }

    /// The user clicked a column header.
    pub fn on_sort(mut self, f: impl Fn(Sort) -> Message + 'a) -> Self {
        self.on_sort = Some(Box::new(f));
        self
    }

    /// A checkbox cell was clicked: (row, column id, new value).
    pub fn on_toggle(mut self, f: impl Fn(usize, String, bool) -> Message + 'a) -> Self {
        self.on_toggle = Some(Box::new(move |(row, column, value)| f(row, column, value)));
        self
    }

    /// Columns were moved or resized.
    pub fn on_columns_changed(mut self, f: impl Fn(Vec<ColumnState>) -> Message + 'a) -> Self {
        self.on_columns = Some(Box::new(f));
        self
    }
}

impl<T: 'static, Message, Theme, R> Widget<Message, Theme, R> for Table<'_, T, Message>
where
    R: Renderer + image::Renderer<Handle = image::Handle>,
{
    fn size(&self) -> Size<Length> {
        Widget::<Message, Theme, R>::size(&self.slot)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &R, limits: &layout::Limits) -> layout::Node {
        Widget::<Message, Theme, R>::layout(&mut self.slot, tree, renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &R, operation: &mut dyn Operation) {
        Widget::<Message, Theme, R>::operate(&mut self.slot, tree, layout, renderer, operation);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut R,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.model.ensure_mounted();
        Widget::<Message, Theme, R>::draw(&self.slot, tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        _event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &R,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        // Native events were queued on the model (and a redraw requested);
        // turn them into the app's messages.
        for event in self.model.drain_events() {
            let message = match event {
                TableEvent::Selection(rows) => self.on_select.as_ref().map(|f| f(rows)),
                TableEvent::Activate(row) => self.on_activate.as_ref().map(|f| f(row)),
                TableEvent::Sort(sort) => self.on_sort.as_ref().map(|f| f(sort)),
                TableEvent::Toggle { row, column, value } => self.on_toggle.as_ref().map(|f| f((row, column, value))),
                TableEvent::ColumnsChanged => self.on_columns.as_ref().map(|f| f(self.model.column_state())),
            };
            if let Some(message) = message {
                shell.publish(message);
            }
        }
    }
}

impl<'a, T: 'static, Message: 'a, Theme: 'a, R> From<Table<'a, T, Message>> for Element<'a, Message, Theme, R>
where
    R: Renderer + image::Renderer<Handle = image::Handle> + 'a,
{
    fn from(table: Table<'a, T, Message>) -> Self {
        Element::new(table)
    }
}
