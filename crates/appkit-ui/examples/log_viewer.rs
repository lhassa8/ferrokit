//! F3.5 exit check: a log viewer that follows a growing file. Open a log,
//! filter it by text and level, and it keeps up with new lines: they're read
//! on a background thread and added once per display frame, and the table
//! follows the end until you scroll up.
//! `cargo run --release -p appkit-ui --example log_viewer [file]`; `CHECK=all`
//! runs its self-checks and measurements (a 1,000,000-line live tail);
//! `CHECK=follow <file>` follows a real log for 10 s and checks it (e.g.
//! `log stream --style compact > /tmp/sys.log`).

use std::cell::RefCell;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use appkit_ui::*;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

/// One log line: "<time> <LEVEL> <message>".
struct Line {
    text: Box<str>,
    level: Level,
    time_end: u16,
    message_start: u16,
}

impl Line {
    /// Parses "<time> <level> <message>". The time runs to the first word
    /// that is a clock time ("2026-10-28T12:00:00.000", "2026-10-29
    /// 08:47:54.561", "Oct 29 08:47:54"); the level is a word such as ERROR,
    /// warn, [E] or `log stream`'s Df/E/F codes, or missing (Info). A line
    /// starting with whitespace continues the one before and keeps its level.
    fn parse(text: &str, previous: Level) -> Line {
        let short = |n: usize| u16::try_from(n).unwrap_or(0);
        if text.starts_with(char::is_whitespace) {
            return Line { text: text.into(), level: previous, time_end: 0, message_start: 0 };
        }
        // Word boundaries: (start, end) of the next word at or after `from`.
        let word = |from: usize| {
            let start = from + text[from..].len() - text[from..].trim_start().len();
            (start, text[start..].find(' ').map_or(text.len(), |e| start + e))
        };
        let mut time_end = 0;
        let mut at = 0;
        for _ in 0..3 {
            let (start, end) = word(at);
            let w = &text[start..end];
            if w.contains(':') && w.starts_with(|c: char| c.is_ascii_digit()) {
                time_end = end;
                break;
            }
            at = end;
        }
        let (level_start, level_end) = word(time_end);
        let level = level_of(text[level_start..level_end].trim_matches(['[', ']', ':', '<', '>']));
        let message_start = match level {
            Some(_) => word(level_end).0,
            None => word(time_end).0,
        };
        Line { text: text.into(), level: level.unwrap_or(Level::Info), time_end: short(time_end), message_start: short(message_start) }
    }

    fn time(&self) -> &str {
        &self.text[..self.time_end as usize]
    }

    fn message(&self) -> &str {
        self.text.get(self.message_start as usize..).unwrap_or(&self.text)
    }
}

/// A level word, in any case: the usual names and `log stream`'s codes
/// (Df default, I info, Db debug, E error, F fault, A activity).
fn level_of(w: &str) -> Option<Level> {
    let is = |names: &[&str]| names.iter().any(|n| w.eq_ignore_ascii_case(n));
    if is(&["ERROR", "ERR", "E", "FATAL", "F", "FAULT", "CRITICAL", "CRIT"]) {
        Some(Level::Error)
    } else if is(&["WARN", "WARNING", "W"]) {
        Some(Level::Warn)
    } else if is(&["DEBUG", "DB", "D", "TRACE"]) {
        Some(Level::Debug)
    } else if is(&["INFO", "I", "DF", "DEFAULT", "NOTICE", "A"]) {
        Some(Level::Info)
    } else {
        None
    }
}

/// The filter: lines at or above `level` whose message contains `query`
/// (ASCII case-insensitive; `query` already lowercase).
fn matches(line: &Line, query: &[u8], level: Level) -> bool {
    if line.level < level {
        return false;
    }
    let hay = line.message().as_bytes();
    query.is_empty() || hay.len() >= query.len() && hay.windows(query.len()).any(|w| w.eq_ignore_ascii_case(query))
}

/// Follows a file from a background thread: whole lines go to `pending`.
struct Tail {
    stop: Arc<AtomicBool>,
}

impl Tail {
    fn start(path: PathBuf, pending: Arc<Mutex<Vec<Line>>>) -> Tail {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            let Ok(mut file) = std::fs::File::open(&path) else { return };
            let (mut offset, mut partial, mut level) = (0u64, Vec::new(), Level::Info);
            let mut chunk = vec![0u8; 8 << 20];
            while !stopped.load(Ordering::Relaxed) {
                let len = file.metadata().map_or(0, |m| m.len());
                if len < offset {
                    // Truncated or rotated: start over.
                    (offset, partial) = (0, Vec::new());
                    let _ = file.seek(SeekFrom::Start(0));
                }
                let n = file.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                offset += n as u64;
                partial.extend_from_slice(&chunk[..n]);
                let Some(last) = partial.iter().rposition(|&b| b == b'\n') else { continue };
                let lines: Vec<Line> = String::from_utf8_lossy(&partial[..last])
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| {
                        let line = Line::parse(l, level);
                        level = line.level;
                        line
                    })
                    .collect();
                partial.drain(..=last);
                pending.lock().unwrap().extend(lines);
            }
        });
        Tail { stop }
    }
}

impl Drop for Tail {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone)]
struct Viewer {
    lines: Rc<RefCell<Vec<Line>>>,
    /// Shown rows: indices into `lines`.
    table: Rc<TableModel<u32>>,
    query: Signal<String>,
    level: Signal<Level>,
    follow: Signal<bool>,
    total: Signal<usize>,
    shown: Signal<usize>,
    path: Signal<Option<PathBuf>>,
    pending: Arc<Mutex<Vec<Line>>>,
    tail: Rc<RefCell<Option<Tail>>>,
    frames: Rc<RefCell<Option<FrameLoop>>>,
    /// Per frame that added lines: (when, lines in total, microseconds spent).
    log: Rc<RefCell<Vec<(Instant, usize, f64)>>>,
    effects: Rc<RefCell<Vec<Effect>>>,
}

impl Viewer {
    fn new() -> Viewer {
        let lines: Rc<RefCell<Vec<Line>>> = Rc::default();
        let (l1, l2, l3) = (lines.clone(), lines.clone(), lines.clone());
        let table = Rc::new(TableModel::new(
            ViewId(1),
            vec![
                Column::number("n", "#", |i: &u32| *i as f64 + 1.0).width(80.0).sortable(false),
                Column::text("time", "Time", move |i: &u32| l1.borrow()[*i as usize].time().to_string()).width(170.0).sortable(false),
                Column::text("level", "Level", move |i: &u32| format!("{:?}", l2.borrow()[*i as usize].level).to_uppercase())
                    .width(70.0)
                    .sortable(false),
                Column::text("message", "Message", move |i: &u32| l3.borrow()[*i as usize].message().to_string()).width(600.0).sortable(false),
            ],
        ));
        let v = Viewer {
            lines,
            table,
            query: Signal::new(String::new()),
            level: Signal::new(Level::Debug),
            follow: Signal::new(true),
            total: Signal::new(0),
            shown: Signal::new(0),
            path: Signal::new(None),
            pending: Arc::default(),
            tail: Rc::default(),
            frames: Rc::default(),
            log: Rc::default(),
            effects: Rc::default(),
        };
        // Changing the filter rebuilds the shown rows.
        let w = v.clone();
        let refilter = Effect::new(move || {
            let (query, level) = (w.query.get().to_lowercase(), w.level.get());
            untrack(|| w.refilter(&query, level));
        });
        v.effects.borrow_mut().push(refilter);
        v
    }

    fn refilter(&self, query: &str, level: Level) {
        let rows: Vec<u32> = self
            .lines
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, l)| matches(l, query.as_bytes(), level))
            .map(|(i, _)| i as u32)
            .collect();
        self.shown.set(rows.len());
        self.table.set_rows(rows);
        if self.follow.get_untracked() {
            self.table.scroll_to_end();
        }
    }

    /// Starts following `path` (replacing what was shown).
    fn open(&self, path: &Path, window: &objc2_app_kit::NSWindow) {
        self.tail.borrow_mut().take();
        self.pending.lock().unwrap().clear();
        self.lines.borrow_mut().clear();
        self.table.set_rows(Vec::new());
        self.total.set(0);
        self.shown.set(0);
        self.path.set(Some(path.to_path_buf()));
        *self.tail.borrow_mut() = Some(Tail::start(path.to_path_buf(), self.pending.clone()));
        let v = self.clone();
        *self.frames.borrow_mut() = Some(on_frame(window, move |_| v.take_pending()));
    }

    /// Once per frame: adds the lines that arrived since the last one.
    fn take_pending(&self) {
        let new = std::mem::take(&mut *self.pending.lock().unwrap());
        if new.is_empty() {
            return;
        }
        let t = Instant::now();
        let (query, level) = (self.query.get_untracked().to_lowercase(), self.level.get_untracked());
        let at_end = self.table.is_at_end();
        let shown: Vec<u32> = {
            let mut lines = self.lines.borrow_mut();
            let start = lines.len();
            let shown = new.iter().enumerate().filter(|(_, l)| matches(l, query.as_bytes(), level)).map(|(i, _)| (start + i) as u32).collect();
            lines.extend(new);
            shown
        };
        let total = self.lines.borrow().len();
        self.shown.update(|s| *s += shown.len());
        self.table.append(shown);
        self.total.set(total);
        // Follow the end, unless the user scrolled up to read.
        if self.follow.get_untracked() && at_end {
            self.table.scroll_to_end();
        }
        self.log.borrow_mut().push((Instant::now(), total, t.elapsed().as_secs_f64() * 1e6));
    }

    fn choose(&self, window: &objc2_app_kit::NSWindow) {
        let (v, w) = (self.clone(), objc2::rc::Retained::from(window));
        open_panel(window, OpenOptions { extensions: vec![], multiple: false, directories: false }, move |paths| {
            if let Some(p) = paths.and_then(|p| p.into_iter().next()) {
                v.open(&p, &w);
            }
        });
    }
}

fn content(v: &Viewer) -> impl View {
    let (v1, v2) = (v.clone(), v.clone());
    vstack((
        table(&v.table).row_drag(),
        hstack((
            picker(&v.level, vec![("All levels", Level::Debug), ("Info and up", Level::Info), ("Warnings and up", Level::Warn), ("Errors", Level::Error)]).accessibility_label("Level"),
            checkbox("Follow", &v.follow),
            spacer(),
            label(move || {
                let name = v1.path.with(|p| p.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()));
                format!("{}  ·  {} of {} lines", name.unwrap_or_else(|| "No file".into()), v2.shown.get(), v2.total.get())
            }),
        ))
        .padding(8.0),
    ))
}

fn main() {
    let v = Viewer::new();
    let (v1, v2) = (v.clone(), v.clone());
    let check = std::env::var("CHECK").unwrap_or_default();
    let arg = std::env::args().nth(1).map(PathBuf::from);
    App::new("Log Viewer")
        .window(
            Window::new("Log")
                .size(1100.0, 640.0)
                .remember_frame("log")
                .toolbar(vec![
                    toolbar_button("open", "Open", "folder", move || v1.choose(&windows()[0])),
                    toolbar_space(),
                    toolbar_search("search", &v.query),
                ])
                .view(content(&v)),
        )
        .on_launch(move || {
            if check == "all" {
                self_checks::run(v2.clone());
            } else if let (Some(p), "follow") = (&arg, check.as_str()) {
                self_checks::follow(v2.clone(), p);
            } else if let Some(p) = &arg {
                v2.open(p, &windows()[0]);
            }
        })
        .run();
}

// ---- self-checks: a 1,000,000-line live tail ----

mod self_checks {
    use std::cell::Cell;
    use std::io::Write;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use appkit_ui::testing::{footprint_mb, peak_footprint_mb};

    thread_local! {
        static FAILED: Cell<u32> = const { Cell::new(0) };
    }

    fn check(name: &str, pass: bool, note: String) {
        eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
        if !pass {
            FAILED.with(|f| f.set(f.get() + 1));
        }
    }

    /// Line `n` of the generated log (deterministic: 2% errors, 6% warnings).
    fn line(n: usize) -> String {
        let level = match n % 50 {
            0 => "ERROR",
            1..=3 => "WARN",
            _ if n.is_multiple_of(7) => "DEBUG",
            _ => "INFO",
        };
        let ms = n % 86_400_000;
        format!(
            "2026-10-28T{:02}:{:02}:{:02}.{:03} {level} worker-{} request {n} took {} ms path=/api/v{}/items\n",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms % 1000,
            n % 8,
            n % 997,
            n % 3
        )
    }

    fn append(path: &Path, from: usize, count: usize) {
        let mut f = std::io::BufWriter::new(std::fs::OpenOptions::new().append(true).create(true).open(path).unwrap());
        for n in from..from + count {
            f.write_all(line(n).as_bytes()).unwrap();
        }
    }

    /// Calls `then` once `ready` holds (checked every 10 ms off the main thread's back).
    fn wait_until(ready: impl Fn() -> bool + 'static, then: impl FnOnce() + 'static) {
        if ready() {
            return then();
        }
        spawn(|| std::thread::sleep(Duration::from_millis(10)), move |()| wait_until(ready, then));
    }

    /// Appends `rate` lines per second in 100 Hz batches for `seconds`, from
    /// line `from`; records (lines written in total, when) per batch.
    fn writer(path: PathBuf, from: usize, rate: usize, seconds: f64, written: Arc<Mutex<Vec<(usize, Instant)>>>, done: Arc<AtomicUsize>) {
        std::thread::spawn(move || {
            let per_batch = rate / 100;
            let batches = (seconds * 100.0) as usize;
            let start = Instant::now();
            let mut n = from;
            for b in 0..batches {
                append(&path, n, per_batch);
                n += per_batch;
                written.lock().unwrap().push((n, Instant::now()));
                let next = start + Duration::from_millis(10 * (b as u64 + 1));
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
            }
            done.store(n, Ordering::SeqCst);
        });
    }

    fn pct(mut v: Vec<f64>, p: f64) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * p).round() as usize]
    }

    struct Phase {
        frames: Vec<f64>,
        costs: Vec<f64>,
        latencies: Vec<f64>,
    }

    /// Frame intervals, per-frame costs and write→shown latencies between
    /// `since` and now.
    fn measure(v: &Viewer, ticks: &[f64], since: Instant, written: &[(usize, Instant)]) -> Phase {
        let log = v.log.borrow();
        let window: Vec<&(Instant, usize, f64)> = log.iter().filter(|(t, _, _)| *t >= since).collect();
        let costs = window.iter().map(|(_, _, us)| us / 1000.0).collect();
        let latencies = written
            .iter()
            .filter_map(|(n, at)| window.iter().find(|(_, total, _)| total >= n).map(|(t, _, _)| t.duration_since(*at).as_secs_f64() * 1e3))
            .collect();
        let frames = ticks.windows(2).map(|w| (w[1] - w[0]) * 1e3).collect();
        Phase { frames, costs, latencies }
    }

    fn report(name: &str, p: &Phase, lines: usize, secs: f64, probe: &FrameLoop) -> (f64, f64, usize, f64) {
        let (display, fallback) = probe.counts();
        eprintln!("[measure] {name}: frame calls so far: {display} from display frames, {fallback} from the stand-in timer (display asleep or window hidden)");
        let period = pct(p.frames.clone(), 0.5);
        let missed = p.frames.iter().filter(|f| **f > period * 1.5).count();
        let (c95, cmax) = (pct(p.costs.clone(), 0.95), p.costs.iter().copied().fold(0.0, f64::max));
        let l95 = pct(p.latencies.clone(), 0.95);
        eprintln!(
            "[measure] {name}: {lines} lines in {secs:.1} s; display frames {} at {period:.2} ms ({:.0} Hz), {missed} late; \
             per-frame cost p50 {:.2} ms, p95 {c95:.2} ms, max {cmax:.2} ms over {} frames with new lines; \
             write→table latency p50 {:.1} ms, p95 {l95:.1} ms",
            p.frames.len() + 1,
            1000.0 / period,
            pct(p.costs.clone(), 0.5),
            p.costs.len(),
            pct(p.latencies.clone(), 0.5),
        );
        (c95, cmax, missed, l95)
    }

    /// Sample lines in the formats the parser knows: (line, time, level, message).
    const FORMATS: &[(&str, &str, Level, &str)] = &[
        ("2026-10-28T12:00:00.000 ERROR worker-1 failed", "2026-10-28T12:00:00.000", Level::Error, "worker-1 failed"),
        ("2026-10-29 08:47:54.561 Df locationd[589:299473] scan", "2026-10-29 08:47:54.561", Level::Info, "locationd[589:299473] scan"),
        ("2026-10-29 08:47:54.562 E  kernel[0:1] oops", "2026-10-29 08:47:54.562", Level::Error, "kernel[0:1] oops"),
        ("2026-10-29 08:47:54.563 F  launchd[1:2] fault", "2026-10-29 08:47:54.563", Level::Error, "launchd[1:2] fault"),
        ("2026-10-29 08:47:54.564 Db app[2:3] detail", "2026-10-29 08:47:54.564", Level::Debug, "app[2:3] detail"),
        ("Oct 29 08:47:54 host sshd[12]: accepted", "Oct 29 08:47:54", Level::Info, "host sshd[12]: accepted"),
        ("12:00:01 [warn] disk almost full", "12:00:01", Level::Warn, "disk almost full"),
        ("12:00:02 warning: deprecated", "12:00:02", Level::Warn, "deprecated"),
        ("no time here at all", "", Level::Info, "no time here at all"),
        ("    at frame 3 (continues an error)", "", Level::Error, "    at frame 3 (continues an error)"),
        ("2026-10-29 08:47:54.565 Df 東京[5:6] ünïcode", "2026-10-29 08:47:54.565", Level::Info, "東京[5:6] ünïcode"),
    ];

    fn check_formats() {
        let wrong: Vec<String> = FORMATS
            .iter()
            .filter_map(|(text, time, level, message)| {
                let l = Line::parse(text, Level::Error);
                (l.time() != *time || l.level != *level || l.message() != *message)
                    .then(|| format!("{text:?} → ({:?}, {:?}, {:?})", l.time(), l.level, l.message()))
            })
            .collect();
        check("formats", wrong.is_empty(), format!("{} sample lines; wrong: {wrong:?}", FORMATS.len()));
    }

    /// `CHECK=follow <file>`: follows a real, growing log (e.g. `log stream`)
    /// for 10 s and checks that every line arrives, parsed, while following.
    pub fn follow(v: Viewer, path: &Path) {
        check_formats();
        v.open(path, &windows()[0]);
        let probe = on_frame(&windows()[0], |_| {});
        let start = Instant::now();
        let v1 = v.clone();
        wait_until(
            move || start.elapsed() > Duration::from_secs(10),
            move || {
                // Lines written so far (whole lines only), then wait for the table to have them.
                let text = std::fs::read(v1.path.get_untracked().unwrap()).unwrap();
                let end = text.iter().rposition(|&b| b == b'\n').map_or(0, |e| e + 1);
                let want = String::from_utf8_lossy(&text[..end]).lines().filter(|l| !l.is_empty()).count();
                let v2 = v1.clone();
                let asked = Instant::now();
                wait_until(
                    move || v2.total.get_untracked() >= want || asked.elapsed() > Duration::from_secs(2),
                    move || {
                        let lines = v1.lines.borrow();
                        let timed = lines.iter().filter(|l| !l.time().is_empty()).count();
                        let by_level = |lv: Level| lines.iter().filter(|l| l.level == lv).count();
                        // log stream's compact style: "<date> <time> <type> ...": the type is word 3.
                        let misparsed = lines
                            .iter()
                            .filter(|l| l.text.starts_with(|c: char| c.is_ascii_digit()))
                            .filter(|l| {
                                let code = l.text.split_whitespace().nth(2).unwrap_or("");
                                level_of(code).is_some_and(|lv| lv != l.level) || l.time().len() != 23
                            })
                            .count();
                        let (display, fallback) = probe.counts();
                        let costs: Vec<f64> = v1.log.borrow().iter().map(|(_, _, us)| us / 1000.0).collect();
                        let cmax = costs.iter().copied().fold(0.0, f64::max);
                        check(
                            "follow",
                            v1.total.get_untracked() >= want && want > 0 && misparsed == 0 && v1.table.is_at_end() && cmax < 8.3,
                            format!(
                                "{} of {want} lines in the file shown after 10 s ({timed} with a time; {} errors, {} warnings, {} debug); \
                                 {misparsed} misparsed; frames: {display} display, {fallback} stand-in; per-frame cost max {cmax:.2} ms; following the end: {}",
                                v1.total.get_untracked(),
                                by_level(Level::Error),
                                by_level(Level::Warn),
                                by_level(Level::Debug),
                                v1.table.is_at_end()
                            ),
                        );
                        drop(lines);
                        // Errors only (outside this batch, so the filter runs now).
                        later(move || {
                            v1.tail.borrow_mut().take();
                            v1.level.set(Level::Error);
                            let errors = v1.table.rows().iter().all(|&i| v1.lines.borrow()[i as usize].level == Level::Error);
                            let want = v1.lines.borrow().iter().filter(|l| l.level == Level::Error).count();
                            check("follow-filter", errors && v1.table.len() == want, format!("Errors only: {} rows (recount {want}), all errors: {errors}", v1.table.len()));
                            let failed = FAILED.with(Cell::get);
                            eprintln!("[check] done, {failed} failed");
                            std::process::exit(if failed == 0 { 0 } else { 1 });
                        });
                    },
                );
            },
        );
    }

    pub fn run(v: Viewer) {
        check_formats();
        let problems = appkit_ui::testing::ax_audit(&windows()[0]);
        check("accessibility-audit", problems.is_empty(), format!("controls VoiceOver can't name: {problems:?}"));
        let dir = std::env::temp_dir().join("appkit-ui-log-check");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.log");
        let base = footprint_mb();

        // 1. A 1,000,000-line log, opened and read to the end.
        let t = Instant::now();
        append(&path, 0, 1_000_000);
        let size = std::fs::metadata(&path).map_or(0, |m| m.len()) as f64 / 1e6;
        eprintln!("[time] wrote 1,000,000 lines ({size:.0} MB) in {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        let w = windows()[0].clone();
        let t = Instant::now();
        v.open(&path, &w);
        // The window's frame ticks, recorded alongside the viewer's own loop.
        let ticks: Rc<RefCell<Vec<f64>>> = Rc::default();
        let tk = ticks.clone();
        let probe = on_frame(&w, move |at| tk.borrow_mut().push(at));
        let v1 = v.clone();
        wait_until(
            move || v1.total.get_untracked() >= 1_000_000,
            move || {
                let load = t.elapsed().as_secs_f64() * 1e3;
                let frames = v.log.borrow().len();
                let worst = v.log.borrow().iter().map(|e| e.2).fold(0.0, f64::max) / 1e3;
                check(
                    "load",
                    v.total.get_untracked() == 1_000_000 && v.table.len() == 1_000_000 && v.table.is_at_end(),
                    format!("1,000,000 lines ({size:.0} MB) shown in {load:.0} ms over {frames} frames (worst frame {worst:.1} ms); following the end: {}", v.table.is_at_end()),
                );
                tail_phase(v, path, ticks, probe, base);
            },
        );
    }

    /// 2. Live tail at 10,000 lines/s for 5 s, then a 100,000 lines/s storm for 2 s.
    fn tail_phase(v: Viewer, path: PathBuf, ticks: Rc<RefCell<Vec<f64>>>, probe: FrameLoop, base: f64) {
        let written = Arc::new(Mutex::new(Vec::new()));
        let done = Arc::new(AtomicUsize::new(0));
        ticks.borrow_mut().clear();
        let since = Instant::now();
        writer(path.clone(), 1_000_000, 10_000, 5.0, written.clone(), done.clone());
        let d = done.clone();
        let v1 = v.clone();
        wait_until(
            move || d.load(Ordering::SeqCst) > 0 && v1.total.get_untracked() >= d.load(Ordering::SeqCst),
            move || {
                let p = measure(&v, &ticks.borrow(), since, &written.lock().unwrap());
                let (c95, cmax, missed, l95) = report("steady tail", &p, 50_000, 5.0, &probe);
                let period = pct(p.frames.clone(), 0.5);
                let last_shown = v.table.is_at_end() && v.table.row_at(v.table.len() - 1) == Some(1_049_999);
                check(
                    "tail",
                    v.total.get_untracked() == 1_050_000 && c95 < 4.0 && cmax < period.min(8.3) && l95 < 3.0 * period + 10.0 && last_shown,
                    format!("1,050,000 lines; frame cost p95 {c95:.2} ms, max {cmax:.2} ms (budget {:.1}: the frame period, at most a 120 Hz frame); latency p95 {l95:.1} ms; {missed} late frames; last line on screen: {last_shown}", period.min(8.3)),
                );
                storm_phase(v, path, ticks, probe, base);
            },
        );
    }

    fn storm_phase(v: Viewer, path: PathBuf, ticks: Rc<RefCell<Vec<f64>>>, probe: FrameLoop, base: f64) {
        let written = Arc::new(Mutex::new(Vec::new()));
        let done = Arc::new(AtomicUsize::new(0));
        ticks.borrow_mut().clear();
        let since = Instant::now();
        writer(path.clone(), 1_050_000, 100_000, 2.0, written.clone(), done.clone());
        let d = done.clone();
        let v1 = v.clone();
        wait_until(
            move || d.load(Ordering::SeqCst) > 0 && v1.total.get_untracked() >= d.load(Ordering::SeqCst),
            move || {
                let p = measure(&v, &ticks.borrow(), since, &written.lock().unwrap());
                let (c95, cmax, missed, l95) = report("storm", &p, 200_000, 2.0, &probe);
                let period = pct(p.frames.clone(), 0.5);
                check(
                    "storm",
                    v.total.get_untracked() == 1_250_000 && cmax < period.min(8.3) && v.table.is_at_end(),
                    format!("100,000 lines/s: frame cost p95 {c95:.2} ms, max {cmax:.2} ms (budget {:.1}); latency p95 {l95:.1} ms; {missed} late frames", period.min(8.3)),
                );
                scrolled_up_phase(v, path, probe, base);
            },
        );
    }

    /// 3. Scrolled up to read: new lines arrive but the view stays put.
    fn scrolled_up_phase(v: Viewer, path: PathBuf, probe: FrameLoop, base: f64) {
        let tv = v.table.table_view();
        tv.scrollRowToVisible(0);
        let top = tv.visibleRect().origin.y;
        let done = Arc::new(AtomicUsize::new(0));
        writer(path.clone(), 1_250_000, 10_000, 1.0, Arc::default(), done.clone());
        let d = done.clone();
        let v1 = v.clone();
        wait_until(
            move || d.load(Ordering::SeqCst) > 0 && v1.total.get_untracked() >= d.load(Ordering::SeqCst),
            move || {
                let still = v.table.table_view().visibleRect().origin.y;
                check(
                    "scrolled-up",
                    v.total.get_untracked() == 1_260_000 && still == top && !v.table.is_at_end(),
                    format!("10,000 more lines while scrolled to the top: view stayed at y={still} (was {top}); total {}", v.total.get_untracked()),
                );
                v.table.scroll_to_end();
                // Outside this callback's batch, so each filter change applies at once (as from a key press).
                let (v, path, probe) = (v.clone(), path.clone(), Rc::new(RefCell::new(Some(probe))));
                later(move || {
                    if let Some(probe) = probe.borrow_mut().take() {
                        filter_phase(v.clone(), path.clone(), probe, base);
                    }
                });
            },
        );
    }

    /// 4. Filtering 1.26M lines, then tailing with the filter on.
    fn filter_phase(v: Viewer, path: PathBuf, probe: FrameLoop, base: f64) {
        let count = |v: &Viewer, q: &str, level: Level| v.lines.borrow().iter().filter(|l| matches(l, q.as_bytes(), level)).count();
        let t = Instant::now();
        batch(|| v.query.set("request 12".into()));
        let text_ms = t.elapsed().as_secs_f64() * 1e3;
        let text_ok = v.table.len() == count(&v, "request 12", Level::Debug);
        let t = Instant::now();
        batch(|| {
            v.query.set(String::new());
            v.level.set(Level::Error);
        });
        let level_ms = t.elapsed().as_secs_f64() * 1e3;
        let errors = v.table.len();
        let done = Arc::new(AtomicUsize::new(0));
        writer(path, 1_260_000, 10_000, 1.0, Arc::default(), done.clone());
        let d = done.clone();
        let v1 = v.clone();
        wait_until(
            move || d.load(Ordering::SeqCst) > 0 && v1.total.get_untracked() >= d.load(Ordering::SeqCst),
            move || {
                let want = count(&v, "", Level::Error);
                let first_bad = v.table.rows().iter().position(|&i| v.lines.borrow()[i as usize].level != Level::Error);
                check(
                    "filter",
                    text_ok && errors == count(&v, "", Level::Error) - 200 && v.table.len() == want && v.shown.get_untracked() == want && first_bad.is_none(),
                    format!(
                        "\"request 12\" over 1.26M lines in {text_ms:.0} ms ({} rows, matches a recount: {text_ok}); Errors in {level_ms:.0} ms ({errors}); +10,000 lines → {} errors shown (recount {want}); non-errors shown: {first_bad:?}",
                        count(&v, "request 12", Level::Debug),
                        v.table.len()
                    ),
                );
                let (now, peak) = (footprint_mb(), peak_footprint_mb());
                check(
                    "memory",
                    peak - base < 600.0,
                    format!("footprint {base:.0} MB before → {now:.0} MB with 1.27M lines; peak {peak:.0} MB ({:.0} bytes per line)", (now - base) * 1e6 / 1.27e6),
                );
                drop(probe);
                v.frames.borrow_mut().take();
                v.tail.borrow_mut().take();
                let failed = FAILED.with(Cell::get);
                eprintln!("[check] done, {failed} failed");
                std::process::exit(if failed == 0 { 0 } else { 1 });
            },
        );
    }
}
