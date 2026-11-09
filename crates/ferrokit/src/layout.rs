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
        // A hidden view takes no room (see `ViewExt::hidden`).
        if c.meta.hidden.get() {
            continue;
        }
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
    if node.meta.hidden.get() {
        return Size::default();
    }
    if let Some(s) = node.meta.ideal.get() {
        return s;
    }
    bump(&MEASURED);
    let p = (node.padding.0 * 2.0, node.padding.1 * 2.0);
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
        Kind::Layers { children, .. } => {
            let mut s = Size::default();
            each_child(children, false, &mut |c| {
                let i = ideal(c);
                s = Size { w: s.w.max(i.w), h: s.h.max(i.h) };
            });
            s
        }
        Kind::Split { panes, stacked, .. } => {
            let (mut w, mut h) = (0.0f64, 0.0f64);
            for (_, n) in panes {
                let s = ideal(n);
                if *stacked {
                    h += s.h + 1.0;
                    w = w.max(s.w);
                } else {
                    w += s.w + 1.0;
                    h = h.max(s.h);
                }
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
    s.w += p.0;
    s.h += p.1;
    if let Some(w) = node.fixed.0 {
        s.w = w;
    }
    if let Some(h) = node.fixed.1 {
        s.h = h;
    }
    s.w = s.w.max(node.min.0.unwrap_or(0.0));
    s.h = s.h.max(node.min.1.unwrap_or(0.0));
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
        Kind::Layers { children, .. } => {
            let mut any = false;
            each_child(children, false, &mut |c| any |= flexible(c, axis, None));
            any
        }
    }
}

pub(crate) struct Snap(pub f64);

impl Snap {
    fn r(&self, v: f64) -> f64 {
        (v * self.0).round() / self.0
    }
}

thread_local! {
    /// Right to left: the width of the container being laid out, whose
    /// frames are mirrored within it (`None` left to right).
    static MIRROR: Cell<Option<f64>> = const { Cell::new(None) };
}

/// Runs `f` laying out a container `width` wide, mirrored right to left
/// when `rtl`.
pub(crate) fn mirrored<R>(rtl: bool, width: f64, f: impl FnOnce() -> R) -> R {
    let prev = MIRROR.with(|m| m.replace(rtl.then_some(width)));
    let r = f();
    MIRROR.with(|m| m.set(prev));
    r
}

/// Whether the container being laid out is mirrored.
fn rtl() -> bool {
    MIRROR.with(Cell::get).is_some()
}

/// `r` as it lands: mirrored across the container right to left (the
/// layout computes left to right; this flips every frame together, as
/// AppKit's own right-to-left layout does).
fn out(r: Rect) -> Rect {
    match MIRROR.with(Cell::get) {
        Some(width) => Rect { x: width - r.x - r.w, ..r },
        None => r,
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
    for d in &node.decor {
        Native::place(d, out(snapped(x, y, w, h, snap)));
    }
    place_body(node, x, y, w, h, 0.0, snap);
    finish(&node.meta, rect);
}

/// Places a window's root node in its `w × h` content view, under a title
/// bar and toolbar `top` points tall (a window with full-size content). Its
/// backgrounds and materials reach the top edge, and so do a split view's
/// panes (a sidebar's material runs up behind the window buttons), while
/// the content starts below the bar.
pub(crate) fn place_under_bar(node: &Node, w: f64, h: f64, top: f64, snap: &Snap) {
    place_under_bar_at(node, 0.0, w, h, top, snap);
}

/// [`place_under_bar`] for a node `w` wide at `x`.
fn place_under_bar_at(node: &Node, x: f64, w: f64, h: f64, top: f64, snap: &Snap) {
    if top <= 0.0 {
        return place(node, x, 0.0, w, h, snap);
    }
    // Its own key: the same size under a different bar is a new layout.
    let rect = [x, -top, w, h];
    if !node.meta.dirty.get() && node.meta.rect.get() == Some(rect) {
        bump(&SKIPPED);
        return;
    }
    bump(&PLACED);
    for d in &node.decor {
        Native::place(d, out(snapped(x, 0.0, w, h, snap)));
    }
    match &node.kind {
        Kind::Split { .. } => place_body(node, x, 0.0, w, h, top, snap),
        Kind::Stack { axis: Axis::Horizontal, children, .. } => {
            place_body(node, x, top, w, (h - top).max(0.0), 0.0, snap);
            // The columns of a row reach up too, when they draw something
            // (a sidebar's material, a pane's background): as a split's do.
            each_child(children, false, &mut |c| {
                if let (false, Some([cx, _, cw, _])) = (c.decor.is_empty(), c.meta.rect.get()) {
                    place_under_bar_at(c, cx, cw, h, top, snap);
                }
            });
        }
        _ => place_body(node, x, top, w, (h - top).max(0.0), 0.0, snap),
    }
    finish(&node.meta, rect);
}

/// Places what's inside `node`'s padding. `top` is a title bar over a split
/// view's panes (see [`place_under_bar`]).
fn place_body(node: &Node, x: f64, y: f64, w: f64, h: f64, top: f64, snap: &Snap) {
    let (px, py) = node.padding;
    let (ix, iy, iw, ih) = (x + px, y + py, (w - 2.0 * px).max(0.0), (h - 2.0 * py).max(0.0));
    match &node.kind {
        Kind::Leaf(l) => Native::place(&l.view, out(snapped(ix, iy, iw, ih, snap))),
        Kind::Spacer => {}
        Kind::Scroll { scroll, doc, content } => {
            let visible = Native::place_scroll(scroll, out(snapped(ix, iy, iw, ih, snap)));
            let mut size = Size { w: visible.w, h: ideal(content).h.max(visible.h) };
            Native::set_size(doc, size);
            // Classic (always-shown) scroll bars appear once the document is
            // taller than the view and take its width: size it again to fit.
            let after = Native::visible_size(scroll);
            if (after.w - size.w).abs() > 0.25 {
                size = Size { w: after.w, h: ideal(content).h.max(after.h) };
                Native::set_size(doc, size);
            }
            // The document is a container of its own.
            mirrored(rtl(), size.w, || place(content, 0.0, 0.0, size.w, size.h, snap));
        }
        Kind::Split { split, panes, position, trailing, stacked } => {
            Native::place_split(split, out(snapped(ix, iy, iw, ih, snap)));
            // A stacked split under the title bar: the app's position counts
            // from below the bar, where its top pane's content starts (a
            // saved one is the pane's whole height). The bar is known once
            // the split is in its window, so it waits for that.
            if let Some((p, given)) = position.get().filter(|(_, given)| !(*stacked && *given) || Native::in_window(split)) {
                position.set(None);
                let bar = if *stacked && given { top } else { 0.0 };
                Native::set_divider(split, 0, p + bar);
            }
            // Not into a hidden (collapsed) last pane: moving its divider
            // would open it. It opens at this size when shown.
            if let Some(t) = trailing.take().filter(|_| panes.len() >= 2 && !panes.last().is_some_and(|(p, _)| Native::is_hidden(p))) {
                let along = if *stacked { ih } else { iw };
                Native::set_divider(split, panes.len() - 2, along - t - 1.0);
            }
            for (i, (pane, child)) in panes.iter().enumerate() {
                let b = Native::size(pane);
                // Stacked, only the top pane is under the bar.
                let bar = if *stacked && i > 0 { 0.0 } else { top };
                // Each pane is a container of its own (AppKit orders the
                // panes right to left itself).
                mirrored(rtl(), b.w, || place_under_bar(child, b.w, b.h, bar, snap));
            }
        }
        Kind::Group(g) => place_stack(&g.borrow(), Axis::Vertical, 0.0, None, [ix, iy, iw, ih], snap),
        Kind::Stack { axis, spacing, align, children } => {
            place_stack(children, *axis, *spacing, *align, [ix, iy, iw, ih], snap)
        }
        Kind::Grid { spacing, rows, align } => place_grid(rows, *spacing, align, [ix, iy, iw, ih], snap),
        Kind::Layers { align, children } => {
            let offset = |a: Align, room: f64, len: f64| match a {
                Align::Start => 0.0,
                Align::Center => (room - len) / 2.0,
                Align::End => room - len,
            };
            for c in flat(children, true) {
                let s = ideal(c);
                let w = if flexible(c, Axis::Horizontal, None) { iw } else { s.w.min(iw) };
                let h = if flexible(c, Axis::Vertical, None) { ih } else { s.h.min(ih) };
                place(c, ix + offset(align.0, iw, w), iy + offset(align.1, ih, h), w, h, snap);
            }
        }
    }
}

fn finish(meta: &Meta, rect: [f64; 4]) {
    meta.rect.set(Some(rect));
    meta.dirty.set(false);
}

/// Gives a stack's flexible children equal lengths out of `total` (the
/// stack's length less its spacing), so a row of cards that all fill come
/// out the same width whatever their content. A child whose own length is
/// more than its share keeps it, and the rest share what's left.
fn share(lengths: &mut [f64], flex: &[bool], total: f64) {
    let fixed: f64 = lengths.iter().zip(flex).filter(|(_, f)| !**f).map(|(l, _)| l).sum();
    let mut pool = total - fixed;
    let mut open: Vec<usize> = (0..lengths.len()).filter(|&i| flex[i]).collect();
    loop {
        if open.is_empty() {
            return;
        }
        let each = pool / open.len() as f64;
        let (big, small): (Vec<usize>, Vec<usize>) = open.iter().partition(|&&i| lengths[i] > each);
        if big.is_empty() {
            for i in small {
                lengths[i] = each;
            }
            return;
        }
        pool -= big.iter().map(|&i| lengths[i]).sum::<f64>();
        open = small;
    }
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
    if len > used {
        share(&mut lengths, &flex, len - spacing * kids.len().saturating_sub(1) as f64);
    } else if len < used {
        // Short of room: children that can shrink (truncating text, or rows
        // holding it) give it up, in proportion to their length.
        // A minimum length (`min_width`) is a floor: such a child shrinks
        // to it and no further, and the others give up the rest.
        let can: Vec<bool> = kids.iter().map(|c| shrinkable(c, axis)).collect();
        let floors: Vec<f64> = kids.iter().map(|c| min_along(c, axis)).collect();
        shrink(&mut lengths, &can, &floors, used - len);
    }
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

/// Takes `cut` from the lengths that can shrink, in proportion to each, but
/// none below its floor; what a floored one can't give, the others do.
fn shrink(lengths: &mut [f64], can: &[bool], floors: &[f64], mut cut: f64) {
    for _ in 0..lengths.len() {
        let total: f64 = lengths.iter().zip(can).zip(floors).filter(|((l, c), f)| **c && **l > **f).map(|((l, _), _)| *l).sum();
        if total <= 0.0 || cut <= 0.01 {
            return;
        }
        let step = cut.min(total);
        for ((l, c), f) in lengths.iter_mut().zip(can).zip(floors) {
            if *c && *l > *f {
                let new = (*l - step * *l / total).max(*f).max(0.0);
                cut -= *l - new;
                *l = new;
            }
        }
    }
}

/// The minimum length set on `node` along `axis` (`min_width` /
/// `min_height`), or zero.
fn min_along(node: &Node, axis: Axis) -> f64 {
    let min = if axis == Axis::Horizontal { node.min.0 } else { node.min.1 };
    min.unwrap_or(0.0)
}

/// Whether `node` can be shorter than its ideal length along `axis`: a
/// truncating text, or a stack or box holding one, with no fixed length that
/// way. A minimum length doesn't stop it; it shrinks down to it.
fn shrinkable(node: &Node, axis: Axis) -> bool {
    let fixed = if axis == Axis::Horizontal { node.fixed.0 } else { node.fixed.1 };
    if fixed.is_some() {
        return false;
    }
    match &node.kind {
        Kind::Leaf(_) => node.shrinks && axis == Axis::Horizontal,
        Kind::Stack { children, .. } | Kind::Layers { children, .. } => {
            let mut any = false;
            each_child(children, false, &mut |c| any |= shrinkable(c, axis));
            any
        }
        Kind::Group(g) => g.borrow().iter().any(|c| shrinkable(c, axis)),
        _ => false,
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

#[cfg(test)]
mod tests {
    use super::shrink;

    #[test]
    fn shrink_stops_at_a_floor_and_the_others_give_the_rest() {
        // 300 + 300 into 220: the first stops at 110, the second takes the rest.
        let mut l = [300.0, 300.0];
        shrink(&mut l, &[true, true], &[110.0, 0.0], 380.0);
        assert!((l[0] - 110.0).abs() < 0.02 && (l[1] - 110.0).abs() < 0.02, "{l:?}");
        // Without floors, in proportion.
        let mut l = [300.0, 100.0];
        shrink(&mut l, &[true, true], &[0.0, 0.0], 200.0);
        assert!((l[0] - 150.0).abs() < 0.02 && (l[1] - 50.0).abs() < 0.02, "{l:?}");
        // A child that can't shrink keeps its length.
        let mut l = [300.0, 100.0];
        shrink(&mut l, &[true, false], &[0.0, 0.0], 100.0);
        assert_eq!(l, [200.0, 100.0]);
    }
}
