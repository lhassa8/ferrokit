#![allow(dead_code)]
//! `DEMO=1` in the showcase examples: a scripted tour, for recording.
//! `scripts/record-demo.sh` reads the frame each prints, records it, and
//! quits the app at `[demo] done`; `--shots` takes window screenshots too.

use ferrokit::*;
use objc2_app_kit::{NSScreen, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};

/// A pause in milliseconds, then an action.
pub type Step = (u64, Box<dyn FnOnce()>);

/// An action after `ms`.
pub fn at(ms: u64, f: impl FnOnce() + 'static) -> Step {
    (ms, Box::new(f))
}

/// Puts `window` at `width` × `height` (at most the screen) in the middle of
/// the main screen, in front, with every other window of the app hidden;
/// then prints its frame for the recorder: `[demo] frame x y w h`, points
/// from the screen's top left, title bar included. The pointer is hidden
/// while the app is in front (the tour's clicks don't move it).
pub fn place(window: &NSWindow, width: f64, height: f64) {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    // SAFETY: +[NSCursor hide] on the main thread (no binding enabled for it).
    let _: () = unsafe { objc2::msg_send![objc2::class!(NSCursor), hide] };
    for w in windows() {
        if &*w != window {
            w.orderOut(None);
        }
    }
    let Some(screen) = NSScreen::mainScreen(mtm) else { return };
    let v = screen.visibleFrame();
    let (wd, ht) = (width.min(v.size.width * 0.97), height.min(v.size.height * 0.97));
    let origin = NSPoint::new((v.origin.x + (v.size.width - wd) / 2.0).round(), (v.origin.y + (v.size.height - ht) / 2.0).round());
    window.setFrame_display(NSRect::new(origin, NSSize::new(wd, ht)), true);
    testing::input::ready(window);
    let full = screen.frame().size.height;
    let f = window.frame();
    eprintln!("[demo] frame {:.0} {:.0} {:.0} {:.0}", f.origin.x, full - f.origin.y - f.size.height, f.size.width, f.size.height);
    // For window screenshots (`screencapture -l`).
    eprintln!("[demo] window {}", window.windowNumber());
}

/// Runs the steps in turn, then prints `[demo] done`.
pub fn play(mut steps: Vec<Step>) {
    if steps.is_empty() {
        eprintln!("[demo] done");
        return;
    }
    let (wait, step) = steps.remove(0);
    spawn(move || std::thread::sleep(std::time::Duration::from_millis(wait)), move |()| {
        step();
        play(steps);
    });
}

/// Types `text` into the key window a character at a time, `ms` apart.
pub fn type_slowly(text: &str, ms: u64) -> Vec<Step> {
    text.chars()
        .map(|c| {
            at(ms, move || {
                if let Some(w) = focused_window() {
                    testing::input::type_text(&w, &c.to_string());
                }
            })
        })
        .collect()
}

/// Every view in `window`, depth first.
pub fn views(window: &NSWindow) -> Vec<objc2::rc::Retained<objc2_app_kit::NSView>> {
    fn all(v: &objc2_app_kit::NSView, out: &mut Vec<objc2::rc::Retained<objc2_app_kit::NSView>>) {
        for s in v.subviews().iter() {
            out.push(s.clone());
            all(&s, out);
        }
    }
    let mut out = Vec::new();
    if let Some(c) = window.contentView() {
        all(&c, &mut out);
    }
    out
}

/// Clicks the button titled `title` in a visible window, as the user would.
pub fn press(title: &'static str) -> Step {
    at(0, move || {
        for w in windows().into_iter().filter(|w| w.isVisible()) {
            let found = views(&w).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok()).find(|b| b.title().to_string() == title);
            if let Some(b) = found {
                testing::input::click(&b);
                return;
            }
        }
    })
}

/// `step` after `ms` instead of right away.
pub fn after(ms: u64, step: Step) -> Step {
    (ms, step.1)
}
