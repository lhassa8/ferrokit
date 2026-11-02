//! Charts on the [canvas](crate::canvas): bars (plain, stacked or grouped)
//! for categories, lines, areas and points for numeric series, pies,
//! histograms, heatmaps and sparklines, with [rules](Rule) for thresholds.
//! Axes with round-number ticks and gridlines, values on hover, a
//! VoiceOver summary, and lines of millions of points drawn at one or two
//! points per pixel column.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ferrokit_core::text::Color;

use objc2::rc::Retained;
use objc2_foundation::{NSLocale, NSNumber, NSNumberFormatter, NSNumberFormatterStyle, NSString};

use crate::canvas::{Canvas, Painter, canvas};
use crate::signal::{Effect, Signal, keep_with_owner, untrack};
use crate::view::Align;

/// Colours of the series, in order.
const PALETTE: [Color; 8] = [
    Color::Blue,
    Color::Orange,
    Color::Green,
    Color::Pink,
    Color::Purple,
    Color::Teal,
    Color::Yellow,
    Color::Brown,
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
    /// Its own colour (else the chart's next one).
    pub color: Option<Color>,
}

impl Series {
    /// A series named `name` through `points`.
    pub fn new(name: &str, points: Vec<(f64, f64)>) -> Self {
        Series { name: name.into(), points, color: None }
    }

    /// The same series drawn in `color`.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

/// A straight line across a chart at a value: a threshold, a target, an
/// average. [`Rule::y`] runs across at a value (bar, line, area, scatter
/// and histogram charts, and sparklines); [`Rule::x`] runs down at an x
/// (line, area and scatter charts, and histograms). The chart's range
/// takes it in, so a target above every value still shows.
///
/// ```no_run
/// use ferrokit::*;
///
/// let chart = bar_chart(|| vec![("Mon".to_string(), 1200.0), ("Tue".to_string(), 800.0)])
///     .rule(Rule::y(1000.0).label("Target").color(Color::Green));
/// # let _ = chart;
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    value: f64,
    vertical: bool,
    label: Option<String>,
    color: Color,
}

impl Rule {
    /// A line across the chart at `value` on the value axis.
    pub fn y(value: f64) -> Rule {
        Rule { value, vertical: false, label: None, color: Color::Red }
    }

    /// A line down the chart at `x`.
    pub fn x(x: f64) -> Rule {
        Rule { vertical: true, ..Rule::y(x) }
    }

    /// A label at its end, also read by VoiceOver.
    pub fn label(mut self, label: &str) -> Rule {
        self.label = Some(label.into());
        self
    }

    /// Its colour (red unless set).
    pub fn color(mut self, color: Color) -> Rule {
        self.color = color;
        self
    }
}

/// `lo..hi` widened to take in the chart's rules across (or, with
/// `vertical`, down).
fn with_rules(o: &Options, vertical: bool, lo: f64, hi: f64) -> (f64, f64) {
    o.rules.iter().filter(|r| r.vertical == vertical && r.value.is_finite()).fold((lo, hi), |(a, b), r| (a.min(r.value), b.max(r.value)))
}

/// Draws the rules over the plot, labels at their right or top end; rules
/// down need an x scale, and are left out where it doesn't reach.
fn draw_rules(p: &mut Painter, o: &Options, plot: &Plot, to_y: &dyn Fn(f64) -> f64, to_x: Option<&dyn Fn(f64) -> f64>) {
    for r in &o.rules {
        if r.vertical {
            let Some(x) = to_x.map(|f| f(r.value)).filter(|x| *x >= plot.x - 0.5 && *x <= plot.x + plot.w + 0.5) else { continue };
            p.line(x, plot.y, x, plot.y + plot.h, r.color, 1.5);
            if let Some(l) = &r.label {
                let end = x + 4.0 + p.measure(l, FONT).0 > plot.x + plot.w;
                rule_label(p, if end { x - 5.0 } else { x + 5.0 }, plot.y + 1.0, l, r.color, if end { Align::End } else { Align::Start });
            }
        } else {
            let y = to_y(r.value);
            p.line(plot.x, y, plot.x + plot.w, y, r.color, 1.5);
            if let Some(l) = &r.label {
                rule_label(p, plot.x + plot.w - 4.0, y - FONT - 6.0, l, r.color, Align::End);
            }
        }
    }
}

/// A rule's label on a patch of the background, so it reads over the data.
fn rule_label(p: &mut Painter, x: f64, y: f64, text: &str, color: Color, align: Align) {
    let w = p.measure(text, FONT).0;
    let left = match align {
        Align::End => x - w,
        Align::Center => x - w / 2.0,
        Align::Start => x,
    };
    p.set_opacity(0.85);
    p.fill_rounded_rect(left - 3.0, y - 1.0, w + 6.0, FONT + 4.0, 3.0, Color::Background);
    p.set_opacity(1.0);
    p.text(x, y, text, FONT, color, align);
}

/// The rules for VoiceOver: "; Target at $1,000".
fn rules_text(o: &Options, x: &Format, y: &Format) -> String {
    o.rules
        .iter()
        .map(|r| {
            let v = if r.vertical { x.value(r.value) } else { y.value(r.value) };
            match &r.label {
                Some(l) => format!("; {l} at {v}"),
                None => format!("; a rule at {v}"),
            }
        })
        .collect()
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
fn y_axis(p: &mut Painter, lo: f64, hi: f64, top: f64, bottom: f64, format: &Format) -> (Plot, impl Fn(f64) -> f64 + use<>) {
    let (ticks, step) = ticks(lo, hi, ((p.height() - top - bottom) / 40.0).clamp(2.0, 8.0) as usize);
    let labels: Vec<String> = ticks.iter().map(|&t| format.tick(t, step)).collect();
    let label_w = labels.iter().map(|l| p.measure(l, FONT).0).fold(0.0, f64::max);
    let plot = Plot { x: label_w + 10.0, y: top, w: (p.width() - label_w - 22.0).max(1.0), h: (p.height() - top - bottom).max(1.0) };
    let (t0, t1) = (ticks[0], *ticks.last().unwrap());
    let (py, ph) = (plot.y, plot.h);
    let to_y = move |v: f64| py + ph - (v - t0) / (t1 - t0).max(f64::MIN_POSITIVE) * ph;
    for (t, label) in ticks.iter().zip(&labels) {
        let y = to_y(*t);
        p.line(plot.x, y, plot.x + plot.w, y, Color::Separator, 1.0);
        p.text(plot.x - 6.0, y - FONT * 0.65, label, FONT, Color::Secondary, Align::End);
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
    p.fill_rounded_rect(bx, by, w, h, 5.0, Color::Background);
    p.set_opacity(1.0);
    p.stroke_rect(bx, by, w, h, Color::Separator, 1.0);
    for (i, l) in lines.iter().enumerate() {
        p.text(bx + 6.0, by + 4.0 + i as f64 * (FONT + 4.0), l, FONT, Color::Primary, Align::Start);
    }
}

fn no_data(p: &mut Painter) {
    p.text(p.width() / 2.0, p.height() / 2.0 - FONT, &crate::t("No data"), FONT + 2.0, Color::Secondary, Align::Center);
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

/// Points sorted by x, and their x's (for finding the one under the pointer).
struct Prepared {
    name: String,
    points: Vec<(f64, f64)>,
    color: Option<Color>,
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

/// How a chart prints its values: on the axes, in the hover readout and
/// in what VoiceOver reads.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Format {
    /// Plain numbers; large ones compact on the axis (12k, 1.5M).
    Number,
    /// Numbers with this many decimals, grouped as the user's locale does.
    Decimals(usize),
    /// Money in the currency with this ISO code (`"USD"`, `"EUR"`), as the
    /// user's locale writes it.
    Currency(String),
    /// Fractions as percentages: 0.25 is 25%.
    Percent,
    /// Dates: values are seconds since 1970 (UTC), shown in the user's
    /// time zone, with ticks on whole days, weeks, months or years.
    Date,
    /// Dates with times of day, ticks on whole hours where they fit.
    DateTime,
}

impl Format {
    fn is_date(&self) -> bool {
        matches!(self, Format::Date | Format::DateTime)
    }

    fn formatter(&self, decimals: usize) -> Option<Retained<NSNumberFormatter>> {
        let f = NSNumberFormatter::new();
        f.setLocale(Some(&NSLocale::currentLocale()));
        match self {
            Format::Currency(code) => {
                f.setNumberStyle(NSNumberFormatterStyle::CurrencyStyle);
                f.setCurrencyCode(Some(&NSString::from_str(code)));
            }
            Format::Percent => f.setNumberStyle(NSNumberFormatterStyle::PercentStyle),
            Format::Decimals(_) => f.setNumberStyle(NSNumberFormatterStyle::DecimalStyle),
            _ => return None,
        }
        f.setMinimumFractionDigits(decimals);
        f.setMaximumFractionDigits(decimals);
        Some(f)
    }

    fn number(&self, v: f64, decimals: usize) -> Option<String> {
        let f = self.formatter(decimals)?;
        f.stringFromNumber(&NSNumber::new_f64(v)).map(|s| s.to_string())
    }

    /// An axis label for `v` among ticks `step` apart.
    fn tick(&self, v: f64, step: f64) -> String {
        let places = |step: f64| if step >= 1.0 { 0 } else { (-step.log10().floor()) as usize };
        let text = match self {
            Format::Decimals(d) => self.number(v, *d),
            Format::Currency(_) => self.number(v, places(step)),
            Format::Percent => self.number(v, places(step * 100.0)),
            Format::Date | Format::DateTime => Some(date_text(v, if *self == Format::Date { "yMMMd" } else { "MMMd jm" })),
            Format::Number => None,
        };
        text.unwrap_or_else(|| tick_label(v, step))
    }

    /// `v` in full, for the readout and VoiceOver.
    fn value(&self, v: f64) -> String {
        let text = match self {
            Format::Decimals(d) => self.number(v, *d),
            Format::Currency(_) => self.number(v, if v.fract() == 0.0 { 0 } else { 2 }),
            Format::Percent => self.number(v, if (v * 100.0).fract() == 0.0 { 0 } else { 1 }),
            Format::Date => Some(date_text(v, "yMMMd")),
            Format::DateTime => Some(date_text(v, "yMMMd jm")),
            Format::Number => None,
        };
        text.unwrap_or_else(|| value_label(v))
    }
}

fn date_text(seconds: f64, template: &str) -> String {
    crate::l10n::format_date(seconds, template)
}

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date (year, month) of a day since 1970.
fn civil_from_days(z: i64) -> (i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m)
}

/// Date ticks within `min..=max` (seconds since 1970), about `target` of
/// them, on whole local hours, days, weeks (Mondays), months or years; with
/// the date format template their labels use. `offset` is the local time
/// zone's offset from UTC at a moment (it changes with daylight saving).
pub(crate) fn date_ticks(min: f64, max: f64, target: usize, offset: &dyn Fn(f64) -> f64) -> (Vec<f64>, &'static str) {
    const H: f64 = 3600.0;
    const D: f64 = 86_400.0;
    let span = (max - min).max(1.0);
    let target = target.max(1) as f64;
    // Local midnight starting day `d` (days since 1970, local), in UTC.
    let midnight = |d: i64| {
        let guess = d as f64 * D - offset(d as f64 * D);
        d as f64 * D - offset(guess)
    };
    let first_day = ((min + offset(min)) / D).floor() as i64;
    let hours = [(60.0, "jm"), (300.0, "jm"), (900.0, "jm"), (1800.0, "jm"), (H, "jm"), (3.0 * H, "jm"), (6.0 * H, "MMMd jm"), (12.0 * H, "MMMd jm")];
    if let Some(&(step, template)) = hours.iter().find(|(step, _)| span / step <= target) {
        let shift = offset(min);
        let first = ((min + shift) / step).ceil() * step - shift;
        return ((0..).map(|i| first + i as f64 * step).take_while(|t| *t <= max).collect(), template);
    }
    if let Some(days) = [1i64, 2, 7].into_iter().find(|d| span / (*d as f64 * D) <= target) {
        // Every day or two, or Mondays (day 4, 1970-01-05, was one).
        let mut d = first_day;
        while (if days == 7 { d - 4 } else { d }).rem_euclid(days) != 0 {
            d += 1;
        }
        let ticks = (0..).map(|i| midnight(d + i * days)).skip_while(|t| *t < min).take_while(|t| *t <= max).collect();
        return (ticks, "MMMd");
    }
    // Months (1, 3 or 6 at a time) or years: on the first of the month.
    let months = [1, 3, 6, 12, 24, 60, 120].into_iter().find(|m| span / (*m as f64 * 30.44 * D) <= target).unwrap_or(120);
    let (y0, m0) = civil_from_days(first_day);
    let mut index = y0 * 12 + (m0 - 1);
    index += (months - index.rem_euclid(months)) % months;
    let mut ticks = Vec::new();
    loop {
        let (y, m) = (index.div_euclid(12), index.rem_euclid(12) + 1);
        let t = midnight(days_from_civil(y, m, 1));
        if t > max {
            break;
        }
        if t >= min {
            ticks.push(t);
        }
        index += months;
    }
    (ticks, if months >= 12 { "y" } else if span > 330.0 * D { "MMMy" } else { "MMM" })
}

/// The local time zone's offset from UTC at `seconds` since 1970.
fn local_offset(seconds: f64) -> f64 {
    crate::l10n::local_offset(seconds)
}

/// What a chart draws (see the chart functions).
enum Data {
    Bars(Box<dyn Fn() -> Vec<(String, f64)>>),
    Stacked(Box<dyn Fn() -> Vec<(String, Vec<f64>)>>),
    Lines(Box<dyn Fn() -> Vec<Series>>, Mark),
    Pie(Box<dyn Fn() -> Vec<(String, f64)>>),
    Histogram(Box<dyn Fn() -> Vec<f64>>),
    Heatmap(Box<dyn Fn() -> Vec<(String, String, f64)>>),
    Spark(Box<dyn Fn() -> Vec<f64>>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Line,
    Area,
    Points,
}

/// A chart, made by [`bar_chart`], [`stacked_bar_chart`], [`line_chart`],
/// [`area_chart`], [`scatter_chart`], [`pie_chart`], [`histogram`],
/// [`heatmap`] or [`sparkline`]: drawn on a canvas,
/// redrawn when the signals its data function reads change, with values
/// on hover, a legend for more than one series, and a summary for
/// VoiceOver.
pub struct Chart {
    data: Data,
    options: Options,
    min: Option<(f64, f64)>,
    export: Option<String>,
}

#[derive(Clone)]
struct Options {
    values: Format,
    x: Format,
    colors: Vec<Color>,
    names: Vec<String>,
    grouped: bool,
    donut: bool,
    legend: bool,
    rules: Vec<Rule>,
    bins: usize,
    selection: Option<Signal<Option<usize>>>,
    x_selection: Option<Signal<Option<f64>>>,
    x_range: Option<Signal<Option<(f64, f64)>>>,
    zoomable: bool,
}

impl Options {
    fn color(&self, i: usize) -> Color {
        if self.colors.is_empty() { PALETTE[i % PALETTE.len()] } else { self.colors[i % self.colors.len()] }
    }

    /// The selected mark, if the chart has a [selection](Chart::selection).
    fn selected(&self) -> Option<usize> {
        self.selection.and_then(|s| s.get())
    }

    /// Mark `i`'s opacity: dimmed while another is selected.
    fn emphasis(&self, i: usize) -> f64 {
        match self.selected() {
            Some(s) if s != i => 0.35,
            _ => 1.0,
        }
    }
}

/// What's under a click: the mark's index, from the layout last drawn.
type Hit = Rc<RefCell<Option<Rc<dyn Fn(f64, f64) -> Option<usize>>>>>;

/// Clicking selects the mark under the pointer, or nothing off the marks.
fn selectable(c: Canvas, o: &Options, hit: &Hit) -> Canvas {
    let Some(selection) = o.selection else { return c };
    let hit = hit.clone();
    c.on_click(move |x, y| {
        let i = hit.borrow().as_ref().and_then(|h| h(x, y));
        selection.set(i);
    })
}

/// "; selected Mar ($1,370)", for VoiceOver.
fn selected_text(o: &Options, label: impl Fn(usize) -> Option<String>) -> String {
    o.selected().and_then(label).map(|l| format!("; selected {l}")).unwrap_or_default()
}

/// `range` kept within `full` and at least a millionth of it wide; `None`
/// when it covers all of it.
pub(crate) fn clamp_range((a, b): (f64, f64), (f0, f1): (f64, f64)) -> Option<(f64, f64)> {
    let full = f1 - f0;
    if !(a.is_finite() && b.is_finite()) || full <= 0.0 {
        return None;
    }
    let span = (b - a).abs().max(full * 1e-6);
    if span >= full * 0.999 {
        return None;
    }
    let a = a.min(b).clamp(f0, f1 - span);
    Some((a, a + span))
}

/// The points of a sorted series within `a..=b`; for a line, with points
/// at `a` and `b` where its segments cross them.
fn visible(points: &[(f64, f64)], a: f64, b: f64, line: bool) -> Vec<(f64, f64)> {
    let lo = points.partition_point(|p| p.0 < a);
    let hi = points.partition_point(|p| p.0 <= b);
    let at = |p: (f64, f64), q: (f64, f64), x: f64| (x, p.1 + (q.1 - p.1) * (x - p.0) / (q.0 - p.0));
    let mut out = Vec::with_capacity(hi.saturating_sub(lo) + 2);
    if line && lo > 0 && lo < points.len() && points[lo].0 > a {
        out.push(at(points[lo - 1], points[lo], a));
    }
    out.extend_from_slice(&points[lo..hi.max(lo)]);
    if line && hi > 0 && hi < points.len() && points[hi - 1].0 < b {
        out.push(at(points[hi - 1], points[hi], b));
    }
    out
}

impl Chart {
    fn new(data: Data) -> Chart {
        Chart {
            data,
            options: Options { values: Format::Number, x: Format::Number, colors: Vec::new(), names: Vec::new(), grouped: false, donut: false, legend: true, rules: Vec::new(), bins: 10, selection: None, x_selection: None, x_range: None, zoomable: false },
            min: None,
            export: None,
        }
    }

    /// How values print (the value axis, the readout, VoiceOver):
    /// [`Format::Currency`], [`Format::Percent`]…
    pub fn value_format(mut self, format: Format) -> Self {
        self.options.values = format;
        self
    }

    /// How x values print on line, area and scatter charts: dates with
    /// [`Format::Date`], money, percentages…
    pub fn x_format(mut self, format: Format) -> Self {
        self.options.x = format;
        self
    }

    /// The colours of the series (or bars, or slices), in order, repeating.
    /// A [`Series`] with its own colour keeps it.
    pub fn colors(mut self, colors: &[Color]) -> Self {
        self.options.colors = colors.to_vec();
        self
    }

    /// The names of a stacked bar chart's series, in the order of each
    /// category's values (for the legend and the readout).
    pub fn series_names(mut self, names: &[&str]) -> Self {
        self.options.names = names.iter().map(|n| n.to_string()).collect();
        self
    }

    /// A stacked bar chart's series side by side instead of stacked.
    pub fn grouped(mut self) -> Self {
        self.options.grouped = true;
        self
    }

    /// A pie chart with a hole in the middle, showing the total.
    pub fn donut(mut self) -> Self {
        self.options.donut = true;
        self
    }

    /// A [`Rule`] across or down the chart: a threshold, a target, an
    /// average. Add as many as needed.
    pub fn rule(mut self, rule: Rule) -> Self {
        self.options.rules.push(rule);
        self
    }

    /// Clicking a bar, a stacked bar's category, a slice, a histogram's
    /// bin or a heatmap's cell (row × columns + column) sets `selected` to
    /// its index, and clicking off the marks to `None`; the others dim
    /// while one is selected. Setting the signal selects from code.
    pub fn selection(mut self, selected: &Signal<Option<usize>>) -> Self {
        self.options.selection = Some(*selected);
        self
    }

    /// Clicking a line, area or scatter chart sets `x` to the nearest
    /// point's x (on a scatter chart, the point's under the pointer), and
    /// clicking off it to `None`; a line marks it, with each series' value.
    pub fn x_selection(mut self, x: &Signal<Option<f64>>) -> Self {
        self.options.x_selection = Some(*x);
        self
    }

    /// The x range a line, area or scatter chart shows, bound two ways:
    /// `None` shows all of it; `Some((from, to))` zooms in (the value axis
    /// fits what's shown). With [`zoomable`](Chart::zoomable), pinching and
    /// scrolling change it.
    pub fn x_range(mut self, range: &Signal<Option<(f64, f64)>>) -> Self {
        self.options.x_range = Some(*range);
        self
    }

    /// Pinching (or ⌘-scrolling) zooms a line, area or scatter chart in and
    /// out around the pointer, and scrolling sideways moves along; zooming
    /// out all the way shows everything again. Vertical scrolling still
    /// scrolls what the chart is in.
    pub fn zoomable(mut self) -> Self {
        self.options.zoomable = true;
        self
    }

    /// About how many bins a [`histogram`] sorts its values into (10
    /// unless set); their edges fall on round numbers.
    pub fn bins(mut self, bins: usize) -> Self {
        self.options.bins = bins.max(1);
        self
    }

    /// Hides the legend (shown when there's more than one series, and a
    /// heatmap's scale).
    pub fn no_legend(mut self) -> Self {
        self.options.legend = false;
        self
    }

    /// The smallest size it's laid out at (default 200 × 120); it takes the
    /// room it's given beyond that.
    pub fn min_size(mut self, width: f64, height: f64) -> Self {
        self.min = Some((width, height));
        self
    }

    /// Copy Image and Save Image… on its right-click menu, and dragging it
    /// out gives `name`.png (see [`Canvas::exportable`]).
    pub fn exportable(mut self, name: &str) -> Self {
        self.export = Some(name.into());
        self
    }
}

impl crate::view::View for Chart {
    fn build(mut self, cx: &mut crate::view::Cx) -> crate::view::Node {
        if self.options.zoomable && self.options.x_range.is_none() {
            self.options.x_range = Some(Signal::new(None));
        }
        let o = Rc::new(self.options);
        let mut c = match self.data {
            Data::Bars(f) => bars(f, o),
            Data::Stacked(f) => stacked(f, o),
            Data::Lines(f, mark) => lines(f, mark, o),
            Data::Pie(f) => pie(f, o),
            Data::Histogram(f) => histogram_canvas(f, o),
            Data::Heatmap(f) => heatmap_canvas(f, o),
            Data::Spark(f) => spark(f, o),
        };
        if let Some((w, h)) = self.min {
            c = c.min_size(w, h);
        }
        if let Some(name) = &self.export {
            c = c.exportable(name);
        }
        c.build(cx)
    }
}

/// A row of legend entries (a colour swatch and a name) from `x, y`,
/// wrapping within `width`; returns its height.
fn legend(p: &mut Painter, items: &[(String, Color)], x: f64, y: f64, width: f64) -> f64 {
    let (mut cx, mut cy) = (x, y);
    for (name, color) in items {
        let w = 24.0 + p.measure(name, FONT).0;
        if cx > x && cx + w > x + width {
            cx = x;
            cy += FONT + 7.0;
        }
        p.fill_rounded_rect(cx, cy + 2.0, 10.0, 10.0, 2.0, *color);
        p.text(cx + 14.0, cy, name, FONT, Color::Secondary, Align::Start);
        cx += w;
    }
    cy - y + FONT + 7.0
}

/// A bar chart of labelled values. Hovering a bar shows its value.
///
/// ```no_run
/// use ferrokit::*;
///
/// let sales = Signal::new(vec![("Mon".to_string(), 1200.0), ("Tue".to_string(), 3000.0)]);
/// let chart = bar_chart(move || sales.get()).value_format(Format::Currency("USD".into())).min_size(300.0, 200.0);
/// # let _ = chart;
/// ```
pub fn bar_chart(data: impl Fn() -> Vec<(String, f64)> + 'static) -> Chart {
    Chart::new(Data::Bars(Box::new(data)))
}

/// Bars made of several series per category, stacked (or side by side
/// with [`grouped`](Chart::grouped)): each category's values are in the
/// order of [`series_names`](Chart::series_names).
///
/// ```no_run
/// use ferrokit::*;
///
/// let chart = stacked_bar_chart(|| vec![("Jan".to_string(), vec![1200.0, 420.0, 180.0]), ("Feb".to_string(), vec![1200.0, 380.0, 260.0])])
///     .series_names(&["Rent", "Food", "Fun"])
///     .value_format(Format::Currency("USD".into()));
/// # let _ = chart;
/// ```
pub fn stacked_bar_chart(data: impl Fn() -> Vec<(String, Vec<f64>)> + 'static) -> Chart {
    Chart::new(Data::Stacked(Box::new(data)))
}

/// A line chart of one or more series. Hovering shows each series' value
/// at that x. Series of any length draw quickly: at most a few points per
/// pixel column are kept.
///
/// ```no_run
/// use ferrokit::*;
///
/// let chart = line_chart(|| {
///     let wave: Vec<(f64, f64)> = (0..1_000).map(|i| (i as f64, (i as f64 / 50.0).sin())).collect();
///     vec![Series::new("sine", wave)]
/// });
/// # let _ = chart;
/// ```
pub fn line_chart(series: impl Fn() -> Vec<Series> + 'static) -> Chart {
    Chart::new(Data::Lines(Box::new(series), Mark::Line))
}

/// A line chart filled down to zero under each line: totals over time.
pub fn area_chart(series: impl Fn() -> Vec<Series> + 'static) -> Chart {
    Chart::new(Data::Lines(Box::new(series), Mark::Area))
}

/// Points without lines: how two values relate. Hovering shows the point
/// nearest the pointer.
pub fn scatter_chart(series: impl Fn() -> Vec<Series> + 'static) -> Chart {
    Chart::new(Data::Lines(Box::new(series), Mark::Points))
}

/// Shares of a whole as slices, with a legend of names, values and
/// percentages. [`donut`](Chart::donut) makes a ring with the total in its
/// middle. Values of zero or less are left out.
///
/// ```no_run
/// use ferrokit::*;
///
/// let chart = pie_chart(|| vec![("Rent".to_string(), 1200.0), ("Food".to_string(), 420.0)]).donut();
/// # let _ = chart;
/// ```
pub fn pie_chart(data: impl Fn() -> Vec<(String, f64)> + 'static) -> Chart {
    Chart::new(Data::Pie(Box::new(data)))
}

/// How many values fall in each range: bars side by side over round-number
/// bins (about 10, or [`bins`](Chart::bins)). The values print with
/// [`value_format`](Chart::value_format); hovering a bar shows its range
/// and count.
///
/// ```no_run
/// use ferrokit::*;
///
/// let minutes = vec![12.0, 18.0, 25.0, 31.0, 33.0, 47.0, 52.0];
/// let chart = histogram(move || minutes.clone()).bins(5).rule(Rule::x(30.0).label("Average"));
/// # let _ = chart;
/// ```
pub fn histogram(values: impl Fn() -> Vec<f64> + 'static) -> Chart {
    Chart::new(Data::Histogram(Box::new(values)))
}

/// A grid of cells shaded by their values: `(column, row, value)`, with
/// rows and columns in the order they first appear. A missing cell stays
/// empty; hovering shows a cell's value, and a scale from the lowest to
/// the highest is drawn above (unless [`no_legend`](Chart::no_legend)).
/// The shade is the first of [`colors`](Chart::colors), else blue.
///
/// ```no_run
/// use ferrokit::*;
///
/// let days = ["Mon", "Tue", "Wed"];
/// let chart = heatmap(move || {
///     let mut cells = Vec::new();
///     for hour in 9..17 {
///         for (d, day) in days.iter().enumerate() {
///             cells.push((format!("{hour}"), day.to_string(), ((hour * 7 + d * 3) % 10) as f64));
///         }
///     }
///     cells
/// });
/// # let _ = chart;
/// ```
pub fn heatmap(cells: impl Fn() -> Vec<(String, String, f64)> + 'static) -> Chart {
    Chart::new(Data::Heatmap(Box::new(cells)))
}

/// A small line with no axes, its last value marked: a trend beside a
/// number or in a row. It takes the width it's given at a fixed height (80
/// × 24 unless [`min_size`](Chart::min_size) says). A [`Rule::y`] draws a
/// faint target line.
///
/// ```no_run
/// use ferrokit::*;
///
/// let week = vec![3.0, 5.0, 4.0, 7.0, 6.0, 8.0, 9.0];
/// let trend = sparkline(move || week.clone()).colors(&[Color::Green]);
/// # let _ = trend;
/// ```
pub fn sparkline(values: impl Fn() -> Vec<f64> + 'static) -> Chart {
    Chart { min: Some((80.0, 24.0)), ..Chart::new(Data::Spark(Box::new(values))) }
}

fn bars(data: Box<dyn Fn() -> Vec<(String, f64)>>, o: Rc<Options>) -> Canvas {
    let o3 = o.clone();
    let data = hold(data);
    let hover = Signal::new(None::<(f64, f64)>);
    let o2 = o.clone();
    let hit: Hit = Rc::default();
    let h2 = hit.clone();
    let c = canvas(move |p| {
        let Some(bars) = data.get().filter(|b| !b.is_empty()) else { return no_data(p) };
        let lo = bars.iter().map(|b| b.1).fold(0.0, f64::min);
        let hi = bars.iter().map(|b| b.1).fold(0.0, f64::max);
        let (lo, hi) = with_rules(&o, false, lo, hi);
        let (plot, to_y) = y_axis(p, lo, hi, 10.0, 22.0, &o.values);
        let n = bars.len() as f64;
        let slot = plot.w / n;
        let bar_w = (slot * 0.75).max(1.0);
        let zero = to_y(0.0);
        let hovered = hover.get().filter(|&(x, y)| x >= plot.x && x < plot.x + plot.w && y >= 0.0).map(|(x, _)| ((x - plot.x) / slot) as usize);
        let (px, py, ph, count) = (plot.x, plot.y, plot.h, bars.len());
        *h2.borrow_mut() = Some(Rc::new(move |x, y| (x >= px && y >= py && y <= py + ph).then(|| ((x - px) / slot) as usize).filter(|i| *i < count)));
        let base = o.colors.first().copied();
        for (i, (_, v)) in bars.iter().enumerate() {
            let x = plot.x + i as f64 * slot + (slot - bar_w) / 2.0;
            let (top, bottom) = (to_y(*v).min(zero), to_y(*v).max(zero));
            let lit = hovered == Some(i);
            let color = match base {
                None if lit => Color::Indigo,
                None => Color::Blue,
                Some(c) => c,
            };
            p.set_opacity(o.emphasis(i) * if lit && base.is_some() { 0.75 } else { 1.0 });
            p.fill_rect(x, top, bar_w, (bottom - top).max(0.5), color);
            p.set_opacity(1.0);
        }
        category_labels(p, &plot, bars.iter().map(|b| b.0.as_str()));
        p.line(plot.x, zero, plot.x + plot.w, zero, Color::Secondary, 1.0);
        draw_rules(p, &o, &plot, &to_y, None);
        if let Some((i, (x, y))) = hovered.zip(hover.get()).filter(|(i, _)| *i < bars.len()) {
            tooltip(p, x, y, &[format!("{}: {}", bars[i].0, o.values.value(bars[i].1))]);
        }
    })
    .on_hover(move |at| hover.set(at))
    .description(move || {
        let bars = data.get().unwrap_or_default();
        let max = bars.iter().max_by(|a, b| a.1.total_cmp(&b.1));
        let min = bars.iter().min_by(|a, b| a.1.total_cmp(&b.1));
        match (min, max) {
            (Some(min), Some(max)) => {
                let rules = rules_text(&o2, &o2.x, &o2.values);
                let selected = selected_text(&o2, |i| bars.get(i).map(|b| format!("{} ({})", b.0, o2.values.value(b.1))));
                format!("Bar chart, {} bars, from {} ({}) to {} ({}){rules}{selected}", bars.len(), o2.values.value(min.1), min.0, o2.values.value(max.1), max.0)
            }
            _ => "Bar chart, no data".into(),
        }
    });
    selectable(c, &o3, &hit)
}

/// Category labels under the bars: every k-th one so they don't overlap.
fn category_labels<'a>(p: &mut Painter, plot: &Plot, labels: impl Iterator<Item = &'a str> + Clone) {
    let n = labels.clone().count().max(1);
    let slot = plot.w / n as f64;
    let widest = labels.clone().map(|l| p.measure(l, FONT).0).fold(0.0, f64::max) + 6.0;
    let every = ((widest / slot).ceil() as usize).max(1);
    for (i, label) in labels.enumerate().step_by(every) {
        p.text(plot.x + (i as f64 + 0.5) * slot, plot.y + plot.h + 4.0, label, FONT, Color::Secondary, Align::Center);
    }
}

fn stacked(data: Box<dyn Fn() -> Vec<(String, Vec<f64>)>>, o: Rc<Options>) -> Canvas {
    let o3 = o.clone();
    let data = hold(data);
    let hover = Signal::new(None::<(f64, f64)>);
    let o2 = o.clone();
    let hit: Hit = Rc::default();
    let h2 = hit.clone();
    let name = |o: &Options, j: usize| o.names.get(j).cloned().unwrap_or_else(|| format!("Series {}", j + 1));
    let c = canvas(move |p| {
        let Some(rows) = data.get().filter(|b| !b.is_empty()) else { return no_data(p) };
        let series = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
        let items: Vec<(String, Color)> = (0..series).map(|j| (name(&o, j), o.color(j))).collect();
        let top = 10.0 + if o.legend && series > 1 { legend(p, &items, 8.0, 2.0, p.width() - 16.0) } else { 0.0 };
        // The range: sums of the positive and negative parts when stacked.
        let (mut lo, mut hi) = (0.0f64, 0.0f64);
        for (_, vs) in rows.iter() {
            if o.grouped {
                vs.iter().for_each(|v| (lo, hi) = (lo.min(*v), hi.max(*v)));
            } else {
                hi = hi.max(vs.iter().filter(|v| **v > 0.0).sum());
                lo = lo.min(vs.iter().filter(|v| **v < 0.0).sum());
            }
        }
        let (lo, hi) = with_rules(&o, false, lo, hi);
        let (plot, to_y) = y_axis(p, lo, hi, top, 22.0, &o.values);
        let slot = plot.w / rows.len() as f64;
        let bar_w = (slot * 0.75).max(1.0);
        let zero = to_y(0.0);
        let hovered = hover.get().filter(|&(x, y)| x >= plot.x && x < plot.x + plot.w && y >= plot.y).map(|(x, _)| ((x - plot.x) / slot) as usize);
        let (px, py, ph, count) = (plot.x, plot.y, plot.h, rows.len());
        *h2.borrow_mut() = Some(Rc::new(move |x, y| (x >= px && y >= py && y <= py + ph).then(|| ((x - px) / slot) as usize).filter(|i| *i < count)));
        for (i, (_, vs)) in rows.iter().enumerate() {
            let x = plot.x + i as f64 * slot + (slot - bar_w) / 2.0;
            if hovered == Some(i) {
                p.set_opacity(0.08);
                p.fill_rect(plot.x + i as f64 * slot, plot.y, slot, plot.h, Color::Primary);
                p.set_opacity(1.0);
            }
            let (mut up, mut down) = (0.0, 0.0);
            for (j, v) in vs.iter().enumerate() {
                let (a, b, bx, bw) = if o.grouped {
                    let w = bar_w / vs.len().max(1) as f64;
                    (0.0, *v, x + j as f64 * w, (w - 1.0).max(1.0))
                } else if *v >= 0.0 {
                    up += v;
                    (up - v, up, x, bar_w)
                } else {
                    down += v;
                    (down - v, down, x, bar_w)
                };
                let (y1, y2) = (to_y(a), to_y(b));
                p.set_opacity(o.emphasis(i));
                p.fill_rect(bx, y1.min(y2), bw, (y1 - y2).abs().max(0.5), o.color(j));
                p.set_opacity(1.0);
            }
        }
        category_labels(p, &plot, rows.iter().map(|r| r.0.as_str()));
        p.line(plot.x, zero, plot.x + plot.w, zero, Color::Secondary, 1.0);
        draw_rules(p, &o, &plot, &to_y, None);
        if let Some((i, (x, y))) = hovered.zip(hover.get()).filter(|(i, _)| *i < rows.len()) {
            let (label, vs) = &rows[i];
            let mut lines = vec![label.clone()];
            lines.extend(vs.iter().enumerate().map(|(j, v)| format!("{}: {}", name(&o, j), o.values.value(*v))));
            if !o.grouped && vs.len() > 1 {
                lines.push(format!("{}: {}", crate::t("Total"), o.values.value(vs.iter().sum())));
            }
            tooltip(p, x, y, &lines);
        }
    })
    .on_hover(move |at| hover.set(at))
    .description(move || {
        let rows = data.get().unwrap_or_default();
        if rows.is_empty() {
            return "Bar chart, no data".into();
        }
        let series = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
        let names: Vec<String> = (0..series).map(|j| name(&o2, j)).collect();
        let totals: Vec<(String, f64)> = rows.iter().map(|(l, vs)| (l.clone(), vs.iter().sum())).collect();
        let max = totals.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        format!(
            "{} bar chart of {}, {} categories; the largest total is {} ({}){}{}",
            if o2.grouped { "Grouped" } else { "Stacked" },
            names.join(", "),
            rows.len(),
            o2.values.value(max.1),
            max.0,
            rules_text(&o2, &o2.x, &o2.values),
            selected_text(&o2, |i| totals.get(i).map(|t| format!("{} ({})", t.0, o2.values.value(t.1))))
        )
    });
    selectable(c, &o3, &hit)
}

/// A line chart's layout as last drawn, for clicks, scrolling and pinching.
#[derive(Clone, Copy, Default)]
struct Layout {
    plot: (f64, f64, f64, f64),
    /// The x range of all the data (and rules down).
    full: (f64, f64),
    /// The x range shown.
    shown: (f64, f64),
}

/// The point of a sorted series nearest `at` in x.
fn nearest(points: &[(f64, f64)], at: f64) -> Option<(f64, f64)> {
    let j = points.partition_point(|q| q.0 < at);
    [j.wrapping_sub(1), j].into_iter().filter_map(|k| points.get(k)).min_by(|a, b| (a.0 - at).abs().total_cmp(&(b.0 - at).abs())).copied()
}

fn lines(series: Box<dyn Fn() -> Vec<Series>>, mark: Mark, o: Rc<Options>) -> Canvas {
    let data = hold(move || {
        series()
            .into_iter()
            .map(|s| {
                let mut points: Vec<(f64, f64)> = s.points.into_iter().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
                if !points.is_sorted_by(|a, b| a.0 <= b.0) {
                    points.sort_by(|a, b| a.0.total_cmp(&b.0));
                }
                Prepared { name: s.name, points, color: s.color }
            })
            .collect::<Vec<_>>()
    });
    let hover = Signal::new(None::<(f64, f64)>);
    // The decimated lines, for the data, plot width and x range they were made for.
    type Cache = Option<((usize, i64, u64, u64), Rc<Vec<Vec<(f64, f64)>>>)>;
    let cache: Rc<RefCell<Cache>> = Rc::default();
    let layout: Rc<std::cell::Cell<Layout>> = Rc::default();
    let o2 = o.clone();
    let o3 = o.clone();
    let lay = layout.clone();
    let range = o.x_range;
    let c = canvas(move |p| {
        let Some(all) = data.get() else { return no_data(p) };
        let points = || all.iter().flat_map(|s| s.points.iter());
        if points().next().is_none() {
            return no_data(p);
        }
        let color = |i: usize| all[i].color.unwrap_or_else(|| o.color(i));
        let full = points().fold((f64::MAX, f64::MIN), |(a, b), &(x, _)| (a.min(x), b.max(x)));
        let full = with_rules(&o, true, full.0, full.1);
        // Zoomed in: the points within the range (and where lines cross
        // its ends); the value axis fits them.
        let zoom = range.and_then(|r| r.get()).and_then(|r| clamp_range(r, full));
        let (x0, x1) = zoom.unwrap_or(full);
        let shown: Option<Vec<Vec<(f64, f64)>>> = zoom.map(|(a, b)| all.iter().map(|s| visible(&s.points, a, b, mark != Mark::Points)).collect());
        let series_points = |i: usize| -> &[(f64, f64)] { shown.as_ref().map_or(&all[i].points, |v| &v[i]) };
        let in_view = || (0..all.len()).flat_map(|i| series_points(i).iter());
        let (y0, y1) = in_view().fold((f64::MAX, f64::MIN), |(a, b), &(_, y)| (a.min(y), b.max(y)));
        let (mut y0, mut y1) = if y0 > y1 { (0.0, 1.0) } else { (y0, y1) };
        if mark == Mark::Area {
            y0 = y0.min(0.0); // areas fill down to zero
        }
        (y0, y1) = with_rules(&o, false, y0, y1);
        let items: Vec<(String, Color)> = all.iter().enumerate().map(|(i, s)| (s.name.clone(), color(i))).collect();
        let top = 10.0 + if o.legend && all.len() > 1 { legend(p, &items, 8.0, 2.0, p.width() - 16.0) } else { 0.0 };
        let (plot, to_y) = y_axis(p, y0, y1, top, 22.0, &o.values);
        lay.set(Layout { plot: (plot.x, plot.y, plot.w, plot.h), full, shown: (x0, x1) });
        let span = (x1 - x0).max(f64::MIN_POSITIVE);
        let to_x = |x: f64| plot.x + (x - x0) / span * plot.w;
        // X ticks: dates on whole days, months…; numbers on round values.
        let target = (plot.w / 90.0).clamp(2.0, 10.0) as usize;
        let xticks: Vec<(f64, String)> = if o.x.is_date() {
            let (ts, template) = date_ticks(x0, x1, target, &local_offset);
            ts.into_iter().map(|t| (t, date_text(t, template))).collect()
        } else {
            let (xt, xstep) = ticks(x0, x1, target);
            xt.into_iter().filter(|t| *t >= x0 - xstep * 1e-9 && *t <= x1 + xstep * 1e-9).map(|t| (t, o.x.tick(t, xstep))).collect()
        };
        for (t, label) in &xticks {
            p.text(to_x(*t), plot.y + plot.h + 4.0, label, FONT, Color::Secondary, Align::Center);
        }
        // Lines, decimated once per data, width and range.
        let key = (Rc::as_ptr(&all) as usize, plot.w.round() as i64, x0.to_bits(), x1.to_bits());
        let lines = {
            let mut c = cache.borrow_mut();
            match c.as_ref() {
                Some((k, lines)) if *k == key => lines.clone(),
                _ => {
                    let lines = Rc::new((0..all.len()).map(|i| decimate(series_points(i), x0, x1 + span * 1e-9, plot.w.ceil() as usize)).collect::<Vec<_>>());
                    *c = Some((key, lines.clone()));
                    lines
                }
            }
        };
        let base = to_y(0.0_f64.clamp(y0, y1)).min(plot.y + plot.h);
        for (i, line) in lines.iter().enumerate() {
            let mapped: Vec<(f64, f64)> = line.iter().map(|&(x, y)| (to_x(x), to_y(y))).collect();
            let c = color(i);
            match mark {
                Mark::Points => mapped.iter().for_each(|&(x, y)| p.fill_circle(x, y, 3.0, c)),
                Mark::Line | Mark::Area => {
                    if mark == Mark::Area && mapped.len() > 1 {
                        let mut shape = mapped.clone();
                        shape.push((mapped.last().unwrap().0, base));
                        shape.push((mapped[0].0, base));
                        p.set_opacity(0.22);
                        p.fill_polygon(&shape, c);
                        p.set_opacity(1.0);
                    }
                    if mapped.len() == 1 {
                        p.fill_circle(mapped[0].0, mapped[0].1, 2.5, c);
                    }
                    p.polyline(&mapped, c, if mark == Mark::Area { 2.0 } else { 1.5 });
                }
            }
        }
        draw_rules(p, &o, &plot, &to_y, Some(&to_x));
        // The selected x: a line down, each series' value marked.
        if let Some(sx) = o.x_selection.and_then(|s| s.get()).filter(|x| *x >= x0 && *x <= x1) {
            let hx = to_x(sx);
            p.line(hx, plot.y, hx, plot.y + plot.h, Color::Accent, 1.5);
            for (i, s) in all.iter().enumerate() {
                if let Some((x, y)) = nearest(&s.points, sx).filter(|q| mark != Mark::Points || q.0 == sx) {
                    p.fill_circle(to_x(x), to_y(y), 4.5, Color::Background);
                    p.fill_circle(to_x(x), to_y(y), 3.5, color(i));
                }
            }
            let label = o.x.value(sx);
            let end = hx + 5.0 + p.measure(&label, FONT).0 > plot.x + plot.w;
            rule_label(p, if end { hx - 5.0 } else { hx + 5.0 }, plot.y + 1.0, &label, Color::Accent, if end { Align::End } else { Align::Start });
        }
        let Some((hx, hy)) = hover.get().filter(|&(x, _)| x >= plot.x && x <= plot.x + plot.w) else { return };
        if mark == Mark::Points {
            // The point nearest the pointer, if it's close.
            let near = (0..all.len())
                .flat_map(|i| series_points(i).iter().map(move |q| (i, *q)))
                .map(|(i, (x, y))| (i, x, y, (to_x(x) - hx).hypot(to_y(y) - hy)))
                .min_by(|a, b| a.3.total_cmp(&b.3))
                .filter(|n| n.3 < 14.0);
            if let Some((i, x, y, _)) = near {
                p.stroke_circle(to_x(x), to_y(y), 5.5, color(i), 2.0);
                let mut lines = vec![format!("{}: {}", crate::t("x"), o.x.value(x)), format!("{}: {}", crate::t("y"), o.values.value(y))];
                if all.len() > 1 {
                    lines.insert(0, all[i].name.clone());
                }
                tooltip(p, hx, hy, &lines);
            }
            return;
        }
        // The values under the pointer.
        let at = x0 + (hx - plot.x) / plot.w * span;
        p.line(hx, plot.y, hx, plot.y + plot.h, Color::Secondary, 1.0);
        let mut lines = Vec::new();
        let mut nearest_x = None;
        for (i, s) in all.iter().enumerate() {
            if let Some((x, y)) = nearest(&s.points, at) {
                p.fill_circle(to_x(x), to_y(y), 3.5, color(i));
                nearest_x.get_or_insert(x);
                lines.push(if all.len() > 1 { format!("{}: {}", s.name, o.values.value(y)) } else { o.values.value(y) });
            }
        }
        if let Some(x) = nearest_x {
            lines.insert(0, if o.x == Format::Number { format!("x = {}", value_label(x)) } else { o.x.value(x) });
        }
        tooltip(p, hx, hy, &lines);
    })
    .on_hover(move |at| hover.set(at))
    .description(move || {
        let o = &o2;
        let all = data.get().unwrap_or_default();
        let n: usize = all.iter().map(|s| s.points.len()).sum();
        let pts = || all.iter().flat_map(|s| s.points.iter());
        let kind = match mark {
            Mark::Line => "Line chart",
            Mark::Area => "Area chart",
            Mark::Points => "Scatter chart",
        };
        if n == 0 {
            return format!("{kind}, no data");
        }
        let (x0, x1) = pts().fold((f64::MAX, f64::MIN), |(a, b), &(x, _)| (a.min(x), b.max(x)));
        let (y0, y1) = pts().fold((f64::MAX, f64::MIN), |(a, b), &(_, y)| (a.min(y), b.max(y)));
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        let zoom = range.and_then(|r| r.get()).and_then(|r| clamp_range(r, with_rules(o, true, x0, x1)));
        let zoom = zoom.map(|(a, b)| format!("; showing x from {} to {}", o.x.value(a), o.x.value(b))).unwrap_or_default();
        let selected = o.x_selection.and_then(|s| s.get()).map(|x| format!("; selected x {}", o.x.value(x))).unwrap_or_default();
        format!(
            "{kind} of {}, {n} points, x from {} to {}, values from {} to {}{}{zoom}{selected}",
            names.join(", "),
            o.x.value(x0),
            o.x.value(x1),
            o.values.value(y0),
            o.values.value(y1),
            rules_text(o, &o.x, &o.values)
        )
    });
    // Clicking selects the nearest x (a scatter chart's point under the pointer).
    let c = match o3.x_selection {
        Some(selection) => {
            let lay = layout.clone();
            c.on_click(move |x, y| {
                let l = lay.get();
                let (px, py, pw, ph) = l.plot;
                let inside = x >= px && x <= px + pw && y >= py && y <= py + ph;
                let all = data.get_untracked().unwrap_or_default();
                let at = l.shown.0 + (x - px) / pw.max(1.0) * (l.shown.1 - l.shown.0);
                let picked = if !inside {
                    None
                } else if mark == Mark::Points {
                    // Within 14 points, as the hover readout finds it.
                    let per_x = pw / (l.shown.1 - l.shown.0).max(f64::MIN_POSITIVE);
                    all.iter().flat_map(|s| s.points.iter()).filter(|q| ((q.0 - at) * per_x).abs() < 14.0).min_by(|a, b| (a.0 - at).abs().total_cmp(&(b.0 - at).abs())).map(|q| q.0)
                } else {
                    all.iter().filter_map(|s| nearest(&s.points, at)).min_by(|a, b| (a.0 - at).abs().total_cmp(&(b.0 - at).abs())).map(|q| q.0)
                };
                selection.set(picked);
            })
        }
        None => c,
    };
    let Some(range) = range.filter(|_| o3.zoomable) else { return c };
    let (l1, l2) = (layout.clone(), layout);
    // The range as it is now (the drawing may not have caught up).
    let shown = move |l: Layout| range.get_untracked().and_then(|r| clamp_range(r, l.full)).unwrap_or(l.full);
    c.on_magnify(move |amount, x, _| {
        let l = l1.get();
        let (a, b) = shown(l);
        let factor = (1.0 + amount).max(0.1);
        let anchor = a + ((x - l.plot.0) / l.plot.2.max(1.0)).clamp(0.0, 1.0) * (b - a);
        range.set(clamp_range((anchor - (anchor - a) / factor, anchor + (b - anchor) / factor), l.full));
        true
    })
    .on_scroll(move |dx, dy, _, _| {
        // Sideways scrolling moves along a zoomed chart; the rest scrolls on.
        let l = l2.get();
        let (a, b) = shown(l);
        if dx.abs() <= dy.abs() || (a, b) == l.full {
            return false;
        }
        let shift = -dx / l.plot.2.max(1.0) * (b - a);
        range.set(clamp_range((a + shift, b + shift), l.full));
        true
    })
}

/// The points of an arc around `(cx, cy)` from angle `a0` to `a1`
/// (radians, clockwise from three o'clock on screen).
fn arc(cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> Vec<(f64, f64)> {
    let steps = (((a1 - a0).abs() / std::f64::consts::TAU) * 96.0).ceil().max(2.0) as usize;
    (0..=steps).map(|k| a0 + (a1 - a0) * k as f64 / steps as f64).map(|a| (cx + r * a.cos(), cy + r * a.sin())).collect()
}

fn pie(data: Box<dyn Fn() -> Vec<(String, f64)>>, o: Rc<Options>) -> Canvas {
    let o3 = o.clone();
    let data = hold(move || data().into_iter().filter(|(_, v)| *v > 0.0 && v.is_finite()).collect::<Vec<_>>());
    let hover = Signal::new(None::<(f64, f64)>);
    let o2 = o.clone();
    let hit: Hit = Rc::default();
    let h2 = hit.clone();
    let c = canvas(move |p| {
        let Some(slices) = data.get().filter(|s| !s.is_empty()) else { return no_data(p) };
        let total: f64 = slices.iter().map(|s| s.1).sum();
        let percent = |v: f64| Format::Percent.value((v / total * 1000.0).round() / 1000.0);
        // The legend on the right: swatch, name, value and share.
        let entries: Vec<String> = slices.iter().map(|(n, v)| format!("{n}  {} ({})", o.values.value(*v), percent(*v))).collect();
        let legend_w = if o.legend { entries.iter().map(|e| p.measure(e, FONT).0).fold(0.0, f64::max) + 30.0 } else { 0.0 };
        let r = ((p.width() - legend_w - 16.0).min(p.height() - 16.0) / 2.0).max(4.0);
        let (cx, cy) = (8.0 + r, p.height() / 2.0);
        let inner = if o.donut { r * 0.58 } else { 0.0 };
        let start = -std::f64::consts::FRAC_PI_2;
        // The slice under a point.
        let ends: Vec<f64> = slices
            .iter()
            .scan(0.0, |acc, s| {
                *acc += s.1 / total * std::f64::consts::TAU;
                Some(*acc)
            })
            .collect();
        let under = move |x: f64, y: f64| {
            let d = (x - cx).hypot(y - cy);
            if d > r || d < inner {
                return None;
            }
            let a = ((y - cy).atan2(x - cx) - start).rem_euclid(std::f64::consts::TAU);
            ends.iter().position(|e| a < *e)
        };
        let hovered = hover.get().and_then(|(x, y)| under(x, y));
        *h2.borrow_mut() = Some(Rc::new(under));
        let mut a = start;
        for (i, (_, v)) in slices.iter().enumerate() {
            let sweep = v / total * std::f64::consts::TAU;
            // The hovered (or selected) slice moves out a little.
            let (dx, dy) = if hovered == Some(i) || o.selected() == Some(i) { let m = a + sweep / 2.0; (4.0 * m.cos(), 4.0 * m.sin()) } else { (0.0, 0.0) };
            let mut shape = arc(cx + dx, cy + dy, r, a, a + sweep);
            if inner > 0.0 {
                shape.extend(arc(cx + dx, cy + dy, inner, a + sweep, a));
            } else {
                shape.push((cx + dx, cy + dy));
            }
            p.set_opacity(o.emphasis(i));
            p.fill_polygon(&shape, o.color(i));
            p.set_opacity(1.0);
            // A thin gap between slices.
            if slices.len() > 1 {
                p.line(cx + inner * a.cos(), cy + inner * a.sin(), cx + r * a.cos(), cy + r * a.sin(), Color::Background, 1.5);
            }
            a += sweep;
        }
        if inner > 0.0 {
            let total_text = o.values.value(total);
            p.bold_text(cx, cy - FONT * 1.2, &total_text, FONT + 4.0, Color::Primary, Align::Center);
            p.text(cx, cy + 4.0, &crate::t("Total"), FONT, Color::Secondary, Align::Center);
        }
        if o.legend {
            let x = cx + r + 22.0;
            let mut y = (cy - slices.len() as f64 * (FONT + 7.0) / 2.0).max(4.0);
            for (i, e) in entries.iter().enumerate() {
                p.fill_rounded_rect(x, y + 2.0, 10.0, 10.0, 2.0, o.color(i));
                p.text(x + 16.0, y, e, FONT, if hovered == Some(i) { Color::Primary } else { Color::Secondary }, Align::Start);
                y += FONT + 7.0;
            }
        }
        if let Some((i, (x, y))) = hovered.zip(hover.get()) {
            tooltip(p, x, y, &[format!("{}: {} ({})", slices[i].0, o.values.value(slices[i].1), percent(slices[i].1))]);
        }
    })
    .on_hover(move |at| hover.set(at))
    .description(move || {
        let slices = data.get().unwrap_or_default();
        if slices.is_empty() {
            return "Pie chart, no data".into();
        }
        let total: f64 = slices.iter().map(|s| s.1).sum();
        let parts: Vec<String> = slices.iter().map(|(n, v)| format!("{n} {}", Format::Percent.value((v / total * 1000.0).round() / 1000.0))).collect();
        let selected = selected_text(&o2, |i| slices.get(i).map(|s| format!("{} ({})", s.0, o2.values.value(s.1))));
        format!("{} chart, {} slices, total {}: {}{selected}", if o2.donut { "Donut" } else { "Pie" }, slices.len(), o2.values.value(total), parts.join(", "))
    });
    selectable(c, &o3, &hit)
}

/// `values` sorted into bins on round-number edges, about `target` of
/// them: the edges (one more than the bins) and the count in each. A value
/// on the last edge counts in the last bin.
pub(crate) fn bin(values: &[f64], target: usize) -> (Vec<f64>, Vec<usize>) {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let (lo, hi) = finite.iter().fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let (mut edges, step) = ticks(lo, hi, target);
    if edges.len() < 2 {
        edges.push(edges[0] + step);
    }
    let n = edges.len() - 1;
    let mut counts = vec![0; n];
    for v in finite {
        counts[(((v - edges[0]) / step).floor().max(0.0) as usize).min(n - 1)] += 1;
    }
    (edges, counts)
}

fn histogram_canvas(values: Box<dyn Fn() -> Vec<f64>>, o: Rc<Options>) -> Canvas {
    let o3 = o.clone();
    let bins = o.bins;
    let data = hold(move || bin(&values(), bins));
    let hover = Signal::new(None::<(f64, f64)>);
    let o2 = o.clone();
    let hit: Hit = Rc::default();
    let h2 = hit.clone();
    let c = canvas(move |p| {
        let Some(h) = data.get().filter(|h| !h.1.is_empty()) else { return no_data(p) };
        let (edges, counts) = (&h.0, &h.1);
        let most = counts.iter().copied().max().unwrap_or(0) as f64;
        let (lo, hi) = with_rules(&o, false, 0.0, most);
        let (plot, to_y) = y_axis(p, lo, hi, 10.0, 22.0, &Format::Number);
        let (x0, x1) = (edges[0], edges[counts.len()]);
        let to_x = move |x: f64| plot.x + (x - x0) / (x1 - x0) * plot.w;
        let n = counts.len();
        let hovered = hover.get().filter(|&(x, y)| x >= plot.x && x < plot.x + plot.w && y >= plot.y).map(|(x, _)| (((x - plot.x) / plot.w * n as f64) as usize).min(n - 1));
        let (px, pw, py, ph) = (plot.x, plot.w, plot.y, plot.h);
        *h2.borrow_mut() = Some(Rc::new(move |x, y| (x >= px && x < px + pw && y >= py && y <= py + ph).then(|| (((x - px) / pw * n as f64) as usize).min(n - 1))));
        let color = o.colors.first().copied().unwrap_or(Color::Blue);
        let zero = to_y(0.0);
        for (i, &c) in counts.iter().enumerate().filter(|(_, c)| **c > 0) {
            let (a, b) = (to_x(edges[i]), to_x(edges[i + 1]));
            let top = to_y(c as f64);
            p.set_opacity(o.emphasis(i) * if hovered == Some(i) { 0.75 } else { 1.0 });
            p.fill_rect(a + 0.5, top, (b - a - 1.0).max(0.5), (zero - top).max(0.5), color);
            p.set_opacity(1.0);
        }
        // The edges, every k-th so they don't overlap.
        let step = edges[1] - edges[0];
        let labels: Vec<String> = edges.iter().map(|e| o.values.tick(*e, step)).collect();
        let widest = labels.iter().map(|l| p.measure(l, FONT).0).fold(0.0, f64::max) + 6.0;
        let every = ((widest / (plot.w / n as f64)).ceil() as usize).max(1);
        for (i, label) in labels.iter().enumerate().step_by(every) {
            p.text(to_x(edges[i]), plot.y + plot.h + 4.0, label, FONT, Color::Secondary, Align::Center);
        }
        p.line(plot.x, zero, plot.x + plot.w, zero, Color::Secondary, 1.0);
        draw_rules(p, &o, &plot, &to_y, Some(&to_x));
        if let Some((i, (x, y))) = hovered.zip(hover.get()) {
            tooltip(p, x, y, &[format!("{} – {}: {}", o.values.value(edges[i]), o.values.value(edges[i + 1]), counts[i])]);
        }
    })
    .on_hover(move |at| hover.set(at))
    .description(move || {
        let h = data.get().unwrap_or_default();
        let (edges, counts) = (&h.0, &h.1);
        let Some((i, most)) = counts.iter().enumerate().max_by_key(|(i, c)| (**c, std::cmp::Reverse(*i))) else { return "Histogram, no data".into() };
        let v = |x: f64| o2.values.value(x);
        format!(
            "Histogram of {} values in {} bins from {} to {}; the most, {most}, from {} to {}{}{}",
            counts.iter().sum::<usize>(),
            counts.len(),
            v(edges[0]),
            v(edges[counts.len()]),
            v(edges[i]),
            v(edges[i + 1]),
            rules_text(&o2, &o2.values, &Format::Number),
            selected_text(&o2, |i| counts.get(i).map(|c| format!("{} to {} ({c})", v(edges[i]), v(edges[i + 1]))))
        )
    });
    selectable(c, &o3, &hit)
}

/// A heatmap's cells by row and column, in the order they first appeared.
#[derive(Default)]
struct Cells {
    columns: Vec<String>,
    rows: Vec<String>,
    /// Row by row.
    values: Vec<Option<f64>>,
}

impl Cells {
    fn new(cells: Vec<(String, String, f64)>) -> Cells {
        let (mut columns, mut rows) = (Vec::new(), Vec::new());
        let (mut col_at, mut row_at) = (HashMap::new(), HashMap::new());
        let mut found = Vec::new();
        for (c, r, v) in cells.into_iter().filter(|c| c.2.is_finite()) {
            let ci = *col_at.entry(c.clone()).or_insert_with(|| {
                columns.push(c);
                columns.len() - 1
            });
            let ri = *row_at.entry(r.clone()).or_insert_with(|| {
                rows.push(r);
                rows.len() - 1
            });
            found.push((ri, ci, v));
        }
        let mut values = vec![None; rows.len() * columns.len()];
        for (r, c, v) in found {
            values[r * columns.len() + c] = Some(v);
        }
        Cells { columns, rows, values }
    }

    fn range(&self) -> Option<(f64, f64)> {
        self.values.iter().flatten().fold(None, |acc, &v| Some(acc.map_or((v, v), |(a, b): (f64, f64)| (a.min(v), b.max(v)))))
    }

    /// The cell with the lowest (or, with `highest`, the highest) value:
    /// its row, column and value.
    fn extreme(&self, highest: bool) -> Option<(&str, &str, f64)> {
        let n = self.columns.len();
        let pick = self.values.iter().enumerate().filter_map(|(i, v)| v.map(|v| (i, v)));
        let (i, v) = if highest { pick.max_by(|a, b| a.1.total_cmp(&b.1)) } else { pick.min_by(|a, b| a.1.total_cmp(&b.1)) }?;
        Some((&self.rows[i / n], &self.columns[i % n], v))
    }
}

fn heatmap_canvas(cells: Box<dyn Fn() -> Vec<(String, String, f64)>>, o: Rc<Options>) -> Canvas {
    let o3 = o.clone();
    let data = hold(move || Cells::new(cells()));
    let hover = Signal::new(None::<(f64, f64)>);
    let o2 = o.clone();
    let hit: Hit = Rc::default();
    let h2 = hit.clone();
    let c = canvas(move |p| {
        let Some(g) = data.get().filter(|g| !g.values.is_empty()) else { return no_data(p) };
        let Some((lo, hi)) = g.range() else { return no_data(p) };
        let color = o.colors.first().copied().unwrap_or(Color::Blue);
        let shade = |v: f64| if hi > lo { 0.12 + 0.88 * (v - lo) / (hi - lo) } else { 1.0 };
        // The scale above: lowest, five steps, highest.
        let mut top = 6.0;
        if o.legend {
            let (low, high) = (o.values.value(lo), o.values.value(hi));
            let mut x = p.width() - 8.0 - p.measure(&high, FONT).0;
            p.text(x, 2.0, &high, FONT, Color::Secondary, Align::Start);
            for k in (0..5).rev() {
                x -= 13.0;
                p.set_opacity(0.12 + 0.88 * k as f64 / 4.0);
                p.fill_rounded_rect(x, 4.0, 10.0, 10.0, 2.0, color);
                p.set_opacity(1.0);
            }
            p.text(x - 4.0, 2.0, &low, FONT, Color::Secondary, Align::End);
            top += FONT + 9.0;
        }
        let label_w = g.rows.iter().map(|r| p.measure(r, FONT).0).fold(0.0, f64::max);
        let plot = Plot { x: label_w + 10.0, y: top, w: (p.width() - label_w - 18.0).max(1.0), h: (p.height() - top - 22.0).max(1.0) };
        let (nc, nr) = (g.columns.len(), g.rows.len());
        let (cw, ch) = (plot.w / nc as f64, plot.h / nr as f64);
        let gap = if cw.min(ch) > 6.0 { 1.5 } else { 0.0 };
        let hovered = hover
            .get()
            .filter(|&(x, y)| x >= plot.x && x < plot.x + plot.w && y >= plot.y && y < plot.y + plot.h)
            .map(|(x, y)| ((((y - plot.y) / ch) as usize).min(nr - 1), (((x - plot.x) / cw) as usize).min(nc - 1)));
        let (px, py, pw, ph) = (plot.x, plot.y, plot.w, plot.h);
        let present: Vec<bool> = g.values.iter().map(Option::is_some).collect();
        *h2.borrow_mut() = Some(Rc::new(move |x, y| {
            let inside = x >= px && x < px + pw && y >= py && y < py + ph;
            let i = (((y - py) / ch) as usize).min(nr - 1) * nc + (((x - px) / cw) as usize).min(nc - 1);
            (inside && present.get(i) == Some(&true)).then_some(i)
        }));
        for (i, v) in g.values.iter().enumerate() {
            let Some(v) = v else { continue };
            let (r, c) = (i / nc, i % nc);
            p.set_opacity(shade(*v) * o.emphasis(i));
            p.fill_rect(plot.x + c as f64 * cw + gap / 2.0, plot.y + r as f64 * ch + gap / 2.0, (cw - gap).max(0.5), (ch - gap).max(0.5), color);
            p.set_opacity(1.0);
        }
        // Row labels (every k-th when rows are short), then columns'.
        let every = (((FONT + 2.0) / ch).ceil() as usize).max(1);
        for (r, label) in g.rows.iter().enumerate().step_by(every) {
            p.text(plot.x - 6.0, plot.y + (r as f64 + 0.5) * ch - FONT * 0.65, label, FONT, Color::Secondary, Align::End);
        }
        category_labels(p, &plot, g.columns.iter().map(String::as_str));
        if let Some(((r, c), (x, y))) = hovered.zip(hover.get()) {
            p.stroke_rect(plot.x + c as f64 * cw, plot.y + r as f64 * ch, cw, ch, Color::Primary, 1.5);
            let value = g.values[r * nc + c].map_or_else(|| crate::t("No value"), |v| o.values.value(v));
            tooltip(p, x, y, &[format!("{}, {}: {value}", g.rows[r], g.columns[c])]);
        }
    })
    .on_hover(move |at| hover.set(at))
    .description(move || {
        let g = data.get().unwrap_or_default();
        match (g.extreme(false), g.extreme(true)) {
            (Some(lo), Some(hi)) => format!(
                "Heatmap, {} rows by {} columns, values from {} ({}, {}) to {} ({}, {})",
                g.rows.len(),
                g.columns.len(),
                o2.values.value(lo.2),
                lo.0,
                lo.1,
                o2.values.value(hi.2),
                hi.0,
                hi.1
            ) + &selected_text(&o2, |i| {
                let (n, v) = (g.columns.len(), g.values.get(i).copied().flatten()?);
                Some(format!("{}, {} ({})", g.rows[i / n], g.columns[i % n], o2.values.value(v)))
            }),
            _ => "Heatmap, no data".into(),
        }
    });
    selectable(c, &o3, &hit)
}

fn spark(values: Box<dyn Fn() -> Vec<f64>>, o: Rc<Options>) -> Canvas {
    let data = hold(move || values().into_iter().filter(|v| v.is_finite()).collect::<Vec<f64>>());
    let o2 = o.clone();
    canvas(move |p| {
        // Too small for "No data": an empty sparkline draws nothing.
        let Some(vs) = data.get().filter(|v| !v.is_empty()) else { return };
        let (lo, hi) = vs.iter().fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        let (lo, hi) = with_rules(&o, false, lo, hi);
        let pad = 3.0;
        let (w, h) = ((p.width() - pad * 2.0).max(1.0), (p.height() - pad * 2.0).max(1.0));
        let to_y = move |v: f64| if hi > lo { pad + h - (v - lo) / (hi - lo) * h } else { pad + h / 2.0 };
        let last = (vs.len() - 1) as f64;
        let to_x = move |i: f64| pad + if last > 0.0 { i / last * w } else { w / 2.0 };
        for r in o.rules.iter().filter(|r| !r.vertical) {
            p.set_opacity(0.6);
            p.line(pad, to_y(r.value), pad + w, to_y(r.value), r.color, 1.0);
            p.set_opacity(1.0);
        }
        let color = o.colors.first().copied().unwrap_or(Color::Blue);
        let points: Vec<(f64, f64)> = vs.iter().enumerate().map(|(i, v)| (i as f64, *v)).collect();
        let line: Vec<(f64, f64)> = decimate(&points, 0.0, last + 1.0, w.ceil() as usize).into_iter().map(|(i, v)| (to_x(i), to_y(v))).collect();
        p.polyline(&line, color, 1.5);
        p.fill_circle(to_x(last), to_y(vs[vs.len() - 1]), 2.5, color);
    })
    .description(move || {
        let vs = data.get().unwrap_or_default();
        let Some(last) = vs.last() else { return "Sparkline, no data".into() };
        let (lo, hi) = vs.iter().fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        let v = |x: f64| o2.values.value(x);
        format!("Sparkline, {} values from {} to {}, last {}{}", vs.len(), v(lo), v(hi), v(*last), rules_text(&o2, &o2.x, &o2.values))
    })
    .fixed_height()
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
    fn civil_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 10, 31), 20_757);
        for d in [-1000, 0, 59, 11_017, 20_757, 50_000] {
            let (y, m) = civil_from_days(d);
            assert!(days_from_civil(y, m, 1) <= d && d < days_from_civil(y + i64::from(m == 12), m % 12 + 1, 1), "day {d} in {y}-{m}");
        }
    }

    #[test]
    fn date_ticks_fall_on_whole_units() {
        const D: f64 = 86_400.0;
        let start = days_from_civil(2026, 1, 1) as f64 * D;
        // Ten days, UTC: a tick every day or two, at midnight.
        let (t, template) = date_ticks(start + 3_600.0, start + 10.0 * D, 6, &|_| 0.0);
        assert!(t.iter().all(|t| t % D == 0.0) && t.len() >= 4 && template == "MMMd", "{t:?} {template}");
        // In UTC+2, midnights are two hours earlier.
        let (t, _) = date_ticks(start, start + 10.0 * D, 6, &|_| 7_200.0);
        assert!(t.iter().all(|t| (t + 7_200.0) % D == 0.0), "{t:?}");
        // Two years: the first of a month, every 3 or 6 months.
        let (t, template) = date_ticks(start, start + 730.0 * D, 6, &|_| 0.0);
        let firsts = t.iter().all(|t| {
            let day = (t / D) as i64;
            let (y, m) = civil_from_days(day);
            days_from_civil(y, m, 1) == day
        });
        assert!(firsts && (4..=9).contains(&t.len()) && template == "MMMy", "{t:?} {template}");
        // Daylight saving: an offset of -5 h before day 20_500 and -4 h
        // after still puts every tick at local midnight.
        let dst = |t: f64| if t < 20_500.0 * D { -18_000.0 } else { -14_400.0 };
        let (t, _) = date_ticks(20_490.0 * D, 20_510.0 * D, 10, &dst);
        assert!(t.iter().all(|t| (t + dst(*t)) % D == 0.0) && t.len() >= 8, "{t:?}");
        // Hours.
        let (t, template) = date_ticks(start, start + 6.0 * 3_600.0, 6, &|_| 0.0);
        assert!(t.iter().all(|t| t % 3_600.0 == 0.0) && template == "jm", "{t:?}");
    }

    #[test]
    fn bins_on_round_edges() {
        let (edges, counts) = bin(&[12.0, 18.0, 25.0, 31.0, 33.0, 47.0, 52.0, f64::NAN], 5);
        assert_eq!(edges, vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0]);
        assert_eq!(counts, vec![2, 1, 2, 1, 1]);
        // A value on the last edge counts in the last bin.
        let (edges, counts) = bin(&[0.0, 5.0, 10.0], 2);
        assert_eq!((edges.len() - 1, counts.iter().sum::<usize>(), *counts.last().unwrap() >= 1), (counts.len(), 3, true));
        // One value still makes a bin.
        assert_eq!(bin(&[4.0], 10).1.iter().sum::<usize>(), 1);
        assert_eq!(bin(&[], 10), (vec![], vec![]));
    }

    #[test]
    fn heatmap_cells_by_row_and_column() {
        let g = Cells::new(vec![("9".into(), "Mon".into(), 1.0), ("10".into(), "Mon".into(), 4.0), ("9".into(), "Tue".into(), 2.0)]);
        assert_eq!((g.columns.clone(), g.rows.clone()), (vec!["9".to_string(), "10".to_string()], vec!["Mon".to_string(), "Tue".to_string()]));
        assert_eq!(g.values, vec![Some(1.0), Some(4.0), Some(2.0), None]);
        assert_eq!(g.extreme(true), Some(("Mon", "10", 4.0)));
        assert_eq!(g.range(), Some((1.0, 4.0)));
    }

    #[test]
    fn ranges_stay_inside_the_data() {
        assert_eq!(clamp_range((20.0, 40.0), (0.0, 100.0)), Some((20.0, 40.0)));
        // Moved past an end: kept inside, the same width.
        assert_eq!(clamp_range((90.0, 110.0), (0.0, 100.0)), Some((80.0, 100.0)));
        assert_eq!(clamp_range((40.0, 20.0), (0.0, 100.0)), Some((20.0, 40.0)));
        // All of it (or more) is no zoom.
        assert_eq!(clamp_range((-10.0, 120.0), (0.0, 100.0)), None);
        assert_eq!(clamp_range((f64::NAN, 1.0), (0.0, 100.0)), None);
    }

    #[test]
    fn visible_points_meet_the_edges() {
        let pts = [(0.0, 0.0), (10.0, 10.0), (20.0, 0.0)];
        assert_eq!(visible(&pts, 5.0, 15.0, true), vec![(5.0, 5.0), (10.0, 10.0), (15.0, 5.0)]);
        assert_eq!(visible(&pts, 5.0, 15.0, false), vec![(10.0, 10.0)]);
        // A range between two points: the segment across it.
        assert_eq!(visible(&pts, 2.0, 4.0, true), vec![(2.0, 2.0), (4.0, 4.0)]);
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
