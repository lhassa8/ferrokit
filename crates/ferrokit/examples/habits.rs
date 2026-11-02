//! S2: Habits, a habit tracker built to sit next to Apple's apps. A sidebar
//! in the system material, today's habits as cards with a progress ring,
//! each habit's streaks and a calendar of its history, insights charts, a
//! sheet for new habits, daily reminders, a menu-bar panel, and a printable
//! report. Everything is stored in the app's user defaults.
//! `cargo run -p ferrokit --example habits`; `CHECK=all` runs its
//! self-checks (on a fresh sample, not the stored habits).

use std::collections::BTreeSet;
use std::rc::Rc;
use std::cell::RefCell;

use ferrokit::*;
use ferrokit::t as tr;

const PALETTE: [Color; 8] = [Color::Green, Color::Orange, Color::Blue, Color::Purple, Color::Indigo, Color::Pink, Color::Teal, Color::Red];
const SYMBOLS: [&str; 10] =
    ["figure.walk", "book.fill", "drop.fill", "brain.head.profile", "bed.double.fill", "dumbbell.fill", "leaf.fill", "pencil", "music.note", "sun.max.fill"];
/// What VoiceOver reads for each of `SYMBOLS`.
const SYMBOL_NAMES: [&str; 10] = ["Walking", "Book", "Water", "Mind", "Bed", "Weights", "Leaf", "Pencil", "Music", "Sun"];

const NEW_HABIT: Command = Command::new("new-habit");
const PRINT_REPORT: Command = Command::new("print-report");
const EXPORT_REPORT: Command = Command::new("export-report");

#[derive(Clone, PartialEq, Debug)]
struct Habit {
    id: u32,
    name: String,
    symbol: String,
    /// Index into PALETTE.
    color: usize,
    /// A daily reminder, hour and minute.
    reminder: Option<(u32, u32)>,
}

impl Identifiable for Habit {
    type Id = u32;
    fn id(&self) -> u32 {
        self.id
    }
}

impl Habit {
    fn tint(&self) -> Color {
        PALETTE[self.color % PALETTE.len()]
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Page {
    Today,
    Insights,
    Habit(u32),
}

/// The app's state: habits and the days each was done.
#[derive(Clone, Copy)]
struct Tracker {
    habits: Signal<Vec<Habit>>,
    /// (habit, local day) for every day a habit was done.
    done: Signal<BTreeSet<(u32, i64)>>,
    page: Signal<Page>,
    today: Signal<i64>,
}

impl Tracker {
    fn habit(&self, id: u32) -> Option<Habit> {
        self.habits.with(|hs| hs.iter().find(|h| h.id == id).cloned())
    }

    fn is_done(&self, id: u32, day: i64) -> bool {
        self.done.with(|d| d.contains(&(id, day)))
    }

    fn toggle(&self, id: u32, day: i64) {
        self.done.update(|d| {
            if !d.remove(&(id, day)) {
                d.insert((id, day));
            }
        });
    }

    fn done_today(&self) -> usize {
        let today = self.today.get();
        self.habits.with(|hs| hs.iter().filter(|h| self.is_done(h.id, today)).count())
    }

    /// Days in a row up to today (or yesterday, while today is still open).
    fn streak(&self, id: u32) -> u32 {
        let today = self.today.get();
        let mut day = if self.is_done(id, today) { today } else { today - 1 };
        let mut n = 0;
        while self.is_done(id, day) {
            n += 1;
            day -= 1;
        }
        n
    }

    /// The longest run of days in a row.
    fn best_streak(&self, id: u32) -> u32 {
        self.done.with(|d| {
            let (mut best, mut run, mut last) = (0, 0, i64::MIN);
            for &(_, day) in d.range((id, i64::MIN)..=(id, i64::MAX)) {
                run = if day == last + 1 { run + 1 } else { 1 };
                best = best.max(run);
                last = day;
            }
            best
        })
    }

    /// The share of the last `days` days (today included) it was done.
    fn rate(&self, id: u32, days: i64) -> f64 {
        let today = self.today.get();
        let n = (today - days + 1..=today).filter(|&d| self.is_done(id, d)).count();
        n as f64 / days as f64
    }

    fn add(&self, name: &str, symbol: &str, color: usize) -> u32 {
        let id = self.habits.with(|hs| hs.iter().map(|h| h.id).max().unwrap_or(0) + 1);
        self.habits.update(|hs| hs.push(Habit { id, name: name.trim().into(), symbol: symbol.into(), color, reminder: None }));
        id
    }

    fn delete(&self, id: u32) {
        self.habits.update(|hs| hs.retain(|h| h.id != id));
        self.done.update(|d| d.retain(|&(h, _)| h != id));
        cancel_notification(&format!("habit-{id}"));
        if self.page.get_untracked() == Page::Habit(id) {
            self.page.set(Page::Today);
        }
    }

    /// Turns a habit's daily reminder on or off (and posts or cancels it).
    fn set_reminder(&self, id: u32, at: Option<(u32, u32)>) {
        self.habits.update(|hs| {
            if let Some(h) = hs.iter_mut().find(|h| h.id == id) {
                h.reminder = at;
            }
        });
        let Some(h) = self.habit(id) else { return };
        match at {
            Some((hour, minute)) => {
                let page = self.page;
                notify(
                    Notification::new(&h.name)
                        .body(&tr("Time for today's habit."))
                        .sound()
                        .id(&format!("habit-{id}"))
                        .daily_at(hour, minute)
                        .on_click(move || page.set(Page::Habit(id))),
                )
            }
            None => cancel_notification(&format!("habit-{id}")),
        }
    }
}

// ---- storage: one line per habit and per day done ----

fn save(habits: &[Habit], done: &BTreeSet<(u32, i64)>) -> String {
    let mut out = String::new();
    for h in habits {
        let at = h.reminder.map_or("-".into(), |(hh, mm)| format!("{hh:02}:{mm:02}"));
        out += &format!("h\t{}\t{}\t{}\t{at}\t{}\n", h.id, h.color, h.symbol, h.name.replace(['\t', '\n'], " "));
    }
    for (id, day) in done {
        out += &format!("d\t{id}\t{day}\n");
    }
    out
}

fn load(text: &str) -> (Vec<Habit>, BTreeSet<(u32, i64)>) {
    let (mut habits, mut done) = (Vec::new(), BTreeSet::new());
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            ["h", id, color, symbol, at, name] => {
                let reminder = at.split_once(':').and_then(|(h, m)| Some((h.parse().ok()?, m.parse().ok()?)));
                if let (Ok(id), Ok(color)) = (id.parse(), color.parse()) {
                    habits.push(Habit { id, name: name.to_string(), symbol: symbol.to_string(), color, reminder });
                }
            }
            ["d", id, day] => {
                if let (Ok(id), Ok(day)) = (id.parse(), day.parse()) {
                    done.insert((id, day));
                }
            }
            _ => {}
        }
    }
    (habits, done)
}

/// Five habits with four months of history, from a fixed seed: a first
/// launch that looks lived in.
fn sample(today: i64) -> (Vec<Habit>, BTreeSet<(u32, i64)>) {
    let habits = vec![
        Habit { id: 1, name: tr("Walk 8,000 steps"), symbol: "figure.walk".into(), color: 0, reminder: None },
        Habit { id: 2, name: tr("Read 20 pages"), symbol: "book.fill".into(), color: 1, reminder: Some((21, 0)) },
        Habit { id: 3, name: tr("Drink water"), symbol: "drop.fill".into(), color: 2, reminder: None },
        Habit { id: 4, name: tr("Meditate"), symbol: "brain.head.profile".into(), color: 3, reminder: None },
        Habit { id: 5, name: tr("In bed by 23:00"), symbol: "bed.double.fill".into(), color: 4, reminder: None },
    ];
    let odds = [85u64, 70, 90, 50, 60];
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % 100
    };
    let mut done = BTreeSet::new();
    for day in today - 120..today {
        for (h, &p) in habits.iter().zip(&odds) {
            if next() < p {
                done.insert((h.id, day));
            }
        }
    }
    // Current streaks: walking 12 days (today too), reading 5; walking and
    // water done today.
    for d in today - 11..today {
        done.insert((1, d));
    }
    for d in today - 5..today {
        done.insert((2, d));
    }
    done.remove(&(1, today - 12));
    done.remove(&(2, today - 6));
    done.insert((1, today));
    done.insert((3, today));
    (habits, done)
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn days(n: u32) -> String {
    t_plural(n as i64, "{n} day", "{n} days")
}

/// A card's streak line: "12 days", or an invitation with none going.
fn streak_line(n: u32) -> String {
    if n == 0 { tr("Start a streak today") } else { days(n) }
}

fn percent(x: f64) -> String {
    format!("{}%", format_number((x * 100.0).round(), 0))
}

// ---- small pieces ----

/// A habit's symbol in a circle of its colour.
fn badge(symbol: &str, tint: Color, size: f64) -> impl View + use<> {
    zstack((
        canvas(move |p| {
            let r = p.width().min(p.height()) / 2.0;
            p.fill_circle(p.width() / 2.0, p.height() / 2.0, r, tint.opacity(0.18));
        })
        .min_size(size, size),
        image(symbol).symbol_size(size * 0.45).color(tint),
    ))
    .frame(size, size)
}

/// A card as AppKit's grouped boxes draw them: the system's fill for
/// grouped content, rounded, no border or shadow.
fn card(content: impl View) -> impl View {
    content.padding(16.0).background(Color::QuaternaryFill).corner_radius(12.0)
}

/// A ring `done()` full, in `tint`, with the percentage inside.
fn ring(done: impl Fn() -> f64 + 'static, tint: Color, size: f64) -> Canvas {
    let label = Rc::new(done);
    let described = label.clone();
    canvas(move |p| {
        let (cx, cy) = (p.width() / 2.0, p.height() / 2.0);
        let width = size * 0.11;
        let r = cx.min(cy) - width / 2.0 - 1.0;
        let f = label().clamp(0.0, 1.0);
        p.stroke_circle(cx, cy, r, tint.opacity(0.18), width);
        p.stroke_arc(cx, cy, r, 0.0..360.0 * f, tint, width);
        p.bold_text(cx, cy - size * 0.11, &percent(f), size * 0.18, Color::Primary, Align::Center);
    })
    .description(move || t_with("{percent} done today", &[("percent", &percent(described()))]))
    .min_size(size, size)
}

/// The last seven days as dots under their weekday letters.
fn week_dots(t: Tracker, id: u32, tint: Color) -> impl View {
    canvas(move |p| {
        let today = t.today.get();
        for i in 0..7 {
            let day = today - 6 + i;
            let x = 9.0 + i as f64 * 20.0;
            let letter = format_date(local_day_start(day) + 43_200.0, "EEEEE");
            p.text(x, 0.0, &letter, 10.0, if day == today { Color::Primary } else { Color::Tertiary }, Align::Center);
            if t.is_done(id, day) {
                p.fill_circle(x, 25.0, 6.0, tint);
            } else {
                p.stroke_circle(x, 25.0, 5.5, Color::Quaternary, 1.5);
            }
        }
    })
    .description(move || {
        let today = t.today.get();
        let n = (today - 6..=today).filter(|&d| t.is_done(id, d)).count();
        t_with("Done {n} of the last 7 days", &[("n", &n)])
    })
    .min_size(140.0, 34.0)
    .frame(140.0, 34.0)
}

/// A round check: filled with a tick when done.
fn check_mark(t: Tracker, id: u32, tint: Color) -> impl View {
    canvas(move |p| {
        let (cx, cy) = (p.width() / 2.0, p.height() / 2.0);
        if t.is_done(id, t.today.get()) {
            p.fill_circle(cx, cy, 13.0, tint);
            p.polyline(&[(cx - 5.5, cy + 0.5), (cx - 1.5, cy + 4.5), (cx + 6.0, cy - 4.5)], Color::rgb(1.0, 1.0, 1.0), 2.5);
        } else {
            p.stroke_circle(cx, cy, 12.0, Color::Tertiary, 2.0);
        }
    })
    .description(move || if t.is_done(id, t.today.get()) { tr("Done") } else { tr("Not done") })
    .min_size(30.0, 30.0)
    .frame(30.0, 30.0)
}

// ---- today ----

fn habit_card(t: Tracker, h: Habit) -> impl View {
    let (id, tint) = (h.id, h.tint());
    hstack((
        badge(&h.symbol, tint, 44.0),
        vstack((
            text(move || t.habit(id).map(|h| h.name).unwrap_or_default()).font(Font::headline()),
            hstack((image("flame.fill").symbol_size(11.0).color(Color::Orange), text(move || streak_line(t.streak(id))).font(Font::caption()).color(Color::Secondary))).spacing(4.0),
        ))
        .spacing(3.0)
        .alignment(Align::Start),
        spacer(),
        week_dots(t, id, tint),
        check_mark(t, id, tint),
    ))
    .spacing(14.0)
    .padding(14.0)
    .background(move || if t.is_done(id, t.today.get()) { tint.opacity(0.14) } else { Color::QuaternaryFill })
    .corner_radius(12.0)
    .hover_highlight(Color::Quaternary)
    .on_click(move || animated(|| t.toggle(id, t.today.get_untracked())))
    // VoiceOver reads the card's text, then whether it's done.
    .accessibility_value(move || if t.is_done(id, t.today.get()) { tr("Done") } else { tr("Not done") })
    .transition(Transition::Slide(Edge::Below))
    .fill_width()
}

fn today_view(t: Tracker) -> impl View {
    let total = move || t.habits.with(Vec::len);
    let summary = card(hstack((
        ring(move || if total() == 0 { 0.0 } else { t.done_today() as f64 / total() as f64 }, Color::Green, 116.0).frame(116.0, 116.0),
        vstack((
            text(move || t_with("{done} of {total} done", &[("done", &t.done_today()), ("total", &total())])).font(Font::title2().weight(Weight::Bold)),
            text(move || match total().saturating_sub(t.done_today()) {
                0 => tr("Everything done. See you tomorrow."),
                left => t_plural(left as i64, "{n} habit to go today", "{n} habits to go today"),
            })
            .color(Color::Secondary),
            hstack((image("flame.fill").color(Color::Orange), text(move || best_now(t)).font(Font::callout()))).spacing(6.0),
        ))
        .spacing(6.0)
        .alignment(Align::Start),
        spacer(),
    ))
    .spacing(24.0))
    .fill_width();
    scroll(
        vstack((
            vstack((
                text(format_date(now(), "EEEEdMMMM")).font(Font::large_title().weight(Weight::Bold)),
                text(tr("Today")).font(Font::title3()).color(Color::Secondary),
            ))
            .spacing(2.0)
            .alignment(Align::Start),
            summary,
            vstack((for_each(move || t.habits.get(), move |h| habit_card(t, h)),)).spacing(10.0).fill_width(),
        ))
        .spacing(20.0)
        .alignment(Align::Start)
        .padding(28.0),
    )
}

/// "Longest going: Walk, 12 days".
fn best_now(t: Tracker) -> String {
    let best = t.habits.with(|hs| hs.iter().map(|h| (t.streak(h.id), h.name.clone())).max_by_key(|(n, _)| *n));
    match best {
        Some((n, name)) if n > 0 => t_with("Longest going: {name}, {days}", &[("name", &name), ("days", &days(n))]),
        _ => tr("No streaks yet"),
    }
}

// ---- a habit ----

fn stat(title: String, value: impl Fn() -> String + 'static, tint: Color) -> impl View {
    card(vstack((text(title).font(Font::caption()).color(Color::Secondary), text(value).font(Font::title().weight(Weight::Bold)).color(tint))).spacing(4.0).alignment(Align::Start)).fill_width()
}

const WEEKS: i64 = 26;

/// The first day (a Monday) the history calendar shows.
fn first_shown(today: i64) -> i64 {
    today - weekday(today) as i64 - (WEEKS - 1) * 7
}

/// Six months of days, a column per week, in the habit's colour; a click
/// changes a day.
fn calendar(t: Tracker, id: u32, tint: Color) -> impl View {
    const CELL: f64 = 15.0;
    const GAP: f64 = 4.0;
    const LEFT: f64 = 30.0;
    const TOP: f64 = 18.0;
    canvas(move |p| {
        let today = t.today.get();
        let first = first_shown(today);
        for (row, name) in [(0, 0), (2, 2), (4, 4)] {
            let letter = format_date(local_day_start(first + name) + 43_200.0, "EEE");
            p.text(0.0, TOP + row as f64 * (CELL + GAP), &letter, 10.0, Color::Tertiary, Align::Start);
        }
        let mut last_month = String::new();
        for week in 0..WEEKS {
            let x = LEFT + week as f64 * (CELL + GAP);
            let month = format_date(local_day_start(first + week * 7) + 43_200.0, "MMM");
            if month != last_month {
                p.text(x, 0.0, &month, 10.0, Color::Secondary, Align::Start);
                last_month = month;
            }
            for wd in 0..7 {
                let day = first + week * 7 + wd;
                if day > today {
                    continue;
                }
                let y = TOP + wd as f64 * (CELL + GAP);
                let color = if t.is_done(id, day) { tint } else { Color::Quaternary.opacity(0.6) };
                p.fill_rounded_rect(x, y, CELL, CELL, 4.0, color);
                if day == today {
                    p.stroke_rect(x - 1.5, y - 1.5, CELL + 3.0, CELL + 3.0, Color::Primary, 1.0);
                }
            }
        }
    })
    .on_click(move |x, y| {
        let (col, row) = (((x - LEFT) / (CELL + GAP)).floor() as i64, ((y - TOP) / (CELL + GAP)).floor() as i64);
        if !(0..WEEKS).contains(&col) || !(0..7).contains(&row) {
            return;
        }
        let today = t.today.get_untracked();
        let day = first_shown(today) + col * 7 + row;
        if day <= today {
            t.toggle(id, day);
        }
    })
    .description(move || {
        let name = t.habit(id).map(|h| h.name).unwrap_or_default();
        t_with("{name}: done {percent} of the last 30 days", &[("name", &name), ("percent", &percent(t.rate(id, 30)))])
    })
    .min_size(LEFT + WEEKS as f64 * (CELL + GAP), TOP + 7.0 * (CELL + GAP))
}

fn habit_view(t: Tracker, id: u32) -> AnyView {
    let Some(h) = untrack(|| t.habit(id)) else { return AnyView::new(text(tr("No habit")).padding(28.0)) };
    let tint = h.tint();
    let remind = Signal::new(h.reminder.is_some());
    let time = Signal::new(h.reminder.unwrap_or((8, 0)));
    let share_text = move || {
        let h = t.habit(id).map(|h| h.name).unwrap_or_default();
        t_with("{days} of “{name}” in a row, in Habits.", &[("days", &days(t.streak(id))), ("name", &h)])
    };
    let by_weekday = move || {
        let today = t.today.get();
        (0..7)
            .map(|wd: i64| {
                let days: Vec<i64> = (today - 83..=today).filter(|&d| weekday(d) as i64 == wd).collect();
                let rate = days.iter().filter(|&&d| t.is_done(id, d)).count() as f64 / days.len().max(1) as f64;
                (format_date(local_day_start(first_shown(today) + wd) + 43_200.0, "EEE"), rate)
            })
            .collect::<Vec<_>>()
    };
    let name = h.name.clone();
    AnyView::new(scroll(
        vstack((
            hstack((
                badge(&h.symbol, tint, 64.0),
                vstack((
                    text(move || t.habit(id).map(|h| h.name).unwrap_or_default()).font(Font::large_title().weight(Weight::Bold)),
                    text(move || match t.habit(id).and_then(|h| h.reminder) {
                        Some((hh, mm)) => t_with("Every day · reminder at {time}", &[("time", &format_date(local_day_start(0) + (hh * 3600 + mm * 60) as f64, "jm"))]),
                        None => tr("Every day"),
                    })
                    .color(Color::Secondary),
                ))
                .spacing(2.0)
                .alignment(Align::Start),
                spacer(),
                share_button(move || vec![ShareItem::Text(share_text())]).tooltip(&tr("Share this streak")),
                button(tr("Delete…"), move || confirm_delete(t, id, name.clone())),
            ))
            .spacing(16.0),
            hstack((
                stat(tr("Current streak"), move || days(t.streak(id)), tint),
                stat(tr("Best streak"), move || days(t.best_streak(id)), Color::Primary),
                stat(tr("Last 30 days"), move || percent(t.rate(id, 30)), Color::Primary),
            ))
            .spacing(12.0)
            .fill_width(),
            card(vstack((
                text(tr("History")).font(Font::headline()),
                calendar(t, id, tint),
                text(tr("Click a day to mark it done or not.")).font(Font::caption()).color(Color::Tertiary),
            ))
            .spacing(10.0)
            .alignment(Align::Start))
            .fill_width(),
            card(vstack((
                text(tr("By weekday, last 12 weeks")).font(Font::headline()),
                bar_chart(by_weekday).value_format(Format::Percent).colors(&[tint]).min_size(300.0, 160.0),
            ))
            .spacing(10.0)
            .alignment(Align::Start))
            .fill_width(),
            card(hstack((
                image("bell.fill").color(Color::Orange),
                checkbox(&tr("Remind me every day at"), &remind),
                time_picker(&time).accessibility_label(&tr("Reminder time")).disabled(move || !remind.get()),
                spacer(),
            ))
            .spacing(8.0))
            .fill_width(),
            // Changes to the reminder settings post or cancel it.
            watch(move || {
                let at = remind.get().then(|| time.get());
                if untrack(|| t.habit(id).map(|h| h.reminder)) != Some(at) {
                    untrack(|| t.set_reminder(id, at));
                }
            }),
        ))
        .spacing(16.0)
        .alignment(Align::Start)
        .padding(28.0),
    ))
}

fn confirm_delete(t: Tracker, id: u32, name: String) {
    let Some(w) = focused_window() else { return };
    let alert_ = Alert::new(&t_with("Delete “{name}”?", &[("name", &name)]), &tr("Its history is deleted too."))
        .button(&tr("Delete"))
        .button(&tr("Cancel"))
        .destructive(0);
    alert(&w, alert_, move |answer| {
        if answer == 0 {
            animated(|| t.delete(id));
        }
    });
}

// ---- insights ----

fn insights(t: Tracker) -> impl View {
    let per_day = move || {
        let today = t.today.get();
        let points: Vec<(f64, f64)> = (today - 89..=today)
            .map(|d| (local_day_start(d) + 43_200.0, t.habits.with(|hs| hs.iter().filter(|h| t.is_done(h.id, d)).count()) as f64))
            .collect();
        vec![Series::new(&tr("Done"), points).color(Color::Green)]
    };
    let weekly = move || {
        let today = t.today.get();
        let monday = today - weekday(today) as i64;
        (0..8)
            .rev()
            .map(|w| {
                let start = monday - w * 7;
                let label = format_date(local_day_start(start) + 43_200.0, "MMMd");
                let counts = t.habits.with(|hs| hs.iter().map(|h| (start..start + 7).filter(|&d| d <= today && t.is_done(h.id, d)).count() as f64).collect());
                (label, counts)
            })
            .collect::<Vec<_>>()
    };
    let shares = move || {
        t.habits.with(|hs| hs.iter().map(|h| (h.name.clone(), (t.today.get() - 29..=t.today.get()).filter(|&d| t.is_done(h.id, d)).count() as f64)).collect::<Vec<_>>())
    };
    let this_week = move || {
        let today = t.today.get();
        let monday = today - weekday(today) as i64;
        let n: usize = t.habits.with(|hs| hs.iter().map(|h| (monday..=today).filter(|&d| t.is_done(h.id, d)).count()).sum());
        format_number(n as f64, 0)
    };
    let best_day = move || {
        let today = t.today.get();
        let rates: Vec<(i64, usize)> = (0..7)
            .map(|wd| (wd, (today - 83..=today).filter(|&d| weekday(d) as i64 == wd).map(|d| t.habits.with(|hs| hs.iter().filter(|h| t.is_done(h.id, d)).count())).sum()))
            .collect();
        let best = rates.iter().max_by_key(|(_, n)| *n).map_or(0, |(wd, _)| *wd);
        format_date(local_day_start(first_shown(today) + best) + 43_200.0, "EEEE")
    };
    // Rebuilt when habits come or go: the series' names and colours change.
    dynamic(move || {
        let (names, colors): (Vec<String>, Vec<Color>) = t.habits.with(|hs| hs.iter().map(|h| (h.name.clone(), h.tint())).unzip());
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        AnyView::new(scroll(
            vstack((
                text(tr("Insights")).font(Font::large_title().weight(Weight::Bold)),
                hstack((
                    stat(tr("Done this week"), this_week, Color::Green),
                    stat(tr("Best day"), best_day, Color::Primary),
                    stat(tr("Longest going"), move || best_now(t).split(": ").nth(1).unwrap_or("–").to_string(), Color::Orange),
                ))
                .spacing(12.0)
                .fill_width(),
                card(vstack((text(tr("Done per day")).font(Font::headline()), area_chart(per_day).x_format(Format::Date).min_size(400.0, 170.0))).spacing(10.0).alignment(Align::Start)).fill_width(),
                hstack((
                    card(vstack((
                        text(tr("By week")).font(Font::headline()),
                        stacked_bar_chart(weekly).series_names(&names).colors(&colors).min_size(320.0, 220.0),
                    ))
                    .spacing(10.0)
                    .alignment(Align::Start))
                    .fill_width(),
                    card(vstack((text(tr("Last 30 days")).font(Font::headline()), pie_chart(shares).donut().colors(&colors).min_size(260.0, 220.0))).spacing(10.0).alignment(Align::Start))
                        .fill_width(),
                ))
                .spacing(12.0)
                .fill_width(),
            ))
            .spacing(16.0)
            .alignment(Align::Start)
            .padding(28.0),
        ))
    })
}

// ---- the sidebar ----

/// A clickable row with a symbol (New Habit…).
fn row<F, S, A>(symbol: &str, tint: Color, title: F, selected: S, action: A) -> impl View + use<F, S, A>
where
    F: Fn() -> String + 'static,
    S: Fn() -> bool + 'static,
    A: Fn() + 'static,
{
    hstack((image(symbol).symbol_size(14.0).color(tint).frame(20.0, 20.0), text(title), spacer()))
        .spacing(8.0)
        .padding(7.0)
        .background(move || if selected() { Color::Gray.opacity(0.22) } else { Color::Clear })
        .corner_radius(7.0)
        .hover_highlight(Color::Quaternary)
        .on_click(action)
}

/// A sidebar item: a page, or a section heading.
#[derive(Clone, PartialEq)]
struct Item {
    key: String,
    title: String,
    symbol: Option<String>,
    tint: Option<Color>,
}

impl Identifiable for Item {
    type Id = String;
    fn id(&self) -> String {
        self.key.clone()
    }
}

fn key_of(page: Page) -> String {
    match page {
        Page::Today => "today".into(),
        Page::Insights => "insights".into(),
        Page::Habit(id) => format!("habit:{id}"),
    }
}

fn page_of(key: &str) -> Option<Page> {
    match key {
        "today" => Some(Page::Today),
        "insights" => Some(Page::Insights),
        _ => key.strip_prefix("habit:").and_then(|id| id.parse().ok()).map(Page::Habit),
    }
}

/// AppKit's own sidebar (arrow keys, type-select, the system's selection),
/// with each habit's symbol in its colour.
fn sidebar_view(t: Tracker) -> impl View {
    let selected = Signal::new(Some(key_of(untrack(|| t.page.get()))));
    let item = |key: &str, title: String, symbol: &str, tint: Color| TreeNode::leaf(Item { key: key.into(), title, symbol: Some(symbol.into()), tint: Some(tint) });
    let sections = move || {
        let habits = t.habits.get().into_iter().map(|h| item(&key_of(Page::Habit(h.id)), h.name.clone(), &h.symbol, h.tint())).collect();
        vec![
            TreeNode::branch(Item { key: "overview".into(), title: tr("Overview"), symbol: None, tint: None }, vec![
                item("today", tr("Today"), "checkmark.circle.fill", Color::Green),
                item("insights", tr("Insights"), "chart.bar.fill", Color::Blue),
            ]),
            TreeNode::branch(Item { key: "habits".into(), title: tr("Habits"), symbol: None, tint: None }, habits),
        ]
    };
    vstack((
        sidebar(sections, |i: &Item| i.title.clone()).icon(|i: &Item| i.symbol.clone()).icon_color(|i: &Item| i.tint).selection(&selected),
        row("plus.circle", Color::Secondary, || tr("New Habit…"), || false, move || new_habit(t)).padding(8.0),
        // The selection and the page follow each other.
        watch(move || {
            if let Some(page) = selected.get().as_deref().and_then(page_of)
                && untrack(|| t.page.get()) != page
            {
                untrack(|| animated(|| t.page.set(page)));
            }
        }),
        watch(move || {
            let key = Some(key_of(t.page.get()));
            if untrack(|| selected.get()) != key {
                untrack(|| selected.set(key));
            }
        }),
    ))
    .spacing(0.0)
}

// ---- new habit ----

thread_local! {
    static SHEET: RefCell<Option<SheetHandle>> = const { RefCell::new(None) };
}

/// The New Habit sheet: a name, a symbol and a colour.
fn new_habit(t: Tracker) {
    let Some(w) = focused_window().or_else(|| windows().into_iter().find(|w| w.title().to_string() == "Habits")) else { return };
    // The sheet's signals are made in its builder: freed when it closes.
    let handle = sheet_with(
        &w,
        move || {
            let name = Signal::new(String::new());
            let symbol = Signal::new(0usize);
            let color = Signal::new(0usize);
            let close = || {
                if let Some(h) = SHEET.with_borrow_mut(Option::take) {
                    h.close();
                }
            };
            let symbols: Vec<AnyView> = SYMBOLS
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    AnyView::new(
                        zstack((
                            canvas(move |p| {
                                let c = if symbol.get() == i { PALETTE[color.get()].opacity(0.25) } else { Color::Quaternary.opacity(0.5) };
                                p.fill_circle(p.width() / 2.0, p.height() / 2.0, 17.0, c);
                            })
                            .min_size(36.0, 36.0),
                            image(s).symbol_size(15.0).color(Color::Secondary),
                        ))
                        .frame(36.0, 36.0)
                        .on_click(move || symbol.set(i))
                        .accessibility_label(&tr(SYMBOL_NAMES[i]))
                        .accessibility_value(move || if symbol.get() == i { tr("Selected") } else { String::new() }),
                    )
                })
                .collect();
            let colors: Vec<AnyView> = PALETTE
                .iter()
                .enumerate()
                .map(|(i, &c)| {
                    AnyView::new(
                        canvas(move |p| {
                            let (cx, cy) = (p.width() / 2.0, p.height() / 2.0);
                            p.fill_circle(cx, cy, 10.0, c);
                            if color.get() == i {
                                p.stroke_circle(cx, cy, 13.5, c, 2.0);
                            }
                        })
                        .min_size(30.0, 30.0)
                        .frame(30.0, 30.0)
                        // A button to VoiceOver, like the symbols (a canvas's own click is an image's).
                        .on_click(move || color.set(i))
                        .accessibility_label(&tr(&format!("{c:?}")))
                        .accessibility_value(move || if color.get() == i { tr("Selected") } else { String::new() }),
                    )
                })
                .collect();
            let content = vstack((
                text(tr("New Habit")).font(Font::title2().weight(Weight::Bold)),
                text_field(&name).placeholder(&tr("Name, like “Stretch for 10 minutes”")).accessibility_label(&tr("Name")),
                text(tr("Symbol")).font(Font::caption()).color(Color::Secondary),
                hstack(symbols).spacing(4.0),
                text(tr("Colour")).font(Font::caption()).color(Color::Secondary),
                hstack(colors).spacing(4.0),
                hstack((
                    spacer(),
                    button(tr("Cancel"), close),
                    button(tr("Add"), move || {
                        let id = t.add(&name.get_untracked(), SYMBOLS[symbol.get_untracked()], color.get_untracked());
                        animated(|| t.page.set(Page::Habit(id)));
                        close();
                    })
                    .default_action()
                    .disabled(move || name.get().trim().is_empty()),
                ))
                .spacing(8.0),
            ))
            .spacing(10.0)
            .alignment(Align::Start)
            .padding(20.0);
            Window::new(&tr("New Habit")).size(440.0, 262.0).view(content)
        },
        || {},
    );
    SHEET.with_borrow_mut(|s| *s = Some(handle));
}

// ---- the report ----

/// What Print and Export as PDF make: each habit's numbers and month.
fn report(t: Tracker) -> impl View {
    let today = untrack(|| t.today.get());
    let rows: Vec<AnyView> = untrack(|| t.habits.get())
        .into_iter()
        .map(|h| {
            let (id, tint) = (h.id, h.tint());
            let month = canvas(move |p| {
                for i in 0..30 {
                    let day = today - 29 + i;
                    let color = if t.is_done(id, day) { tint } else { Color::Quaternary };
                    p.fill_rounded_rect(i as f64 * 15.0, 0.0, 12.0, 12.0, 3.0, color);
                }
            })
            .min_size(450.0, 12.0)
            .frame(450.0, 12.0);
            AnyView::new(card(vstack((
                hstack((
                    image(&h.symbol).color(tint),
                    text(h.name.clone()).font(Font::headline()),
                    spacer(),
                    text(t_with("Streak {days} · best {best} · {percent} of 30 days", &[
                        ("days", &days(untrack(|| t.streak(id)))),
                        ("best", &days(untrack(|| t.best_streak(id)))),
                        ("percent", &percent(untrack(|| t.rate(id, 30)))),
                    ]))
                    .font(Font::caption())
                    .color(Color::Secondary),
                ))
                .spacing(8.0),
                month,
            ))
            .spacing(8.0)
            .alignment(Align::Start)))
        })
        .collect();
    vstack((
        text(tr("Habits")).font(Font::large_title().weight(Weight::Bold)),
        text(t_with("The 30 days to {date}", &[("date", &format_date(now(), "yMMMMd"))])).color(Color::Secondary),
        vstack(rows).spacing(10.0),
    ))
    .spacing(12.0)
    .alignment(Align::Start)
    .padding(8.0)
}

fn print_report(t: Tracker) {
    if let Some(w) = focused_window() {
        print(&w, &tr("Habits report"), report(t));
    }
}

fn export_report(t: Tracker) {
    let Some(w) = focused_window() else { return };
    save_panel(&w, &format!("{}.pdf", tr("Habits report")), None, move |path| {
        if let Some(path) = path
            && let Err(e) = save_pdf(report(t), &path)
        {
            eprintln!("Couldn't save the report: {e}");
        }
    });
}

// ---- the menu bar panel ----

fn menu_panel(t: Tracker) -> impl View {
    vstack((
        hstack((
            text(tr("Today")).font(Font::headline()),
            spacer(),
            text(move || t_with("{done} of {total}", &[("done", &t.done_today()), ("total", &t.habits.with(Vec::len))])).color(Color::Secondary),
        )),
        vstack((for_each(move || t.habits.get(), move |h: Habit| {
            let (id, tint) = (h.id, h.tint());
            hstack((image(&h.symbol).color(tint).frame(20.0, 20.0), text(move || t.habit(id).map(|h| h.name).unwrap_or_default()), spacer(), check_mark(t, id, tint)))
                .spacing(8.0)
                .padding(4.0)
                .hover_highlight(Color::Quaternary)
                .corner_radius(6.0)
                .on_click(move || t.toggle(id, t.today.get_untracked()))
        }),))
        .spacing(2.0),
    ))
    .spacing(8.0)
    .padding(14.0)
    .width(280.0)
}

fn main() {
    let checks = std::env::var("CHECK").is_ok_and(|v| v == "all");
    // `SOAK=<minutes>`: random actions for that long (see `soak` below).
    let soaking = std::env::var("SOAK").is_ok();
    // `DEMO=1`: a fast, scripted tour of everything, sized to the screen.
    let demo = std::env::var("DEMO").is_ok();
    // The checks and soaks start from the sample, under their own keys.
    let data = stored(if checks { "habits.check" } else if soaking { "habits.soak" } else if demo { "habits.demo" } else { "habits" }, String::new());
    let today = Signal::new(ferrokit::today());
    let (habits, done) = match untrack(|| data.get()) {
        text if !checks && !soaking && !demo && !text.is_empty() => load(&text),
        _ => sample(today.get_untracked()),
    };
    let t = Tracker { habits: Signal::new(habits), done: Signal::new(done), page: Signal::new(Page::Today), today };
    if checks || soaking || demo {
        ferrokit::testing::hold_notifications();
    }
    if soaking || demo {
        soak::STATE.with(|s| s.set(Some(t)));
    }
    if checks {
        self_checks::STATE.with(|s| *s.borrow_mut() = Some((t, data)));
    }
    let content = split((
        sidebar_view(t).material(Material::Sidebar),
        dynamic(move || match t.page.get() {
            Page::Today => AnyView::new(today_view(t).transition(Transition::Fade)),
            Page::Insights => AnyView::new(insights(t).transition(Transition::Fade)),
            Page::Habit(id) => AnyView::new(habit_view(t, id).transition(Transition::Fade)),
        }),
        // Saved on every change; the day rolls over at midnight.
        watch(move || data.set(save(&t.habits.get(), &t.done.get()))),
    ))
    .position(230.0);
    App::new("Habits")
        .window(
            Window::new("Habits")
                .size(1060.0, 760.0)
                .full_size_content()
                .toolbar_style(ToolbarStyle::Unified)
                .toolbar(vec![toolbar_command("new", &tr("New Habit"), "plus", NEW_HABIT), toolbar_command("print", &tr("Print"), "printer", PRINT_REPORT)])
                .view(content)
                .on_command(NEW_HABIT, move || new_habit(t))
                .on_command(PRINT_REPORT, move || print_report(t))
                .on_command(EXPORT_REPORT, move || export_report(t))
                .remember_frame("habits"),
        )
        .menu(
            "File",
            vec![
                menu_command(tr("New Habit…"), NEW_HABIT).key("n"),
                menu_separator(),
                menu_command(tr("Export Report as PDF…"), EXPORT_REPORT).key("E"),
                menu_command(tr("Print Report…"), PRINT_REPORT).key("p"),
            ],
        )
        .menu(
            &tr("View"),
            vec![
                menu_item(tr("Today"), move || animated(|| t.page.set(Page::Today))).key("1"),
                menu_item(tr("Insights"), move || animated(|| t.page.set(Page::Insights))).key("2"),
            ],
        )
        .menu_bar_extra(
            MenuBarExtra::new("checkmark.circle")
                .title(move || format!("{}/{}", t.done_today(), t.habits.with(Vec::len)))
                .accessibility_label(&tr("Habits done today"))
                .panel(menu_panel(t)),
        )
        .on_launch(move || {
            // At midnight a new day starts.
            spawn_day_watch(today);
            if checks {
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(600)), |()| self_checks::run());
            }
            if soaking {
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(600)), |()| soak::run());
            }
            if demo {
                spawn(|| std::thread::sleep(std::time::Duration::from_millis(400)), |()| soak::demo());
            }
        })
        .run();
}

/// The soak test's actions: what a user does, at random, for a long time.
mod soak {
    use std::cell::Cell;

    use ferrokit::testing::input::{self, Key, Mod};
    use ferrokit::testing::soak::{Rng, Settings, action};
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindow};

    use super::{Page, Tracker};

    thread_local! {
        pub static STATE: Cell<Option<Tracker>> = const { Cell::new(None) };
    }

    fn views(w: &NSWindow) -> Vec<Retained<NSView>> {
        fn all(v: &NSView, out: &mut Vec<Retained<NSView>>) {
            for s in v.subviews().iter() {
                out.push(s.clone());
                all(&s, out);
            }
        }
        let mut out = Vec::new();
        if let Some(c) = w.contentView() {
            all(&c, &mut out);
        }
        out
    }

    fn label_of(v: &NSView) -> String {
        objc2_app_kit::NSAccessibility::accessibilityLabel(v).map(|l| l.to_string()).unwrap_or_default()
    }

    fn main_window() -> Option<Retained<NSWindow>> {
        windows().into_iter().find(|w| w.title().to_string() == "Habits" && w.isVisible())
    }

    fn later(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    fn button_in(w: &NSWindow, title: &str) -> Option<Retained<objc2_app_kit::NSButton>> {
        views(w).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok()).find(|b| b.title().to_string() == title)
    }

    /// The demo: a scripted tour of everything, fast, in a window sized to
    /// the screen. It starts with nothing done today and ends with all done.
    pub fn demo() {
        let t = STATE.with(Cell::get).unwrap();
        let Some(w) = main_window() else { return };
        let mtm = objc2::MainThreadMarker::new().unwrap();
        // Most of the screen, centred.
        if let Some(screen) = w.screen().or_else(|| objc2_app_kit::NSScreen::mainScreen(mtm)) {
            let v = screen.visibleFrame();
            let (wd, ht) = ((v.size.width * 0.97).min(1600.0), (v.size.height * 0.97).min(1100.0));
            let origin = objc2_foundation::NSPoint::new(v.origin.x + (v.size.width - wd) / 2.0, v.origin.y + (v.size.height - ht) / 2.0);
            let frame = objc2_foundation::NSRect::new(origin, objc2_foundation::NSSize::new(wd, ht));
            w.setFrame_display(frame, true);
            eprintln!("demo window {:.0} × {:.0} on a {:.0} × {:.0} screen", wd, ht, v.size.width, v.size.height);
        }
        set_appearance(Appearance::Light);
        let today = untrack(|| t.today.get());
        untrack(|| t.done.update(|d| d.retain(|&(_, day)| day != today)));
        untrack(|| t.page.set(Page::Today));
        input::ready(&w);

        // The tour, a step every `pace` milliseconds.
        let mut steps: Vec<(u64, Box<dyn FnOnce()>)> = Vec::new();
        let card = move |name: String| -> Box<dyn FnOnce()> {
            Box::new(move || {
                if let Some(w) = main_window()
                    && let Some(c) = views(&w).into_iter().find(|v| v.class().name().to_string_lossy() == "FerrokitInteraction" && label_of(v).starts_with(&format!("{name}, ")))
                {
                    input::click(&c);
                }
            })
        };
        let key = |k: Key, mods: &'static [Mod]| -> Box<dyn FnOnce()> {
            Box::new(move || {
                if let Some(w) = main_window() {
                    input::key(&w, k, mods);
                }
            })
        };
        // Paints days on the calendar: a few clicks on random past days.
        let paint = move |n: usize, seed: u64| -> Box<dyn FnOnce()> {
            Box::new(move || {
                let mut r = Rng::new(seed);
                let Some(w) = main_window() else { return };
                if let Some(c) = views(&w).into_iter().find(|v| label_of(v).contains(": done ")) {
                    let b = c.bounds();
                    for _ in 0..n {
                        input::click_at(&c, 30.0 + r.below((b.size.width - 40.0).max(1.0) as usize) as f64, 18.0 + r.below(130) as f64);
                    }
                }
            })
        };
        let names: Vec<String> = untrack(|| t.habits.get()).into_iter().map(|h| h.name).collect();
        // Today: tick everything off, the ring filling.
        steps.push((900, Box::new(|| {})));
        for n in &names {
            steps.push((450, card(n.clone())));
        }
        steps.push((900, card(names[1].clone())));
        steps.push((500, card(names[1].clone())));
        // Each habit's page: ↓ through the sidebar, painting its calendar.
        // A real click on Insights in the sidebar (so the arrow keys go to it).
        steps.push((700, Box::new(|| {
            let Some(w) = main_window() else { return };
            if let Some(o) = views(&w).into_iter().find_map(|v| v.downcast::<objc2_app_kit::NSOutlineView>().ok()) {
                let row = (0..o.numberOfRows()).find(|&r| {
                    o.viewAtColumn_row_makeIfNecessary(0, r, true)
                        .and_then(|v| v.downcast::<objc2_app_kit::NSTableCellView>().ok())
                        // SAFETY: reading the cell's outlet on the main thread.
                        .and_then(|c| unsafe { c.textField() })
                        .is_some_and(|f| f.stringValue().to_string() == "Insights")
                });
                if let Some(r) = row {
                    let rect = o.rectOfRow(r);
                    input::click_at(&o, rect.origin.x + 40.0, rect.origin.y + rect.size.height / 2.0);
                }
            }
        })));
        steps.push((1400, key(Key::Down, &[])));
        for i in 0..names.len() {
            steps.push((450, paint(6, 11 + i as u64)));
            steps.push((900, key(Key::Down, &[])));
        }
        // Insights, then dark, back through the pages.
        steps.push((1400, key(Key::Char('2'), &[Mod::Command])));
        steps.push((1600, Box::new(|| set_appearance(Appearance::Dark))));
        steps.push((1400, key(Key::Char('1'), &[Mod::Command])));
        for n in names.iter().take(3) {
            steps.push((380, card(n.clone())));
        }
        for n in names.iter().take(3) {
            steps.push((380, card(n.clone())));
        }
        steps.push((900, Box::new(move || untrack(|| t.page.set(Page::Habit(4))))));
        steps.push((600, paint(10, 99)));
        steps.push((1200, key(Key::Char('2'), &[Mod::Command])));
        // A new habit through the sheet: a name, a symbol, a colour.
        steps.push((1500, key(Key::Char('n'), &[Mod::Command])));
        steps.push((600, Box::new(|| {
            if let Some(s) = main_window().map(|_| ()).and(windows().into_iter().find_map(|w| w.attachedSheet())) {
                input::type_text(&s, "Piano");
            }
        })));
        steps.push((700, Box::new(|| {
            if let Some(s) = windows().into_iter().find_map(|w| w.attachedSheet()) {
                // The music note, then pink.
                let picks: Vec<_> = views(&s).into_iter().filter(|v| v.class().name().to_string_lossy() == "FerrokitInteraction").collect();
                if let Some(v) = picks.get(8) {
                    input::click(v);
                }
                let dots: Vec<_> = views(&s).into_iter().filter(|v| label_of(v) == "Pink").collect();
                if let Some(v) = dots.first() {
                    input::click(v);
                }
            }
        })));
        steps.push((900, Box::new(|| {
            if let Some(s) = windows().into_iter().find_map(|w| w.attachedSheet())
                && let Some(b) = button_in(&s, "Add")
            {
                input::click(&b);
            }
        })));
        steps.push((900, paint(14, 7)));
        steps.push((1000, Box::new(|| set_appearance(Appearance::Light))));
        // The menu bar's panel, then back to Today, all done.
        steps.push((900, Box::new(move || {
            if let Some((item, _)) = ferrokit::testing::menu_bar_extras().first()
                && let Some(b) = item.button(mtm)
            {
                // SAFETY: clicking a button on the main thread.
                unsafe { b.performClick(None) };
            }
        })));
        steps.push((1800, Box::new(move || {
            if let Some((_, Some(p))) = ferrokit::testing::menu_bar_extras().first() {
                p.close();
            }
            untrack(|| animated(|| t.page.set(Page::Today)));
        })));
        let all: Vec<String> = names.iter().cloned().chain(["Piano".to_string()]).collect();
        steps.push((800, Box::new(move || {
            let today = untrack(|| t.today.get());
            for (i, n) in all.iter().enumerate() {
                let id = untrack(|| t.habits.get()).iter().find(|h| &h.name == n).map(|h| h.id);
                // The rest done, one by one (a card below the fold too).
                if let Some(id) = id.filter(|&id| !untrack(|| t.is_done(id, today))) {
                    later(250 * i as u64, move || untrack(|| animated(|| t.toggle(id, today))));
                }
            }
        })));
        play(steps);
    }

    fn play(mut steps: Vec<(u64, Box<dyn FnOnce()>)>) {
        if steps.is_empty() {
            eprintln!("demo done");
            return;
        }
        let (wait, step) = steps.remove(0);
        later(wait, move || {
            step();
            play(steps);
        });
    }

    pub fn run() {
        let t = STATE.with(Cell::get).unwrap();
        let Some(w) = main_window() else { return };
        input::ready(&w);
        // Every action works on the window as it is now (a sheet or alert
        // may be up: those actions just do nothing).
        let win = || main_window().filter(|w| w.attachedSheet().is_none());
        let actions = vec![
            action("toggle-card", 6, move |r: &mut Rng| {
                let Some(w) = win() else { return };
                let cards: Vec<_> = views(&w).into_iter().filter(|v| v.class().name().to_string_lossy() == "FerrokitInteraction" && label_of(v).contains(", ")).collect();
                if !cards.is_empty() {
                    input::click(&cards[r.below(cards.len())]);
                }
            }),
            action("sidebar-click", 4, move |r: &mut Rng| {
                let Some(w) = win() else { return };
                if let Some(o) = views(&w).into_iter().find_map(|v| v.downcast::<objc2_app_kit::NSOutlineView>().ok()) {
                    let row = r.below(o.numberOfRows().max(1) as usize) as isize;
                    let rect = o.rectOfRow(row);
                    input::click_at(&o, rect.origin.x + 40.0, rect.origin.y + rect.size.height / 2.0);
                }
            }),
            action("arrow-key", 3, move |r: &mut Rng| {
                if let Some(w) = win() {
                    input::key(&w, *r.pick(&[Key::Up, Key::Down]), &[]);
                }
            }),
            action("page-shortcut", 2, move |r: &mut Rng| {
                if let Some(w) = win() {
                    input::key(&w, Key::Char(*r.pick(&['1', '2'])), &[Mod::Command]);
                }
            }),
            action("calendar-click", 3, move |r: &mut Rng| {
                let Some(w) = win() else { return };
                if let Some(c) = views(&w).into_iter().find(|v| label_of(v).contains(": done ")) {
                    let b = c.bounds();
                    input::click_at(&c, 30.0 + r.below((b.size.width - 30.0).max(1.0) as usize) as f64, 18.0 + r.below(133) as f64);
                }
            }),
            action("reminder", 1, move |_: &mut Rng| {
                if let Some(b) = win().and_then(|w| button_in(&w, "Remind me every day at")) {
                    input::click(&b);
                }
            }),
            action("new-habit", 1, move |r: &mut Rng| {
                let Some(w) = win() else { return };
                if untrack(|| t.habits.with(Vec::len)) >= 9 {
                    return;
                }
                input::key(&w, Key::Char('n'), &[Mod::Command]);
                let name = r.pick(&["Stretch", "Journal", "No sugar", "Floss", "Call a friend", "Piano"]).to_string();
                let add = r.chance(0.7);
                later(500, move || {
                    let Some(s) = main_window().and_then(|w| w.attachedSheet()) else { return };
                    input::type_text(&s, &name);
                    later(300, move || {
                        if let Some(b) = button_in(&s, if add { "Add" } else { "Cancel" }) {
                            input::click(&b);
                        }
                    });
                });
            }),
            action("delete-habit", 1, move |_: &mut Rng| {
                let Some(w) = win() else { return };
                // The Delete… button and its alert, as a user does it.
                if untrack(|| t.habits.with(Vec::len)) <= 5 {
                    return;
                }
                let Some(last) = untrack(|| t.habits.get()).last().map(|h| h.id) else { return };
                untrack(|| t.page.set(Page::Habit(last)));
                later(500, move || {
                    let Some(w) = main_window() else { return };
                    if let Some(b) = button_in(&w, "Delete…") {
                        input::click(&b);
                    }
                    later(500, move || {
                        if let Some(alert) = main_window().and_then(|w| w.attachedSheet())
                            && let Some(b) = button_in(&alert, "Delete")
                        {
                            input::click(&b);
                        }
                    });
                });
                let _ = w;
            }),
            action("resize", 1, move |r: &mut Rng| {
                if let Some(w) = win() {
                    let f = w.frame();
                    let size = objc2_foundation::NSSize::new(760.0 + r.below(600) as f64, 560.0 + r.below(400) as f64);
                    w.setFrame_display(objc2_foundation::NSRect::new(f.origin, size), true);
                }
            }),
            action("appearance", 1, move |r: &mut Rng| {
                if r.chance(0.3) {
                    set_appearance(*r.pick(&[Appearance::Light, Appearance::Dark, Appearance::System]));
                }
            }),
            action("menu-bar-panel", 1, move |_: &mut Rng| {
                let mtm = objc2::MainThreadMarker::new().unwrap();
                if let Some((item, _)) = ferrokit::testing::menu_bar_extras().first()
                    && let Some(b) = item.button(mtm)
                {
                    // SAFETY: clicking a button on the main thread.
                    unsafe { b.performClick(None) };
                }
            }),
            action("report-pdf", 1, move |r: &mut Rng| {
                if r.chance(0.2) {
                    let data = pdf_data(super::report(t));
                    assert!(data.starts_with(b"%PDF"), "the report didn't print");
                }
            }),
        ];
        ferrokit::testing::soak::run(Settings::from_env(), actions);
    }
}

/// Moves `today` on once a minute when the date changes.
fn spawn_day_watch(today: Signal<i64>) {
    spawn(
        || std::thread::sleep(std::time::Duration::from_secs(60)),
        move |()| {
            today.set_if_changed(ferrokit::today());
            spawn_day_watch(today);
        },
    );
}

mod self_checks {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use ferrokit::testing::{ax_audit, canvas_click, canvas_drawing, menu_bar_extras, press, sent_notifications};
    use ferrokit::*;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindow};

    use super::{Page, Tracker};

    thread_local! {
        pub static STATE: RefCell<Option<(Tracker, Signal<String>)>> = const { RefCell::new(None) };
    }

    type Check = Rc<dyn Fn(&str, bool, String)>;

    fn later_ms(ms: u64, f: impl FnOnce() + 'static) {
        spawn(move || std::thread::sleep(std::time::Duration::from_millis(ms)), move |()| f());
    }

    fn all(v: &NSView, out: &mut Vec<Retained<NSView>>) {
        for s in v.subviews().iter() {
            out.push(s.clone());
            all(&s, out);
        }
    }

    fn views(w: &NSWindow) -> Vec<Retained<NSView>> {
        let mut out = Vec::new();
        all(&w.contentView().unwrap(), &mut out);
        out
    }

    fn texts(w: &NSWindow) -> Vec<String> {
        views(w).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).map(|f| f.stringValue().to_string()).collect()
    }

    /// The canvases whose VoiceOver description starts with `start`.
    fn canvases(w: &NSWindow, start: &str) -> Vec<Retained<NSView>> {
        views(w)
            .into_iter()
            .filter(|v| canvas_drawing(v).is_some() && objc2_app_kit::NSAccessibility::accessibilityLabel(&**v).is_some_and(|l| l.to_string().starts_with(start)))
            .collect()
    }

    /// The clickable view VoiceOver names `name` exactly (a sidebar row),
    /// or with more after it (`card`: a card, named by all its text).
    fn clickable(w: &NSWindow, name: &str, card: bool) -> Option<Retained<NSView>> {
        views(w).into_iter().find(|v| {
            let label = objc2_app_kit::NSAccessibility::accessibilityLabel(&**v).map(|l| l.to_string()).unwrap_or_default();
            v.class().name().to_string_lossy() == "FerrokitInteraction" && if card { label.starts_with(&format!("{name}, ")) } else { label == name }
        })
    }

    /// The sidebar, the row titled `name`, and its icon's colour.
    fn sidebar_row(w: &NSWindow, name: &str) -> (Option<Retained<objc2_app_kit::NSOutlineView>>, Option<usize>, Option<String>) {
        let Some(o) = views(w).into_iter().find_map(|v| v.downcast::<objc2_app_kit::NSOutlineView>().ok()) else { return (None, None, None) };
        for r in 0..o.numberOfRows() {
            let Some(cell) = o.viewAtColumn_row_makeIfNecessary(0, r, true).and_then(|v| v.downcast::<objc2_app_kit::NSTableCellView>().ok()) else { continue };
            // SAFETY: reading the cell's outlets on the main thread.
            let (text, image) = unsafe { (cell.textField(), cell.imageView()) };
            if text.is_some_and(|t| t.stringValue().to_string() == name) {
                let tint = image.and_then(|i| i.contentTintColor()).and_then(|c| Color::from_ns(&c)).map(|c| format!("{c:?}"));
                return (Some(o), Some(r as usize), tint);
            }
        }
        (Some(o), None, None)
    }

    /// The opacity of the background behind a clickable row.
    fn row_fill(w: &NSWindow, row: &NSView) -> Option<f64> {
        let surface = views(w).into_iter().find(|v| v.class().name().to_string_lossy() == "FerrokitSurface" && v.frame() == row.frame())?;
        let color = surface.layer()?.backgroundColor()?;
        Some(objc2_core_graphics::CGColor::alpha(Some(&color)))
    }

    fn click(v: &NSView) {
        press(v, true);
        press(v, false);
    }

    fn snap(w: &NSWindow, name: &str) {
        let Ok(dir) = std::env::var("SNAP") else { return };
        let Some(rep) = ferrokit::testing::capture_window(w) else { return };
        // SAFETY: encoding a bitmap as PNG with no options.
        if let Some(png) = unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &objc2_foundation::NSDictionary::new()) } {
            png.writeToFile_atomically(&objc2_foundation::NSString::from_str(&format!("{dir}/habits-{name}.png")), true);
        }
    }

    pub fn run() {
        let w = windows().into_iter().find(|w| w.title().to_string() == "Habits").unwrap();
        let (t, data) = STATE.with(|s| s.borrow_mut().take()).unwrap();
        let failed = Rc::new(Cell::new(0));
        let check: Check = {
            let failed = failed.clone();
            Rc::new(move |name: &str, pass: bool, note: String| {
                eprintln!("[check] {name} {} {note}", if pass { "PASS" } else { "FAIL" });
                failed.set(failed.get() + u32::from(!pass));
            })
        };
        let problems = ax_audit(&w);
        check("accessibility", problems.is_empty(), format!("{problems:?}"));
        let shown = texts(&w);
        let date = format_date(super::now(), "EEEEdMMMM");
        check("today-header", shown.contains(&date) && shown.contains(&"2 of 5 done".to_string()), format!("{date:?} and \"2 of 5 done\" in {:?}", &shown[..shown.len().min(12)]));
        let ring = canvases(&w, "40%").into_iter().next();
        let arc = ring.as_ref().and_then(|r| canvas_drawing(r)).and_then(|d| d.into_iter().find(|o| o.starts_with("stroke arc")));
        check("progress-ring", arc.as_deref().is_some_and(|a| a.contains(" 0-144 ")), format!("{arc:?} (0-144 degrees: 2 of 5)"));
        let streaks = ["12 days", "5 days"].iter().all(|s| shown.contains(&s.to_string()));
        check("streaks", streaks && untrack(|| t.streak(1)) == 12 && untrack(|| t.best_streak(1)) >= 12, format!("walk {} (best {}), read {}", untrack(|| t.streak(1)), untrack(|| t.best_streak(1)), untrack(|| t.streak(2))));
        snap(&w, "today");
        // A done card keeps its colour under the pointer.
        let row = clickable(&w, "Walk 8,000 steps", true);
        let before = row.as_ref().and_then(|r| row_fill(&w, r));
        if let Some(r) = &row {
            ferrokit::testing::hover(r, true);
        }

        // Clicking a card marks it done: the summary, ring, card colour and storage follow.
        let today = untrack(|| t.today.get());
        let card = clickable(&w, "Read 20 pages", true);
        if let Some(c) = &card {
            click(c);
        }
        let check2 = check.clone();
        later_ms(400, move || {
            let shown = texts(&w);
            let arc = canvases(&w, "60%").into_iter().next().and_then(|r| canvas_drawing(&r)).and_then(|d| d.into_iter().find(|o| o.starts_with("stroke arc")));
            let after = row.as_ref().and_then(|r| row_fill(&w, r));
            check2("hover-keeps-background", before.is_some_and(|b| b > 0.1) && after == before, format!("done card {before:?}, hovered {after:?}"));
            if let Some(r) = &row {
                ferrokit::testing::hover(r, false);
            }
            let saved = untrack(|| data.get());
            check2(
                "mark-done",
                card.is_some() && untrack(|| t.is_done(2, today)) && shown.contains(&"3 of 5 done".to_string()) && arc.as_deref().is_some_and(|a| a.contains(" 0-216 ")) && saved.contains(&format!("d\t2\t{today}\n")),
                format!("card found {}; done {}; ring {arc:?}; saved {}", card.is_some(), untrack(|| t.is_done(2, today)), saved.contains(&format!("d\t2\t{today}"))),
            );
            let (habits, done) = super::load(&saved);
            check2("storage-round-trip", untrack(|| t.habits.get()) == habits && untrack(|| t.done.get()) == done, format!("{} habits, {} days", habits.len(), done.len()));
            // The menu bar title and panel.
            let mtm = objc2::MainThreadMarker::new().unwrap();
            let extras = menu_bar_extras();
            let title = extras.first().and_then(|(i, _)| i.button(mtm)).map(|b| b.title().to_string());
            let (item, popover) = extras[0].clone();
            if let Some(b) = item.button(mtm) {
                // SAFETY: clicking a button on the main thread.
                unsafe { b.performClick(None) };
            }
            let check3 = check2.clone();
            later_ms(400, move || {
                let inside = popover.as_ref().and_then(|p| p.contentViewController()).map(|c| {
                    let mut out = Vec::new();
                    all(&c.view(), &mut out);
                    out.into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).map(|f| f.stringValue().to_string()).collect::<Vec<_>>()
                });
                let inside = inside.unwrap_or_default();
                check3(
                    "menu-bar",
                    title.as_deref() == Some(" 3/5") && popover.as_ref().is_some_and(|p| p.isShown()) && inside.contains(&"Meditate".to_string()) && inside.contains(&"3 of 5".to_string()),
                    format!("title {title:?}; panel {inside:?}"),
                );
                if let Some(p) = &popover {
                    p.close();
                }
                habit_phase(w, t, check3, failed);
            });
        });
    }

    fn habit_phase(w: Retained<NSWindow>, t: Tracker, check: Check, failed: Rc<Cell<u32>>) {
        w.makeKeyAndOrderFront(None);
        // The sidebar is AppKit's: select the row as a click (or arrow key) would.
        let (outline, row, tint) = sidebar_row(&w, "Walk 8,000 steps");
        if let (Some(o), Some(r)) = (&outline, row) {
            o.selectRowIndexes_byExtendingSelection(&objc2_foundation::NSIndexSet::indexSetWithIndex(r), false);
        }
        check(
            "sidebar",
            outline.as_ref().is_some_and(|o| o.style() == objc2_app_kit::NSTableViewStyle::SourceList) && tint.as_deref() == Some("Green"),
            format!("source list {}; row {row:?}; icon tint {tint:?}", outline.as_ref().is_some_and(|o| o.style() == objc2_app_kit::NSTableViewStyle::SourceList)),
        );
        later_ms(500, move || {
            let page = untrack(|| t.page.get());
            let shown = texts(&w);
            let stats = ["Current streak", "Best streak", "Last 30 days", "12 days"].iter().all(|s| shown.contains(&s.to_string()));
            check("habit-page", page == Page::Habit(1) && stats, format!("page {page:?}; stats {stats}"));
            snap(&w, "habit");
            let cal = canvases(&w, "Walk 8,000 steps: done").into_iter().next();
            let today = untrack(|| t.today.get());
            let first = super::first_shown(today);
            let done_shown = (first..=today).filter(|&d| untrack(|| t.is_done(1, d))).count();
            let fills = cal.as_ref().and_then(|c| canvas_drawing(c)).map(|d| d.iter().filter(|o| o.starts_with("fill rect") && o.ends_with("Green")).count()).unwrap_or(0);
            check("calendar", fills == done_shown && fills > 50, format!("{fills} green days drawn, {done_shown} done since {first}"));
            // A click on a day two weeks ago changes it.
            let day = today - 14;
            let (col, row) = ((day - first) / 7, (day - first) % 7);
            let was = untrack(|| t.is_done(1, day));
            let clicked = cal.as_ref().is_some_and(|c| canvas_click(c, 30.0 + col as f64 * 19.0 + 7.0, 18.0 + row as f64 * 19.0 + 7.0));
            check("calendar-click", clicked && untrack(|| t.is_done(1, day)) != was, format!("clicked {clicked}; {was} → {}", untrack(|| t.is_done(1, day))));
            // The reminder.
            let boxes: Vec<Retained<objc2_app_kit::NSButton>> =
                views(&w).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok()).filter(|b| b.title().to_string() == "Remind me every day at").collect();
            if let Some(b) = boxes.first() {
                // SAFETY: as above.
                unsafe { b.performClick(None) };
            }
            later_ms(200, move || {
                let sent = sent_notifications();
                let n = sent.iter().find(|n| n.id == "habit-1");
                check(
                    "reminder",
                    n.is_some_and(|n| n.daily == Some((8, 0)) && n.title == "Walk 8,000 steps") && untrack(|| t.habit(1)).and_then(|h| h.reminder) == Some((8, 0)),
                    format!("{sent:?}"),
                );
                // A time with minutes, picked as the user does: the reminder moves to it.
                use objc2_foundation::{NSCalendar, NSCalendarOptions};
                let time_picker = views(&w).into_iter().find_map(|v| v.downcast::<objc2_app_kit::NSDatePicker>().ok());
                if let Some(p) = &time_picker
                    && let Some(d) = NSCalendar::currentCalendar().dateBySettingHour_minute_second_ofDate_options(7, 45, 0, &p.dateValue(), NSCalendarOptions::empty())
                {
                    p.setDateValue(&d);
                    // SAFETY: the picker's own action, sent to its own target.
                    unsafe { p.sendAction_to(p.action(), p.target().as_deref()) };
                }
                later_ms(100, move || {
                    let moved = sent_notifications().iter().find(|n| n.id == "habit-1").and_then(|n| n.daily);
                    check(
                        "reminder-minutes",
                        time_picker.is_some() && moved == Some((7, 45)) && untrack(|| t.habit(1)).and_then(|h| h.reminder) == Some((7, 45)),
                        format!("picker {}; notification daily at {moved:?}", time_picker.is_some()),
                    );
                    untrack(|| t.set_reminder(1, None));
                    check("reminder-off", !sent_notifications().iter().any(|n| n.id == "habit-1"), format!("{:?}", sent_notifications()));
                    new_phase(w, t, check, failed);
                });
            });
        });
    }

    fn new_phase(w: Retained<NSWindow>, t: Tracker, check: Check, failed: Rc<Cell<u32>>) {
        perform_in(Some(&w), super::NEW_HABIT);
        later_ms(600, move || {
            let sheet = w.attachedSheet();
            let fields: Vec<Retained<objc2_app_kit::NSTextField>> =
                sheet.as_ref().map(|s| views(s)).unwrap_or_default().into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).filter(|f| f.isEditable()).collect();
            if let Some(f) = fields.first() {
                f.setStringValue(&objc2_foundation::NSString::from_str("Stretch"));
                // SAFETY: AppKit's notification name, posted for the field.
                unsafe {
                    objc2_foundation::NSNotificationCenter::defaultCenter().postNotificationName_object(objc2_app_kit::NSControlTextDidChangeNotification, Some(&**f))
                };
            }
            let add = sheet.as_ref().map(|s| views(s)).unwrap_or_default().into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok()).find(|b| b.title().to_string() == "Add");
            later_ms(400, move || {
                if let Some(s) = &sheet {
                    snap(s, "new-habit");
                }
                let enabled = add.as_ref().is_some_and(|b| b.isEnabled());
                if let Some(b) = &add {
                    // SAFETY: as above.
                    unsafe { b.performClick(None) };
                }
                later_ms(700, move || {
                    let habits = untrack(|| t.habits.get());
                    let new = habits.last().cloned();
                    let page = untrack(|| t.page.get());
                    check(
                        "new-habit",
                        enabled && habits.len() == 6 && new.as_ref().is_some_and(|h| h.name == "Stretch") && page == Page::Habit(6) && w.attachedSheet().is_none() && texts(&w).contains(&"Stretch".to_string()),
                        format!("Add enabled {enabled}; {} habits, last {:?}; page {page:?}", habits.len(), new.map(|h| h.name)),
                    );
                    // The menu bar's panel, built at launch with 5 rows, fits the 6th.
                    let mtm = objc2::MainThreadMarker::new().unwrap();
                    let (item, popover) = menu_bar_extras()[0].clone();
                    if let Some(b) = item.button(mtm) {
                        // SAFETY: clicking a button on the main thread.
                        unsafe { b.performClick(None) };
                    }
                    later_ms(400, move || {
                        let content = popover.as_ref().and_then(|p| p.contentViewController()).map(|c| c.view());
                        let row = content.as_ref().and_then(|c| {
                            let mut out = Vec::new();
                            all(c, &mut out);
                            let f = out.into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).find(|f| f.stringValue().to_string() == "Stretch")?;
                            Some(f.convertRect_toView(f.bounds(), Some(c)))
                        });
                        let bounds = content.as_ref().map(|c| c.bounds());
                        let inside = row.zip(bounds).is_some_and(|(r, b)| r.origin.y >= b.origin.y && r.origin.y + r.size.height <= b.origin.y + b.size.height);
                        check("menu-bar-grows", inside, format!("Stretch at {row:?} in {bounds:?}"));
                        if let Some(p) = &popover {
                            p.close();
                        }
                        untrack(|| t.delete(6));
                        later_ms(500, move || {
                            check("delete-habit", untrack(|| t.habits.with(Vec::len)) == 5 && untrack(|| t.page.get()) == Page::Today, format!("{} habits", untrack(|| t.habits.with(Vec::len))));
                            insights_phase(w, t, check, failed);
                        });
                    });
                });
            });
        });
    }

    fn insights_phase(w: Retained<NSWindow>, t: Tracker, check: Check, failed: Rc<Cell<u32>>) {
        untrack(|| t.page.set(Page::Insights));
        later_ms(600, move || {
            let charts: Vec<String> = views(&w)
                .into_iter()
                .filter_map(|v| objc2_app_kit::NSAccessibility::accessibilityLabel(&*v).map(|l| l.to_string()))
                .filter(|l| l.contains("chart"))
                .collect();
            let all_drawn: Vec<String> = views(&w).iter().filter_map(|v| canvas_drawing(v)).flatten().collect();
            let legend = ["Walk 8,000 steps", "Meditate", "In bed by 23:00"].iter().all(|n| all_drawn.iter().any(|d| d.contains(&format!("\"{n}\""))));
            check("insights", charts.len() >= 3 && legend, format!("charts {charts:?}; legend {legend}"));
            snap(&w, "insights");
            // The report as a PDF.
            let data = pdf_data(super::report(t));
            // SAFETY: PDFKit's initialiser; None when unreadable.
            let doc = unsafe { objc2_pdf_kit::PDFDocument::initWithData(objc2::AnyThread::alloc(), &objc2_foundation::NSData::with_bytes(&data)) };
            let body = doc.as_ref().and_then(|d| unsafe { d.string() }).map(|s| s.to_string()).unwrap_or_default();
            let names = ["Walk 8,000 steps", "Read 20 pages", "Drink water", "Meditate", "In bed by 23:00"].iter().all(|n| body.contains(n));
            check("report-pdf", data.starts_with(b"%PDF") && names && body.contains("Habits"), format!("{} bytes; names {names}", data.len()));
            if let Ok(dir) = std::env::var("SNAP") {
                let _ = std::fs::write(format!("{dir}/habits-report.pdf"), &data);
            }
            input_phase(w, t, check, failed);
        });
    }

    /// Real clicks and keys: a card, the calendar, the sidebar's arrow
    /// keys, and the New Habit sheet by ⌘N, typing and a click on Add.
    fn input_phase(w: Retained<NSWindow>, t: Tracker, check: Check, failed: Rc<Cell<u32>>) {
        use ferrokit::testing::input::{self, Key, Mod};
        input::ready(&w);
        untrack(|| t.page.set(Page::Today));
        later_ms(600, move || {
            if !input::ready(&w) {
                check("input-ready", false, "the window couldn't become key".into());
                return dark_phase(w, t, check, failed);
            }
            let today = untrack(|| t.today.get());
            let was = untrack(|| t.is_done(4, today));
            if let Some(card) = clickable(&w, "Meditate", true) {
                input::click(&card);
            }
            later_ms(400, move || {
                check("input-card", untrack(|| t.is_done(4, today)) != was, format!("Meditate done {was} → {}", untrack(|| t.is_done(4, today))));
                // The sidebar: a click on Insights, then ↓ (past the Habits heading).
                let (outline, row, _) = sidebar_row(&w, "Insights");
                if let (Some(o), Some(r)) = (&outline, row) {
                    let rect = o.rectOfRow(r as isize);
                    input::click_at(o, rect.origin.x + 40.0, rect.origin.y + rect.size.height / 2.0);
                }
                later_ms(400, move || {
                    let clicked = untrack(|| t.page.get());
                    input::key(&w, Key::Down, &[]);
                    later_ms(500, move || {
                        let after = untrack(|| t.page.get());
                        check("input-sidebar", clicked == Page::Insights && after == Page::Habit(1), format!("click {clicked:?}, then ↓ {after:?}"));
                        // The calendar: a real click on a day two weeks ago.
                        let cal = canvases(&w, "Walk 8,000 steps: done").into_iter().next();
                        let day = today - 14;
                        let first = super::first_shown(today);
                        let was = untrack(|| t.is_done(1, day));
                        if let Some(c) = &cal {
                            input::click_at(c, 30.0 + ((day - first) / 7) as f64 * 19.0 + 7.0, 18.0 + ((day - first) % 7) as f64 * 19.0 + 7.0);
                        }
                        later_ms(400, move || {
                            check("input-calendar", cal.is_some() && untrack(|| t.is_done(1, day)) != was, format!("day {day}: {was} → {}", untrack(|| t.is_done(1, day))));
                            // ⌘N, typing a name, a click on Add.
                            input::key(&w, Key::Char('n'), &[Mod::Command]);
                            later_ms(700, move || {
                                let sheet = w.attachedSheet();
                                if let Some(s) = &sheet {
                                    // Every symbol and colour is a button VoiceOver can name and press.
                                    let buttons = ferrokit::testing::ax_elements(s).into_iter().filter(|e| e.role == "AXButton").count();
                                    let problems = ax_audit(s);
                                    check("new-habit-sheet-ax", buttons == super::SYMBOLS.len() + super::PALETTE.len() + 2 && problems.is_empty(), format!("{buttons} buttons; {problems:?}"));
                                    input::type_text(s, "Yoga");
                                }
                                later_ms(300, move || {
                                    // A click on Add. (A posted Return doesn't press a
                                    // default button, even in plain AppKit: only window-server
                                    // input can check that.)
                                    if let Some(add) = sheet.as_ref().and_then(|s| views(s).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSButton>().ok()).find(|b| b.title().to_string() == "Add")) {
                                        input::click(&add);
                                    }
                                    later_ms(800, move || {
                                        let last = untrack(|| t.habits.get()).last().map(|h| h.name.clone());
                                        check("input-new-habit", sheet.is_some() && last.as_deref() == Some("Yoga") && w.attachedSheet().is_none(), format!("sheet {}; last habit {last:?}", sheet.is_some()));
                                        if let Some(id) = untrack(|| t.habits.get()).last().filter(|h| h.name == "Yoga").map(|h| h.id) {
                                            untrack(|| t.delete(id));
                                        }
                                        transition_phase(w, t, check, failed);
                                    });
                                });
                            });
                        });
                    });
                });
            });
        });
    }

    /// An animated page change back to Today shows every card's text.
    fn transition_phase(w: Retained<NSWindow>, t: Tracker, check: Check, failed: Rc<Cell<u32>>) {
        untrack(|| t.page.set(Page::Insights));
        later_ms(500, move || {
            untrack(|| animated(|| t.page.set(Page::Today)));
            // Cards clicked while the page is still fading in.
            let today = untrack(|| t.today.get());
            later_ms(100, move || {
                for id in [2, 3] {
                    untrack(|| animated(|| t.toggle(id, today)));
                }
            });
            later_ms(1200, move || {
                let names: Vec<String> = untrack(|| t.habits.get()).into_iter().map(|h| h.name).collect();
                // The page's labels, not the sidebar's (in its outline view).
                let in_sidebar = |f: &objc2_app_kit::NSTextField| {
                    let mut a = unsafe { f.superview() };
                    while let Some(v) = a {
                        if v.downcast_ref::<objc2_app_kit::NSOutlineView>().is_some() {
                            return true;
                        }
                        a = unsafe { v.superview() };
                    }
                    false
                };
                let fields: Vec<Retained<objc2_app_kit::NSTextField>> = views(&w).into_iter().filter_map(|v| v.downcast::<objc2_app_kit::NSTextField>().ok()).filter(|f| !in_sidebar(f)).collect();
                let mut bad = Vec::new();
                for n in &names {
                    // The card's title (not the sidebar's row).
                    let shown = fields.iter().filter(|f| f.stringValue().to_string() == *n).any(|f| {
                        let visible = f.alphaValue() > 0.99 && !f.isHiddenOrHasHiddenAncestor() && f.frame().size.width > 10.0 && f.window().is_some();
                        let mut ancestor = unsafe { f.superview() };
                        let mut faded = false;
                        while let Some(a) = ancestor {
                            faded |= a.alphaValue() < 0.99;
                            ancestor = unsafe { a.superview() };
                        }
                        visible && !faded
                    });
                    let detail: Vec<String> = fields.iter().filter(|f| f.stringValue().to_string() == *n).map(|f| format!("alpha {:.2} frame {:?} hidden {}", f.alphaValue(), f.frame(), f.isHiddenOrHasHiddenAncestor())).collect();
                    if !shown {
                        bad.push(format!("{n}: {detail:?}"));
                    }
                }
                check("animated-page-text", bad.is_empty(), format!("{bad:?}"));
                dark_phase(w, t, check, failed);
            });
        });
    }

    fn dark_phase(w: Retained<NSWindow>, t: Tracker, check: Check, failed: Rc<Cell<u32>>) {
        if std::env::var("SNAP").is_err() {
            return finish(failed);
        }
        set_appearance(Appearance::Dark);
        untrack(|| t.page.set(Page::Today));
        later_ms(700, move || {
            snap(&w, "today-dark");
            untrack(|| t.page.set(Page::Habit(1)));
            later_ms(600, move || {
                snap(&w, "habit-dark");
                untrack(|| t.page.set(Page::Insights));
                later_ms(600, move || {
                    snap(&w, "insights-dark");
                    set_appearance(Appearance::System);
                    let _ = check;
                    finish(failed);
                });
            });
        });
    }

    fn finish(failed: Rc<Cell<u32>>) {
        eprintln!("[check] done, {} failed", failed.get());
        std::process::exit(if failed.get() == 0 { 0 } else { 1 });
    }
}
