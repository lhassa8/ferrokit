//! A Ferrokit window with SwiftUI views inside (compiled by
//! ferrokit-build). `CHECK=all` checks that `testing::ax_elements` and
//! `ax_audit` see inside SwiftUI's views without anything else having asked
//! for their accessibility first.

use std::ffi::c_void;

use ferrokit::*;
use objc2::rc::Retained;
use objc2_app_kit::NSView;

unsafe extern "C" {
    fn named_panel() -> *mut c_void;
    fn unnamed_panel() -> *mut c_void;
}

fn hosting(make: unsafe extern "C" fn() -> *mut c_void) -> impl View {
    // SAFETY: each returns a +1 NSHostingView, owned from here on.
    native(move |_| unsafe { Retained::from_raw(make().cast::<NSView>()) }.expect("a hosting view"))
}

fn main() {
    let unnamed = Signal::new(false);
    let level = Signal::new(50.0);
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    App::new("SwiftUI Window")
        .window(Window::new("SwiftUI Window").size(520.0, 360.0).view(
            vstack((
                hstack((text("Ferrokit:"), slider(&level, 0.0, 100.0).accessibility_label("Volume"))),
                hosting(named_panel),
                dynamic(move || if unnamed.get() { AnyView::new(hosting(unnamed_panel)) } else { AnyView::new(text("")) }),
            ))
            .padding(16.0),
        ))
        .on_launch(move || {
            if checks {
                self_checks(unnamed);
            }
        })
        .run();
}

fn after(ms: u64, f: impl FnOnce() + 'static) {
    spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
}

fn check(failed: &mut u32, name: &str, pass: bool, note: String) {
    eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
    *failed += u32::from(!pass);
}

fn self_checks(unnamed: Signal<bool>) {
    after(300, move || {
        let w = windows().into_iter().find(|w| w.title().to_string() == "SwiftUI Window").unwrap();
        let mut failed = 0;
        // Nothing has asked for accessibility yet: this is the first walk.
        let elements = testing::ax_elements(&w);
        let names: Vec<String> = elements.iter().map(|e| format!("{} {:?}", e.role, e.name)).collect();
        let sliders = elements.iter().filter(|e| e.role == "AXSlider").count();
        let title = elements.iter().any(|e| e.role == "AXStaticText" && e.name.contains("Swift Charts"));
        check(&mut failed, "swiftui-ax-elements", sliders == 2 && title, format!("{sliders} sliders (Ferrokit's and SwiftUI's), the chart's title {title}; {names:?}"));
        let audit = testing::ax_audit(&w);
        check(&mut failed, "swiftui-ax-audit-named", audit.is_empty(), format!("findings {audit:?}"));
        // A SwiftUI view added later, with an unnamed slider.
        unnamed.set(true);
        after(300, move || {
            let audit = testing::ax_audit(&w);
            let found = audit.len() == 1 && audit[0].contains("AXSlider without a name");
            check(&mut failed, "swiftui-ax-audit-unnamed", found, format!("findings {audit:?}"));
            eprintln!("[check] done, {failed} failed");
            std::process::exit(if failed == 0 { 0 } else { 1 });
        });
    });
}
