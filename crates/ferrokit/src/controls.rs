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

use crate::signal::{Effect, Signal, batch, untrack};
use ferrokit_core::text::Color;
use crate::appkit_views::{ActionTarget, text};
use crate::view::{Cx, Node, Size, View, ViewExt, hstack};

define_class!(
    // Live edits of a text field → its signal.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitFieldDelegate"]
    #[ivars = FieldState]
    pub(crate) struct FieldDelegate;

    unsafe impl NSObjectProtocol for FieldDelegate {}

    unsafe impl NSControlTextEditingDelegate for FieldDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn did_change(&self, notification: &NSNotification) {
            crate::failure::guard("a control (did_change)", (), || {
                let Some(field) = notification.object().and_then(|o| o.downcast::<NSTextField>().ok()) else { return };
                let value = field.stringValue().to_string();
                self.ivars().returned.set(false);
                let signal = self.ivars().value;
                batch(|| signal.set(value));
            })
        }

        // Return, noted for a search field's action (which a keystroke
        // sends too); AppKit goes on with the key as it would.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &objc2_app_kit::NSControl, _editor: &objc2_app_kit::NSTextView, command: objc2::runtime::Sel) -> bool {
            self.ivars().returned.set(command == sel!(insertNewline:));
            false
        }
    }

    unsafe impl NSTextFieldDelegate for FieldDelegate {}
    unsafe impl NSSearchFieldDelegate for FieldDelegate {}
);

/// A field's signal, and whether Return was the last key it took.
pub(crate) struct FieldState {
    value: Signal<String>,
    returned: std::cell::Cell<bool>,
}

impl FieldDelegate {
    /// Whether Return was pressed since the last change (asked once).
    fn take_return(&self) -> bool {
        self.ivars().returned.take()
    }

    pub(crate) fn new(mtm: MainThreadMarker, value: Signal<String>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FieldState { value, returned: std::cell::Cell::new(false) });
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
    /// `on_submit` also fires when editing ends without Return.
    on_end_editing: bool,
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

    /// Runs (in a batch) whenever editing ends: Return, Tab, or a click
    /// elsewhere, so an edit is never lost by leaving the field. The value
    /// may be unchanged (the user only passed through).
    pub fn on_commit(mut self, f: impl Fn() + 'static) -> Self {
        self.on_submit = Some(Box::new(f));
        self.on_end_editing = true;
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
            if self.on_end_editing
                && let Some(cell) = field.cell()
            {
                cell.setSendsActionOnEndEditing(true);
            }
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
    TextField { value: *value, placeholder: None, on_submit: None, on_end_editing: false, secure: false }
}

/// A password field (`NSSecureTextField`) bound two ways to `value`.
pub fn secure_field(value: &Signal<String>) -> TextField {
    TextField { value: *value, placeholder: None, on_submit: None, on_end_editing: false, secure: true }
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

/// An on-off button (a disclosure triangle…) bound two ways to `value`.
pub(crate) fn bound_toggle(cx: &mut Cx, view: Retained<objc2_app_kit::NSView>, b: Retained<NSButton>, value: Signal<bool>) -> Node {
    let (b1, b2) = (b.clone(), b.clone());
    let (mut node, target) = bound_switch(cx, view, move |s| b1.setState(s), move || b2.state(), value);
    // SAFETY: the target responds to `fire:`; the node keeps it alive.
    unsafe {
        b.setTarget(Some(&target));
        b.setAction(Some(sel!(fire:)));
    }
    node.keep(target);
    node
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

/// A labelled switch (`NSSwitch`) bound two ways to `value`. With an
/// empty title, the switch alone (a [`form_row`](crate::form_row) names it).
pub fn toggle(title: &str, value: &Signal<bool>) -> impl View {
    if title.is_empty() {
        return crate::view::AnyView::new(SwitchView(*value));
    }
    // The switch itself is named too: VoiceOver reads it apart from the text.
    crate::view::AnyView::new(hstack((text(title), SwitchView(*value).accessibility_label(title))))
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
        // Added to the menu itself: `addItemWithTitle:` removes an earlier
        // item with the same title, and the options are found by index.
        if let Some(menu) = popup.menu() {
            for (title, _) in &self.options {
                // SAFETY: a plain item; the pop-up button sends its own action.
                let item = unsafe { objc2_app_kit::NSMenuItem::initWithTitle_action_keyEquivalent(cx.mtm.alloc(), &NSString::from_str(title), None, &NSString::from_str("")) };
                menu.addItem(&item);
            }
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

/// A slider bound two ways to `value`.
pub struct Slider {
    value: Signal<f64>,
    range: (f64, f64),
    ticks: usize,
    snap: bool,
    shape: SliderShape,
    size: crate::ControlSize,
}

#[derive(Clone, Copy, PartialEq)]
enum SliderShape {
    Horizontal,
    Vertical,
    Circular,
}

impl Slider {
    /// `n` tick marks along it, from end to end (a 0–100 slider with 11
    /// ticks marks every 10).
    pub fn ticks(mut self, n: usize) -> Self {
        self.ticks = n;
        self
    }

    /// Only the tick marks' values can be picked (give it [`ticks`](Self::ticks)).
    pub fn snap_to_ticks(mut self) -> Self {
        self.snap = true;
        self
    }

    /// Upright, filling the height it's given (a volume or level control).
    pub fn vertical(mut self) -> Self {
        self.shape = SliderShape::Vertical;
        self
    }

    /// A dial, turned around its centre (an angle or a rotation); `min` is
    /// at the top.
    pub fn circular(mut self) -> Self {
        self.shape = SliderShape::Circular;
        self
    }

    /// Mini, small, regular or large.
    pub fn control_size(mut self, size: crate::ControlSize) -> Self {
        self.size = size;
        self
    }
}

impl View for Slider {
    fn build(self, cx: &mut Cx) -> Node {
        let slider = objc2_app_kit::NSSlider::new(cx.mtm);
        slider.setMinValue(self.range.0);
        slider.setMaxValue(self.range.1);
        slider.setContinuous(true);
        slider.setControlSize(self.size.ns());
        if self.ticks > 0 {
            slider.setNumberOfTickMarks(self.ticks as isize);
            slider.setAllowsTickMarkValuesOnly(self.snap);
        }
        // Grows along its length; a dial keeps its own size.
        let (grow, base) = match self.shape {
            SliderShape::Horizontal => ((true, false), Size { w: 100.0, h: 0.0 }),
            SliderShape::Vertical => {
                slider.setVertical(true);
                ((false, true), Size { w: 0.0, h: 100.0 })
            }
            SliderShape::Circular => {
                slider.setSliderType(objc2_app_kit::NSSliderType::Circular);
                ((false, false), Size::default())
            }
        };
        let (value, s) = (self.value, slider.clone());
        let target = on_action(cx, &slider, move || value.set(s.doubleValue()));
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(slider.clone())), grow, base);
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

/// A slider over `min..=max`, bound two ways to `value`: horizontal and
/// continuous unless given [`ticks`](Slider::ticks), [`vertical`](Slider::vertical)
/// or [`circular`](Slider::circular).
pub fn slider(value: &Signal<f64>, min: f64, max: f64) -> Slider {
    Slider { value: *value, range: (min, max), ticks: 0, snap: false, shape: SliderShape::Horizontal, size: crate::ControlSize::Regular }
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

/// A progress bar or spinner: `Some(fraction)` (0–1) or `None` for
/// indeterminate.
pub struct Progress {
    f: Box<dyn Fn() -> Option<f64>>,
    spinner: bool,
    size: crate::ControlSize,
}

impl Progress {
    /// A spinner instead of a bar: spinning while the amount is unknown, a
    /// filling circle while it's known. For "loading" beside a label or in a
    /// toolbar.
    pub fn spinner(mut self) -> Self {
        self.spinner = true;
        self
    }

    /// Mini, small, regular or large (a small spinner suits a text line).
    pub fn control_size(mut self, size: crate::ControlSize) -> Self {
        self.size = size;
        self
    }
}

impl View for Progress {
    fn build(self, cx: &mut Cx) -> Node {
        use objc2_app_kit::NSProgressIndicatorStyle;
        let bar = objc2_app_kit::NSProgressIndicator::new(cx.mtm);
        bar.setStyle(if self.spinner { NSProgressIndicatorStyle::Spinning } else { NSProgressIndicatorStyle::Bar });
        bar.setControlSize(self.size.ns());
        bar.setMinValue(0.0);
        bar.setMaxValue(1.0);
        let (grow, base) = if self.spinner {
            bar.sizeToFit();
            let s = bar.frame().size;
            ((false, false), Size { w: s.width, h: s.height })
        } else {
            ((true, false), Size { w: 100.0, h: 0.0 })
        };
        let (mut node, _) = Node::leaf(cx, Retained::into_super(bar.clone()), grow, base);
        if self.spinner
            && let crate::view::Kind::Leaf(l) = &node.kind
        {
            l.content.set(Some(base));
        }
        let f = self.f;
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
/// `None` while the amount is unknown. [`spinner`](Progress::spinner) makes
/// it a spinner.
pub fn progress(f: impl Fn() -> Option<f64> + 'static) -> Progress {
    Progress { f: Box::new(f), spinner: false, size: crate::ControlSize::Regular }
}

/// An image: an SF Symbol ([`image`]), or a picture from a file or bytes
/// ([`image_file`], [`image_data`]).
pub struct Image {
    source: ImageSource,
    description: Option<String>,
    symbol: SymbolLook,
    fill: bool,
    radius: f64,
    effects: Vec<(SymbolEffect, EffectWhen)>,
}

/// An SF Symbol animation ([`Image::effect`], [`Image::effect_on`]), drawn
/// by AppKit (macOS 14; Wiggle, Rotate and Breathe from macOS 15; nothing
/// on older systems). AppKit tones them down under Reduce motion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SymbolEffect {
    /// A bounce: a new message, an item added.
    Bounce,
    /// A fade in and out: waiting, recording.
    Pulse,
    /// Layers lit one after another: searching, connecting (`"wifi"`).
    VariableColor,
    /// Grown slightly while it lasts: selected, hovered.
    Scale,
    /// A shake: an error, attention needed.
    Wiggle,
    /// A turn: syncing, refreshing (`"arrow.triangle.2.circlepath"`).
    Rotate,
    /// A slow swell: alive, listening.
    Breathe,
}

impl SymbolEffect {
    fn class_name(self) -> &'static std::ffi::CStr {
        match self {
            SymbolEffect::Bounce => c"NSSymbolBounceEffect",
            SymbolEffect::Pulse => c"NSSymbolPulseEffect",
            SymbolEffect::VariableColor => c"NSSymbolVariableColorEffect",
            SymbolEffect::Scale => c"NSSymbolScaleEffect",
            SymbolEffect::Wiggle => c"NSSymbolWiggleEffect",
            SymbolEffect::Rotate => c"NSSymbolRotateEffect",
            SymbolEffect::Breathe => c"NSSymbolBreatheEffect",
        }
    }

    /// `[NSSymbol…Effect effect]`, if this macOS has it.
    fn object(self) -> Option<Retained<objc2::runtime::AnyObject>> {
        let class = objc2::runtime::AnyClass::get(self.class_name())?;
        // SAFETY: every NSSymbolEffect subclass answers +effect with an instance.
        unsafe { msg_send![class, effect] }
    }
}

enum EffectWhen {
    /// Runs while the value is true.
    While(crate::Value<bool>),
    /// Plays once each time the closure's result changes.
    On(Box<dyn Fn() -> u64>),
}

thread_local! {
    /// Symbol effects started and not stopped, and one-shot effects
    /// played, for tests ([`testing::symbol_effects`](crate::testing::symbol_effects)):
    /// AppKit doesn't say which are running.
    pub(crate) static SYMBOL_EFFECTS: std::cell::Cell<(isize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

enum ImageSource {
    Symbol(String),
    File(std::path::PathBuf),
    Data(Vec<u8>),
}

#[derive(Default)]
struct SymbolLook {
    size: Option<f64>,
    weight: Option<crate::Weight>,
    color: Option<Color>,
    rendering: Option<Rendering>,
}

enum Rendering {
    Hierarchical(Color),
    Palette(Vec<Color>),
    Multicolor,
}

impl Image {
    /// What VoiceOver says. Without one the image is decorative and
    /// VoiceOver skips it (as for an icon beside a labelled control).
    pub fn description(mut self, text: &str) -> Self {
        self.description = Some(text.into());
        self
    }

    /// A symbol's size in points (as text of that size would be).
    pub fn symbol_size(mut self, points: f64) -> Self {
        self.symbol.size = Some(points);
        self
    }

    /// A symbol's stroke weight, to match the text beside it.
    pub fn symbol_weight(mut self, weight: crate::Weight) -> Self {
        self.symbol.weight = Some(weight);
        self
    }

    /// A symbol's colour (one colour for all of it).
    pub fn color(mut self, color: Color) -> Self {
        self.symbol.color = Some(color);
        self
    }

    /// A symbol in shades of `color`: its layers from full to faint.
    pub fn hierarchical(mut self, color: Color) -> Self {
        self.symbol.rendering = Some(Rendering::Hierarchical(color));
        self
    }

    /// A symbol's layers in these colours, in order.
    pub fn palette(mut self, colors: &[Color]) -> Self {
        self.symbol.rendering = Some(Rendering::Palette(colors.to_vec()));
        self
    }

    /// A symbol in its own colours, where it has them (`"cloud.sun.fill"`).
    pub fn multicolor(mut self) -> Self {
        self.symbol.rendering = Some(Rendering::Multicolor);
        self
    }

    /// A picture fills its frame, cropping what doesn't fit (a cover, an
    /// avatar). By default it fits inside, all of it shown.
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }

    /// A symbol effect running while `active` is true (`true`, or a
    /// closure reading signals): `image("wifi").effect(SymbolEffect::VariableColor,
    /// move || searching.get())`. For the repeating effects: pulse, variable
    /// colour, scale, rotate, breathe.
    pub fn effect(mut self, effect: SymbolEffect, active: impl crate::IntoValue<bool>) -> Self {
        self.effects.push((effect, EffectWhen::While(active.into_value())));
        self
    }

    /// A symbol effect played once each time `trigger`'s value changes
    /// (it reads signals): `image("bell").effect_on(SymbolEffect::Bounce,
    /// move || unread.get())`. For the one-shot effects: bounce, wiggle,
    /// rotate.
    pub fn effect_on<T: std::hash::Hash>(mut self, effect: SymbolEffect, trigger: impl Fn() -> T + 'static) -> Self {
        self.effects.push((
            effect,
            EffectWhen::On(Box::new(move || {
                use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
                BuildHasherDefault::<DefaultHasher>::default().hash_one(trigger())
            })),
        ));
        self
    }

    /// Rounds the picture's corners, in points (half its size for a
    /// circle). For a background's corners, see
    /// [`ViewExt::corner_radius`](crate::ViewExt::corner_radius).
    pub fn corner_radius(mut self, points: f64) -> Self {
        self.radius = points;
        self
    }
}

impl View for Image {
    fn build(mut self, cx: &mut Cx) -> Node {
        use objc2_app_kit::{NSImage, NSImageSymbolConfiguration, NSImageView};
        let mut effects: Vec<Effect> = Vec::new();
        let description = self.description.as_deref().map(NSString::from_str);
        let (view, size): (Retained<NSView>, Option<Size>) = match &self.source {
            ImageSource::Symbol(name) => {
                // An unknown name (a symbol newer than this macOS, a typo)
                // is reported and shows a placeholder: the app's views
                // still build.
                let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), description.as_deref()).unwrap_or_else(|| {
                    crate::failure::report(crate::Failure { place: "an image", message: format!("no SF Symbol named {name:?} on this macOS; a placeholder is shown"), exception: false });
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("questionmark.square.dashed"), description.as_deref()).unwrap_or_default()
                });
                let view = NSImageView::imageViewWithImage(&image, cx.mtm);
                let look = &self.symbol;
                let mut config: Option<Retained<NSImageSymbolConfiguration>> = None;
                let mut add = |c: Retained<NSImageSymbolConfiguration>| {
                    config = Some(match config.take() {
                        Some(prev) => prev.configurationByApplyingConfiguration(&c),
                        None => c,
                    });
                };
                if look.size.is_some() || look.weight.is_some() {
                    let size = look.size.unwrap_or_else(objc2_app_kit::NSFont::systemFontSize);
                    add(NSImageSymbolConfiguration::configurationWithPointSize_weight(size, look.weight.unwrap_or(crate::Weight::Regular).ns()));
                }
                match &look.rendering {
                    Some(Rendering::Hierarchical(c)) => add(NSImageSymbolConfiguration::configurationWithHierarchicalColor(&c.ns())),
                    Some(Rendering::Palette(cs)) => {
                        let colors: Vec<_> = cs.iter().map(|c| c.ns()).collect();
                        add(NSImageSymbolConfiguration::configurationWithPaletteColors(&objc2_foundation::NSArray::from_retained_slice(&colors)));
                    }
                    Some(Rendering::Multicolor) => add(NSImageSymbolConfiguration::configurationPreferringMulticolor()),
                    None => {}
                }
                if let Some(config) = &config {
                    view.setSymbolConfiguration(Some(config));
                }
                if let Some(c) = look.color {
                    view.setContentTintColor(Some(&c.ns()));
                }
                for (effect, when) in std::mem::take(&mut self.effects) {
                    let Some(object) = effect.object() else { continue };
                    let v = view.clone();
                    // SAFETY: NSSymbolEffectOptions answers +options.
                    let options: Option<Retained<objc2::runtime::AnyObject>> = objc2::runtime::AnyClass::get(c"NSSymbolEffectOptions").and_then(|c| unsafe { msg_send![c, options] });
                    let Some(options) = options else { continue };
                    match when {
                        EffectWhen::While(active) => {
                            let on = std::cell::Cell::new(false);
                            effects.push(Effect::new(move || {
                                let want = active.get();
                                if want == on.replace(want) {
                                    return;
                                }
                                // SAFETY: NSImageView's symbol-effect methods (macOS 14),
                                // with an effect and options from the classes above.
                                unsafe {
                                    if want {
                                        let _: () = msg_send![&*v, addSymbolEffect: &*object, options: &*options, animated: true];
                                    } else {
                                        let _: () = msg_send![&*v, removeSymbolEffectOfType: &*object, options: &*options, animated: true];
                                    }
                                }
                                SYMBOL_EFFECTS.with(|c| {
                                    let (running, played) = c.get();
                                    c.set((running + if want { 1 } else { -1 }, played));
                                });
                            }));
                        }
                        EffectWhen::On(trigger) => {
                            let last = std::cell::Cell::new(None);
                            effects.push(Effect::new(move || {
                                let now = trigger();
                                // Not on the first run: only changes play it.
                                if last.replace(Some(now)).is_some_and(|before| before != now) {
                                    // SAFETY: as above.
                                    unsafe {
                                        let _: () = msg_send![&*v, addSymbolEffect: &*object, options: &*options, animated: true];
                                    }
                                    SYMBOL_EFFECTS.with(|c| {
                                        let (running, played) = c.get();
                                        c.set((running, played + 1));
                                    });
                                }
                            }));
                        }
                    }
                }
                (Retained::into_super(Retained::into_super(view)), None)
            }
            ImageSource::File(_) | ImageSource::Data(_) => {
                let image = match &self.source {
                    ImageSource::File(path) => NSImage::initWithContentsOfFile(objc2::AnyThread::alloc(), &NSString::from_str(&path.to_string_lossy())),
                    ImageSource::Data(bytes) => NSImage::initWithData(objc2::AnyThread::alloc(), &objc2_foundation::NSData::with_bytes(bytes)),
                    ImageSource::Symbol(_) => unreachable!(),
                };
                let size = image.as_ref().map(|i| i.size()).map(|s| Size { w: s.width, h: s.height });
                let view = crate::surface::picture(cx.mtm, image, self.fill, self.radius);
                if let Some(d) = &description {
                    use objc2_app_kit::NSAccessibility;
                    view.setAccessibilityElement(true);
                    view.setAccessibilityRole(Some(unsafe { objc2_app_kit::NSAccessibilityImageRole }));
                    view.setAccessibilityLabel(Some(d));
                }
                (view, size)
            }
        };
        if self.radius > 0.0 && matches!(self.source, ImageSource::Symbol(_)) {
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                layer.setCornerRadius(self.radius);
                layer.setMasksToBounds(true);
            }
        }
        if self.description.is_none() {
            // Decorative: hidden from VoiceOver (`accessibility_label` shows it again).
            <crate::platform::Native as crate::platform::Platform>::set_accessibility_element(&view, false);
        }
        let (mut node, _) = Node::leaf(cx, view, (false, false), Size::default());
        if let (Some(size), crate::view::Kind::Leaf(l)) = (size, &node.kind) {
            l.content.set(Some(size));
        }
        for e in effects {
            node.own(e);
        }
        node
    }
}

fn new_image(source: ImageSource) -> Image {
    Image { source, description: None, symbol: SymbolLook::default(), fill: false, radius: 0.0, effects: Vec::new() }
}

/// An SF Symbol by name (e.g. `"star.fill"`). A name this macOS doesn't
/// have (a newer symbol, a typo) shows a placeholder and is reported as a
/// failure (see [`App::on_error`](crate::App::on_error)).
pub fn image(symbol: &str) -> Image {
    new_image(ImageSource::Symbol(symbol.into()))
}

/// A picture from a file (PNG, JPEG, HEIC, PDF…), shown at its own size
/// unless given one (`.frame(..)`). An unreadable file shows nothing.
pub fn image_file(path: impl AsRef<std::path::Path>) -> Image {
    new_image(ImageSource::File(path.as_ref().to_path_buf()))
}

/// A picture from bytes in a file format (PNG, JPEG…): one built into the
/// app with `include_bytes!`.
pub fn image_data(bytes: &[u8]) -> Image {
    new_image(ImageSource::Data(bytes.to_vec()))
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

/// A time-of-day picker (hour and minute, in the user's 12- or 24-hour
/// format) bound two ways to `value`.
pub struct TimePicker(Signal<(u32, u32)>);

impl View for TimePicker {
    fn build(self, cx: &mut Cx) -> Node {
        use objc2_app_kit::{NSDatePickerElementFlags, NSDatePickerStyle};
        use objc2_foundation::{NSCalendar, NSCalendarOptions, NSCalendarUnit};
        let picker = objc2_app_kit::NSDatePicker::new(cx.mtm);
        picker.setDatePickerElements(NSDatePickerElementFlags::HourMinute);
        picker.setDatePickerStyle(NSDatePickerStyle::TextFieldAndStepper);
        // Today at that time: only the hour and minute are read back.
        let date_at = |(hour, minute): (u32, u32)| {
            NSCalendar::currentCalendar().dateBySettingHour_minute_second_ofDate_options(hour as isize, minute as isize, 0, &objc2_foundation::NSDate::now(), NSCalendarOptions::empty())
        };
        let time_of = |d: &objc2_foundation::NSDate| {
            let c = NSCalendar::currentCalendar();
            (c.component_fromDate(NSCalendarUnit::Hour, d) as u32, c.component_fromDate(NSCalendarUnit::Minute, d) as u32)
        };
        let (value, p) = (self.0, picker.clone());
        let target = on_action(cx, &picker, move || value.set(time_of(&p.dateValue())));
        let (mut node, _) = Node::leaf(cx, Retained::into_super(Retained::into_super(picker.clone())), (false, false), Size::default());
        let value = self.0;
        node.own(Effect::new(move || {
            let want = value.get();
            if time_of(&picker.dateValue()) != want
                && let Some(d) = date_at(want)
            {
                picker.setDateValue(&d);
            }
        }));
        node.keep(target);
        node
    }
}

/// A time-of-day picker bound two ways to `value`, an hour (0–23) and a
/// minute: for a daily reminder or an opening time, which keep their hour
/// and minute whatever the date or time zone. For a moment in time, use
/// [`date_picker`] with [`DatePicker::with_time`].
pub fn time_picker(value: &Signal<(u32, u32)>) -> TimePicker {
    TimePicker(*value)
}

/// Whether two colours look the same (within an 8-bit step).
fn close_to(a: Color, b: Color) -> bool {
    match (a.components(), b.components()) {
        (Some(a), Some(b)) => [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)].iter().all(|(x, y)| (x - y).abs() < 1.0 / 512.0),
        _ => a == b,
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
            if !Color::from_ns(&well.color()).is_some_and(|c| close_to(c, want)) {
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
    options: SegmentOptions<T>,
}

type LiveOptions<T> = Box<dyn Fn() -> Vec<(String, T)>>;

enum SegmentOptions<T> {
    Fixed(Vec<(String, T)>),
    Live(LiveOptions<T>),
}

impl<T: PartialEq + Clone + 'static> View for Segmented<T> {
    fn build(self, cx: &mut Cx) -> Node {
        use objc2_app_kit::NSSegmentSwitchTracking;
        let control = objc2_app_kit::NSSegmentedControl::new(cx.mtm);
        control.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
        // The segments shown, set in place when live options change.
        let shown: std::rc::Rc<std::cell::RefCell<Vec<(String, T)>>> = Default::default();
        let set = {
            let (control, shown) = (control.clone(), shown.clone());
            move |options: Vec<(String, T)>| {
                let changed = {
                    let old = shown.borrow();
                    old.len() != options.len() || old.iter().zip(&options).any(|(a, b)| a.0 != b.0)
                };
                if changed {
                    control.setSegmentCount(options.len() as isize);
                    for (i, (title, _)) in options.iter().enumerate() {
                        control.setLabel_forSegment(&NSString::from_str(title), i as isize);
                        control.setWidth_forSegment(0.0, i as isize);
                    }
                }
                *shown.borrow_mut() = options;
                changed
            }
        };
        let live = match self.options {
            SegmentOptions::Fixed(options) => {
                set(options);
                None
            }
            SegmentOptions::Live(f) => {
                set(untrack(&f));
                Some(f)
            }
        };
        let (value, opts, c) = (self.value, shown.clone(), control.clone());
        let target = on_action(cx, &control, move || {
            let chosen = usize::try_from(c.selectedSegment()).ok().and_then(|i| opts.borrow().get(i).map(|(_, v)| v.clone()));
            if let Some(v) = chosen {
                value.set(v);
            }
        });
        let (mut node, handle) = Node::leaf(cx, Retained::into_super(Retained::into_super(control.clone())), (false, false), Size::default());
        let value = self.value;
        let select = {
            let (control, shown) = (control.clone(), shown.clone());
            move |current: &T| {
                let i = shown.borrow().iter().position(|(_, v)| v == current).map_or(-1, |i| i as isize);
                if control.selectedSegment() != i {
                    control.setSelectedSegment(i);
                }
            }
        };
        if let Some(f) = live {
            let (control, select) = (control.clone(), select.clone());
            node.own(Effect::new(move || {
                let options = f();
                if set(options) {
                    crate::appkit_views::remeasure(&control, &handle);
                }
                select(&untrack(|| value.get()));
            }));
        }
        node.own(Effect::new(move || select(&value.get())));
        node.keep(target);
        node
    }
}

/// A segmented control bound two ways to `value`, one segment per option
/// (title, value). VoiceOver reads each segment's title.
pub fn segmented<T: PartialEq + Clone + 'static>(value: &Signal<T>, options: Vec<(&str, T)>) -> Segmented<T> {
    Segmented { value: *value, options: SegmentOptions::Fixed(options.into_iter().map(|(t, v)| (t.to_string(), v)).collect()) }
}

/// A segmented control whose segments come from `options` (title, value),
/// re-read when the signals it reads change: segments are added, removed
/// and retitled in place (a tab bar of documents or queries). Bound two
/// ways to `value`; no segment is selected while `value` matches none.
///
/// ```no_run
/// use ferrokit::*;
///
/// let tabs = Signal::new(vec![(1u64, "Query 1".to_string())]);
/// let current = Signal::new(1u64);
/// let bar = segmented_with(&current, move || tabs.get().into_iter().map(|(id, title)| (title, id)).collect());
/// # let _ = bar;
/// ```
pub fn segmented_with<T: PartialEq + Clone + 'static>(value: &Signal<T>, options: impl Fn() -> Vec<(String, T)> + 'static) -> Segmented<T> {
    Segmented { value: *value, options: SegmentOptions::Live(Box::new(options)) }
}

define_class!(
    // A combo box's typed text and chosen suggestions → its signal.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FerrokitComboDelegate"]
    #[ivars = Signal<String>]
    struct ComboDelegate;

    unsafe impl NSObjectProtocol for ComboDelegate {}

    unsafe impl NSControlTextEditingDelegate for ComboDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn did_change(&self, notification: &NSNotification) {
            crate::failure::guard("a control (did_change)", (), || {
                let Some(field) = notification.object().and_then(|o| o.downcast::<NSTextField>().ok()) else { return };
                let value = field.stringValue().to_string();
                let signal = *self.ivars();
                batch(|| signal.set_if_changed(value));
            })
        }
    }

    unsafe impl NSTextFieldDelegate for ComboDelegate {}

    unsafe impl NSComboBoxDelegate for ComboDelegate {
        #[unsafe(method(comboBoxSelectionDidChange:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            crate::failure::guard("a control (selection_did_change)", (), || {
                let Some(combo) = notification.object().and_then(|o| o.downcast::<objc2_app_kit::NSComboBox>().ok()) else { return };
                // The field shows the choice only after this notification: read
                // the chosen item itself.
                let chosen = combo.objectValueOfSelectedItem().and_then(|o| o.downcast::<NSString>().ok()).map(|s| s.to_string());
                if let Some(value) = chosen {
                    let signal = *self.ivars();
                    batch(|| signal.set_if_changed(value));
                }
            })
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
        // change notification: the action reads the field too. A keystroke
        // sends the action as Return does, so Return is what the delegate
        // saw, not "the text is as it was".
        let (value, f, submit, d) = (self.value, field.clone(), self.on_submit, delegate.clone());
        let target = on_action(cx, &field, move || {
            value.set_if_changed(f.stringValue().to_string());
            if d.take_return()
                && let Some(s) = &submit
            {
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
