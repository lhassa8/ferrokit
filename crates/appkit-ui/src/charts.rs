//! Charts on the [canvas](crate::canvas): bars for categories, lines for
//! numeric series. Axes with round-number ticks and gridlines, values on
//! hover, a VoiceOver summary, and lines of millions of points drawn at one
//! or two points per pixel column.

use std::cell::RefCell;
use std::rc::Rc;

use appkit_host::text::TextColor;

use crate::canvas::{Canvas, Painter, canvas};
use crate::signal::{Effect, Signal, keep_with_owner, untrack};
use crate::view::Align;

/// Colours of the series, in order.
const PALETTE: [TextColor; 8] = [
    TextColor::Blue,
    TextColor::Orange,
    TextColor::Green,
    TextColor::Pink,
    TextColor::Purple,
    TextColor::Teal,
    TextColor::Yellow,
    TextColor::Brown,
];

const FONT: f64 = 11.0;

/// One line of a [`line_chart`]: a name and its points (sorted by x or
/// not; they're sorted for drawing).
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// The name in the legend and the hover readout.
    pub name: String,
    /// `(x, y)` points.
    pub points: Vec<(f64, f64)>,
}

impl Series {
    /// A series named `name` through `points`.
    pub fn new(name: &str, points: Vec<(f64, f64)>) -> Self {
        Series { name: name.into(), points }
    }
}

/// Round-number ticks covering `min..=max`, about `target` of them.
pub(crate) fn ticks(min: f64, max: f64, target: usize) -> (Vec<f64>, f64) {
    // An empty or flat range (or NaN) still gets an axis around it.
    let (min, max) = if max.partial_cmp(&min) != Some(std::cmp::Ordering::Greater) {
        if min == 0.0 { (0.0, 1.0) } else { (min - min.abs() * 0.1, max + max.abs() * 0.1) }
    } else {
        (min, max)
    };
    let raw = (max - min) / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = mag * if norm < 1.5 { 1.0 } else if norm < 3.0 { 2.0 } else if norm < 7.0 { 5.0 } else { 10.0 };
    let (start, end) = ((min / step).floor(), (max / step).ceil());
    let ticks = (start as i64..=end as i64).map(|i| i as f64 * step).collect();
    (ticks, step)
}

/// A tick label: compact for large numbers (12k, 1.5M), with as many
/// decimals as the step needs.
pub(crate) fn tick_label(v: f64, step: f64) -> String {
    let a = v.abs();
    if a >= 1e9 && step >= 1e8 {
        return format!("{}B", trim(v / 1e9, step / 1e9));
    }
    if a >= 1e6 && step >= 1e5 {
        return format!("{}M", trim(v / 1e6, step / 1e6));
    }
    if a >= 1e4 && step >= 1e3 {
        return format!("{}k", trim(v / 1e3, step / 1e3));
    }
    trim(v, step)
}

fn trim(v: f64, step: f64) -> String {
    let decimals = if step >= 1.0 { 0 } else { (-step.log10().floor()) as usize };
    let s = format!("{v:.decimals$}");
    if s == "-0" { "0".into() } else { s }
}

/// A value for the readout: up to 4 significant decimals, no trailing zeros.
fn value_label(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{v:.0}");
    }
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The plot area inside the axes' labels.
struct Plot {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Draws the y axis (ticks, labels, gridlines) and returns the plot area
/// and the value → y mapping.
fn y_axis(p: &mut Painter, lo: f64, hi: f64, top: f64, bottom: f64) -> (Plot, impl Fn(f64) -> f64 + use<>) {
    let (ticks, step) = ticks(lo, hi, ((p.height() - top - bottom) / 40.0).clamp(2.0, 8.0) as usize);
    let labels: Vec<String> = ticks.iter().map(|&t| tick_label(t, step)).collect();
    let label_w = labels.iter().map(|l| p.measure(l, FONT).0).fold(0.0, f64::max);
    let plot = Plot { x: label_w + 10.0, y: top, w: (p.width() - label_w - 22.0).max(1.0), h: (p.height() - top - bottom).max(1.0) };
    let (t0, t1) = (ticks[0], *ticks.last().unwrap());
    let (py, ph) = (plot.y, plot.h);
    let to_y = move |v: f64| py + ph - (v - t0) / (t1 - t0).max(f64::MIN_POSITIVE) * ph;
    for (t, label) in ticks.iter().zip(&labels) {
        let y = to_y(*t);
        p.line(plot.x, y, plot.x + plot.w, y, TextColor::Separator, 1.0);
        p.text(plot.x - 6.0, y - FONT * 0.65, label, FONT, TextColor::Secondary, Align::End);
    }
    (plot, to_y)
}

/// A tooltip box with `lines` near `x, y`, kept inside the canvas.
fn tooltip(p: &mut Painter, x: f64, y: f64, lines: &[String]) {
    let w = lines.iter().map(|l| p.measure(l, FONT).0).fold(0.0, f64::max) + 12.0;
    let h = lines.len() as f64 * (FONT + 4.0) + 8.0;
    let bx = if x + 12.0 + w > p.width() { x - 12.0 - w } else { x + 12.0 };
    let by = (y - h / 2.0).clamp(0.0, (p.height() - h).max(0.0));
    p.set_opacity(0.95);
    p.fill_rounded_rect(bx, by, w, h, 5.0, TextColor::Background);
    p.set_opacity(1.0);
    p.stroke_rect(bx, by, w, h, TextColor::Separator, 1.0);
    for (i, l) in lines.iter().enumerate() {
        p.text(bx + 6.0, by + 4.0 + i as f64 * (FONT + 4.0), l, FONT, TextColor::Primary, Align::Start);
    }
}

fn no_data(p: &mut Painter) {
    p.text(p.width() / 2.0, p.height() / 2.0 - FONT, &crate::t("No data"), FONT + 2.0, TextColor::Secondary, Align::Center);
}

/// Keeps `data()` in a signal, recomputed when what it reads changes (so
/// hovering redraws without recomputing it).
fn hold<T: 'static>(data: impl Fn() -> T + 'static) -> Signal<Option<Rc<T>>> {
    let held = Signal::new(None);
    // Runs now (so it's never `None` when drawn) and when `data` changes.
    let effect = Effect::new(move || {
        let v = Rc::new(data());
        untrack(|| held.set(Some(v)));
    });
    keep_with_owner(effect);
    held
}

/// A bar chart of labelled values, redrawn when the signals `data` reads
/// change. Hovering a bar shows its value.
///
/// ```no_run
/// use appkit_ui::*;
///
/// let sales = Signal::new(vec![("Mon".to_string(), 12.0), ("Tue".to_string(), 30.0), ("Wed".to_string(), 21.0)]);
/// let chart = bar_chart(move || sales.get()).min_size(300.0, 200.0);
/// # let _ = chart;
/// ```
pub fn bar_chart(data: impl Fn() -> Vec<(String, f64)> + 'static) -> Canvas {
    let data = hold(data);
    let hover = Signal::new(None::<(f64, f64)>);
    canvas(move |p| {
        let Some(bars) = data.get().filter(|b| !b.is_empty()) else { return no_data(p) };
        let lo = bars.iter().map(|b| b.1).fold(0.0, f64::min);
        let hi = bars.iter().map(|b| b.1).fold(0.0, f64::max);
        let (plot, to_y) = y_axis(p, lo, hi, 10.0, 22.0);
        let n = bars.len() as f64;
        let slot = plot.w / n;
        let bar_w = (slot * 0.75).max(1.0);
        let zero = to_y(0.0);
        let hovered = hover.get().filter(|&(x, y)| x >= plot.x && x < plot.x + plot.w && y >= 0.0).map(|(x, _)| ((x - plot.x) / slot) as usize);
        for (i, (_, v)) in bars.iter().enumerate() {
            let x = plot.x + i as f64 * slot + (slot - bar_w) / 2.0;
            let (top, bottom) = (to_y(*v).min(zero), to_y(*v).max(zero));
            let color = if hovered == Some(i) { TextColor::Indigo } else { TextColor::Blue };
            p.fill_rect(x, top, bar_w, (bottom - top).max(0.5), color);
        }
        // Labels under the bars: every k-th one so they don't overlap.
        let widest = bars.iter().map(|b| p.measure(&b.0, FONT).0).fold(0.0, f64::max) + 6.0;
        let every = ((widest / slot).ceil() as usize).max(1);
        for (i, (label, _)) in bars.iter().enumerate().step_by(every) {
            p.text(plot.x + (i as f64 + 0.5) * slot, plot.y + plot.h + 4.0, label, FONT, TextColor::Secondary, Align::Center);
        }
        p.line(plot.x, zero, plot.x + plot.w, zero, TextColor::Secondary, 1.0);
        if let Some((i, (x, y))) = hovered.zip(hover.get()).filter(|(i, _)| *i < bars.len()) {
            tooltip(p, x, y, &[format!("{}: {}", bars[i].0, value_label(bars[i].1))]);
        }
    })
    .on_hover(move |at| hover.set(at))
    .description(move || {
        let bars = data.get().unwrap_or_default();
        let max = bars.iter().max_by(|a, b| a.1.total_cmp(&b.1));
        let min = bars.iter().min_by(|a, b| a.1.total_cmp(&b.1));
        match (min, max) {
            (Some(min), Some(max)) => format!(
                "Bar chart, {} bars, from {} ({}) to {} ({})",
                bars.len(),
                value_label(min.1),
                min.0,
                value_label(max.1),
                max.0
            ),
            _ => "Bar chart, no data".into(),
        }
    })
}

/// Points sorted by x, and their x's (for finding the one under the pointer).
struct Prepared {
    name: String,
    points: Vec<(f64, f64)>,
}

/// One or two points per pixel column (the first, lowest and highest, and
/// last of each), so a million points draw like the line they make.
pub(crate) fn decimate(points: &[(f64, f64)], x0: f64, x1: f64, columns: usize) -> Vec<(f64, f64)> {
    if points.len() <= columns * 4 || columns == 0 || x1 <= x0 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(columns * 4);
    let width = (x1 - x0) / columns as f64;
    let mut i = 0;
    while i < points.len() {
        let col = ((points[i].0 - x0) / width).floor();
        let start = i;
        let (mut lo, mut hi) = (i, i);
        while i < points.len() && ((points[i].0 - x0) / width).floor() == col {
            if points[i].1 < points[lo].1 {
                lo = i;
            }
            if points[i].1 > points[hi].1 {
                hi = i;
            }
            i += 1;
        }
        let mut keep = [start, lo, hi, i - 1];
        keep.sort_unstable();
        for (k, &j) in keep.iter().enumerate() {
            if k == 0 || keep[k - 1] != j {
                out.push(points[j]);
            }
        }
    }
    out
}

/// A line chart of one or more series, redrawn when the signals `series`
/// reads change. Hovering shows each series' value at that x. Series of
/// any length draw quickly: at most a few points per pixel column are kept.
///
/// ```no_run
/// use appkit_ui::*;
///
/// let chart = line_chart(|| {
///     let wave: Vec<(f64, f64)> = (0..1_000).map(|i| (i as f64, (i as f64 / 50.0).sin())).collect();
///     vec![Series::new("sine", wave)]
/// });
/// # let _ = chart;
/// ```
pub fn line_chart(series: impl Fn() -> Vec<Series> + 'static) -> Canvas {
    let data = hold(move || {
        series()
            .into_iter()
            .map(|s| {
                let mut points: Vec<(f64, f64)> = s.points.into_iter().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
                if !points.is_sorted_by(|a, b| a.0 <= b.0) {
                    points.sort_by(|a, b| a.0.total_cmp(&b.0));
                }
                Prepared { name: s.name, points }
            })
            .collect::<Vec<_>>()
    });
    let hover = Signal::new(None::<(f64, f64)>);
    // The decimated lines, for the data and plot width they were made for.
    type Cache = Option<(usize, i64, Rc<Vec<Vec<(f64, f64)>>>)>;
    let cache: Rc<RefCell<Cache>> = Rc::default();
    canvas(move |p| {
        let Some(all) = data.get() else { return no_data(p) };
        let points = || all.iter().flat_map(|s| s.points.iter());
        if points().next().is_none() {
            return no_data(p);
        }
        let (x0, x1) = points().fold((f64::MAX, f64::MIN), |(a, b), &(x, _)| (a.min(x), b.max(x)));
        let (y0, y1) = points().fold((f64::MAX, f64::MIN), |(a, b), &(_, y)| (a.min(y), b.max(y)));
        let legend = if all.len() > 1 { 18.0 } else { 0.0 };
        let (plot, to_y) = y_axis(p, y0, y1, 10.0 + legend, 22.0);
        let span = (x1 - x0).max(f64::MIN_POSITIVE);
        let to_x = |x: f64| plot.x + (x - x0) / span * plot.w;
        // X ticks.
        let (xt, xstep) = ticks(x0, x1, (plot.w / 90.0).clamp(2.0, 10.0) as usize);
        for t in xt.iter().filter(|t| **t >= x0 - xstep * 1e-9 && **t <= x1 + xstep * 1e-9) {
            p.text(to_x(*t), plot.y + plot.h + 4.0, &tick_label(*t, xstep), FONT, TextColor::Secondary, Align::Center);
        }
        // Lines, decimated once per data and width.
        let key = (Rc::as_ptr(&all) as usize, plot.w.round() as i64);
        let lines = {
            let mut c = cache.borrow_mut();
            match c.as_ref() {
                Some((k, w, lines)) if (*k, *w) == key => lines.clone(),
                _ => {
                    let lines = Rc::new(all.iter().map(|s| decimate(&s.points, x0, x1 + span * 1e-9, plot.w.ceil() as usize)).collect::<Vec<_>>());
                    *c = Some((key.0, key.1, lines.clone()));
                    lines
                }
            }
        };
        for (i, line) in lines.iter().enumerate() {
            let mapped: Vec<(f64, f64)> = line.iter().map(|&(x, y)| (to_x(x), to_y(y))).collect();
            let color = PALETTE[i % PALETTE.len()];
            if mapped.len() == 1 {
                p.fill_circle(mapped[0].0, mapped[0].1, 2.5, color);
            }
            p.polyline(&mapped, color, 1.5);
        }
        // Legend.
        if all.len() > 1 {
            let mut x = plot.x;
            for (i, s) in all.iter().enumerate() {
                p.fill_rounded_rect(x, 4.0, 10.0, 10.0, 2.0, PALETTE[i % PALETTE.len()]);
                p.text(x + 14.0, 2.0, &s.name, FONT, TextColor::Secondary, Align::Start);
                x += 24.0 + p.measure(&s.name, FONT).0;
            }
        }
        // The values under the pointer.
        if let Some((hx, hy)) = hover.get().filter(|&(x, _)| x >= plot.x && x <= plot.x + plot.w) {
            let at = x0 + (hx - plot.x) / plot.w * span;
            p.line(hx, plot.y, hx, plot.y + plot.h, TextColor::Secondary, 1.0);
            let mut lines = Vec::new();
            let mut nearest_x = None;
            for (i, s) in all.iter().enumerate() {
                let j = s.points.partition_point(|q| q.0 < at);
                let near = [j.wrapping_sub(1), j].into_iter().filter_map(|k| s.points.get(k)).min_by(|a, b| (a.0 - at).abs().total_cmp(&(b.0 - at).abs()));
                if let Some(&(x, y)) = near {
                    p.fill_circle(to_x(x), to_y(y), 3.5, PALETTE[i % PALETTE.len()]);
                    nearest_x.get_or_insert(x);
                    lines.push(if all.len() > 1 { format!("{}: {}", s.name, value_label(y)) } else { value_label(y) });
                }
            }
            if let Some(x) = nearest_x {
                lines.insert(0, format!("x = {}", value_label(x)));
            }
            tooltip(p, hx, hy, &lines);
        }
    })
    .on_hover(move |at| hover.set(at))
    .description(move || {
        let all = data.get().unwrap_or_default();
        let n: usize = all.iter().map(|s| s.points.len()).sum();
        let pts = || all.iter().flat_map(|s| s.points.iter());
        if n == 0 {
            return "Line chart, no data".into();
        }
        let (x0, x1) = pts().fold((f64::MAX, f64::MIN), |(a, b), &(x, _)| (a.min(x), b.max(x)));
        let (y0, y1) = pts().fold((f64::MAX, f64::MIN), |(a, b), &(_, y)| (a.min(y), b.max(y)));
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        format!(
            "Line chart of {}, {n} points, x from {} to {}, values from {} to {}",
            names.join(", "),
            value_label(x0),
            value_label(x1),
            value_label(y0),
            value_label(y1)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_ticks() {
        assert_eq!(ticks(0.0, 97.0, 5), (vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0], 20.0));
        assert_eq!(ticks(0.0, 1785.24, 5).0.last(), Some(&2000.0));
        let (t, step) = ticks(-3.2, 7.9, 4);
        assert_eq!((t.first(), t.last(), step), (Some(&-4.0), Some(&8.0), 2.0));
        // A flat series still gets an axis.
        assert!(ticks(5.0, 5.0, 4).0.len() >= 2);
        assert_eq!(ticks(0.0, 0.0, 4).0, vec![0.0, 0.2, 0.4, 0.6000000000000001, 0.8, 1.0]);
    }

    #[test]
    fn labels() {
        assert_eq!(tick_label(250_000.0, 50_000.0), "250k");
        assert_eq!(tick_label(1_500_000.0, 500_000.0), "1.5M");
        assert_eq!(tick_label(0.25, 0.05), "0.25");
        assert_eq!(tick_label(-0.0, 1.0), "0");
        assert_eq!(value_label(96.25), "96.25");
        assert_eq!(value_label(42.0), "42");
    }

    #[test]
    fn decimation_keeps_extremes() {
        let points: Vec<(f64, f64)> = (0..1_000_000).map(|i| (i as f64, if i == 500_123 { 99.0 } else { (i % 7) as f64 })).collect();
        let d = decimate(&points, 0.0, 1_000_000.0, 500);
        assert!(d.len() <= 2_000, "{} points", d.len());
        assert!(d.contains(&(500_123.0, 99.0)), "the spike survives");
        assert_eq!(d.first(), points.first());
        assert_eq!(d.last(), points.last());
        assert!(d.is_sorted_by(|a, b| a.0 <= b.0));
    }
}
