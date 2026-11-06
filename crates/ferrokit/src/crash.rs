//! Crash reports: a panic that Ferrokit doesn't catch (one outside the
//! places AppKit calls in, or on a background thread) is written to a
//! report before the app ends, and handed to the app at its next launch
//! (see [`App::crash_reports`](crate::App::crash_reports)).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

thread_local! {
    /// How deep the current thread is inside Ferrokit's catching of
    /// failures: a panic there is reported, not a crash.
    static CATCHING: Cell<u32> = const { Cell::new(0) };
}

/// Runs `f` with panics in it counted as caught (not crashes).
pub(crate) fn catching<R>(f: impl FnOnce() -> R) -> R {
    struct Out;
    impl Drop for Out {
        fn drop(&mut self) {
            let _ = CATCHING.try_with(|c| c.set(c.get().saturating_sub(1)));
        }
    }
    CATCHING.with(|c| c.set(c.get() + 1));
    let _out = Out;
    f()
}

fn caught_here() -> bool {
    CATCHING.try_with(Cell::get).unwrap_or(0) > 0
}

/// A report of a crash in an earlier run (see
/// [`App::crash_reports`](crate::App::crash_reports)).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CrashReport {
    /// The report's file.
    pub path: PathBuf,
    /// What it says: the app and its version, macOS, when, the thread, the
    /// panic's message and place, and the backtrace.
    pub text: String,
    /// When the app crashed.
    pub when: Option<SystemTime>,
}

impl CrashReport {
    /// Moves the report to the `Sent` folder beside it, so it isn't handed
    /// to the app again: call it once it's sent (or the user declined).
    pub fn mark_sent(&self) -> std::io::Result<()> {
        let dir = self.path.parent().ok_or_else(|| std::io::Error::other("a report without a folder"))?.join("Sent");
        std::fs::create_dir_all(&dir)?;
        std::fs::rename(&self.path, dir.join(self.path.file_name().unwrap_or_default()))
    }
}

/// Where an app called `name` keeps its reports: `~/Library/Logs/<name>/Crashes`
/// (inside its container when sandboxed), or `FERROKIT_CRASH_DIR` if set.
pub(crate) fn folder(name: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("FERROKIT_CRASH_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map_or_else(std::env::temp_dir, PathBuf::from);
    home.join("Library/Logs").join(name).join("Crashes")
}

/// Writes a report for each panic that isn't caught, from now on.
pub(crate) fn install(name: &str) {
    let dir = folder(name);
    let header = header(name);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        if caught_here() {
            return;
        }
        let message = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into());
        let place = info.location().map_or_else(String::new, |l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let thread = std::thread::current();
        let thread = match thread.name() {
            Some("main") => "the main thread".to_string(),
            Some(n) => format!("thread {n}"),
            None => "a background thread".to_string(),
        };
        let when = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let text = format!(
            "{header}Crashed at: {when} (seconds since 1970)\nThread: {thread}\nPanic: {message}\nAt: {place}\n\nBacktrace:\n{}\n",
            std::backtrace::Backtrace::force_capture()
        );
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(format!("{when}-{}.crash.txt", std::process::id())), text);
    }));
}

/// The app, its version and macOS, read once at install (not in the hook).
fn header(name: &str) -> String {
    use objc2_foundation::{NSBundle, NSProcessInfo, NSString};
    let info = NSBundle::mainBundle().infoDictionary();
    let get = |key: &str| {
        info.as_ref().and_then(|i| i.objectForKey(&NSString::from_str(key))).and_then(|v| v.downcast::<NSString>().ok()).map(|s| s.to_string())
    };
    let version = match (get("CFBundleShortVersionString"), get("CFBundleVersion")) {
        (Some(v), Some(b)) if v != b => format!("{v} ({b})"),
        (Some(v), _) => v,
        _ => "(not bundled)".to_string(),
    };
    let id = get("CFBundleIdentifier").map_or_else(String::new, |id| format!(" ({id})"));
    let macos = NSProcessInfo::processInfo().operatingSystemVersionString().to_string();
    format!("App: {name} {version}{id}\nmacOS: {macos}\nFerrokit: {}\n", env!("CARGO_PKG_VERSION"))
}

/// The reports in `dir` not yet marked sent, oldest first.
pub(crate) fn pending(dir: &Path) -> Vec<CrashReport> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut reports: Vec<CrashReport> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".crash.txt"))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let when = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.split('-').next())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|s| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s));
            Some(CrashReport { path, text, when })
        })
        .collect();
    reports.sort_by(|a, b| a.path.cmp(&b.path));
    reports
}
