//! Soak tests: random user actions for a long time, watching for crashes,
//! hangs and growth.
//!
//! An app gives [`run`] its actions (clicks, typing, page switches, sheets,
//! resizes, built with [`input`](super::input)), each with a weight. The
//! driver picks one every `interval` from a seeded generator (the seed is
//! printed, so a run can be repeated), and prints:
//! - `[soak] action <n> <name>` every 200 actions (and the last 20 before a
//!   problem, so a crash's lead-up is in the log);
//! - `[soak] sample <minutes> footprint <MB> nodes <n> signals <n> windows <n>`
//!   every 30 seconds;
//! - `[soak] hang <seconds>` when the main thread stops answering for more
//!   than 2 seconds (a watchdog thread checks it);
//! - `[soak] growth ...`: the footprint's, views' and signals' growth per
//!   1,000 actions over the second half of the actions (time paused
//!   doesn't count), then `[soak] done ...` with the totals.
//!
//! The run pauses while another app is in front (it doesn't take focus back
//! after the start), so the machine stays usable. `scripts/soak.sh` runs
//! an app's soak and summarises the log, crash reports included.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use objc2_app_kit::NSApplication;

/// A small, seeded random generator (xorshift), for repeatable runs.
pub struct Rng(u64);

impl Rng {
    /// A generator from `seed` (not 0).
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    /// A number in `0..n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        if n == 0 { 0 } else { (self.0 % n as u64) as usize }
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.below(10_000) < (p * 10_000.0) as usize
    }

    /// One of `items`.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// One kind of action: a name for the log, a weight, and what it does.
pub struct Action {
    name: &'static str,
    weight: u32,
    run: Rc<dyn Fn(&mut Rng)>,
}

/// An action called `name`, picked `weight` times as often as one of
/// weight 1.
pub fn action(name: &'static str, weight: u32, run: impl Fn(&mut Rng) + 'static) -> Action {
    Action { name, weight, run: Rc::new(run) }
}

/// How long and how fast.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// How long to run.
    pub duration: Duration,
    /// Time between actions.
    pub interval: Duration,
    /// The random seed.
    pub seed: u64,
}

impl Settings {
    /// From the environment: `SOAK` minutes (default 10), `SOAK_SEED`
    /// (default the time), `SOAK_MS` between actions (default 120);
    /// `SOAK_ONLY=<action>` runs just that one (see [`run`]).
    pub fn from_env() -> Settings {
        let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok());
        let seed = env("SOAK_SEED").map(|s| s as u64).unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
        });
        Settings {
            duration: Duration::from_secs_f64(env("SOAK").unwrap_or(10.0) * 60.0),
            interval: Duration::from_millis(env("SOAK_MS").unwrap_or(120.0) as u64),
            seed,
        }
    }
}

/// (actions done, footprint MB, live nodes, live signals).
type Sample = (u64, f64, usize, usize);

struct State {
    rng: Rng,
    actions: Vec<Action>,
    total: u32,
    done: u64,
    paused: u64,
    recent: Vec<&'static str>,
    start: Instant,
    settings: Settings,
    samples: Vec<Sample>,
    first: (usize, usize),
    last: (usize, usize),
    hangs: Rc<Cell<u32>>,
}

/// Runs the soak, then exits the process: 0 if no hang was seen, 1 if one
/// was. Call it from `on_launch`, once the windows are up.
pub fn run(settings: Settings, actions: Vec<Action>) {
    // `SOAK_ONLY=<name>`: just that action, to find which one grows memory.
    let actions = match std::env::var("SOAK_ONLY") {
        Ok(only) => actions.into_iter().filter(|a| a.name == only).collect(),
        Err(_) => actions,
    };
    let total = actions.iter().map(|a| a.weight).sum::<u32>().max(1);
    eprintln!(
        "[soak] start seed {} for {:.0} min, an action every {} ms, {} kinds",
        settings.seed,
        settings.duration.as_secs_f64() / 60.0,
        settings.interval.as_millis(),
        actions.len()
    );
    // The watchdog: the main thread stamps `beat`; a thread watches it.
    let beat = Arc::new(AtomicU64::new(now_ms()));
    let hangs = Rc::new(Cell::new(0));
    {
        let beat = beat.clone();
        std::thread::spawn(move || {
            let mut reported = 0u64;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let gap = now_ms().saturating_sub(beat.load(Ordering::Relaxed));
                if gap > 2000 && gap / 1000 > reported {
                    reported = gap / 1000;
                    eprintln!("[soak] hang {:.1} s (main thread not answering)", gap as f64 / 1000.0);
                } else if gap <= 2000 {
                    reported = 0;
                }
            }
        });
    }
    // In front once at the start; after that it waits while the user has
    // another app in front.
    let app = NSApplication::sharedApplication(crate::mtm());
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    if let Some(w) = crate::windows().into_iter().rev().find(|w| w.isVisible()) {
        w.makeKeyAndOrderFront(None);
    }
    let counts = (crate::live_nodes(), crate::live_signals());
    let state = Rc::new(RefCell::new(State {
        rng: Rng::new(settings.seed),
        actions,
        total,
        done: 0,
        paused: 0,
        recent: Vec::new(),
        start: Instant::now(),
        settings,
        samples: Vec::new(),
        first: counts,
        last: counts,
        hangs: hangs.clone(),
    }));
    heartbeat(beat, hangs);
    sample(state.clone());
    step(state);
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Stamps the watchdog's clock from the main thread, 4 times a second, and
/// counts the hangs it sees afterwards (a late stamp).
fn heartbeat(beat: Arc<AtomicU64>, hangs: Rc<Cell<u32>>) {
    let last = beat.swap(now_ms(), Ordering::Relaxed);
    if now_ms().saturating_sub(last) > 2000 {
        hangs.set(hangs.get() + 1);
    }
    crate::spawn(|| std::thread::sleep(Duration::from_millis(250)), move |()| heartbeat(beat, hangs));
}

/// The footprint (measured off the main thread) and the live counts, every
/// 30 seconds.
fn sample(state: Rc<RefCell<State>>) {
    crate::spawn(super::footprint_mb, move |mb| {
        let mut s = state.borrow_mut();
        let minutes = s.start.elapsed().as_secs_f64() / 60.0;
        s.last = (crate::live_nodes(), crate::live_signals());
        let (done, last) = (s.done, s.last);
        s.samples.push((done, mb, last.0, last.1));
        eprintln!("[soak] sample {minutes:.1} min footprint {mb:.0} MB nodes {} signals {} windows {} actions {}", s.last.0, s.last.1, crate::windows().len(), s.done);
        drop(s);
        crate::spawn(|| std::thread::sleep(Duration::from_secs(30)), move |()| sample(state));
    });
}

fn step(state: Rc<RefCell<State>>) {
    let (interval, over) = {
        let s = state.borrow();
        (s.settings.interval, s.start.elapsed() >= s.settings.duration)
    };
    if over {
        return finish(&state);
    }
    let active = NSApplication::sharedApplication(crate::mtm()).isActive();
    if active {
        // Pick, then run outside the borrow (an action may open a sheet,
        // run a nested loop, or quit).
        let (run, mut rng) = {
            let mut s = state.borrow_mut();
            let total = s.total as usize;
            let mut r = s.rng.below(total) as u32;
            let index = s.actions.iter().position(|a| {
                if r < a.weight {
                    return true;
                }
                r -= a.weight;
                false
            });
            let index = index.unwrap_or(0);
            let name = s.actions[index].name;
            s.done += 1;
            s.recent.push(name);
            if s.recent.len() > 20 {
                s.recent.remove(0);
            }
            if s.done.is_multiple_of(200) {
                eprintln!("[soak] action {} {name}", s.done);
            }
            let seed = s.rng.below(usize::MAX) as u64 + 1;
            (s.actions[index].run.clone(), Rng::new(seed))
        };
        run(&mut rng);
    } else {
        let mut s = state.borrow_mut();
        if s.paused.is_multiple_of(250) {
            eprintln!("[soak] paused: another app is in front (windows: {:?})", crate::windows().iter().filter(|w| w.isVisible()).map(|w| w.title().to_string()).collect::<Vec<_>>());
        }
        s.paused += 1;
    }
    crate::spawn(move || std::thread::sleep(interval), move |()| step(state));
}

fn finish(state: &Rc<RefCell<State>>) {
    let s = state.borrow();
    // Growth per 1,000 actions over the second half of the actions (the
    // first half includes warming up: caches, fonts, the first pages
    // built; and time paused doesn't count).
    let half = s.done / 2;
    let late: Vec<_> = s.samples.iter().filter(|x| x.0 >= half).collect();
    let per_k = |f: &dyn Fn(&Sample) -> f64| match (late.first(), late.last()) {
        (Some(a), Some(b)) if b.0 > a.0 => (f(b) - f(a)) / (b.0 - a.0) as f64 * 1000.0,
        _ => 0.0,
    };
    let (mb_k, nodes_k, signals_k) = (per_k(&|x| x.1), per_k(&|x| x.2 as f64), per_k(&|x| x.3 as f64));
    let peak = s.samples.iter().map(|x| x.1).fold(0.0, f64::max);
    eprintln!(
        "[soak] growth per 1000 actions over the second half: footprint {mb_k:.1} MB, nodes {nodes_k:.1}, signals {signals_k:.1}"
    );
    eprintln!(
        "[soak] done {} actions in {:.1} min ({} ticks paused); hangs {}; footprint peak {peak:.0} MB; nodes {} → {}, signals {} → {}",
        s.done,
        s.start.elapsed().as_secs_f64() / 60.0,
        s.paused,
        s.hangs.get(),
        s.first.0,
        s.last.0,
        s.first.1,
        s.last.1
    );
    eprintln!("[soak] last actions: {}", s.recent.join(", "));
    let code = if s.hangs.get() == 0 { 0 } else { 1 };
    drop(s);
    std::process::exit(code);
}
