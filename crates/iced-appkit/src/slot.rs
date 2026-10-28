//! An Iced widget that reserves space for a native view and moves the view to
//! its layout bounds each time Iced draws.

use iced::advanced::image::{self, Image};
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{Renderer, renderer};
use iced::{Color, Element, Length, Rectangle, Size, Vector, mouse};

use appkit_host::{Rect, ViewId};

/// An Iced widget that holds a mounted native view in the layout.
pub struct NativeSlot {
    id: ViewId,
    width: Length,
    height: Length,
    background: Option<Color>,
    scroll: Option<ScrollContext>,
    focusable: bool,
}

/// Where the enclosing Iced scrollable sits in the window, and the offset its
/// last `on_scroll` reported.
#[derive(Debug, Clone, Copy)]
struct ScrollContext {
    /// Unknown until the scrollable first reports; the offset is then 0.
    bounds: Option<Rectangle>,
    reported_offset: Vector,
}

impl NativeSlot {
    /// A slot for the view mounted as `id`.
    pub fn new(id: ViewId, width: impl Into<Length>, height: impl Into<Length>) -> Self {
        Self { id, width: width.into(), height: height.into(), background: None, scroll: None, focusable: false }
    }

    /// Width in the Iced layout.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Height in the Iced layout.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Declares that the slot is inside an Iced scrollable with these window
    /// bounds; the view is then clipped to what the scrollable shows.
    pub fn in_scrollable(mut self, bounds: Option<Rectangle>, reported_offset: Vector) -> Self {
        self.scroll = Some(ScrollContext { bounds, reported_offset });
        self
    }

    /// Fills the slot on the Iced side, snapped to the same pixel grid as the
    /// native view. Use the native view's own background colour, so a frame
    /// where the two layers disagree isn't visible.
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Makes the native view a stop in Iced's focus order (Tab / Shift-Tab,
    /// `focus_next`, `focus(id)` with [`NativeSlot::widget_id`]).
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    /// The Iced widget id of a slot, for `iced::widget::operation::focus`.
    pub fn widget_id(id: ViewId) -> iced::advanced::widget::Id {
        crate::slot_widget_id(id)
    }
}

/// The native view as an Iced `Focusable`: focus state lives in AppKit.
struct NativeFocus(ViewId);

impl Focusable for NativeFocus {
    fn is_focused(&self) -> bool {
        appkit_host::focused_view() == Some(self.0)
    }

    fn focus(&mut self) {
        appkit_host::focus(self.0);
    }

    fn unfocus(&mut self) {
        if self.is_focused() {
            appkit_host::focus_host();
        }
    }
}

fn to_rect(r: Rectangle) -> Rect {
    Rect::new(r.x as f64, r.y as f64, r.width as f64, r.height as f64)
}

/// `r` (content coordinates) snapped to the device-pixel grid in window space.
fn snapped(r: Rectangle, translation: Vector) -> Rectangle {
    let s = to_rect(r - translation).snapped(appkit_host::scale());
    Rectangle::new(
        iced::Point::new(s.x as f32 + translation.x, s.y as f32 + translation.y),
        Size::new(s.width as f32, s.height as f32),
    )
}

impl<Message, Theme, R> Widget<Message, Theme, R> for NativeSlot
where
    R: Renderer + image::Renderer<Handle = image::Handle>,
{
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &R, limits: &layout::Limits) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
    }

    fn operate(&mut self, _tree: &mut Tree, layout: Layout<'_>, _renderer: &R, operation: &mut dyn Operation) {
        if self.focusable {
            operation.focusable(Some(&crate::slot_widget_id(self.id)), layout.bounds(), &mut NativeFocus(self.id));
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut R,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        // Inside a scrollable, `bounds` and `viewport` are in content
        // coordinates; the translation Iced applies this very frame is the
        // viewport's offset from the scrollable's window position.
        let bounds = layout.bounds();
        let translation = match self.scroll {
            None => Vector::ZERO,
            Some(ScrollContext { bounds: Some(sb), .. }) => Vector::new(viewport.x - sb.x, viewport.y - sb.y),
            Some(ScrollContext { bounds: None, reported_offset }) => reported_offset,
        };
        let visible = match self.scroll {
            None => bounds,
            Some(_) => bounds.intersection(viewport).unwrap_or(Rectangle::new(bounds.position(), Size::ZERO)),
        };

        if let Some(color) = self.background {
            renderer.fill_quad(renderer::Quad { bounds: snapped(bounds, translation), ..renderer::Quad::default() }, color);
        }
        // Under an Iced overlay: a picture of the view, drawn where the view is
        // (the root decides when the real view hides).
        if let Some(handle) = crate::root::snapshot_handle(self.id) {
            let clip = snapped(visible, translation);
            renderer.draw_image(Image::new(handle), snapped(bounds, translation), clip);
        }

        // Draw runs on the main thread in the frame Iced renders, so the native
        // change and the GPU frame are produced together.
        appkit_host::place(self.id, to_rect(bounds - translation), to_rect(visible - translation));
    }
}

impl<'a, Message: 'a, Theme: 'a, R> From<NativeSlot> for Element<'a, Message, Theme, R>
where
    R: Renderer + image::Renderer<Handle = image::Handle> + 'a,
{
    fn from(slot: NativeSlot) -> Self {
        Element::new(slot)
    }
}
