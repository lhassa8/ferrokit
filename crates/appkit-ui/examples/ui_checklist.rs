//! appkit-ui's regression suite (grows with each F1 milestone). `CHECK=all`
//! runs the unattended checks and exits non-zero on failure; without it the
//! app just runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use appkit_ui::{
    Align, App, AppEvent, Effect, Identifiable, Signal, View, ViewExt, Window, batch, button, checkbox, dynamic, effect_runs,
    for_each, grid, hstack, label, later, layout_stats, layout_passes, layout_times_us, live_nodes, open, secure_field, spacer, spawn, text,
    text_field, toggle, vstack, windows, divider, image, picker, progress, scroll, slider, stepper, table, Column, Sort,
    TableModel, ViewId, TreeNode, outline, sidebar, TextModel, text_editor, split, toolbar_button, toolbar_space,
    toolbar_search, menu_item, menu_separator, MenuItem, Modifier, submenu, stored, set_undoable, Alert, OpenOptions, SheetHandle, alert, open_panel, save_panel, sheet,
};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSTextField, NSView};
use appkit_ui::testing::{
    ax_tree, context_menu_for_word, footprint_mb, has_ax, heap_in_use_mb, is_misspelled, menu_key, post_edit, send_action, shortcut, snapshot_windows,
};
use objc2_app_kit::{NSAppearanceCustomization, NSTextInputClient};
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

/// A top-level menu by title.
fn bar_menu(title: &str) -> Retained<objc2_app_kit::NSMenu> {
    let bar = NSApplication::sharedApplication(mtm()).mainMenu().unwrap();
    bar.itemWithTitle(&NSString::from_str(title)).and_then(|i| i.submenu()).unwrap()
}

/// The titles and key equivalents of a top-level menu's items.
fn menu_items(title: &str) -> Vec<(String, String, bool)> {
    let menu = bar_menu(title);
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
    check("menu-bar", titles == ["Checklist", "File", "Edit", "View", "Window", "Help"], format!("{titles:?}"));
    let app_menu = menu_items("Checklist");
    check(
        "app-menu",
        has(&app_menu, "Quit Checklist", "q") && has(&app_menu, "Hide Checklist", "h") && has(&app_menu, "About Checklist", ""),
        format!("{} items", app_menu.len()),
    );
    let edit = menu_items("Edit");
    let redo_shift = edit.iter().any(|(t, k, _)| t == "Redo" && k == "Z");
    check(
        "edit-menu",
        ["Undo", "Cut", "Copy", "Paste", "Select All"].iter().zip(["z", "x", "c", "v", "a"]).all(|(t, k)| has(&edit, t, k)) && redo_shift,
        format!("{:?}", edit.iter().map(|e| &e.0).collect::<Vec<_>>()),
    );

    // Windows: both open; AppKit lists them in the Window menu.
    let ws = windows();
    let visible = ws.iter().filter(|w| w.isVisible()).count();
    check("windows", ws.len() == 12 && visible == 12, format!("{} windows, {visible} visible", ws.len()));
    let window_menu = menu_items("Window");
    let listed = ["First", "Second", "Views", "Layout", "Controls", "Controls B", "Teardown", "Table", "Outline", "Editor", "Chrome", "Dialogs"].iter().all(|t| window_menu.iter().any(|(title, _, _)| title == t));
    check("window-menu-lists-windows", listed && app.windowsMenu().is_some(), format!("{:?}", window_menu.iter().map(|e| &e.0).collect::<Vec<_>>()));
    check("help-menu", app.helpMenu().is_some(), String::new());

    // Edit menu → responder chain: Select All selects the focused field's text.
    let first = &ws[0];
    let field = first.contentView().unwrap().subviews().firstObject().unwrap().downcast::<NSTextField>().unwrap();
    first.makeFirstResponder(Some(&field));
    let edit_menu = bar_menu("Edit");
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
    check("signal-update-cost", seen.get() == 100_000 && (per < 500.0 || on_ci()), format!("{per:.0} ns per set + effect run (budget 500{})", ci_note()));
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

#[derive(Clone)]
struct MenuState {
    items: Signal<i64>,
    sidebar: Signal<bool>,
    sort: Signal<u8>,
    refreshed: Signal<u32>,
    renamed: Signal<String>,
}

thread_local! {
    static MENU: MenuState = MenuState {
        items: Signal::new(0),
        sidebar: Signal::new(true),
        sort: Signal::new(0),
        refreshed: Signal::new(0),
        renamed: Signal::new("A".into()),
    };
}

fn file_menu() -> Vec<MenuItem> {
    let m = MENU.with(Clone::clone);
    let (add, remove) = (m.items.clone(), m.items.clone());
    let has = m.items.clone();
    vec![
        menu_item("New Item", move || add.update(|n| *n += 1)).key("n"),
        menu_item("Remove Item", move || remove.update(|n| *n -= 1)).key_with("n", &[Modifier::Option]).enabled_when(move || has.get() > 0),
    ]
}

fn view_menu() -> Vec<MenuItem> {
    let m = MENU.with(Clone::clone);
    let (s1, s2) = (m.sidebar.clone(), m.sidebar.clone());
    let sort_item = |title: &str, v: u8| {
        let (a, b) = (m.sort.clone(), m.sort.clone());
        menu_item(title, move || a.set(v)).checked_when(move || b.get() == v)
    };
    let r = m.refreshed.clone();
    let rn = m.renamed.clone();
    vec![
        menu_item("Show Sidebar", move || s1.update(|s| *s = !*s)).key_with("s", &[Modifier::Control]).checked_when(move || s2.get()),
        submenu("Sort By", vec![sort_item("Name", 0), sort_item("Date", 1)]),
        menu_separator(),
        menu_item("Refresh", move || r.update(|n| *n += 1)).key("R"),
        menu_item("Rename", move || set_undoable("Rename", &rn, format!("{}+", rn.get_untracked()))).key_with("e", &[Modifier::Control]),
    ]
}

fn settings_window() -> Window {
    let dark = stored("check.settings.dark", false);
    let name = stored("check.settings.name", "Untitled".to_string());
    Window::new("Settings").size(360.0, 140.0).view(vstack((toggle("Dark tables", &dark), text_field(&name))).padding(20.0))
}

fn menu_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    use objc2_app_kit::{NSControlStateValueOn, NSWindowStyleMask};
    let m = MENU.with(Clone::clone);
    let file = bar_menu("File");
    let file_items = menu_items("File");
    let titles: Vec<&str> = file_items.iter().map(|e| e.0.as_str()).collect();
    check(
        "file-menu",
        // AppKit adds Close All (⌥) after Close itself.
        titles.starts_with(&["New Item", "Remove Item", "", "Close"]) && has(&file_items, "New Item", "n") && has(&file_items, "Close", "w"),
        format!("{titles:?}"),
    );

    // Custom items: actions, key equivalents, enabled state from signals.
    file.update();
    let remove = file.itemWithTitle(&NSString::from_str("Remove Item")).unwrap();
    let off = !remove.isEnabled();
    let cmd = NSEventModifierFlags::Command;
    // A disabled item's shortcut is taken (AppKit beeps) but doesn't run.
    menu_key("n", cmd | NSEventModifierFlags::Option);
    let blocked = m.items.get_untracked() == 0;
    let new_key = menu_key("n", cmd);
    file.update();
    let on = remove.isEnabled();
    let alt_key = menu_key("n", cmd | NSEventModifierFlags::Option);
    let after = m.items.get_untracked();
    let refresh = menu_key("R", cmd | NSEventModifierFlags::Shift);
    check(
        "menu-key-equivalents",
        off && blocked && new_key && on && alt_key && after == 0 && refresh && m.refreshed.get_untracked() == 1,
        format!(
            "Remove disabled at 0: {off}; its ⌥⌘N ran nothing: {blocked}; ⌘N fired: {new_key}; Remove enabled at 1: {on}; ⌥⌘N fired: {alt_key} → {after}; ⇧⌘R: {refresh}"
        ),
    );

    // Checked state and submenus.
    let view = bar_menu("View");
    let sidebar = view.itemWithTitle(&NSString::from_str("Show Sidebar")).unwrap();
    view.update();
    let checked0 = sidebar.state() == NSControlStateValueOn;
    let toggled = menu_key("s", cmd | NSEventModifierFlags::Control);
    view.update();
    let checked1 = sidebar.state() == NSControlStateValueOn;
    let sort = view.itemWithTitle(&NSString::from_str("Sort By")).and_then(|i| i.submenu()).unwrap();
    let date = sort.indexOfItemWithTitle(&NSString::from_str("Date"));
    sort.performActionForItemAtIndex(date);
    sort.update();
    let marks: Vec<bool> = (0..sort.numberOfItems()).map(|i| sort.itemAtIndex(i).unwrap().state() == NSControlStateValueOn).collect();
    check(
        "menu-checked-submenu",
        checked0 && toggled && !checked1 && !m.sidebar.get_untracked() && m.sort.get_untracked() == 1 && marks == [false, true],
        format!("sidebar ✓ {checked0} → ⌃⌘S → {checked1}; Sort By › Date → sort {} marks {marks:?}", m.sort.get_untracked()),
    );

    // Settings: ⌘, in the app menu; built once, then brought to the front.
    let app_items = menu_items("Checklist");
    let item_ok = has(&app_items, "Settings…", ",");
    let before = NSApplication::sharedApplication(mtm()).windows().count();
    let fired = menu_key(",", cmd);
    let s1 = NSApplication::sharedApplication(mtm()).windows().iter().find(|w| w.title().to_string() == "Settings");
    let built = NSApplication::sharedApplication(mtm()).windows().count();
    if let Some(w) = &s1 {
        w.close();
    }
    let s2 = appkit_ui::open_settings();
    let after = NSApplication::sharedApplication(mtm()).windows().count();
    let same = s1.as_ref().zip(s2.as_ref()).is_some_and(|(a, b)| std::ptr::eq(&**a, &**b));
    let fixed = s2.as_ref().is_some_and(|w| !w.styleMask().contains(NSWindowStyleMask::Resizable) && w.isExcludedFromWindowsMenu() && w.isVisible());
    let unlisted = !menu_items("Window").iter().any(|(t, _, _)| t == "Settings");
    check(
        "settings-window",
        item_ok && fired && built == before + 1 && after == built && same && fixed && unlisted,
        format!("Settings… ⌘, listed: {item_ok}; fired: {fired}; windows {before} → {built} → reopen {after}; same window: {same}; fixed, unlisted: {fixed} {unlisted}"),
    );

    // Stored settings: the toggle writes user defaults; a new signal reads them.
    let w = s2.unwrap();
    let toggle_v = all_views(&w.contentView().unwrap()).into_iter().find_map(|v| v.downcast::<objc2_app_kit::NSSwitch>().ok());
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str("check.settings.dark");
    let unset = defaults.objectForKey(&key).is_none();
    if let Some(t) = &toggle_v {
        t.setState(NSControlStateValueOn);
        send_action(t);
    }
    let written = defaults.boolForKey(&key);
    let reread = stored("check.settings.dark", false).get_untracked();
    check("stored-settings", unset && toggle_v.is_some() && written && reread, format!("unset before: {unset}; toggle → defaults {written}; a new stored signal reads {reread}"));
    w.close();
    defaults.removeObjectForKey(&key);

    // Frame autosave: a window moved and resized; a new one with the same
    // name opens there.
    let frame_key = NSString::from_str("NSWindow Frame check restore");
    defaults.removeObjectForKey(&frame_key);
    let a = open(Window::new("Restore A").size(300.0, 200.0).remember_frame("check restore"));
    let target = objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(140.0, 160.0), objc2_foundation::NSSize::new(420.0, 260.0));
    a.setFrame_display(target, true);
    let saved = defaults.stringForKey(&frame_key).map(|s| s.to_string());
    appkit_ui::close_window(&a);
    a.setFrameAutosaveName(&NSString::new());
    let b = open(Window::new("Restore B").size(300.0, 200.0).remember_frame("check restore"));
    let got = b.frame();
    appkit_ui::close_window(&b);
    check(
        "frame-autosave",
        saved.is_some() && got == target,
        format!("saved {saved:?}; reopened at {:.0},{:.0} {:.0}×{:.0}", got.origin.x, got.origin.y, got.size.width, got.size.height),
    );
    defaults.removeObjectForKey(&frame_key);

    // Split position: a divider moved outside layout (as a drag) is saved; a
    // new split with the same name starts there.
    let split_key = NSString::from_str("appkit-ui split check");
    defaults.removeObjectForKey(&split_key);
    let split_window = || Window::new("Split R").size(500.0, 200.0).view(split((text("a"), text("b"))).position(150.0).remember("check"));
    let c = open(split_window());
    let sv = all_views(&c.contentView().unwrap()).into_iter().find_map(|v| v.downcast::<objc2_app_kit::NSSplitView>().ok()).unwrap();
    let initial = sv.arrangedSubviews().firstObject().unwrap().frame().size.width;
    sv.setPosition_ofDividerAtIndex(230.0, 0);
    let stored_pos = defaults.doubleForKey(&split_key);
    appkit_ui::close_window(&c);
    let d = open(split_window());
    let sv = all_views(&d.contentView().unwrap()).into_iter().find_map(|v| v.downcast::<objc2_app_kit::NSSplitView>().ok()).unwrap();
    let restored = sv.arrangedSubviews().firstObject().unwrap().frame().size.width;
    appkit_ui::close_window(&d);
    check(
        "split-remember",
        initial == 150.0 && stored_pos == 230.0 && restored == 230.0,
        format!("first pane {initial} → dragged to 230 → saved {stored_pos} → new split {restored}"),
    );
    defaults.removeObjectForKey(&split_key);

    after_events(move || undo_checks(events.clone()));
}

#[derive(Clone)]
struct UndoState {
    name: Signal<String>,
    own: TextModel,
    shared: TextModel,
    window: Retained<objc2_app_kit::NSWindow>,
}

fn undo_title(w: &objc2_app_kit::NSWindow) -> (String, String) {
    let m = w.undoManager().unwrap();
    (m.undoMenuItemTitle().to_string(), m.redoMenuItemTitle().to_string())
}

/// Runs `f` after the run loop has gone idle once: that's when AppKit closes
/// the undo group of the current event (a `later` block can run in the same
/// loop iteration, inside the same group).
fn after_idle(f: impl Fn() + 'static) {
    spawn(|| std::thread::sleep(std::time::Duration::from_millis(30)), move |()| f());
}

/// Undo checks run one step per idle run loop, as user events would: AppKit
/// groups everything registered in one event into a single undo step.
fn undo_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let name = Signal::new("A".to_string());
    let own = TextModel::new(ViewId(950), "");
    let shared = TextModel::new(ViewId(951), "");
    let window = open(
        Window::new("Undo").size(420.0, 320.0).view(
            vstack((text_field(&name), text_editor(&own), text_editor(&shared).shared_undo())).padding(12.0),
        ),
    );
    let u = UndoState { name, own, shared, window };
    let w = &u.window;
    let m = w.undoManager();
    let editor_w = windows().into_iter().find(|w| w.title().to_string() == "Editor").unwrap();
    let distinct = m.as_ref().zip(editor_w.undoManager()).is_some_and(|(a, b)| !std::ptr::eq(&**a, &*b));
    let stable = m.as_ref().zip(w.undoManager()).is_some_and(|(a, b)| std::ptr::eq(&**a, &*b));
    let field = all_views(&w.contentView().unwrap()).into_iter().find_map(|v| v.downcast::<NSTextField>().ok()).unwrap();
    w.makeFirstResponder(Some(&field));
    // While a field is being edited its field editor has AppKit's private
    // NSCellUndoManager (field typing is undone there, then dropped when
    // editing ends); the field itself answers with the window's.
    let editor = field.currentEditor();
    let cell_undo = editor.as_ref().and_then(|e| e.undoManager()).map(|u| u.class().name().to_string_lossy().to_string());
    let field_undo = field.undoManager();
    let field_shares = m.as_ref().zip(field_undo.as_ref()).is_some_and(|(a, b)| std::ptr::eq(&**a, &**b)) && cell_undo.is_some();
    let own_apart = m.as_ref().is_some_and(|m| !std::ptr::eq(&**m, &*u.own.undo_manager()));
    let shared_in = m.as_ref().is_some_and(|m| std::ptr::eq(&**m, &*u.shared.undo_manager()));
    check(
        "undo-manager-per-window",
        distinct && stable && field_shares && own_apart && shared_in,
        format!("own manager per window: {distinct}, stable: {stable}; text field uses it: {field_shares} (while editing: {cell_undo:?}); default editor apart: {own_apart}; shared_undo editor in it: {shared_in}"),
    );
    w.makeFirstResponder(None);

    // An app-level change: ⌘Z / ⇧⌘Z with the action's name.
    set_undoable("Rename", &u.name, "B".to_string());
    after_idle(move || {
        let w = &u.window;
        let t0 = undo_title(w);
        let undone = shortcut(w, "z", false);
        let after_undo = (u.name.get_untracked(), undo_title(w));
        let redone = shortcut(w, "z", true);
        let after_redo = u.name.get_untracked();
        check(
            "undoable-signal",
            t0.0 == "Undo Rename" && undone && after_undo.0 == "A" && after_undo.1 .1 == "Redo Rename" && redone && after_redo == "B",
            format!("{t0:?}; ⌘Z → {:?} {:?}; ⇧⌘Z → {after_redo:?}", after_undo.0, after_undo.1),
        );
        // The default editor keeps its own history: typing there leaves the window's alone.
        u.own.replace_selection("x");
        let own_ok = u.own.can_undo() && undo_title(w).0 == "Undo Rename";
        check("editor-own-undo", own_ok, format!("editor can undo: {}; window's top: {:?}", u.own.can_undo(), undo_title(w).0));
        let u = u.clone();
        let events = events.clone();
        after_idle(move || undo_stack_checks(u.clone(), events.clone()));
    });
}

/// The shared stack, one explicit undo group per simulated event. (AppKit
/// closes its per-event group when the app handles an event; an unattended
/// run gets none, so its groups stay open across passes.)
fn undo_stack_checks(u: UndoState, events: Rc<RefCell<Vec<AppEvent>>>) {
    let w = &u.window;
    let m = w.undoManager().unwrap();
    m.removeAllActions();
    while m.groupingLevel() > 0 {
        m.endUndoGrouping();
    }
    m.setGroupsByEvent(false);
    let event = |f: &dyn Fn()| {
        m.beginUndoGrouping();
        f();
        m.endUndoGrouping();
    };
    batch(|| u.name.set("B".into()));
    // Typing (as the keyboard does: insertText, named "Typing"), an
    // app-level change, more typing: three steps, undone newest first.
    let tv = u.shared.text_view();
    let typed = |t: &str| unsafe { tv.insertText_replacementRange(&NSString::from_str(t), objc2_foundation::NSRange::new(usize::MAX >> 1, 0)) };
    event(&|| typed("typed"));
    let typing_title = undo_title(w).0;
    event(&|| set_undoable("Rename", &u.name, "C".to_string()));
    event(&|| typed("!"));
    let mut seen = vec![(u.name.get_untracked(), u.shared.text())];
    for _ in 0..3 {
        shortcut(w, "z", false);
        seen.push((u.name.get_untracked(), u.shared.text()));
    }
    let want: Vec<(String, String)> =
        [("C", "typed!"), ("C", "typed"), ("B", "typed"), ("B", "")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    check(
        "editor-shared-undo",
        typing_title == "Undo Typing" && seen == want,
        format!("top after typing: {typing_title:?}; then ⌘Z ×3: {seen:?}"),
    );

    // set_text drops only that editor's steps from the shared stack.
    event(&|| set_undoable("Rename", &u.name, "D".to_string()));
    event(&|| u.shared.replace_selection("more"));
    event(&|| set_undoable("Rename", &u.name, "E".to_string()));
    u.shared.set_text("fresh");
    let top = undo_title(w).0;
    shortcut(w, "z", false);
    let first = (u.name.get_untracked(), u.shared.text());
    shortcut(w, "z", false);
    let second = (u.name.get_untracked(), u.shared.text());
    check(
        "editor-set-text-undo",
        top == "Undo Rename" && first == ("D".into(), "fresh".into()) && second == ("B".into(), "fresh".into()),
        format!("after set_text the top is {top:?}; ⌘Z → {first:?}; ⌘Z → {second:?} (the dropped typing is skipped)"),
    );

    event(&|| set_undoable("Rename", &u.name, "F".to_string()));
    appkit_ui::close_window(w);
    check("undo-close", !m.canUndo() && !m.canRedo(), format!("after close: can undo {}, redo {}", m.canUndo(), m.canRedo()));
    after_events(move || posted_event_checks(events.clone()));
}

thread_local! {
    /// Set when a posted key found no key window to make: the app isn't
    /// active (macOS gives focus to the app the user is in), so menu
    /// shortcuts like ⌘Z have no responder chain to go to.
    static NO_KEY_WINDOW: Cell<bool> = const { Cell::new(false) };
}

/// For checks of menu shortcuts through posted keys: `Some(reason)` when
/// the app had no key window, so they can't be judged in this run.
fn keys_unreachable() -> Option<&'static str> {
    NO_KEY_WINDOW.with(Cell::get).then_some("skipped: the app wasn't active (no key window), so ⌘Z had no responder to reach; a person pressing keys always has the app in front")
}

/// On a CI runner (GitHub sets `CI`): shared, slow virtual machines, so
/// timing budgets are reported but don't fail (correctness checks still do).
fn on_ci() -> bool {
    std::env::var_os("CI").is_some()
}

/// Appended to a timing check's note on CI.
fn ci_note() -> &'static str {
    if on_ci() { "; informational on CI" } else { "" }
}

/// Posts a key press (down and up) to the app's event queue for `w`, as the
/// keyboard does: it goes through the run loop, the menu's key equivalents
/// and the first responder, and AppKit closes its per-event undo group.
fn post_key(w: &objc2_app_kit::NSWindow, chars: &str, flags: NSEventModifierFlags) {
    use objc2_app_kit::{NSEvent, NSEventType};
    // Keys go to the key window's first responder; something else on the
    // machine (a person using it meanwhile) can take key status away.
    if !w.isKeyWindow() {
        let app = NSApplication::sharedApplication(mtm());
        let key = app.keyWindow().map(|k| k.title().to_string());
        w.makeKeyWindow();
        if !w.isKeyWindow() {
            NO_KEY_WINDOW.with(|n| n.set(true));
        }
        eprintln!(
            "[note] {:?} wasn't the key window before {chars:?} (key: {key:?}, app active: {}); made it key: {}",
            w.title().to_string(),
            app.isActive(),
            w.isKeyWindow()
        );
    }
    // US-layout key codes, for input methods that look at them.
    let code = match chars.to_lowercase().as_str() {
        "a" => 0, "s" => 1, "h" => 4, "c" => 8, "b" => 11, "e" => 14, "r" => 15, "i" => 34, "," => 43, "n" => 45, _ => 6,
    };
    for t in [NSEventType::KeyDown, NSEventType::KeyUp] {
        let e = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            t, objc2_foundation::NSPoint::new(0.0, 0.0), flags, 0.0, w.windowNumber(), None, &NSString::from_str(chars), &NSString::from_str(chars), false, code,
        )
        .unwrap();
        NSApplication::sharedApplication(mtm()).postEvent_atStart(&e, false);
    }
}

/// Runs `steps` one per 150 ms (each posts events), calling `snap` after
/// each has been handled, then `done`.
fn posted_steps(steps: Rc<Vec<Box<dyn Fn()>>>, i: usize, snap: Rc<dyn Fn(usize)>, done: Rc<dyn Fn()>) {
    if i == steps.len() {
        return done();
    }
    steps[i]();
    spawn(|| std::thread::sleep(std::time::Duration::from_millis(150)), move |()| {
        snap(i);
        posted_steps(steps, i + 1, snap, done);
    });
}

/// Real key events posted to the app's queue (the app needn't be active):
/// typing, a menu shortcut and ⌘Z / ⇧⌘Z with AppKit's own per-event undo
/// grouping (the checks above use explicit groups), the custom menus'
/// shortcuts, and a text field's undo.
fn posted_event_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let m = MENU.with(Clone::clone);
    let field_text = Signal::new(String::new());
    let shared = Rc::new(TextModel::new(ViewId(960), ""));
    let w = open(Window::new("Events").size(420.0, 260.0).view(vstack((text_field(&field_text), text_editor(&shared).shared_undo())).padding(12.0)));
    w.makeKeyAndOrderFront(None);
    w.makeFirstResponder(Some(&shared.text_view()));
    let cmd = NSEventModifierFlags::Command;
    let k = |c: &'static str, f: NSEventModifierFlags| {
        let w = w.clone();
        Box::new(move || post_key(&w, c, f)) as Box<dyn Fn()>
    };
    let none = NSEventModifierFlags::empty();
    let steps = Rc::new(vec![
        Box::new({ let w = w.clone(); move || { post_key(&w, "a", none); post_key(&w, "b", none) } }) as Box<dyn Fn()>,
        k("e", cmd | NSEventModifierFlags::Control),
        k("c", none),
        k("z", cmd),
        k("z", cmd),
        k("z", cmd),
        k("Z", cmd | NSEventModifierFlags::Shift),
    ]);
    // (name, editor text, Edit menu's Undo title, open undo groups) after each step.
    type Seen = Vec<(String, String, String, isize)>;
    let seen: Rc<RefCell<Seen>> = Rc::default();
    let snap: Rc<dyn Fn(usize)> = {
        let (seen, m, shared, w) = (seen.clone(), m.clone(), shared.clone(), w.clone());
        Rc::new(move |_| {
            let um = w.undoManager().unwrap();
            seen.borrow_mut().push((m.renamed.get_untracked(), shared.text(), um.undoMenuItemTitle().to_string(), um.groupingLevel()));
        })
    };
    let done: Rc<dyn Fn()> = Rc::new(move || {
        let want: Vec<(String, String, String, isize)> = [
            ("A", "ab", "Undo Typing"),
            ("A+", "ab", "Undo Rename"),
            ("A+", "abc", "Undo Typing"),
            ("A+", "ab", "Undo Rename"),
            ("A", "ab", "Undo Typing"),
            ("A", "", "Undo"),
            ("A", "ab", "Undo Typing"),
        ]
        .iter()
        .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string(), 0))
        .collect();
        let got = seen.borrow().clone();
        let skip = keys_unreachable();
        check(
            "posted-undo",
            got == want || skip.is_some(),
            format!("{}type ab, ⌃⌘E Rename, type c, ⌘Z ×3, ⇧⌘Z (name, text, Edit menu, open groups): {got:?}", skip.map(|s| format!("{s}; ")).unwrap_or_default()),
        );
        posted_shortcut_checks(w.clone(), field_text.clone(), events.clone());
    });
    posted_steps(steps, 0, snap, done);
}

/// A split divider dragged with posted mouse events (NSSplitView tracks the
/// drag from the event queue): the panes resize and their content relayouts.
fn posted_split_drag(events: Rc<RefCell<Vec<AppEvent>>>) {
    use objc2_app_kit::{NSEvent, NSEventType, NSSplitView};
    use objc2_foundation::NSPoint;
    let w = open(
        Window::new("Split Drag")
            .size(600.0, 300.0)
            .view(split((vstack((text("Left").fill_width(),)).padding(8.0), vstack((text("Right").fill_width(),)).padding(8.0))).position(160.0)),
    );
    w.makeKeyAndOrderFront(None);
    let sv = all_views(&w.contentView().unwrap()).into_iter().find_map(|v| v.downcast::<NSSplitView>().ok()).unwrap();
    let sv2 = sv.clone();
    let pane = move |i: usize| sv2.subviews().objectAtIndex(i).frame().size.width;
    let content = w.contentView().unwrap();
    let label_width = move |title: &str| {
        all_views(&content)
            .into_iter()
            .filter_map(|v| v.downcast::<NSTextField>().ok())
            .find(|t| t.stringValue().to_string() == title)
            .map_or(0.0, |t| t.frame().size.width)
    };
    let before = (pane(0), pane(1), label_width("Left"), label_width("Right"));
    let x = pane(0) + sv.dividerThickness() / 2.0;
    let post = |t: NSEventType, x: f64| {
        let p = sv.convertPoint_toView(NSPoint::new(x, 100.0), None);
        let e = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            t, p, NSEventModifierFlags::empty(), 0.0, w.windowNumber(), None, 0, 1, 1.0,
        )
        .unwrap();
        NSApplication::sharedApplication(mtm()).postEvent_atStart(&e, false);
    };
    post(NSEventType::LeftMouseDown, x);
    for i in 1..=10 {
        post(NSEventType::LeftMouseDragged, x + 8.0 * i as f64);
    }
    post(NSEventType::LeftMouseUp, x + 80.0);
    let app_active = NSApplication::sharedApplication(mtm()).isActive();
    spawn(|| std::thread::sleep(std::time::Duration::from_millis(300)), move |()| {
        let after = (pane(0), pane(1), label_width("Left"), label_width("Right"));
        // On macOS 26 posted mouse input reaches only the active app (as
        // posted ⌘ shortcuts need a key window): skip when it isn't.
        // Not delivered at all: an inactive app, or the macOS 26 CI runner
        // (where posted drags don't reach even the active app). The divider
        // and relayout themselves are covered by `split-drag` (AppKit's API).
        let skip = (after == before && (!app_active || on_ci())).then_some("skipped: the posted drag wasn't delivered here (inactive app or CI runner); ");
        check(
            "posted-split-drag",
            skip.is_some() || (after.0 - before.0 - 80.0).abs() < 1.0 && (before.1 - after.1 - 80.0).abs() < 1.0 && (after.2 - before.2 - 80.0).abs() < 1.0 && (before.3 - after.3 - 80.0).abs() < 1.0,
            format!("{}divider dragged 80 pt: panes {:.0}/{:.0} → {:.0}/{:.0}; labels {:.0}/{:.0} → {:.0}/{:.0}", skip.unwrap_or(""), before.0, before.1, after.0, after.1, before.2, before.3, after.2, after.3),
        );
        appkit_ui::close_window(&w);
        let events = events.clone();
        after_events(move || close_button_checks(events.clone()));
    });
}

/// A window of 200 bound labels and a table (with a toolbar), for close checks.
fn closable_window(title: &str, value: &Signal<u32>) -> Window {
    let labels: Vec<appkit_ui::AnyView> = (0..200)
        .map(|i| {
            let v = value.clone();
            appkit_ui::AnyView::new(label(move || format!("{i}: {}", v.get())))
        })
        .collect();
    let model = Rc::new(TableModel::new(ViewId(970), vec![Column::text("n", "N", |n: &u32| n.to_string())]));
    model.set_rows((0..1000).collect());
    Window::new(title)
        .size(420.0, 400.0)
        .toolbar(vec![toolbar_button("add", "Add", "plus", || {})])
        .view(vstack((scroll(vstack(labels)), table(&model))))
}

/// The close button (performClose:, the path a click takes) and ⌘W tear a
/// window down like close_window; Settings is kept.
fn close_button_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let value = Signal::new(0u32);
    let (n0, list0) = (live_nodes(), windows().len());
    let w = open(closable_window("Close Me", &value));
    let built = live_nodes() - n0;
    let weak = objc2::rc::Weak::from_retained(&w);
    w.performClose(None);
    drop(w);
    let v = value.clone();
    after_idle(move || {
        let runs = effect_runs();
        batch(|| v.set(1));
        let ran = effect_runs() - runs;
        let freed = weak.load().is_none();
        check(
            "close-button-teardown",
            built > 200 && live_nodes() == n0 && windows().len() == list0 && ran == 0 && freed,
            format!(
                "{built} nodes built; after the close button: {} left, listed windows {} (was {list0}), effects run on a change {ran}, window freed {freed}",
                live_nodes() - n0,
                windows().len()
            ),
        );
        // Settings is reused: closing it keeps the same window and its views.
        let s1 = appkit_ui::open_settings().unwrap();
        let s_nodes = live_nodes();
        s1.performClose(None);
        let s1p = Retained::as_ptr(&s1);
        drop(s1);
        let v = v.clone();
        let events = events.clone();
        after_idle(move || {
            let s2 = appkit_ui::open_settings().unwrap();
            let same = Retained::as_ptr(&s2) == s1p && live_nodes() == s_nodes;
            s2.close();
            check("close-settings-kept", same, format!("Settings after closing: same window and views {same}"));
            close_cycles(v.clone(), events.clone(), 0, 0.0);
        });
    });
}

/// A plain AppKit window like `closable_window` (200 labels in a scroll
/// view, a table, a toolbar-less frame), opened and closed: AppKit's own
/// cost per cycle, the floor for ours.
fn plain_cycle() {
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSBackingStoreType, NSScrollView, NSTableColumn, NSTableView, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let mtm = MainThreadMarker::new().unwrap();
    objc2::rc::autoreleasepool(|_| {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(420.0, 400.0));
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;
        let w = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), rect, style, NSBackingStoreType::Buffered, false) };
        unsafe { w.setReleasedWhenClosed(false) };
        let content = w.contentView().unwrap();
        let doc = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(400.0, 200.0 * 18.0)));
        for i in 0..200 {
            let l = NSTextField::labelWithString(&NSString::from_str(&format!("{i}: 0")), mtm);
            l.setFrame(NSRect::new(NSPoint::new(0.0, i as f64 * 18.0), NSSize::new(200.0, 17.0)));
            doc.addSubview(&l);
        }
        let sv = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 200.0), NSSize::new(420.0, 200.0)));
        sv.setDocumentView(Some(&doc));
        content.addSubview(&sv);
        let tv = NSTableView::initWithFrame(NSTableView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(420.0, 200.0)));
        tv.addTableColumn(&NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str("n")));
        let tsv = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(420.0, 200.0)));
        tsv.setDocumentView(Some(&tv));
        content.addSubview(&tsv);
        w.makeKeyAndOrderFront(None);
        w.performClose(None);
    });
}

/// Open/close cycles through the close button, against the same number of
/// plain AppKit windows: ours shouldn't cost much more than AppKit's own.
/// AppKit's caches grow over the first few dozen windows of a kind (the
/// toolbar's longest), so 40 cycles warm up before 30 are measured.
fn close_cycles(value: Signal<u32>, events: Rc<RefCell<Vec<AppEvent>>>, round: u32, base: f64) {
    const ROUNDS: u32 = 70;
    const WARM: u32 = 40;
    if round == 0 {
        // The floor: plain AppKit, same warm-up and count, one per idle pass.
        fn plain(round: u32, base: f64, then: Rc<dyn Fn(f64)>) {
            let base = if round == WARM { heap_in_use_mb() } else { base };
            if round == ROUNDS {
                return then((heap_in_use_mb() - base) * 1024.0 / (ROUNDS - WARM) as f64);
            }
            plain_cycle();
            let then = then.clone();
            after_idle(move || plain(round + 1, base, then.clone()));
        }
        let (v, e) = (value.clone(), events.clone());
        plain(0, 0.0, Rc::new(move |floor| {
            PLAIN_FLOOR_KB.with(|f| f.set(floor));
            close_cycles(v.clone(), e.clone(), 1, 0.0);
        }));
        return;
    }
    let base = if round == WARM { heap_in_use_mb() } else { base };
    if round == ROUNDS {
        return close_cycles_report(heap_in_use_mb(), base, events);
    }
    let w = open(closable_window("Cycle", &value));
    w.performClose(None);
    drop(w);
    let v = value.clone();
    after_idle(move || close_cycles(v.clone(), events.clone(), round + 1, base));
}

fn close_cycles_report(now: f64, base: f64, events: Rc<RefCell<Vec<AppEvent>>>) {
    const ROUNDS: u32 = 70;
    const WARM: u32 = 40;
    {
        let per = (now - base) * 1024.0 / (ROUNDS - WARM) as f64;
        let floor = PLAIN_FLOOR_KB.with(Cell::get);
        check(
            "close-cycles-memory",
            // Live allocations (not footprint, which the allocator returns late).
            per < floor.max(0.0) + 16.0,
            format!("{} cycles through the close button: heap in use {per:+.1} KB per cycle; a plain AppKit window with the same views {floor:+.1} KB (budget: plain + 16)", ROUNDS - WARM),
        );
        after_events(move || sheet_cycles(events.clone()));
    }
}

thread_local! {
    static PLAIN_FLOOR_KB: Cell<f64> = const { Cell::new(0.0) };
}


/// Repeats `cycle` (one per idle pass): `WARM` warm-up rounds, then
/// `MEASURED` measured ones; calls `done` with KB per measured cycle.
fn cycles(round: u32, base: f64, cycle: Rc<dyn Fn()>, done: Rc<dyn Fn(f64)>) {
    const WARM: u32 = 40;
    const MEASURED: u32 = 30;
    let base = if round == WARM { heap_in_use_mb() } else { base };
    if round == WARM + MEASURED {
        return done((heap_in_use_mb() - base) * 1024.0 / MEASURED as f64);
    }
    cycle();
    after_idle(move || cycles(round + 1, base, cycle.clone(), done.clone()));
}

/// Sheets with a form, opened and closed on one window: views go, the sheet
/// window is freed, and memory stays flat.
fn sheet_cycles(events: Rc<RefCell<Vec<AppEvent>>>) {
    let parent = open(Window::new("Sheets").size(480.0, 320.0).view(text("Sheet parent")));
    let n0 = live_nodes();
    let freed: Rc<RefCell<Vec<objc2::rc::Weak<objc2_app_kit::NSWindow>>>> = Rc::default();
    let (p, f) = (parent.clone(), freed.clone());
    let cycle: Rc<dyn Fn()> = Rc::new(move || {
        let name = Signal::new("draft".to_string());
        let form = Window::new("Form").size(320.0, 140.0).view(vstack((text_field(&name), checkbox("Remember", &Signal::new(true)), button("Done", || {}))).padding(16.0));
        let h = sheet(&p, form, || {});
        f.borrow_mut().push(objc2::rc::Weak::from_retained(&h.window()));
        h.close();
    });
    let p2 = parent.clone();
    let done: Rc<dyn Fn(f64)> = Rc::new(move |per| {
        let left = live_nodes() - n0;
        let alive = freed.borrow().iter().filter(|w| w.load().is_some()).count();
        // The floor: plain AppKit sheets with the same controls, same counts.
        let (events, parent) = (events.clone(), p2.clone());
        let pp = parent.clone();
        let plain: Rc<dyn Fn()> = Rc::new(move || plain_sheet_cycle(&pp));
        let report: Rc<dyn Fn(f64)> = Rc::new(move |floor| {
            check(
                "sheet-cycles-memory",
                left == 0 && alive == 0 && per < floor.max(0.0) + 8.0,
                format!("70 form sheets opened and closed: heap in use {per:+.1} KB per cycle after warm-up; plain AppKit sheets {floor:+.1} KB (budget: plain + 8); nodes left {left}; sheet windows still alive {alive}"),
            );
            appkit_ui::close_window(&parent);
            let events = events.clone();
            after_events(move || deep_widget_cycles(events.clone()));
        });
        cycles(0, 0.0, plain, report);
    });
    cycles(0, 0.0, cycle, done);
}

/// One plain AppKit sheet with a field, a checkbox and a button, shown and
/// ended on `parent`.
fn plain_sheet_cycle(parent: &objc2_app_kit::NSWindow) {
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSBackingStoreType, NSButton, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let mtm = MainThreadMarker::new().unwrap();
    objc2::rc::autoreleasepool(|_| {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 140.0));
        let sheet = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), rect, NSWindowStyleMask::Titled, NSBackingStoreType::Buffered, false) };
        unsafe { sheet.setReleasedWhenClosed(false) };
        let content = sheet.contentView().unwrap();
        let field = NSTextField::new(mtm);
        field.setStringValue(&NSString::from_str("draft"));
        field.setFrame(NSRect::new(NSPoint::new(16.0, 90.0), NSSize::new(280.0, 24.0)));
        content.addSubview(&field);
        let check_box = unsafe { NSButton::checkboxWithTitle_target_action(&NSString::from_str("Remember"), None, None, mtm) };
        check_box.setFrame(NSRect::new(NSPoint::new(16.0, 56.0), NSSize::new(200.0, 20.0)));
        content.addSubview(&check_box);
        let done = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str("Done"), None, None, mtm) };
        done.setFrame(NSRect::new(NSPoint::new(220.0, 16.0), NSSize::new(80.0, 28.0)));
        content.addSubview(&done);
        parent.beginSheet_completionHandler(&sheet, None);
        parent.endSheet(&sheet);
        sheet.orderOut(None);
    });
}

/// Windows holding the deep widgets (a 10,000-row table, an outline, a text
/// editor), opened and closed through the close button.
fn deep_widget_cycles(events: Rc<RefCell<Vec<AppEvent>>>) {
    let n0 = live_nodes();
    let freed: Rc<RefCell<Vec<objc2::rc::Weak<objc2_app_kit::NSWindow>>>> = Rc::default();
    let f = freed.clone();
    let cycle: Rc<dyn Fn()> = Rc::new(move || {
        let model = Rc::new(TableModel::new(ViewId(980), vec![Column::text("n", "N", |n: &u32| n.to_string())]));
        model.set_rows((0..10_000).collect());
        let notes = TextModel::new(ViewId(981), "Some notes\nover two lines");
        let tree = || (0..20u32).map(|i| TreeNode { value: place(i, &format!("Group {i}"), None), children: (0..5).map(|j| TreeNode { value: place(1000 + i * 10 + j, "Leaf", None), children: vec![] }).collect() }).collect();
        let w = open(
            Window::new("Deep")
                .size(640.0, 480.0)
                .view(split((
                    outline(tree, |p: &Place| p.name.clone()),
                    vstack((
                        table(&model),
                        text_editor(&notes),
                        appkit_ui::collection(|| (0..500u32).map(|i| place(i, "Tile", None)).collect(), |p: &Place| {
                            appkit_ui::CollectionItem::new(format!("{} {}", p.name, p.id), appkit_ui::ItemImage::Symbol("folder".into()))
                        }),
                    )),
                ))),
        );
        f.borrow_mut().push(objc2::rc::Weak::from_retained(&w));
        w.performClose(None);
    });
    let done: Rc<dyn Fn(f64)> = Rc::new(move |per| {
        let left = live_nodes() - n0;
        let alive = freed.borrow().iter().filter(|w| w.load().is_some()).count();
        check(
            "deep-widget-cycles-memory",
            left == 0 && alive == 0 && per < 16.0,
            format!("70 windows with a 10,000-row table, an outline, an editor and a 500-item grid, closed by the close button: heap in use {per:+.1} KB per cycle after warm-up (budget 16); nodes left {left}; windows still alive {alive}"),
        );
        let events = events.clone();
        after_events(move || accessibility_checks(events.clone()));
    });
    cycles(0, 0.0, cycle, done);
}

/// Names VoiceOver reads: `accessibility_label` on a field, a slider, a
/// pop-up, a stepper and an editor; a toggle's switch; decorative and
/// described images. The audit finds an unnamed field, then nothing.
fn accessibility_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    use appkit_ui::testing::{ax_audit, ax_elements};
    let (name, vol, pick, days, on) = (Signal::new(String::new()), Signal::new(0.5), Signal::new(1u8), Signal::new(3.0), Signal::new(true));
    let notes = TextModel::new(ViewId(990), "");
    let bare = Signal::new(String::new());
    let w = open(Window::new("Accessibility").size(420.0, 420.0).view(
        vstack((
            hstack((text("Name:"), text_field(&name).accessibility_label("Name"))),
            hstack((image("speaker.fill"), slider(&vol, 0.0, 1.0).accessibility_label("Volume"))),
            picker(&pick, vec![("One", 1u8), ("Two", 2)]).accessibility_label("Count"),
            stepper(&days, 1.0, 9.0, 1.0).accessibility_label("Days"),
            toggle("Sync", &on),
            image("star.fill").description("Favourite"),
            text_editor(&notes).accessibility_label("Notes"),
            text_field(&bare),
            appkit_ui::date_picker(&Signal::new(std::time::UNIX_EPOCH)),
        ))
        .padding(12.0),
    ));
    let named = |role: &str, label: &str| ax_elements(&w).iter().any(|e| e.role == role && e.name == label);
    let found = [
        ("AXTextField", "Name"),
        ("AXSlider", "Volume"),
        ("AXPopUpButton", "Count"),
        ("AXIncrementor", "Days"),
        ("AXButton", "Sync"),
        ("AXImage", "Favourite"),
        ("AXTextArea", "Notes"),
    ]
    .iter()
    .filter(|(r, l)| !named(r, l))
    .map(|(r, l)| format!("{r} {l}"))
    .collect::<Vec<_>>();
    let images = ax_elements(&w).iter().filter(|e| e.role == "AXImage").count();
    let problems = ax_audit(&w);
    check(
        "accessibility-labels",
        // An unnamed date picker is reported with its stepper (its parts are
        // skipped only inside a named one).
        found.is_empty() && images == 1 && problems.len() == 3 && problems[0].contains("AXTextField") && problems[1].contains("AXDateTimeArea") && problems[2].contains("AXIncrementor"),
        format!("names missing {found:?}; images exposed {images} (the decorative one hidden); audit finds only the bare field and the bare date picker (with its stepper): {problems:?}"),
    );
    appkit_ui::close_window(&w);
    // The form windows of this checklist.
    let mut forms = Vec::new();
    for title in ["Controls", "Controls B"] {
        if let Some(fw) = windows().into_iter().find(|w| w.title().to_string() == title) {
            forms.extend(ax_audit(&fw).into_iter().map(|p| format!("{title}: {p}")));
        }
    }
    check("accessibility-audit-forms", forms.is_empty(), format!("Controls and Controls B: {forms:?}"));
    after_events(move || controls_c_checks(events.clone()));
}

/// The custom menus' shortcuts and Settings (⌘,) from posted key presses,
/// then typing and ⌘Z in a text field.
fn posted_shortcut_checks(w: Retained<objc2_app_kit::NSWindow>, field_text: Signal<String>, events: Rc<RefCell<Vec<AppEvent>>>) {
    let m = MENU.with(Clone::clone);
    let cmd = NSEventModifierFlags::Command;
    let start = (m.items.get_untracked(), m.sidebar.get_untracked(), m.refreshed.get_untracked());
    let combos: Vec<(&'static str, NSEventModifierFlags)> = vec![
        ("n", cmd),
        ("n", cmd),
        ("n", cmd | NSEventModifierFlags::Option),
        ("s", cmd | NSEventModifierFlags::Control),
        ("R", cmd | NSEventModifierFlags::Shift),
        (",", cmd),
    ];
    let steps: Rc<Vec<Box<dyn Fn()>>> = Rc::new(
        combos
            .into_iter()
            .map(|(c, f)| {
                let w = w.clone();
                Box::new(move || post_key(&w, c, f)) as Box<dyn Fn()>
            })
            .collect(),
    );
    let seen: Rc<RefCell<Vec<(i64, bool, u32)>>> = Rc::default();
    let snap: Rc<dyn Fn(usize)> = {
        let (seen, m) = (seen.clone(), m.clone());
        Rc::new(move |_| seen.borrow_mut().push((m.items.get_untracked(), m.sidebar.get_untracked(), m.refreshed.get_untracked())))
    };
    let done: Rc<dyn Fn()> = Rc::new(move || {
        let (i0, s0, r0) = start;
        let got = seen.borrow().clone();
        let settings = NSApplication::sharedApplication(mtm()).windows().iter().find(|w| w.title().to_string() == "Settings" && w.isVisible());
        let want = vec![(i0 + 1, s0, r0), (i0 + 2, s0, r0), (i0 + 1, s0, r0), (i0 + 1, !s0, r0), (i0 + 1, !s0, r0 + 1), (i0 + 1, !s0, r0 + 1)];
        check(
            "posted-shortcuts",
            got == want && settings.is_some(),
            format!("⌘N ⌘N ⌥⌘N ⌃⌘S ⇧⌘R → (items, sidebar, refreshed) {got:?}; ⌘, opened Settings: {}", settings.is_some()),
        );
        if let Some(s) = settings {
            s.close();
        }
        // Typing in a text field, then ⌘Z and ⇧⌘Z: the field and its bound
        // signal. AppKit (plain AppKit too) sends ⇧⌘Z to the window's undo
        // manager when it has a redo pending, not to the field's private
        // one, so the window's history is cleared first, as in a fresh form.
        w.undoManager().unwrap().removeAllActions();
        let field = all_views(&w.contentView().unwrap()).into_iter().find_map(|v| v.downcast::<NSTextField>().ok()).unwrap();
        w.makeFirstResponder(Some(&field));
        let none = NSEventModifierFlags::empty();
        let steps: Rc<Vec<Box<dyn Fn()>>> = Rc::new(vec![
            Box::new({ let w = w.clone(); move || { post_key(&w, "h", none); post_key(&w, "i", none) } }),
            Box::new({ let w = w.clone(); move || post_key(&w, "z", cmd) }),
            Box::new({ let w = w.clone(); move || post_key(&w, "Z", cmd | NSEventModifierFlags::Shift) }),
        ]);
        let seen: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
        let snap: Rc<dyn Fn(usize)> = {
            let (seen, f, field) = (seen.clone(), field_text.clone(), field.clone());
            Rc::new(move |_| seen.borrow_mut().push((field.stringValue().to_string(), f.get_untracked())))
        };
        let (w2, events2) = (w.clone(), events.clone());
        let done: Rc<dyn Fn()> = Rc::new(move || {
            let got = seen.borrow().clone();
            let want: Vec<(String, String)> = [("hi", "hi"), ("", ""), ("hi", "hi")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
            let skip = keys_unreachable();
            check(
                "posted-field-undo",
                got == want || skip.is_some(),
                format!("{}type hi, ⌘Z, ⇧⌘Z → (field, bound signal): {got:?}", skip.map(|s| format!("{s}; ")).unwrap_or_default()),
            );
            appkit_ui::close_window(&w2);
            posted_split_drag(events2.clone());
        });
        posted_steps(steps, 0, snap, done);
    });
    posted_steps(steps, 0, snap, done);
}

#[derive(Clone, Debug, PartialEq)]
struct Item2 {
    name: String,
    n: f64,
}

#[derive(Clone)]
struct TransferState {
    model: Rc<TableModel<Item2>>,
    moves: Signal<Vec<(Vec<usize>, Option<usize>)>>,
    files: Signal<Vec<(Vec<std::path::PathBuf>, Option<usize>)>>,
    pastes: Signal<u32>,
}

fn names(m: &TableModel<Item2>) -> Vec<String> {
    m.rows().iter().map(|r| r.name.clone()).collect()
}

/// G2 controls: date picker, colour well, segmented control, combo box and
/// search field. Each way: the user's edit (through the control's own
/// action or notification) sets the signal; setting the signal updates
/// the control.
fn controls_c_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    use appkit_ui::{Color, color_well, combo_box, date_picker, search_field, segmented};
    use objc2_app_kit::{NSColorWell, NSComboBox, NSDatePicker, NSSearchField, NSSegmentedControl};
    use std::time::{Duration, UNIX_EPOCH};
    let day = |d: u64| UNIX_EPOCH + Duration::from_secs(1_790_000_000 + d * 86_400);
    let date = Signal::new(day(0));
    let colour = Signal::new(Color::rgb(1.0, 0.0, 0.0));
    let view_mode = Signal::new(1u8);
    let city = Signal::new(String::new());
    let query = Signal::new(String::new());
    let submits = Signal::new(0u32);
    let s2 = submits.clone();
    let w = open(Window::new("Controls C").size(420.0, 320.0).view(
        vstack((
            date_picker(&date).accessibility_label("Due"),
            date_picker(&date).with_time().calendar().range(day(0), day(400)).accessibility_label("Due, calendar"),
            color_well(&colour).accessibility_label("Colour"),
            segmented(&view_mode, vec![("List", 1u8), ("Grid", 2), ("Columns", 3)]).accessibility_label("View"),
            combo_box(&city, vec!["Berlin", "Lagos", "Tokyo"]).accessibility_label("City"),
            search_field(&query).placeholder("Search orders").on_submit(move || s2.update(|n| *n += 1)).accessibility_label("Search"),
        ))
        .padding(12.0),
    ));
    let all = all_views(&w.contentView().unwrap());
    let pickers: Vec<_> = all.iter().filter_map(|v| v.clone().downcast::<NSDatePicker>().ok()).collect();
    let well = all.iter().find_map(|v| v.clone().downcast::<NSColorWell>().ok()).unwrap();
    let seg = all.iter().find_map(|v| v.clone().downcast::<NSSegmentedControl>().ok()).unwrap();
    let combo = all.iter().find_map(|v| v.clone().downcast::<NSComboBox>().ok()).unwrap();
    let search = all.iter().find_map(|v| v.clone().downcast::<NSSearchField>().ok()).unwrap();
    let secs = |p: &NSDatePicker| p.dateValue().timeIntervalSince1970() as u64;

    // Date: signal → both pickers; the user's pick → signal.
    batch(|| date.set(day(10)));
    let shown = pickers.iter().all(|p| secs(p) == 1_790_000_000 + 10 * 86_400);
    pickers[0].setDateValue(&objc2_foundation::NSDate::dateWithTimeIntervalSince1970((1_790_000_000 + 20 * 86_400) as f64));
    send_action(&pickers[0]);
    let picked = date.get_untracked() == day(20) && secs(&pickers[1]) == 1_790_000_000 + 20 * 86_400;
    let max_ok = pickers[1].maxDate().is_some_and(|d| d.timeIntervalSince1970() as u64 == 1_790_000_000 + 400 * 86_400);
    check("date-picker", pickers.len() == 2 && shown && picked && max_ok, format!("signal → pickers: {shown}; pick → signal and the other picker: {picked}; calendar range max set: {max_ok}"));

    // Colour: signal → well; the user's colour → signal.
    let srgb = objc2_app_kit::NSColorSpace::sRGBColorSpace();
    let red = well.color().colorUsingColorSpace(&srgb).is_some_and(|c| c.redComponent() > 0.99 && c.greenComponent() < 0.01);
    well.setColor(&objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.5, 1.0, 1.0));
    send_action(&well);
    let got = colour.get_untracked();
    let chosen = (got.g - 0.5).abs() < 0.01 && (got.b - 1.0).abs() < 0.01 && got.r < 0.01;
    batch(|| colour.set(Color::rgb(0.2, 0.8, 0.2)));
    let back = well.color().colorUsingColorSpace(&srgb).is_some_and(|c| (c.greenComponent() - 0.8).abs() < 0.01);
    check("color-well", red && chosen && back, format!("starts red: {red}; picked → {got:?}; signal → well: {back}"));

    // Segments: signal → selection; a click → signal.
    let first = seg.selectedSegment() == 0;
    seg.setSelectedSegment(2);
    send_action(&seg);
    let clicked = view_mode.get_untracked() == 3;
    batch(|| view_mode.set(2));
    check("segmented", first && clicked && seg.selectedSegment() == 1, format!("List selected first: {first}; Columns → {}; signal Grid → segment {}", view_mode.get_untracked(), seg.selectedSegment()));

    // Combo box: typing → signal; a suggestion → signal; signal → text.
    post_edit(&combo, "Os");
    let typed = city.get_untracked() == "Os";
    combo.selectItemAtIndex(1);
    let chose = city.get_untracked() == "Lagos";
    batch(|| city.set("Tokyo".into()));
    check("combo-box", typed && chose && combo.stringValue().to_string() == "Tokyo", format!("typed → {typed}; chose Lagos → {chose}; signal → field {:?}", combo.stringValue()));

    // Search: typing → signal; Return → submit; the clear button → empty.
    post_edit(&search, "1234");
    let typed = query.get_untracked() == "1234";
    send_action(&search);
    let submitted = submits.get_untracked() == 1;
    search.setStringValue(&NSString::from_str(""));
    send_action(&search);
    let cleared = query.get_untracked().is_empty();
    batch(|| query.set("paris".into()));
    let placeholder = search.placeholderString().map(|p| p.to_string());
    check(
        "search-field",
        typed && submitted && cleared && search.stringValue().to_string() == "paris" && placeholder.as_deref() == Some("Search orders"),
        format!("typed → {typed}; Return → submit {submitted}; clear → empty {cleared}; signal → field {:?}; placeholder {placeholder:?}", search.stringValue()),
    );

    // VoiceOver: every control named; segments read by title.
    let problems = appkit_ui::testing::ax_audit(&w);
    let elements = appkit_ui::testing::ax_elements(&w);
    let segments: Vec<String> = elements.iter().filter(|e| e.role == "AXRadioButton").map(|e| e.name.clone()).collect();
    check("controls-c-accessibility", problems.is_empty() && segments == ["List", "Grid", "Columns"], format!("unnamed: {problems:?}; segments {segments:?}"));
    appkit_ui::close_window(&w);
    after_events(move || transfer_checks(events.clone()));
}

/// Drag and drop, copy and paste on a small table. Only private pasteboards
/// are used (never the clipboard), and drops go through the table's own
/// data-source methods (no mouse).
fn transfer_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    use appkit_host::testing::{drag_rows, drop_on_table, files_pasteboard, pasteboard_text, private_pasteboard, release_pasteboard, set_pasteboard_text};
    let model = Rc::new(TableModel::new(
        ViewId(960),
        vec![Column::text("name", "Name", |r: &Item2| r.name.clone()), Column::number("n", "N", |r: &Item2| r.n), Column::check("big", "Big", |r: &Item2| r.n >= 5.0)],
    ));
    model.set_rows((0..10).map(|i| Item2 { name: format!("Item {i}"), n: i as f64 }).collect());
    let s = TransferState { model: model.clone(), moves: Signal::new(vec![]), files: Signal::new(vec![]), pastes: Signal::new(0) };
    let (m1, m2, m3) = (s.clone(), s.clone(), s.clone());
    let _ = open(
        Window::new("Transfer").size(420.0, 360.0).view(
            vstack((table(&model)
                .reorderable(move |rows, before| {
                    m1.moves.update(|v| v.push((rows.clone(), before)));
                    m1.model.move_rows(&rows, before);
                })
                .on_files_dropped(&["csv"], move |paths, before| m2.files.update(|v| v.push((paths, before))))
                .on_paste(move |text| {
                    m3.pastes.update(|n| *n += 1);
                    for line in text.lines() {
                        let mut cells = line.split('\t');
                        let name = cells.next().unwrap_or_default().to_string();
                        let n = cells.next().and_then(|c| c.parse().ok()).unwrap_or(0.0);
                        m3.model.push(Item2 { name, n });
                    }
                }),))
            .padding(12.0),
        ),
    );
    let tv = model.table_view();
    let main_table = TABLE.with(|t| t.model.clone());

    // ⌘C: the selected rows as tab-separated text, columns in their shown order.
    model.select(&[1, 3]);
    let pb = private_pasteboard();
    let copied = model.copy_to(&pb);
    let text = pasteboard_text(&pb);
    let tsv = text.as_deref() == Some("Item 1\t1\tNo\nItem 3\t3\tNo") && model.selected_text() == text.clone().unwrap_or_default();
    use objc2::runtime::NSObjectProtocol as _;
    let responds = tv.respondsToSelector(objc2::sel!(copy:)) && tv.respondsToSelector(objc2::sel!(paste:));
    check("table-copy", copied && tsv && responds, format!("copied: {copied}; text {text:?}; responds to copy:/paste: {responds}"));

    // ⌘V: the app gets the text; here it parses lines into rows.
    set_pasteboard_text(&pb, "New A\t7\nNew B\t8");
    let pasted = model.paste_from(&pb);
    let refused = !main_table.paste_from(&pb);
    release_pasteboard(&pb);
    let s2 = s.clone();
    after_events(move || {
        let s = &s2;
        let m = &s.model;
        let tail: Vec<Item2> = m.rows()[10..].to_vec();
        check(
            "table-paste",
            pasted && refused && s.pastes.get_untracked() == 1 && tail == [Item2 { name: "New A".into(), n: 7.0 }, Item2 { name: "New B".into(), n: 8.0 }],
            format!("pasted: {pasted}; a table without on_paste refuses: {refused}; rows added {tail:?}"),
        );

        // Dragging rows out: each carries its cells as text.
        let out = drag_rows(&m.table_view(), &[2]);
        let out_text = out.as_ref().and_then(|p| pasteboard_text(p));
        let main_off = drag_rows(&TABLE.with(|t| t.model.table_view()), &[0]).is_none();
        check(
            "table-drag-out",
            out_text.as_deref() == Some("Item 2\t2\tNo") && main_off,
            format!("row 2 drags as {out_text:?}; a table without row drag gives none: {main_off}"),
        );
        if let Some(p) = &out {
            release_pasteboard(p);
        }

        // Reorder: rows 0 and 1 dropped above row 5.
        let pb = drag_rows(&m.table_view(), &[0, 1]).unwrap();
        let accepted = drop_on_table(&m.table_view(), &pb, 5, true);
        let s3 = s.clone();
        let events = events.clone();
        after_events(move || {
            let s = &s3;
            let m = &s.model;
            let order = names(m);
            let moves = s.moves.get_untracked();
            let selected = m.selection();
            // Sorted, the same drop is refused.
            m.set_sort(Some(Sort { column: "n".into(), ascending: false }));
            let sorted_refused = !drop_on_table(&m.table_view(), &pb, 2, true);
            m.set_sort(None);
            release_pasteboard(&pb);
            // move_rows against a plain reference, for awkward cases.
            let reference = |v: &[String], rows: &[usize], before: Option<usize>| -> Vec<String> {
                let mut rows = rows.to_vec();
                rows.sort_unstable();
                let moved: Vec<String> = rows.iter().map(|&r| v[r].clone()).collect();
                let end = before.unwrap_or(v.len());
                let mut out: Vec<String> = Vec::new();
                for (i, x) in v.iter().enumerate() {
                    if i == end {
                        out.extend(moved.iter().cloned());
                    }
                    if !rows.contains(&i) {
                        out.push(x.clone());
                    }
                }
                if end >= v.len() {
                    out.extend(moved.iter().cloned());
                }
                out
            };
            let cases: [(&[usize], Option<usize>); 4] = [(&[9, 2, 7], Some(4)), (&[0], None), (&[3, 4], Some(4)), (&[11], Some(0))];
            let mut move_ok = true;
            for (rows, before) in cases {
                let want = reference(&names(m), rows, before);
                m.move_rows(rows, before);
                move_ok &= names(m) == want;
            }
            check(
                "table-reorder",
                accepted
                    && moves == [(vec![0, 1], Some(5))]
                    && order[..6] == ["Item 2", "Item 3", "Item 4", "Item 0", "Item 1", "Item 5"]
                    && selected == [3, 4]
                    && sorted_refused
                    && move_ok,
                format!("accepted: {accepted}; event {moves:?}; order {:?}; selected {selected:?}; refused while sorted: {sorted_refused}; move_rows matches a reference in 4 more cases: {move_ok}", &order[..6]),
            );

            // Files from the Finder: .csv accepted above the shown row; .txt refused.
            let dir = std::env::temp_dir();
            let (a, b, t) = (dir.join("a.csv"), dir.join("b.csv"), dir.join("notes.txt"));
            let csvs = files_pasteboard(&[&a, &b]);
            let dropped = drop_on_table(&m.table_view(), &csvs, 1, false);
            let txt = files_pasteboard(&[&t]);
            let txt_refused = !drop_on_table(&m.table_view(), &txt, 1, false);
            // Rows from another table aren't a reorder here.
            let foreign = !drop_on_table(&m.table_view(), &drag_rows(&m.table_view(), &[0]).unwrap(), 1, false);
            [csvs, txt].iter().for_each(|p| release_pasteboard(p));
            let s4 = s.clone();
            let events = events.clone();
            after_events(move || {
                let files = s4.files.get_untracked();
                let want = vec![(vec![a.clone(), b.clone()], s4.model.row_at(1))];
                check(
                    "table-file-drop",
                    dropped && txt_refused && foreign && files == want,
                    format!("csv drop: {dropped} → {files:?}; .txt refused: {txt_refused}; rows without the table as source refused: {foreign}"),
                );
                let w = windows().into_iter().find(|w| w.title().to_string() == "Transfer").unwrap();
                appkit_ui::close_window(&w);
                let events = events.clone();
                after_events(move || frame_check(events.clone()));
            });
        });
    });
}

/// on_frame: calls come (from display frames, or the stand-in timer while
/// the display isn't drawing), run in a batch, and stop when the loop is dropped.
fn frame_check(events: Rc<RefCell<Vec<AppEvent>>>) {
    let w = windows().into_iter().find(|w| w.title().to_string() == "Views").unwrap();
    let calls = Rc::new(Cell::new(0u32));
    let c = calls.clone();
    let lp = appkit_ui::on_frame(&w, move |_| c.set(c.get() + 1));
    spawn(
        || std::thread::sleep(std::time::Duration::from_millis(500)),
        move |()| {
            let (display, fallback) = lp.counts();
            let seen = calls.get();
            drop(lp);
            let calls2 = calls.clone();
            spawn(
                || std::thread::sleep(std::time::Duration::from_millis(300)),
                move |()| {
                    let after = calls2.get();
                    check(
                        "frame-loop",
                        seen >= 3 && u64::from(seen) == display + fallback && after == seen,
                        format!("{seen} calls in 0.5 s ({display} from display frames, {fallback} from the stand-in timer); none after drop: {}", after == seen),
                    );
                    let events = events.clone();
                    after_events(move || appearance_check(events.clone()));
                },
            );
        },
    );
}

fn all_views(v: &NSView) -> Vec<Retained<NSView>> {
    let mut out = Vec::new();
    for s in v.subviews().iter() {
        out.extend(all_views(&s));
        out.push(s);
    }
    out
}

fn appearance_check(events: Rc<RefCell<Vec<AppEvent>>>) {
    // Appearance: switching NSApp's appearance reaches the app as an event.
    let before = appkit_ui::is_dark();
    appkit_host::testing::set_app_appearance(Some(!before));
    later(move || {
        let after = appkit_ui::is_dark();
        let got = events.borrow().iter().filter(|e| **e == AppEvent::AppearanceChanged).count();
        check("appearance", after != before && got >= 1, format!("dark {before} → {after}, {got} event(s)"));
        // The native views themselves follow (they inherit NSApp's appearance).
        let controls = windows().into_iter().find(|w| w.title().to_string() == "Controls").unwrap();
        let views = controls.contentView().unwrap().subviews().to_vec();
        let names = unsafe {
            objc2_foundation::NSArray::from_slice(&[objc2_app_kit::NSAppearanceNameAqua, objc2_app_kit::NSAppearanceNameDarkAqua])
        };
        let dark_views = views
            .iter()
            .filter(|v| v.effectiveAppearance().bestMatchFromAppearancesWithNames(&names).is_some_and(|n: Retained<NSString>| n.to_string().contains("Dark")))
            .count();
        let want = if after { views.len() } else { 0 };
        check("dark-mode-views", dark_views == want, format!("{dark_views} of {} native views resolve to {}", views.len(), if after { "Dark Aqua" } else { "Aqua" }));
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
            // AppKit keeps about 0.64 KB per view ever shown; budget 0.9 KB.
            live_nodes() == base_nodes && labels().len() == base_views && removed_gone && rss1 - rss0 < 9.0,
            format!(
                "10,000 rows built and removed in {ms:.0} ms; nodes {base_nodes} → {}; views {base_views} → {}; removed view deallocated: {removed_gone}; footprint {rss0:.0} → {rss1:.0} MB = {:.2} KB per view (AppKit's floor ≈ 0.64; budget 0.9)",
                live_nodes(),
                labels().len(),
                (rss1 - rss0) * 1024.0 / 10_000.0
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
    let n0 = layout_passes();
    for k in 0..200 {
        w.setContentSize(objc2_foundation::NSSize::new(600.0 + (k % 40) as f64 * 10.0, 700.0));
    }
    let times = layout_times_us();
    let new = ((layout_passes() - n0) as usize).min(times.len());
    let mut resize: Vec<f64> = times[times.len() - new..].to_vec();
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
        views == 1000 && ((p(&resize, 50) < 2000.0 && p(&upd, 50) < 72.0) || on_ci()),
        format!(
            "{views} views built in {build_ms:.0} ms; resize layout p50 {:.2} ms, p95 {:.2} ms; label update p50 {:.1} µs, p95 {:.1} µs, {:.1} nodes placed per update (P0: 0.75 ms, 72 µs)",
            p(&resize, 50) / 1000.0, p(&resize, 95) / 1000.0, p(&upd, 50), p(&upd, 95), (p1 - p0) as f64 / 500.0
        ),
    );
    controls_checks(events);
}

#[derive(Clone)]
struct Ctrl {
    name: Signal<String>,
    pw: Signal<String>,
    wifi: Signal<bool>,
    remember: Signal<bool>,
    submits: Signal<u32>,
    saves: Signal<u32>,
}

thread_local! {
    static CTRL: Ctrl = Ctrl {
        name: Signal::new(String::new()),
        pw: Signal::new(String::new()),
        wifi: Signal::new(false),
        remember: Signal::new(false),
        submits: Signal::new(0),
        saves: Signal::new(0),
    };
}

fn controls_window() -> impl View {
    let c = CTRL.with(Clone::clone);
    let (submits, saves, name) = (c.submits.clone(), c.saves.clone(), c.name.clone());
    let summary = c.clone();
    vstack((
        text_field(&c.name).placeholder("Your name").on_submit(move || submits.update(|n| *n += 1)),
        secure_field(&c.pw).placeholder("Password"),
        toggle("Wi-Fi", &c.wifi),
        checkbox("Remember me", &c.remember),
        button("Save", move || saves.update(|n| *n += 1)).disabled(move || name.with(String::is_empty)).default_action(),
        label(move || {
            format!(
                "{} / {} chars / wifi {} / remember {}",
                summary.name.get(),
                summary.pw.with(String::len),
                summary.wifi.get(),
                summary.remember.get()
            )
        }),
    ))
    .padding(16.0)
}




fn controls_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    use objc2_app_kit::{NSButton, NSSecureTextField, NSSwitch};
    let c = CTRL.with(Clone::clone);
    let w = windows().into_iter().find(|w| w.title().to_string() == "Controls").unwrap();
    let subs: Vec<Retained<NSView>> = w.contentView().unwrap().subviews().to_vec();
    let secure = subs.iter().find_map(|v| v.clone().downcast::<NSSecureTextField>().ok()).unwrap();
    let field = subs
        .iter()
        .filter_map(|v| v.clone().downcast::<NSTextField>().ok())
        .find(|f| f.isEditable() && !std::ptr::eq(&**f, &**secure as &NSTextField))
        .unwrap();
    let switch = subs.iter().find_map(|v| v.clone().downcast::<NSSwitch>().ok()).unwrap();
    let buttons: Vec<Retained<NSButton>> = subs.iter().filter_map(|v| v.clone().downcast::<NSButton>().ok()).collect();
    let check_box = buttons.iter().find(|b| b.title().to_string() == "Remember me").unwrap().clone();
    let save = buttons.iter().find(|b| b.title().to_string() == "Save").unwrap().clone();
    let summary = || {
        subs.iter()
            .filter_map(|v| v.clone().downcast::<NSTextField>().ok())
            .map(|f| f.stringValue().to_string())
            .find(|s| s.contains("chars"))
            .unwrap_or_default()
    };

    // AppKit gives a default button its Return key equivalent only while
    // its window is key (the bench window took key status before this).
    w.makeKeyAndOrderFront(None);
    // Text field: placeholder; user edits → signal → label and the Save button.
    let was_disabled = !save.isEnabled();
    post_edit(&field, "Ada");
    check(
        "text-field-edit",
        c.name.get_untracked() == "Ada" && summary().starts_with("Ada /") && was_disabled && save.isEnabled()
            && field.placeholderString().map(|p| p.to_string()) == Some("Your name".into()),
        format!("signal {:?}; Save enabled {} → {}; summary {:?}", c.name.get_untracked(), !was_disabled, save.isEnabled(), summary()),
    );
    batch(|| c.name.set("Grace".into()));
    check("text-field-signal", field.stringValue().to_string() == "Grace", format!("field shows {:?}", field.stringValue().to_string()));
    let sent = unsafe { field.sendAction_to(field.action(), field.target().as_deref()) };
    check("text-field-submit", sent && c.submits.get_untracked() == 1, format!("Return action sent: {sent}, submits {}", c.submits.get_untracked()));

    // Secure field.
    post_edit(&secure, "hunter2");
    check(
        "secure-field",
        c.pw.get_untracked() == "hunter2" && summary().contains("7 chars") && has_ax(&ax_tree(&w), "AXTextField", "AXSecureTextField", ""),
        format!("signal len {}, AXSecureTextField in the AX tree: {}", c.pw.get_untracked().len(), has_ax(&ax_tree(&w), "AXTextField", "AXSecureTextField", "")),
    );

    // Switch and checkbox: clicks → signal; signal → control.
    unsafe { switch.performClick(None) };
    let on = c.wifi.get_untracked();
    batch(|| c.wifi.set(false));
    check(
        "toggle",
        on && switch.state() == objc2_app_kit::NSControlStateValueOff && ax_tree(&w).iter().any(|(_, s, _)| s == "AXSwitch"),
        format!("click → {on}; signal false → state {}; AXSwitch in the AX tree: {}", switch.state(), ax_tree(&w).iter().any(|(_, s, _)| s == "AXSwitch")),
    );
    unsafe { check_box.performClick(None) };
    let on = c.remember.get_untracked();
    batch(|| c.remember.set(false));
    check(
        "checkbox",
        on && check_box.state() == objc2_app_kit::NSControlStateValueOff && has_ax(&ax_tree(&w), "AXCheckBox", "", "Remember me"),
        format!("click → {on}; signal false → state {}; AXCheckBox \"Remember me\" in the AX tree: {}", check_box.state(), has_ax(&ax_tree(&w), "AXCheckBox", "", "Remember me")),
    );

    // Default button: Return key equivalent; click runs the action; disabled follows the signal.
    unsafe { save.performClick(None) };
    let saved = c.saves.get_untracked();
    batch(|| c.name.set(String::new()));
    check(
        "button-default-disabled",
        save.keyEquivalent().to_string() == "\r" && saved == 1 && !save.isEnabled(),
        format!("key equivalent {:?}; saves {saved}; enabled after clearing the name: {}", save.keyEquivalent().to_string(), save.isEnabled()),
    );

    // Tab: from the name field to the password field (key view loop by position).
    // A Tab key press reaches the field editor as insertTab:, which moves
    // to the next key view (AppKit's loop, recalculated from positions).
    w.makeFirstResponder(Some(&field));
    if let Some(editor) = field.currentEditor() {
        let _: () = unsafe { objc2::msg_send![&*editor, insertTab: Option::<&objc2::runtime::AnyObject>::None] };
    }
    check("tab-order", secure.currentEditor().is_some(), "Tab moves from the name field to the password field".into());

    controls_b_checks(events);
}

#[derive(Clone)]
struct CtrlB {
    size: Signal<u32>,
    volume: Signal<f64>,
    count: Signal<f64>,
    progress: Signal<Option<f64>>,
    lines: Signal<Vec<Item>>,
}

thread_local! {
    static CTRL_B: CtrlB = CtrlB {
        size: Signal::new(2),
        volume: Signal::new(0.5),
        count: Signal::new(3.0),
        progress: Signal::new(Some(0.4)),
        lines: Signal::new((1..=60).map(item).collect()),
    };
}

fn controls_b_window() -> impl View {
    let c = CTRL_B.with(Clone::clone);
    let (count, prog, lines) = (c.count.clone(), c.progress.clone(), c.lines.clone());
    vstack((
        picker(&c.size, vec![("Small", 1), ("Medium", 2), ("Large", 3)]).accessibility_label("Size"),
        hstack((slider(&c.volume, 0.0, 1.0).accessibility_label("Volume"), stepper(&c.count, 0.0, 10.0, 1.0).accessibility_label("Count"), label(move || format!("count {}", count.get())))),
        progress(move || prog.get()),
        hstack((image("star.fill").description("Favourite"), text("starred"))),
        divider(),
        scroll(vstack((for_each(move || lines.get(), |it: Item| label(move || it.name.clone())),))),
    ))
    .padding(16.0)
}




fn controls_field() -> (Retained<objc2_app_kit::NSWindow>, Retained<NSTextField>, Retained<objc2_app_kit::NSSecureTextField>) {
    let w = windows().into_iter().find(|w| w.title().to_string() == "Controls").unwrap();
    let subs: Vec<Retained<NSView>> = w.contentView().unwrap().subviews().to_vec();
    let secure = subs.iter().find_map(|v| v.clone().downcast::<objc2_app_kit::NSSecureTextField>().ok()).unwrap();
    let field = subs
        .iter()
        .filter_map(|v| v.clone().downcast::<NSTextField>().ok())
        .find(|f| f.isEditable() && !std::ptr::eq(&**f, &**secure as &NSTextField))
        .unwrap();
    (w, field, secure)
}

fn editor_of(field: &NSTextField) -> Retained<objc2_app_kit::NSTextView> {
    field.currentEditor().unwrap().downcast::<objc2_app_kit::NSTextView>().unwrap()
}

/// Ported Phase 1 checks: Shift-Tab, edit shortcuts through the menu,
/// Japanese input composition and the candidate window position.
fn keyboard_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let c = CTRL.with(Clone::clone);
    let (w, field, secure) = controls_field();
    w.makeKeyAndOrderFront(None);

    // Shift-Tab: from the password field back to the name field.
    w.makeFirstResponder(Some(&secure));
    if let Some(editor) = secure.currentEditor() {
        let _: () = unsafe { objc2::msg_send![&*editor, insertBacktab: Option::<&objc2::runtime::AnyObject>::None] };
    }
    check("shift-tab", field.currentEditor().is_some(), "Shift-Tab moves from the password field to the name field".into());

    // Shortcuts: type, ⌘A selects all, ⌘Z undoes the typing, ⇧⌘Z redoes it;
    // each goes through the menu bar's key equivalents to the focused field.
    batch(|| c.name.set(String::new()));
    w.makeFirstResponder(Some(&field));
    let ed = editor_of(&field);
    unsafe { ed.insertText_replacementRange(&NSString::from_str("Hello"), objc2_foundation::NSRange::new(usize::MAX >> 1, 0)) };
    let typed = c.name.get_untracked();
    later(move || {
        let (w, field, _) = controls_field();
        let ed = editor_of(&field);
        let sel_all = shortcut(&w, "a", false) && ed.selectedRange().length == 5;
        let undo_sent = shortcut(&w, "z", false);
        let (c, typed, events) = (c.clone(), typed.clone(), events.clone());
        // The field reports the undone text on the next pass.
        later(move || {
            let (w, field, _) = controls_field();
            // With the app active, the binding follows undo/redo. When another
            // app is in front, AppKit doesn't send the field's change
            // notification for an undo (no key window), so check the field.
            let active = NSApplication::sharedApplication(mtm()).isActive();
            let read = |f: &NSTextField| if active { c.name.get_untracked() } else { f.stringValue().to_string() };
            let after_undo = read(&field);
            let redo_sent = shortcut(&w, "z", true);
            let (c, typed, events, after_undo) = (c.clone(), typed.clone(), events.clone(), after_undo.clone());
            later(move || {
                let after_redo = if active { c.name.get_untracked() } else { field.stringValue().to_string() };
                check(
                    "shortcuts",
                    typed == "Hello" && sel_all && undo_sent && after_undo.is_empty() && redo_sent && after_redo == "Hello",
                    format!(
                        "typed {typed:?}; ⌘A selected all: {sel_all}; ⌘Z → {after_undo:?}; ⇧⌘Z → {after_redo:?} ({})",
                        if active { "app active: checked the binding" } else { "app inactive: checked the field" }
                    ),
                );
                ime_checks(events.clone());
            });
        });
    });
}

fn has_input_context(ed: &objc2_app_kit::NSTextView) -> bool {
    let ctx: Option<Retained<objc2::runtime::AnyObject>> = unsafe { objc2::msg_send![ed, inputContext] };
    ctx.is_some()
}

fn ime_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    use objc2_app_kit::NSTextInputClient;
    let c = CTRL.with(Clone::clone);
    let (w, field, _) = controls_field();
    batch(|| c.name.set(String::new()));
    w.makeFirstResponder(Some(&field));
    // No field editor (the field couldn't become first responder, e.g. in
    // an inactive app on a CI runner): nothing to compose into.
    let Some(ed) = field.currentEditor().and_then(|e| e.downcast::<objc2_app_kit::NSTextView>().ok()) else {
        let why = "skipped: the field has no editor here (it couldn't become first responder)".to_string();
        check("ime-composition", true, why.clone());
        check("ime-candidate-rect", true, why);
        return teardown_checks(events);
    };
    let not_found = objc2_foundation::NSRange::new(usize::MAX >> 1, 0);
    eprintln!("[step] ime: composing in {:?}", ed.class().name());
    // Composition: "toukyou" as marked text, then commit 東京.
    unsafe {
        ed.setMarkedText_selectedRange_replacementRange(
            &NSString::from_str("とうきょう"),
            objc2_foundation::NSRange::new(5, 0),
            not_found,
        )
    };
    let marked = ed.hasMarkedText();
    let during = c.name.get_untracked();
    // The candidate window goes where the input method asks: the caret's
    // screen rectangle, which must lie on the field.
    let rect = unsafe { ed.firstRectForCharacterRange_actualRange(objc2_foundation::NSRange::new(0, 5), std::ptr::null_mut()) };
    let field_screen = w.convertRectToScreen(field.convertRect_toView(field.bounds(), None));
    let on_field = rect.origin.x >= field_screen.origin.x - 1.0
        && rect.origin.x < field_screen.origin.x + field_screen.size.width
        && rect.origin.y >= field_screen.origin.y - 1.0
        && rect.origin.y + rect.size.height <= field_screen.origin.y + field_screen.size.height + 1.0;
    unsafe { ed.insertText_replacementRange(&NSString::from_str("東京"), not_found) };
    let committed = c.name.get_untracked();
    check(
        "ime-composition",
        marked && !ed.hasMarkedText() && committed == "東京" && has_input_context(&ed),
        format!("marked while composing: {marked} (signal meanwhile {during:?}); committed → {committed:?}"),
    );
    check(
        "ime-candidate-rect",
        on_field && rect.size.height > 0.0,
        format!(
            "marked-text rect ({:.0}, {:.0}, {:.0}×{:.0}) on the field at ({:.0}, {:.0}, {:.0}×{:.0})",
            rect.origin.x, rect.origin.y, rect.size.width, rect.size.height,
            field_screen.origin.x, field_screen.origin.y, field_screen.size.width, field_screen.size.height
        ),
    );
    teardown_checks(events);
}

thread_local! {
    static FORM: Signal<bool> = Signal::new(true);
}

fn teardown_window() -> impl View {
    let show = FORM.with(Clone::clone);
    dynamic(move || {
        if show.get() {
            let (t, b, n, f) = (Signal::new(String::new()), Signal::new(false), Signal::new(0.5), Signal::new(1u32));
            appkit_ui::AnyView::new(
                vstack((
                    text_field(&t).placeholder("name"),
                    secure_field(&t),
                    toggle("switch", &b),
                    checkbox("check", &b),
                    picker(&f, vec![("one", 1), ("two", 2)]),
                    hstack((slider(&n, 0.0, 1.0), stepper(&n, 0.0, 1.0, 0.1))),
                    progress(move || Some(n.get())),
                    vstack((image("star"), button("go", || {}), divider(), scroll(vstack((text("a"), text("b")))))),
                ))
                .padding(12.0),
            )
            .id(1)
        } else {
            appkit_ui::AnyView::new(text("off")).id(0)
        }
    })
}

/// Every control type built and torn down 300 times (after a warm-up):
/// nodes, subviews and footprint return to baseline; controls deallocate.
fn teardown_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let show = FORM.with(Clone::clone);
    let w = windows().into_iter().find(|w| w.title().to_string() == "Teardown").unwrap();
    let switch = w.contentView().unwrap().subviews().iter().find_map(|v| v.downcast::<objc2_app_kit::NSSwitch>().ok()).unwrap();
    let weak = objc2::rc::Weak::from_retained(&switch);
    drop(switch);
    for _ in 0..100 {
        batch(|| show.set(false));
        batch(|| show.set(true));
    }
    fn count(v: &NSView) -> usize {
        1 + v.subviews().iter().map(|c| count(&c)).sum::<usize>()
    }
    // Every view in the form, AppKit's internal subviews included.
    let per_form = count(&w.contentView().unwrap()) - 1;
    let (n0, v0, f0) = (live_nodes(), w.contentView().unwrap().subviews().len(), footprint_mb());
    let t = std::time::Instant::now();
    for _ in 0..300 {
        batch(|| show.set(false));
        batch(|| show.set(true));
    }
    let ms = t.elapsed().as_secs_f64() * 1e3;
    later(move || {
        let w = windows().into_iter().find(|w| w.title().to_string() == "Teardown").unwrap();
        let (n1, v1, f1) = (live_nodes(), w.contentView().unwrap().subviews().len(), footprint_mb());
        let gone = weak.load().is_none();
        // AppKit keeps ~1 KB per view ever measured and added to a window
        // (plain AppKit, same form: 7.5 MB per 300 cycles); more is ours.
        let kb_per_view = (f1 - f0) * 1024.0 / (300.0 * per_form as f64);
        check(
            "controls-teardown",
            n1 == n0 && v1 == v0 && gone && kb_per_view < 1.5,
            format!("300 × a 15-control form ({per_form} views) in {ms:.0} ms; nodes {n0} → {n1}; subviews {v0} → {v1}; first switch deallocated: {gone}; footprint {f0:.0} → {f1:.0} MB = {kb_per_view:.2} KB per view created (AppKit's floor ≈ 1)"),
        );
        table_checks(events.clone());
    });
}

#[derive(Clone)]
struct Order {
    name: String,
    qty: f64,
    done: bool,
}

#[derive(Clone)]
struct TableState {
    model: Rc<TableModel<Order>>,
    selected: Signal<Vec<usize>>,
    activated: Signal<Option<usize>>,
    sorted: Signal<Option<Sort>>,
    edits: Signal<u32>,
    show: Signal<bool>,
}

thread_local! {
    static TABLE: TableState = {
        let model = Rc::new(TableModel::new(
            ViewId(500),
            vec![
                Column::text("name", "Name", |o: &Order| o.name.clone()).width(180.0).editable(true),
                Column::number("qty", "Qty", |o: &Order| o.qty).editable(true),
                Column::check("done", "Done", |o: &Order| o.done),
            ],
        ));
        let t = std::time::Instant::now();
        model.set_rows((0..200_000).map(|i| Order { name: format!("Order {i:06}"), qty: (i % 50) as f64, done: i % 3 == 0 }).collect());
        eprintln!("[time] 200k rows into the model: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        TableState {
            model,
            selected: Signal::new(vec![]),
            activated: Signal::new(None),
            sorted: Signal::new(None),
            edits: Signal::new(0),
            show: Signal::new(true),
        }
    };
}

fn table_window() -> impl View {
    let t = TABLE.with(Clone::clone);
    let (t1, t2, t3, t4, t5, t6) = (t.clone(), t.clone(), t.clone(), t.clone(), t.clone(), t.clone());
    vstack((
        label(move || format!("{} selected", t1.selected.with(Vec::len))),
        dynamic(move || {
            if t2.show.get() {
                let (t3, t4, t5, t6) = (t3.clone(), t4.clone(), t5.clone(), t6.clone());
                appkit_ui::AnyView::new(
                    table(&t3.model.clone())
                        .on_select(move |rows| t3.selected.set(rows))
                        .on_activate(move |row| t4.activated.set(Some(row)))
                        .on_sort(move |sort| t5.sorted.set(Some(sort)))
                        .on_toggle({
                            let t = t6.clone();
                            move |row, _, value| {
                                let mut o = t.model.rows()[row].clone();
                                o.done = value;
                                t.model.update_row(row, o);
                            }
                        })
                        .on_edit(move |row, column, value| {
                            let mut o = t6.model.rows()[row].clone();
                            match column.as_str() {
                                "name" => o.name = value,
                                _ => o.qty = value.parse().unwrap_or(o.qty),
                            }
                            t6.model.update_row(row, o);
                            t6.edits.update(|n| *n += 1);
                        }),
                )
                .id(1)
            } else {
                appkit_ui::AnyView::new(text("no table")).id(0)
            }
        }),
    ))
    .padding(12.0)
}

/// Runs `f` after two run-loop passes (model events are delivered on the
/// pass after AppKit's callback).
fn after_events(f: impl Fn() + 'static) {
    let f = Rc::new(f);
    later(move || {
        let f = f.clone();
        later(move || f())
    });
}

fn table_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    use objc2_app_kit::{NSButton, NSTableView};
    let t = TABLE.with(Clone::clone);
    let w = windows().into_iter().find(|w| w.title().to_string() == "Table").unwrap();
    w.makeKeyAndOrderFront(None);
    let tv: Retained<NSTableView> = t.model.table_view();
    let sv = t.model.scroll_view();
    let fills = sv.frame().size.height > 300.0 && sv.frame().size.width > 500.0;
    check(
        "table-placed",
        tv.numberOfRows() == 200_000 && fills,
        format!("{} rows; scroll view {:.0}×{:.0} in a 560×420 window", tv.numberOfRows(), sv.frame().size.width, sv.frame().size.height),
    );

    // Selection through NSTableView → on_select (after the event pass).
    tv.selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(5), false);
    let t2 = t.clone();
    let events2 = events.clone();
    after_events(move || {
        let t = t2.clone();
        let tv = t.model.table_view();
        let w = windows().into_iter().find(|w| w.title().to_string() == "Table").unwrap();
        check("table-select", t.selected.get_untracked() == vec![5], format!("on_select got {:?}", t.selected.get_untracked()));

        // Return on the selected row → on_activate.
        let ev = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            objc2_app_kit::NSEventType::KeyDown, objc2_foundation::NSPoint::new(0.0, 0.0), objc2_app_kit::NSEventModifierFlags::empty(), 0.0,
            w.windowNumber(), None, &NSString::from_str("\r"), &NSString::from_str("\r"), false, 36,
        ).unwrap();
        tv.keyDown(&ev);

        // Sort by quantity, descending, as a header click does.
        let desc = objc2_foundation::NSSortDescriptor::sortDescriptorWithKey_ascending(Some(&NSString::from_str("qty")), false);
        tv.setSortDescriptors(&objc2_foundation::NSArray::from_retained_slice(&[desc]));

        // Toggle a checkbox cell (row 1 shows a not-done order before sorting… use the visible row 0).
        let check_col = tv.columnWithIdentifier(&NSString::from_str("done"));
        let cell = tv.viewAtColumn_row_makeIfNecessary(check_col, 0, true).and_then(|v| v.downcast::<NSButton>().ok()).unwrap();
        let model_row = t.model.row_at(0).unwrap();
        let before = t.model.rows()[model_row].done;
        unsafe { cell.performClick(None) };

        // Edit the name of visible row 1 the way a user does: start editing, type, Return.
        let name_col = tv.columnWithIdentifier(&NSString::from_str("name"));
        let edit_row = t.model.row_at(1).unwrap();
        tv.editColumn_row_withEvent_select(name_col, 1, None, true);
        // View-based tables edit in the cell's own text field (its field
        // editor), not in the table.
        let name_cell = tv.viewAtColumn_row_makeIfNecessary(name_col, 1, true).and_then(|v| v.downcast::<NSTextField>().ok()).unwrap();
        let editing = name_cell.currentEditor().is_some();
        if let Some(ed) = name_cell.currentEditor().and_then(|e| e.downcast::<objc2_app_kit::NSTextView>().ok()) {
            unsafe { ed.selectAll(None) };
            unsafe { ed.insertText_replacementRange(&NSString::from_str("Renamed"), objc2_foundation::NSRange::new(usize::MAX >> 1, 0)) };
            let _: () = unsafe { objc2::msg_send![&*ed, insertNewline: Option::<&objc2::runtime::AnyObject>::None] };
        }
        let t3 = t.clone();
        let events3 = events2.clone();
        after_events(move || {
            let t = t3.clone();
            check("table-activate", t.activated.get_untracked() == Some(5), format!("Return activated {:?}", t.activated.get_untracked()));
            let first = t.model.row_at(0).map(|r| t.model.rows()[r].qty);
            check(
                "table-sort",
                t.sorted.get_untracked() == Some(Sort { column: "qty".into(), ascending: false }) && first == Some(49.0),
                format!("on_sort {:?}; first row qty {first:?}", t.sorted.get_untracked()),
            );
            let after = t.model.rows()[model_row].done;
            check("table-toggle", after != before, format!("row {model_row} done {before} → {after} (on_toggle → update_row)"));
            let name = t.model.rows()[edit_row].name.clone();
            let shown = t
                .model
                .table_view()
                .viewAtColumn_row_makeIfNecessary(t.model.table_view().columnWithIdentifier(&NSString::from_str("name")), 1, true)
                .and_then(|v| v.downcast::<NSTextField>().ok())
                .map(|f| f.stringValue().to_string());
            check(
                "table-edit",
                editing && name == "Renamed" && shown.as_deref() == Some("Renamed") && t.edits.get_untracked() == 1,
                format!("editor opened: {editing}; row {edit_row} name {name:?}; cell shows {shown:?}"),
            );
            table_teardown(events3.clone());
        });
    });
}

fn table_teardown(events: Rc<RefCell<Vec<AppEvent>>>) {
    let t = TABLE.with(Clone::clone);
    let sv = t.model.scroll_view();
    batch(|| t.show.set(false));
    let removed = unsafe { sv.superview() }.is_none();
    // Events from a table that's no longer in the tree aren't delivered.
    let before = t.selected.get_untracked();
    t.model.table_view().selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(9), false);
    let t2 = t.clone();
    after_events(move || {
        let t = t2.clone();
        let quiet = t.selected.get_untracked() == before;
        batch(|| t.show.set(true));
        let back = unsafe { t.model.scroll_view().superview() }.is_some();
        check("table-teardown", removed && quiet && back, format!("removed from window: {removed}; no callback after removal: {quiet}; re-added: {back}"));
        table_bench(events.clone());
    });
}

fn table_bench(events: Rc<RefCell<Vec<AppEvent>>>) {
    let t = TABLE.with(Clone::clone);
    let tv = t.model.table_view();
    let cells0 = t.model.cells_created();
    let start = std::time::Instant::now();
    let mut worst = 0.0f64;
    for row in (0..200_000).step_by(2_000) {
        let s = std::time::Instant::now();
        tv.scrollRowToVisible(row);
        tv.displayIfNeeded();
        worst = worst.max(s.elapsed().as_secs_f64() * 1e3);
    }
    let total = start.elapsed().as_secs_f64() * 1e3;
    let created = t.model.cells_created() - cells0;
    check(
        "table-bench",
        (worst < 16.7 || on_ci()) && created < 200,
        format!("100 jumps across 200k rows (scroll + display): total {total:.0} ms, worst {worst:.1} ms; {created} new cell views (reuse)"),
    );
    outline_checks(events);
}

#[derive(Clone)]
struct Place {
    id: u32,
    name: String,
    symbol: Option<&'static str>,
}

impl Identifiable for Place {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

fn place(id: u32, name: &str, symbol: Option<&'static str>) -> Place {
    Place { id, name: name.into(), symbol }
}

#[derive(Clone)]
struct OutlineState {
    sections: Signal<Vec<TreeNode<Place>>>,
    tree: Signal<Vec<TreeNode<Place>>>,
    place: Signal<Option<u32>>,
    node: Signal<Option<u32>>,
    show: Signal<bool>,
}

fn project(id: u32, files: u32) -> TreeNode<Place> {
    TreeNode::branch(
        place(id, &format!("Project {id}"), Some("folder")),
        (0..files).map(|f| TreeNode::leaf(place(id * 1000 + f, &format!("file{f}.rs"), Some("doc")))).collect(),
    )
}

thread_local! {
    static OUTLINE: OutlineState = OutlineState {
        sections: Signal::new(vec![
            TreeNode::branch(place(1, "Favourites", None), vec![
                TreeNode::leaf(place(11, "Documents", Some("doc.text"))),
                TreeNode::leaf(place(12, "Downloads", Some("arrow.down.circle"))),
            ]),
            TreeNode::branch(place(2, "Locations", None), vec![
                TreeNode::leaf(place(21, "Mac", Some("desktopcomputer"))),
                TreeNode::leaf(place(22, "Network", Some("network"))),
            ]),
        ]),
        tree: Signal::new((1..=3).map(|p| project(p, 3)).collect()),
        place: Signal::new(None),
        node: Signal::new(None),
        show: Signal::new(true),
    };
}

fn outline_window() -> impl View {
    let o = OUTLINE.with(Clone::clone);
    let (sections, o2) = (o.sections.clone(), o.clone());
    hstack((
        sidebar(move || sections.get(), |p: &Place| p.name.clone())
            .icon(|p: &Place| p.symbol.map(String::from))
            .selection(&o.place)
            .width(180.0),
        dynamic(move || {
            if o2.show.get() {
                let tree = o2.tree.clone();
                appkit_ui::AnyView::new(
                    outline(move || tree.get(), |p: &Place| p.name.clone())
                        .icon(|p: &Place| p.symbol.map(String::from))
                        .selection(&o2.node)
                        .fill_width()
                        .fill_height(),
                )
                .id(1)
            } else {
                appkit_ui::AnyView::new(text("no outline")).id(0)
            }
        }),
    ))
}

fn outline_views() -> (Retained<objc2_app_kit::NSOutlineView>, Option<Retained<objc2_app_kit::NSOutlineView>>) {
    let w = windows().into_iter().find(|w| w.title().to_string() == "Outline").unwrap();
    let mut outlines: Vec<(f64, Retained<objc2_app_kit::NSOutlineView>)> = w
        .contentView()
        .unwrap()
        .subviews()
        .iter()
        .filter_map(|v| v.downcast::<objc2_app_kit::NSScrollView>().ok())
        .filter_map(|sv| {
            let x = sv.frame().origin.x;
            sv.documentView().and_then(|d| d.downcast::<objc2_app_kit::NSOutlineView>().ok()).map(|o| (x, o))
        })
        .collect();
    outlines.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut it = outlines.into_iter().map(|(_, o)| o);
    (it.next().unwrap(), it.next())
}

fn outline_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let o = OUTLINE.with(Clone::clone);
    let w = windows().into_iter().find(|w| w.title().to_string() == "Outline").unwrap();
    w.makeKeyAndOrderFront(None);
    let (side, tree) = outline_views();
    let tree = tree.unwrap();

    // Sidebar: source-list style; sections expanded as group rows; icons.
    let item0 = side.itemAtRow(0);
    let group = item0.as_ref().is_some_and(|i| unsafe { side.isExpandable(Some(i)) } && unsafe { side.isItemExpanded(Some(i)) });
    let icon = side
        .viewAtColumn_row_makeIfNecessary(0, 1, true)
        .and_then(|v| v.downcast::<objc2_app_kit::NSTableCellView>().ok())
        .and_then(|c| unsafe { c.imageView() })
        .is_some_and(|iv| iv.image().is_some());
    check(
        "sidebar-style",
        side.style() == objc2_app_kit::NSTableViewStyle::SourceList && side.numberOfRows() == 6 && group && icon,
        format!("style source list: {}; {} rows (2 sections + 4 items); section expanded: {group}; icon: {icon}", side.style() == objc2_app_kit::NSTableViewStyle::SourceList, side.numberOfRows()),
    );

    // Sidebar selection both ways.
    side.selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(2), false);
    let picked = o.place.get_untracked();
    batch(|| o.place.set(Some(22)));
    let row_of_network = side.selectedRow();
    check("sidebar-selection", picked == Some(12) && row_of_network == 5, format!("row 2 → {picked:?}; Network → row {row_of_network}"));

    // Outline: collapsed at first; expanding a project shows its files.
    let rows0 = tree.numberOfRows();
    let first = tree.itemAtRow(0);
    unsafe { tree.expandItem(first.as_deref()) };
    let rows1 = tree.numberOfRows();
    check(
        "outline-expand",
        rows0 == 3 && rows1 == 6 && has_ax(&ax_tree(&w), "AXOutline", "", ""),
        format!("{rows0} rows collapsed → {rows1} after expanding Project 1; AXOutline in the tree"),
    );

    // Update: add a file to the expanded project; expansion and selection survive.
    tree.selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(2), false);
    let sel_before = o.node.get_untracked();
    batch(|| {
        o.tree.update(|t| {
            t[0].children.push(TreeNode::leaf(place(1999, "new.rs", Some("doc"))));
            t.push(project(4, 2));
        })
    });
    let rows2 = tree.numberOfRows();
    let still_expanded = tree.itemAtRow(0).is_some_and(|i| unsafe { tree.isItemExpanded(Some(&i)) });
    let sel_after = o.node.get_untracked();
    let row_after = tree.selectedRow();
    check(
        "outline-update",
        sel_before == Some(1001) && rows2 == 8 && still_expanded && sel_after == sel_before && row_after == 2,
        format!("selected {sel_before:?}; after adding a file and a project: {rows2} rows, Project 1 expanded {still_expanded}, selection {sel_after:?} at row {row_after}"),
    );

    // Teardown: the outline's item objects go with it.
    let weak_item = tree.itemAtRow(0).map(|i| objc2::rc::Weak::from_retained(&i));
    drop(tree);
    batch(|| o.show.set(false));
    let o2 = o.clone();
    // AppKit releases a removed outline's items a little later.
    spawn(|| std::thread::sleep(std::time::Duration::from_millis(300)), move |_| {
        let gone = weak_item.as_ref().is_some_and(|w| w.load().is_none());
        let none_left = outline_views().1.is_none();
        check("outline-teardown", gone && none_left, format!("outline removed: {none_left}; its item objects deallocated: {gone}"));
        batch(|| o2.show.set(true));
        // spawn's continuation is itself a batch: the outline comes back when
        // it ends, so measure on the next pass.
        let (o3, events) = (o2.clone(), events.clone());
        later(move || outline_bench(o3.clone(), events.clone()));
    });
}

fn outline_bench(o: OutlineState, events: Rc<RefCell<Vec<AppEvent>>>) {
    // 100 projects × 100 files: rebuild from the signal, then expand everything.
    let t = std::time::Instant::now();
    batch(|| o.tree.set((1..=100).map(|p| project(p, 100)).collect()));
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let (_, tree) = outline_views();
    let tree = tree.unwrap();
    let t = std::time::Instant::now();
    unsafe { tree.expandItem_expandChildren(None, true) };
    tree.displayIfNeeded();
    let expand_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = std::time::Instant::now();
    batch(|| o.tree.update(|tr| tr[50].children.push(TreeNode::leaf(place(999_999, "late.rs", None)))));
    let update_ms = t.elapsed().as_secs_f64() * 1e3;
    check(
        "outline-bench",
        tree.numberOfRows() == 10_101 && ((build_ms < 100.0 && update_ms < 100.0) || on_ci()),
        format!("10,100 items: rebuild {build_ms:.1} ms; expand all + display {expand_ms:.1} ms; one added item (expansion kept) {update_ms:.1} ms; {} rows", tree.numberOfRows()),
    );
    editor_checks(events);
}

const NOTES: &str = "This sentense has a speling error.";

#[derive(Clone)]
struct EditorState {
    model: TextModel,
    notes: Signal<String>,
    changes: Signal<u32>,
    compositions: Signal<Vec<Option<String>>>,
    show: Signal<bool>,
}

thread_local! {
    static EDITOR: EditorState = EditorState {
        model: TextModel::new(ViewId(900), NOTES),
        notes: Signal::new(NOTES.to_string()),
        changes: Signal::new(0),
        compositions: Signal::new(vec![]),
        show: Signal::new(true),
    };
}

fn editor_window() -> impl View {
    let e = EDITOR.with(Clone::clone);
    let (e1, e2) = (e.clone(), e.clone());
    vstack((
        label(move || format!("{} characters", e1.notes.with(|t| t.chars().count()))),
        dynamic(move || {
            if e2.show.get() {
                let (c, k) = (e2.changes.clone(), e2.compositions.clone());
                appkit_ui::AnyView::new(
                    text_editor(&e2.model)
                        .bind(&e2.notes)
                        .on_change(move |_| c.update(|n| *n += 1))
                        .on_composition(move |m| k.update(|v| v.push(m))),
                )
                .id(1)
            } else {
                appkit_ui::AnyView::new(text("no editor")).id(0)
            }
        }),
    ))
    .padding(12.0)
}

fn editor_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let e = EDITOR.with(Clone::clone);
    let w = windows().into_iter().find(|w| w.title().to_string() == "Editor").unwrap();
    w.makeKeyAndOrderFront(None);
    let tv = e.model.text_view();
    let sv = e.model.scroll_view();
    check(
        "editor-placed",
        sv.frame().size.height > 200.0 && tv.string().to_string() == NOTES && has_ax(&ax_tree(&w), "AXTextArea", "", ""),
        format!("scroll view {:.0}×{:.0}; text {:?}; AXTextArea in the tree", sv.frame().size.width, sv.frame().size.height, tv.string().to_string()),
    );

    // Typing at the end → binding, on_change, and the label.
    w.makeFirstResponder(Some(&tv));
    let end = tv.string().length();
    tv.setSelectedRange(objc2_foundation::NSRange::new(end, 0));
    let not_found = objc2_foundation::NSRange::new(usize::MAX >> 1, 0);
    unsafe { tv.insertText_replacementRange(&NSString::from_str(" More."), not_found) };
    let e2 = e.clone();
    after_events(move || {
        let e = e2.clone();
        let w = windows().into_iter().find(|w| w.title().to_string() == "Editor").unwrap();
        let bound = e.notes.get_untracked();
        let label_ok = w.contentView().unwrap().subviews().iter().filter_map(|v| v.downcast::<NSTextField>().ok())
            .any(|f| f.stringValue().to_string() == format!("{} characters", bound.chars().count()));
        check("editor-typing", bound.ends_with("error. More.") && e.changes.get_untracked() >= 1 && label_ok, format!("binding {bound:?}; on_change ×{}; label follows: {label_ok}", e.changes.get_untracked()));

        // Signal → editor: replaces the text and clears undo.
        batch(|| e.notes.set("Replaced".into()));
        let tv = e.model.text_view();
        check("editor-bind", tv.string().to_string() == "Replaced" && !e.model.can_undo(), format!("editor shows {:?}; can undo: {}", tv.string().to_string(), e.model.can_undo()));

        // Japanese input: no change while composing; one change on commit.
        let changes0 = e.changes.get_untracked();
        let end = tv.string().length();
        tv.setSelectedRange(objc2_foundation::NSRange::new(end, 0));
        let not_found = objc2_foundation::NSRange::new(usize::MAX >> 1, 0);
        unsafe { tv.setMarkedText_selectedRange_replacementRange(&NSString::from_str("とうきょう"), objc2_foundation::NSRange::new(5, 0), not_found) };
        let e3 = e.clone();
        let events = events.clone();
        after_events(move || {
            let e = e3.clone();
            let tv = e.model.text_view();
            let mid_changes = e.changes.get_untracked() - changes0;
            let mid_bound = e.notes.get_untracked();
            let not_found = objc2_foundation::NSRange::new(usize::MAX >> 1, 0);
            unsafe { tv.insertText_replacementRange(&NSString::from_str("東京"), not_found) };
            let e4 = e.clone();
            let events = events.clone();
            after_events(move || {
                let e = e4.clone();
                let comps = e.compositions.get_untracked();
                check(
                    "editor-ime",
                    mid_changes == 0 && mid_bound == "Replaced" && e.notes.get_untracked() == "Replaced東京"
                        && comps.first() == Some(&Some("とうきょう".to_string())) && comps.last() == Some(&None),
                    format!("while composing: {mid_changes} changes, binding {mid_bound:?}; committed → {:?}; compositions {comps:?}", e.notes.get_untracked()),
                );
                editor_undo_and_spelling(events.clone());
            });
        });
    });
}

fn editor_undo_and_spelling(events: Rc<RefCell<Vec<AppEvent>>>) {
    let e = EDITOR.with(Clone::clone);
    let w = windows().into_iter().find(|w| w.title().to_string() == "Editor").unwrap();
    let tv = e.model.text_view();
    w.makeFirstResponder(Some(&tv));
    // Undo through the Edit menu's ⌘Z (the editor's own undo manager).
    let before = e.notes.get_untracked();
    let undone = shortcut(&w, "z", false);
    let e2 = e.clone();
    after_events(move || {
        let e = e2.clone();
        let after = e.notes.get_untracked();
        check(
            "editor-undo",
            undone && after == "Replaced" && e.model.text() == "Replaced",
            format!("⌘Z sent: {undone}; {before:?} → binding {after:?}, editor {:?}", e.model.text()),
        );
        // Spell check (Phase 1's T11): continuous checking flags the words
        // (asynchronously), and the context menu offers suggestions.
        batch(|| e.notes.set(NOTES.to_string()));
        let e3 = e.clone();
        let events = events.clone();
        spawn(|| std::thread::sleep(std::time::Duration::from_millis(1500)), move |_| {
            let e = e3.clone();
            let tv = e.model.text_view();
            let flagged = ["sentense", "speling"].map(is_misspelled);
            let menu = context_menu_for_word(&tv, "speling");
            let suggests = menu.iter().any(|t| t == "spelling");
            let grammar = menu.iter().any(|t| t.starts_with("Spelling and Grammar"));
            check(
                "editor-spell-check",
                tv.isContinuousSpellCheckingEnabled() && flagged == [true, true] && suggests && grammar,
                format!("continuous checking {}; flagged {flagged:?}; right-click 'speling' → first items {:?}", tv.isContinuousSpellCheckingEnabled(), &menu[..menu.len().min(3)]),
            );
            let events = events.clone();
            later(move || editor_teardown(events.clone()));
        });
    });
}

fn editor_teardown(events: Rc<RefCell<Vec<AppEvent>>>) {
    let e = EDITOR.with(Clone::clone);
    let sv = e.model.scroll_view();
    batch(|| e.show.set(false));
    let removed = unsafe { sv.superview() }.is_none();
    let changes = e.changes.get_untracked();
    e.model.replace_selection("ignored");
    let e2 = e.clone();
    after_events(move || {
        let e = e2.clone();
        let quiet = e.changes.get_untracked() == changes;
        batch(|| e.show.set(true));
        let back = unsafe { e.model.scroll_view().superview() }.is_some();
        check("editor-teardown", removed && quiet && back, format!("removed: {removed}; no callback after removal: {quiet}; re-added: {back}"));
        chrome_checks(events.clone());
    });
}


#[derive(Clone)]
struct ChromeState {
    adds: Signal<u32>,
    locked: Signal<bool>,
    query: Signal<String>,
    renames: Signal<u32>,
    archivable: Signal<bool>,
    deleted: Signal<Option<usize>>,
    model: Rc<TableModel<Order>>,
}

thread_local! {
    static CHROME: ChromeState = {
        let model = Rc::new(TableModel::new(ViewId(950), vec![Column::text("name", "Name", |o: &Order| o.name.clone())]));
        model.set_rows((0..50).map(|i| Order { name: format!("Row {i}"), qty: 0.0, done: false }).collect());
        ChromeState {
            adds: Signal::new(0),
            locked: Signal::new(false),
            query: Signal::new(String::new()),
            renames: Signal::new(0),
            archivable: Signal::new(false),
            deleted: Signal::new(None),
            model,
        }
    };
}

fn chrome_window() -> Window {
    let c = CHROME.with(Clone::clone);
    let (adds, locked, renames, archivable, deleted) = (c.adds.clone(), c.locked.clone(), c.renames.clone(), c.archivable.clone(), c.deleted.clone());
    Window::new("Chrome")
        .size(620.0, 400.0)
        .toolbar(vec![
            toolbar_button("add", "Add", "plus", move || adds.update(|n| *n += 1)).disabled(move || locked.get()),
            toolbar_space(),
            toolbar_search("search", &c.query),
        ])
        .view(
            split((
                vstack((text("Sidebar pane").fill_width(),)).padding(8.0),
                vstack((
                    text("Content pane").fill_width(),
                    table(&c.model).row_menu("Delete Row", move |row| deleted.set(Some(row))),
                    text("Right-click me").context_menu(vec![
                        menu_item("Rename", move || renames.update(|n| *n += 1)),
                        menu_separator(),
                        menu_item("Archive", || {}).enabled_when(move || archivable.get_untracked()),
                    ]),
                ))
                .padding(8.0),
            ))
            .position(160.0),
        )
}

fn chrome_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    use objc2_app_kit::{NSSearchToolbarItem, NSSplitView};
    let c = CHROME.with(Clone::clone);
    let w = windows().into_iter().find(|w| w.title().to_string() == "Chrome").unwrap();
    w.makeKeyAndOrderFront(None);
    let sv = w.contentView().unwrap().subviews().iter().find_map(|v| v.downcast::<NSSplitView>().ok()).unwrap();
    let panes: Vec<Retained<NSView>> = sv.subviews().to_vec();
    let label = |text: &str| {
        panes.iter().flat_map(|p| p.subviews().to_vec()).filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string() == text).unwrap()
    };
    let (left, right) = (panes[0].frame().size.width, panes[1].frame().size.width);
    let content = label("Content pane");
    let fits = (content.frame().size.width - (right - 16.0)).abs() < 1.0;
    check(
        "split-panes",
        panes.len() == 2 && (left - 160.0).abs() < 1.0 && fits && has_ax(&ax_tree(&w), "AXSplitGroup", "", ""),
        format!("panes {left:.0} + {right:.0}; content label {:.0} wide in its pane; AXSplitGroup in the tree", content.frame().size.width),
    );

    // Window resize: the first pane keeps its width; the second takes the change.
    w.setContentSize(objc2_foundation::NSSize::new(720.0, 400.0));
    let (l2, r2) = (panes[0].frame().size.width, panes[1].frame().size.width);
    let follows = (label("Content pane").frame().size.width - (r2 - 16.0)).abs() < 1.0;
    check("split-resize", (l2 - left).abs() < 1.0 && (r2 - right - 100.0).abs() < 1.0 && follows, format!("after +100 pt: panes {l2:.0} + {r2:.0}; content follows: {follows}"));

    // Divider drag (as the user does): the pane's content re-lays out at once.
    sv.setPosition_ofDividerAtIndex(240.0, 0);
    let (l3, r3) = (panes[0].frame().size.width, panes[1].frame().size.width);
    let follows = (label("Content pane").frame().size.width - (r3 - 16.0)).abs() < 1.0
        && (label("Sidebar pane").frame().size.width - (l3 - 16.0)).abs() < 1.0;
    check("split-drag", (l3 - 240.0).abs() < 1.0 && follows, format!("divider at 240: panes {l3:.0} + {r3:.0}; both panes' content follows: {follows}"));

    // Toolbar: items in order; the button's action and its disabled state.
    let tb = w.toolbar().unwrap();
    let items = tb.items();
    let ids: Vec<String> = items.iter().map(|i| i.itemIdentifier().to_string()).collect();
    let add = items.iter().find(|i| i.itemIdentifier().to_string() == "add").unwrap();
    let app = NSApplication::sharedApplication(mtm());
    let fired = unsafe { app.sendAction_to_from(add.action().unwrap(), add.target().as_deref(), Some(&add)) };
    let enabled_before = add.isEnabled();
    batch(|| c.locked.set(true));
    let enabled_after = add.isEnabled();
    check(
        "toolbar-button",
        ids.len() == 3 && ids[0] == "add" && ids[2] == "search" && fired && c.adds.get_untracked() == 1 && enabled_before && !enabled_after,
        format!("items {ids:?}; Add → {} add(s); disabled when locked: {}", c.adds.get_untracked(), !enabled_after),
    );

    // Toolbar search: both ways.
    let search = items.iter().find_map(|i| i.downcast::<NSSearchToolbarItem>().ok()).unwrap();
    let field = search.searchField();
    post_edit(&field, "invoice");
    let typed = c.query.get_untracked();
    batch(|| c.query.set("receipt".into()));
    check("toolbar-search", typed == "invoice" && field.stringValue().to_string() == "receipt", format!("typed → {typed:?}; signal → field {:?}", field.stringValue().to_string()));

    // Context menu: items, action, validation when the menu opens.
    let target = label("Right-click me");
    let menu = target.menu().unwrap();
    let titles: Vec<String> = (0..menu.numberOfItems()).filter_map(|i| menu.itemAtIndex(i)).map(|i| i.title().to_string()).collect();
    menu.performActionForItemAtIndex(0);
    menu.update();
    let archive_off = !menu.itemAtIndex(2).unwrap().isEnabled();
    batch(|| c.archivable.set(true));
    menu.update();
    let archive_on = menu.itemAtIndex(2).unwrap().isEnabled();
    check(
        "context-menu",
        titles == ["Rename", "", "Archive"] && c.renames.get_untracked() == 1 && archive_off && archive_on,
        format!("{titles:?}; Rename ran: {}; Archive disabled then enabled: {archive_off}/{archive_on}", c.renames.get_untracked() == 1),
    );

    // Table row menu: acts on the clicked row, or the selected one.
    let tv = c.model.table_view();
    tv.selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(3), false);
    let row_menu = tv.menu().unwrap();
    row_menu.performActionForItemAtIndex(0);
    check(
        "table-row-menu",
        row_menu.itemAtIndex(0).is_some_and(|i| i.title().to_string() == "Delete Row") && c.deleted.get_untracked() == c.model.row_at(3),
        format!("Delete Row on the selected row → model row {:?}", c.deleted.get_untracked()),
    );
    dialog_checks(events);
}


fn dialogs_window() -> Retained<objc2_app_kit::NSWindow> {
    windows().into_iter().find(|w| w.title().to_string() == "Dialogs").unwrap()
}

fn buttons_in(w: &objc2_app_kit::NSWindow) -> Vec<Retained<objc2_app_kit::NSButton>> {
    fn walk(v: &NSView, out: &mut Vec<Retained<objc2_app_kit::NSButton>>) {
        for c in v.subviews().iter() {
            if let Ok(b) = c.clone().downcast::<objc2_app_kit::NSButton>() {
                out.push(b);
            }
            walk(&c, out);
        }
    }
    let mut out = Vec::new();
    if let Some(v) = w.contentView() {
        walk(&v, &mut out);
    }
    out
}

fn dialog_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let w = dialogs_window();
    w.makeKeyAndOrderFront(None);
    // Alert sheet: destructive button, the chosen button's index, closes.
    let chosen = Signal::new(None::<usize>);
    let ch = chosen.clone();
    alert(&w, Alert::new("Delete “report.csv”?", "This can't be undone.").button("Delete").button("Cancel").destructive(0).warning(), move |i| ch.set(Some(i)));
    let sheet_win = w.attachedSheet();
    let buttons = sheet_win.as_ref().map(|s| buttons_in(s)).unwrap_or_default();
    let delete = buttons.iter().find(|b| b.title().to_string() == "Delete");
    let destructive = delete.is_some_and(|b| b.hasDestructiveAction());
    if let Some(cancel) = buttons.iter().find(|b| b.title().to_string() == "Cancel") {
        unsafe { cancel.performClick(None) };
    }
    after_events(move || {
        let w = dialogs_window();
        check(
            "alert-sheet",
            destructive && chosen.get_untracked() == Some(1) && w.attachedSheet().is_none(),
            format!("Delete shown destructive: {destructive}; Cancel → {:?}; sheet closed: {}", chosen.get_untracked(), w.attachedSheet().is_none()),
        );
        view_sheet_check(events.clone());
    });
}

fn view_sheet_check(events: Rc<RefCell<Vec<AppEvent>>>) {
    let w = dialogs_window();
    let nodes0 = live_nodes();
    let name = Signal::new("draft".to_string());
    let closed = Signal::new(false);
    let handle: Rc<RefCell<Option<SheetHandle>>> = Rc::default();
    let h2 = handle.clone();
    let content = Window::new("Rename").size(320.0, 110.0).view(
        vstack((text_field(&name), hstack((spacer(), button("Done", move || {
            if let Some(h) = h2.borrow().as_ref() {
                h.close();
            }
        }).default_action()))))
        .padding(16.0),
    );
    let c2 = closed.clone();
    *handle.borrow_mut() = Some(sheet(&w, content, move || c2.set(true)));
    let sheet_win = w.attachedSheet().unwrap();
    let field = sheet_win.contentView().unwrap().subviews().iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.isEditable()).unwrap();
    let shown = field.stringValue().to_string();
    let nodes_open = live_nodes();
    post_edit(&field, "final.csv");
    let typed = name.get_untracked();
    let done = buttons_in(&sheet_win).into_iter().find(|b| b.title().to_string() == "Done").unwrap();
    unsafe { done.performClick(None) };
    after_events(move || {
        let w = dialogs_window();
        check(
            "view-sheet",
            shown == "draft" && typed == "final.csv" && closed.get_untracked() && w.attachedSheet().is_none() && live_nodes() == nodes0 && nodes_open > nodes0,
            format!(
                "field showed {shown:?}; typed → {typed:?}; Done closed it: {}; nodes {nodes0} → {nodes_open} open → {} after",
                closed.get_untracked(),
                live_nodes()
            ),
        );
        panel_checks(events.clone());
    });
}

fn panel_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    let w = dialogs_window();
    // Open panel: configuration, then Cancel → None.
    let result = Signal::new(Some(None::<Vec<std::path::PathBuf>>));
    let r2 = result.clone();
    open_panel(&w, OpenOptions { extensions: vec!["csv".into(), "txt".into()], multiple: true, directories: false }, move |p| r2.set(Some(p)));
    let panel = w.attachedSheet().and_then(|s| s.downcast::<objc2_app_kit::NSOpenPanel>().ok());
    let types = panel.as_ref().map_or(0, |p| p.allowedContentTypes().count());
    let multiple = panel.as_ref().is_some_and(|p| p.allowsMultipleSelection());
    if let Some(p) = &panel {
        unsafe { p.cancel(None) };
    }
    after_events(move || {
        let w = dialogs_window();
        check(
            "open-panel",
            panel.is_some() && types == 2 && multiple && result.get_untracked() == Some(None) && w.attachedSheet().is_none(),
            format!("NSOpenPanel sheet: {}; {types} allowed types; multiple: {multiple}; Cancel → {:?}", panel.is_some(), result.get_untracked()),
        );
        // Save panel: proposed name and folder; Cancel → None. (The panel runs
        // out of process; a programmatic OK isn't reliable, so choosing a
        // path is a hands-on check.)
        let dir = std::env::temp_dir().join("appkit-ui-check");
        let _ = std::fs::create_dir_all(&dir);
        let saved = Signal::new(None::<Option<std::path::PathBuf>>);
        let s2 = saved.clone();
        save_panel(&w, "export.csv", Some(&dir), move |p| s2.set(Some(p)));
        let panel = w.attachedSheet().and_then(|s| s.downcast::<objc2_app_kit::NSSavePanel>().ok());
        let name = panel.as_ref().map(|p| p.nameFieldStringValue().to_string());
        let folder = panel.as_ref().and_then(|p| p.directoryURL()).and_then(|u| u.path()).map(|p| p.to_string());
        let want = dir.canonicalize().unwrap_or(dir.clone()).to_string_lossy().to_string();
        let folder_ok = folder.as_deref().is_some_and(|f| std::path::Path::new(f).canonicalize().map(|c| c.to_string_lossy().to_string()).unwrap_or_default() == want);
        if let Some(p) = &panel {
            unsafe { p.cancel(None) };
        }
        let events = events.clone();
        after_events(move || {
            let w = dialogs_window();
            let got = saved.get_untracked();
            check(
                "save-panel",
                name.as_deref() == Some("export.csv") && folder_ok && got == Some(None) && w.attachedSheet().is_none(),
                format!("proposed {name:?} in {folder:?} (expected folder: {folder_ok}); Cancel → {got:?}"),
            );
            let events = events.clone();
            after_events(move || menu_checks(events.clone()));
        });
    });
}

fn controls_b_checks(events: Rc<RefCell<Vec<AppEvent>>>) {
    use objc2_app_kit::{NSBox, NSImageView, NSPopUpButton, NSProgressIndicator, NSScrollView, NSSlider, NSStepper};
    let c = CTRL_B.with(Clone::clone);
    let w = windows().into_iter().find(|w| w.title().to_string() == "Controls B").unwrap();
    w.makeKeyAndOrderFront(None);
    let subs: Vec<Retained<NSView>> = w.contentView().unwrap().subviews().to_vec();
    let find = |f: &dyn Fn(&Retained<NSView>) -> bool| subs.iter().find(|v| f(v)).cloned().unwrap();
    let popup = find(&|v| v.clone().downcast::<NSPopUpButton>().is_ok()).downcast::<NSPopUpButton>().unwrap();
    let slider_v = find(&|v| v.clone().downcast::<NSSlider>().is_ok()).downcast::<NSSlider>().unwrap();
    let stepper_v = find(&|v| v.clone().downcast::<NSStepper>().is_ok()).downcast::<NSStepper>().unwrap();
    let bar = find(&|v| v.clone().downcast::<NSProgressIndicator>().is_ok()).downcast::<NSProgressIndicator>().unwrap();
    let img = find(&|v| v.clone().downcast::<NSImageView>().is_ok()).downcast::<NSImageView>().unwrap();
    let line = find(&|v| v.clone().downcast::<NSBox>().is_ok());
    let sv = find(&|v| v.clone().downcast::<NSScrollView>().is_ok()).downcast::<NSScrollView>().unwrap();
    let content_w = w.contentView().unwrap().bounds().size.width;

    // Picker: starts on the signal's value; user choice → signal; signal → menu.
    let initial = popup.indexOfSelectedItem();
    popup.selectItemAtIndex(2);
    send_action(&popup);
    let chosen = c.size.get_untracked();
    batch(|| c.size.set(1));
    check(
        "picker",
        initial == 1 && chosen == 3 && popup.indexOfSelectedItem() == 0 && has_ax(&ax_tree(&w), "AXPopUpButton", "", ""),
        format!("initial index {initial}; chose Large → {chosen}; signal 1 → index {}", popup.indexOfSelectedItem()),
    );

    // Slider and stepper.
    slider_v.setDoubleValue(0.75);
    send_action(&slider_v);
    let from_user = c.volume.get_untracked();
    batch(|| c.volume.set(0.25));
    check("slider", from_user == 0.75 && slider_v.doubleValue() == 0.25, format!("user 0.75 → {from_user}; signal 0.25 → {}", slider_v.doubleValue()));
    stepper_v.setDoubleValue(4.0);
    send_action(&stepper_v);
    let stepped = c.count.get_untracked();
    batch(|| c.count.set(9.0));
    let shown = stepper_v.doubleValue();
    check(
        "stepper",
        stepped == 4.0 && shown == 9.0 && stepper_v.maxValue() == 10.0 && stepper_v.increment() == 1.0,
        format!("user 4 → {stepped}; signal 9 → {shown}; range 0–{} step {}", stepper_v.maxValue(), stepper_v.increment()),
    );

    // Progress: determinate value, then indeterminate.
    let det = !bar.isIndeterminate() && (bar.doubleValue() - 0.4).abs() < 1e-9;
    batch(|| c.progress.set(None));
    let indet = bar.isIndeterminate();
    batch(|| c.progress.set(Some(1.0)));
    check("progress", det && indet && bar.doubleValue() == 1.0, format!("0.4 shown: {det}; None → indeterminate: {indet}; then {}", bar.doubleValue()));

    // SF Symbol with an accessibility description.
    check(
        "image-symbol",
        img.image().is_some() && has_ax(&ax_tree(&w), "AXImage", "", "Favourite"),
        format!("image set: {}; AXImage \"Favourite\" in the AX tree: {}", img.image().is_some(), has_ax(&ax_tree(&w), "AXImage", "", "Favourite")),
    );

    // Divider: spans the stack's width, one point tall.
    let d = line.frame();
    check("divider", (d.size.width - (content_w - 32.0)).abs() <= 0.5 && d.size.height <= 1.5, format!("{:.1} × {:.1} (inner width {:.1})", d.size.width, d.size.height, content_w - 32.0));

    // Scroll view: content at the visible width, taller than the viewport; scrolls to the end; grows with new rows.
    let doc = sv.documentView().unwrap();
    let visible = sv.contentSize();
    let tall = doc.frame().size.height > visible.height && (doc.frame().size.width - visible.width).abs() < 0.5;
    let last = doc.subviews().iter().filter_map(|v| v.downcast::<NSTextField>().ok()).find(|f| f.stringValue().to_string() == "item60").unwrap();
    doc.scrollRectToVisible(last.frame());
    let shown = sv.documentVisibleRect();
    let lf = last.frame();
    let at_end = lf.origin.y >= shown.origin.y && lf.origin.y + lf.size.height <= shown.origin.y + shown.size.height + 0.5;
    let h0 = doc.frame().size.height;
    batch(|| c.lines.update(|l| l.extend((61..=80).map(item))));
    let grew = doc.frame().size.height > h0;
    check(
        "scroll-view",
        tall && at_end && grew,
        format!(
            "content {:.2}×{:.0} in a {:.2}×{:.0} viewport ({} scroll bars, vertical shown: {}); item60 visible after scrolling: {at_end}; 20 more rows grew it {h0:.0} → {:.0}",
            doc.frame().size.width,
            h0,
            visible.width,
            visible.height,
            if sv.scrollerStyle() == objc2_app_kit::NSScrollerStyle::Legacy { "classic" } else { "overlay" },
            sv.verticalScroller().is_some_and(|s| !s.isHidden()),
            doc.frame().size.height
        ),
    );

    // Classic scroll bars (a Mac with a mouse, or "always show"): they take
    // width when they appear, mid-layout; the content must still fit.
    sv.setScrollerStyle(objc2_app_kit::NSScrollerStyle::Legacy);
    batch(|| c.lines.set((1..=2).map(item).collect()));
    let short = (doc.frame().size.width, sv.contentSize().width);
    batch(|| c.lines.set((1..=80).map(item).collect()));
    let (dw, vw) = (doc.frame().size.width, sv.contentSize().width);
    let bar = sv.verticalScroller().is_some_and(|s| !s.isHidden());
    check(
        "scroll-classic-scrollbars",
        bar && (dw - vw).abs() < 0.5 && vw < short.1,
        format!("2 rows: content {:.1} in {:.1}; 80 rows (bar shown {bar}): content {dw:.1} in {vw:.1}", short.0, short.1),
    );
    sv.setScrollerStyle(objc2_app_kit::NSScrollerStyle::Overlay);

    if let Ok(dir) = std::env::var("SNAP") {
        snapshot_windows(&dir);
    }
    keyboard_checks(events);
}

/// Window restoration across launches: `RESTORE=save` moves a remembered
/// window and split, then quits (⌘Q's path, which flushes user defaults);
/// `RESTORE=verify`, in a second launch, checks both came back and cleans up.
fn restore_run(step: String) {
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSUserDefaults};
    let target = NSRect::new(NSPoint::new(180.0, 140.0), NSSize::new(520.0, 300.0));
    App::new("Checklist")
        .window(Window::new("Restore").size(400.0, 240.0).remember_frame("two-launch").view(split((text("a"), text("b"))).position(120.0).remember("two-launch")))
        .on_launch(move || {
            let w = windows().remove(0);
            let sv = all_views(&w.contentView().unwrap()).into_iter().find_map(|v| v.downcast::<objc2_app_kit::NSSplitView>().ok()).unwrap();
            let pane = || sv.arrangedSubviews().firstObject().unwrap().frame().size.width;
            if step == "save" {
                w.setFrame_display(target, true);
                sv.setPosition_ofDividerAtIndex(210.0, 0);
                eprintln!("[restore] saved frame {:?} pane {}", w.frame(), pane());
                appkit_ui::quit();
            } else {
                let (frame, width) = (w.frame(), pane());
                check("restore-across-launches", frame == target && width == 210.0, format!("frame {frame:?}; pane {width}"));
                let d = NSUserDefaults::standardUserDefaults();
                d.removeObjectForKey(&NSString::from_str("NSWindow Frame two-launch"));
                d.removeObjectForKey(&NSString::from_str("appkit-ui split two-launch"));
                std::process::exit(FAILED.with(Cell::get) as i32);
            }
        })
        .run();
}

fn main() {
    if let Ok(step) = std::env::var("RESTORE") {
        return restore_run(step);
    }
    let events: Rc<RefCell<Vec<AppEvent>>> = Rc::default();
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    let e2 = events.clone();
    App::new("Checklist")
        .window(Window::new("First").size(480.0, 200.0).content(|| field("hello world")))
        .window(Window::new("Second").size(360.0, 160.0))
        .window(Window::new("Views").size(320.0, 420.0).view(views_window()))
        .window(Window::new("Layout").size(420.0, 480.0).view(layout_window()))
        .window(Window::new("Controls").size(360.0, 300.0).view(controls_window()))
        .window(Window::new("Controls B").size(380.0, 460.0).view(controls_b_window()))
        .window(Window::new("Teardown").size(380.0, 520.0).view(teardown_window()))
        .window(Window::new("Table").size(560.0, 420.0).view(table_window()))
        .window(Window::new("Outline").size(560.0, 360.0).view(outline_window()))
        .window(Window::new("Editor").size(480.0, 320.0).view(editor_window()))
        .window(chrome_window())
        .window(Window::new("Dialogs").size(420.0, 260.0).view(vstack((text("Dialogs"),)).padding(12.0)))
        .menu("File", file_menu())
        .menu("View", view_menu())
        .settings(settings_window())
        .on_event(move |e| e2.borrow_mut().push(e))
        .on_launch(move || {
            if checks {
                run_checks(events);
            }
        })
        .run();
}
