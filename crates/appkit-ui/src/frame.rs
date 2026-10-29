//! Per-frame callbacks, in step with the display (a display link): for live
//! data and animation. While the display isn't drawing (asleep, the screen
//! locked, the window occluded) no frames come, so a slow timer stands in.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSWindow;

use crate::signal::batch;

/// The stand-in timer's interval, and how long without a display frame
/// before it takes over.
const FALLBACK_INTERVAL: f64 = 0.1;
const STALLED_AFTER: f64 = 0.05;

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    /// Seconds on the clock display links use.
    fn CACurrentMediaTime() -> f64;
}

struct Ticks {
    f: RefCell<Box<dyn FnMut(f64)>>,
    /// When the display last delivered a frame.
    last_display: Cell<f64>,
    display: Cell<u64>,
    fallback: Cell<u64>,
}

define_class!(
    // The display link's and the stand-in timer's target.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppKitUiFrameTarget"]
    #[ivars = Ticks]
    struct FrameTarget;

    impl FrameTarget {
        #[unsafe(method(tick:))]
        fn tick(&self, link: &AnyObject) {
            // SAFETY: CADisplayLink's targetTimestamp (seconds, media time).
            let target: f64 = unsafe { msg_send![link, targetTimestamp] };
            let t = self.ivars();
            t.last_display.set(unsafe { CACurrentMediaTime() });
            t.display.set(t.display.get() + 1);
            self.call(target);
        }

        #[unsafe(method(fallback:))]
        fn fallback(&self, _timer: &AnyObject) {
            let t = self.ivars();
            let now = unsafe { CACurrentMediaTime() };
            if now - t.last_display.get() > STALLED_AFTER {
                t.fallback.set(t.fallback.get() + 1);
                self.call(now);
            }
        }
    }
);

impl FrameTarget {
    fn call(&self, at: f64) {
        objc2::rc::autoreleasepool(|_| {
            // A re-entrant call (a nested run loop inside the callback) is skipped.
            if let Ok(mut f) = self.ivars().f.try_borrow_mut() {
                batch(|| f(at));
            }
        });
    }
}

/// Calls a function once per frame of the display a window is on (see
/// [`on_frame`]). Dropping it stops the calls.
pub struct FrameLoop {
    link: Retained<AnyObject>,
    timer: Retained<AnyObject>,
    target: Retained<FrameTarget>,
}

impl FrameLoop {
    /// Calls so far: (driven by display frames, by the stand-in timer).
    pub fn counts(&self) -> (u64, u64) {
        let t = self.target.ivars();
        (t.display.get(), t.fallback.get())
    }
}

impl Drop for FrameLoop {
    fn drop(&mut self) {
        // SAFETY: CADisplayLink's and NSTimer's invalidate (each releases the target).
        unsafe {
            let () = msg_send![&*self.link, invalidate];
            let () = msg_send![&*self.timer, invalidate];
        }
    }
}

/// Calls `f` once per frame of the display `window` is on, in step with its
/// refresh (60 or 120 Hz), with the frame's target time in seconds. Runs in
/// a [`batch`], so signal updates and layout happen once per frame: the
/// place to apply data that arrived since the last frame.
///
/// While the display isn't drawing the window (asleep, screen locked,
/// window occluded), a timer calls `f` 10 times a second instead, so live
/// data keeps flowing in small steps rather than piling up.
///
/// Keep the returned [`FrameLoop`]; dropping it stops the calls.
pub fn on_frame(window: &NSWindow, f: impl FnMut(f64) + 'static) -> FrameLoop {
    let mtm = crate::mtm();
    let ticks = Ticks { f: RefCell::new(Box::new(f)), last_display: Cell::new(0.0), display: Cell::new(0), fallback: Cell::new(0) };
    let target = FrameTarget::alloc(mtm).set_ivars(ticks);
    // SAFETY: NSObject's designated initialiser.
    let target: Retained<FrameTarget> = unsafe { msg_send![super(target), init] };
    // SAFETY: the target responds to tick: and fallback:; the link and the
    // timer retain it until invalidated. Both run in the main run loop's
    // common modes, so they continue during live resize and tracking.
    unsafe {
        let link: Retained<AnyObject> = msg_send![window, displayLinkWithTarget: &*target, selector: sel!(tick:)];
        let run_loop: Retained<AnyObject> = msg_send![objc2::class!(NSRunLoop), mainRunLoop];
        let () = msg_send![&*link, addToRunLoop: &*run_loop, forMode: objc2_foundation::NSRunLoopCommonModes];
        let timer: Retained<AnyObject> = msg_send![
            objc2::class!(NSTimer),
            timerWithTimeInterval: FALLBACK_INTERVAL,
            target: &*target,
            selector: sel!(fallback:),
            userInfo: Option::<&AnyObject>::None,
            repeats: true
        ];
        let () = msg_send![&*timer, setTolerance: FALLBACK_INTERVAL / 10.0];
        let () = msg_send![&*run_loop, addTimer: &*timer, forMode: objc2_foundation::NSRunLoopCommonModes];
        FrameLoop { link, timer, target }
    }
}
