# Live data

Data that arrives faster than people read it (a log tail, telemetry, a
download list) should be applied **once per display frame**, not once per
item. `on_frame` gives you that.

```rust,no_run
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use appkit_ui::*;

// Lines from a background reader, applied each frame.
let incoming: Arc<Mutex<Vec<String>>> = Arc::default();
let model = Rc::new(TableModel::new(ViewId(1), vec![Column::text("line", "Line", |l: &String| l.clone())]));

let writer = incoming.clone();
std::thread::spawn(move || loop {
    writer.lock().unwrap().push(format!("event at {:?}", std::time::Instant::now()));
    std::thread::sleep(std::time::Duration::from_millis(1));
});

let m = model.clone();
App::new("Tail")
    .window(Window::new("Tail").size(600.0, 400.0).view(table(&model)))
    .on_launch(move || {
        let m = m.clone();
        let incoming = incoming.clone();
        let frames = on_frame(&windows()[0], move |_time| {
            let new = std::mem::take(&mut *incoming.lock().unwrap());
            if new.is_empty() {
                return;
            }
            let follow = m.is_at_end();
            m.append(new);            // cost doesn't depend on the rows already there
            if follow {
                m.scroll_to_end();    // keep following unless the user scrolled up
            }
        });
        std::mem::forget(frames);     // keep it for the app's life (dropping it stops the calls)
    })
    .run();
```

- `on_frame(window, f)` calls `f` once per frame of the window's display
  (60 or 120 Hz), inside a batch, with the frame's target time. While the
  display isn't drawing (asleep, the screen locked) a 10 Hz timer stands in,
  so data keeps flowing in small steps.
- Keep the returned `FrameLoop`; dropping it stops the calls.
- `TableModel::append` adds rows without touching the existing ones;
  `is_at_end` and `scroll_to_end` implement "follow unless scrolled up".

The `log_viewer` example follows a 1,000,000-line file this way: new lines
cost under 1 ms per frame at 10,000 lines a second, and filtering 1.26M
lines takes about 70 ms.
