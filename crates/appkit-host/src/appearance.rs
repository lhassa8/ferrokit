//! System appearance and colours (risk 8).

use std::cell::{Cell, RefCell};

use block2::RcBlock;
use objc2::Message as _;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSColor, NSColorSpace};
use objc2_foundation::{
    NSDictionary, NSKeyValueChangeKey, NSKeyValueObservingOptions, NSNotification, NSNotificationCenter,
    NSObjectNSKeyValueObserverRegistration, NSObjectProtocol, NSString,
};

use crate::events::{Event, emit};
use crate::host::mtm;

/// An sRGB colour, components in 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    /// Red, 0..=1.
    pub r: f32,
    /// Green, 0..=1.
    pub g: f32,
    /// Blue, 0..=1.
    pub b: f32,
    /// Alpha, 0..=1.
    pub a: f32,
}

/// The system's semantic colours, resolved for the window's current appearance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemColors {
    /// Whether the app resolves to a dark appearance.
    pub dark: bool,
    /// `windowBackgroundColor`.
    pub window_background: Rgba,
    /// `controlBackgroundColor` (e.g. behind table rows).
    pub control_background: Rgba,
    /// `labelColor`.
    pub text: Rgba,
    /// `secondaryLabelColor`.
    pub secondary_text: Rgba,
    /// `separatorColor`.
    pub separator: Rgba,
    /// `controlAccentColor` (the user's accent colour).
    pub accent: Rgba,
    /// `selectedContentBackgroundColor`.
    pub selected_content_background: Rgba,
    /// `systemGreenColor`.
    pub green: Rgba,
    /// `systemRedColor`.
    pub red: Rgba,
    /// `systemOrangeColor`.
    pub orange: Rgba,
}

define_class!(
    // Observes NSApp.effectiveAppearance (system Dark Mode switches and app
    // overrides). The window's own appearance can't be used: Iced pins it to
    // its theme's mode whenever the app sets a theme.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitHostAppearanceObserver"]
    struct AppearanceObserver;

    impl AppearanceObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            _context: *mut std::ffi::c_void,
        ) {
            changed();
        }
    }
);

fn changed() {
    GENERATION.with(|g| g.set(g.get() + 1));
    emit(Event::AppearanceChanged);
}

thread_local! {
    static KVO: RefCell<Option<Retained<AppearanceObserver>>> = const { RefCell::new(None) };
    static OBSERVER: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// Increments whenever the app's appearance (Dark Mode) or the system colours
/// (accent) change.
pub fn generation() -> u64 {
    GENERATION.with(Cell::get)
}

pub(crate) fn install() {
    if OBSERVER.with_borrow(Option::is_some) {
        return;
    }
    let observer = AppearanceObserver::alloc(mtm()).set_ivars(());
    let observer: Retained<AppearanceObserver> = unsafe { msg_send![super(observer), init] };
    // SAFETY: the observer is kept alive in KVO for the rest of the program.
    unsafe {
        NSApplication::sharedApplication(mtm()).addObserver_forKeyPath_options_context(
            &observer,
            &NSString::from_str("effectiveAppearance"),
            NSKeyValueObservingOptions::New,
            std::ptr::null_mut(),
        );
    }
    KVO.with_borrow_mut(|k| *k = Some(observer));

    let block = RcBlock::new(|_: std::ptr::NonNull<NSNotification>| changed());
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: observer block retained by the notification center; delivered on
    // the posting (main) thread.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(&NSString::from_str("NSSystemColorsDidChangeNotification")),
            None,
            None,
            &block,
        )
    };
    OBSERVER.with_borrow_mut(|o| *o = Some(token));
}

fn rgba(color: &NSColor) -> Rgba {
    let c = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()).unwrap_or_else(|| color.retain());
    Rgba { r: c.redComponent() as f32, g: c.greenComponent() as f32, b: c.blueComponent() as f32, a: c.alphaComponent() as f32 }
}

/// Whether the app currently resolves to a dark appearance (system setting or
/// app override).
pub fn is_dark() -> bool {
    let appearance = NSApplication::sharedApplication(mtm()).effectiveAppearance();
    let names = objc2_foundation::NSArray::from_retained_slice(&[
        // SAFETY: framework string constants.
        unsafe { objc2_app_kit::NSAppearanceNameAqua }.retain(),
        unsafe { objc2_app_kit::NSAppearanceNameDarkAqua }.retain(),
    ]);
    appearance.bestMatchFromAppearancesWithNames(&names).is_some_and(|n: Retained<NSString>| n.to_string().contains("Dark"))
}

/// Resolves the system colours for the app's appearance.
pub fn system_colors() -> SystemColors {
    let appearance = NSApplication::sharedApplication(mtm()).effectiveAppearance();
    let dark = is_dark();
    let out = RefCell::new(None);
    let block = RcBlock::new(|| {
        *out.borrow_mut() = Some(SystemColors {
            dark,
            window_background: rgba(&NSColor::windowBackgroundColor()),
            control_background: rgba(&NSColor::controlBackgroundColor()),
            text: rgba(&NSColor::labelColor()),
            secondary_text: rgba(&NSColor::secondaryLabelColor()),
            separator: rgba(&NSColor::separatorColor()),
            accent: rgba(&NSColor::controlAccentColor()),
            selected_content_background: rgba(&NSColor::selectedContentBackgroundColor()),
            green: rgba(&NSColor::systemGreenColor()),
            red: rgba(&NSColor::systemRedColor()),
            orange: rgba(&NSColor::systemOrangeColor()),
        });
    });
    appearance.performAsCurrentDrawingAppearance(&block);
    drop(block);
    out.into_inner().expect("performAsCurrentDrawingAppearance runs the block")
}
