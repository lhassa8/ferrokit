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

## Memo: derived state

A memo recomputes when its inputs change, and notifies its own readers only
when its value actually changes.

```rust,no_run
use appkit_ui::*;

let items = Signal::new(vec![3, 1, 4, 1, 5]);
let i2 = items.clone();
let total = Memo::new(move || i2.with(|v| v.iter().sum::<i32>()));
let view = label(move || format!("Total: {}", total.get()));
# let _ = view;
```

## Effect: reacting to changes

An effect runs now, then again whenever a signal it read changes. It stops
when its handle is dropped, so keep it.

```rust,no_run
use appkit_ui::*;

let theme = Signal::new(Appearance::System);
let t = theme.clone();
let _keep = Effect::new(move || set_appearance(t.get()));
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
let r = rows.clone();
spawn(
    || std::fs::read_to_string("/etc/hosts").unwrap_or_default().lines().map(String::from).collect::<Vec<_>>(),
    move |lines| r.set(lines),
);
```

`later(f)` runs `f` on the next pass of the main run loop.

## Stored settings

`stored(key, default)` is a signal backed by the user's defaults: it starts
from the saved value and saves every change.

```rust,no_run
use appkit_ui::*;

let sounds = stored("play sounds", true);
let view = checkbox("Play sounds", &sounds);
# let _ = view;
```
