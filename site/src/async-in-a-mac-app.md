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

## tokio, reqwest and sqlx

Ferrokit has no I/O runtime of its own, and doesn't depend on one. reqwest, sqlx and most of tokio's I/O need tokio's reactor, so keep a tokio runtime in your app, spawn the I/O there, and await the join handle from the task. A `JoinHandle` is an ordinary future, so no glue is needed:

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
        match handle.spawn(async move { reqwest::get(url).await?.text().await }).await {
            Ok(Ok(body)) => page.set(body),
            Ok(Err(e)) => error.set(Some(e.to_string())),
            Err(e) => error.set(Some(e.to_string())), // the runtime task panicked
        }
    });
});
```

The request runs on tokio's threads; the result comes back to the main thread, where the task sets the signals. If the window closes first, the task is dropped and the result is ignored (the request itself still finishes on tokio unless you abort its handle).

An app in the App Sandbox needs `network = true` under `[package.metadata.ferrokit]` to make outgoing connections.

## What not to do

- Don't block inside a task: each poll runs on the main thread, and the UI waits for it. Blocking work goes in `background`.
- Don't call `block_on` on the main thread: it freezes the UI.

More in the guide: [Async tasks](guide/02-state.md#async-tasks), and [Live data](guide/07-live-data.md) for `Resource`, which reloads a value on a background thread when the signals it reads change. Async resources and awaitable panels and alerts are being designed in [issue #2](https://github.com/lhassa8/ferrokit/issues/2): say what you'd want there.
