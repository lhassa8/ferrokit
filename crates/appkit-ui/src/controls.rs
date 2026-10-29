//! Standard controls bound two ways to signals: the user's edits set the
//! signal (inside a batch); setting the signal updates the control.

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSSearchFieldDelegate, NSSecureTextField, NSSwitch,
    NSTextField, NSTextFieldDelegate, NSView,
};
use objc2_foundation::{NSNotification, NSString};

use crate::signal::{Effect, Signal, batch};
use crate::view::{ActionTarget, Cx, Node, Size, View, ViewExt, hstack, text};

define_class!(
    // Live edits of a text field → its signal.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiFieldDelegate"]
    #[ivars = Signal<String>]
    pub(crate) struct FieldDelegate;

    unsafe impl NSObjectProtocol for FieldDelegate {}

    unsafe impl NSControlTextEditingDelegate for FieldDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn did_change(&self, notification: &NSNotification) {
            let Some(field) = notification.object().and_then(|o| o.downcast::<NSTextField>().ok()) else { return };
            let value = field.stringValue().to_string();
            let signal = self.ivars().clone();
            batch(|| signal.set(value));
        }
    }

    unsafe impl NSTextFieldDelegate for FieldDelegate {}
    unsafe impl NSSearchFieldDelegate for FieldDelegate {}
);

impl FieldDelegate {
    pub(crate) fn new(mtm: MainThreadMarker, value: Signal<String>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(value);
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// A single-line text field bound to a `Signal<String>`.
pub struct TextField {
    value: Signal<String>,
    placeholder: Option<String>,
    on_submit: Option<Box<dyn Fn()>>,
    secure: bool,
}

impl TextField {
    /// Grey text shown while the field is empty.
    pub fn placeholder(mut self, text: &str) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// Runs (in a batch) when the user presses Return in the field.
    pub fn on_submit(mut self, f: impl Fn() + 'static) -> Self {
        self.on_submit = Some(Box::new(f));
        self
    }
}

impl View for TextField {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        let field: Retained<NSTextField> = if self.secure {
            Retained::into_super(NSSecureTextField::new(mtm))
        } else {
            NSTextField::new(mtm)
        };
        if let Some(p) = &self.placeholder {
            field.setPlaceholderString(Some(&NSString::from_str(p)));
        }
        let delegate = FieldDelegate::new(mtm, self.value.clone());
        // SAFETY: the delegate is weak in AppKit; the node keeps it alive.
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        let (mut node, _) = Node::leaf(
            cx,
            Retained::into_super(Retained::into_super(field.clone())),
            (true, false),
            Size { w: 120.0, h: 0.0 },
        );
        if let Some(f) = self.on_submit {
            let target = ActionTarget::new(mtm, f);
            // SAFETY: the target responds to `fire:`; the node keeps it alive.
            unsafe {
                field.setTarget(Some(&target));
                field.setAction(Some(sel!(fire:)));
            }
            node.keep(Retained::into_super(target));
        }
        let value = self.value;
        node.own(Effect::new(move || {
            let v = value.get();
            if field.stringValue().to_string() != v {
                field.setStringValue(&NSString::from_str(&v));
            }
        }));
        node.keep(Retained::into_super(delegate));
        node
    }
}

/// A text field bound two ways to `value`.
pub fn text_field(value: &Signal<String>) -> TextField {
    TextField { value: value.clone(), placeholder: None, on_submit: None, secure: false }
}

/// A password field (`NSSecureTextField`) bound two ways to `value`.
pub fn secure_field(value: &Signal<String>) -> TextField {
    TextField { value: value.clone(), placeholder: None, on_submit: None, secure: true }
}

fn state_of(on: bool) -> isize {
    if on { NSControlStateValueOn } else { NSControlStateValueOff }
}

/// Builds a two-state control: the user's clicks set `value`; setting it
/// updates the control.
fn bound_switch(cx: &mut Cx, control: Retained<NSView>, set_state: impl Fn(isize) + 'static, get_state: impl Fn() -> isize + 'static, value: Signal<bool>) -> (Node, Retained<NSObject>) {
    let v = value.clone();
    let get = std::rc::Rc::new(get_state);
    let g = get.clone();
    let target = ActionTarget::new(cx.mtm, move || v.set(g() == NSControlStateValueOn));
    let (mut node, _) = Node::leaf(cx, control, (false, false), Size::default());
    node.own(Effect::new(move || {
        let want = state_of(value.get());
        if get() != want {
            set_state(want);
        }
    }));
    (node, Retained::into_super(target))
}

/// A checkbox (`NSButton`) bound two ways to `value`.
pub struct Checkbox {
    title: String,
    value: Signal<bool>,
}

impl View for Checkbox {
    fn build(self, cx: &mut Cx) -> Node {
        let mtm = cx.mtm;
        // SAFETY: no target yet; set below.
        let b = unsafe { NSButton::checkboxWithTitle_target_action(&NSString::from_str(&self.title), None, None, mtm) };
        let (b1, b2) = (b.clone(), b.clone());
        let (mut node, target) = bound_switch(
            cx,
            Retained::into_super(Retained::into_super(b.clone())),
            move |s| b1.setState(s),
            move || b2.state(),
            self.value,
        );
        // SAFETY: the target responds to `fire:`; the node keeps it alive.
        unsafe {
            b.setTarget(Some(&target));
            b.setAction(Some(sel!(fire:)));
        }
        node.keep(target);
        node
    }
}

/// A checkbox titled `title`, bound two ways to `value`.
pub fn checkbox(title: &str, value: &Signal<bool>) -> Checkbox {
    Checkbox { title: title.into(), value: value.clone() }
}

/// The switch part of a [`toggle`].
struct SwitchView(Signal<bool>);

impl View for SwitchView {
    fn build(self, cx: &mut Cx) -> Node {
        let s = NSSwitch::new(cx.mtm);
        let (s1, s2) = (s.clone(), s.clone());
        let (mut node, target) = bound_switch(
            cx,
            Retained::into_super(Retained::into_super(s.clone())),
            move |v| s1.setState(v),
            move || s2.state(),
            self.0,
        );
        // SAFETY: the target responds to `fire:`; the node keeps it alive.
        unsafe {
            s.setTarget(Some(&target));
            s.setAction(Some(sel!(fire:)));
        }
        node.keep(target);
        node
    }
}

/// A labelled switch (`NSSwitch`) bound two ways to `value`.
pub fn toggle(title: &str, value: &Signal<bool>) -> impl View {
    // The switch itself is named too: VoiceOver reads it apart from the text.
    hstack((text(title), SwitchView(value.clone()).accessibility_label(title)))
}

/// Wires `control` to call `f` (in a batch) when the user changes it.
fn on_action(cx: &Cx, control: &objc2_app_kit::NSControl, f: impl Fn() + 'static) -> Retained<NSObject> {
    let target = ActionTarget::new(cx.mtm, f);
    // SAFETY: the target responds to `fire:`; the caller's node keeps it alive.
    unsafe {
        control.setTarget(Some(&target));
        control.setAction(Some(sel!(fire:)));
    }
    Retained::into_super(target)
}

/// A pop-up menu choosing one of several values.
pub struct Picker<T> {
    value: Signal<T>,
    options: Vec<(String, T)>,
}

impl<T: PartialEq + Clone + 'static> View for Picker<T> {
    fn build(self, cx: &mut Cx) -> Node {
        let popup = objc2_app_kit::NSPopUpButton::new(cx.mtm);
        for (title, _) in &self.options {
            popup.addItemWithTitle(&NSString::from_str(title));
        }
        let options = std::rc::Rc::new(self.options);
        let (value, opts, p) = (self.value.clone(), options.clone(), popup.clone());
        let target = on_action(cx, &popup, move || {
            if let Some((_, v)) = usize::try_from(p.indexOfSelectedItem()).ok().and_then(|i| opts.get(i)) {
                value.set(v.clone());
            }
        });
        let (mut node, _) =
            Node::leaf(cx, Retained::into_super(Retained::into_super(Retained::into_super(popup.clone()))), (false, false), Size::default());
        let value = self.value;
        node.own(Effect::new(move || {
            let current = value.get();
            if let Some(i) = options.iter().position(|(_, v)| *v == current)
                && popup.indexOfSelectedItem() != i as isize
            {
                popup.selectItemAtIndex(i as isize);
            }
        }));
        node.keep(target);
        node
    }
}

/// A pop-up menu bound two ways to `value`, offering `options` (title, value).
pub fn picker<T: PartialEq + Clone + 'static>(value: &Signal<T>, options: Vec<(&str, T)>) -> Picker<T> {
    Picker { value: value.clone(), options: options.into_iter().map(|(t, v)| (t.to_string(), v)).collect() }
}

/// A continuous slider bound two ways to `value`.
pub struct Slider {
    value: Signal<f64>,
    range: (f64, f64),
}

impl View for Slider {
    fn build(self, cx: &mut Cx) -> Node {
        let slider = objc2_app_kit::NSSlider::new(cx.mtm);
        slider.setMinValue(self.range.0);
        slider.setMaxValue(self.range.1);
        slider.setContinuous(true);
        let (value, s) = (self.value.clone(), slider.clone());
        let target = on_action(cx, &slider, move || value.set(s.doubleValue()));
        let (mut node, _) =
            Node::leaf(cx, Retained::into_super(Retained::into_super(slider.clone())), (true, false), Size { w: 100.0, h: 0.0 });
        let value = self.value;
        node.own(Effect::new(move || {
            let v = value.get();
            if slider.doubleValue() != v {
                slider.setDoubleValue(v);
            }
        }));
        node.keep(target);
        node
    }
}

/// A slider over `min..=max`, bound two ways to `value`.
pub fn slider(value: &Signal<f64>, min: f64, max: f64) -> Slider {
    Slider { value: value.clone(), range: (min, max) }
}

/// Up/down arrows stepping `value` within a range.
pub struct Stepper {
    value: Signal<f64>,
    range: (f64, f64),
    step: f64,
}

impl View for Stepper {
    fn build(self, cx: &mut Cx) -> Node {
        let stepper = objc2_app_kit::NSStepper::new(cx.mtm);
        stepper.setMinValue(self.range.0);
        stepper.setMaxValue(self.range.1);
        stepper.setIncrement(self.step);
        stepper.setValueWraps(false);
        let (value, s) = (self.value.clone(), stepper.clone());
        let target = on_action(cx, &stepper, move || value.set(s.doubleValue()));
        let (mut node, _) =
            Node::leaf(cx, Retained::into_super(Retained::into_super(stepper.clone())), (false, false), Size::default());
        let value = self.value;
        node.own(Effect::new(move || {
            let v = value.get();
            if stepper.doubleValue() != v {
                stepper.setDoubleValue(v);
            }
        }));
        node.keep(target);
        node
    }
}

/// A stepper over `min..=max` in steps of `step`, bound two ways to `value`.
pub fn stepper(value: &Signal<f64>, min: f64, max: f64, step: f64) -> Stepper {
    Stepper { value: value.clone(), range: (min, max), step }
}

/// A progress bar: `Some(fraction)` (0–1) or `None` for indeterminate.
pub struct Progress(Box<dyn Fn() -> Option<f64>>);

impl View for Progress {
    fn build(self, cx: &mut Cx) -> Node {
        let bar = objc2_app_kit::NSProgressIndicator::new(cx.mtm);
        bar.setStyle(objc2_app_kit::NSProgressIndicatorStyle::Bar);
        bar.setMinValue(0.0);
        bar.setMaxValue(1.0);
        let (mut node, _) =
            Node::leaf(cx, Retained::into_super(bar.clone()), (true, false), Size { w: 100.0, h: 0.0 });
        let f = self.0;
        node.own(Effect::new(move || match f() {
            Some(v) => {
                // SAFETY: nil sender.
                unsafe { bar.stopAnimation(None) };
                bar.setIndeterminate(false);
                bar.setDoubleValue(v.clamp(0.0, 1.0));
            }
            None => {
                bar.setIndeterminate(true);
                // SAFETY: nil sender.
                unsafe { bar.startAnimation(None) };
            }
        }));
        node
    }
}

/// A progress bar driven by `f` (reading signals): `Some(0.0..=1.0)` or
/// `None` while the amount is unknown.
pub fn progress(f: impl Fn() -> Option<f64> + 'static) -> Progress {
    Progress(Box::new(f))
}

/// An SF Symbol.
pub struct Image {
    symbol: String,
    description: Option<String>,
}

impl Image {
    /// What VoiceOver says. Without one the image is decorative and
    /// VoiceOver skips it (as for an icon beside a labelled control).
    pub fn description(mut self, text: &str) -> Self {
        self.description = Some(text.into());
        self
    }
}

impl View for Image {
    fn build(self, cx: &mut Cx) -> Node {
        let image = objc2_app_kit::NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(&self.symbol),
            self.description.as_deref().map(NSString::from_str).as_deref(),
        )
        .unwrap_or_else(|| panic!("no SF Symbol named {:?}", self.symbol));
        let view = objc2_app_kit::NSImageView::imageViewWithImage(&image, cx.mtm);
        let view: Retained<NSView> = Retained::into_super(Retained::into_super(view));
        if self.description.is_none() {
            // Decorative: hidden from VoiceOver (`accessibility_label` shows it again).
            crate::view::set_accessibility_element(&view, false);
        }
        Node::leaf(cx, view, (false, false), Size::default()).0
    }
}

/// An SF Symbol by name (e.g. `"star.fill"`). Panics if the name is unknown.
pub fn image(symbol: &str) -> Image {
    Image { symbol: symbol.into(), description: None }
}
