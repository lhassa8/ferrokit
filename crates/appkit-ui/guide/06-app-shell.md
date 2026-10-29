# The app shell

## Menus

Every app gets the standard menu bar. Add your own menus with `App::menu`;
a menu named "File" is merged above the standard Close item.

```rust,no_run
use appkit_ui::*;

let items = Signal::new(0);
let show_sidebar = Signal::new(true);
let (add, remove, has) = (items.clone(), items.clone(), items.clone());
let (s1, s2) = (show_sidebar.clone(), show_sidebar.clone());
App::new("Notes")
    .menu("File", vec![
        menu_item("New Note", move || add.update(|n| *n += 1)).key("n"),                // ⌘N
        menu_item("Delete Note", move || remove.update(|n| *n -= 1))
            .key_with("n", &[Modifier::Option])                                      // ⌥⌘N
            .enabled_when(move || has.get() > 0),                                     // greyed out at 0
    ])
    .menu("View", vec![
        menu_item("Show Sidebar", move || s1.update(|s| *s = !*s))
            .key_with("s", &[Modifier::Control])
            .checked_when(move || s2.get()),                                         // ✓ when shown
        menu_separator(),
        submenu("Sort By", vec![menu_item("Name", || {}), menu_item("Date", || {})]),
    ])
    .window(Window::new("Notes"))
    .run();
```

`enabled_when` and `checked_when` are evaluated when the menu opens or a
shortcut is pressed.

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
let (a, b) = (count.clone(), count.clone());
undoable("Add Item", move || a.update(|n| *n += 1), move || b.update(|n| *n -= 1));
```

Steps go to the frontmost window's history (`undoable_in` names the
window). Closing a window clears its history.

## Windows, appearance, quitting

- `open(window)` opens a window; the close button, ⌘W and `close_window`
  tear it down.
- `set_appearance(Appearance::Dark)` pins the app's look; `System` follows
  the system setting. `App::on_event` delivers `AppEvent::AppearanceChanged`.
- `quit()` ends the app, as ⌘Q does.
