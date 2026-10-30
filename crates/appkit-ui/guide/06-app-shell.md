# The app shell

## Menus

Every app gets the standard menu bar. Add your own menus with `App::menu`;
a menu named "File" is merged above the standard Close item.

```rust,no_run
use appkit_ui::*;

let items = Signal::new(0);
let show_sidebar = Signal::new(true);
App::new("Notes")
    .menu("File", vec![
        menu_item("New Note", move || items.update(|n| *n += 1)).key("n"),                // ⌘N
        menu_item("Delete Note", move || items.update(|n| *n -= 1))
            .key_with("n", &[Modifier::Option])                                      // ⌥⌘N
            .enabled_when(move || items.get() > 0),                                     // greyed out at 0
    ])
    .menu("View", vec![
        menu_item("Show Sidebar", move || show_sidebar.update(|s| *s = !*s))
            .key_with("s", &[Modifier::Control])
            .checked_when(move || show_sidebar.get()),                                         // ✓ when shown
        menu_separator(),
        submenu("Sort By", vec![menu_item("Name", || {}), menu_item("Date", || {})]),
    ])
    .window(Window::new("Notes"))
    .run();
```

`enabled_when` and `checked_when` are evaluated when the menu opens or a
shortcut is pressed. `dynamic_submenu(title, || items)` rebuilds its items
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
use appkit_ui::*;

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
Each window is one file.

- **The File menu:** New (⌘N), Open… (⌘O), Open Recent, Save (⌘S), Save As… (⇧⌘S) and Revert to Saved.
- **The title bar:** shows the file's name and proxy icon, and the dot in the close button while there are unsaved changes.
- **Closing or quitting with changes:** asks whether to save.

```rust,no_run
use appkit_ui::*;

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
  without panels.

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
use appkit_ui::*;

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

`.remember(name)` keeps the divider's position across launches;
`Window::remember_frame(name)` does the same for the window's frame.

`.stacked()` puts the panes top to bottom (an editor over its results).

### Sidebars and inspectors that collapse

`.collapsible(index, &signal)` hides a pane while the signal is true and
sets the signal when the user drags the pane shut. The first pane and
collapsible ones keep their size as the window resizes, and a pane reopens
at the size it had. `.trailing(points)` gives the last pane (an inspector)
its first width. The menu items follow the Mac convention: ⌃⌘S and ⌥⌘I,
titled "Hide…" or "Show…" through `title_when`:

```rust,no_run
use appkit_ui::*;

let (no_sidebar, no_inspector) = (Signal::new(false), Signal::new(false));
let app = App::new("Library")
    .menu(
        "View",
        vec![
            menu_item("Hide Sidebar", move || no_sidebar.update(|h| *h = !*h))
                .key_with("s", &[Modifier::Control])
                .title_when(move || format!("{} Sidebar", if no_sidebar.get() { "Show" } else { "Hide" })),
            menu_item("Hide Inspector", move || no_inspector.update(|h| *h = !*h))
                .key_with("i", &[Modifier::Option])
                .title_when(move || format!("{} Inspector", if no_inspector.get() { "Show" } else { "Hide" })),
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
use appkit_ui::*;

let shown = Signal::new(false);
let info = button("Info", move || shown.set(true))
    .popover(&shown, || vstack((text("Version 1.0"), text("Made with appkit-ui"))).padding(12.0));
# let _ = info;
```

## Sheets, alerts and panels

```rust,no_run
use appkit_ui::*;

fn delete(window: &NSWindow) {
    let alert_box = Alert::new("Delete the note?", "This can't be undone.").button("Delete").button("Cancel").destructive(0);
    alert(window, alert_box, |index| if index == 0 { println!("deleted") });
}

fn rename(window: &NSWindow) {
    let name = Signal::new(String::from("Untitled"));
    let form = Window::new("Rename").size(320.0, 100.0).view(text_field(&name).accessibility_label("Name").padding(16.0));
    let handle = sheet(window, form, || println!("sheet closed"));
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

`NSWindow` is AppKit's window type, re-exported by `appkit-ui`;
`windows()` returns the app's open windows.

## Settings

`App::settings(window)` adds Settings… (⌘,) to the app menu. The window is
built the first time it opens and kept; pair it with `stored` signals so
the values persist.

```rust,no_run
use appkit_ui::*;

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
use appkit_ui::*;

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

## Languages

Strings the user reads go through `t`, with the English text as the key.
The app's translations are `.strings` tables, one per language:

```rust,no_run
use appkit_ui::*;

App::new("Notes")
    .translations(&[("de", "\"Run Query\" = \"Abfrage ausführen\";\n\"{n} row\" = \"{n} Zeile\";\n\"{n} rows\" = \"{n} Zeilen\";")])
    .menu(&t("Query"), vec![menu_item(&t("Run Query"), || {})])
    .window(Window::new(&t("Notes")).view(label(|| t_plural(3, "{n} row", "{n} rows"))))
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
- Try another language without changing the system:
  `my-app -AppleLanguages "(de)"`.
- In the bundle, list the languages under `languages` in
  `[package.metadata.ferrokit]`. macOS then shows AppKit's own panels and
  menu items (Close All, the Open panel) in that language too.
