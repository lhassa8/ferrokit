//! Hover and click on any view: a transparent view framed by layout to the
//! view's rectangle (like a surface) that tracks the pointer, and, for a
//! clickable view, takes its clicks, dims the view while it's pressed, and
//! shows it to VoiceOver as a button.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSTrackingArea, NSTrackingAreaOptions, NSView};
use objc2_foundation::{NSCopying, NSPoint, NSRect, NSString};


/// How dim a pressed view gets (its own opacity times this).
const PRESSED: f64 = 0.55;

#[derive(Default)]
pub(crate) struct Handlers {
    hover: Option<Rc<dyn Fn(bool)>>,
    click: Option<Rc<dyn Fn()>>,
    /// The view's own native views (dimmed while pressed), with their
    /// opacity before the press.
    views: RefCell<Vec<(Retained<NSView>, f64)>>,
    pressed: std::cell::Cell<bool>,
    /// The name VoiceOver reads: given, or the text of these labels.
    label: RefCell<Option<String>>,
    texts: RefCell<Vec<Retained<objc2_app_kit::NSTextField>>>,
}

define_class!(
    #[unsafe(super(NSView, objc2_app_kit::NSResponder, objc2::runtime::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitInteraction"]
    #[ivars = Handlers]
    pub(crate) struct Interaction;

    impl Interaction {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        // Only a clickable view takes clicks; a hover-only one lets them
        // through to the views under it.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            if self.ivars().click.is_none() || self.isHidden() {
                return std::ptr::null_mut();
            }
            let local = self.convertPoint_fromView(point, unsafe { self.superview() }.as_deref());
            if objc2_foundation::NSPointInRect(local, self.bounds()) { self as *const Self as *mut NSView } else { std::ptr::null_mut() }
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.hover(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.hover(false);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            self.press(true);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.press(self.inside(event));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let was = self.ivars().pressed.get();
            self.press(false);
            if was && self.inside(event) {
                self.click();
            }
        }

        #[unsafe(method(isAccessibilityElement))]
        fn is_ax_element(&self) -> bool {
            self.ivars().click.is_some()
        }

        #[unsafe(method_id(accessibilityRole))]
        fn ax_role(&self) -> Option<Retained<NSString>> {
            // SAFETY: AppKit's role constant.
            Some(unsafe { objc2_app_kit::NSAccessibilityButtonRole }.copy())
        }

        #[unsafe(method_id(accessibilityLabel))]
        fn ax_label(&self) -> Option<Retained<NSString>> {
            let label = self.ivars().label.borrow().clone().unwrap_or_else(|| {
                let texts = self.ivars().texts.borrow();
                texts.iter().map(|t| t.stringValue().to_string()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", ")
            });
            Some(NSString::from_str(&label))
        }

        #[unsafe(method(accessibilityPerformPress))]
        fn ax_press(&self) -> bool {
            self.click();
            self.ivars().click.is_some()
        }
    }
);

impl Interaction {
    pub(crate) fn new(mtm: MainThreadMarker, hover: Option<Rc<dyn Fn(bool)>>, click: Option<Rc<dyn Fn()>>) -> Retained<Interaction> {
        let this = Interaction::alloc(mtm).set_ivars(Handlers { hover, click, ..Default::default() });
        // SAFETY: NSView's designated initialiser.
        let this: Retained<Interaction> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        if this.ivars().hover.is_some() {
            // SAFETY: a tracking area owned by the view, following its bounds.
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveInActiveApp | NSTrackingAreaOptions::InVisibleRect,
                    Some(&*(&*this as *const Interaction as *const AnyObject)),
                    None,
                )
            };
            this.addTrackingArea(&area);
        }
        this
    }

    /// The views a press dims, and the name VoiceOver reads for the button.
    /// VoiceOver reads `label`, or else the current text of `texts`.
    pub(crate) fn set_views(&self, views: Vec<Retained<NSView>>, label: Option<String>, texts: Vec<Retained<objc2_app_kit::NSTextField>>) {
        *self.ivars().views.borrow_mut() = views.into_iter().map(|v| (v, 1.0)).collect();
        *self.ivars().label.borrow_mut() = label;
        *self.ivars().texts.borrow_mut() = texts;
    }

    fn inside(&self, event: &NSEvent) -> bool {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        objc2_foundation::NSPointInRect(p, self.bounds())
    }

    fn hover(&self, on: bool) {
        if let Some(f) = self.ivars().hover.clone() {
            crate::failure::entry("a hover", (), || f(on));
        }
    }

    fn click(&self) {
        if let Some(f) = self.ivars().click.clone() {
            crate::failure::entry("a click", (), || f());
        }
    }

    fn press(&self, on: bool) {
        let ivars = self.ivars();
        if self.ivars().click.is_none() || ivars.pressed.replace(on) == on {
            return;
        }
        for (view, alpha) in ivars.views.borrow_mut().iter_mut() {
            if on {
                *alpha = view.alphaValue();
                view.setAlphaValue(*alpha * PRESSED);
            } else {
                view.setAlphaValue(*alpha);
            }
        }
    }
}

/// Whether `view` (a clickable view's interaction) is pressed (for checks).
pub(crate) fn is_pressed(view: &NSView) -> Option<bool> {
    view.downcast_ref::<Interaction>().map(|i| i.ivars().pressed.get())
}

/// Sends `view` (an interaction) the pointer entering or leaving (for
/// checks).
pub(crate) fn hover(view: &NSView, inside: bool) -> bool {
    let Some(i) = view.downcast_ref::<Interaction>() else { return false };
    i.hover(inside);
    true
}

/// Presses and releases a clickable view's interaction as a click does
/// (for checks: the press dims it, the release clicks).
pub(crate) fn press(view: &NSView, down: bool) -> bool {
    let Some(i) = view.downcast_ref::<Interaction>() else { return false };
    if down {
        i.press(true);
    } else {
        let was = i.ivars().pressed.get();
        i.press(false);
        if was {
            i.click();
        }
    }
    true
}
