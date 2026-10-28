//! The spike checklist, rebuilt on `iced-appkit`: the regression suite for the
//! embedding core. Every test is on the native Tests menu (⌘1…⌘9) and in the
//! sidebar; the automated checks run unattended.
//!
//! Env:
//!   CHECK=all | <n>[,<n>...]   run automated checks, print `[check]` lines, quit
//!                              (exit code 1 if any failed)
//!   START=<n>                  open test n at launch
//!   RED=1                      red Iced background under every opaque native slot
//!   T2_RESIZE=1                test 2: oscillate the window width continuously
//!   T6_SCROLL=1                test 6: trackpad-timed auto-scroll continuously
//!   T4_TOGGLE=<ms>             test 4: click the dropdown open/closed every ms
//!   T5_TOGGLE=<ms>             test 5: open/close the modal every ms
//!   FLOAT=1                    keep the window above others (screen recording)
//!
//! Content area: window at (160, 30) on screen, 1024 x 800 points.
mod native;

use std::cell::Cell;
use std::time::{Duration, Instant};

use iced::widget::{
    Space, button, center, column, container, opaque, operation, pick_list, row, scrollable, stack, text, text_input,
    tooltip,
};
use iced::{Color, Element, Length, Point, Rectangle, Size, Subscription, Task, Theme, Vector, window};
use iced_appkit::table::{Column, Sort, TableModel};
use iced_appkit::text::TextModel;
use iced_appkit::{Event, NativeSlot, ViewId, host};

const TESTS: &[(u8, &str)] = &[
    (1, "Embed and click"),
    (2, "Live resize"),
    (3, "Pixel scale"),
    (4, "Dropdown over table"),
    (5, "Modal over native"),
    (6, "Scroll clipping"),
    (7, "Large data"),
    (8, "Tab order"),
    (9, "Shortcuts"),
    (13, "Dark Mode"),
    (14, "Teardown"),
    (15, "Table features"),
    (10, "Japanese input"),
    (11, "Spell check"),
    (16, "Text editor features"),
    (17, "Lifecycle and edge cases"),
];
const AUTOMATED: &[u8] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 14, 15, 16, 17];

const T1_BUTTON: u64 = 10;
const T2_LABELS: &[(u64, &str, (f32, f32, f32))] = &[
    (20, "top-left (fixed)", (0.0, 0.4, 0.95)),
    (21, "top-right (fixed)", (0.0, 0.4, 0.95)),
    (22, "full width", (0.2, 0.7, 0.3)),
    (23, "1/4", (0.5, 0.2, 0.8)),
    (24, "2/4", (0.9, 0.5, 0.0)),
    (25, "1/4", (0.5, 0.2, 0.8)),
];
const T3_LABELS: &[(u64, f32)] = &[(30, 120.0), (31, 120.5), (32, 99.25)];
const T4_TABLE: u64 = 40;
const T5_TABLE: u64 = 50;
const T5_BUTTON: u64 = 51;
const T6_TABLE: u64 = 60;
const T6_BUTTON: u64 = 61;
const T8_TABLE: u64 = 80;
const T8_TEXT: u64 = 81;
const T9_TEXT: u64 = 90;
const T9_TABLE: u64 = 91;
const T13_TABLE: u64 = 130;
const T13_TEXT: u64 = 131;
const T7_TABLE: u64 = 70;
const T7_ROWS: u32 = 1_000_000;
const T15_TABLE: u64 = 150;
const T15_ROWS: usize = 100_000;

/// A row for the table-features test.
#[derive(Debug, Clone)]
struct Person {
    id: u32,
    name: String,
    age: u32,
    active: bool,
}

fn people(n: usize) -> Vec<Person> {
    const A: &[&str] = &["Ka", "Lo", "Mi", "Zo", "Be", "Ar", "Ti", "Sa", "No", "Ve", "Ru", "Ji"];
    const B: &[&str] = &["ren", "la", "mon", "sie", "dra", "vik", "na", "tor", "lin", "bel"];
    let mut seed: u64 = 42;
    (0..n)
        .map(|i| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let r = (seed >> 33) as usize;
            Person {
                id: i as u32,
                name: format!("{}{} {}{}", A[r % A.len()], B[(r >> 4) % B.len()], A[(r >> 8) % A.len()], B[(r >> 12) % B.len()]),
                age: 18 + (r >> 16) as u32 % 70,
                active: (r >> 24).is_multiple_of(3),
            }
        })
        .collect()
}
const T10_TEXT: u64 = 100;
const T11_TEXT: u64 = 110;
const T16_A: u64 = 160;
const T16_B: u64 = 161;
const T16_INPUT: &str = "t16-input";
const T11_INITIAL: &str = "This sentense has a speling error. Look up a word or translate it.";
const T9_INITIAL: &str = "Select all of this, type over it, undo.";
const SCROLLABLE: &str = "t6-scroll";
const INPUT1: &str = "t8-input-1";
const INPUT2: &str = "t8-input-2";

#[derive(Debug, Clone)]
enum Message {
    Opened(window::Id),
    Attached(bool),
    Native(Event),
    Select(u8),
    Pick(&'static str),
    Input1(String),
    Input2(String),
    Modal(bool),
    Scrolled(iced::widget::scrollable::Viewport),
    Tick(f32),
    Step(u8, u32),
    Focus(u8, bool),
    Toggle,
    Frame(Instant),
    TableSelect(Vec<usize>),
    TableActivate(usize),
    TableSort(Sort),
    TableToggle(usize, String, bool),
    EditorChanged(u64, String),
    EditorSelection(u64, std::ops::Range<usize>),
    EditorComposition(u64, Option<String>),
}

struct App {
    test: u8,
    red: bool,
    clicks: u32,
    picked: Option<&'static str>,
    input1: String,
    input2: String,
    modal: bool,
    scroll_bounds: Option<Rectangle>,
    scroll_offset: Vector,
    scroll_dir: f32,
    auto_scroll: bool,
    queue: Vec<u8>,
    results: Vec<(u8, bool, String)>,
    /// Test 8: where focus was after each step ("in1", "table", ...).
    focus_log: Vec<String>,
    focus_probe: [bool; 2],
    t13_log: Vec<String>,
    keep: native::Keep,
    big: Option<TableModel<u32>>,
    people: Option<TableModel<Person>>,
    /// Test 7: frame intervals while auto-scrolling (ms), start RSS.
    t7_frames: Vec<f32>,
    t7_last: Option<Instant>,
    t7_running: bool,
    t7_rss: f64,
    last_select: Option<Vec<usize>>,
    last_activate: Option<usize>,
    last_sort: Option<Sort>,
    last_toggle: Option<(usize, String, bool)>,
    t15_saved_selection: Vec<usize>,
    editors: std::collections::HashMap<u64, TextModel>,
    /// Editor events seen by the app: (editor, "changed: …" / "selection …" / "composition …").
    editor_log: Vec<(u64, String)>,
    t16_input: String,
    t17_baseline: (usize, usize, usize, f64),
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn after(ms: u64, message: Message) -> Task<Message> {
    Task::perform(async move { std::thread::sleep(Duration::from_millis(ms)) }, move |_| message.clone())
}

thread_local! {
    static RESIZE_T0: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Oscillates the window width from a run-loop timer (~120 steps/s) with
/// `setFrame:display:YES`, the synchronous path a live resize takes.
fn start_resize(seconds: Option<f64>) {
    RESIZE_T0.with(|t| t.set(Some(Instant::now())));
    // SAFETY: a main-run-loop timer whose block only touches main-thread state.
    unsafe {
        use objc2_core_foundation::{CFAbsoluteTimeGetCurrent, CFRunLoop, CFRunLoopTimer, kCFRunLoopCommonModes};
        let block = block2::RcBlock::new(move |_: *mut CFRunLoopTimer| {
            let Some(t0) = RESIZE_T0.with(Cell::get) else { return };
            if seconds.is_some_and(|s| t0.elapsed().as_secs_f64() > s) {
                return;
            }
            let Some(window) = host::content_view().and_then(|v| v.window()) else { return };
            let phase = t0.elapsed().as_secs_f64() / 1.2 * std::f64::consts::TAU;
            let mut frame = window.frame();
            frame.size.width = (900.0 + 250.0 * (0.5 - 0.5 * phase.cos())).round();
            window.setFrame_display(frame, true);
        });
        let timer = CFRunLoopTimer::with_handler(None, CFAbsoluteTimeGetCurrent(), 0.008, 0, 0, Some(&block)).unwrap();
        CFRunLoop::main().unwrap().add_timer(Some(&timer), kCFRunLoopCommonModes);
        std::mem::forget(timer);
    }
}

fn center_of(id: u64) -> Option<(f64, f64)> {
    host::testing::visible_rect(ViewId(id)).map(|r| (r.x + r.width / 2.0, r.y + r.height / 2.0))
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let queue = match env("CHECK").as_deref() {
            Some("all") => AUTOMATED.to_vec(),
            Some(list) => list.split(',').filter_map(|n| n.trim().parse().ok()).collect(),
            None => Vec::new(),
        };
        let app = App {
            test: env("START").and_then(|s| s.parse().ok()).unwrap_or(0),
            red: env("RED").is_some(),
            clicks: 0,
            picked: None,
            input1: String::new(),
            input2: String::new(),
            modal: false,
            scroll_bounds: None,
            scroll_offset: Vector::ZERO,
            scroll_dir: 1.0,
            auto_scroll: env("T6_SCROLL").is_some(),
            queue,
            results: Vec::new(),
            focus_log: Vec::new(),
            focus_probe: [false; 2],
            t13_log: Vec::new(),
            keep: native::keep(),
            big: None,
            people: None,
            t7_frames: Vec::new(),
            t7_last: None,
            t7_running: false,
            t7_rss: 0.0,
            last_select: None,
            last_activate: None,
            last_sort: None,
            last_toggle: None,
            t15_saved_selection: Vec::new(),
            editors: std::collections::HashMap::new(),
            editor_log: Vec::new(),
            t16_input: String::new(),
            t17_baseline: (0, 0, 0, 0.0),
        };
        (app, Task::none())
    }

    fn record(&mut self, test: u8, pass: bool, note: impl Into<String>) {
        let note = note.into();
        eprintln!("[check] T{test} {} {note}", if pass { "PASS" } else { "FAIL" });
        self.results.push((test, pass, note));
    }

    /// Starts the next queued automated test, or reports and quits.
    fn next_check(&mut self) -> Task<Message> {
        if self.queue.is_empty() {
            let failed = self.results.iter().filter(|r| !r.1).count();
            eprintln!("[check] done: {} checks, {} failed", self.results.len(), failed);
            std::process::exit(if failed == 0 { 0 } else { 1 });
        }
        let test = self.queue.remove(0);
        Task::done(Message::Step(test, 0))
    }

    fn mount_all(&mut self) {
        native::mount_button(T1_BUTTON, "Native NSButton");
        for &(id, title, color) in T2_LABELS {
            native::mount_label(id, title, color);
        }
        for &(id, width) in T3_LABELS {
            native::mount_label(id, &format!("{width} pt"), (0.0, 0.4, 0.95));
        }
        native::mount_table(T4_TABLE, 1000, &mut self.keep);
        native::mount_table(T5_TABLE, 1000, &mut self.keep);
        native::mount_button(T5_BUTTON, "Native button");
        native::mount_table(T6_TABLE, 1000, &mut self.keep);
        native::mount_button(T6_BUTTON, "Native button in scroll");
        native::mount_table(T8_TABLE, 1000, &mut self.keep);
        let t8 = TextModel::new(ViewId(T8_TEXT), "Native NSTextView (Tab leaves it)");
        t8.set_accepts_tab(false);
        self.editors.insert(T8_TEXT, t8);
        self.editors.insert(T9_TEXT, TextModel::new(ViewId(T9_TEXT), T9_INITIAL));
        self.editors.insert(T10_TEXT, TextModel::new(ViewId(T10_TEXT), "Tokyo: "));
        self.editors.insert(T11_TEXT, TextModel::new(ViewId(T11_TEXT), T11_INITIAL));
        self.editors.insert(T16_A, TextModel::new(ViewId(T16_A), "Editor A"));
        self.editors.insert(T16_B, TextModel::new(ViewId(T16_B), "Editor B"));
        native::mount_table(T9_TABLE, 1000, &mut self.keep);
        native::mount_table(T13_TABLE, 1000, &mut self.keep);
        self.editors.insert(T13_TEXT, TextModel::new(ViewId(T13_TEXT), "Native text view follows the appearance."));
        let rss = native::rss_mb();
        let big = TableModel::new(
            ViewId(T7_TABLE),
            vec![
                Column::number("row", "#", |i: &u32| *i as f64).width(90.0),
                Column::text("name", "Name", |i: &u32| format!("Item {i:07}")).width(160.0),
                Column::number("value", "Value", |i: &u32| (*i as u64 * 2654435761 % 100_000) as f64 / 100.0).decimals(2),
                Column::check("even", "Even", |i: &u32| i.is_multiple_of(2)),
            ],
        );
        big.set_rows((0..T7_ROWS).collect());
        self.t7_rss = rss;
        eprintln!("[checklist] 1M-row model: RSS {rss:.0} → {:.0} MB", native::rss_mb());
        self.big = Some(big);
        let people_model = TableModel::new(
            ViewId(T15_TABLE),
            vec![
                Column::text("name", "Name", |p: &Person| p.name.clone()).width(200.0),
                Column::number("age", "Age", |p: &Person| p.age as f64).width(60.0),
                Column::check("active", "Active", |p: &Person| p.active),
                Column::number("id", "ID", |p: &Person| p.id as f64).width(80.0),
            ],
        );
        people_model.set_rows(people(T15_ROWS));
        self.people = Some(people_model);
        native::install_tests_menu(TESTS);
        host::install_edit_menu();
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Opened(id) => return iced_appkit::attach(id, Message::Attached),
            Message::Attached(ok) => {
                assert!(ok, "attach failed");
                self.mount_all();
                if env("FLOAT").is_some()
                    && let Some(w) = host::content_view().and_then(|v| v.window()) {
                        w.setLevel(3);
                        w.orderFrontRegardless();
                    }
                if env("T2_RESIZE").is_some() {
                    start_resize(None);
                }
                let mut tasks = vec![];
                if let Some(ms) = env("T4_TOGGLE").or(env("T5_TOGGLE")).and_then(|v| v.parse().ok()) {
                    tasks.push(after(ms, Message::Toggle));
                }
                if !self.queue.is_empty() {
                    tasks.push(after(500, Message::Step(0, 0)));
                }
                return Task::batch(tasks);
            }
            Message::Native(event) => {
                let task = iced_appkit::update(&event);
                if let Event::Action(ViewId(id)) = event {
                    match id {
                        T1_BUTTON => self.clicks += 1,
                        1000.. => self.test = (id - 1000) as u8,
                        _ => {}
                    }
                }
                return task;
            }
            Message::Select(test) => self.test = test,
            Message::Pick(item) => self.picked = Some(item),
            Message::Input1(v) => self.input1 = v,
            Message::Input2(v) => self.input2 = v,
            Message::Modal(open) => self.modal = open,
            Message::Scrolled(viewport) => {
                self.scroll_bounds = Some(viewport.bounds());
                let o = viewport.absolute_offset();
                self.scroll_offset = Vector::new(o.x, o.y);
            }
            Message::Tick(step) => {
                if self.test == 6 && self.auto_scroll {
                    if self.scroll_offset.y > 700.0 {
                        self.scroll_dir = -1.0;
                    } else if self.scroll_offset.y < 5.0 {
                        self.scroll_dir = 1.0;
                    }
                    return operation::scroll_by(
                        SCROLLABLE,
                        iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: step * self.scroll_dir },
                    );
                }
            }
            Message::Toggle => {
                if let Some(ms) = env("T4_TOGGLE").and_then(|v| v.parse::<u64>().ok()) {
                    // Click the dropdown; when open, click it again to close.
                    if let Some((x, y)) = self.t4_picker_center() {
                        host::testing::post_click(x, y);
                    }
                    return after(ms, Message::Toggle);
                }
                if let Some(ms) = env("T5_TOGGLE").and_then(|v| v.parse::<u64>().ok()) {
                    self.modal = !self.modal;
                    return after(ms, Message::Toggle);
                }
            }
            Message::Frame(now) => {
                if self.t7_running {
                    if let Some(last) = self.t7_last {
                        self.t7_frames.push((now - last).as_secs_f32() * 1000.0);
                    }
                    self.t7_last = Some(now);
                    if let Some(big) = &self.big {
                        objc2::rc::autoreleasepool(|_| native::scroll_table(&big.table_view(), 800.0));
                    }
                }
            }
            Message::TableSelect(rows) => self.last_select = Some(rows),
            Message::EditorChanged(id, text) => self.editor_log.push((id, format!("changed: {text}"))),
            Message::EditorSelection(id, range) => self.editor_log.push((id, format!("selection {range:?}"))),
            Message::EditorComposition(id, marked) => self.editor_log.push((id, format!("composition {marked:?}"))),
            Message::TableActivate(row) => self.last_activate = Some(row),
            Message::TableSort(sort) => self.last_sort = Some(sort),
            Message::TableToggle(row, column, value) => {
                self.last_toggle = Some((row, column.clone(), value));
                if let Some(model) = &self.people {
                    let mut person = model.rows()[row].clone();
                    if column == "active" {
                        person.active = value;
                    }
                    model.update_row(row, person);
                }
            }
            Message::Focus(slot, focused) => self.focus_probe[slot as usize] = focused,
            Message::Step(0, 0) => return self.next_check(),
            Message::Step(test, step) => return self.step(test, step),
        }
        Task::none()
    }

    /// The dropdown sits right above test 4's table (see `test4`).
    fn t4_picker_center(&self) -> Option<(f64, f64)> {
        let r = host::testing::visible_rect(ViewId(T4_TABLE))?;
        Some((r.x + 80.0, r.y - 12.0 - 15.0))
    }

    fn where_focus(&self) -> String {
        match host::focused_view() {
            Some(ViewId(T8_TABLE)) => "table".into(),
            Some(ViewId(T8_TEXT)) => "text".into(),
            Some(other) => format!("native {}", other.0),
            None if self.focus_probe[0] => "in1".into(),
            None if self.focus_probe[1] => "in2".into(),
            None => "none".into(),
        }
    }

    fn probe_focus(&self, test: u8, next_step: u32) -> Task<Message> {
        Task::batch([
            operation::is_focused(INPUT1).map(|f| Message::Focus(0, f)),
            operation::is_focused(INPUT2).map(|f| Message::Focus(1, f)),
        ])
        .chain(after(50, Message::Step(test, next_step)))
    }

    /// One step of an automated test. Each returns the task that runs the next.
    fn step(&mut self, test: u8, step: u32) -> Task<Message> {
        let next = |ms| after(ms, Message::Step(test, step + 1));
        let done = Task::done(Message::Step(0, 0));
        if step == 0 {
            self.test = test;
            self.modal = false;
            return next(400);
        }
        match (test, step) {
            (1, 1) => {
                let Some((x, y)) = center_of(T1_BUTTON) else {
                    self.record(1, false, "button not placed");
                    return done;
                };
                host::testing::post_click(x, y);
                next(400)
            }
            (1, 2) => {
                let pass = self.clicks == 1;
                self.record(1, pass, format!("native click reached Iced: {} click(s)", self.clicks));
                done
            }
            (2, 1) => {
                start_resize(Some(3.0));
                next(3500)
            }
            (2, 2) => {
                let shown = T2_LABELS.iter().all(|&(id, ..)| host::testing::is_visible(ViewId(id)));
                let off = host::testing::off_grid();
                self.record(2, shown && off.is_empty(), format!("after 3 s of resizing: all labels shown {shown}, off-grid {off:?} (lockstep: video)"));
                done
            }
            (3, 1) => {
                let off = host::testing::off_grid();
                let shown = T3_LABELS.iter().all(|&(id, _)| host::testing::is_visible(ViewId(id)));
                self.record(3, off.is_empty() && shown, format!("scale {}x, off-grid views {off:?}", host::scale()));
                done
            }
            (4, 1) => {
                let Some((x, y)) = self.t4_picker_center() else {
                    self.record(4, false, "table not placed");
                    return done;
                };
                host::testing::post_click(x, y);
                next(400)
            }
            (4, 2) => {
                let hidden = host::is_suppressed(ViewId(T4_TABLE)) && host::testing::click_reaches(ViewId(T4_TABLE)) != Some(true);
                self.record(4, hidden, format!("dropdown open: table swapped for its picture and hidden: {hidden}"));
                // Choose the first option (it lies over the table).
                let r = host::testing::visible_rect(ViewId(T4_TABLE)).unwrap();
                host::testing::post_click(r.x + 80.0, r.y + 12.0);
                next(400)
            }
            (4, 3) => {
                let back = !host::is_suppressed(ViewId(T4_TABLE)) && host::testing::is_visible(ViewId(T4_TABLE));
                self.record(4, back && self.picked.is_some(), format!("picked {:?}; table back after close: {back}", self.picked));
                // Hover the tooltip button (right of the dropdown).
                let (x, y) = self.t4_picker_center().unwrap();
                host::testing::post_mouse_move(x + 80.0 + 12.0 + 60.0, y);
                next(800)
            }
            (4, 4) => {
                let hidden = host::is_suppressed(ViewId(T4_TABLE));
                self.record(4, hidden, format!("tooltip shown on hover: table swapped for its picture: {hidden}"));
                host::testing::post_mouse_move(20.0, 20.0);
                next(500)
            }
            (4, 5) => {
                let back = !host::is_suppressed(ViewId(T4_TABLE)) && host::testing::is_visible(ViewId(T4_TABLE));
                self.record(4, back, format!("tooltip gone after the pointer left: table back {back}"));
                done
            }
            (5, 1) => {
                self.modal = true;
                next(400)
            }
            (5, 2) => {
                let blocked = [T5_TABLE, T5_BUTTON].map(|id| host::testing::click_reaches(ViewId(id)) != Some(true));
                self.record(5, blocked == [true, true], format!("modal open: clicks blocked (table, button) {blocked:?}"));
                self.modal = false;
                next(400)
            }
            (5, 3) => {
                let reach = [T5_TABLE, T5_BUTTON].map(|id| host::testing::click_reaches(ViewId(id)));
                self.record(5, reach == [Some(true), Some(true)], format!("modal closed: clicks reach natives {reach:?}"));
                done
            }
            (6, 1) => {
                self.auto_scroll = true;
                next(2500)
            }
            (6, 2) => {
                self.auto_scroll = env("T6_SCROLL").is_some();
                let inside = |id: u64| match (host::testing::visible_rect(ViewId(id)), self.scroll_bounds) {
                    (Some(v), Some(b)) => {
                        v.y >= b.y as f64 - 0.5 && v.y + v.height <= (b.y + b.height) as f64 + 0.5
                    }
                    (None, _) => true, // scrolled fully out of view: hidden
                    _ => false,
                };
                let ok = inside(T6_TABLE) && inside(T6_BUTTON);
                self.record(6, ok, format!("after 2.5 s of scrolling, natives clipped to the scrollable: {ok} (lockstep: video)"));
                done
            }
            (8, 1) => {
                self.focus_log.clear();
                operation::focus(INPUT1).chain(after(150, Message::Step(8, 2)))
            }
            (8, 2..=9) => {
                if step > 2 || self.focus_log.is_empty() {
                    // Record where focus is now, then press the next key.
                }
                host::testing::post_tab(step > 5);
                after(150, Message::Step(8, 100 + step))
            }
            (8, 102..=109) => {
                let probe_for = step - 100;
                self.probe_focus(8, 200 + probe_for)
            }
            (8, 202..=209) => {
                let w = self.where_focus();
                self.focus_log.push(w);
                if step < 209 {
                    after(10, Message::Step(8, step - 200 + 1))
                } else {
                    let expected = ["table", "text", "in2", "in1", "in2", "text", "table", "in1"];
                    let pass = self.focus_log == expected;
                    self.record(8, pass, format!("Tab ×4 then Shift-Tab ×4 from in1: {:?}", self.focus_log));
                    // Clicks: into the text view (Iced must let go of in1), then Tab.
                    // An NSTextView ignores the click that activates a window, so
                    // when the window isn't key (unattended runs), AppKit's part
                    // is done directly and the click only drives the monitor.
                    let (key, _) = host::testing::window_state();
                    if !key {
                        host::testing::make_first_responder_raw(ViewId(T8_TEXT));
                    }
                    let (x, y) = center_of(T8_TEXT).unwrap_or_default();
                    host::testing::post_click(x, y);
                    after(300, Message::Step(8, 300))
                }
            }
            (8, 300) => self.probe_focus(8, 301),
            (8, 301) => {
                let w = self.where_focus();
                let iced_free = !self.focus_probe[0] && !self.focus_probe[1];
                let how = if host::testing::window_state().0 { "real click" } else { "window not key: AppKit focus set directly, posted click drives the monitor" };
                self.record(8, w == "text" && iced_free, format!("click into text view: focus {w}, Iced inputs unfocused {iced_free} ({how})"));
                host::testing::post_tab(false);
                after(200, Message::Step(8, 302))
            }
            (8, 302) => self.probe_focus(8, 303),
            (8, 303) => {
                let w = self.where_focus();
                self.record(8, w == "in2", format!("Tab after the click: {w}"));
                // Click into the Iced input 1 while a native view has focus.
                host::focus(ViewId(T8_TABLE));
                after(150, Message::Step(8, 304))
            }
            (8, 304) => {
                let table = host::testing::visible_rect(ViewId(T8_TABLE)).unwrap();
                host::testing::post_click(table.x + 40.0, table.y - 12.0 - 15.0);
                after(300, Message::Step(8, 305))
            }
            (8, 305) => self.probe_focus(8, 306),
            (8, 306) => {
                let w = self.where_focus();
                let responder = host::testing::first_responder();
                self.record(8, w == "in1" && responder == "host", format!("click into Iced input from the table: focus {w}, first responder {responder}"));
                // Edit shortcuts with Iced focused and the Edit menu installed:
                // ⌘A must reach the Iced input, not the menu.
                for (c, code) in [("h", 4), ("e", 14), ("y", 16)] {
                    host::testing::post_key(c, code, false, false);
                }
                after(200, Message::Step(8, 307))
            }
            (8, 307) => {
                host::testing::post_key("a", 0, true, false);
                host::testing::post_key("q", 12, false, false);
                after(200, Message::Step(8, 308))
            }
            (8, 308) => {
                self.record(8, self.input1 == "q", format!("typed 'hey', ⌘A, 'q' into Iced input 1 (Edit menu installed): {:?}", self.input1));
                done
            }
            (9, 1) => {
                host::focus(ViewId(T9_TEXT));
                host::testing::post_key("a", 0, true, false);
                next(200)
            }
            (9, 2) => {
                let state = native::text_state(T9_TEXT);
                let all = state.as_ref().is_some_and(|(t, sel)| *sel == t.encode_utf16().count());
                self.record(9, all, format!("⌘A in text view selects all: {all}"));
                host::testing::post_key("x", 7, false, false);
                next(200)
            }
            (9, 3) => {
                let typed = native::text_state(T9_TEXT).map(|(t, _)| t);
                host::testing::post_key("z", 6, true, false);
                self.record(9, typed.as_deref() == Some("x"), format!("typing replaced the selection: {typed:?}"));
                next(200)
            }
            (9, 4) => {
                let text = native::text_state(T9_TEXT).map(|(t, _)| t);
                self.record(9, text.as_deref() == Some(T9_INITIAL), "⌘Z restored the text");
                host::focus(ViewId(T9_TABLE));
                host::testing::post_key("a", 0, true, false);
                next(200)
            }
            (9, 5) => {
                let sel = native::table_selected(T9_TABLE);
                self.record(9, sel.is_some_and(|(s, n)| s == n && n > 0), format!("⌘A in table selects all rows: {sel:?}"));
                // An app menu shortcut still works with a native view focused.
                host::testing::post_key("1", 18, true, false);
                next(400)
            }
            (9, 6) => {
                self.record(9, self.test == 1, format!("⌘1 (Tests menu) still works: now on test {}", self.test));
                done
            }
            (13, 1) => {
                self.t13_log.clear();
                host::testing::set_app_appearance(Some(true));
                next(500)
            }
            (13, 2) | (13, 3) => {
                let want_dark = step == 2;
                let theme_dark = iced_appkit::theme::system().palette().background.r < 0.5;
                let views = [T13_TABLE, T13_TEXT].map(|id| host::testing::view_is_dark(ViewId(id)));
                let ok = theme_dark == want_dark && views == [Some(want_dark); 2];
                self.record(13, ok, format!("forced {}: Iced system theme dark {theme_dark}, native views dark {views:?}", if want_dark { "dark" } else { "light" }));
                host::testing::set_app_appearance(if want_dark { Some(false) } else { None });
                if want_dark { next(500) } else { done }
            }
            (7, 1) => {
                self.t7_frames.clear();
                self.t7_last = None;
                self.t7_running = true;
                next(1_500)
            }
            (7, 2) => {
                // After a warm-up (the screen's first frames), memory must stay flat.
                self.t7_rss = native::rss_mb();
                self.t7_frames.clear();
                next(10_000)
            }
            (7, 3) => {
                self.t7_running = false;
                let summary = t7_summary(&self.t7_frames);
                let mut f = self.t7_frames.clone();
                f.sort_by(|a, b| a.total_cmp(b));
                let avg = f.iter().sum::<f32>() / f.len().max(1) as f32;
                let p95 = f.get(f.len() * 95 / 100).copied().unwrap_or(f32::MAX);
                let rss = native::rss_mb();
                let cells = self.big.as_ref().unwrap().cells_created();
                let pass = f.len() > 300 && avg < 17.5 && p95 < 25.0 && rss - self.t7_rss < 10.0;
                self.record(7, pass, format!("{summary}; RSS during 10 s of scrolling {:.0} → {rss:.0} MB; {cells} cell views ever created", self.t7_rss));
                done
            }
            (10, 1) => {
                let model = self.editors[&T10_TEXT].clone();
                model.set_text("Tokyo: ");
                model.set_selection(8..8);
                host::focus(model.id());
                self.editor_log.clear();
                let (candidate, view_rect) = native::ime_compose(&model.text_view(), "とうきょう");
                let inside = candidate.origin.x >= view_rect.origin.x
                    && candidate.origin.x <= view_rect.origin.x + view_rect.size.width
                    && candidate.origin.y >= view_rect.origin.y
                    && candidate.origin.y <= view_rect.origin.y + view_rect.size.height;
                self.record(10, model.composition().as_deref() == Some("とうきょう") && inside, format!(
                    "composing: marked text {:?}; candidate-window anchor ({:.0}, {:.0}) inside the editor on screen: {inside}",
                    model.composition(), candidate.origin.x, candidate.origin.y
                ));
                next(300)
            }
            (10, 2) => {
                let events = self.events_for(T10_TEXT);
                let ok = events.iter().any(|e| e == "composition Some(\"とうきょう\")") && !events.iter().any(|e| e.starts_with("changed"));
                self.record(10, ok, format!("app saw the composition and no text change while composing: {events:?}"));
                native::ime_commit(&self.editors[&T10_TEXT].text_view(), "東京");
                next(300)
            }
            (10, 3) => {
                let model = self.editors[&T10_TEXT].clone();
                let events = self.events_for(T10_TEXT);
                let ok = model.text() == "Tokyo: 東京"
                    && events.iter().any(|e| e == "composition None")
                    && events.iter().any(|e| e == "changed: Tokyo: 東京")
                    && model.selection() == (13..13);
                self.record(10, ok, format!("committed 東京: text {:?}, caret {:?} (bytes), events {events:?}", model.text(), model.selection()));
                done
            }
            // Continuous spell checking runs asynchronously once the text is
            // on screen: give it a moment before reading its results.
            (11, 1) => next(1500),
            (11, 2) => {
                let model = self.editors[&T11_TEXT].clone();
                let view = model.text_view();
                let flagged = ["sentense", "speling"].map(native::is_misspelled);
                let spelling = view.isContinuousSpellCheckingEnabled();
                self.record(11, spelling && flagged == [true, true], format!("continuous spell checking {spelling}; 'sentense', 'speling' flagged {flagged:?}"));
                let menu = native::context_menu_for_word(&view, "speling");
                let suggests = menu.iter().any(|t| t == "spelling");
                // Writing Tools appears only where Apple Intelligence is on.
                let services: Vec<&str> = ["Look Up", "Translate", "Writing Tools", "Spelling and Grammar"]
                    .into_iter()
                    .filter(|want| menu.iter().any(|t| t.starts_with(want)))
                    .collect();
                self.record(11, suggests && services.contains(&"Spelling and Grammar"), format!("right-click 'speling': suggestions first {:?}; menu has {services:?}", &menu[..menu.len().min(3)]));
                model.set_grammar_checking(true);
                self.record(11, view.isGrammarCheckingEnabled(), "grammar checking can be turned on");
                done
            }
            (16, 1) => {
                let a = self.editors[&T16_A].clone();
                self.editor_log.clear();
                a.set_text("Hello");
                a.set_selection(5..5);
                let quiet = self.events_for(T16_A).is_empty() && !a.can_undo();
                a.replace_selection(" wörld");
                let replaced = a.text() == "Hello wörld" && a.selection() == (12..12);
                self.record(16, quiet && replaced, format!("set_text/set_selection emit nothing and reset undo: {quiet}; replace_selection → {:?}, caret {:?}", a.text(), a.selection()));
                next(200)
            }
            (16, 2) => {
                let a = self.editors[&T16_A].clone();
                let changed = self.events_for(T16_A).iter().any(|e| e == "changed: Hello wörld");
                a.undo();
                self.record(16, changed && a.text() == "Hello", format!("replace_selection emitted Changed: {changed}; undo → {:?}", a.text()));
                // Typing goes to the focused editor; Tab inserts a tab there.
                host::focus(a.id());
                a.set_selection(5..5);
                host::testing::post_key("\t", 48, false, false);
                host::testing::post_key("A", 0, false, false);
                next(250)
            }
            (16, 3) => {
                let a = self.editors[&T16_A].clone();
                self.record(16, a.text() == "Hello\tA", format!("Tab inserts a tab in the editor: {:?}", a.text()));
                // Ctrl-Tab moves on to the next stop: editor B.
                host::testing::post_key_with("\t", 48, objc2_app_kit::NSEventModifierFlags::Control);
                next(250)
            }
            (16, 4) => {
                let focus = host::focused_view();
                self.record(16, focus == Some(ViewId(T16_B)), format!("Ctrl-Tab moves focus from A to B: {focus:?}"));
                // Each editor has its own undo: type in B, ⌘Z in B undoes only B.
                let b = self.editors[&T16_B].clone();
                b.set_selection(8..8);
                host::testing::post_key("!", 18, false, false);
                next(250)
            }
            (16, 5) => {
                host::testing::post_key("z", 6, true, false);
                next(250)
            }
            (16, 6) => {
                let (a, b) = (self.editors[&T16_A].text(), self.editors[&T16_B].text());
                self.record(16, a == "Hello\tA" && b == "Editor B", format!("⌘Z in B undid B's edit only: A {a:?}, B {b:?}"));
                // ⌘Z with the Iced input focused touches neither editor.
                operation::focus(T16_INPUT).chain(after(200, Message::Step(16, 7)))
            }
            (16, 7) => {
                host::testing::post_key("z", 6, true, false);
                next(250)
            }
            (16, 8) => {
                let (a, b) = (self.editors[&T16_A].text(), self.editors[&T16_B].text());
                self.record(16, a == "Hello\tA" && b == "Editor B", format!("⌘Z with Iced focused: A {a:?}, B {b:?} unchanged"));
                // Read-only: keys don't change the text, selection still works.
                let a = self.editors[&T16_A].clone();
                a.set_editable(false);
                host::focus(a.id());
                a.set_selection(0..0);
                host::testing::post_key("x", 7, false, false);
                next(250)
            }
            (16, 9) => {
                let a = self.editors[&T16_A].clone();
                self.record(16, a.text() == "Hello\tA" && !a.is_editable(), format!("read-only ignores typing: {:?}", a.text()));
                a.set_editable(true);
                // Find bar, font.
                a.show_find();
                a.set_font(Some("Menlo"), 15.0);
                next(300)
            }
            (16, 10) => {
                let a = self.editors[&T16_A].clone();
                self.record(16, a.is_find_visible() && a.font_size() == Some(15.0), format!("find bar shown {}; font size {:?}", a.is_find_visible(), a.font_size()));
                // A selection made by the user (AppKit) arrives in byte offsets.
                a.set_text("a東京😀b");
                self.editor_log.clear();
                a.text_view().setSelectedRange(objc2_foundation::NSRange::new(3, 2));
                next(250)
            }
            (16, 11) => {
                let a = self.editors[&T16_A].clone();
                let events = self.events_for(T16_A);
                let ok = events.last().map(String::as_str) == Some("selection 7..11") && &a.text()[7..11] == "😀";
                self.record(16, ok, format!("selecting 😀 (UTF-16 3+2) arrives as bytes: {events:?}"));
                let (role, value) = native::text_accessibility(&a.text_view());
                self.record(16, role == "AXTextArea" && value == a.text(), format!("accessibility: {role} with value {value:?}"));
                done
            }
            (17, 1) => {
                // Churn: create, mount, place, drop 200 tables and 200 editors.
                let (models, objc) = host::testing::live_objects();
                self.t17_baseline = (models, objc, host::testing::subview_count(), native::rss_mb());
                for i in 0..200u32 {
                    objc2::rc::autoreleasepool(|_| {
                        let t = TableModel::new(ViewId(9000), vec![Column::text("v", "Value", |r: &u32| r.to_string())]);
                        t.set_rows((0..1000).map(|r| r + i).collect());
                        t.ensure_mounted();
                        host::place(t.id(), iced_appkit::Rect::new(220.0, 100.0, 300.0, 200.0), iced_appkit::Rect::new(220.0, 100.0, 300.0, 200.0));
                        t.select(&[3, 5]);
                        let e = TextModel::new(ViewId(9001), "churn");
                        e.ensure_mounted();
                        host::place(e.id(), iced_appkit::Rect::new(220.0, 320.0, 300.0, 100.0), iced_appkit::Rect::new(220.0, 320.0, 300.0, 100.0));
                        e.replace_selection("x");
                        drop(t);
                        drop(e);
                    });
                }
                // AppKit may still hold the last views for a moment (deferred
                // work such as spell checking runs on later run-loop passes).
                next(3000)
            }
            (17, 2) => {
                let (m0, o0, v0, r0) = self.t17_baseline;
                let (m1, o1) = host::testing::live_objects();
                let v1 = host::testing::subview_count();
                let r1 = native::rss_mb();
                self.record(17, m1 == m0 && o1 == o0 && v1 == v0, format!(
                    "200 tables + 200 editors created and dropped: models {m0} → {m1}, ObjC objects {o0} → {o1}, subviews {v0} → {v1}, RSS {r0:.0} → {r1:.0} MB"
                ));
                // Table edge cases.
                let t = TableModel::new(ViewId(9002), vec![
                    Column::text("name", "Name", |p: &Person| p.name.clone()),
                    Column::number("age", "Age", |p: &Person| p.age as f64),
                ]);
                t.set_rows(Vec::new());
                let empty_ok = t.is_empty() && t.selection().is_empty() && t.row_at(0).is_none();
                t.update_row(5, people(1).remove(0)); // out of range: ignored
                let removed = t.remove(0);
                t.set_rows(people(50));
                t.select(&[2, 999]); // 999 ignored
                let sel = t.selection();
                t.set_sort(Some(Sort { column: "nope".into(), ascending: true })); // unknown column: unsorted
                let unsorted = t.row_at(0) == Some(0);
                t.set_sort(Some(Sort { column: "age".into(), ascending: true }));
                t.set_rows(people(50)); // new rows keep the sort
                let ages: Vec<u32> = (0..50).filter_map(|i| t.row_at(i)).map(|r| t.rows()[r].age).collect();
                let sorted = ages.windows(2).all(|w| w[0] <= w[1]);
                t.set_column_state(&[iced_appkit::table::ColumnState { id: "ghost".into(), width: 10.0 }]);
                let cols = t.column_state().len();
                let ok = empty_ok && removed.is_none() && sel == vec![2] && unsorted && sorted && cols == 2;
                self.record(17, ok, format!("table edges: empty {empty_ok}, remove(0) on empty {:?}, select [2, 999] → {sel:?}, unknown sort column leaves order {unsorted}, set_rows keeps the sort {sorted}, unknown column state ignored ({cols} columns)", removed.map(|p| p.id)));
                // Text edge cases.
                let e = TextModel::new(ViewId(9003), "");
                let empty = e.text().is_empty() && e.selection() == (0..0);
                e.set_text("😀 ok");
                e.set_selection(2..999); // inside 😀 and past the end
                let clamped = e.selection();
                e.set_editable(false);
                e.set_selection(0..0);
                e.replace_selection("nope");
                let read_only_kept = e.text() == "😀 ok";
                let ok = empty && clamped == (0..e.text().len()) && read_only_kept;
                self.record(17, ok, format!("text edges: empty {empty}, set_selection(2..999) on \"😀 ok\" → {clamped:?}, replace_selection on read-only leaves {:?}", e.text()));
                done
            }
            (15, 1) => {
                let model = self.people.clone().unwrap();
                let table = model.table_view();
                // Sorting 100k rows (keys cached, stable), then by another column.
                let t = Instant::now();
                model.set_sort(Some(Sort { column: "name".into(), ascending: true }));
                let by_name = t.elapsed();
                let t = Instant::now();
                model.set_sort(Some(Sort { column: "age".into(), ascending: false }));
                let by_age = t.elapsed();
                let first_ages: Vec<String> = (0..3).filter_map(|r| native::cell_text(&table, 1, r)).collect();
                self.record(15, by_name.as_millis() < 100 && by_age.as_millis() < 100, format!("sort {T15_ROWS} rows + reload: by name {by_name:.1?}, by age {by_age:.1?}; top ages {first_ages:?}"));
                // Selection through AppKit, as a click would.
                self.last_select = None;
                native::select_view_row(&table, 5);
                next(300)
            }
            (15, 2) => {
                let model = self.people.clone().unwrap();
                let sel = model.selection();
                self.record(15, self.last_select.as_ref() == Some(&sel) && sel.len() == 1, format!("selection event {:?} = model selection {sel:?}", self.last_select));
                self.t15_saved_selection = sel;
                // Return on the selected row activates it.
                host::focus(model.id());
                host::testing::post_key("\r", 36, false, false);
                next(300)
            }
            (15, 3) => {
                let ok = self.last_activate == self.t15_saved_selection.first().copied();
                self.record(15, ok, format!("Return activates the selected row: {:?}", self.last_activate));
                // A header click (sort descriptors set by AppKit): event + selection kept.
                self.last_sort = None;
                native::click_header_sort(&self.people.as_ref().unwrap().table_view(), "name", true);
                next(300)
            }
            (15, 4) => {
                let model = self.people.clone().unwrap();
                let table = model.table_view();
                let names: Vec<String> = (0..3).filter_map(|r| native::cell_text(&table, 0, r)).collect();
                let sorted = names.windows(2).all(|w| w[0].to_lowercase() <= w[1].to_lowercase());
                let kept = model.selection() == self.t15_saved_selection;
                self.record(
                    15,
                    self.last_sort == Some(Sort { column: "name".into(), ascending: true }) && sorted && kept,
                    format!("header click: event {:?}, first names {names:?}, selection kept {kept}", self.last_sort),
                );
                // Type-select: typing a prefix jumps to the first match.
                host::focus(model.id());
                for (c, code) in [("z", 6), ("o", 31)] {
                    host::testing::post_key(c, code, false, false);
                }
                next(400)
            }
            (15, 5) => {
                let model = self.people.clone().unwrap();
                let sel = model.selection();
                let name = sel.first().map(|&r| model.rows()[r].name.clone());
                self.record(15, name.as_deref().is_some_and(|n| n.to_lowercase().starts_with("zo")), format!("type-select 'zo' selected {name:?}"));
                // Checkbox toggle in the first visible row.
                self.last_toggle = None;
                let table = model.table_view();
                table.scrollRowToVisible(0);
                native::click_check(&table, 2, 0);
                next(300)
            }
            (15, 6) => {
                let model = self.people.clone().unwrap();
                let ok = match &self.last_toggle {
                    Some((row, col, value)) => col == "active" && model.rows()[*row].active == *value,
                    None => false,
                };
                self.record(15, ok, format!("checkbox click → toggle event {:?}, row updated: {ok}", self.last_toggle));
                // Insert and remove keep the selection on the same rows.
                let before = model.selection();
                let picked = before.first().map(|&r| model.rows()[r].id);
                model.insert(0, Person { id: 999_999, name: "Inserted".into(), age: 1, active: false });
                let after_insert = model.selection().first().map(|&r| model.rows()[r].id);
                model.remove(0);
                let after_remove = model.selection().first().map(|&r| model.rows()[r].id);
                self.record(15, picked.is_some() && picked == after_insert && picked == after_remove, format!("insert/remove keep the selected row: {picked:?} → {after_insert:?} → {after_remove:?}"));
                // Column state: move Age first, widen it; read it back.
                let mut state = model.column_state();
                let age = state.iter().position(|c| c.id == "age").unwrap();
                let mut col = state.remove(age);
                col.width = 90.0;
                state.insert(0, col);
                model.set_column_state(&state);
                let read = model.column_state();
                let ok = read.first().is_some_and(|c| c.id == "age" && (c.width - 90.0).abs() < 0.5) && model.drain_events().is_empty();
                self.record(15, ok, format!("column state restored: {:?}", read.iter().map(|c| format!("{}:{:.0}", c.id, c.width)).collect::<Vec<_>>()));
                // VoiceOver's view of the table (Name is column 1 after the move).
                let table = model.table_view();
                if env("AX_DUMP").is_some()
                    && let Some(scroll) = table.enclosingScrollView() {
                        native::ax_dump(&scroll, 0, 4, 4);
                    }
                let (role, rows, columns, header, values, check) = native::table_accessibility(&table);
                let expected = native::cell_text(&table, 1, 0);
                self.record(
                    15,
                    role == "AXTable" && rows > 10 && columns == 4 && header && expected.is_some_and(|e| values.contains(&e)) && check.as_deref() == Some("Active"),
                    format!("accessibility: {role}, {rows} rows, {columns} columns, header {header}, row 0 values {values:?}, checkbox label {check:?}"),
                );
                done
            }
            (14, 1) => {
                // Lifecycle: views of other tests are hidden (not drawn), not removed.
                let hidden = [T1_BUTTON, T4_TABLE, T9_TEXT].iter().all(|&id| host::is_mounted(ViewId(id)) && !host::testing::is_visible(ViewId(id)));
                self.record(14, hidden, format!("views not in the layout are hidden: {hidden}"));
                let base = host::testing::subview_count();
                let (kb, after_count) = native::teardown(1000);
                self.record(14, after_count == base, format!("1,000 mount/unmount cycles: subviews {base} → {after_count}, RSS +{kb} KB ({:.2} KB/view)", kb as f64 / 1000.0));
                done
            }
            _ => {
                self.record(test, false, format!("no step {step}"));
                done
            }
        }
    }

    fn editor(&self, id: u64, height: u16) -> Element<'_, Message> {
        iced_appkit::text_editor(&self.editors[&id])
            .height(f32::from(height))
            .on_change(move |t| Message::EditorChanged(id, t))
            .on_selection(move |r| Message::EditorSelection(id, r))
            .on_composition(move |m| Message::EditorComposition(id, m))
            .into()
    }

    fn log_for(&self, id: u64) -> String {
        self.editor_log.iter().rev().filter(|(e, _)| *e == id).take(4).map(|(_, l)| l.clone()).collect::<Vec<_>>().join("\n")
    }

    fn events_for(&self, id: u64) -> Vec<String> {
        self.editor_log.iter().filter(|(e, _)| *e == id).map(|(_, l)| l.clone()).collect()
    }

    fn slot(&self, id: u64, w: impl Into<Length>, h: impl Into<Length>) -> NativeSlot {
        let slot = NativeSlot::new(ViewId(id), w, h);
        // Not under buttons: an NSButton is transparent outside its bezel, so
        // red would always show there.
        let opaque = ![T1_BUTTON, T5_BUTTON, T6_BUTTON].contains(&id);
        if self.red && opaque { slot.background(Color::from_rgb(1.0, 0.0, 0.0)) } else { slot }
    }

    fn view(&self) -> Element<'_, Message> {
        let mut sidebar = column![text("Tests").size(18), button("Home").on_press(Message::Select(0))].spacing(6);
        for &(n, title) in TESTS {
            sidebar = sidebar.push(button(text(format!("{n}. {title}")).size(13)).on_press(Message::Select(n)));
        }
        let content: Element<'_, Message> = match self.test {
            1 => column![
                text("Test 1: Embed and click").size(22),
                self.slot(T1_BUTTON, 180, 32),
                text(format!("Native button clicks seen by Iced: {}", self.clicks)),
            ]
            .spacing(12)
            .into(),
            2 => {
                let l = |i: usize, w: Length, h: Length| -> Element<'_, Message> { self.slot(T2_LABELS[i].0, w, h).into() };
                column![
                    text("Test 2: Live resize").size(22),
                    row![l(0, Length::Fixed(160.0), Length::Fixed(40.0)), Space::new().width(Length::Fill), l(1, Length::Fixed(160.0), Length::Fixed(40.0))],
                    l(2, Length::Fill, Length::Fixed(40.0)),
                    row![l(3, Length::FillPortion(1), Length::Fill), l(4, Length::FillPortion(2), Length::Fill), l(5, Length::FillPortion(1), Length::Fill)].spacing(12.5),
                ]
                .spacing(12.5)
                .into()
            }
            3 => {
                let mut labels = row![].spacing(12.5);
                for &(id, w) in T3_LABELS {
                    labels = labels.push(self.slot(id, w, 40));
                }
                column![text("Test 3: Pixel scale").size(22), text(format!("Backing scale {}x", host::scale())), labels].spacing(12).into()
            }
            4 => column![
                text("Test 4: Dropdown and tooltip over a native table").size(22),
                row![
                    pick_list(["Apple", "Banana", "Cherry", "Damson", "Elderberry"], self.picked, Message::Pick).width(160),
                    tooltip(button("Hover for a tooltip"), container(text("An Iced tooltip, above the native table")).padding(8).style(container::rounded_box), tooltip::Position::Bottom),
                ]
                .spacing(12),
                self.slot(T4_TABLE, Length::Fill, 400),
            ]
            .spacing(12)
            .into(),
            5 => {
                let base = column![
                    text("Test 5: Modal over native views").size(22),
                    button("Open modal").on_press(Message::Modal(true)),
                    self.slot(T5_TABLE, Length::Fill, 300),
                    self.slot(T5_BUTTON, 160, 32),
                ]
                .spacing(12);
                if self.modal {
                    let dialog = container(column![text("An Iced modal").size(20), button("Close").on_press(Message::Modal(false))].spacing(12))
                        .padding(24)
                        .style(container::rounded_box);
                    let dim = container(center(dialog)).width(Length::Fill).height(Length::Fill).style(|_: &Theme| {
                        container::Style { background: Some(Color { a: 0.55, ..Color::BLACK }.into()), ..Default::default() }
                    });
                    stack![base, iced_appkit::cover(opaque(dim))].into()
                } else {
                    base.into()
                }
            }
            6 => {
                let items = column![
                    text("Iced content above the native views").size(14),
                    Space::new().height(200),
                    self.slot(T6_TABLE, Length::Fill, 300).in_scrollable(self.scroll_bounds, self.scroll_offset),
                    text("Iced content between").size(14),
                    self.slot(T6_BUTTON, 200, 32).in_scrollable(self.scroll_bounds, self.scroll_offset),
                    Space::new().height(600),
                ]
                .spacing(12);
                column![
                    text("Test 6: Scroll clipping").size(22),
                    scrollable(items).id(SCROLLABLE).on_scroll(Message::Scrolled).height(420),
                ]
                .spacing(12)
                .into()
            }
            8 => column![
                text("Test 8: Tab order").size(22),
                text_input("Iced input 1", &self.input1).id(INPUT1).on_input(Message::Input1),
                self.slot(T8_TABLE, Length::Fill, 200).focusable(true),
                self.editor(T8_TEXT, 120),
                text_input("Iced input 2", &self.input2).id(INPUT2).on_input(Message::Input2),
            ]
            .spacing(12)
            .into(),
            9 => column![
                text("Test 9: Shortcuts").size(22),
                self.editor(T9_TEXT, 120),
                self.slot(T9_TABLE, Length::Fill, 240).focusable(true),
            ]
            .spacing(12)
            .into(),
            13 => column![
                text("Test 13: Dark Mode (Iced uses iced_appkit::theme::system())").size(22),
                self.slot(T13_TABLE, Length::Fill, 240),
                self.editor(T13_TEXT, 120),
            ]
            .spacing(12)
            .into(),
            14 => column![text("Test 14: Teardown").size(22), text("Run with CHECK=14.")].spacing(12).into(),
            17 => column![text("Test 17: Lifecycle and edge cases").size(22), text("Run with CHECK=17.")].spacing(12).into(),
            7 => {
                let stats = if self.t7_frames.is_empty() { "Run with CHECK=7 to auto-scroll.".to_string() } else { t7_summary(&self.t7_frames) };
                let mut col = column![text(format!("Test 7: {} rows in a native table", T7_ROWS)).size(22), text(stats)].spacing(12);
                if let Some(big) = &self.big {
                    col = col.push(iced_appkit::table(big).height(Length::Fill));
                }
                col.into()
            }
            10 => column![
                text("Test 10: Japanese input (try typing toukyou with a Japanese input source)").size(22),
                self.editor(T10_TEXT, 160),
                text(self.log_for(T10_TEXT)).size(12),
            ]
            .spacing(12)
            .into(),
            11 => column![
                text("Test 11: Spell check (right-click a red-underlined word)").size(22),
                self.editor(T11_TEXT, 160),
            ]
            .spacing(12)
            .into(),
            16 => column![
                text("Test 16: Text editor features").size(22),
                self.editor(T16_A, 140),
                self.editor(T16_B, 140),
                text_input("Iced input", &self.t16_input).id(T16_INPUT).on_input(Message::Input2),
                text(self.log_for(T16_A)).size(12),
            ]
            .spacing(12)
            .into(),
            15 => {
                let mut col = column![
                    text("Test 15: Table features (sort, select, activate, toggle, columns)").size(22),
                    text(format!(
                        "select {:?}  activate {:?}  sort {:?}  toggle {:?}",
                        self.last_select.as_ref().map(|s| s.len()),
                        self.last_activate,
                        self.last_sort,
                        self.last_toggle
                    ))
                    .size(12),
                ]
                .spacing(12);
                if let Some(model) = &self.people {
                    col = col.push(
                        iced_appkit::table(model)
                            .height(Length::Fill)
                            .on_select(Message::TableSelect)
                            .on_activate(Message::TableActivate)
                            .on_sort(Message::TableSort)
                            .on_toggle(Message::TableToggle),
                    );
                }
                col.into()
            }
            _ => column![text("iced-appkit checklist").size(22), text("Pick a test from the sidebar or the Tests menu.")].spacing(12).into(),
        };
        let results = text(
            self.results.iter().rev().take(6).map(|(t, p, n)| format!("T{t} {} {n}", if *p { "PASS" } else { "FAIL" })).collect::<Vec<_>>().join("\n"),
        )
        .size(11);
        iced_appkit::root(row![
            container(sidebar).width(200).padding(16),
            column![container(content).height(Length::Fill), results].padding(20).spacing(8),
        ])
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            window::open_events().map(Message::Opened),
            iced_appkit::events().map(Message::Native),
            Subscription::run(ticks).map(Message::Tick),
            if self.t7_running { window::frames().map(Message::Frame) } else { Subscription::none() },
        ])
    }
}

fn t7_summary(frames: &[f32]) -> String {
    let mut f = frames.to_vec();
    f.sort_by(|a, b| a.total_cmp(b));
    if f.is_empty() {
        return "no frames".into();
    }
    let avg = f.iter().sum::<f32>() / f.len() as f32;
    let over = f.iter().filter(|&&x| x > 25.0).count();
    format!(
        "{} frames at 800 pt/frame: avg {avg:.1} ms, p95 {:.1} ms, max {:.1} ms, {over} > 25 ms",
        f.len(),
        f[f.len() * 95 / 100],
        f[f.len() - 1]
    )
}

/// Trackpad-like timing: steps of 2–14 pt every 5–11 ms, off the frame clock.
fn ticks() -> impl iced::futures::Stream<Item = f32> {
    let (tx, rx) = iced::futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        loop {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let r = (seed >> 33) as u32;
            std::thread::sleep(Duration::from_micros(5_000 + (r % 6_000) as u64));
            if tx.unbounded_send(2.0 + ((r >> 8) % 13) as f32).is_err() {
                break;
            }
        }
    });
    rx
}

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("iced-appkit checklist")
        .theme(|_: &App| iced_appkit::theme::system())
        .subscription(App::subscription)
        .window(window::Settings { size: Size::new(1024.0, 800.0), position: window::Position::Specific(Point::new(160.0, 30.0)), ..Default::default() })
        .run()
}
