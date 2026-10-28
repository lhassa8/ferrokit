//! The layout pass (first version, from the P0.4 prototype): every node has
//! an ideal size and a flexibility per axis. Stacks give fixed children their
//! ideal length and share what's left between flexible ones. Groups
//! ([`crate::dynamic`], [`crate::for_each`]) are laid out inline in their
//! parent stack. Frames are snapped to device pixels.

use objc2_foundation::{NSPoint, NSRect, NSSize};

use crate::view::{Axis, Kind, Node, Size};

fn along(s: Size, axis: Axis) -> f64 {
    if axis == Axis::Horizontal { s.w } else { s.h }
}

fn across(s: Size, axis: Axis) -> f64 {
    if axis == Axis::Horizontal { s.h } else { s.w }
}

fn cross_axis(a: Axis) -> Axis {
    if a == Axis::Horizontal { Axis::Vertical } else { Axis::Horizontal }
}

/// Calls `f` for each child of a stack, flattening groups.
fn each_child(children: &[Node], f: &mut dyn FnMut(&Node)) {
    for c in children {
        match &c.kind {
            Kind::Group(g) => each_child(&g.borrow(), f),
            _ => f(c),
        }
    }
}

fn flat(children: &[Node]) -> Vec<*const Node> {
    let mut v = Vec::new();
    each_child(children, &mut |c| v.push(c as *const Node));
    v
}

pub(crate) fn ideal(node: &Node) -> Size {
    let p = node.padding * 2.0;
    let mut s = match &node.kind {
        Kind::Leaf(l) => {
            let s = l.ideal.get().unwrap_or_else(|| {
                let f = l.view.fittingSize();
                let s = Size { w: f.width, h: f.height };
                l.ideal.set(Some(s));
                s
            });
            Size { w: s.w.max(l.min.w), h: s.h.max(l.min.h) }
        }
        Kind::Spacer => Size::default(),
        Kind::Group(g) => stack_ideal(&g.borrow(), Axis::Vertical, 0.0),
        Kind::Stack { axis, spacing, children } => stack_ideal(children, *axis, *spacing),
    };
    s.w += p;
    s.h += p;
    if let Some(w) = node.fixed.0 {
        s.w = w;
    }
    if let Some(h) = node.fixed.1 {
        s.h = h;
    }
    s
}

fn stack_ideal(children: &[Node], axis: Axis, spacing: f64) -> Size {
    let (mut a, mut c, mut n) = (0.0f64, 0.0f64, 0usize);
    each_child(children, &mut |ch| {
        let s = ideal(ch);
        a += along(s, axis);
        c = c.max(across(s, axis));
        n += 1;
    });
    a += spacing * n.saturating_sub(1) as f64;
    if axis == Axis::Horizontal { Size { w: a, h: c } } else { Size { w: c, h: a } }
}

/// Whether the node grows along `axis` (spacers only along their stack's).
pub(crate) fn flexible(node: &Node, axis: Axis, parent: Option<Axis>) -> bool {
    let fixed = if axis == Axis::Horizontal { node.fixed.0 } else { node.fixed.1 };
    if fixed.is_some() {
        return false;
    }
    match &node.kind {
        Kind::Leaf(l) => {
            if axis == Axis::Horizontal {
                l.flex.0
            } else {
                l.flex.1
            }
        }
        Kind::Spacer => parent == Some(axis),
        Kind::Group(g) => g.borrow().iter().any(|c| flexible(c, axis, parent)),
        Kind::Stack { axis: own, children, .. } => {
            let mut any = false;
            each_child(children, &mut |c| any |= flexible(c, axis, Some(*own)));
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

/// Places `node` (and its subtree) in the rectangle `x, y, w, h` of the
/// flipped container.
pub(crate) fn place(node: &Node, x: f64, y: f64, w: f64, h: f64, snap: &Snap) {
    let p = node.padding;
    let (x, y, w, h) = (x + p, y + p, (w - 2.0 * p).max(0.0), (h - 2.0 * p).max(0.0));
    match &node.kind {
        Kind::Leaf(l) => {
            let (x0, y0) = (snap.r(x), snap.r(y));
            let r = NSRect::new(NSPoint::new(x0, y0), NSSize::new(snap.r(x + w) - x0, snap.r(y + h) - y0));
            if l.view.frame() != r {
                l.view.setFrame(r);
            }
        }
        Kind::Spacer => {}
        Kind::Group(g) => place_stack(&g.borrow(), Axis::Vertical, 0.0, x, y, w, h, snap),
        Kind::Stack { axis, spacing, children } => place_stack(children, *axis, *spacing, x, y, w, h, snap),
    }
}

#[allow(clippy::too_many_arguments)]
fn place_stack(children: &[Node], axis: Axis, spacing: f64, x: f64, y: f64, w: f64, h: f64, snap: &Snap) {
    // SAFETY of the raw pointers: `children` (and the groups they borrow) are
    // borrowed for the whole call; nothing mutates the tree during layout.
    let kids = flat(children);
    let kids: Vec<&Node> = kids.into_iter().map(|p| unsafe { &*p }).collect();
    let len = if axis == Axis::Horizontal { w } else { h };
    let cross = if axis == Axis::Horizontal { h } else { w };
    let ideals: Vec<Size> = kids.iter().map(|c| ideal(c)).collect();
    let flex: Vec<bool> = kids.iter().map(|c| flexible(c, axis, Some(axis))).collect();
    let fixed: f64 = ideals.iter().zip(&flex).filter(|(_, f)| !**f).map(|(s, _)| along(*s, axis)).sum();
    let gaps = spacing * kids.len().saturating_sub(1) as f64;
    let n_flex = flex.iter().filter(|f| **f).count().max(1) as f64;
    let share = ((len - fixed - gaps) / n_flex).max(0.0);
    let mut pos = 0.0;
    for ((child, s), f) in kids.iter().zip(&ideals).zip(&flex) {
        let a = if *f { share.max(along(*s, axis)) } else { along(*s, axis) };
        let c = if flexible(child, cross_axis(axis), Some(axis)) { cross } else { across(*s, axis).min(cross) };
        // Vertical stacks align leading; horizontal ones centre.
        let off = if axis == Axis::Horizontal { (cross - c) / 2.0 } else { 0.0 };
        if axis == Axis::Horizontal {
            place(child, x + pos, y + off, a, c, snap);
        } else {
            place(child, x + off, y + pos, c, a, snap);
        }
        pos += a + spacing;
    }
}
