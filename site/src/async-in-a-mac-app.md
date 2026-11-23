# Async code in a native Mac app (tokio, reqwest)

AppKit runs on the main thread, and so does every Ferrokit view. Async code fits in without a second UI thread: `spawn_local` runs a future *on the main thread*, so between its `.await`s it can read and set signals like any button action. The future doesn't have to be `Send`.

```rust,no_run
{{#include ../snippets/src/bin/async_primes.rs}}
```

Inside a task:

- `background(|| …).await` runs blocking work (a big computation, a file read, a synchronous database call) on another thread and gives the result back to the task.
- `sleep(duration).await` waits on a timer in the main run loop, without blocking anything.
- Each poll is a batch: views that read several signals the task sets update once.

## A task stops with its window

A task belongs to the window or view that was current when it started. Close the window, or remove the view, and the future is dropped: the task stops at the `.await` it was waiting at, so a closed window never gets a late update. `spawn_local` returns a `Task` handle:

- `task.cancel()` stops it now (a Cancel button). Work a `background` call already started on its thread runs to the end, and its result is dropped.
- `task.detach()` keeps it running after its owner goes (an upload that should finish when its window is closed).

Tasks keep running while a menu is open or a window is being resized. A panic inside a task, or in the `background` work it awaits, is reported through `App::on_error` and ends only that task.

## Awaiting alerts, panels and sheets

The dialogs that take a callback have awaitable versions, so a flow of questions reads top to bottom:

```rust,ignore
spawn_local(async move {
    let Some(path) = save_panel_async(&window, "export.csv", None).await else { return };
    let ask = Alert::new("Export 10,000 rows?", "This takes a moment.").button("Export").button("Cancel");
    if alert_async(&window, ask).await != Some(0) {
        return;
    }
    background(move || write_csv(&path)).await;
});
```

`alert_async` gives the button's index, `open_panel_async` and `save_panel_async` the paths (`None` for Cancel), and `sheet_async` shows your own sheet and gives what its buttons send. If the task is cancelled while it waits, the sheet closes.

## tokio, reqwest and sqlx

Ferrokit has no I/O runtime of its own, and doesn't depend on one. reqwest, sqlx and most of tokio's I/O need tokio's reactor, so keep a tokio runtime in your app and run the I/O there. With Ferrokit's `tokio` feature, `on_runtime` awaits work on it:

```toml
ferrokit = { version = "0.13", features = ["tokio"] }
```

```rust,ignore
// Needs tokio (rt-multi-thread) and reqwest in Cargo.toml; not compiled on this site.
let rt = tokio::runtime::Runtime::new()?; // kept for the app's lifetime
let handle = rt.handle().clone();

let page = Signal::new(String::new());
let error = Signal::new(None::<String>);
let fetch = button("Fetch", move || {
    let handle = handle.clone();
    spawn_local(async move {
        let url = "https://example.com";
        match on_runtime(&handle, async move { reqwest::get(url).await?.text().await }).await {
            Ok(body) => page.set(body),
            Err(e) => error.set(Some(e.to_string())),
        }
    });
});
```

The request runs on tokio's threads; the result comes back to the main thread, where the task sets the signals. If the window closes first, the task is dropped and the request is aborted on tokio too. A panic on the runtime is reported through `App::on_error` and ends the task, so there's no `JoinError` to unwrap.

Without the feature, `handle.spawn(…).await` works as well (a `JoinHandle` is an ordinary future), but it isn't aborted with the task and gives a `Result<_, JoinError>`.

## Async loading

`Resource::new_async` reloads a value when the signals it reads change, with an async load: the newest load wins, and an older one is dropped at its `.await`. A `sleep` at the start of the load debounces search-as-you-type.

An app in the App Sandbox needs `network = true` under `[package.metadata.ferrokit]` to make outgoing connections.

## What not to do

- Don't block inside a task: each poll runs on the main thread, and the UI waits for it. Blocking work goes in `background`.
- Don't call `block_on` on the main thread: it freezes the UI.

More in the guide: [Async tasks](guide/02-state.md#async-tasks), and [Loading data](guide/02-state.md#loading-data-resource) for `Resource`. The design is in [issue #2](https://github.com/lhassa8/ferrokit/issues/2).
