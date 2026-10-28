//! The root wrapper: per-frame bookkeeping, and "snapshot in place" so Iced
//! overlays (dropdowns, tooltips, menus) and modals draw above native views.
//!
//! Native views are separate layers in front of Iced's GPU surface, so
//! anything Iced draws on top lands *under* them. While something covers a
//! native view, the view is swapped for a picture of itself drawn by Iced, and
//! the real view is hidden. Timing (measured on 60 fps video in the spike):
//! - Iced shows a newly created image one frame after it is first drawn, so
//!   the real view stays up, over its picture, for two frames before hiding.
//!   An overlay therefore appears ~2 frames after it opens.
//! - Hiding and showing happen during the slot's draw, so they commit with
//!   the GPU frame that matches.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{Clipboard, Shell, image, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

use appkit_host::{Rect, ViewId};

/// Frames the real view stays over its fresh picture before hiding.
const SWAP_DELAY_FRAMES: u64 = 2;

struct Swap {
    handle: image::Handle,
    created: u64,
}

thread_local! {
    static FRAME: Cell<u64> = const { Cell::new(0) };
    /// Areas covered by an open Iced overlay (set while laying it out).
    static OVERLAY_COVERS: RefCell<Vec<Rectangle>> = const { RefCell::new(Vec::new()) };
    /// Areas covered by `Cover` widgets (set during layout).
    static LAYOUT_COVERS: RefCell<Vec<Rectangle>> = const { RefCell::new(Vec::new()) };
    static SWAPS: RefCell<HashMap<ViewId, Swap>> = RefCell::new(HashMap::new());
}

pub(crate) fn snapshot_handle(id: ViewId) -> Option<image::Handle> {
    SWAPS.with_borrow(|s| s.get(&id).map(|s| s.handle.clone()))
}

fn rect(r: Rectangle) -> Rect {
    Rect::new(r.x as f64, r.y as f64, r.width as f64, r.height as f64)
}

/// Updates the swaps for this frame. Returns whether more frames are needed.
fn update_swaps() -> bool {
    let frame = FRAME.with(|f| {
        f.set(f.get() + 1);
        f.get()
    });
    let covers: Vec<Rectangle> = OVERLAY_COVERS
        .with_borrow(|o| o.clone())
        .into_iter()
        .chain(LAYOUT_COVERS.with_borrow(|l| l.clone()))
        .collect();
    let covered: HashSet<ViewId> = covers.iter().flat_map(|c| appkit_host::views_in(rect(*c))).collect();

    SWAPS.with_borrow_mut(|swaps| {
        swaps.retain(|id, _| {
            let keep = covered.contains(id);
            if !keep {
                appkit_host::set_suppressed(*id, false);
            }
            keep
        });
        for id in &covered {
            if !swaps.contains_key(id)
                && let Some(s) = appkit_host::snapshot(*id) {
                    swaps.insert(*id, Swap { handle: image::Handle::from_rgba(s.width, s.height, s.rgba), created: frame });
                }
        }
        let mut pending = false;
        for (id, swap) in swaps.iter() {
            if frame >= swap.created + SWAP_DELAY_FRAMES {
                appkit_host::set_suppressed(*id, true);
            } else {
                pending = true;
            }
        }
        pending
    })
}

/// Wraps the window's root element. See the crate docs.
pub fn root<'a, Message, Theme, Renderer>(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Root<'a, Message, Theme, Renderer> {
    Root { content: content.into() }
}

/// The root wrapper widget; see [`root`].
pub struct Root<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
}

impl<Message, Theme, R> Widget<Message, Theme, R> for Root<'_, Message, Theme, R>
where
    R: iced::advanced::Renderer,
{
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &R, limits: &layout::Limits) -> layout::Node {
        LAYOUT_COVERS.with_borrow_mut(Vec::clear);
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
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
        appkit_host::begin_frame();
        if update_swaps() {
            appkit_host::request_redraw();
        }
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        appkit_host::end_frame();
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &R, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &R,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &R,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &R,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, R>> {
        let inner = self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation);
        match inner {
            None => {
                OVERLAY_COVERS.with_borrow_mut(Vec::clear);
                None
            }
            Some(inner) => Some(overlay::Element::new(Box::new(CoverOverlay { inner }))),
        }
    }
}

impl<'a, Message: 'a, Theme: 'a, R: iced::advanced::Renderer + 'a> From<Root<'a, Message, Theme, R>> for Element<'a, Message, Theme, R> {
    fn from(root: Root<'a, Message, Theme, R>) -> Self {
        Element::new(root)
    }
}

/// An Iced overlay that records where it is while being laid out.
struct CoverOverlay<'a, Message, Theme, R> {
    inner: overlay::Element<'a, Message, Theme, R>,
}

fn node_bounds(node: &layout::Node, out: &mut Vec<Rectangle>, offset: Vector) {
    let b = node.bounds() + offset;
    if b.width > 0.0 && b.height > 0.0 {
        out.push(b);
    }
    for child in node.children() {
        node_bounds(child, out, offset + Vector::new(node.bounds().x, node.bounds().y));
    }
}

impl<Message, Theme, R> overlay::Overlay<Message, Theme, R> for CoverOverlay<'_, Message, Theme, R>
where
    R: iced::advanced::Renderer,
{
    fn layout(&mut self, renderer: &R, bounds: Size) -> layout::Node {
        let node = self.inner.as_overlay_mut().layout(renderer, bounds);
        // Only the overlay's own top-level box: its children lie inside it.
        let mut covers = Vec::new();
        let b = node.bounds();
        if b.width > 0.0 && b.height > 0.0 {
            covers.push(b);
        } else {
            node_bounds(&node, &mut covers, Vector::ZERO);
        }
        OVERLAY_COVERS.with_borrow_mut(|o| *o = covers);
        node
    }

    fn draw(&self, renderer: &mut R, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor) {
        self.inner.as_overlay().draw(renderer, theme, style, layout, cursor);
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &R, operation: &mut dyn Operation) {
        self.inner.as_overlay_mut().operate(layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &R,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.inner.as_overlay_mut().update(event, layout, cursor, renderer, clipboard, shell);
    }

    fn mouse_interaction(&self, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &R) -> mouse::Interaction {
        self.inner.as_overlay().mouse_interaction(layout, cursor, renderer)
    }

    fn overlay<'b>(&'b mut self, layout: Layout<'b>, renderer: &R) -> Option<overlay::Element<'b, Message, Theme, R>> {
        self.inner.as_overlay_mut().overlay(layout, renderer)
    }

    fn index(&self) -> f32 {
        self.inner.as_overlay().index()
    }
}

/// Marks its content's area as covering native views beneath it, e.g. a modal
/// dialog layer in a `stack!`. Native views under it are swapped for pictures
/// while it is in the layout, so it draws above them and they can't be clicked.
pub fn cover<'a, Message, Theme, Renderer>(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Cover<'a, Message, Theme, Renderer> {
    Cover { content: content.into() }
}

/// A modal-layer marker; see [`cover`].
pub struct Cover<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
}

impl<Message, Theme, R> Widget<Message, Theme, R> for Cover<'_, Message, Theme, R>
where
    R: iced::advanced::Renderer,
{
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &R, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
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
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &R, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &R,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // Every frame starts with an update pass, before drawing: record the
        // cover's window position for the root's draw. (The root clears the
        // list on every relayout, so a removed or moved cover drops out.)
        if matches!(event, Event::Window(iced::window::Event::RedrawRequested(_))) {
            LAYOUT_COVERS.with_borrow_mut(|c| {
                if !c.contains(&layout.bounds()) {
                    c.push(layout.bounds());
                }
            });
        }
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &R,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &R,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, R>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a, Theme: 'a, R: iced::advanced::Renderer + 'a> From<Cover<'a, Message, Theme, R>> for Element<'a, Message, Theme, R> {
    fn from(cover: Cover<'a, Message, Theme, R>) -> Self {
        Element::new(cover)
    }
}
