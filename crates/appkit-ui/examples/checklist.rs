//! appkit-ui's regression suite (grows with each F1 milestone). `CHECK=all`
//! runs the unattended checks and exits non-zero on failure; without it the
//! app just runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use appkit_ui::{App, AppEvent, Effect, Signal, Window, batch, effect_runs, later, spawn, windows};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSTextField, NSView};
use objc2_foundation::NSString;

thread_local! {
    static FAILED: Cell<u32> = const { Cell::new(0) };
}

fn check(name: &str, pass: bool, note: String) {
    eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
    if !pass {
        FAILED.with(|f| f.set(f.get() + 1));
    }
}

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().unwrap()
}

fn field(text: &str) -> Retained<NSView> {
    let f = NSTextField::textFieldWithString(&NSString::from_str(text), mtm());
    f.setFrame(objc2_foundation::NSRect::new(
        objc2_foundation::NSPoint::new(20.0, 20.0),
        objc2_foundation::NSSize::new(300.0, 24.0),
    ));
    let content = NSView::new(mtm());
    content.addSubview(&f);
    content
}

/// The titles and key equivalents of a top-level menu's items.
fn menu_items(index: isize) -> Vec<(String, String, bool)> {
    let app = NSApplication::sharedApplication(mtm());
    let bar = app.mainMenu().unwrap();
    let menu = bar.itemAtIndex(index).and_then(|i| i.submenu()).unwrap();
    (0..menu.numberOfItems())
        .filter_map(|i| menu.itemAtIndex(i))
        .map(|i| {
            let shift = i.keyEquivalentModifierMask().contains(NSEventModifierFlags::Shift);
            (i.title().to_string(), i.keyEquivalent().to_string(), shift)
        })
        .collect()
}

fn has(items: &[(String, String, bool)], title: &str, key: &str) -> bool {
    items.iter().any(|(t, k, _)| t == title && k == key)
}

fn run_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let app = NSApplication::sharedApplication(mtm());

    // Menu bar: App, Edit, Window, Help.
    let bar = app.mainMenu().unwrap();
    let titles: Vec<String> = (0..bar.numberOfItems()).filter_map(|i| bar.itemAtIndex(i)).map(|i| i.title().to_string()).collect();
    check("menu-bar", titles == ["Checklist", "Edit", "Window", "Help"], format!("{titles:?}"));
    let app_menu = menu_items(0);
    check(
        "app-menu",
        has(&app_menu, "Quit Checklist", "q") && has(&app_menu, "Hide Checklist", "h") && has(&app_menu, "About Checklist", ""),
        format!("{} items", app_menu.len()),
    );
    let edit = menu_items(1);
    let redo_shift = edit.iter().any(|(t, k, s)| t == "Redo" && k == "z" && *s);
    check(
        "edit-menu",
        ["Undo", "Cut", "Copy", "Paste", "Select All"].iter().zip(["z", "x", "c", "v", "a"]).all(|(t, k)| has(&edit, t, k)) && redo_shift,
        format!("{:?}", edit.iter().map(|e| &e.0).collect::<Vec<_>>()),
    );

    // Windows: both open; AppKit lists them in the Window menu.
    let ws = windows();
    let visible = ws.iter().filter(|w| w.isVisible()).count();
    check("windows", ws.len() == 2 && visible == 2, format!("{} windows, {visible} visible", ws.len()));
    let window_menu = menu_items(2);
    let listed = ["First", "Second"].iter().all(|t| window_menu.iter().any(|(title, _, _)| title == t));
    check("window-menu-lists-windows", listed && app.windowsMenu().is_some(), format!("{:?}", window_menu.iter().map(|e| &e.0).collect::<Vec<_>>()));
    check("help-menu", app.helpMenu().is_some(), String::new());

    // Edit menu → responder chain: Select All selects the focused field's text.
    let first = &ws[0];
    let field = first.contentView().unwrap().subviews().firstObject().unwrap().downcast::<NSTextField>().unwrap();
    first.makeFirstResponder(Some(&field));
    let edit_menu = app.mainMenu().unwrap().itemAtIndex(1).and_then(|i| i.submenu()).unwrap();
    let select_all = edit_menu.indexOfItemWithTitle(&NSString::from_str("Select All"));
    edit_menu.performActionForItemAtIndex(select_all);
    let selected = field.currentEditor().map(|e| e.selectedRange().length).unwrap_or(0);
    check("edit-select-all", selected == 11, format!("{selected} characters selected in \"hello world\""));

    // Signals: cost of one update (one subscribed effect), and batching.
    let n = Signal::new(0u64);
    let n2 = n.clone();
    let seen = Rc::new(Cell::new(0u64));
    let s2 = seen.clone();
    let effect = Effect::new(move || s2.set(n2.get()));
    let t = std::time::Instant::now();
    for i in 1..=100_000u64 {
        n.set(i);
    }
    let per = t.elapsed().as_secs_f64() * 1e9 / 100_000.0;
    check("signal-update-cost", seen.get() == 100_000, format!("{per:.0} ns per set + effect run"));
    let runs = effect_runs();
    batch(|| (0..1000).for_each(|i| n.set(i)));
    check("signal-batch", effect_runs() - runs == 1, format!("{} effect run(s) for 1,000 sets in one batch", effect_runs() - runs));
    drop(effect);

    // spawn: work on a background thread, continuation on the main thread in one batch.
    let main_thread = std::thread::current().id();
    let result = Signal::new(String::new());
    let r2 = result.clone();
    let runs_seen = Rc::new(Cell::new(0u32));
    let rs = runs_seen.clone();
    let watcher = Effect::new(move || {
        let _ = r2.get();
        rs.set(rs.get() + 1);
    });
    let r3 = result.clone();
    spawn(
        move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            (std::thread::current().id(), (1..=1000u64).sum::<u64>())
        },
        move |(worker, sum)| {
            r3.set("partial".into());
            r3.set(format!("sum {sum}"));
            let on_main = std::thread::current().id() == main_thread;
            later(move || {
                let _ = &watcher;
                check(
                    "spawn",
                    worker != main_thread && on_main && result.get_untracked() == "sum 500500" && runs_seen.get() == 2,
                    format!("worker off main: {}, continuation on main: {on_main}, value {:?}, watcher runs {}", worker != main_thread, result.get_untracked(), runs_seen.get()),
                );
                appearance_check(events.clone());
            });
        },
    );
}

fn appearance_check(events: Rc<RefCell<Vec<AppEvent>>>) {
    // Appearance: switching NSApp's appearance reaches the app as an event.
    let before = appkit_ui::is_dark();
    appkit_host::testing::set_app_appearance(Some(!before));
    later(move || {
        let after = appkit_ui::is_dark();
        let got = events.borrow().iter().filter(|e| **e == AppEvent::AppearanceChanged).count();
        check("appearance", after != before && got >= 1, format!("dark {before} → {after}, {got} event(s)"));
        appkit_host::testing::set_app_appearance(None);
        let failed = FAILED.with(Cell::get);
        eprintln!("[check] done, {failed} failed");
        std::process::exit(if failed == 0 { 0 } else { 1 });
    });
}

fn main() {
    let events: Rc<RefCell<Vec<AppEvent>>> = Rc::default();
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    let e2 = events.clone();
    App::new("Checklist")
        .window(Window::new("First").size(480.0, 200.0).content(|| field("hello world")))
        .window(Window::new("Second").size(360.0, 160.0))
        .on_event(move |e| e2.borrow_mut().push(e))
        .on_launch(move || {
            if checks {
                run_checks(events);
            }
        })
        .run();
}
