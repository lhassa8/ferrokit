//! Surfaces: a background, rounded corners, a border and a shadow behind any
//! view (a card, a panel, a badge). A layer-backed view sits behind the
//! view's own native views, framed by layout to the view's rectangle
//! (padding included); its colours resolve again when the appearance
//! changes, so the named ones follow Dark Mode.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSView, NSViewLayerContentsRedrawPolicy};
use objc2_foundation::NSPoint;

use crate::Color;

/// A drop shadow (see [`ViewExt::shadow`](crate::ViewExt::shadow)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    /// Its colour (black at partial opacity, usually).
    pub color: Color,
    /// How far it blurs, in points.
    pub radius: f64,
    /// How far it falls right, in points.
    pub x: f64,
    /// How far it falls down, in points.
    pub y: f64,
}

impl Shadow {
    /// A shadow blurred over `radius` points, falling `y` points down.
    pub fn new(radius: f64, y: f64) -> Shadow {
        Shadow { color: Color::Rgba(0.0, 0.0, 0.0, 0.2), radius, x: 0.0, y }
    }

    /// A light, close shadow: cards on a background.
    pub fn soft() -> Shadow {
        Shadow { color: Color::Rgba(0.0, 0.0, 0.0, 0.12), radius: 6.0, x: 0.0, y: 2.0 }
    }

    /// A deeper shadow: something floating above the content.
    pub fn floating() -> Shadow {
        Shadow { color: Color::Rgba(0.0, 0.0, 0.0, 0.22), radius: 18.0, x: 0.0, y: 8.0 }
    }

    /// The same shadow in another colour.
    pub fn color(mut self, color: Color) -> Shadow {
        self.color = color;
        self
    }
}

/// What a surface draws.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Look {
    pub(crate) background: Option<Color>,
    pub(crate) radius: f64,
    pub(crate) border: Option<(Color, f64)>,
    pub(crate) shadow: Option<Shadow>,
    /// A picture as the content, and whether it fills (else fits).
    pub(crate) picture: Option<(Retained<objc2_app_kit::NSImage>, bool)>,
    /// A material or glass under the surface.
    pub(crate) backdrop: Option<crate::material::Backdrop>,
}

define_class!(
    // Draws a surface with its layer; passes clicks through.
    #[unsafe(super(NSView, objc2_app_kit::NSResponder, objc2::runtime::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitSurface"]
    #[ivars = RefCell<Look>]
    pub(crate) struct Surface;

    impl Surface {
        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        // Called when the view first shows and whenever the appearance
        // changes: the named colours resolve again here.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            crate::failure::guard("a surface (update_layer)", (), || {
                self.apply();
            })
        }

        // Printing draws views, not layers: the surface draws itself then.
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: objc2_foundation::NSRect) {
            crate::failure::guard("a surface (draw_rect)", (), || {
                if !objc2_app_kit::NSGraphicsContext::currentContextDrawingToScreen() {
                    self.draw_for_print();
                }
            })
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl Surface {
    pub(crate) fn new(mtm: MainThreadMarker, look: Look) -> Retained<Surface> {
        let this = Surface::alloc(mtm).set_ivars(RefCell::new(look));
        // SAFETY: NSView's designated initialiser.
        let this: Retained<Surface> = unsafe { msg_send![super(this), init] };
        this.setWantsLayer(true);
        this.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::OnSetNeedsDisplay);
        this.apply();
        this
    }

    /// Changes the background now (animated inside an animated change).
    pub(crate) fn set_background(&self, color: Color) {
        self.ivars().borrow_mut().background = Some(color);
        // Resolve named colours for this view's appearance, as in updateLayer.
        let block = block2::StackBlock::new(|| self.apply());
        objc2_app_kit::NSAppearanceCustomization::effectiveAppearance(&**self).performAsCurrentDrawingAppearance(&block);
    }

    /// Draws the background, picture and border into the current context.
    fn draw_for_print(&self) {
        use objc2_app_kit::{NSBezierPath, NSCompositingOperation, NSGraphicsContext};
        let look = self.ivars().borrow().clone();
        let bounds = self.bounds();
        let r = look.radius.min(bounds.size.width / 2.0).min(bounds.size.height / 2.0);
        let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, r, r);
        if let Some(c) = look.background {
            c.ns().setFill();
            path.fill();
        }
        if let Some((image, fill)) = &look.picture {
            let size = image.size();
            if size.width > 0.0 && size.height > 0.0 {
                let scale = (bounds.size.width / size.width).max(bounds.size.height / size.height);
                let scale = if *fill { scale } else { (bounds.size.width / size.width).min(bounds.size.height / size.height) };
                let (w, h) = (size.width * scale, size.height * scale);
                let rect = objc2_foundation::NSRect::new(
                    NSPoint::new(bounds.origin.x + (bounds.size.width - w) / 2.0, bounds.origin.y + (bounds.size.height - h) / 2.0),
                    objc2_foundation::NSSize::new(w, h),
                );
                NSGraphicsContext::saveGraphicsState_class();
                path.addClip();
                // SAFETY: drawing an image into the current context, the
                // right way up in this flipped view.
                unsafe { image.drawInRect_fromRect_operation_fraction_respectFlipped_hints(rect, objc2_foundation::NSRect::ZERO, NSCompositingOperation::SourceOver, 1.0, true, None) };
                NSGraphicsContext::restoreGraphicsState_class();
            }
        }
        if let Some((c, width)) = look.border {
            let inset = objc2_foundation::NSRect::new(
                NSPoint::new(bounds.origin.x + width / 2.0, bounds.origin.y + width / 2.0),
                objc2_foundation::NSSize::new(bounds.size.width - width, bounds.size.height - width),
            );
            let edge = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(inset, r, r);
            edge.setLineWidth(width);
            c.ns().setStroke();
            edge.stroke();
        }
    }

    fn apply(&self) {
        let Some(layer) = self.layer() else { return };
        let look = self.ivars().borrow().clone();
        // Inside updateLayer AppKit has made this view's appearance the
        // current one, so the named colours resolve for it.
        let cg = |c: Color| c.ns().CGColor();
        layer.setBackgroundColor(look.background.map(cg).as_deref());
        if let Some((image, fill)) = &look.picture {
            // SAFETY: an NSImage is a valid layer content on macOS, and the
            // gravity constants are Core Animation's own. AppKit resets the
            // contents of a view that updates its layer, so they're set here.
            unsafe {
                layer.setContents(Some(image.as_ref()));
                layer.setContentsGravity(if *fill { objc2_quartz_core::kCAGravityResizeAspectFill } else { objc2_quartz_core::kCAGravityResizeAspect });
            }
            layer.setMasksToBounds(true);
        }
        layer.setCornerRadius(look.radius);
        match look.border {
            Some((color, width)) => {
                layer.setBorderColor(Some(&cg(color)));
                layer.setBorderWidth(width);
            }
            None => layer.setBorderWidth(0.0),
        }
        match look.shadow {
            Some(s) => {
                let (color, opacity) = match s.color.components() {
                    Some((r, g, b, a)) => (Color::Rgb(r, g, b), a),
                    None => (s.color, 1.0),
                };
                layer.setShadowColor(Some(&cg(color)));
                layer.setShadowOpacity(opacity as f32);
                layer.setShadowRadius(s.radius);
                // The view is flipped: positive y falls down.
                layer.setShadowOffset(objc2_core_foundation::CGSize::new(s.x, -s.y));
                layer.setMasksToBounds(false);
            }
            None => layer.setShadowOpacity(0.0),
        }
    }
}

/// A view showing `image` through its layer: fitted inside (or filling and
/// cropped), with rounded corners.
pub(crate) fn picture(mtm: MainThreadMarker, image: Option<Retained<objc2_app_kit::NSImage>>, fill: bool, radius: f64) -> Retained<NSView> {
    Retained::into_super(Surface::new(mtm, Look { radius, picture: image.map(|i| (i, fill)), ..Look::default() }))
}
