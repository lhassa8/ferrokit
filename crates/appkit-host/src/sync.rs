//! Keeping native frame changes and the host's GPU frame in step (risk 2).
//!
//! Background (measured on 60 fps video in the prototype): winit redraws from a
//! run-loop observer that runs *after* Core Animation's commit observer. Native
//! frames set during the host's draw therefore wait for the next run-loop pass
//! to commit, while the GPU frame presents on its own schedule.
//!
//! Two independent switches, both off by default:
//! - [`set_flush_after_draw`]: commits the pending Core Animation transaction
//!   right after the host's redraw. During the draw, `place` queues a block on
//!   the main run loop and wakes it; the block runs as soon as the host's
//!   redraw (and its GPU present) returns. (A plain run-loop observer at the
//!   host's own order ran *before* the host's redraw, ~16 ms after the
//!   previous draw, so it couldn't be used.)
//! - [`set_layer_presents_with_transaction`]: asks the Metal layer to present
//!   inside that transaction. Only honoured by wgpu 28 and later, which read
//!   the flag from the layer each frame; wgpu 27 (Iced 0.14) resets it.

use std::cell::Cell;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_app_kit::NSWindowOcclusionState;
use objc2_core_foundation::{CFRunLoop, kCFRunLoopCommonModes};

/// When the flush observer may start flushing. Flushing the first frames
/// while the window is still being put on screen lost the last startup frame
/// in the spike (the window kept a stale frame until the next redraw).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StartGate {
    /// Flush from the first frame.
    Immediately,
    /// Once the window reports itself visible (occlusion state).
    WindowVisible,
    /// A fixed delay after the first observed draw (the spike's workaround).
    After(Duration),
}

#[derive(Debug, Clone, Copy, Default)]
/// Counters for the flush switch (diagnostics).
pub struct SyncStats {
    /// Flush blocks that ran (at most one per drawn frame).
    pub passes_after_draw: u64,
    /// Of those, blocks that ran within 4 ms of the last `place` (i.e. right
    /// after the host's redraw, as intended).
    pub passes_right_after_draw: u64,
    /// Flushes done.
    pub flushes: u64,
    /// Passes held back by the start gate.
    pub gated: u64,
}

thread_local! {
    static SCHEDULED: Cell<bool> = const { Cell::new(false) };
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static GATE: Cell<StartGate> = const { Cell::new(StartGate::WindowVisible) };
    static OPEN: Cell<bool> = const { Cell::new(false) };
    static FIRST_PLACE: Cell<Option<Instant>> = const { Cell::new(None) };
    static LAST_PLACE: Cell<Option<Instant>> = const { Cell::new(None) };
    static DIRTY: Cell<bool> = const { Cell::new(false) };
    /// Time from the last `place` to each flush block, in microseconds.
    static GAPS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    static STATS: Cell<SyncStats> = const { Cell::new(SyncStats { passes_after_draw: 0, passes_right_after_draw: 0, flushes: 0, gated: 0 }) };
}

pub(crate) fn note_place() {
    let now = Instant::now();
    FIRST_PLACE.with(|f| if f.get().is_none() { f.set(Some(now)) });
    LAST_PLACE.with(|l| l.set(Some(now)));
    DIRTY.with(|d| d.set(true));
    if ENABLED.with(Cell::get) && !SCHEDULED.with(|s| s.replace(true)) {
        let main = CFRunLoop::main().expect("main run loop");
        let block = RcBlock::new(|| {
            SCHEDULED.with(|s| s.set(false));
            on_pass();
        });
        // SAFETY: the block only touches main-thread state and runs on the
        // main run loop; common modes include live-resize event tracking.
        unsafe { main.perform_block(kCFRunLoopCommonModes.map(|m| m.as_ref()), Some(&block)) };
        main.wake_up();
    }
}

/// Percentiles (p10, p50, p90) of the time from the last `place` to the
/// flush block, in microseconds. Diagnostic.
pub fn gap_percentiles() -> Option<(u32, u32, u32)> {
    GAPS.with_borrow(|g| {
        let mut v = g.clone();
        v.sort_unstable();
        let at = |q: usize| v[(v.len() - 1) * q / 100];
        (!v.is_empty()).then(|| (at(10), at(50), at(90)))
    })
}

/// Counters so far.
pub fn stats() -> SyncStats {
    STATS.with(Cell::get)
}

/// Turns flushing right after each host redraw on or off.
pub fn set_flush_after_draw(enabled: bool, gate: StartGate) {
    let _ = crate::host::mtm();
    ENABLED.with(|e| e.set(enabled));
    GATE.with(|g| g.set(gate));
}

fn on_pass() {
    if !DIRTY.with(|d| d.replace(false)) {
        return;
    }
    let mut stats = STATS.with(Cell::get);
    stats.passes_after_draw += 1;
    if let Some(t) = LAST_PLACE.with(Cell::get) {
        let gap = t.elapsed();
        GAPS.with_borrow_mut(|g| g.push(gap.as_micros().min(u32::MAX as u128) as u32));
        if gap < Duration::from_millis(4) {
            stats.passes_right_after_draw += 1;
        }
    }
    if ENABLED.with(Cell::get) {
        if gate_open() {
            flush();
            stats.flushes += 1;
        } else {
            stats.gated += 1;
        }
    }
    STATS.with(|s| s.set(stats));
}

fn gate_open() -> bool {
    if OPEN.with(Cell::get) {
        return true;
    }
    let open = match GATE.with(Cell::get) {
        StartGate::Immediately => true,
        StartGate::After(d) => FIRST_PLACE.with(Cell::get).is_some_and(|t| t.elapsed() >= d),
        StartGate::WindowVisible => crate::host::content_view()
            .and_then(|v| v.window())
            .is_some_and(|w| w.occlusionState().contains(NSWindowOcclusionState::Visible)),
    };
    OPEN.with(|o| o.set(open));
    open
}

/// Commits the current Core Animation transaction now.
pub fn flush() {
    // SAFETY: class method with no arguments.
    let () = unsafe { msg_send![class!(CATransaction), flush] };
}

/// Sets `presentsWithTransaction` on the content view's Metal layer. Returns
/// false if there is no such layer. See the module docs for which wgpu
/// versions honour it; on wgpu 27 it freezes the GPU output.
pub fn set_layer_presents_with_transaction(on: bool) -> bool {
    let Some(content) = crate::host::content_view() else { return false };
    let Some(layer) = content.layer() else { return false };
    let layer: &AnyObject = &layer;
    // SAFETY: checked with respondsToSelector before calling the setter.
    unsafe {
        let responds: bool = msg_send![layer, respondsToSelector: objc2::sel!(setPresentsWithTransaction:)];
        if !responds {
            return false;
        }
        let () = msg_send![layer, setPresentsWithTransaction: on];
    }
    true
}
