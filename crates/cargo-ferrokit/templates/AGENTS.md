# Working on {{display}}

{{display}} is a native Mac app built with [Ferrokit](https://github.com/lhassa8/ferrokit) (`ferrokit` {{version}}): every view is a real AppKit view (`NSTableView`, `NSTextView`…), built once and updated in place by signals. macOS 15 or later; everything runs on the main thread.

## Commands

- `cargo ferrokit run`: build, bundle and run the app (with its name and icon).
- `cargo ferrokit check`: clippy (warnings are errors), tests, a bundle, then the app's self-checks (`CHECK=all`). Run it after every change.
- `cargo ferrokit bundle` / `notarize`: a signed `.app`; settings in `[package.metadata.ferrokit]` in `Cargo.toml`.

## Where the docs are

Read before guessing; names are regular, but behaviour has rules.
- **The guide:** <https://docs.rs/ferrokit/latest/ferrokit/guide/>, 13 chapters with compiled examples. Also in the crate's source: `~/.cargo/registry/src/*/ferrokit-{{version}}.*/guide/`.
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
/// `2026-11-03`: a date column that sorts as text.
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
- **A table's selection in a signal:** `table(&model).selection(&selected)` binds a `Signal<Vec<usize>>` of model indices both ways.
- **A filtered table that keeps its selection:** keep the selected item's id in a signal (set it from the bound selection), and in the `watch` that calls `set_rows`, find that id's new model index and set the bound signal to it (`set_rows` clears the selection).
- **Delete with ⌘⌫:** handle it with `.on_command(DELETE, …)` on the table, not the window: then it doesn't fire while the focus is in a text field or editor, where ⌘⌫ deletes text.
- **Counts in a sidebar:** the title is a closure, so `format!("{} ({n})", …)` shows a count.
- **Menu titles that change:** `menu_command(move || if read.get() { "Mark as Unread" } else { "Mark as Read" }.to_string(), TOGGLE)`.

## Self-checks

`cargo ferrokit check` runs the app with `CHECK=all`; the app drives its own UI, prints `[check] <name> PASS|FAIL <note>` lines and exits non-zero on a failure (see `self_checks` in `src/main.rs`). Add a check for every behaviour you add.
- **Act in one turn, look in the next.** Effects and table updates run after the current callback returns. Chain steps with `spawn(|| std::thread::sleep(Duration::from_millis(30)), move |()| …)`.
- **Undo steps:** in an unattended run, AppKit's per-event undo groups don't close, so one ⌘Z can undo several steps. Use `as_undo_steps` (above).
- **Selecting from code is silent:** `TableModel::select` doesn't call `on_select`, and `set_rows` clears the selection. Run the selection's effect yourself in a check.
- **Checks see the real user defaults:** reset the `stored` keys and data files a check depends on at its start (or point the app at a temporary file under `CHECK`).
- **Commands:** `perform_in(Some(&window), ADD)` runs one as the menu would; `can_perform_in` says whether its menu item is enabled. `testing::menu_key` presses a menu shortcut; `testing::shortcut` performs a ⌘ or ⇧⌘ shortcut without the app being active: your own menu items and commands, and standard actions (Undo, Copy) along the window's responder chain.
- **AppKit types in checks** need the bindings in `Cargo.toml` (`ferrokit` re-exports only `NSWindow` and `Retained`); methods from a trait (`NSTextInputClient`, `NSStandardKeyBindingResponding`, `NSAnimatablePropertyContainer`) need it in scope (`use … as _;`):
  ```toml
  objc2 = "0.6"
  objc2-foundation = "0.3"
  objc2-app-kit = { version = "0.3", features = ["NSResponder", "NSView", "NSWindow", "NSControl", "NSTextField", "NSButton", "NSText", "NSTextView", "NSTableView", "NSOutlineView", "NSEvent", "NSMenu", "NSMenuItem"] }
  ```
- **Seeing your own output:** `cargo ferrokit check` shows only the `[check]` lines. To see everything, run the bundled app yourself: `CHECK=all "target/debug/bundle/<App>.app/Contents/MacOS/<binary>"`.
- `testing::ax_audit(&window)` lists controls VoiceOver can't name: give each `.accessibility_label(…)`.
- A caught panic doesn't end the app; `check` stops 15 s after one if no more checks arrive. Look for `ferrokit: a panic in …` in the output.

## Known limits (0.4)

Work around these instead of searching for them:
- Table shortcuts need ⌘ (`MenuItem::key` adds it); a bare Delete key isn't bindable yet. Use ⌘⌫: `.key("\u{8}")`.
- Tables take dropped files (`on_files_dropped(&["txt"], |paths, row| …)`), not URLs dragged from a browser.
- Columns are text, number, check and progress: show dates as sortable text (`iso_date`, above).
- Table rows don't animate in or out; `for_each` lists do. Leave a table's rows still.
- A popover anchors to a view, not to a toolbar item: use a sheet from the toolbar.
- Sidebar items have a symbol and a title, no count badge: put the count in the title.
- No custom native views: `View` can't be implemented outside Ferrokit (`Cx` and `Node` are opaque). Raw AppKit on a view Ferrokit made (`model.table_view()`, `scroll_view()`) works, but it's on you to keep it working.
