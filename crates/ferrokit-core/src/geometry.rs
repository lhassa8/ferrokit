//! The one conversion point from host layout to AppKit frames (risk 3).

/// A rectangle in the host's logical coordinates: points, origin at the
/// top-left of the window's content area, y growing downwards.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Rect {
    /// A rectangle from its top-left corner and size.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    /// True when it has no area.
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// The overlapping part of two rectangles, if any.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.width).min(other.x + other.width);
        let y1 = (self.y + self.height).min(other.y + other.height);
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
    }

    /// Snaps every edge to the device-pixel grid. Host layouts are fractional
    /// (text line heights, centring), and a native view at a fractional
    /// position draws blurry text. Snapping edges rather than origin and size
    /// keeps adjacent views gap-free.
    pub fn snapped(&self, scale: f64) -> Rect {
        let snap = |v: f64| (v * scale).round() / scale;
        let (x0, y0) = (snap(self.x), snap(self.y));
        Rect::new(x0, y0, snap(self.x + self.width) - x0, snap(self.y + self.height) - y0)
    }
}

/// AppKit frames for a clip container (in the content view's coordinates) and
/// for the view inside it (in the container's flipped coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frames {
    /// The clip container's frame, in the content view's coordinates.
    pub clip: Rect,
    /// The view's frame inside the clip container.
    pub view: Rect,
}

/// `bounds` is where the whole view would be; `visible` the part that may
/// show (e.g. inside a scroll area). Both are snapped. When the content view
/// isn't flipped, the clip frame's y is measured from the bottom.
pub fn frames(bounds: Rect, visible: Rect, scale: f64, content_height: f64, content_flipped: bool) -> Frames {
    let v = visible.snapped(scale);
    let b = bounds.snapped(scale);
    let clip_y = if content_flipped { v.y } else { content_height - v.y - v.height };
    Frames {
        clip: Rect::new(v.x, clip_y, v.width, v.height),
        view: Rect::new(b.x - v.x, b.y - v.y, b.width, b.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_keeps_edges_on_the_grid() {
        let r = Rect::new(265.43, 407.40, 120.5, 40.0).snapped(2.0);
        assert_eq!(r, Rect::new(265.5, 407.5, 120.5, 40.0));
        let r = Rect::new(265.43, 407.40, 120.5, 40.0).snapped(1.0);
        assert_eq!(r, Rect::new(265.0, 407.0, 121.0, 40.0));
    }

    #[test]
    fn adjacent_rects_stay_adjacent() {
        let a = Rect::new(10.3, 0.0, 99.25, 10.0).snapped(1.0);
        let b = Rect::new(109.55, 0.0, 50.0, 10.0).snapped(1.0);
        assert_eq!(a.x + a.width, b.x);
    }

    #[test]
    fn unflipped_content_measures_from_the_bottom() {
        let f = frames(Rect::new(10.0, 20.0, 100.0, 50.0), Rect::new(10.0, 20.0, 100.0, 50.0), 2.0, 600.0, false);
        assert_eq!(f.clip, Rect::new(10.0, 530.0, 100.0, 50.0));
        assert_eq!(f.view, Rect::new(0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn clipped_view_is_offset_inside_its_container() {
        // Scrolled 30 pt: the top 30 pt of the view are cut off.
        let f = frames(Rect::new(0.0, -30.0, 200.0, 100.0), Rect::new(0.0, 0.0, 200.0, 70.0), 2.0, 600.0, true);
        assert_eq!(f.clip, Rect::new(0.0, 0.0, 200.0, 70.0));
        assert_eq!(f.view, Rect::new(0.0, -30.0, 200.0, 100.0));
    }
}
