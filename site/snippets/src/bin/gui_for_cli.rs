// The app half of "A native Mac GUI for an existing Rust CLI". In a real
// project, `tally` is the crate's own library; here it's a stand-in module.
mod tally {
    #[derive(Clone, PartialEq)]
    pub struct Entry {
        pub project: String,
        pub minutes: u32,
    }

    pub fn path() -> std::path::PathBuf {
        std::env::var_os("TALLY_FILE").map(Into::into).unwrap_or_else(|| "entries.txt".into())
    }

    /// The library's own async loader, as the CLI calls it.
    pub async fn load() -> std::io::Result<Vec<Entry>> {
        let text = match tokio::fs::read_to_string(path()).await {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        Ok(text
            .lines()
            .filter_map(|l| l.split_once(','))
            .map(|(p, m)| Entry { project: p.to_string(), minutes: m.trim().parse().unwrap_or(0) })
            .collect())
    }
}

use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use ferrokit::*;

/// The tokio runtime the library's async functions run on. `main` belongs
/// to AppKit, so it can't be `#[tokio::main]`.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("a tokio runtime"))
}

fn modified() -> Option<SystemTime> {
    std::fs::metadata(tally::path()).and_then(|m| m.modified()).ok()
}

fn main() {
    // Before the app starts, blocking on the library is fine.
    let entries = Signal::new(runtime().block_on(tally::load()).unwrap_or_default());

    let model = Rc::new(TableModel::new(
        vec![
            Column::text("project", "Project", |e: &tally::Entry| e.project.clone()).width(240.0),
            Column::number("minutes", "Minutes", |e: &tally::Entry| f64::from(e.minutes)),
        ],
    ));
    let m = model.clone();

    App::new("Tally")
        .window(Window::new("Tally").size(420.0, 360.0).view(
            vstack((
                text(move || format!("{} entries", entries.with(Vec::len))).font(Font::headline()),
                table(&model).accessibility_label("Entries"),
                watch(move || m.set_rows(entries.get())),
            ))
            .padding(12.0),
        ))
        .on_launch(move || {
            // The CLI may change the file while the window is open: look
            // once a second, read it off the main thread, and set the
            // signal only when the entries differ (set_rows clears the
            // table's selection).
            spawn_local(async move {
                let mut seen = modified();
                loop {
                    sleep(Duration::from_secs(1)).await;
                    if modified() == seen {
                        continue;
                    }
                    seen = modified();
                    if let Ok(fresh) = background(|| runtime().block_on(tally::load())).await
                        && fresh != entries.get_untracked()
                    {
                        entries.set(fresh);
                    }
                }
            })
            .detach();
        })
        .run();
}
