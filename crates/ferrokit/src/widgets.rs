//! More standard controls: tabs, radio groups, links, gauges and ratings,
//! token fields and path controls. Each is AppKit's own control, bound two
//! ways to a signal like the controls in `controls.rs`.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSLevelIndicator, NSLevelIndicatorStyle, NSPathControl,
    NSTabView, NSTabViewDelegate, NSTabViewItem, NSTabViewType, NSTextField, NSTextFieldDelegate, NSTokenField, NSTokenFieldDelegate, NSView,
};
use objc2_foundation::{NSArray, NSAttributedString, NSDictionary, NSNotification, NSString, NSURL};

use crate::appkit_views::ActionTarget;
use crate::signal::{Effect, Signal, batch};
use crate::view::{AnyView, Axis, Cx, Kind, Node, Size, View, hstack, spacer, vstack};

fn as_view<T: objc2::ClassType>(v: &Retained<T>) -> Retained<NSView> {
    // SAFETY: only called with NSView subclasses.
    unsafe { Retained::cast_unchecked(v.clone()) }
}

fn wire(control: &objc2_app_kit::NSControl, target: &ActionTarget) {
    // SAFETY: the target responds to `fire:`; the caller's node keeps it.
    unsafe {
        control.setTarget(Some(target));
        control.setAction(Some(sel!(fire:)));
    }
}

// ---- tabs ----

/// How [`tabs`] shows its tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TabStyle {
    /// AppKit's tab view: tabs along the top of a bordered box, as in
    /// System Settings' older panes and many inspectors.
    #[default]
    Bordered,
    /// A segmented control above borderless content, as in Finder's Get
    /// Info sections and most modern Mac apps.
    Segmented,
}

/// Panes chosen by tabs (see [`tabs`]).
pub struct Tabs<T> {
    value: Signal<T>,
    pages: Vec<(String, T, AnyView)>,
    style: TabStyle,
}

impl<T> Tabs<T> {
    /// How the tabs look ([`TabStyle::Bordered`] by default).
    pub fn style(mut self, style: TabStyle) -> Self {
        self.style = style;
        self
    }
}

/// Releases the panes' view trees with the tab view.
struct Hosted(Vec<Retained<NSView>>);

impl Drop for Hosted {
    fn drop(&mut self) {
        for root in &self.0 {
            crate::drop_hosted(root);
        }
    }
}

define_class!(
    // A tab chosen by the user → the signal.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitTabDelegate"]
    #[ivars = Box<dyn Fn(usize)>]
    struct TabDelegate;

    unsafe impl NSObjectProtocol for TabDelegate {}

    unsafe impl NSTabViewDelegate for TabDelegate {
        #[unsafe(method(tabView:didSelectTabViewItem:))]
        fn did_select(&self, tab_view: &NSTabView, item: Option<&NSTabViewItem>) {
            crate::failure::guard("a control (did_select)", (), || {
                if let Some(item) = item {
                    let i = tab_view.indexOfTabViewItem(item);
                    if i >= 0 {
                        batch(|| (self.ivars())(i as usize));
                    }
                }
            })
        }
    }
);

/// The native tab view of [`Tabs`], with or without its own tabs.
struct TabView<T> {
    value: Signal<T>,
    pages: Vec<(String, T, AnyView)>,
    tabless: bool,
}

impl<T: PartialEq + Clone + 'static> View for TabView<T> {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        let tab_view = NSTabView::new(mtm);
        if self.tabless {
            tab_view.setTabViewType(NSTabViewType::NoTabsNoBorder);
        }
        let mut roots = Vec::new();
        let mut biggest = NSSizeLike::default();
        let mut values = Vec::new();
        for (title, value, view) in self.pages {
            // Each pane is laid out like a window's content, in the room
            // the tab view gives it.
            // At the top left, unless it fills the pane (a chart, a list).
            let view = AnyView::new(crate::view::zstack((view,)).alignment(crate::Align::Start, crate::Align::Start));
            let (root, size) = crate::host_view(view);
            biggest.w = biggest.w.max(size.width);
            biggest.h = biggest.h.max(size.height);
            // SAFETY: an item with no identifier object.
            let item = unsafe { NSTabViewItem::initWithIdentifier(objc2::AnyThread::alloc(), None) };
            item.setLabel(&NSString::from_str(&title));
            item.setView(Some(&root));
            tab_view.addTabViewItem(&item);
            roots.push(root);
            values.push(value);
        }
        let values = Rc::new(values);
        let (signal, vs) = (self.value, values.clone());
        let delegate = TabDelegate::alloc(mtm).set_ivars(Box::new(move |i: usize| {
            if let Some(v) = vs.get(i) {
                signal.set_if_changed(v.clone());
            }
        }) as Box<dyn Fn(usize)>);
        // SAFETY: NSObject's designated initialiser.
        let delegate: Retained<TabDelegate> = unsafe { msg_send![super(delegate), init] };
        tab_view.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The tabs and border around the largest pane.
        let chrome = {
            let frame = tab_view.frame().size;
            let content = tab_view.contentRect().size;
            (frame.width - content.width, frame.height - content.height)
        };
        let min = Size { w: biggest.w + chrome.0.max(0.0), h: biggest.h + chrome.1.max(0.0) };
        let (mut node, _) = Node::leaf(cx, as_view(&tab_view), (true, true), min);
        let value = self.value;
        node.own(Effect::new(move || {
            let current = value.get();
            if let Some(i) = values.iter().position(|v| *v == current) {
                let selected = tab_view.selectedTabViewItem().map(|t| tab_view.indexOfTabViewItem(&t));
                if selected != Some(i as isize) {
                    tab_view.selectTabViewItemAtIndex(i as isize);
                }
            }
        }));
        node.keep(Retained::into_super(delegate));
        node.hold(Rc::new(Hosted(roots)));
        node
    }
}

#[derive(Default)]
struct NSSizeLike {
    w: f64,
    h: f64,
}

impl<T: PartialEq + Clone + 'static> View for Tabs<T> {
    fn build(self, cx: &mut Cx) -> Node {
        match self.style {
            TabStyle::Bordered => TabView { value: self.value, pages: self.pages, tabless: false }.build(cx),
            TabStyle::Segmented => {
                let options: Vec<(String, T)> = self.pages.iter().map(|(t, v, _)| (t.clone(), v.clone())).collect();
                let picker = crate::controls::segmented(&self.value, options.iter().map(|(t, v)| (t.as_str(), v.clone())).collect());
                vstack((hstack((spacer(), picker, spacer())), TabView { value: self.value, pages: self.pages, tabless: true }))
                    .spacing(12.0)
                    .build(cx)
            }
        }
    }
}

/// Panes chosen by tabs, bound two ways to `value`: one per page (its tab
/// title, the value that selects it, its content). Every pane is built
/// once and keeps its state while hidden.
///
/// ```no_run
/// use ferrokit::*;
///
/// let page = Signal::new(0);
/// let view = tabs(&page, vec![
///     ("General", 0, AnyView::new(text("General settings"))),
///     ("Advanced", 1, AnyView::new(text("Advanced settings"))),
/// ])
/// .style(TabStyle::Segmented);
/// # let _ = view;
/// ```
pub fn tabs<T: PartialEq + Clone + 'static>(value: &Signal<T>, pages: Vec<(&str, T, AnyView)>) -> Tabs<T> {
    Tabs { value: *value, pages: pages.into_iter().map(|(t, v, p)| (t.to_string(), v, p)).collect(), style: TabStyle::default() }
}

// ---- radio group ----

/// Radio buttons choosing one of several values (see [`radio_group`]).
pub struct RadioGroup<T> {
    value: Signal<T>,
    options: Vec<(String, T)>,
    horizontal: bool,
}

impl<T> RadioGroup<T> {
    /// The buttons side by side instead of one above another.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }
}

impl<T: PartialEq + Clone + 'static> View for RadioGroup<T> {
    fn build(self, cx: &mut Cx) -> Node {
        let mut children = Vec::new();
        let mut buttons = Vec::new();
        let mut targets = Vec::new();
        for (title, v) in &self.options {
            // SAFETY: no target yet; set below.
            let b = unsafe { NSButton::radioButtonWithTitle_target_action(&NSString::from_str(title), None, None, cx.mtm) };
            let (signal, v) = (self.value, v.clone());
            let target = ActionTarget::new(cx.mtm, move || signal.set_if_changed(v.clone()));
            wire(&b, &target);
            let (node, _) = Node::leaf(cx, as_view(&b), (false, false), Size::default());
            children.push(node);
            buttons.push(b);
            targets.push(target);
        }
        let axis = if self.horizontal { Axis::Horizontal } else { Axis::Vertical };
        let mut node = Node::new(Kind::Stack { axis, spacing: if self.horizontal { 14.0 } else { 6.0 }, align: Some(crate::Align::Start), children });
        let (value, options) = (self.value, self.options);
        node.own(Effect::new(move || {
            let current = value.get();
            for (b, (_, v)) in buttons.iter().zip(&options) {
                let want = if *v == current { NSControlStateValueOn } else { NSControlStateValueOff };
                if b.state() != want {
                    b.setState(want);
                }
            }
        }));
        for t in targets {
            node.keep(Retained::into_super(t));
        }
        node
    }
}

/// Radio buttons bound two ways to `value`, one per option (title, value),
/// one above another (or [`horizontal`](RadioGroup::horizontal)).
pub fn radio_group<T: PartialEq + Clone + 'static>(value: &Signal<T>, options: Vec<(&str, T)>) -> RadioGroup<T> {
    RadioGroup { value: *value, options: options.into_iter().map(|(t, v)| (t.to_string(), v)).collect(), horizontal: false }
}

// ---- link ----

define_class!(
    // A label holding a link: VoiceOver reads it as a link, and pressing
    // it there opens the link as a click does.
    #[unsafe(super(NSTextField, objc2_app_kit::NSControl, NSView, objc2_app_kit::NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitLink"]
    #[ivars = Retained<NSURL>]
    struct LinkField;

    impl LinkField {
        #[unsafe(method_id(accessibilityRole))]
        fn ax_role(&self) -> Option<Retained<NSString>> {
            crate::failure::guard("a control (ax_role)", None, || {
                // SAFETY: AppKit's role constant.
                Some(objc2_foundation::NSCopying::copy(unsafe { objc2_app_kit::NSAccessibilityLinkRole }))
            })
        }

        #[unsafe(method(accessibilityPerformPress))]
        fn ax_press(&self) -> bool {
            crate::failure::guard("a control (ax_press)", false, || {
                objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(self.ivars())
            })
        }
    }
);

/// A link to a web page or file (see [`link`]).
pub struct Link {
    title: String,
    url: String,
}

impl View for Link {
    fn build(self, cx: &mut Cx) -> Node {
        let Some(url) = NSURL::URLWithString(&NSString::from_str(&self.url)) else {
            return crate::appkit_views::text(&self.title).build(cx);
        };
        let link = url.clone();
        // SAFETY: AppKit's attribute name, with an NSURL value.
        let attrs = unsafe {
            NSDictionary::from_retained_objects(
                &[objc2_app_kit::NSLinkAttributeName, objc2_app_kit::NSFontAttributeName],
                &[Retained::<AnyObject>::from(Retained::into_super(link)), Retained::<AnyObject>::from(Retained::into_super(objc2_app_kit::NSFont::systemFontOfSize(objc2_app_kit::NSFont::systemFontSize())))],
            )
        };
        // SAFETY: a string with plain attributes.
        let text = unsafe { NSAttributedString::initWithString_attributes(objc2::AnyThread::alloc(), &NSString::from_str(&self.title), Some(&attrs)) };
        let field = LinkField::alloc(cx.mtm).set_ivars(url.clone());
        // SAFETY: NSTextField's designated initialiser.
        let field: Retained<LinkField> = unsafe { msg_send![super(field), initWithFrame: objc2_foundation::NSRect::ZERO] };
        // A selectable label with link attributes opens the link when
        // clicked, with the pointing hand over it.
        field.setBezeled(false);
        field.setDrawsBackground(false);
        field.setEditable(false);
        field.setSelectable(true);
        field.setAllowsEditingTextAttributes(true);
        field.setAttributedStringValue(&text);
        Node::leaf(cx, as_view(&field), (false, false), Size::default()).0
    }
}

/// A link titled `title` that opens `url` (a web page, `mailto:`, or a
/// `file://` URL) in its app when clicked. VoiceOver reads it as a link.
pub fn link(title: &str, url: &str) -> Link {
    Link { title: title.into(), url: url.into() }
}

// ---- gauge and rating ----

/// A level indicator (see [`gauge`] and [`rating`]).
pub struct Gauge {
    value: GaugeValue,
    max: f64,
    style: NSLevelIndicatorStyle,
    warning: Option<f64>,
    critical: Option<f64>,
}

enum GaugeValue {
    Shown(Box<dyn Fn() -> f64>),
    Edited(Signal<f64>),
}

impl Gauge {
    /// Segments instead of a continuous bar.
    pub fn discrete(mut self) -> Self {
        self.style = NSLevelIndicatorStyle::DiscreteCapacity;
        self
    }

    /// Turns yellow from `value` up (or down, if it's above the critical
    /// value): a disk filling up, a battery running down.
    pub fn warning(mut self, value: f64) -> Self {
        self.warning = Some(value);
        self
    }

    /// Turns red from `value` up (or down, if it's below the warning value).
    pub fn critical(mut self, value: f64) -> Self {
        self.critical = Some(value);
        self
    }
}

impl View for Gauge {
    fn build(self, cx: &mut Cx) -> Node {
        let g = NSLevelIndicator::new(cx.mtm);
        g.setLevelIndicatorStyle(self.style);
        g.setMinValue(0.0);
        g.setMaxValue(self.max);
        if let Some(w) = self.warning {
            g.setWarningValue(w);
        }
        if let Some(c) = self.critical {
            g.setCriticalValue(c);
        }
        let rating = self.style == NSLevelIndicatorStyle::Rating;
        let flex = (!rating, false);
        let min = if rating { Size::default() } else { Size { w: 120.0, h: 0.0 } };
        let (mut node, _) = Node::leaf(cx, as_view(&g), flex, min);
        match self.value {
            GaugeValue::Shown(f) => {
                let g2 = g.clone();
                node.own(Effect::new(move || g2.setDoubleValue(f())));
            }
            GaugeValue::Edited(signal) => {
                g.setEditable(true);
                let g2 = g.clone();
                let target = ActionTarget::new(cx.mtm, move || signal.set_if_changed(g2.doubleValue()));
                wire(&g, &target);
                node.keep(Retained::into_super(target));
                let g3 = g.clone();
                node.own(Effect::new(move || {
                    let v = signal.get();
                    if g3.doubleValue() != v {
                        g3.setDoubleValue(v);
                    }
                }));
            }
        }
        node
    }
}

/// A level indicator showing `value()` out of `max` (0 to `max`), redrawn
/// when the signals it reads change: storage used, a battery, signal
/// strength. Give it a [`warning`](Gauge::warning) and
/// [`critical`](Gauge::critical) level to colour it.
pub fn gauge(value: impl Fn() -> f64 + 'static, max: f64) -> Gauge {
    Gauge { value: GaugeValue::Shown(Box::new(value)), max, style: NSLevelIndicatorStyle::ContinuousCapacity, warning: None, critical: None }
}

/// Stars from 0 to `max`, bound two ways to `value`: clicking a star sets
/// it, as in Music and Photos.
pub fn rating(value: &Signal<f64>, max: u32) -> Gauge {
    Gauge { value: GaugeValue::Edited(*value), max: max as f64, style: NSLevelIndicatorStyle::Rating, warning: None, critical: None }
}

// ---- token field ----

/// What a token field's delegate needs.
struct TokenState {
    value: Signal<Vec<String>>,
    suggestions: Vec<String>,
}

define_class!(
    // Edits of a token field → its signal; completions from suggestions.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitTokenDelegate"]
    #[ivars = TokenState]
    struct TokenDelegate;

    unsafe impl NSObjectProtocol for TokenDelegate {}

    unsafe impl NSControlTextEditingDelegate for TokenDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn did_change(&self, notification: &NSNotification) {
            crate::failure::guard("a control (did_change)", (), || {
                self.read(notification);
            })
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn did_end(&self, notification: &NSNotification) {
            crate::failure::guard("a control (did_end)", (), || {
                self.read(notification);
            })
        }
    }

    unsafe impl NSTextFieldDelegate for TokenDelegate {}

    unsafe impl NSTokenFieldDelegate for TokenDelegate {
        #[unsafe(method_id(tokenField:completionsForSubstring:indexOfToken:indexOfSelectedItem:))]
        fn completions(&self, _field: &NSTokenField, substring: &NSString, _index: isize, _selected: *mut isize) -> Option<Retained<NSArray>> {
            crate::failure::guard("a control (completions)", None, || {
                let typed = substring.to_string().to_lowercase();
                let matches: Vec<Retained<NSString>> =
                    self.ivars().suggestions.iter().filter(|s| s.to_lowercase().starts_with(&typed)).map(|s| NSString::from_str(s)).collect();
                Retained::into_super(NSArray::from_retained_slice(&matches)).downcast().ok()
            })
        }
    }
);

impl TokenDelegate {
    fn read(&self, notification: &NSNotification) {
        let Some(field) = notification.object().and_then(|o| o.downcast::<NSTokenField>().ok()) else { return };
        let tokens = tokens_of(&field);
        let signal = self.ivars().value;
        batch(|| signal.set_if_changed(tokens));
    }
}

/// The tokens a token field holds (finished ones; the text being typed
/// becomes one when it's done).
fn tokens_of(field: &NSTokenField) -> Vec<String> {
    let Some(value) = field.objectValue() else { return Vec::new() };
    match value.downcast::<NSArray>() {
        Ok(array) => array.iter().filter_map(|o| o.downcast::<NSString>().ok()).map(|s| s.to_string()).filter(|s| !s.is_empty()).collect(),
        Err(other) => other.downcast::<NSString>().ok().map(|s| s.to_string()).filter(|s| !s.is_empty()).into_iter().collect(),
    }
}

/// A token field (see [`token_field`]).
pub struct TokenField {
    value: Signal<Vec<String>>,
    suggestions: Vec<String>,
    placeholder: Option<String>,
}

impl TokenField {
    /// Completions offered as the user types (those starting with what
    /// they've typed, ignoring case).
    pub fn suggestions(mut self, suggestions: &[&str]) -> Self {
        self.suggestions = suggestions.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Grey text shown while it's empty.
    pub fn placeholder(mut self, text: &str) -> Self {
        self.placeholder = Some(text.into());
        self
    }
}

impl View for TokenField {
    fn build(self, cx: &mut Cx) -> Node {
        let field = NSTokenField::new(cx.mtm);
        if let Some(p) = &self.placeholder {
            field.setPlaceholderString(Some(&NSString::from_str(p)));
        }
        let delegate = TokenDelegate::alloc(cx.mtm).set_ivars(TokenState { value: self.value, suggestions: self.suggestions });
        // SAFETY: NSObject's designated initialiser.
        let delegate: Retained<TokenDelegate> = unsafe { msg_send![super(delegate), init] };
        // SAFETY: the node keeps the delegate alive as long as the field.
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        let (mut node, handle) = Node::leaf(cx, as_view(&field), (true, false), Size { w: 160.0, h: 0.0 });
        // A modest natural width: it fills the room it's given.
        handle.content.set(Some(Size { w: 160.0, h: field.fittingSize().height }));
        let value = self.value;
        node.own(Effect::new(move || {
            let tokens = value.get();
            if tokens_of(&field) != tokens {
                let strings: Vec<Retained<NSString>> = tokens.iter().map(|t| NSString::from_str(t)).collect();
                // SAFETY: an array of strings is a token field's value.
                unsafe { field.setObjectValue(Some(&NSArray::from_retained_slice(&strings))) };
            }
        }));
        node.keep(Retained::into_super(delegate));
        node
    }
}

/// A token field bound two ways to `value`: words or names as tokens
/// (tags, recipients), finished with Return or a comma.
pub fn token_field(value: &Signal<Vec<String>>) -> TokenField {
    TokenField { value: *value, suggestions: Vec::new(), placeholder: None }
}

// ---- path control ----

/// A path control (see [`path_control`]).
pub struct PathControl(Signal<PathBuf>);

impl View for PathControl {
    fn build(self, cx: &mut Cx) -> Node {
        let control = NSPathControl::new(cx.mtm);
        let signal = self.0;
        let c = control.clone();
        // A click on a folder in the path chooses it.
        let target = ActionTarget::new(cx.mtm, move || {
            let url = c.clickedPathItem().and_then(|i| i.URL()).or_else(|| c.URL());
            if let Some(path) = url.and_then(|u| u.path()) {
                signal.set_if_changed(PathBuf::from(path.to_string()));
            }
        });
        wire(&control, &target);
        // Its height is measured with a path in it.
        control.setURL(Some(&file_url(&crate::signal::untrack(|| signal.get()))));
        let (mut node, handle) = Node::leaf(cx, as_view(&control), (true, false), Size { w: 160.0, h: 0.0 });
        // Not the whole path's width: it fills the room it's given, and
        // shortens the middle folders to fit.
        handle.content.set(Some(Size { w: 160.0, h: control.fittingSize().height }));
        node.own(Effect::new(move || {
            let path = signal.get();
            let current = control.URL().and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string()));
            if current.as_deref() != Some(path.as_path()) {
                control.setURL(Some(&file_url(&path)));
            }
        }));
        node.keep(Retained::into_super(target));
        node
    }
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

/// The folders down to a file or folder, as the Finder's path bar shows
/// them, bound two ways to `path`: clicking a folder sets `path` to it.
pub fn path_control(path: &Signal<PathBuf>) -> PathControl {
    PathControl(*path)
}
