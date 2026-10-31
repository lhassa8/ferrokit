//! AppKit's own views for the node tree: text and labels, buttons, the
//! divider and the scroll view. The core (`view`) knows them only as
//! nodes; a second platform would provide its own (see `platform`).

use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSBox, NSBoxType, NSButton, NSResponder, NSScrollView, NSTextField, NSView};
use objc2_foundation::NSString;

use crate::signal::{Effect, batch};
use crate::view::{AnyView, Cx, Kind, LeafHandle, Meta, Node, Size, View, invalidate_layout};

/// A thin separator line across its stack: horizontal in a vertical stack,
/// vertical in a horizontal one.
pub struct Divider;

impl View for Divider {
    fn build(self, cx: &mut Cx) -> Node {
        let line = NSBox::new(cx.mtm);
        line.setBoxType(NSBoxType::Separator);
        let (mut node, _) = Node::leaf(cx, Retained::into_super(line), (false, false), Size { w: 1.0, h: 1.0 });
        if let Kind::Leaf(l) = &mut node.kind {
            l.cross_only = true;
            l.content.set(Some(Size { w: 1.0, h: 1.0 }));
        }
        node
    }
}

/// A separator line across its stack.
pub fn divider() -> Divider {
    Divider
}

define_class!(
    // A scroll view's document view: flipped, so content starts at the top.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitFlippedView"]
    struct FlippedView;

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

/// Vertically scrolling content (`NSScrollView`). The content is laid out at
/// the visible width and at least the visible height.
pub struct Scroll(AnyView);

impl View for Scroll {
    fn build(self, cx: &mut Cx) -> Node {
        let scroll = NSScrollView::new(cx.mtm);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        // SAFETY: NSView's designated initialiser.
        let doc: Retained<FlippedView> = unsafe { msg_send![FlippedView::alloc(cx.mtm), init] };
        let doc: Retained<NSView> = Retained::into_super(doc);
        scroll.setDocumentView(Some(&doc));
        cx.container.addSubview(&scroll);
        let content = self.0.build_node(&mut Cx::new(doc.clone()));
        invalidate_layout();
        let clip = scroll.contentView();
        let mut node = Node::new(Kind::Scroll { scroll, doc: doc.clone(), content: Box::new(content) });
        // Classic scroll bars appear and go when AppKit tiles the view (also
        // while drawing, after our layout): re-fit the content when the
        // visible width no longer matches it.
        let observer = ClipObserver::alloc(cx.mtm).set_ivars(ClipWatch { meta: Rc::downgrade(&node.meta), doc: objc2::rc::Weak::from_retained(&doc) });
        // SAFETY: NSObject's designated initialiser.
        let observer: Retained<ClipObserver> = unsafe { msg_send![super(observer), init] };
        clip.setPostsFrameChangedNotifications(true);
        // SAFETY: the observer answers clipChanged:; the node keeps it, and
        // the notification centre doesn't retain selector observers.
        unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &observer,
                objc2::sel!(clipChanged:),
                Some(objc2_app_kit::NSViewFrameDidChangeNotification),
                Some(&clip),
            )
        };
        node.hold(Rc::new(Observing(observer)));
        node
    }
}

/// Unregisters a notification observer when its node goes (the centre
/// would otherwise keep its record of it).
struct Observing(Retained<ClipObserver>);

impl Drop for Observing {
    fn drop(&mut self) {
        // SAFETY: removing an observer this module registered.
        unsafe { objc2_foundation::NSNotificationCenter::defaultCenter().removeObserver(&self.0) };
    }
}

struct ClipWatch {
    meta: Weak<Meta>,
    doc: objc2::rc::Weak<NSView>,
}

define_class!(
    // Watches a scroll view's visible area for size changes.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitClipObserver"]
    #[ivars = ClipWatch]
    struct ClipObserver;

    impl ClipObserver {
        #[unsafe(method(clipChanged:))]
        fn clip_changed(&self, note: &objc2_foundation::NSNotification) {
            let (Some(clip), Some(doc)) = (note.object().and_then(|o| o.downcast::<NSView>().ok()), self.ivars().doc.load()) else { return };
            if (clip.frame().size.width - doc.frame().size.width).abs() < 0.25 || crate::in_layout() {
                return;
            }
            if let Some(meta) = self.ivars().meta.upgrade() {
                meta.mark();
                crate::later(crate::relayout_now);
            }
        }
    }
);

/// Vertically scrolling `content`.
pub fn scroll(content: impl View) -> Scroll {
    Scroll(AnyView::new(content))
}

/// After a content change: re-measure, and mark the leaf (and its
/// ancestors) for layout only if its size changed.
pub(crate) fn remeasure(view: &NSView, leaf: &LeafHandle) {
    let s = view.fittingSize();
    let new = Size { w: s.width, h: s.height };
    if leaf.content.get() != Some(new) {
        leaf.content.set(Some(new));
        leaf.meta.mark();
    }
}

/// How a label's text looks: its font and colour.
#[derive(Clone, Default)]
struct TextLook {
    font: Option<crate::Font>,
    color: Option<crate::Color>,
}

impl TextLook {
    fn apply(&self, field: &NSTextField) {
        if let Some(font) = &self.font {
            field.setFont(Some(&font.ns()));
        }
        if let Some(color) = self.color {
            field.setTextColor(Some(&color.ns()));
        }
    }
}

/// A static label.
pub struct Text(String, TextLook);

impl Text {
    /// Its font: a text style (`Font::title()`, `Font::caption()`) or a
    /// custom one.
    pub fn font(mut self, font: crate::Font) -> Self {
        self.1.font = Some(font);
        self
    }

    /// Its colour (`Color::Secondary` for less prominent text).
    pub fn color(mut self, color: crate::Color) -> Self {
        self.1.color = Some(color);
        self
    }
}

define_class!(
    // A label that prints its text once: AppKit's labels write their text
    // into a printed PDF twice, a few points apart, which breaks searching
    // and copying the PDF (seen on macOS 27).
    #[unsafe(super(NSTextField, objc2_app_kit::NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitLabel"]
    struct PrintableLabel;

    impl PrintableLabel {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: objc2_foundation::NSRect) {
            if objc2_app_kit::NSGraphicsContext::currentContextDrawingToScreen() {
                // SAFETY: NSTextField's own drawing.
                let _: () = unsafe { msg_send![super(self), drawRect: dirty] };
                return;
            }
            let Some(cell) = self.cell() else { return };
            let mut r = cell.drawingRectForBounds(self.bounds());
            // The cell's text container pads each side by 2 points.
            r.origin.x += 2.0;
            r.size.width -= 4.0;
            let options = objc2_app_kit::NSStringDrawingOptions::UsesLineFragmentOrigin | objc2_app_kit::NSStringDrawingOptions::TruncatesLastVisibleLine;
            objc2_app_kit::NSAttributedStringNSExtendedStringDrawing::drawWithRect_options_context(&*self.attributedStringValue(), r, options, None);
        }
    }
);

/// A label (`NSTextField::labelWithString`) that prints its text once.
pub(crate) fn printable_label(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let _ = mtm;
    // SAFETY: NSTextField's class constructor, called on the subclass.
    let field: Retained<PrintableLabel> = unsafe { msg_send![<PrintableLabel as objc2::ClassType>::class(), labelWithString: &*NSString::from_str(text)] };
    Retained::into_super(field)
}

impl View for Text {
    fn build(self, cx: &mut Cx) -> Node {
        let field = printable_label(&self.0, cx.mtm);
        self.1.apply(&field);
        Node::leaf(cx, Retained::into_super(Retained::into_super(field)), (false, false), Size::default()).0
    }
}

/// A static label.
pub fn text(s: impl Into<String>) -> Text {
    Text(s.into(), TextLook::default())
}

/// A label computed from signals; updates in place.
pub struct Label(Box<dyn Fn() -> String>, TextLook, Option<Box<dyn Fn() -> crate::Color>>);

impl Label {
    /// Its font (see [`Text::font`]).
    pub fn font(mut self, font: crate::Font) -> Self {
        self.1.font = Some(font);
        self
    }

    /// Its colour (see [`Text::color`]).
    pub fn color(mut self, color: crate::Color) -> Self {
        self.1.color = Some(color);
        self
    }

    /// A colour computed from signals, updated in place (a balance red when
    /// it's negative).
    pub fn color_when(mut self, f: impl Fn() -> crate::Color + 'static) -> Self {
        self.2 = Some(Box::new(f));
        self
    }
}

impl View for Label {
    fn build(self, cx: &mut Cx) -> Node {
        let field = printable_label("", cx.mtm);
        self.1.apply(&field);
        let view: Retained<NSView> = Retained::into_super(Retained::into_super(field.clone()));
        let (mut node, handle) = Node::leaf(cx, view.clone(), (false, false), Size::default());
        let f = self.0;
        let fld = field.clone();
        node.own(Effect::new(move || {
            fld.setStringValue(&NSString::from_str(&f()));
            remeasure(&view, &handle);
        }));
        if let Some(color) = self.2 {
            node.own(Effect::new(move || field.setTextColor(Some(&color().ns()))));
        }
        node
    }
}

/// A label computed from signals; updates in place.
pub fn label(f: impl Fn() -> String + 'static) -> Label {
    Label(Box::new(f), TextLook::default(), None)
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitAction"]
    #[ivars = Box<dyn Fn()>]
    pub(crate) struct ActionTarget;

    impl ActionTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, _sender: Option<&AnyObject>) {
            batch(|| (self.ivars())());
        }
    }
);

impl ActionTarget {
    pub(crate) fn new(mtm: MainThreadMarker, f: impl Fn() + 'static) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Box::new(crate::signal::with_current_owner(f)) as Box<dyn Fn()>);
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// A push button.
pub struct Button {
    title: String,
    action: Box<dyn Fn()>,
    disabled: Option<Box<dyn Fn() -> bool>>,
    default_action: bool,
}

impl Button {
    /// Disables the button while `f` (reading signals) returns true.
    pub fn disabled(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Some(Box::new(f));
        self
    }

    /// Makes this the window's default button: blue, and Return presses it.
    pub fn default_action(mut self) -> Self {
        self.default_action = true;
        self
    }
}

impl View for Button {
    fn build(self, cx: &mut Cx) -> Node {
        let target = ActionTarget::new(cx.mtm, self.action);
        // SAFETY: the target responds to `fire:` and the node keeps it alive.
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(&self.title), Some(&target), Some(sel!(fire:)), cx.mtm)
        };
        if self.default_action {
            button.setKeyEquivalent(&NSString::from_str("\r"));
        }
        let (mut node, _) =
            Node::leaf(cx, Retained::into_super(Retained::into_super(button.clone())), (false, false), Size::default());
        if let Some(disabled) = self.disabled {
            node.own(Effect::new(move || button.setEnabled(!disabled())));
        }
        node.keep(Retained::into_super(target));
        node
    }
}

/// A push button; `action` runs inside a batch.
pub fn button(title: &str, action: impl Fn() + 'static) -> Button {
    Button { title: title.into(), action: Box::new(action), disabled: None, default_action: false }
}
