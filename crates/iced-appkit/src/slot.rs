//! An Iced widget that reserves space for a native view and moves the view to
//! its layout bounds each time Iced draws.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Renderer, renderer};
use iced::{Color, Element, Length, Rectangle, Size, Vector, mouse};

use appkit_host::{Rect, ViewId};

pub struct NativeSlot {
    id: ViewId,
    width: Length,
    height: Length,
    background: Option<Color>,
    scroll: Option<ScrollContext>,
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
    pub fn new(id: ViewId, width: impl Into<Length>, height: impl Into<Length>) -> Self {
        Self { id, width: width.into(), height: height.into(), background: None, scroll: None }
    }

    /// Declares that the slot is inside an Iced scrollable with these window
    /// bounds; the view is then clipped to what the scrollable shows.
    pub fn in_scrollable(mut self, bounds: Option<Rectangle>, reported_offset: Vector) -> Self {
        self.scroll = Some(ScrollContext { bounds, reported_offset });
        self
    }

    /// Fills the slot on the Iced side, snapped to the same pixel grid as the
    /// native view. Use the native view's own background colour, so a frame
    /// where the two layers disagree (e.g. during live resize) isn't visible.
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }
}

fn to_rect(r: Rectangle) -> Rect {
    Rect::new(r.x as f64, r.y as f64, r.width as f64, r.height as f64)
}

impl<Message, Theme, R: Renderer> Widget<Message, Theme, R> for NativeSlot {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &R, limits: &layout::Limits) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
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
            // Snap in window space (where the native view is snapped), then
            // move back into the content space Iced is drawing in.
            let r = to_rect(bounds - translation).snapped(appkit_host::scale());
            let snapped = Rectangle::new(
                iced::Point::new(r.x as f32 + translation.x, r.y as f32 + translation.y),
                Size::new(r.width as f32, r.height as f32),
            );
            renderer.fill_quad(renderer::Quad { bounds: snapped, ..renderer::Quad::default() }, color);
        }

        // Draw runs on the main thread in the frame Iced renders, so the native
        // change and the GPU frame are produced together.
        appkit_host::place(self.id, to_rect(bounds - translation), to_rect(visible - translation));
    }
}

impl<'a, Message: 'a, Theme: 'a, R: Renderer + 'a> From<NativeSlot> for Element<'a, Message, Theme, R> {
    fn from(slot: NativeSlot) -> Self {
        Element::new(slot)
    }
}
