//! Standard controls bound two ways to signals: the user's edits set the
//! signal (inside a batch); setting the signal updates the control.

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSControlStateValueOff, NSControlStateValueOn, NSControlTextEditingDelegate, NSSecureTextField, NSSwitch,
    NSTextField, NSTextFieldDelegate, NSView,
};
use objc2_foundation::{NSNotification, NSString};

use crate::signal::{Effect, Signal, batch};
use crate::view::{ActionTarget, Cx, Node, Size, View, hstack, text};

define_class!(
    // Live edits of a text field → its signal.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiFieldDelegate"]
    #[ivars = Signal<String>]
    struct FieldDelegate;

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
);

impl FieldDelegate {
    fn new(mtm: MainThreadMarker, value: Signal<String>) -> Retained<Self> {
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
    hstack((text(title), SwitchView(value.clone())))
}
