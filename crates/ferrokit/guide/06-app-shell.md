# The app shell

## Menus

Every app gets the standard menu bar. Add your own menus with `App::menu`;
a menu named "File" is merged above the standard Close item.

```rust,no_run
use ferrokit::*;

let items = Signal::new(0);
let show_sidebar = Signal::new(true);
App::new("Notes")
    .menu("File", vec![
        menu_item("New Note", move || items.update(|n| *n += 1)).key("n"),                // ⌘N
        menu_item("Delete Note", move || items.update(|n| *n -= 1))
            .key_with("n", &[Modifier::Option])                                      // ⌥⌘N
            .disabled(move || !(items.get() > 0)),                                     // greyed out at 0
    ])
    .menu("View", vec![
        menu_item("Show Sidebar", move || show_sidebar.update(|s| *s = !*s))
            .key_with("s", &[Modifier::Control])
            .checked(move || show_sidebar.get()),                                         // ✓ when shown
        menu_separator(),
        submenu("Sort By", vec![menu_item("Name", || {}), menu_item("Date", || {})]),
    ])
    .window(Window::new("Notes"))
    .run();
```

A menu item's title, `.disabled(..)` and `.checked(..)` take a value or a
closure; closures are read when the menu opens or a shortcut is pressed. `dynamic_submenu(title, || items)` rebuilds its items
each time it opens, for menus that list current things (servers, recent
files).

## Commands: menus that act on the focus

A menu item made with `menu_command` doesn't run a fixed closure. It runs
the `Command` handler **nearest the focus**:

1. the focused view's handler, then each enclosing view's up to the window's root;
2. then the window's;
3. then the app's.

The item is enabled only while some handler answers, as with AppKit's
responder chain: below, ⌘K clears whichever field has focus, and is greyed
out while the title field has focus and is already empty.

```rust,no_run
use ferrokit::*;

const CLEAR: Command = Command::new("clear");
const REFRESH: Command = Command::new("refresh");

let (title, tag) = (Signal::new(String::from("Groceries")), Signal::new(String::new()));
let view = vstack((
    text_field(&title)
        .accessibility_label("Title")
        .on_command_when(CLEAR, move || !title.with(String::is_empty), move || title.set(String::new()))
        .focused_value("field", "title"),
    text_field(&tag)
        .accessibility_label("Tag")
        .on_command(CLEAR, move || tag.set(String::new()))
        .focused_value("field", "tag"),
))
.padding(20.0);
App::new("Notes")
    .menu("Note", vec![menu_command("Clear Field", CLEAR).key("k")])  // ⌘K clears the focused field
    .on_command(REFRESH, || println!("refresh everything"))            // app-wide fallback
    .window(Window::new("Notes").view(view).toolbar(vec![toolbar_command("refresh", "Refresh", "arrow.clockwise", REFRESH)]))
    .run();
```

- `on_command_when(cmd, enabled, action)` answers but greys the item out
  while `enabled` is false. Handlers further out aren't asked.
- `Window::on_command` and `App::on_command` handle a command for the whole
  window or app.
- `.focused_value(name, value)` publishes a value to the focus (a
  selection signal, say). `focused_value::<T>(name)` reads the nearest one,
  for menu items that need to know what they act on.
- `perform(cmd)` and `can_perform(cmd)` do the same from code;
  `perform_in(window, cmd)` names the window.
- Commands go to the key window, or, when the app isn't active, the most
  recently opened visible window (`focused_window()`).

## Document apps

`App::documents` makes an app document-based, like TextEdit or Numbers.
Each window is one file. A document's window comes back where it was
when the file is opened again, and if the user keeps windows when
quitting (System Settings › Desktop & Dock, "Close windows when quitting
an application" off), the documents open at quit reopen at the next
launch, as in Apple's apps.

- **The File menu:** New (⌘N), Open… (⌘O), Open Recent, Save (⌘S), Save As… (⇧⌘S) and Revert to Saved.
- **The title bar:** shows the file's name and proxy icon, and the dot in the close button while there are unsaved changes.
- **Closing or quitting with changes:** asks whether to save.

```rust,no_run
use ferrokit::*;

let text_files = DocumentType::new(&["txt", "md"], String::new, |doc: &Document<String>| {
    let model = TextModel::new(ViewId(1), &doc.content().get_untracked());
    text_editor(&model).bind(doc.content()).accessibility_label("Text")
})
.read(|path| std::fs::read_to_string(path))
.write(|text, path| std::fs::write(path, text));

App::new("Notes").documents(text_files).run(); // starts with an untitled document
```

- A `Document<T>`'s content is a signal: bind views to `doc.content()` or
  change it. The document is **edited** while the content differs from the
  file (so `T: PartialEq`): undoing back to the saved text clears the dot.
- Without `.write`, documents are read-only (a viewer: Save is disabled).
- Undo is per document, since each window has its own history.
- `new_document()`, `open_document(path)`, `save_document(window)`,
  `save_document_as(window, path)`, `revert_document(window)`,
  `document_edited(window)` and `recent_documents()` do the same from code,
  without panels. `choose_documents()` shows the Open panel, as Open… does.
- `DocumentType::window(|w| ..)` styles each document window with
  [`Window`]'s chrome methods, e.g. `|w| w.full_size_content()` for a
  sidebar that runs up under the title bar.

### A welcome window

`App::welcome(make)` shows a window at launch instead of an untitled
document, as Xcode does: recent documents, a sample, Open…. It closes when
a document opens, and Window ▸ Welcome to … (⇧⌘1) or `show_welcome()`
brings it back, built again so its list is current.

```rust,no_run
use ferrokit::*;

fn welcome() -> Window {
    let recent: Vec<AnyView> = recent_documents()
        .into_iter()
        .map(|path| {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            AnyView::new(text(name).padding(6.0).hover_highlight(Color::Quaternary).on_click(move || {
                open_document(&path).ok();
            }))
        })
        .collect();
    Window::new("Welcome").size(600.0, 400.0).view(hstack((
        vstack((button("Open…", choose_documents), button("New", || {
            new_document();
        }))),
        vstack(recent).width(240.0).material(Material::Sidebar),
    )))
}
# let text_files = DocumentType::new(&["txt"], String::new, |_: &Document<String>| text(""));
App::new("Notes").documents(text_files).welcome(welcome).run();
```

The SQL browser's welcome window is a fuller one: a large symbol, rows
that highlight under the pointer, and recent databases in a sidebar.

Files opened from the Finder (double-click, Open With, a drop on the Dock
icon, `open file.txt`) open as documents; an untouched Untitled window
from launch closes. For the Finder to offer your app, declare the types
when packaging:

```bash
scripts/package-app.sh -p my-app --bin my-app --name "My Notes" --doc-type "Plain Text:txt,md"
```

The `notes` example is a complete text editor built this way.

## Toolbar and split view

```rust,no_run
use ferrokit::*;

let query = Signal::new(String::new());
let window = Window::new("Library")
    .size(900.0, 600.0)
    .toolbar(vec![
        toolbar_button("add", "Add", "plus", || {}),
        toolbar_space(),
        toolbar_search("search", &query),
    ])
    .view(split((text("Sidebar"), text("Content"))).position(220.0).remember("library"));
# let _ = window;
```

Toolbar items show their symbols alone, as in Apple's apps; their labels
are what VoiceOver reads and what the overflow menu shows. With
`ToolbarStyle::Expanded` or `Preference` (a Settings window), the labels
show under the symbols.

### A sidebar and an inspector: the window shell

`Window::sidebar(view)` and `Window::inspector(view)` lay the window out
as Mail, Notes and Keynote are: AppKit's split view controller, with a
full-height sidebar on the sidebar material, the window's `view` beside
it, and an inspector after it. The title starts beside the sidebar,
Toggle Sidebar leads the toolbar and Toggle Inspector ends it, and the
user can drag either pane wider, narrower or shut.
`.sidebar_hidden(&signal)` and `.inspector_hidden(&signal)` bind whether
they're hidden, both ways.

```rust,no_run
use ferrokit::*;

let (no_sidebar, no_inspector) = (Signal::new(false), Signal::new(true));
let window = Window::new("Library")
    .toolbar(vec![toolbar_button("add", "Add", "plus", || {})])
    .sidebar(text("Mailboxes"))
    .sidebar_hidden(&no_sidebar)
    .view(text("Messages"))
    .inspector(text("Details"))
    .inspector_hidden(&no_inspector);
# let _ = window;
```

Each pane has its own tree; a closed window leaves no more behind than a
plain AppKit one built the same way (about 1 KB). The `split` view below
is for splits inside a window's content.

### Groups, pickers and customisation

`toolbar_group` puts related buttons in one capsule (zoom out and in,
back and forward); `toolbar_picker` is a choice of one as segments,
bound to a signal (Finder's view modes). `toolbar_fixed_space()` is a
fixed gap, `toolbar_space()` a flexible one.

`Window::customizable_toolbar(name)` lets the user change the toolbar:
Customize Toolbar… on its right-click menu, or `customize_toolbar_item()`
in a View menu, opens AppKit's sheet, where items and spaces can be added,
removed and moved. Items marked `.optional()` are offered there without
being shown at first. The user's layout is remembered under `name`;
until they change it, the app's own layout shows (so a later version's
new default still reaches them).

```rust,no_run
use ferrokit::*;

#[derive(Clone, Copy, PartialEq)]
enum Mode { List, Grid }
let (zoom, mode) = (Signal::new(1.0), Signal::new(Mode::List));
let window = Window::new("Photos")
    .toolbar(vec![
        toolbar_group("zoom", "Zoom", vec![
            toolbar_button("out", "Zoom Out", "minus.magnifyingglass", move || zoom.update(|z| *z /= 1.25)),
            toolbar_button("in", "Zoom In", "plus.magnifyingglass", move || zoom.update(|z| *z *= 1.25)),
        ]),
        toolbar_space(),
        toolbar_picker("mode", "View", &mode, vec![("List", "list.bullet", Mode::List), ("Grid", "square.grid.2x2", Mode::Grid)]),
        toolbar_button("share", "Share", "square.and.arrow.up", || {}).optional(),
    ])
    .customizable_toolbar("photos");
# let _ = window;
```

`.remember(name)` keeps the divider's position across launches;
`Window::remember_frame(name)` does the same for the window's frame.

`.stacked()` puts the panes top to bottom (an editor over its results).

### Sidebars and inspectors that collapse

`.collapsible(index, &signal)` hides a pane while the signal is true and
sets the signal when the user drags the pane shut. The first pane and
collapsible ones keep their size as the window resizes, and a pane reopens
at the size it had. `.trailing(points)` gives the last pane (an inspector)
its first width. The menu items follow the Mac convention: ⌃⌘S and ⌥⌘I,
titled "Hide…" or "Show…" by a closure as the title:

```rust,no_run
use ferrokit::*;

let (no_sidebar, no_inspector) = (Signal::new(false), Signal::new(false));
let app = App::new("Library")
    .menu(
        "View",
        vec![
            menu_item(move || format!("{} Sidebar", if no_sidebar.get() { "Show" } else { "Hide" }), move || no_sidebar.update(|h| *h = !*h))
                .key_with("s", &[Modifier::Control]),
            menu_item(move || format!("{} Inspector", if no_inspector.get() { "Show" } else { "Hide" }), move || no_inspector.update(|h| *h = !*h))
                .key_with("i", &[Modifier::Option]),
        ],
    )
    .window(Window::new("Library").view(
        split((text("Sidebar"), text("Content"), text("Inspector")))
            .collapsible(0, &no_sidebar)
            .collapsible(2, &no_inspector)
            .position(220.0)
            .trailing(260.0),
    ));
# let _ = app;
```

In a document app, make these commands (see above) so they act on the
focused window: the SQL browser example does.

### Popovers

`.popover(&shown, || content)` shows content in a popover pointing at a
view while `shown` is true. Escape or a click outside closes it and sets
`shown` back to false. The content is built each time it opens and freed
when it closes. `popover_on(Edge::Trailing, …)` picks the side.

```rust,no_run
use ferrokit::*;

let shown = Signal::new(false);
let info = button("Info", move || shown.set(true))
    .popover(&shown, || vstack((text("Version 1.0"), text("Made with ferrokit"))).padding(12.0));
# let _ = info;
```

## Sheets, alerts and panels

```rust,no_run
use ferrokit::*;

fn delete(window: &NSWindow) {
    let alert_box = Alert::new("Delete the note?", "This can't be undone.").button("Delete").button("Cancel").destructive(0);
    alert(window, alert_box, |index| if index == 0 { println!("deleted") });
}

fn rename(window: &NSWindow) {
    // Built in `sheet_with`: the sheet's signals are freed when it closes.
    let handle = sheet_with(window, || {
        let name = Signal::new(String::from("Untitled"));
        Window::new("Rename").size(320.0, 100.0).view(text_field(&name).accessibility_label("Name").padding(16.0))
    }, || println!("sheet closed"));
    // handle.close() ends it, e.g. from a Done button.
    # let _ = handle;
}

fn choose(window: &NSWindow) {
    open_panel(window, OpenOptions { extensions: vec!["csv".into()], multiple: false, directories: false }, |paths| {
        println!("{paths:?}");
    });
    save_panel(window, "Untitled.csv", None, |path| println!("{path:?}"));
}
# fn main() {}
```

`NSWindow` is AppKit's window type, re-exported by `ferrokit`;
`windows()` returns the app's open windows.

**Where signals live.** A signal belongs to what was being built when it
was made, and goes with it: a window's view, a row of a list. One made in
an action (a button, a menu item), outside any view, belongs to the window
or app the action is in, and lives as long as that. So build a sheet or a
new window with `sheet_with(parent, || …)` or `open_with(|| …)`: the
signals its builder makes (the form's fields) are freed when it closes.
The habits soak found New Habit keeping three signals per sheet before it
did this.

## Settings

`App::settings(window)` adds Settings… (⌘,) to the app menu. The window is
built the first time it opens and kept; pair it with `stored` signals so
the values persist.

```rust,no_run
use ferrokit::*;

let dark_tables = stored("dark tables", false);
App::new("Viewer")
    .settings(Window::new("Settings").size(360.0, 120.0).view(toggle("Dark tables", &dark_tables).padding(20.0)))
    .window(Window::new("Viewer"))
    .run();
```

## Undo

Every window has its own undo history, shared by its text fields. Make your
own changes undoable, and they appear in the Edit menu by name ("Undo
Rename"):

```rust,no_run
use ferrokit::*;

let title = Signal::new(String::from("Draft"));
set_undoable("Rename", &title, "Final".into());     // ⌘Z puts "Draft" back

let count = Signal::new(0);
undoable("Add Item", move || count.update(|n| *n += 1), move || count.update(|n| *n -= 1));
```

Steps go to the frontmost window's history (`undoable_in` names the
window). Closing a window clears its history.

## Windows, appearance, quitting

- `open(window)` opens a window; the close button, ⌘W and `close_window`
  tear it down.
- `set_appearance(Appearance::Dark)` pins the app's look; `System` follows
  the system setting. `App::on_event` delivers `AppEvent::AppearanceChanged`.
- `quit()` ends the app, as ⌘Q does (starting on the next run-loop pass, so
  a document app's save prompts don't block the UI).

## Tabs, full screen and the Dock

```rust,no_run
use ferrokit::*;

App::new("Notes")
    // The Dock icon's menu, built each time it opens.
    .dock_menu(|| vec![menu_item("New Note", || {})])
    .window(Window::new("Note").tabbing(Tabbing::Preferred, Some("notes")).view(text("…")))
    .on_launch(|| set_dock_badge(Some("2")))
    .run();
```

- **Tabs:** `.tabbing(Tabbing::Preferred, group)` opens windows as tabs of
  the group's other windows (`Automatic` follows the user's "Prefer tabs"
  setting; `Disallowed` never). `open_tab(&window, Window::new(..))` adds
  one. The Window menu's tab commands work as in any Mac app.
- **Full screen:** resizable windows can go full screen; `.no_full_screen()`
  stops it for palettes and small utilities. `toggle_full_screen(&w)` and
  `is_full_screen(&w)` for your own commands.
- **Dock:** `set_dock_badge(Some("3"))` shows a badge on the Dock icon
  (`None` clears it); `App::dock_menu` adds items above the system's.

## Languages

Strings the user reads go through `t`, with the English text as the key.
The app's translations are `.strings` tables, one per language:

```rust,no_run
use ferrokit::*;

App::new("Notes")
    .translations(&[("de", "\"Run Query\" = \"Abfrage ausführen\";\n\"{n} row\" = \"{n} Zeile\";\n\"{n} rows\" = \"{n} Zeilen\";")])
    .menu(&t("Query"), vec![menu_item(&t("Run Query"), || {})])
    .window(Window::new(&t("Notes")).view(text(|| t_plural(3, "{n} row", "{n} rows"))))
    .run();
```

In a real app the table is a file, `include_str!("../locales/de.strings")`:

```text
"Run Query" = "Abfrage ausführen";
"{n} row" = "{n} Zeile";
"{n} rows" = "{n} Zeilen";
```

- The language is the first of the user's preferred languages (System
  Settings, or the app's own setting there) that the app has a table for.
  Strings without a translation stay in English.
- The framework's own strings (the standard menus, document alerts,
  "Untitled") come in German, French and Japanese. An app's table can
  override them.
- `t_with("Open “{file}”", &[("file", &name)])` fills named placeholders,
  which a translation can reorder.
- `t_plural(n, "{n} row", "{n} rows")` picks the language's plural form.
  For languages with more than two (Russian, Polish), the table gives
  `"{n} rows#one"`, `"#few"`, `"#many"` and `"#other"`. `{n}` is formatted
  for the user's region, as is `format_number(value, decimals)`.
- `format_date(seconds, template)` writes a date as the user's locale
  does from a template: `"EEEEdMMMM"` is "Friday, 31 October" in English
  and "Freitag, 31. Oktober" in German. `today()`, `local_day(seconds)`,
  `local_day_start(day)` and `weekday(day)` count calendar days in the
  user's time zone, through daylight saving: the key for "done today",
  streaks and calendars.
- Try another language without changing the system:
  `my-app -AppleLanguages "(de)"`.
- In the bundle, list the languages under `languages` in
  `[package.metadata.ferrokit]`. macOS then shows AppKit's own panels and
  menu items (Close All, the Open panel) in that language too.

### Right-to-left languages

In Arabic, Hebrew, Persian or Urdu (an app localised in one, for a user
who prefers it), the layout mirrors, as AppKit's own does: a row's first
view is on the right, padding and spacing flip, a form's labels sit to the
right of their controls, and a slide from the leading edge comes from the
right. AppKit mirrors the controls themselves (sliders, switches, the
window's buttons). Text keeps its natural direction. `right_to_left()`
says which way the app lays out, for drawing of your own on a canvas.

To try a layout right to left without translating it, launch the app with
`-AppleTextDirection YES -NSForceRightToLeftWritingDirection YES`; checks
can call `testing::set_right_to_left(Some(true))` before opening a window.
