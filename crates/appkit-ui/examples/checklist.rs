//! appkit-ui's regression suite (grows with each F1 milestone). `CHECK=all`
//! runs the unattended checks and exits non-zero on failure; without it the
//! app just runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use appkit_ui::{
    App, AppEvent, Effect, Identifiable, Signal, View, ViewExt, Window, batch, dynamic, effect_runs, for_each, label, later,
    live_nodes, spacer, spawn, text, vstack, windows,
};
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
    check("windows", ws.len() == 3 && visible == 3, format!("{} windows, {visible} visible", ws.len()));
    let window_menu = menu_items(2);
    let listed = ["First", "Second", "Views"].iter().all(|t| window_menu.iter().any(|(title, _, _)| title == t));
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
                view_checks(events.clone());
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

#[derive(Clone)]
struct Item {
    id: u32,
    name: String,
}

impl Identifiable for Item {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

fn item(id: u32) -> Item {
    Item { id, name: format!("item{id}") }
}

#[derive(Clone)]
struct ViewState {
    items: Signal<Vec<Item>>,
    flag: Signal<bool>,
    inner: Signal<u32>,
    tab: Signal<u32>,
}

thread_local! {
    static STATE: ViewState = ViewState {
        items: Signal::new(vec![item(1), item(2), item(3)]),
        flag: Signal::new(true),
        inner: Signal::new(0),
        tab: Signal::new(0),
    };
}

fn state() -> ViewState {
    STATE.with(Clone::clone)
}

fn views_window() -> impl View {
    let st = state();
    let (items, flag, inner, tab) = (st.items.clone(), st.flag.clone(), st.inner.clone(), st.tab.clone());
    vstack((
        label(move || format!("{} items", st.items.with(Vec::len))),
        for_each(move || items.get(), |it: Item| label(move || it.name.clone())),
        dynamic(move || {
            if flag.get() {
                let inner = inner.clone();
                vstack((text("on"), label(move || format!("inner {}", inner.get())))).id("on")
            } else {
                text("off").id("off")
            }
        }),
        dynamic(move || {
            let t = tab.get();
            // Tabs 0 and 1 share an id: switching between them keeps the node.
            text(format!("tab {t}")).id(t / 2)
        }),
        spacer(),
    ))
    .padding(12.0)
}

/// The window's labels: (text, view).
fn labels() -> Vec<(String, Retained<NSTextField>)> {
    let w = windows().into_iter().find(|w| w.title().to_string() == "Views").unwrap();
    w.contentView()
        .unwrap()
        .subviews()
        .iter()
        .filter_map(|v| v.downcast::<NSTextField>().ok())
        .map(|f| (f.stringValue().to_string(), f))
        .collect()
}

fn find_label(text: &str) -> Option<Retained<NSTextField>> {
    labels().into_iter().find(|(t, _)| t == text).map(|(_, v)| v)
}

/// Physical footprint in MB (what Activity Monitor shows; RSS keeps freed
/// heap pages and overstates growth), via vmmap.
fn footprint_mb() -> f64 {
    let out = std::process::Command::new("vmmap").args(["--summary", &std::process::id().to_string()]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.starts_with("Physical footprint:")).unwrap_or("");
    let v = line.split_whitespace().last().unwrap_or("0M");
    let (num, unit) = v.split_at(v.len() - 1);
    num.parse::<f64>().unwrap_or(0.0) * if unit == "G" { 1024.0 } else if unit == "K" { 1.0 / 1024.0 } else { 1.0 }
}

fn view_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let st = state();
    // Build: the tree's leaves are subviews of the window's root view.
    let names: Vec<String> = labels().into_iter().map(|(t, _)| t).collect();
    check(
        "views-build",
        ["3 items", "item1", "item2", "item3", "on", "inner 0", "tab 0"].iter().all(|t| names.iter().any(|n| n == t)),
        format!("{names:?}"),
    );

    // for_each: reorder + insert + remove keeps the surviving items' native views.
    let (v1, v2, v3) = (find_label("item1").unwrap(), find_label("item2").unwrap(), find_label("item3").unwrap());
    let weak2 = objc2::rc::Weak::from_retained(&v2);
    drop(v2);
    batch(|| st.items.set(vec![item(3), item(1), item(4)]));
    let (n1, n3, n4) = (find_label("item1").unwrap(), find_label("item3").unwrap(), find_label("item4").unwrap());
    let same = std::ptr::eq(&*v1, &*n1) && std::ptr::eq(&*v3, &*n3);
    let order = n3.frame().origin.y < n1.frame().origin.y && n1.frame().origin.y < n4.frame().origin.y;
    check(
        "for-each-identity",
        same && order && find_label("item2").is_none() && find_label("3 items").is_some(),
        format!("kept item1/item3 views: {same}; laid out 3,1,4: {order}"),
    );

    // dynamic: switching tears the old subtree down and stops its effects.
    let nodes_on = live_nodes();
    batch(|| st.flag.set(false));
    let runs = effect_runs();
    batch(|| st.inner.set(5));
    let stopped = effect_runs() == runs;
    check(
        "dynamic-teardown",
        find_label("off").is_some() && find_label("on").is_none() && find_label("inner 0").is_none() && stopped,
        format!("nodes {nodes_on} → {}; removed subtree's effect ran after removal: {}", live_nodes(), !stopped),
    );
    batch(|| st.flag.set(true));
    check("dynamic-rebuild", find_label("inner 5").is_some(), "rebuilt subtree reads the current value".into());

    // .id: same id keeps the node (and its text); a new id rebuilds.
    let t0 = find_label("tab 0").unwrap();
    batch(|| st.tab.set(1));
    let kept = find_label("tab 0").is_some_and(|v| std::ptr::eq(&*v, &*t0));
    batch(|| st.tab.set(2));
    let rebuilt = find_label("tab 2").is_some_and(|v| !std::ptr::eq(&*v, &*t0));
    check("id-identity", kept && rebuilt, format!("tab 0→1 kept the node: {kept}; 1→2 rebuilt: {rebuilt}"));

    // Teardown: churn 1,000 × 10 items; nodes and subviews return to baseline.
    let base_nodes = live_nodes();
    let base_views = labels().len();
    // Warm-up: one full churn (first-time AppKit and allocator growth), then
    // measure a second one. A leak shows as growth; a one-off peak doesn't.
    for round in 0..1000u32 {
        batch(|| st.items.set((0..10).map(|i| item(50_000 + round * 10 + i)).collect()));
        batch(|| st.items.set(vec![]));
    }
    let rss0 = footprint_mb();
    let t = std::time::Instant::now();
    for round in 0..1000u32 {
        batch(|| st.items.set((0..10).map(|i| item(10_000 + round * 10 + i)).collect()));
        batch(|| st.items.set(vec![]));
    }
    let ms = t.elapsed().as_secs_f64() * 1e3;
    batch(|| st.items.set(vec![item(3), item(1), item(4)]));
    later(move || {
        let removed_gone = weak2.load().is_none();
        let rss1 = footprint_mb();
        check(
            "teardown-churn",
            live_nodes() == base_nodes && labels().len() == base_views && removed_gone && rss1 - rss0 < 15.0,
            format!(
                "10,000 rows built and removed in {ms:.0} ms; nodes {base_nodes} → {}; views {base_views} → {}; removed view deallocated: {removed_gone}; footprint {rss0:.0} → {rss1:.0} MB",
                live_nodes(),
                labels().len()
            ),
        );
        appearance_check(events.clone());
    });
}

fn main() {
    let events: Rc<RefCell<Vec<AppEvent>>> = Rc::default();
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    let e2 = events.clone();
    App::new("Checklist")
        .window(Window::new("First").size(480.0, 200.0).content(|| field("hello world")))
        .window(Window::new("Second").size(360.0, 160.0))
        .window(Window::new("Views").size(320.0, 420.0).view(views_window()))
        .on_event(move |e| e2.borrow_mut().push(e))
        .on_launch(move || {
            if checks {
                run_checks(events);
            }
        })
        .run();
}
