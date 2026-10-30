//! A drawing canvas: draw shapes and text with a [`Painter`]. The drawing
//! is recorded and replayed by a native view, and recorded again when the
//! signals it reads change or the canvas resizes. Charts
//! ([`bar_chart`](crate::bar_chart), [`line_chart`](crate::line_chart)) are
//! built on it.
//!
//! Coordinates are points from the top-left corner. Colours are
//! [`TextColor`]s: the system's (they follow Dark Mode) or fixed sRGB.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use appkit_host::text::TextColor;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2::AnyThread;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSAccessibility, NSBezierPath, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent, NSFont, NSImage,
    NSResponder, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::signal::{Effect, Signal, untrack};
use crate::view::{Align, Cx, Node, Size, View};

/// One drawing operation (recorded, then replayed when AppKit draws).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Op {
    Fill { shape: Shape, color: TextColor, opacity: f64 },
    Stroke { shape: Shape, color: TextColor, opacity: f64, width: f64 },
    Text { x: f64, y: f64, text: String, size: f64, bold: bool, color: TextColor, opacity: f64, align: Align },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Shape {
    Rect { x: f64, y: f64, w: f64, h: f64, radius: f64 },
    Ellipse { x: f64, y: f64, w: f64, h: f64 },
    Path { points: Vec<(f64, f64)>, closed: bool },
}

/// Draws on a [`canvas`]: shapes and text, in points from the top-left
/// corner.
pub struct Painter {
    pub(crate) ops: Vec<Op>,
    width: f64,
    height: f64,
    opacity: f64,
}

impl Painter {
    pub(crate) fn new(width: f64, height: f64) -> Self {
        Painter { ops: Vec::new(), width, height, opacity: 1.0 }
    }

    /// The canvas's width in points.
    pub fn width(&self) -> f64 {
        self.width
    }

    /// The canvas's height in points.
    pub fn height(&self) -> f64 {
        self.height
    }

    /// The opacity (0–1) of what's drawn next.
    pub fn set_opacity(&mut self, opacity: f64) {
        self.opacity = opacity.clamp(0.0, 1.0);
    }

    fn fill(&mut self, shape: Shape, color: TextColor) {
        self.ops.push(Op::Fill { shape, color, opacity: self.opacity });
    }

    fn stroke(&mut self, shape: Shape, color: TextColor, width: f64) {
        self.ops.push(Op::Stroke { shape, color, opacity: self.opacity, width });
    }

    /// Fills a rectangle.
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: TextColor) {
        self.fill(Shape::Rect { x, y, w, h, radius: 0.0 }, color);
    }

    /// Fills a rectangle with rounded corners.
    pub fn fill_rounded_rect(&mut self, x: f64, y: f64, w: f64, h: f64, radius: f64, color: TextColor) {
        self.fill(Shape::Rect { x, y, w, h, radius }, color);
    }

    /// Outlines a rectangle.
    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: TextColor, width: f64) {
        self.stroke(Shape::Rect { x, y, w, h, radius: 0.0 }, color, width);
    }

    /// Fills a circle around `cx, cy`.
    pub fn fill_circle(&mut self, cx: f64, cy: f64, r: f64, color: TextColor) {
        self.fill(Shape::Ellipse { x: cx - r, y: cy - r, w: 2.0 * r, h: 2.0 * r }, color);
    }

    /// Outlines a circle around `cx, cy`.
    pub fn stroke_circle(&mut self, cx: f64, cy: f64, r: f64, color: TextColor, width: f64) {
        self.stroke(Shape::Ellipse { x: cx - r, y: cy - r, w: 2.0 * r, h: 2.0 * r }, color, width);
    }

    /// A straight line.
    pub fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, color: TextColor, width: f64) {
        self.stroke(Shape::Path { points: vec![(x1, y1), (x2, y2)], closed: false }, color, width);
    }

    /// Lines through `points`.
    pub fn polyline(&mut self, points: &[(f64, f64)], color: TextColor, width: f64) {
        if points.len() >= 2 {
            self.stroke(Shape::Path { points: points.to_vec(), closed: false }, color, width);
        }
    }

    /// Fills the polygon through `points`.
    pub fn fill_polygon(&mut self, points: &[(f64, f64)], color: TextColor) {
        if points.len() >= 3 {
            self.fill(Shape::Path { points: points.to_vec(), closed: true }, color);
        }
    }

    /// Text in the system font at `size` points. `x` is its start, centre
    /// or end (`align`); `y` is its top.
    pub fn text(&mut self, x: f64, y: f64, text: &str, size: f64, color: TextColor, align: Align) {
        self.ops.push(Op::Text { x, y, text: text.into(), size, bold: false, color, opacity: self.opacity, align });
    }

    /// Bold text (see [`text`](Self::text)).
    pub fn bold_text(&mut self, x: f64, y: f64, text: &str, size: f64, color: TextColor, align: Align) {
        self.ops.push(Op::Text { x, y, text: text.into(), size, bold: true, color, opacity: self.opacity, align });
    }

    /// The width and height of `text` at `size` points.
    pub fn measure(&self, text: &str, size: f64) -> (f64, f64) {
        measure(text, size, false)
    }
}

fn font(size: f64, bold: bool) -> Retained<NSFont> {
    if bold { NSFont::boldSystemFontOfSize(size) } else { NSFont::systemFontOfSize(size) }
}

fn attributes(size: f64, bold: bool, color: Option<(TextColor, f64)>) -> Retained<objc2_foundation::NSDictionary<NSString, AnyObject>> {
    use objc2_app_kit::{NSFontAttributeName, NSForegroundColorAttributeName};
    let font: Retained<AnyObject> = Retained::into_super(Retained::into_super(font(size, bold)));
    // SAFETY: the standard attribute keys, with a font and a colour.
    unsafe {
        match color {
            Some((c, opacity)) => {
                let c: Retained<AnyObject> = Retained::into_super(Retained::into_super(c.ns().colorWithAlphaComponent(opacity)));
                objc2_foundation::NSDictionary::from_retained_objects(&[NSFontAttributeName, NSForegroundColorAttributeName], &[font, c])
            }
            None => objc2_foundation::NSDictionary::from_retained_objects(&[NSFontAttributeName], &[font]),
        }
    }
}

pub(crate) fn measure(text: &str, size: f64, bold: bool) -> (f64, f64) {
    let attrs = attributes(size, bold, None);
    // SAFETY: measuring a string with font attributes.
    let s = unsafe { objc2_app_kit::NSStringDrawing::sizeWithAttributes(&*NSString::from_str(text), Some(&attrs)) };
    (s.width, s.height)
}

fn path(shape: &Shape) -> Retained<NSBezierPath> {
    match shape {
        Shape::Rect { x, y, w, h, radius } if *radius > 0.0 => {
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(NSRect::new(NSPoint::new(*x, *y), NSSize::new(*w, *h)), *radius, *radius)
        }
        Shape::Rect { x, y, w, h, .. } => NSBezierPath::bezierPathWithRect(NSRect::new(NSPoint::new(*x, *y), NSSize::new(*w, *h))),
        Shape::Ellipse { x, y, w, h } => NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(*x, *y), NSSize::new(*w, *h))),
        Shape::Path { points, closed } => {
            let p = NSBezierPath::bezierPath();
            for (i, &(x, y)) in points.iter().enumerate() {
                if i == 0 {
                    p.moveToPoint(NSPoint::new(x, y));
                } else {
                    p.lineToPoint(NSPoint::new(x, y));
                }
            }
            if *closed {
                p.closePath();
            }
            p
        }
    }
}

fn replay(ops: &[Op]) {
    for op in ops {
        match op {
            Op::Fill { shape, color, opacity } => {
                color.ns().colorWithAlphaComponent(*opacity).setFill();
                path(shape).fill();
            }
            Op::Stroke { shape, color, opacity, width } => {
                color.ns().colorWithAlphaComponent(*opacity).setStroke();
                let p = path(shape);
                p.setLineWidth(*width);
                p.setLineJoinStyle(objc2_app_kit::NSLineJoinStyle::Round);
                p.stroke();
            }
            Op::Text { x, y, text, size, bold, color, opacity, align } => {
                let attrs = attributes(*size, *bold, Some((*color, *opacity)));
                let s = NSString::from_str(text);
                // SAFETY: drawing a string in the current (flipped) context.
                unsafe {
                    let w = objc2_app_kit::NSStringDrawing::sizeWithAttributes(&*s, Some(&attrs)).width;
                    let x = match align {
                        Align::Start => *x,
                        Align::Center => x - w / 2.0,
                        Align::End => x - w,
                    };
                    objc2_app_kit::NSStringDrawing::drawAtPoint_withAttributes(&*s, NSPoint::new(x, *y), Some(&attrs));
                }
            }
        }
    }
}

type Draw = Rc<dyn Fn(&mut Painter)>;
type Hover = Rc<dyn Fn(Option<(f64, f64)>)>;
type Click = Rc<dyn Fn(f64, f64)>;

type Drag = Rc<dyn Fn() -> Option<crate::DragItem>>;

pub(crate) struct CanvasState {
    ops: RefCell<Vec<Op>>,
    size: Cell<Option<Signal<(f64, f64)>>>,
    hover: RefCell<Option<Hover>>,
    click: RefCell<Option<Click>>,
    drag: RefCell<Option<Drag>>,
    /// Where the mouse went down, until a drag starts.
    down: Cell<Option<NSPoint>>,
    /// Drawing for an image: on the content background, not transparent.
    opaque: Cell<bool>,
}

define_class!(
    // Replays the recorded drawing; reports its size, hover and clicks.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiCanvas"]
    #[ivars = CanvasState]
    pub(crate) struct CanvasView;

    impl CanvasView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            if self.ivars().opaque.get() {
                TextColor::Background.ns().setFill();
                NSBezierPath::fillRect(self.bounds());
            }
            replay(&self.ivars().ops.borrow());
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            if let Some(s) = self.ivars().size.get() {
                s.set_if_changed((size.width, size.height));
            }
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            // Named colours resolve when drawn: draw again in the new look.
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let hover = self.ivars().hover.borrow().clone();
            if let Some(f) = hover {
                crate::signal::batch(|| f(Some((p.x, p.y))));
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            let hover = self.ivars().hover.borrow().clone();
            if let Some(f) = hover {
                crate::signal::batch(|| f(None));
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            self.ivars().down.set(Some(p));
            let click = self.ivars().click.borrow().clone();
            if let Some(f) = click {
                crate::signal::batch(|| f(p.x, p.y));
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let Some(start) = self.ivars().down.get() else { return };
            if (p.x - start.x).hypot(p.y - start.y) < 4.0 {
                return;
            }
            self.ivars().down.set(None);
            let drag = self.ivars().drag.borrow().clone();
            if let Some(item) = drag.and_then(|f| f()) {
                self.begin_drag(&item, event);
            }
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(isAccessibilityElement))]
        fn is_ax_element(&self) -> bool {
            true
        }
    }

    unsafe impl NSObjectProtocol for CanvasView {}

    unsafe impl NSDraggingSource for CanvasView {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operation_mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            NSDragOperation::Copy
        }
    }
);

impl CanvasView {
    fn begin_drag(&self, item: &crate::DragItem, event: &NSEvent) {
        let dragging = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), &item.writer());
        // The drag shows the canvas as it looks.
        let image = png(self).and_then(|png| NSImage::initWithData(NSImage::alloc(), &objc2_foundation::NSData::with_bytes(&png)));
        // SAFETY: an image (or none) as the dragged item's picture.
        unsafe { dragging.setDraggingFrame_contents(self.bounds(), image.as_deref().map(|i| i.as_ref())) };
        let items = objc2_foundation::NSArray::from_retained_slice(&[dragging]);
        self.beginDraggingSessionWithItems_event_source(&items, event, ProtocolObject::from_ref(self));
    }
}

/// The canvas as a PNG, on the content background (2× on Retina).
pub(crate) fn png(view: &NSView) -> Option<Vec<u8>> {
    let canvas = view.downcast_ref::<CanvasView>()?;
    canvas.ivars().opaque.set(true);
    let rep = view.bitmapImageRepForCachingDisplayInRect(view.bounds());
    if let Some(rep) = &rep {
        view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), rep);
    }
    canvas.ivars().opaque.set(false);
    // SAFETY: encoding a bitmap as PNG with no options.
    let data = unsafe { rep?.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) }?;
    Some(data.to_vec())
}

/// A drawing canvas, made by [`canvas`].
pub struct Canvas {
    draw: Draw,
    hover: Option<Hover>,
    click: Option<Click>,
    description: Option<Rc<dyn Fn() -> String>>,
    min: Size,
    drag: Option<Drag>,
    export: Option<String>,
}

impl Canvas {
    /// Called with the pointer's position as it moves over the canvas, and
    /// `None` when it leaves (while the app is active).
    pub fn on_hover(mut self, f: impl Fn(Option<(f64, f64)>) + 'static) -> Self {
        self.hover = Some(Rc::new(f));
        self
    }

    /// Called with the position of a click.
    pub fn on_click(mut self, f: impl Fn(f64, f64) + 'static) -> Self {
        self.click = Some(Rc::new(f));
        self
    }

    /// What VoiceOver reads for the canvas (an image to it), kept up to
    /// date as the signals `f` reads change.
    pub fn description(mut self, f: impl Fn() -> String + 'static) -> Self {
        self.description = Some(Rc::new(f));
        self
    }

    /// Dragging the canvas carries what `f` returns (nothing if `None`):
    /// text, an image, a file, or a file written where it's dropped.
    pub fn draggable(mut self, f: impl Fn() -> Option<crate::DragItem> + 'static) -> Self {
        self.drag = Some(Rc::new(f));
        self
    }

    /// The canvas as an image: a right-click menu with Copy Image and Save
    /// Image…, and dragging it out gives `name`.png (to the Finder, Mail,
    /// a document). Charts use this for exporting.
    pub fn exportable(mut self, name: &str) -> Self {
        self.export = Some(name.into());
        self
    }

    /// The smallest size it's laid out at (default 200 × 120); it takes the
    /// room it's given beyond that.
    pub fn min_size(mut self, width: f64, height: f64) -> Self {
        self.min = Size { w: width, h: height };
        self
    }
}

/// A canvas drawn by `draw`, again whenever the signals it reads change or
/// the canvas resizes:
///
/// ```no_run
/// use appkit_ui::*;
///
/// let level = Signal::new(0.4);
/// let meter = canvas(move |p| {
///     let w = p.width() * level.get();
///     p.fill_rounded_rect(0.0, 0.0, p.width(), p.height(), 6.0, TextColor::Separator);
///     p.fill_rounded_rect(0.0, 0.0, w, p.height(), 6.0, TextColor::Green);
///     p.text(p.width() / 2.0, 4.0, &format!("{:.0}%", level.get() * 100.0), 12.0, TextColor::Primary, Align::Center);
/// })
/// .description(move || format!("Level {:.0}%", level.get() * 100.0));
/// # let _ = meter;
/// ```
pub fn canvas(draw: impl Fn(&mut Painter) + 'static) -> Canvas {
    Canvas { draw: Rc::new(draw), hover: None, click: None, description: None, min: Size { w: 200.0, h: 120.0 }, drag: None, export: None }
}

impl View for Canvas {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        let size = Signal::new((0.0, 0.0));
        let view = CanvasView::alloc(mtm).set_ivars(CanvasState {
            ops: RefCell::new(Vec::new()),
            size: Cell::new(Some(size)),
            hover: RefCell::new(self.hover.clone()),
            click: RefCell::new(self.click),
            drag: RefCell::new(self.drag),
            down: Cell::new(None),
            opaque: Cell::new(false),
        });
        // SAFETY: NSView's designated initialiser.
        let view: Retained<CanvasView> = unsafe { msg_send![super(view), initWithFrame: NSRect::new(NSPoint::ZERO, NSSize::new(self.min.w, self.min.h))] };
        view.setAccessibilityRole(Some(unsafe { objc2_app_kit::NSAccessibilityImageRole }));
        if self.hover.is_some() {
            // SAFETY: a tracking area owned by the view, following its bounds.
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    NSTrackingAreaOptions::MouseMoved
                        | NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveInActiveApp
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(&view),
                    None,
                )
            };
            view.addTrackingArea(&area);
        }
        let ns: Retained<NSView> = Retained::into_super(view.clone());
        let (mut node, _) = Node::leaf(cx, ns, (true, true), self.min);
        // Record the drawing again when what it reads changes, or the size.
        let (v, draw) = (view.clone(), self.draw);
        node.own(Effect::new(move || {
            let (w, h) = size.get();
            let mut painter = Painter::new(w, h);
            if w > 0.0 && h > 0.0 {
                draw(&mut painter);
            }
            untrack(|| {
                *v.ivars().ops.borrow_mut() = painter.ops;
                v.setNeedsDisplay(true);
            });
        }));
        if let Some(name) = self.export {
            export(&mut node, &view, &name, cx.mtm);
        }
        if let Some(describe) = self.description {
            let v = view.clone();
            node.own(Effect::new(move || {
                let label = describe();
                v.setAccessibilityLabel(Some(&NSString::from_str(&label)));
            }));
        }
        node
    }
}

/// Copy Image / Save Image… and dragging out `name`.png.
fn export(node: &mut Node, view: &Retained<CanvasView>, name: &str, mtm: objc2::MainThreadMarker) {
    let file = format!("{name}.png");
    if view.ivars().drag.borrow().is_none() {
        let (v, file) = (objc2::rc::Weak::from_retained(view), file.clone());
        *view.ivars().drag.borrow_mut() = Some(Rc::new(move || {
            // Rendered now (main thread); written when the drop lands.
            let view = v.load()?;
            let png = std::sync::Arc::new(png(&view)?);
            Some(crate::DragItem::promise(&file, move |path| std::fs::write(path, &*png)))
        }));
    }
    let (copy, save) = (objc2::rc::Weak::from_retained(view), objc2::rc::Weak::from_retained(view));
    let items = vec![
        crate::chrome::menu_item(&crate::t("Copy Image"), move || {
            if let Some(png) = copy.load().and_then(|v| png(&v)) {
                crate::Clipboard::general().set_png(&png);
            }
        }),
        crate::chrome::menu_item(&crate::t("Save Image…"), move || {
            let Some(v) = save.load() else { return };
            let (Some(png), Some(window)) = (png(&v), v.window()) else { return };
            crate::dialogs::save_panel(&window, &file, None, move |path| {
                if let Some(path) = path
                    && let Err(e) = std::fs::write(&path, &png)
                {
                    eprintln!("appkit-ui: couldn't save {}: {e}", path.display());
                }
            });
        }),
    ];
    let ns: Retained<NSView> = Retained::into_super(view.clone());
    for t in <crate::platform::Native as crate::platform::Platform>::set_context_menu(mtm, &[ns], items) {
        node.keep(t);
    }
}

/// What dragging a canvas would carry now (for checks).
pub(crate) fn drag_item(view: &NSView) -> Option<crate::DragItem> {
    let drag = view.downcast_ref::<CanvasView>()?.ivars().drag.borrow().clone()?;
    drag()
}

/// What a canvas drew last, one line per operation: `fill rect 10,20 30x40
/// Blue`, `stroke path 120 points Orange`, `text "Jan" 11pt`. `None` if
/// `view` isn't a canvas.
pub(crate) fn describe(view: &NSView) -> Option<Vec<String>> {
    let c = view.downcast_ref::<CanvasView>()?;
    let shape = |s: &Shape| match s {
        Shape::Rect { x, y, w, h, .. } => format!("rect {x:.0},{y:.0} {w:.0}x{h:.0}"),
        Shape::Ellipse { x, y, w, h } => format!("ellipse {:.0},{:.0} {w:.0}x{h:.0}", x + w / 2.0, y + h / 2.0),
        Shape::Path { points, .. } => format!("path {} points", points.len()),
    };
    Some(
        c.ivars()
            .ops
            .borrow()
            .iter()
            .map(|op| match op {
                Op::Fill { shape: s, color, .. } => format!("fill {} {color:?}", shape(s)),
                Op::Stroke { shape: s, color, .. } => format!("stroke {} {color:?}", shape(s)),
                Op::Text { text, size, .. } => format!("text {text:?} {size:.0}pt"),
            })
            .collect(),
    )
}
