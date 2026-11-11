# Working on {{display}}

{{display}} is a native Mac app built with [Ferrokit](https://github.com/lhassa8/ferrokit) (`ferrokit` {{version}}): every view is a real AppKit view (`NSTableView`, `NSTextView`…), built once and updated in place by signals. macOS 15 or later; everything runs on the main thread.

## Commands

- `cargo ferrokit run`: build, bundle and run the app (with its name and icon).
- `cargo ferrokit preview`: the same, then rebuild and restart on every save (for the developer at their screen; an agent checks with `cargo ferrokit check`).
- `cargo ferrokit check`: clippy (warnings are errors), tests, a bundle, then the app's self-checks (`CHECK=all`). Run it after every change.
- `cargo ferrokit bundle` / `notarize`: a signed `.app`; settings in `[package.metadata.ferrokit]` in `Cargo.toml`.

## Where the docs are

Read before guessing; names are regular, but behaviour has rules.
- **The guide:** <https://docs.rs/ferrokit/latest/ferrokit/guide/>, 14 chapters with compiled examples. Also in the crate's source: `~/.cargo/registry/src/*/ferrokit-{{version}}.*/guide/`.
- **Every public item on one page:** `API.md` in the same folder. Search it for what a type can do before reading source.
- **Complete apps:** `examples/` in the same folder. `native_csv.rs` (sidebar, table, search, sheet, notes), `habits.rs` (cards, sheets, charts, a menu-bar panel), `notes.rs` (a document app), `sql_browser.rs` (the flagship).
- `TableModel`, `Column` and `TextModel` are documented in `ferrokit-core` (re-exported by `ferrokit`): `~/.cargo/registry/src/*/ferrokit-core-{{version}}.*/src/table.rs` and `text.rs`.

## How a Ferrokit app is written

- **State is signals.** `Signal::new(v)` is `Copy`: closures take it with `move`, never clone it. `get()` tracks (the view or effect re-runs when it changes), `get_untracked()` doesn't, `with(|v| …)` reads without cloning, `set`/`update` write. A signal is freed with the window, row or sheet that made it.
- **Views are built once.** A view shows a value or a closure over signals (`text(move || format!(…))`); when the signals change, that one native view updates. Don't rebuild views to change them, and don't create native models per update.
- **Deep widgets have models.** Create a `TableModel` (or `TextModel`, `TreeNode`s) once, wrapped in `Rc`, and push data into it from a `watch(move || model.set_rows(rows.get()))` placed in the view tree. The `ViewId` it takes can be any number unique in the window.
- **Row indices are model indices** (positions in what you passed to `set_rows`), whatever the sort. Text offsets are UTF-8 bytes.
- **Menus and shortcuts are commands.** Declare `const ADD: Command = Command::new("add");`, put `menu_command("Add", ADD).key("n")` in `App::menu`, and handle it with `.on_command(ADD, …)` on a view (acts when the focus is in or under it) or `Window::on_command` (anywhere in the window). `on_command_when` greys the menu item out.
- **Background work:** `spawn(work, |result| …)` runs `work` on another thread and hands the result back on the main thread; `Resource` for loading that a newer load replaces.
- **Settings:** `stored("key", default)` is a signal saved in user defaults (bool, numbers, String). App data belongs in a file: `std::fs` and `serde` in `~/Library/Application Support/<bundle id>/` (the `habits` example keeps its small data in `stored` strings; a file scales better).
- **Errors and crashes:** `App::on_error(|failure| …)` hears about panics and exceptions Ferrokit caught (the app goes on); `App::crash_reports(|report| …)` writes a report for a panic that ends the app and hands it over at the next launch (show it or send it, then `report.mark_sent()`).
- **Updates:** Sparkle 2, set up in `Cargo.toml` (`[package.metadata.ferrokit.updates]`), not in code: the bundled app gets Check for Updates…; `updates::on_event`, `updates::check` and `updates::set_automatic_checks` are there if the app wants more. `cargo ferrokit appcast` writes the feed for a release.
- **A view Ferrokit doesn't have** (a map, a view you draw): `native(|mtm| MyView::new(mtm))` wraps any `NSView` (with the `objc2` crates in `Cargo.toml`, below). `.update(move |v| …)` changes it in place as an effect, like any view; `.measure(|v| Size { w, h })` and `.flexible(false, false)` size it; `.keep(delegate)` keeps a delegate (AppKit's are weak); `.on_remove(|v| …)` cleans up. The guide's *Custom native views* chapter has a delegate and a drawn view.
- **Undo:** `undoable("Delete", apply, revert)` runs `apply` now and puts `revert` on the window's undo history; ⌘Z and ⇧⌘Z work per window.
- Prefer Ferrokit's own API over raw `objc2` calls. Reach for `objc2-app-kit` only for something Ferrokit doesn't cover, and say so in a comment.

## Recipes

A sheet that closes itself: keep its handle where its buttons can reach it, and **read the sheet's signals before closing it** (they're freed on close).

```rust
use std::cell::RefCell;
use ferrokit::*;

thread_local! {
    static SHEET: RefCell<Option<SheetHandle>> = const { RefCell::new(None) };
}

fn ask_name(parent: &NSWindow, done: impl Fn(String) + Copy + 'static) {
    let close = || {
        if let Some(h) = SHEET.with_borrow_mut(Option::take) {
            h.close();
        }
    };
    let handle = sheet_with(parent, move || {
        let name = Signal::new(String::new());
        Window::new("Name").size(320.0, 120.0).view(
            vstack((
                text_field(&name).accessibility_label("Name"),
                hstack((
                    spacer(),
                    button("Cancel", close).cancel_action(), // Escape
                    button("Add", move || {
                        let value = name.get_untracked(); // before closing
                        close();
                        done(value);
                    })
                    .default_action(), // Return
                )),
            ))
            .padding(16.0),
        )
    }, || {});
    SHEET.with_borrow_mut(|s| *s = Some(handle));
}

/// A day from `local_day` or `today` (days since 1970-01-01) as
/// `2026-11-03`, for text (a table column takes `Column::date` instead).
fn iso_date(days: i64) -> String {
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// In a check: runs each step as its own undo step. AppKit ends an
/// event's undo group only when the app handles an event, so in an
/// unattended run the groups stay open across turns and one ⌘Z would undo
/// several steps. Then undo with `testing::shortcut` (⌘Z) or
/// `window.undoManager()`.
fn as_undo_steps(window: &NSWindow, steps: &[&dyn Fn()]) {
    let Some(undo) = window.undoManager() else { return };
    while undo.groupingLevel() > 0 {
        undo.endUndoGrouping();
    }
    undo.setGroupsByEvent(false);
    for step in steps {
        undo.beginUndoGrouping();
        step();
        undo.endUndoGrouping();
    }
}
```

- **A table row's menu:** `table(&model).row_menu("Delete", move |row| …)`; each call adds an item. Number columns: `Column::number(…).decimals(2)`.
- **A date column:** `Column::date("modified", "Date Modified", |r: &Row| Some(r.modified))` (a `SystemTime`): sorted by date, shown in the user's format ("Today", "12 Mar 2024"); `.with_time()` adds the time.
- **Delete and Space on a table:** `table(&model).on_key(TableKey::Delete, |rows| …)` (⌫ or ⌦, no ⌘) and `TableKey::Space` (Quick Look); keep a menu item for the same action.
- **Acting on one cell (Set to NULL, Copy Value):** `table(&model).cell_focus(&column)` with a `Signal<Option<String>>`: a click or ← → focuses a column, and the action reads the selected rows and `column.get()`; put it in a menu with a shortcut (⌥⌫).
- **Walking the disk:** read `has_full_disk_access()` once, then skip every folder where `asks_before_reading(path, access)` is true (show it as not scanned): a read that waits for the user's consent can't be cancelled and can stall the Mac. `open_url(FULL_DISK_ACCESS_SETTINGS)` opens the pane where the user allows it.
- **A table's selection in a signal:** `table(&model).selection(&selected)` binds a `Signal<Vec<usize>>` of model indices both ways.
- **A filtered table that keeps its selection:** keep the selected item's id in a signal (set it from the bound selection), and in the `watch` that calls `set_rows`, find that id's new model index and set the bound signal to it (`set_rows` clears the selection).
- **Delete with ⌘⌫:** handle it with `.on_command(DELETE, …)` on the table, not the window: then it doesn't fire while the focus is in a text field or editor, where ⌘⌫ deletes text.
- **A grid's rows are one type:** rows with different controls need each cell wrapped, `(AnyView::new(text("Plan:")), AnyView::new(picker(..)))`; or use `form(vec![section("", vec![form_row("Plan:", picker(..))])])`, which takes any control per row.
- **A command is enabled where its handler has the focus.** `.on_command(DELETE, …)` on the table greys out Delete's menu item and its `toolbar_command` button while the focus is in the search field or the sidebar. For a toolbar button that works wherever the focus is, use `toolbar_button(id, label, symbol, action).disabled(move || selected.with(Vec::is_empty))`, or add `Window::on_command_when`.
- **Shortcut keys without a letter** are their characters: `.key("\u{8}")` is ⌘⌫, `"\u{7f}"` ⌘⌦, `"\r"` ⌘↩, `"\u{1b}"` ⌘⎋, `"\u{f700}"`–`"\u{f703}"` ⌘↑ ↓ ← →.
- **A day as a `Column::date` value:** `UNIX_EPOCH + Duration::from_secs_f64(local_day_start(day))`, with `day` from `today()` or `local_day(seconds)`.
- **A sidebar's roots are section headings** (shown in small capitals, not selectable): put the rows the user picks under one, `TreeNode::branch(heading, rows)`.
- **Find (⌘F) and the Edit menu:** every app has Edit ▸ Find (Find…, Find and Replace…, Find Next, Find Previous, Use Selection for Find), enabled while the focus is in a `text_editor`; don't add your own. `App::menu("Edit", items)` adds items to the standard Edit menu, `"File"` and `"Help"` fill theirs; any other title is a new menu.
- **Lines that don't wrap** (code, logs): `TextModel::set_wraps(false)`.
- **A document's toolbar and its view share commands, not signals.** `DocumentType::toolbar(|doc| …)` can't see signals made in the view closure: use `toolbar_command(id, label, symbol, CMD)` and handle `CMD` with `.on_command(CMD, …)` on the document's root view (it acts wherever the focus is inside that view).
- **A document app's file types are listed twice:** `DocumentType::new(&["md", "txt"], …)` in code (the first is what a new file gets) and `[[package.metadata.ferrokit.documents]]` in `Cargo.toml` (what the Finder offers). Keep them the same.
- **A live HTML preview:** load the page once with `load_html`, then change it with `page.evaluate("document.body.innerHTML = …")` once `is_loading()` is false; `load_html` on every keystroke reloads the page and loses the scroll position.
- **Counts in a sidebar:** the title is a closure, so `format!("{} ({n})", …)` shows a count.
- **Menu titles that change:** `menu_command(move || if read.get() { "Mark as Unread" } else { "Mark as Read" }.to_string(), TOGGLE)`.

## Self-checks

`cargo ferrokit check` runs the app with `CHECK=all`; the app drives its own UI, prints `[check] <name> PASS|FAIL <note>` lines and exits non-zero on a failure (see `self_checks` in `src/main.rs`). Add a check for every behaviour you add.
- **The helpers:** `testing::check(name, pass, note)` prints the line and counts failures; `testing::skip(name, why)` for what can't run here; `testing::steps(100, vec![Box::new(|| …), …])` runs each step in a turn of its own and ends the run after the last (`testing::finish()` ends it by hand).
- **Act in one step, look in the next.** Effects, table updates and sheets settle after the step that caused them returns, so a step that sets a signal and reads a view in the same breath sees the old view.
- **Finding a view:** `testing::view_labeled(&window, "Preview")` returns the view with that `.accessibility_label(..)` (downcast it to `NSTextField`, `NSTableView`…); `testing::ax_elements(&window)` lists what VoiceOver sees, with each element's `name` and `value`. A `TextModel` or `TableModel` the checks need: keep its `Rc`/clone where they can reach it (a `thread_local`), there is no way from a window to its models.
- **Undo steps:** in an unattended run, AppKit's per-event undo groups don't close, so one ⌘Z can undo several steps. Use `as_undo_steps` (above).
- **Selecting from code is silent:** `TableModel::select` doesn't call `on_select`, and `set_rows` clears the selection. Run the selection's effect yourself in a check.
- **Checks see the real user defaults:** reset the `stored` keys and data files a check depends on at its start (or point the app at a temporary file under `CHECK`).
- **Commands:** `perform_in(Some(&window), ADD)` runs one as the menu would; `can_perform_in` says whether its menu item is enabled. `testing::menu_key` presses a menu shortcut; `testing::shortcut` performs a ⌘ or ⇧⌘ shortcut without the app being active: your own menu items and commands, and standard actions (Undo, Copy) along the window's responder chain.
- **AppKit types in checks** need the bindings in `Cargo.toml` (`ferrokit` re-exports only `NSWindow` and `Retained`); methods from a trait (`NSTextInputClient`, `NSStandardKeyBindingResponding`, `NSAnimatablePropertyContainer`) need it in scope (`use … as _;`):
  ```toml
  objc2 = "0.6"
  objc2-foundation = "0.3"
  objc2-app-kit = { version = "0.3", features = ["NSResponder", "NSView", "NSWindow", "NSControl", "NSTextField", "NSButton", "NSText", "NSTextView", "NSTableView", "NSOutlineView", "NSEvent", "NSMenu", "NSMenuItem", "NSApplication", "NSScrollView", "NSToolbar", "NSToolbarItem"] }
  ```
- **Real clicks and keys** (`testing::input`) need the app in front: `input::ready(&window)` stays false while the screen is locked or macOS keeps another app in front. Print `[check] <name> SKIP <why>` then, and drive the same behaviour another way (a command, `testing::shortcut`, `performClick`).
- **Seeing your own output:** `cargo ferrokit check` shows only the `[check]` lines. To see everything, run the bundled app yourself: `CHECK=all "target/debug/bundle/<App>.app/Contents/MacOS/<binary>"`.
- `testing::ax_audit(&window)` lists controls VoiceOver can't name: give each `.accessibility_label(…)`.
- A caught panic doesn't end the app; `check` stops 15 s after one if no more checks arrive. Look for `ferrokit: a panic in …` in the output.

## Known limits (0.6)

Work around these instead of searching for them:
- Table rows animate only through `TableModel::insert`, `push` and `remove` (with `Table::animated()`), not `set_rows` or in a grouped table.
- `View` can't be implemented outside Ferrokit (`Cx` and `Node` are opaque): wrap an `NSView` with `native(..)` instead (see Recipes). A wrapped view is a leaf: Ferrokit views can't go inside it.
