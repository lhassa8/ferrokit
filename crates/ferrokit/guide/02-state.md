# State

State lives in **signals**. Views that read a signal while they build
subscribe to it, and update in place when it changes.

## Signal

```rust,no_run
use ferrokit::*;

let name = Signal::new(String::from("Ada"));
name.set("Grace".into());                 // notifies readers
name.update(|n| n.push_str(" Hopper"));    // changes in place, notifies
let now = name.get();                      // a copy, subscribing the reader
let len = name.with(|n| n.len());          // read without copying
let quiet = name.get_untracked();          // read without subscribing
let chars = name.with_untracked(|n| n.chars().count()); // the same, without copying
# let _ = (now, len, quiet, chars);
```

`set_if_changed` skips the notification when the value is equal.

## Handles and owners

`Signal` and `Memo` are `Copy` handles: closures capture them as they are,
with no cloning. Each belongs to the **owner** that was current when it was
made, and is freed with it:

- **A window's view tree:** signals made while its view builds, by its
  effects, and by its buttons' actions (a sheet's form, say) are freed when
  the window closes.
- **A `dynamic` branch or a `for_each` row:** freed when the branch is
  replaced or the row removed.
- **A document:** its content, path and edited state go with it.

Signals made outside any owner, such as app state created in `main`, live as
long as the app. Reading a freed signal panics with a message saying so;
setting it does nothing, and `try_get` and `is_alive` check first.
`live_signals()` counts the signals in existence, for leak checks.

## Values in views: fixed, or from signals

What a view shows takes either a value or a closure that reads signals:
`text("Total")` or `text(move || format!("{} left", left.get()))`, and the
same for `.color(..)`, `.background(..)`, `.opacity(..)`, `.disabled(..)`, `.hidden(..)`,
a button's title and symbol, and a menu item's title. A closure runs again
when a signal it read changes, and only that view updates. (These take
`impl IntoValue<T>`.)

```rust,no_run
use ferrokit::*;

let done = Signal::new(false);
let row = text("Boots")
    .padding(8.0)
    .background(move || if done.get() { Color::Green.opacity(0.18) } else { Color::Clear })
    .opacity(move || if done.get() { 0.5 } else { 1.0 });
let toggle = button(move || if done.get() { "Unpack".to_string() } else { "Pack".to_string() }, move || done.set(!done.get()));
# let _ = (row, toggle);
```

## Memo: derived state

A memo recomputes when its inputs change, and notifies its own readers only
when its value actually changes.

```rust,no_run
use ferrokit::*;

let items = Signal::new(vec![3, 1, 4, 1, 5]);
let total = Memo::new(move || items.with(|v| v.iter().sum::<i32>()));
let view = text(move || format!("Total: {}", total.get()));
# let _ = view;
```

## Effect: reacting to changes

An effect runs now, then again whenever a signal it read changes. It stops
when its handle is dropped, so keep it.

```rust,no_run
use ferrokit::*;

let theme = Signal::new(Appearance::System);
let _keep = Effect::new(move || set_appearance(theme.get()));
```

Inside a view tree, `watch(…)` does the same and lives as long as its place
in the tree.

## Batching

Event handlers (button actions, field edits, menu items) already run in a
**batch**: effects and layout run once at the end, however many signals
changed. Use `batch` yourself when you change several signals from
elsewhere:

```rust,no_run
use ferrokit::*;

let (first, last) = (Signal::new(String::new()), Signal::new(String::new()));
batch(|| {
    first.set("Ada".into());
    last.set("Lovelace".into());
});
```

`untrack(|| …)` reads signals without subscribing the running effect.

## Background work

`spawn(work, then)` runs `work` on a background thread and `then` with its
result on the main thread, inside a batch.

```rust,no_run
use ferrokit::*;

let rows = Signal::new(Vec::<String>::new());
spawn(
    || std::fs::read_to_string("/etc/hosts").unwrap_or_default().lines().map(String::from).collect::<Vec<_>>(),
    move |lines| rows.set(lines),
);
```

`later(f)` runs `f` on the next pass of the main run loop.

## Async tasks

`spawn_local(future)` runs a future on the main thread. Between its
`.await`s it can read and set signals as an action does. Each poll is a
batch. Inside a task:

- `background(|| …).await` runs blocking work on another thread (it's
  `spawn` as a future);
- `sleep(duration).await` pauses without blocking anything;
- `alert_async`, `open_panel_async`, `save_panel_async` and `sheet_async`
  show a sheet and give its answer.

```rust,no_run
use ferrokit::*;
use std::time::Duration;

let status = Signal::new(String::new());
let save = button("Save", move || {
    spawn_local(async move {
        status.set("Saving…".into());
        let written = background(|| std::fs::write("/tmp/notes.txt", "…")).await;
        status.set(match written {
            Ok(()) => "Saved".into(),
            Err(e) => format!("Couldn't save: {e}"),
        });
        sleep(Duration::from_secs(2)).await;
        status.set(String::new());
    });
});
# let _ = save;
```

- A task belongs to the owner current when it starts, as a signal does.
  Closing its window, or removing its view, drops the future: the task stops
  at the `.await` it's waiting at. `spawn_local` returns a `Task`:
  `cancel()` stops it, `detach()` keeps it running after its owner goes.
- Tasks keep running while a menu is open or a window is being resized.
- A panic in a task, or in `background` work it awaits, is reported (see
  `App::on_error`) and ends that task only.
- Ferrokit has no I/O runtime. Libraries such as reqwest and sqlx need
  their own (tokio): see *tokio, reqwest and sqlx* below.

### Awaiting a sheet

The callback-taking dialogs have awaitable versions, so a flow of
questions reads top to bottom:

```rust,no_run
use ferrokit::*;

fn export(window: Retained<NSWindow>, csv: String, status: Signal<String>) {
    spawn_local(async move {
        let Some(path) = save_panel_async(&window, "export.csv", None).await else { return };
        if path.exists() {
            let ask = Alert::new("Replace “export.csv”?", "A file with that name exists.").button("Replace").button("Cancel");
            if alert_async(&window, ask).await != Some(0) {
                return;
            }
        }
        match background(move || std::fs::write(&path, csv)).await {
            Ok(()) => status.set("Exported".into()),
            Err(e) => status.set(format!("Couldn't export: {e}")),
        }
    });
}
# let _ = export;
```

- `alert_async` gives the index of the button chosen, or `None` if the
  alert ended without one (its window closed) and it has no Cancel button.
- `open_panel_async` and `save_panel_async` give the paths, or `None` for
  Cancel.
- `sheet_async(&window, |reply| Window::new(..).view(..))` shows your own
  sheet and gives what its buttons send: `reply.send(value)` closes it with
  a value, `reply.cancel()` with `None`.
- If the task is cancelled while it waits (its window closed,
  `Task::cancel`), the sheet closes.

### tokio, reqwest and sqlx

Ferrokit has no I/O runtime and doesn't depend on one. Keep a tokio runtime
in your app for the I/O, and with Ferrokit's `tokio` feature
(`ferrokit = { version = "…", features = ["tokio"] }`) await work on it with
`on_runtime`:

```rust,ignore
let rt = tokio::runtime::Runtime::new()?; // kept for the app's lifetime
let handle = rt.handle().clone();
spawn_local(async move {
    match on_runtime(&handle, async move { reqwest::get(url).await?.text().await }).await {
        Ok(body) => page.set(body),
        Err(e) => error.set(Some(e.to_string())),
    }
});
```

- When the task is cancelled (its window closed), the work on the runtime
  is aborted too.
- A panic on the runtime is reported like one in the task, and ends the
  task: there's no `JoinError` to unwrap.
- Without the feature, `handle.spawn(…).await` works as well: a tokio
  `JoinHandle` is a plain future. It isn't aborted with the task, and its
  result is a `Result<_, JoinError>`.

Don't block inside a task: a poll runs on the main thread, and the UI waits
for it.

## Loading data: `Resource`

A resource loads a value on a background thread from the signals it reads,
and loads again when they change. It's made for search-as-you-type:

- Starting a new load cancels the one running. Its result is dropped, so an
  older query's rows never replace a newer one's.
- The last value stays readable while the next loads, so a table doesn't
  flash empty between keystrokes.

```rust,no_run
use ferrokit::*;

let query = Signal::new(String::new());
let results = Resource::new(move || query.get(), |q: String, cancel: &Cancel| {
    let mut rows = Vec::new();
    for i in 0..1_000_000u32 {
        if i % 10_000 == 0 && cancel.is_cancelled() {
            break; // the user typed again; this result won't be shown
        }
        if i.to_string().contains(&q) {
            rows.push(i);
        }
    }
    Ok::<_, String>(rows)
});
let view = vstack((
    search_field(&query),
    text(move || match results.state() {
        Load::Loading => "Searching…".into(),
        Load::Ready(rows) => format!("{} matches", rows.len()),
        Load::Failed(error) => format!("Search failed: {error}"),
    }),
));
# let _ = view;
```

- `get()` is the latest value (`None` before the first load); `loading()`
  is whether a load is running; `error()` is the latest failure's text.
  `state()` combines them.
- An error keeps the last value: `get()` still returns it.
- `refetch()` loads again with the same inputs.
- A resource is owned like a signal: closing its window cancels its load.

`Resource::new_async` takes a load that returns a future instead. It runs on
the main thread as a task, so it needn't be `Send`, and a newer load drops
the older one's future at its `.await` (no `Cancel` to check):

```rust,no_run
use ferrokit::*;
use std::time::Duration;

let query = Signal::new(String::new());
let results = Resource::new_async(move || query.get(), |q: String| async move {
    sleep(Duration::from_millis(200)).await; // typing on drops this load here
    let words = background(|| std::fs::read_to_string("/usr/share/dict/words")).await.map_err(|e| e.to_string())?;
    Ok::<_, String>(words.lines().filter(|w| w.starts_with(&q)).map(String::from).collect::<Vec<_>>())
});
# let _ = results;
```

The `sleep` at the start debounces: while the user keeps typing, each load
is dropped before it reads anything. Await `on_runtime` in it for network
I/O.

## Stored settings

`stored(key, default)` is a signal backed by the user's defaults: it starts
from the saved value and saves every change.

```rust,no_run
use ferrokit::*;

let sounds = stored("play sounds", true);
let view = checkbox("Play sounds", &sounds);
# let _ = view;
```
