//! Standard controls bound two ways to signals: the user's edits set the
//! signal (inside a batch); setting the signal updates the control.

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSComboBoxDelegate, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSSearchFieldDelegate, NSSecureTextField, NSSwitch,
    NSTextField, NSTextFieldDelegate, NSView,
};
use objc2_foundation::{NSNotification, NSString};

use crate::signal::{Effect, Signal, batch};
use crate::appkit_views::{ActionTarget, text};
use crate::view::{Cx, Node, Size, View, ViewExt, hstack};

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
            let signal = *self.ivars();
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
        let delegate = FieldDelegate::new(mtm, self.value);
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
    TextField { value: *value, placeholder: None, on_submit: None, secure: false }
}

/// A password field (`NSSecureTextField`) bound two ways to `value`.
pub fn secure_field(value: &Signal<String>) -> TextField {
    TextField { value: *value, placeholder: None, on_submit: None, secure: true }
}

fn state_of(on: bool) -> isize {
    if on { NSControlStateValueOn } else { NSControlStateValueOff }
}

/// Builds a two-state control: the user's clicks set `value`; setting it
/// updates the control.
fn bound_switch(cx: &mut Cx, control: Retained<NSView>, set_state: impl Fn(isize) + 'static, get_state: impl Fn() -> isize + 'static, value: Signal<bool>) -> (Node, Retained<NSObject>) {
    let v = value;
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
    Checkbox { title: title.into(), value: *value }
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
    hstack((text(title), SwitchView(*value).accessibility_label(title)))
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
        let (value, opts, p) = (self.value, options.clone(), popup.clone());
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
    Picker { value: *value, options: options.into_iter().map(|(t, v)| (t.to_string(), v)).collect() }
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
        let (value, s) = (self.value, slider.clone());
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
    Slider { value: *value, range: (min, max) }
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
        let (value, s) = (self.value, stepper.clone());
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
    Stepper { value: *value, range: (min, max), step }
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
            <crate::platform::Native as crate::platform::Platform>::set_accessibility_element(&view, false);
        }
        Node::leaf(cx, view, (false, false), Size::default()).0
    }
}

/// An SF Symbol by name (e.g. `"star.fill"`). Panics if the name is unknown.
pub fn image(symbol: &str) -> Image {
    Image { symbol: symbol.into(), description: None }
}

// ---- G2: dates, colours, segments, combo boxes, search fields ----

/// A date (and optionally time) picker bound two ways to `value`.
pub struct DatePicker {
    value: Signal<std::time::SystemTime>,
    time: bool,
    calendar: bool,
    range: Option<(std::time::SystemTime, std::time::SystemTime)>,
}

impl DatePicker {
    /// Also picks the time of day.
    pub fn with_time(mut self) -> Self {
        self.time = true;
        self
    }

    /// A month calendar (clock too, with [`with_time`](Self::with_time))
    /// instead of a text field with a stepper.
    pub fn calendar(mut self) -> Self {
        self.calendar = true;
        self
    }

    /// Only dates from `min` to `max` can be picked.
    pub fn range(mut self, min: std::time::SystemTime, max: std::time::SystemTime) -> Self {
        self.range = Some((min, max));
        self
    }
}

fn ns_date(t: std::time::SystemTime) -> Retained<objc2_foundation::NSDate> {
    let secs = match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    };
    objc2_foundation::NSDate::dateWithTimeIntervalSince1970(secs)
}

fn system_time(d: &objc2_foundation::NSDate) -> std::time::SystemTime {
    let secs = d.timeIntervalSince1970();
    let delta = std::time::Duration::from_secs_f64(secs.abs());
    if secs >= 0.0 { std::time::UNIX_EPOCH + delta } else { std::time::UNIX_EPOCH - delta }
}

impl View for DatePicker {
    fn build(self, cx: &mut Cx) -> Node {
        use objc2_app_kit::{NSDatePickerElementFlags, NSDatePickerStyle};
        let picker = objc2_app_kit::NSDatePicker::new(cx.mtm);
        let mut elements = NSDatePickerElementFlags::YearMonthDay;
        if self.time {
            elements |= NSDatePickerElementFlags::HourMinute;
        }
        picker.setDatePickerElements(elements);
        picker.setDatePickerStyle(if self.calendar { NSDatePickerStyle::ClockAndCalendar } else { NSDatePickerStyle::TextFieldAndStepper });
        if let Some((min, max)) = self.range {
            picker.setMinDate(Some(&ns_date(min)));
            picker.setMaxDate(Some(&ns_date(max)));
        }
        let (value, p) = (self.value, picker.clone());
        let target = on_action(cx, &picker, move || value.set(system_time(&p.dateValue())));
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(picker.clone())), (false, false), Size::default());
        let value = self.value;
        node.own(Effect::new(move || {
            let want = ns_date(value.get());
            // A second's tolerance: the picker keeps whole seconds or minutes.
            if (picker.dateValue().timeIntervalSince1970() - want.timeIntervalSince1970()).abs() >= 1.0 {
                picker.setDateValue(&want);
            }
        }));
        node.keep(target);
        node
    }
}

/// A date picker bound two ways to `value` (date only; see
/// [`DatePicker::with_time`]).
pub fn date_picker(value: &Signal<std::time::SystemTime>) -> DatePicker {
    DatePicker { value: *value, time: false, calendar: false, range: None }
}

/// An sRGB colour, components from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red.
    pub r: f64,
    /// Green.
    pub g: f64,
    /// Blue.
    pub b: f64,
    /// Opacity.
    pub a: f64,
}

impl Color {
    /// An opaque colour.
    pub fn rgb(r: f64, g: f64, b: f64) -> Self {
        Color { r, g, b, a: 1.0 }
    }

    fn ns(self) -> Retained<objc2_app_kit::NSColor> {
        objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(self.r, self.g, self.b, self.a)
    }

    fn from_ns(c: &objc2_app_kit::NSColor) -> Option<Self> {
        let c = c.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace())?;
        Some(Color { r: c.redComponent(), g: c.greenComponent(), b: c.blueComponent(), a: c.alphaComponent() })
    }

    fn close_to(self, o: Color) -> bool {
        [(self.r, o.r), (self.g, o.g), (self.b, o.b), (self.a, o.a)].iter().all(|(a, b)| (a - b).abs() < 1.0 / 512.0)
    }
}

/// A colour well bound two ways to `value`: a click opens the system's
/// colour picker.
pub struct ColorWell(Signal<Color>);

impl View for ColorWell {
    fn build(self, cx: &mut Cx) -> Node {
        let well = objc2_app_kit::NSColorWell::new(cx.mtm);
        let (value, w) = (self.0, well.clone());
        let target = on_action(cx, &well, move || {
            if let Some(c) = Color::from_ns(&w.color()) {
                value.set(c);
            }
        });
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(well.clone())), (false, false), Size { w: 44.0, h: 24.0 });
        if let crate::view::Kind::Leaf(l) = &node.kind {
            l.content.set(Some(Size { w: 44.0, h: 24.0 }));
        }
        let value = self.0;
        node.own(Effect::new(move || {
            let want = value.get();
            if !Color::from_ns(&well.color()).is_some_and(|c| c.close_to(want)) {
                well.setColor(&want.ns());
            }
        }));
        node.keep(target);
        node
    }
}

/// A colour well bound two ways to `value`.
pub fn color_well(value: &Signal<Color>) -> ColorWell {
    ColorWell(*value)
}

/// A segmented control choosing one of several values.
pub struct Segmented<T> {
    value: Signal<T>,
    options: Vec<(String, T)>,
}

impl<T: PartialEq + Clone + 'static> View for Segmented<T> {
    fn build(self, cx: &mut Cx) -> Node {
        use objc2_app_kit::NSSegmentSwitchTracking;
        let control = objc2_app_kit::NSSegmentedControl::new(cx.mtm);
        control.setSegmentCount(self.options.len() as isize);
        control.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
        for (i, (title, _)) in self.options.iter().enumerate() {
            control.setLabel_forSegment(&NSString::from_str(title), i as isize);
            control.setWidth_forSegment(0.0, i as isize);
        }
        let options = std::rc::Rc::new(self.options);
        let (value, opts, c) = (self.value, options.clone(), control.clone());
        let target = on_action(cx, &control, move || {
            if let Some((_, v)) = usize::try_from(c.selectedSegment()).ok().and_then(|i| opts.get(i)) {
                value.set(v.clone());
            }
        });
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(control.clone())), (false, false), Size::default());
        let value = self.value;
        node.own(Effect::new(move || {
            let current = value.get();
            if let Some(i) = options.iter().position(|(_, v)| *v == current)
                && control.selectedSegment() != i as isize
            {
                control.setSelectedSegment(i as isize);
            }
        }));
        node.keep(target);
        node
    }
}

/// A segmented control bound two ways to `value`, one segment per option
/// (title, value). VoiceOver reads each segment's title.
pub fn segmented<T: PartialEq + Clone + 'static>(value: &Signal<T>, options: Vec<(&str, T)>) -> Segmented<T> {
    Segmented { value: *value, options: options.into_iter().map(|(t, v)| (t.to_string(), v)).collect() }
}

define_class!(
    // A combo box's typed text and chosen suggestions → its signal.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiComboDelegate"]
    #[ivars = Signal<String>]
    struct ComboDelegate;

    unsafe impl NSObjectProtocol for ComboDelegate {}

    unsafe impl NSControlTextEditingDelegate for ComboDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn did_change(&self, notification: &NSNotification) {
            let Some(field) = notification.object().and_then(|o| o.downcast::<NSTextField>().ok()) else { return };
            let value = field.stringValue().to_string();
            let signal = *self.ivars();
            batch(|| signal.set_if_changed(value));
        }
    }

    unsafe impl NSTextFieldDelegate for ComboDelegate {}

    unsafe impl NSComboBoxDelegate for ComboDelegate {
        #[unsafe(method(comboBoxSelectionDidChange:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            let Some(combo) = notification.object().and_then(|o| o.downcast::<objc2_app_kit::NSComboBox>().ok()) else { return };
            // The field shows the choice only after this notification: read
            // the chosen item itself.
            let chosen = combo.objectValueOfSelectedItem().and_then(|o| o.downcast::<NSString>().ok()).map(|s| s.to_string());
            if let Some(value) = chosen {
                let signal = *self.ivars();
                batch(|| signal.set_if_changed(value));
            }
        }
    }
);

/// A text field with a list of suggestions (`NSComboBox`).
pub struct ComboBox {
    value: Signal<String>,
    suggestions: Vec<String>,
}

impl View for ComboBox {
    fn build(self, cx: &mut Cx) -> Node {
        let combo = objc2_app_kit::NSComboBox::new(cx.mtm);
        for s in &self.suggestions {
            // SAFETY: the combo box takes object values; NSStrings are what
            // it shows and what the delegate reads back.
            unsafe { combo.addItemWithObjectValue(&NSString::from_str(s)) };
        }
        combo.setCompletes(true);
        let delegate = ComboDelegate::alloc(cx.mtm).set_ivars(self.value);
        // SAFETY: NSObject's designated initialiser.
        let delegate: Retained<ComboDelegate> = unsafe { msg_send![super(delegate), init] };
        // SAFETY: the delegate is weak in AppKit; the node keeps it.
        unsafe { combo.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(Retained::into_super(combo.clone()))), (true, false), Size { w: 120.0, h: 0.0 });
        let value = self.value;
        node.own(Effect::new(move || {
            let v = value.get();
            if combo.stringValue().to_string() != v {
                combo.setStringValue(&NSString::from_str(&v));
            }
        }));
        node.keep(Retained::into_super(delegate));
        node
    }
}

/// A text field bound two ways to `value`, with `suggestions` in a pop-up
/// list (typing completes from them).
pub fn combo_box(value: &Signal<String>, suggestions: Vec<&str>) -> ComboBox {
    ComboBox { value: *value, suggestions: suggestions.into_iter().map(String::from).collect() }
}

/// A search field (rounded, with a clear button) bound to `value`.
pub struct SearchField {
    value: Signal<String>,
    placeholder: Option<String>,
    on_submit: Option<Box<dyn Fn()>>,
}

impl SearchField {
    /// Grey text shown while it's empty (default "Search").
    pub fn placeholder(mut self, text: &str) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// Runs `f` on Return.
    pub fn on_submit(mut self, f: impl Fn() + 'static) -> Self {
        self.on_submit = Some(Box::new(f));
        self
    }
}

impl View for SearchField {
    fn build(self, cx: &mut Cx) -> Node {
        let field = objc2_app_kit::NSSearchField::new(cx.mtm);
        if let Some(p) = &self.placeholder {
            field.setPlaceholderString(Some(&NSString::from_str(p)));
        }
        // Each keystroke is a change (not only Return), like the toolbar's.
        field.setSendsSearchStringImmediately(true);
        let delegate = FieldDelegate::new(cx.mtm, self.value);
        // SAFETY: the delegate is weak in AppKit; the node keeps it.
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        // The clear button sends the action with empty text, but no
        // change notification: the action reads the field too.
        let (value, f, submit) = (self.value, field.clone(), self.on_submit);
        let target = on_action(cx, &field, move || {
            let text = f.stringValue().to_string();
            if text.is_empty() || value.get_untracked() != text {
                value.set_if_changed(text);
            } else if let Some(s) = &submit {
                s();
            }
        });
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(Retained::into_super(field.clone()))), (true, false), Size { w: 160.0, h: 0.0 });
        let value = self.value;
        node.own(Effect::new(move || {
            let v = value.get();
            if field.stringValue().to_string() != v {
                field.setStringValue(&NSString::from_str(&v));
            }
        }));
        node.keep(target);
        node.keep(Retained::into_super(delegate));
        node
    }
}

/// A search field bound two ways to `value` (updated as the user types).
pub fn search_field(value: &Signal<String>) -> SearchField {
    SearchField { value: *value, placeholder: None, on_submit: None }
}
