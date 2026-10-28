//! M0 experiment: native views in step with Iced on stock crates (no wgpu patch).
//!
//! Env:
//!   SCENE=resize|scroll          what moves (default resize)
//!   SYNC=none|flush              flush the Core Animation transaction right after
//!                                each Iced redraw, or leave it to Core Animation
//!   GATE=visible|now|delay       when flushing may start (default visible)
//!   BG=red|match                 slot background: red (measurement) or the label colour
//!   SECS=<n>                     quit after n seconds and print sync stats (default 14)
//!   LOAD=<n>                     draw n extra text widgets behind everything (heavier frames)
//!   HOLE=<id>                    leave label <id> unmounted (positive control: its slot shows red)
//!   FLOAT=1                      keep the window above others (for screen recording)
//!
//! Window content area starts at (160, 30 + title bar) on screen, 1024 x 800.
use std::cell::Cell;
use std::time::{Duration, Instant};

use iced::widget::{Space, column, container, operation, row, scrollable, text};
use iced::{Color, Element, Length, Point, Rectangle, Size, Subscription, Task, Vector, window};
use iced_appkit::{NativeSlot, ViewId, host};
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSColor, NSTextAlignment, NSTextField, NSView};
use objc2_foundation::NSString;

const LABELS: &[(u64, &str, (f32, f32, f32))] = &[
    (1, "top-left (fixed)", (0.0, 0.4, 0.95)),
    (2, "top-right (fixed)", (0.0, 0.4, 0.95)),
    (3, "full width", (0.2, 0.7, 0.3)),
    (4, "1/4", (0.5, 0.2, 0.8)),
    (5, "2/4", (0.9, 0.5, 0.0)),
    (6, "1/4", (0.5, 0.2, 0.8)),
    (7, "bottom-right (fixed)", (0.0, 0.4, 0.95)),
];
const SCROLL_LABELS: u64 = 8;
const SCROLLABLE: &str = "scroll";

#[derive(Debug, Clone)]
enum Message {
    Opened(window::Id),
    Attached(bool),
    Tick(f32),
    Scrolled(scrollable::Viewport),
    Quit,
}

struct App {
    scene: String,
    red: bool,
    scroll_bounds: Option<Rectangle>,
    scroll_offset: Vector,
    direction: f32,
    started: Instant,
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn mount_label(mtm: MainThreadMarker, id: u64, title: &str, (r, g, b): (f32, f32, f32)) {
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setDrawsBackground(true);
    label.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(r as f64, g as f64, b as f64, 1.0)));
    label.setTextColor(Some(&NSColor::whiteColor()));
    label.setAlignment(NSTextAlignment::Center);
    let view: Retained<NSView> = Retained::into_super(Retained::into_super(label));
    host::mount(ViewId(id), view);
}

thread_local! {
    static FRAMES: Cell<u64> = const { Cell::new(0) };
    static RESIZE_T0: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Oscillates the window width (left edge fixed) from a run-loop timer, ~120
/// steps/s, with `setFrame:display:YES`: the same synchronous layout-and-draw
/// path a live resize takes.
fn resize_step() {
    let Some(t0) = RESIZE_T0.with(Cell::get) else { return };
    let Some(window) = host::content_view().and_then(|v| v.window()) else { return };
    let phase = t0.elapsed().as_secs_f64() / 1.2 * std::f64::consts::TAU;
    let width = 880.0 + (1150.0 - 880.0) * (0.5 - 0.5 * phase.cos());
    let mut frame = window.frame();
    frame.size.width = width.round();
    window.setFrame_display(frame, true);
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let app = App {
            scene: env("SCENE", "resize"),
            red: env("BG", "red") == "red",
            scroll_bounds: None,
            scroll_offset: Vector::ZERO,
            direction: 1.0,
            started: Instant::now(),
        };
        (app, Task::none())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Opened(id) => return iced_appkit::attach(id, Message::Attached),
            Message::Attached(ok) => {
                assert!(ok, "attach failed");
                let mtm = MainThreadMarker::new().unwrap();
                for &(id, title, color) in LABELS {
                    // HOLE=<id>: leave one slot empty (a positive control for the red counter).
                    if env("HOLE", "") == id.to_string() {
                        continue;
                    }
                    mount_label(mtm, id, title, color);
                }
                for i in 0..SCROLL_LABELS {
                    mount_label(mtm, 100 + i, &format!("native row block {i}"), (0.2, 0.45, 0.85));
                }
                let window = host::content_view().and_then(|v| v.window()).unwrap();
                if env("FLOAT", "0") == "1" {
                    window.setLevel(3);
                    window.orderFrontRegardless();
                }
                let gate = match env("GATE", "visible").as_str() {
                    "now" => host::sync::StartGate::Immediately,
                    "delay" => host::sync::StartGate::After(Duration::from_millis(500)),
                    _ => host::sync::StartGate::WindowVisible,
                };
                host::sync::set_flush_after_draw(env("SYNC", "none") == "flush", gate);
                if self.scene == "resize" && env("RESIZE", "1") == "1" {
                    RESIZE_T0.with(|t| t.set(Some(Instant::now())));
                    // SAFETY: a repeating main-run-loop timer whose block only
                    // touches main-thread state.
                    unsafe {
                        use objc2_core_foundation::{CFAbsoluteTimeGetCurrent, CFRunLoop, CFRunLoopTimer, kCFRunLoopCommonModes};
                        let block = block2::RcBlock::new(|_: *mut CFRunLoopTimer| resize_step());
                        let timer = CFRunLoopTimer::with_handler(None, CFAbsoluteTimeGetCurrent() + 1.0, 0.008, 0, 0, Some(&block)).unwrap();
                        CFRunLoop::main().unwrap().add_timer(Some(&timer), kCFRunLoopCommonModes);
                        std::mem::forget(timer);
                    }
                }
                let secs: u64 = env("SECS", "14").parse().unwrap_or(14);
                return Task::perform(async move { std::thread::sleep(Duration::from_secs(secs)) }, |_| Message::Quit);
            }
            Message::Tick(step) => {
                if self.scene == "scroll" && self.started.elapsed() > Duration::from_secs(1) {
                    if self.scroll_offset.y > 1500.0 {
                        self.direction = -1.0;
                    } else if self.scroll_offset.y < 10.0 {
                        self.direction = 1.0;
                    }
                    return operation::scroll_by(SCROLLABLE, scrollable::AbsoluteOffset { x: 0.0, y: step * self.direction });
                }
            }
            Message::Scrolled(viewport) => {
                self.scroll_bounds = Some(viewport.bounds());
                let o = viewport.absolute_offset();
                self.scroll_offset = Vector::new(o.x, o.y);
            }
            Message::Quit => {
                let frames = FRAMES.with(Cell::get);
                eprintln!("[m0] {:?} view() calls {frames} place→observer gap µs p10/p50/p90 {:?}", host::sync::stats(), host::sync::gap_percentiles());
                std::process::exit(0);
            }
        }
        Task::none()
    }

    fn slot(&self, id: u64, w: impl Into<Length>, h: impl Into<Length>, color: (f32, f32, f32)) -> NativeSlot {
        let bg = if self.red { Color::from_rgb(1.0, 0.0, 0.0) } else { Color::from_rgb(color.0, color.1, color.2) };
        NativeSlot::new(ViewId(id), w, h).background(bg)
    }

    fn view(&self) -> Element<'_, Message> {
        FRAMES.with(|f| f.set(f.get() + 1));
        let content: Element<'_, Message> = if self.scene == "scroll" {
            let mut items = column![text("Native label blocks inside an Iced scrollable.")].spacing(16);
            for i in 0..SCROLL_LABELS {
                items = items.push(
                    self.slot(100 + i, Length::Fill, 220, (0.2, 0.45, 0.85))
                        .in_scrollable(self.scroll_bounds, self.scroll_offset),
                );
                items = items.push(text(format!("Iced text between blocks {i} and {}", i + 1)));
            }
            scrollable(items.padding(12)).id(SCROLLABLE).on_scroll(Message::Scrolled).height(Length::Fill).into()
        } else {
            let l = |i: usize, w: Length, h: Length| -> Element<'_, Message> {
                let (id, _, c) = LABELS[i];
                self.slot(id, w, h, c).into()
            };
            column![
                row![l(0, Length::Fixed(160.0), Length::Fixed(40.0)), Space::new().width(Length::Fill), l(1, Length::Fixed(160.0), Length::Fixed(40.0))],
                l(2, Length::Fill, Length::Fixed(40.0)),
                row![l(3, Length::FillPortion(1), Length::Fill), l(4, Length::FillPortion(2), Length::Fill), l(5, Length::FillPortion(1), Length::Fill)]
                    .spacing(12.5),
                row![Space::new().width(Length::Fill), l(6, Length::Fixed(200.0), Length::Fixed(40.0))],
            ]
            .spacing(12.5)
            .into()
        };
        // LOAD=<n>: n text widgets drawn behind everything, to make each Iced
        // frame expensive (a heavier real app).
        let load: usize = env("LOAD", "0").parse().unwrap_or(0);
        let content: Element<'_, Message> = if load > 0 {
            let mut grid = column![].spacing(0);
            for r in 0..load / 40 {
                let mut line = row![].spacing(2);
                for c in 0..40 {
                    line = line.push(text(format!("{:04}", r * 40 + c)).size(9));
                }
                grid = grid.push(line);
            }
            iced::widget::stack![grid, content].into()
        } else {
            content
        };
        container(column![text(format!("M0 sync: SYNC={} GATE={} SCENE={}", env("SYNC", "none"), env("GATE", "visible"), self.scene)).size(20), content].spacing(12))
            .padding(24)
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([window::open_events().map(Message::Opened), Subscription::run(ticks).map(Message::Tick)])
    }
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
        .title("iced-appkit M0")
        .subscription(App::subscription)
        .window(window::Settings {
            size: Size::new(1024.0, 800.0),
            position: window::Position::Specific(Point::new(160.0, 30.0)),
            ..Default::default()
        })
        .run()
}
