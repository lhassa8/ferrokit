//! appkit-ui's regression suite (grows with each F1 milestone). `CHECK=all`
//! runs the unattended checks and exits non-zero on failure; without it the
//! app just runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use appkit_ui::{
    Align, App, AppEvent, Effect, Identifiable, Signal, View, ViewExt, Window, batch, button, dynamic, effect_runs, for_each,
    grid, hstack, label, later, layout_stats, layout_times_us, live_nodes, open, spacer, spawn, text, vstack, windows,
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
    check("windows", ws.len() == 4 && visible == 4, format!("{} windows, {visible} visible", ws.len()));
    let window_menu = menu_items(2);
    let listed = ["First", "Second", "Views", "Layout"].iter().all(|t| window_menu.iter().any(|(title, _, _)| title == t));
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
        layout_checks(events.clone());
    });
}

thread_local! {
    static WIDE: Signal<String> = Signal::new("short".into());
}

fn layout_window() -> impl View {
    let wide = WIDE.with(Clone::clone);
    vstack((
        vstack((text("A wide label for alignment"), text("mid"))).alignment(Align::Center),
        vstack((text("A wide label for alignment"), text("end"))).alignment(Align::End),
        hstack((text("tall\nlabel\nhere"), text("bottom"))).alignment(Align::End),
        grid([
            (text("Name"), text("Alice"), text("x")),
            (text("A much longer key"), text("Bob"), text("y")),
        ]),
        hstack((text("left"), spacer(), text("right"))),
        text("filled").fill_width(),
        text("framed").frame(121.0, 41.0),
        hstack((label(move || wide.get()), text("after"))),
    ))
    .padding(13.0)
}

fn frame_of(window: &str, text: &str) -> objc2_foundation::NSRect {
    let w = windows().into_iter().find(|w| w.title().to_string() == window).unwrap();
    w.contentView()
        .unwrap()
        .subviews()
        .iter()
        .filter_map(|v| v.downcast::<NSTextField>().ok())
        .filter(|f| f.stringValue().to_string() == text)
        .map(|f| f.frame())
        .last()
        .unwrap()
}

fn layout_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let f = |t: &str| frame_of("Layout", t);
    let mid_x = |r: objc2_foundation::NSRect| r.origin.x + r.size.width / 2.0;
    let max_x = |r: objc2_foundation::NSRect| r.origin.x + r.size.width;
    let max_y = |r: objc2_foundation::NSRect| r.origin.y + r.size.height;
    let wides: Vec<_> = {
        let w = windows().into_iter().find(|w| w.title().to_string() == "Layout").unwrap();
        w.contentView().unwrap().subviews().iter().filter_map(|v| v.downcast::<NSTextField>().ok()).filter(|t| t.stringValue().to_string() == "A wide label for alignment").map(|t| t.frame()).collect()
    };
    let (mid, end) = (f("mid"), f("end"));
    check(
        "align-center-end",
        (mid_x(mid) - mid_x(wides[0])).abs() <= 0.5 && (max_x(end) - max_x(wides[1])).abs() <= 0.5,
        format!("centre {:.1} vs {:.1}; trailing {:.1} vs {:.1}", mid_x(mid), mid_x(wides[0]), max_x(end), max_x(wides[1])),
    );
    let (tall, bottom) = (f("tall\nlabel\nhere"), f("bottom"));
    check("align-bottom", (max_y(tall) - max_y(bottom)).abs() <= 0.5, format!("bottoms {:.1} / {:.1}", max_y(tall), max_y(bottom)));
    let (alice, bob, key) = (f("Alice"), f("Bob"), f("A much longer key"));
    check(
        "grid-columns",
        alice.origin.x == bob.origin.x && bob.origin.x >= max_x(key) + 8.0 - 0.5 && f("x").origin.x == f("y").origin.x,
        format!("column 2 at {:.1} / {:.1}; longest key ends at {:.1}", alice.origin.x, bob.origin.x, max_x(key)),
    );
    let content_w = windows().into_iter().find(|w| w.title().to_string() == "Layout").unwrap().contentView().unwrap().bounds().size.width;
    let (left, right) = (f("left"), f("right"));
    check(
        "spacer",
        left.origin.x == 13.0 && (max_x(right) - (content_w - 13.0)).abs() <= 0.5,
        format!("left at {:.1}, right ends at {:.1} of {:.1}", left.origin.x, max_x(right), content_w - 13.0),
    );
    let (filled, framed) = (f("filled"), f("framed"));
    check(
        "fill-and-frame",
        (filled.size.width - (content_w - 26.0)).abs() <= 0.5 && framed.size.width == 121.0 && framed.size.height == 41.0,
        format!("filled {:.1} wide (of {:.1}); framed {:.1}×{:.1}", filled.size.width, content_w - 26.0, framed.size.width, framed.size.height),
    );
    let all: Vec<_> = {
        let w = windows().into_iter().find(|w| w.title().to_string() == "Layout").unwrap();
        w.contentView().unwrap().subviews().iter().map(|v| v.frame()).collect()
    };
    let snapped = all.iter().all(|r| [r.origin.x, r.origin.y, r.size.width, r.size.height].iter().all(|v| (v * 2.0).fract() == 0.0));
    check("pixel-snapping", snapped, format!("{} frames on the 0.5 pt grid (2x)", all.len()));

    // A label that changes width moves its sibling; the rest of the window is skipped.
    let wide = WIDE.with(Clone::clone);
    let before = f("after").origin.x;
    let (p0, s0, _) = layout_stats();
    batch(|| wide.set("a considerably longer text".into()));
    let (p1, s1, _) = layout_stats();
    let after = f("after").origin.x;
    check(
        "subtree-relayout",
        after > before && p1 - p0 <= 6,
        format!("sibling moved {before:.1} → {after:.1}; {} nodes placed, {} subtrees skipped", p1 - p0, s1 - s0),
    );
    layout_bench(events);
}

/// 1,000 native views (250 stretching rows × label, label, button, label): resize layout
/// and the cost of a label update.
fn layout_bench(events: Rc<RefCell<Vec<AppEvent>>>) {
    let counter = Signal::new(0u64);
    let c = counter.clone();
    let rows: Vec<_> = (0..250)
        .map(|i| {
            let c = c.clone();
            hstack((
                if i == 0 { appkit_ui::AnyView::new(label(move || format!("Counter {}", c.get()))) } else { appkit_ui::AnyView::new(text(format!("Row {i}"))) },
                text("Some label"),
                spacer(), // rows stretch: every row re-lays out on resize
                button("Go", || {}),
                text("tail"),
            ))
        })
        .map(appkit_ui::AnyView::new)
        .collect();
    let t = std::time::Instant::now();
    let w = open(Window::new("Bench").size(700.0, 700.0).view(vstack(rows).padding(12.0)));
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let views = w.contentView().unwrap().subviews().len();
    let n0 = layout_times_us().len();
    for k in 0..200 {
        w.setContentSize(objc2_foundation::NSSize::new(600.0 + (k % 40) as f64 * 10.0, 700.0));
    }
    let mut resize: Vec<f64> = layout_times_us()[n0..].to_vec();
    resize.sort_by(|a, b| a.total_cmp(b));
    let mut upd = Vec::new();
    let (p0, _, _) = layout_stats();
    for _ in 0..500 {
        let t = std::time::Instant::now();
        batch(|| counter.update(|v| *v += 1));
        upd.push(t.elapsed().as_secs_f64() * 1e6);
    }
    let (p1, _, _) = layout_stats();
    upd.sort_by(|a, b| a.total_cmp(b));
    let p = |v: &[f64], q: usize| v[(v.len() - 1) * q / 100];
    check(
        "layout-bench",
        views == 1000 && p(&resize, 50) < 2000.0 && p(&upd, 50) < 72.0,
        format!(
            "{views} views built in {build_ms:.0} ms; resize layout p50 {:.2} ms, p95 {:.2} ms; label update p50 {:.1} µs, p95 {:.1} µs, {:.1} nodes placed per update (P0: 0.75 ms, 72 µs)",
            p(&resize, 50) / 1000.0, p(&resize, 95) / 1000.0, p(&upd, 50), p(&upd, 95), (p1 - p0) as f64 / 500.0
        ),
    );
    appearance_check(events);
}

fn main() {
    let events: Rc<RefCell<Vec<AppEvent>>> = Rc::default();
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    let e2 = events.clone();
    App::new("Checklist")
        .window(Window::new("First").size(480.0, 200.0).content(|| field("hello world")))
        .window(Window::new("Second").size(360.0, 160.0))
        .window(Window::new("Views").size(320.0, 420.0).view(views_window()))
        .window(Window::new("Layout").size(420.0, 480.0).view(layout_window()))
        .on_event(move |e| e2.borrow_mut().push(e))
        .on_launch(move || {
            if checks {
                run_checks(events);
            }
        })
        .run();
}
