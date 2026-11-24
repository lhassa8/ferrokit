# Migrating to the 1.0 API

Three releases, 0.13, 0.14 and 0.15 (November 2026), settled the API that 1.0 will promise. This guide takes an app from 0.12 or earlier to 0.15 in one pass. Each release's [CHANGELOG](CHANGELOG.md) section has the full list; this page orders it by what you'll meet.

**How long it takes:** almost all of it is mechanical, and the compiler finds every place. [Rowboat](https://apps.apple.com/us/app/rowboat-sqlite-editor/id6832358629), a 17,000-line app, changed 162 lines for 0.13, one for 0.14 and about 290 for 0.15, most of them in its self-checks.

## Upgrading

1. Move every Ferrokit crate to the same version: `ferrokit = "0.15"` (and `ferrokit-build`, and `cargo install cargo-ferrokit --locked` for the tool). The crates are released together and must match.
2. Run `cargo build`, then work down the errors with the sections below.
3. Run `cargo ferrokit check`: clippy, your tests and the app's self-checks.

From an older version than 0.12, read the CHANGELOG sections in between first; before 0.4 the crates had other names (see the end of this page).

## Windows are `WindowRef` (0.15)

Every function that returned `Retained<NSWindow>` returns a `WindowRef`, and every one that took `&NSWindow` takes `&WindowRef`: `open`, `windows`, `focused_window`, the dialogs, the document functions, `perform_in`, `undoable_in`, `print` and `testing`'s window functions.

Code that only passes a window along doesn't change:

```rust
let w = open(Window::new("Notes").view(editor));
alert(&w, Alert::new("Saved", "The note was saved."), |_| {});
```

Where you named the type, write `WindowRef` for `Retained<NSWindow>` and `&WindowRef` for `&NSWindow`:

```rust
// Before
fn export(window: &NSWindow, csv: String) { save_panel(window, "export.csv", None, move |path| { /* … */ }); }
// After
fn export(window: &WindowRef, csv: String) { save_panel(window, "export.csv", None, move |path| { /* … */ }); }
```

`WindowRef` has the common needs itself: `title()` (a `String`), `set_title`, `size`, `set_size`, `close`, `is_open`, `is_key` and `bring_to_front`. Two handles to the same window are `==`, and it's `Hash`, so it can be a map key. For anything else, `.ns()` gives AppKit's `NSWindow`:

| Before | After |
| --- | --- |
| `w.title().to_string()` | `w.title()` |
| `w.makeKeyAndOrderFront(None)` | `w.bring_to_front()` |
| `w.setTitle(&NSString::from_str("x"))` | `w.set_title("x")` |
| `w.contentView()` | `w.ns().contentView()` |
| `objc2::Message::retain(window)` or `window.retain()` | `window.clone()` |
| `Retained::as_ptr(&a) == Retained::as_ptr(&b)` | `a == b` |
| a window from AppKit (`view.window()`) passed to Ferrokit | `WindowRef::from_ns(&ns_window)` |

`ferrokit` no longer re-exports `NSWindow`; code that still names it writes `objc2_app_kit::NSWindow` and adds `objc2-app-kit` to its `Cargo.toml`.

## Models choose their own ids (0.13)

`TableModel` and `TextModel` no longer take a `ViewId`:

```rust
// Before
let model = TableModel::new(ViewId(1), vec![Column::text("name", "Name", |p: &Person| p.name.clone())]);
let text = TextModel::new(ViewId(2), "");
// After
let model = TableModel::new(vec![Column::text("name", "Name", |p: &Person| p.name.clone())]);
let text = TextModel::new("");
```

`ViewId` is gone from `ferrokit`.

## Builders instead of struct literals (0.13)

`OpenOptions` and `CellStyle` can gain fields now, so they're built with methods:

```rust
// Before
open_panel(&w, OpenOptions { extensions: vec!["csv".into()], multiple: true, directories: false }, |paths| { /* … */ });
CellStyle { color: Some(Color::Secondary), strikethrough: done, ..Default::default() }
// After
open_panel(&w, OpenOptions::new().extensions(&["csv"]).multiple(), |paths| { /* … */ });
CellStyle::new().color(Some(Color::Secondary)).strikethrough(done)
```

## A `_` arm in matches on growing enums (0.13)

These enums and structs are `#[non_exhaustive]`, so they can grow without breaking apps: `Color`, `ButtonStyle`, `ControlSize`, `SymbolEffect`, `DragItem`, `ItemImage`, `FilterTest`, `FieldKind`, `FormStyle`, `input::Key`, and the structs `OpenOptions`, `CellStyle`, `Failure`, `CollectionItem`, `Series`, `Shadow`, `FilterField`, `SentNotification` and `testing::AxElement`. A `match` on one needs a `_ =>` arm; the structs are made through their constructors and read through their fields.

## Smaller changes

| Before | After | Since |
| --- | --- | --- |
| `label(f)`, `Label` | `text(f)`, `Text` | 0.13 |
| `.color_when(f)`, `background_when(f)`, `opacity_when(f)` | `.color(f)`, `.background(f)`, `.opacity(f)` | 0.13 |
| `MenuItem::enabled_when(f)`, `checked_when(f)`, `title_when(f)` | `.disabled(move \|\| !f())`, `.checked(f)`, `menu_item(f, ..)` | 0.13 |
| `Mod::Command` (`testing::input`) | `Modifier::Command` | 0.13 |
| `testing::menu_key("n", NSEventModifierFlags::Command)` | `testing::menu_key("n", &[Modifier::Command])` | 0.14 |
| `Collection::on_activate(\|item: T\| ..)` | `on_activate(\|item: &T\| ..)` | 0.13 |
| `ferrokit::effect_runs()` and the other diagnostics in the crate root | `testing::live_nodes()`, `testing::live_signals()`; Ferrokit's own counters in `testing::probes` | 0.13, 0.14 |
| `ToolbarItem::Button { .. }` and the other variants | `toolbar_button(..)`, `toolbar_group(..)`, `toolbar_search(..)`… | 0.14 |
| `model.drain_events()`, `set_row_drag(..)` and the models' other plumbing | the `Table` and `TextEditor` modifiers (`.row_drag(..)`, `.on_select(..)`…) | 0.14 |
| implementing `View` yourself (`build`, `Cx`, `Node`) | compose Ferrokit's views in a function returning `impl View`, or wrap an `NSView` with `native(..)` | 0.14 |

Things that got easier and need no change: `.tooltip(..)`, `.accessibility_label(..)`, `progress`, `gauge`, `VideoPlayer::muted` and `MenuBarExtra::title` take a plain value or a closure, and a closure follows the signals it reads (0.13); `popover_on` works on every view (0.13).

## Escape hatches

What still names AppKit types after 0.15 (`WindowRef::ns`, `native(..)`, `table_view()`, `text_view()`, `Color::ns`, `Font::ns`, `Stored`, `testing`'s view lookups) follows objc2's versions rather than Ferrokit's: see [STABILITY.md](STABILITY.md). If your app uses them only in its self-checks, as most do, that's where future objc2 updates will touch it.

## From before 0.4

Until 0.4 the crates were called `appkit-ui` (now `ferrokit`) and `appkit-host` (now `ferrokit-core`). Their last releases only re-export the new crates: change the names in `Cargo.toml` and the `use` lines, then follow the CHANGELOG from 0.4 on. `iced-appkit` (AppKit views inside Iced windows) stopped at 0.2.1.

## Help

If something isn't covered here, or the compiler's error doesn't lead anywhere, [open an issue](https://github.com/lhassa8/ferrokit/issues): a missing step in this guide is a bug.
