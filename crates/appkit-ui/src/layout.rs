//! The layout pass.
//!
//! Every node has an ideal size (cached in its [`Meta`] until something in its
//! subtree changes) and a flexibility per axis. Stacks give fixed children
//! their ideal length and share what's left between flexible ones; grids size
//! columns and rows to their largest cells. Groups ([`crate::dynamic`],
//! [`crate::for_each`]) are laid out inline in their parent.
//!
//! A node whose subtree is clean and whose rectangle didn't change is skipped
//! with its whole subtree, so a change re-lays out only the path from the
//! changed leaf to the root and the siblings it moved. Frames are snapped to
//! device pixels.

use std::cell::Cell;

use crate::platform::{Native, Platform, Rect};
use crate::view::{Align, Axis, Kind, Meta, Node, Size};

thread_local! {
    static PLACED: Cell<u64> = const { Cell::new(0) };
    static SKIPPED: Cell<u64> = const { Cell::new(0) };
    static MEASURED: Cell<u64> = const { Cell::new(0) };
}

/// Layout counters since the app started: nodes placed, subtrees skipped as
/// clean, and ideal sizes computed (cache misses). Diagnostic.
pub fn layout_stats() -> (u64, u64, u64) {
    (PLACED.with(Cell::get), SKIPPED.with(Cell::get), MEASURED.with(Cell::get))
}

fn bump(c: &'static std::thread::LocalKey<Cell<u64>>) {
    c.with(|v| v.set(v.get() + 1));
}

fn along(s: Size, axis: Axis) -> f64 {
    if axis == Axis::Horizontal { s.w } else { s.h }
}

fn across(s: Size, axis: Axis) -> f64 {
    if axis == Axis::Horizontal { s.h } else { s.w }
}

fn cross_axis(a: Axis) -> Axis {
    if a == Axis::Horizontal { Axis::Vertical } else { Axis::Horizontal }
}

/// Calls `f` for each child, flattening groups (and clearing the groups'
/// dirty flags when `clean` is set: they are laid out through their parent).
fn each_child<'a>(children: &'a [Node], clean: bool, f: &mut dyn FnMut(&'a Node)) {
    for c in children {
        match &c.kind {
            Kind::Group(g) => {
                if clean {
                    c.meta.dirty.set(false);
                }
                // SAFETY: the group's children are not mutated during layout
                // (layout runs outside effects), so the borrow can be released
                // while references into the vector are in use.
                let kids: &'a [Node] = unsafe { &*(&*g.borrow() as *const Vec<Node>) };
                each_child(kids, clean, f);
            }
            _ => f(c),
        }
    }
}

fn flat(children: &[Node], clean: bool) -> Vec<&Node> {
    let mut v = Vec::new();
    each_child(children, clean, &mut |c| v.push(c));
    v
}

/// The node's ideal size (cached until its subtree changes).
pub(crate) fn ideal(node: &Node) -> Size {
    if let Some(s) = node.meta.ideal.get() {
        return s;
    }
    bump(&MEASURED);
    let p = node.padding * 2.0;
    let mut s = match &node.kind {
        Kind::Leaf(l) => {
            let s = l.content.get().unwrap_or_else(|| {
                let s = Native::fitting_size(&l.view);
                l.content.set(Some(s));
                s
            });
            Size { w: s.w.max(l.min.w), h: s.h.max(l.min.h) }
        }
        Kind::Spacer => Size::default(),
        Kind::Group(g) => stack_ideal(&g.borrow(), Axis::Vertical, 0.0),
        Kind::Stack { axis, spacing, children, .. } => stack_ideal(children, *axis, *spacing),
        Kind::Split { panes, .. } => {
            let (mut w, mut h) = (0.0f64, 0.0f64);
            for (_, n) in panes {
                let s = ideal(n);
                w += s.w + 1.0;
                h = h.max(s.h);
            }
            Size { w, h }
        }
        Kind::Scroll { content, .. } => {
            // Room for the content's width plus a scroller; a modest height.
            let c = ideal(content);
            Size { w: c.w + 16.0, h: c.h.min(200.0) }
        }
        Kind::Grid { spacing, rows, .. } => {
            let (cols, rws) = grid_tracks(rows);
            Size {
                w: cols.iter().sum::<f64>() + spacing.0 * cols.len().saturating_sub(1) as f64,
                h: rws.iter().sum::<f64>() + spacing.1 * rws.len().saturating_sub(1) as f64,
            }
        }
    };
    s.w += p;
    s.h += p;
    if let Some(w) = node.fixed.0 {
        s.w = w;
    }
    if let Some(h) = node.fixed.1 {
        s.h = h;
    }
    node.meta.ideal.set(Some(s));
    s
}

fn stack_ideal(children: &[Node], axis: Axis, spacing: f64) -> Size {
    let (mut a, mut c, mut n) = (0.0f64, 0.0f64, 0usize);
    each_child(children, false, &mut |ch| {
        let s = ideal(ch);
        a += along(s, axis);
        c = c.max(across(s, axis));
        n += 1;
    });
    a += spacing * n.saturating_sub(1) as f64;
    if axis == Axis::Horizontal { Size { w: a, h: c } } else { Size { w: c, h: a } }
}

/// Column widths and row heights: the largest ideal cell in each.
fn grid_tracks(rows: &[Vec<Node>]) -> (Vec<f64>, Vec<f64>) {
    let n_cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut cols = vec![0.0f64; n_cols];
    let mut rws = Vec::with_capacity(rows.len());
    for row in rows {
        let mut h = 0.0f64;
        for (i, cell) in row.iter().enumerate() {
            let s = ideal(cell);
            cols[i] = cols[i].max(s.w);
            h = h.max(s.h);
        }
        rws.push(h);
    }
    (cols, rws)
}

/// Whether the node grows along `axis` (spacers only along their stack's).
pub(crate) fn flexible(node: &Node, axis: Axis, parent: Option<Axis>) -> bool {
    let (fixed, fill) =
        if axis == Axis::Horizontal { (node.fixed.0, node.fill.0) } else { (node.fixed.1, node.fill.1) };
    if fixed.is_some() {
        return false;
    }
    if fill {
        return true;
    }
    match &node.kind {
        Kind::Leaf(l) if l.cross_only => parent.map_or(axis == Axis::Horizontal, |p| p != axis),
        Kind::Leaf(l) => {
            if axis == Axis::Horizontal {
                l.flex.0
            } else {
                l.flex.1
            }
        }
        Kind::Spacer => parent == Some(axis),
        Kind::Scroll { .. } | Kind::Split { .. } => true,
        Kind::Group(g) => g.borrow().iter().any(|c| flexible(c, axis, parent)),
        Kind::Stack { axis: own, children, .. } => {
            let mut any = false;
            each_child(children, false, &mut |c| any |= flexible(c, axis, Some(*own)));
            any
        }
        Kind::Grid { rows, .. } => rows.iter().flatten().any(|c| flexible(c, axis, None)),
    }
}

pub(crate) struct Snap(pub f64);

impl Snap {
    fn r(&self, v: f64) -> f64 {
        (v * self.0).round() / self.0
    }
}

/// The rectangle `x, y, w, h` with its edges on device pixels.
fn snapped(x: f64, y: f64, w: f64, h: f64, snap: &Snap) -> Rect {
    let (x0, y0) = (snap.r(x), snap.r(y));
    Rect { x: x0, y: y0, w: snap.r(x + w) - x0, h: snap.r(y + h) - y0 }
}

/// Places `node` (and its subtree) in the rectangle `x, y, w, h` of the
/// flipped container, skipping it if it is clean and its rectangle is the same.
pub(crate) fn place(node: &Node, x: f64, y: f64, w: f64, h: f64, snap: &Snap) {
    let rect = [x, y, w, h];
    if !node.meta.dirty.get() && node.meta.rect.get() == Some(rect) {
        bump(&SKIPPED);
        return;
    }
    bump(&PLACED);
    let p = node.padding;
    let (ix, iy, iw, ih) = (x + p, y + p, (w - 2.0 * p).max(0.0), (h - 2.0 * p).max(0.0));
    match &node.kind {
        Kind::Leaf(l) => Native::place(&l.view, snapped(ix, iy, iw, ih, snap)),
        Kind::Spacer => {}
        Kind::Scroll { scroll, doc, content } => {
            let visible = Native::place_scroll(scroll, snapped(ix, iy, iw, ih, snap));
            let mut size = Size { w: visible.w, h: ideal(content).h.max(visible.h) };
            Native::set_size(doc, size);
            // Classic (always-shown) scroll bars appear once the document is
            // taller than the view and take its width: size it again to fit.
            let after = Native::visible_size(scroll);
            if (after.w - size.w).abs() > 0.25 {
                size = Size { w: after.w, h: ideal(content).h.max(after.h) };
                Native::set_size(doc, size);
            }
            place(content, 0.0, 0.0, size.w, size.h, snap);
        }
        Kind::Split { split, panes, position } => {
            Native::place_split(split, snapped(ix, iy, iw, ih, snap));
            if let Some(p) = position.take() {
                Native::set_divider(split, p);
            }
            for (pane, child) in panes {
                let b = Native::size(pane);
                place(child, 0.0, 0.0, b.w, b.h, snap);
            }
        }
        Kind::Group(g) => place_stack(&g.borrow(), Axis::Vertical, 0.0, None, [ix, iy, iw, ih], snap),
        Kind::Stack { axis, spacing, align, children } => {
            place_stack(children, *axis, *spacing, *align, [ix, iy, iw, ih], snap)
        }
        Kind::Grid { spacing, rows, align } => place_grid(rows, *spacing, align, [ix, iy, iw, ih], snap),
    }
    finish(&node.meta, rect);
}

fn finish(meta: &Meta, rect: [f64; 4]) {
    meta.rect.set(Some(rect));
    meta.dirty.set(false);
}

/// Shares `extra` between the flexible tracks (or items).
fn distribute(sizes: &mut [f64], flex: &[bool], extra: f64) {
    let n = flex.iter().filter(|f| **f).count();
    if n == 0 || extra <= 0.0 {
        return;
    }
    let each = extra / n as f64;
    for (s, f) in sizes.iter_mut().zip(flex) {
        if *f {
            *s += each;
        }
    }
}

fn place_stack(children: &[Node], axis: Axis, spacing: f64, align: Option<Align>, r: [f64; 4], snap: &Snap) {
    let [x, y, w, h] = r;
    let kids = flat(children, true);
    let len = if axis == Axis::Horizontal { w } else { h };
    let cross = if axis == Axis::Horizontal { h } else { w };
    let ideals: Vec<Size> = kids.iter().map(|c| ideal(c)).collect();
    let flex: Vec<bool> = kids.iter().map(|c| flexible(c, axis, Some(axis))).collect();
    let mut lengths: Vec<f64> = ideals.iter().map(|s| along(*s, axis)).collect();
    let used: f64 = lengths.iter().sum::<f64>() + spacing * kids.len().saturating_sub(1) as f64;
    distribute(&mut lengths, &flex, len - used);
    let align = align.unwrap_or(if axis == Axis::Horizontal { Align::Center } else { Align::Start });
    let mut pos = 0.0;
    for ((child, s), a) in kids.iter().zip(&ideals).zip(&lengths) {
        let c = if flexible(child, cross_axis(axis), Some(axis)) { cross } else { across(*s, axis).min(cross) };
        let off = match align {
            Align::Start => 0.0,
            Align::Center => (cross - c) / 2.0,
            Align::End => cross - c,
        };
        if axis == Axis::Horizontal {
            place(child, x + pos, y + off, *a, c, snap);
        } else {
            place(child, x + off, y + pos, c, *a, snap);
        }
        pos += a + spacing;
    }
}

fn place_grid(rows: &[Vec<Node>], spacing: (f64, f64), align: &[Align], r: [f64; 4], snap: &Snap) {
    let [x, y, w, h] = r;
    let (mut cols, mut rws) = grid_tracks(rows);
    let n_cols = cols.len();
    let col_flex: Vec<bool> =
        (0..n_cols).map(|i| rows.iter().any(|row| row.get(i).is_some_and(|c| flexible(c, Axis::Horizontal, None)))).collect();
    let row_flex: Vec<bool> = rows.iter().map(|row| row.iter().any(|c| flexible(c, Axis::Vertical, None))).collect();
    let used_w = cols.iter().sum::<f64>() + spacing.0 * n_cols.saturating_sub(1) as f64;
    let used_h = rws.iter().sum::<f64>() + spacing.1 * rws.len().saturating_sub(1) as f64;
    distribute(&mut cols, &col_flex, w - used_w);
    distribute(&mut rws, &row_flex, h - used_h);
    let mut cy = y;
    for (row, rh) in rows.iter().zip(&rws) {
        let mut cx = x;
        for (i, (cell, cw)) in row.iter().zip(&cols).enumerate() {
            let s = ideal(cell);
            let cell_w = if flexible(cell, Axis::Horizontal, None) { *cw } else { s.w.min(*cw) };
            let cell_h = if flexible(cell, Axis::Vertical, None) { *rh } else { s.h.min(*rh) };
            // Cells align per column (leading by default), centred vertically.
            let dx = match align.get(i).copied().unwrap_or(Align::Start) {
                Align::Start => 0.0,
                Align::Center => (cw - cell_w) / 2.0,
                Align::End => cw - cell_w,
            };
            place(cell, cx + dx, cy + (rh - cell_h) / 2.0, cell_w, cell_h, snap);
            cx += cw + spacing.0;
        }
        cy += rh + spacing.1;
    }
}
