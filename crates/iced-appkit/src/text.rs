//! `text_editor(&model)`: a real `NSTextView` in the Iced layout.

use std::ops::Range;

use iced::advanced::image;
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{Clipboard, Renderer, Shell, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

pub use appkit_host::text::{TextEvent, TextModel};

use crate::NativeSlot;

type Callback<'a, A, Message> = Option<Box<dyn Fn(A) -> Message + 'a>>;

/// A native plain-text editor showing `model`. Focusable (Tab) by default.
pub fn text_editor<'a, Message>(model: &TextModel) -> TextEditor<'a, Message> {
    TextEditor {
        model: model.clone(),
        slot: NativeSlot::new(model.id(), Length::Fill, Length::Fill).focusable(true),
        on_change: None,
        on_selection: None,
        on_composition: None,
    }
}

/// The text editor widget; see [`text_editor`].
pub struct TextEditor<'a, Message> {
    model: TextModel,
    slot: NativeSlot,
    on_change: Callback<'a, String, Message>,
    on_selection: Callback<'a, Range<usize>, Message>,
    on_composition: Callback<'a, Option<String>, Message>,
}

impl<'a, Message> TextEditor<'a, Message> {
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

    /// The text changed; receives the whole text. (For very large documents,
    /// prefer reacting with [`TextModel::text`] when you need it.)
    pub fn on_change(mut self, f: impl Fn(String) -> Message + 'a) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// The selection moved (byte range; empty for the caret).
    pub fn on_selection(mut self, f: impl Fn(Range<usize>) -> Message + 'a) -> Self {
        self.on_selection = Some(Box::new(f));
        self
    }

    /// An input method started or updated a composition (`Some`), or ended it.
    pub fn on_composition(mut self, f: impl Fn(Option<String>) -> Message + 'a) -> Self {
        self.on_composition = Some(Box::new(f));
        self
    }
}

impl<Message, Theme, R> Widget<Message, Theme, R> for TextEditor<'_, Message>
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
        for event in self.model.drain_events() {
            let message = match event {
                TextEvent::Changed => self.on_change.as_ref().map(|f| f(self.model.text())),
                TextEvent::Selection(range) => self.on_selection.as_ref().map(|f| f(range)),
                TextEvent::Composition(marked) => self.on_composition.as_ref().map(|f| f(marked)),
            };
            if let Some(message) = message {
                shell.publish(message);
            }
        }
    }
}

impl<'a, Message: 'a, Theme: 'a, R> From<TextEditor<'a, Message>> for Element<'a, Message, Theme, R>
where
    R: Renderer + image::Renderer<Handle = image::Handle> + 'a,
{
    fn from(editor: TextEditor<'a, Message>) -> Self {
        Element::new(editor)
    }
}
