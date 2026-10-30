# State

State lives in **signals**. Views that read a signal while they build
subscribe to it, and update in place when it changes.

## Signal

```rust,no_run
use appkit_ui::*;

let name = Signal::new(String::from("Ada"));
name.set("Grace".into());                 // notifies readers
name.update(|n| n.push_str(" Hopper"));    // changes in place, notifies
let now = name.get();                      // a copy, subscribing the reader
let len = name.with(|n| n.len());          // read without copying
let quiet = name.get_untracked();          // read without subscribing
# let _ = (now, len, quiet);
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

## Memo: derived state

A memo recomputes when its inputs change, and notifies its own readers only
when its value actually changes.

```rust,no_run
use appkit_ui::*;

let items = Signal::new(vec![3, 1, 4, 1, 5]);
let total = Memo::new(move || items.with(|v| v.iter().sum::<i32>()));
let view = label(move || format!("Total: {}", total.get()));
# let _ = view;
```

## Effect: reacting to changes

An effect runs now, then again whenever a signal it read changes. It stops
when its handle is dropped, so keep it.

```rust,no_run
use appkit_ui::*;

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
use appkit_ui::*;

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
use appkit_ui::*;

let rows = Signal::new(Vec::<String>::new());
spawn(
    || std::fs::read_to_string("/etc/hosts").unwrap_or_default().lines().map(String::from).collect::<Vec<_>>(),
    move |lines| rows.set(lines),
);
```

`later(f)` runs `f` on the next pass of the main run loop.

## Loading data: `Resource`

A resource loads a value on a background thread from the signals it reads,
and loads again when they change. It's made for search-as-you-type:

- Starting a new load cancels the one running. Its result is dropped, so an
  older query's rows never replace a newer one's.
- The last value stays readable while the next loads, so a table doesn't
  flash empty between keystrokes.

```rust,no_run
use appkit_ui::*;

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
    label(move || match results.state() {
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

## Stored settings

`stored(key, default)` is a signal backed by the user's defaults: it starts
from the saved value and saves every change.

```rust,no_run
use appkit_ui::*;

let sounds = stored("play sounds", true);
let view = checkbox("Play sounds", &sounds);
# let _ = view;
```
