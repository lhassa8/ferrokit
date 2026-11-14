use ferrokit::*;
use std::time::Duration;

fn is_prime(n: u32) -> bool {
    n >= 2 && (2..).take_while(|d| d * d <= n).all(|d| !n.is_multiple_of(d))
}

fn main() {
    let status = Signal::new(String::from("Ready"));
    let running: Signal<Option<Task>> = Signal::new(None);

    let start = button("Count primes", move || {
        let task = spawn_local(async move {
            status.set("Counting…".into());
            // Blocking work runs on another thread; the window stays responsive.
            let n = background(|| (2..5_000_000u32).filter(|&k| is_prime(k)).count()).await;
            running.set(None);
            status.set(format!("{n} primes below five million"));
            // A timer on the main run loop: nothing is blocked while it waits.
            sleep(Duration::from_secs(5)).await;
            status.set("Ready".into());
        });
        running.set(Some(task));
    });
    let cancel = button("Cancel", move || {
        if let Some(task) = running.get() {
            task.cancel();
            running.set(None);
            status.set("Cancelled".into());
        }
    });

    App::new("Primes")
        .window(Window::new("Primes").size(360.0, 140.0).view(
            vstack((text(move || status.get()), hstack((start, cancel)))).padding(20.0),
        ))
        .run();
}
