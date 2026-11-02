//! AppKit's own views for the node tree: text and labels, buttons, the
//! divider and the scroll view. The core (`view`) knows them only as
//! nodes; a second platform would provide its own (see `platform`).

use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSBox, NSBoxType, NSButton, NSResponder, NSScrollView, NSTextField, NSView};
use objc2_foundation::NSString;

use crate::signal::Effect;
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

/// How a label's text looks: its font.
#[derive(Clone, Default)]
struct TextLook {
    font: Option<crate::Font>,
}

impl TextLook {
    fn apply(&self, field: &NSTextField) {
        if let Some(font) = &self.font {
            field.setFont(Some(&font.ns()));
        }
    }
}

/// Text (see [`text`]).
pub struct Text {
    content: crate::Value<String>,
    look: TextLook,
    color: Option<crate::Value<crate::Color>>,
    truncates: bool,
}

/// [`Text`]: `label(f)` was the name for text computed from signals; `text`
/// takes a closure too.
pub type Label = Text;

impl Text {
    /// Its font: a text style (`Font::title()`, `Font::caption()`) or a
    /// custom one.
    pub fn font(mut self, font: crate::Font) -> Self {
        self.look.font = Some(font);
        self
    }

    /// Its colour (`Color::Secondary` for less prominent text), or a closure
    /// computing it from signals (a balance red when it's negative).
    pub fn color(mut self, color: impl crate::IntoValue<crate::Color>) -> Self {
        self.color = Some(color.into_value());
        self
    }

    /// A colour computed from signals (now [`color`](Self::color) takes a
    /// closure).
    #[deprecated(since = "0.4.0", note = "`color` takes a closure: `.color(move || ..)`")]
    pub fn color_when(self, f: impl Fn() -> crate::Color + 'static) -> Self {
        self.color(f)
    }

    /// On one line, shortened with "…" when there isn't room for it all,
    /// instead of pushing past its row (a file's path, a long value).
    pub fn truncate(mut self) -> Self {
        self.truncates = true;
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
        let field = printable_label("", cx.mtm);
        self.look.apply(&field);
        if self.truncates {
            field.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
            field.setMaximumNumberOfLines(1);
        }
        let view: Retained<NSView> = Retained::into_super(Retained::into_super(field.clone()));
        let (mut node, handle) = Node::leaf(cx, view.clone(), (false, false), Size::default());
        if self.truncates {
            node.shrinks = true;
        }
        match self.content {
            crate::Value::Fixed(s) => field.setStringValue(&NSString::from_str(&s)),
            content => {
                let fld = field.clone();
                node.own(Effect::new(move || {
                    fld.setStringValue(&NSString::from_str(&content.get()));
                    remeasure(&view, &handle);
                }));
            }
        }
        match self.color {
            Some(crate::Value::Fixed(c)) => field.setTextColor(Some(&c.ns())),
            Some(color) => node.own(Effect::new(move || field.setTextColor(Some(&color.get().ns())))),
            None => {}
        }
        node
    }
}

/// Text: fixed (`text("Packing list")`), or computed from signals and
/// updated in place (`text(move || format!("{} left", left.get()))`).
pub fn text(content: impl crate::IntoValue<String>) -> Text {
    Text { content: content.into_value(), look: TextLook::default(), color: None, truncates: false }
}

/// Text computed from signals: the same as [`text`] with a closure.
#[deprecated(since = "0.4.0", note = "`text` takes a closure: `text(move || ..)`")]
pub fn label(f: impl Fn() -> String + 'static) -> Text {
    text(f)
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
            crate::models::flush();
            crate::failure::entry("a control's action", (), || (self.ivars())());
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

/// How a button is drawn ([`Button::style`], [`MenuButton::style`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ButtonStyle {
    /// The standard rounded push button.
    #[default]
    Standard,
    /// No bezel: the title or symbol alone, highlighted when pressed.
    Borderless,
    /// A small capsule, for buttons inside text or a list row.
    Inline,
    /// A bezel that shows only on hover and press, for filter bars
    /// (Mail's "All" and "Unread" above a list).
    Recessed,
    /// A round button, for a symbol alone.
    Circular,
    /// The toolbar's look, for buttons in a bar of the content.
    Toolbar,
    /// Liquid Glass (macOS 26); the standard style before it.
    Glass,
}

impl ButtonStyle {
    fn apply(self, b: &NSButton) {
        use objc2_app_kit::NSBezelStyle;
        match self {
            ButtonStyle::Standard => b.setBezelStyle(NSBezelStyle::Push),
            ButtonStyle::Borderless => b.setBordered(false),
            ButtonStyle::Inline => b.setBezelStyle(NSBezelStyle::Badge),
            ButtonStyle::Recessed => b.setBezelStyle(NSBezelStyle::AccessoryBar),
            ButtonStyle::Circular => b.setBezelStyle(NSBezelStyle::Circular),
            ButtonStyle::Toolbar => b.setBezelStyle(NSBezelStyle::Toolbar),
            ButtonStyle::Glass => b.setBezelStyle(if crate::has_liquid_glass() { NSBezelStyle::Glass } else { NSBezelStyle::Push }),
        }
    }
}

/// A control's size, as in Apple's apps: small controls in inspectors and
/// dense panels, large ones for a welcome window's main actions.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ControlSize {
    /// The smallest, for tight spaces.
    Mini,
    /// Inspectors and panels.
    Small,
    /// The usual size.
    #[default]
    Regular,
    /// Prominent actions.
    Large,
}

impl ControlSize {
    pub(crate) fn ns(self) -> objc2_app_kit::NSControlSize {
        use objc2_app_kit::NSControlSize;
        match self {
            ControlSize::Mini => NSControlSize::Mini,
            ControlSize::Small => NSControlSize::Small,
            ControlSize::Regular => NSControlSize::Regular,
            ControlSize::Large => NSControlSize::Large,
        }
    }
}

/// A push button.
pub struct Button {
    title: crate::Value<String>,
    action: Box<dyn Fn()>,
    disabled: Option<crate::Value<bool>>,
    default_action: bool,
    symbol: Option<crate::Value<String>>,
    style: ButtonStyle,
    size: ControlSize,
    destructive: bool,
}

impl Button {
    /// Disables the button: `true`, or a closure reading signals (disabled
    /// while it returns true).
    pub fn disabled(mut self, when: impl crate::IntoValue<bool>) -> Self {
        self.disabled = Some(when.into_value());
        self
    }

    /// An SF Symbol on the button, before its title, or alone when the
    /// title is empty (the title is then what VoiceOver reads; give it one
    /// with `accessibility_label`). A closure changes it with signals: Play
    /// and Pause, `move || if playing.get() { "pause.fill" } else { "play.fill" }`.
    pub fn symbol(mut self, name: impl crate::IntoValue<String>) -> Self {
        self.symbol = Some(name.into_value());
        self
    }

    /// Makes this the window's default button: blue, and Return presses it.
    pub fn default_action(mut self) -> Self {
        self.default_action = true;
        self
    }

    /// How it's drawn: borderless, inline, recessed, circular, toolbar,
    /// glass.
    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// Mini, small, regular or large.
    pub fn control_size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Marks the action as destructive (Delete, Discard): AppKit draws it in
    /// red where the style allows, and it's never pressed by Return.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}

impl View for Button {
    fn build(self, cx: &mut Cx) -> Node {
        let target = ActionTarget::new(cx.mtm, self.action);
        // SAFETY: the target responds to `fire:` and the node keeps it alive.
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(&self.title.get()), Some(&target), Some(sel!(fire:)), cx.mtm)
        };
        if self.default_action {
            button.setKeyEquivalent(&NSString::from_str("\r"));
        }
        self.style.apply(&button);
        button.setControlSize(self.size.ns());
        if self.destructive {
            button.setHasDestructiveAction(true);
        }
        let view: Retained<NSView> = Retained::into_super(Retained::into_super(button.clone()));
        let (mut node, handle) = Node::leaf(cx, view.clone(), (false, false), Size::default());
        // Title and symbol, set again (and the button measured again) when
        // their signals change.
        let (title, symbol, b) = (self.title, self.symbol, button.clone());
        if !title.is_fixed() || symbol.is_some() {
            node.own(Effect::new(move || {
                let t = title.get();
                b.setTitle(&NSString::from_str(&t));
                if let Some(name) = symbol.as_ref().map(|s| s.get()) {
                    let image = objc2_app_kit::NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(&name), Some(&NSString::from_str(&t)));
                    b.setImage(image.as_deref());
                    b.setImagePosition(if t.is_empty() { objc2_app_kit::NSCellImagePosition::ImageOnly } else { objc2_app_kit::NSCellImagePosition::ImageLeading });
                }
                remeasure(&view, &handle);
            }));
        }
        match self.disabled {
            Some(crate::Value::Fixed(d)) => button.setEnabled(!d),
            Some(disabled) => node.own(Effect::new(move || button.setEnabled(!disabled.get()))),
            None => {}
        }
        node.keep(Retained::into_super(target));
        node
    }
}

/// A push button; `action` runs inside a batch. The title is fixed, or a
/// closure computing it from signals ("Show Sidebar" or "Hide Sidebar").
pub fn button(title: impl crate::IntoValue<String>, action: impl Fn() + 'static) -> Button {
    Button {
        title: title.into_value(),
        action: Box::new(action),
        disabled: None,
        default_action: false,
        symbol: None,
        style: ButtonStyle::Standard,
        size: ControlSize::Regular,
        destructive: false,
    }
}

/// The round "?" button that opens help for the window or panel it's in
/// (put it in the bottom corner of a settings pane or sheet). VoiceOver
/// reads it as "Help".
pub fn help_button(action: impl Fn() + 'static) -> HelpButton {
    HelpButton(Box::new(action))
}

/// See [`help_button`].
pub struct HelpButton(Box<dyn Fn()>);

impl View for HelpButton {
    fn build(self, cx: &mut Cx) -> Node {
        let target = ActionTarget::new(cx.mtm, self.0);
        // SAFETY: the target responds to `fire:` and the node keeps it alive.
        let button = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(""), Some(&target), Some(sel!(fire:)), cx.mtm) };
        button.setBezelStyle(objc2_app_kit::NSBezelStyle::HelpButton);
        objc2_app_kit::NSAccessibility::setAccessibilityLabel(&*button, Some(&NSString::from_str(&crate::t("Help"))));
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(button)), (false, false), Size::default());
        node.keep(Retained::into_super(target));
        node
    }
}

/// A button showing a menu of actions when clicked (a pull-down button:
/// "Export" with "CSV…", "JSON…", "PDF…"). The items are menu items, as in
/// the menu bar: enabled, checked and with key equivalents.
pub fn menu_button(title: &str, items: Vec<crate::MenuItem>) -> MenuButton {
    MenuButton { title: title.into(), items, symbol: None, style: ButtonStyle::Standard, size: ControlSize::Regular, disabled: None }
}

/// See [`menu_button`].
pub struct MenuButton {
    title: String,
    items: Vec<crate::MenuItem>,
    symbol: Option<String>,
    style: ButtonStyle,
    size: ControlSize,
    disabled: Option<crate::Value<bool>>,
}

impl MenuButton {
    /// An SF Symbol before the title, or alone when the title is empty
    /// (give it an `accessibility_label` then): `"ellipsis.circle"` for a
    /// "More" menu.
    pub fn symbol(mut self, name: &str) -> Self {
        self.symbol = Some(name.into());
        self
    }

    /// How it's drawn ([`ButtonStyle`]); borderless suits a symbol alone.
    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// Mini, small, regular or large.
    pub fn control_size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Disables it: `true`, or a closure reading signals.
    pub fn disabled(mut self, when: impl crate::IntoValue<bool>) -> Self {
        self.disabled = Some(when.into_value());
        self
    }
}

impl View for MenuButton {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        let button = objc2_app_kit::NSPopUpButton::initWithFrame_pullsDown(objc2_app_kit::NSPopUpButton::alloc(mtm), objc2_foundation::NSRect::ZERO, true);
        let (menu, targets) = crate::chrome::build_menu(mtm, self.items);
        // A pull-down button shows its first item as its title.
        let head = objc2_app_kit::NSMenuItem::new(mtm);
        head.setTitle(&NSString::from_str(&self.title));
        if let Some(name) = &self.symbol {
            head.setImage(objc2_app_kit::NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), Some(&NSString::from_str(&self.title))).as_deref());
        }
        menu.insertItem_atIndex(&head, 0);
        button.setMenu(Some(&menu));
        if self.symbol.is_some() && self.title.is_empty() {
            button.setImagePosition(objc2_app_kit::NSCellImagePosition::ImageOnly);
        }
        self.style.apply(&button);
        button.setControlSize(self.size.ns());
        let view: Retained<NSView> = Retained::into_super(Retained::into_super(Retained::into_super(button.clone())));
        let (mut node, _) = Node::leaf(cx, view, (false, false), Size::default());
        match self.disabled {
            Some(crate::Value::Fixed(d)) => button.setEnabled(!d),
            Some(disabled) => node.own(Effect::new(move || button.setEnabled(!disabled.get()))),
            None => {}
        }
        for t in targets {
            node.keep(t);
        }
        node
    }
}
