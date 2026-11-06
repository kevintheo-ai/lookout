//! Geometry in Cocoa coordinates (origin bottom-left, y up) with the semantics of CoreGraphics' `CGRect`: every
//! accessor works on the standardized rectangle (a negative size flips its edges), equality compares standardized
//! rectangles, and `Rect::NULL` (origin at +infinity) behaves like `CGRect.null`. How `intersects`, `contains` and
//! `intersection` treat touching and zero-size rectangles was measured against CoreGraphics on macOS 27 (2026-11-06).

use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Compared field by field, like `CGSize`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// `CGRect`: origin `(x, y)` and size `(w, h)` as stored; the methods give the standardized view.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const ZERO: Rect = Rect::new(0.0, 0.0, 0.0, 0.0);
    /// `CGRect.null`: what an intersection of disjoint rectangles yields.
    pub const NULL: Rect = Rect::new(f64::INFINITY, f64::INFINITY, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn origin(self) -> Point {
        Point {
            x: self.x,
            y: self.y,
        }
    }

    pub fn size(self) -> Size {
        Size {
            w: self.w,
            h: self.h,
        }
    }

    pub fn min_x(self) -> f64 {
        self.x.min(self.x + self.w)
    }

    pub fn max_x(self) -> f64 {
        self.x.max(self.x + self.w)
    }

    pub fn mid_x(self) -> f64 {
        self.x + self.w / 2.0
    }

    pub fn min_y(self) -> f64 {
        self.y.min(self.y + self.h)
    }

    pub fn max_y(self) -> f64 {
        self.y.max(self.y + self.h)
    }

    pub fn mid_y(self) -> f64 {
        self.y + self.h / 2.0
    }

    pub fn width(self) -> f64 {
        self.w.abs()
    }

    pub fn height(self) -> f64 {
        self.h.abs()
    }

    pub fn is_null(self) -> bool {
        self.x == f64::INFINITY || self.y == f64::INFINITY
    }

    pub fn is_empty(self) -> bool {
        self.is_null() || self.w == 0.0 || self.h == 0.0
    }

    pub fn offset_by(self, dx: f64, dy: f64) -> Rect {
        if self.is_null() {
            return self;
        }
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// The common area; touching rectangles share a zero-width strip (not null), disjoint ones give `Rect::NULL`.
    pub fn intersection(self, other: Rect) -> Rect {
        if self.is_null() || other.is_null() {
            return Rect::NULL;
        }
        let (x1, x2) = (
            self.min_x().max(other.min_x()),
            self.max_x().min(other.max_x()),
        );
        let (y1, y2) = (
            self.min_y().max(other.min_y()),
            self.max_y().min(other.max_y()),
        );
        if x1 > x2 || y1 > y2 {
            return Rect::NULL;
        }
        Rect::new(x1, y1, x2 - x1, y2 - y1)
    }

    /// Do the two overlap? Touching edges do not; a zero-size rectangle counts on the low edge and inside, not on
    /// the high edge (CoreGraphics).
    pub fn intersects(self, other: Rect) -> bool {
        !self.is_null()
            && !other.is_null()
            && spans_overlap(self.min_x(), self.max_x(), other.min_x(), other.max_x())
            && spans_overlap(self.min_y(), self.max_y(), other.min_y(), other.max_y())
    }

    /// Lies `other` within this rectangle, edges included? Every rectangle contains the null rectangle.
    pub fn contains(self, other: Rect) -> bool {
        other.is_null()
            || (!self.is_null()
                && other.min_x() >= self.min_x()
                && other.max_x() <= self.max_x()
                && other.min_y() >= self.min_y()
                && other.max_y() <= self.max_y())
    }
}

/// Half-open spans `[a0, a1)` and `[b0, b1)`; a zero-length span still holds its own point.
fn spans_overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> bool {
    let (lo, hi) = (a0.max(b0), a1.min(b1));
    lo < hi || (lo == hi && (lo < a1 || a0 == a1) && (lo < b1 || b0 == b1))
}

impl PartialEq for Rect {
    /// `CGRect ==`: standardized rectangles compared; all null rectangles are equal.
    fn eq(&self, other: &Rect) -> bool {
        match (self.is_null(), other.is_null()) {
            (true, true) => true,
            (false, false) => {
                self.min_x() == other.min_x()
                    && self.min_y() == other.min_y()
                    && self.width() == other.width()
                    && self.height() == other.height()
            }
            _ => false,
        }
    }
}

/// Order of two floats as Swift's `<` sees them (`-0 == 0`); NaN, which `<` never orders, falls back to the total
/// order so that sorting stays well defined.
pub fn cmp_f64(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or_else(|| a.total_cmp(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rect = Rect::new(0.0, 0.0, 10.0, 10.0);

    /// Each case was measured against CoreGraphics (Swift, macOS 27).
    #[test]
    fn rect_matches_core_graphics() {
        let touch = Rect::new(10.0, 0.0, 10.0, 10.0);
        assert!(!A.intersects(touch) && A.intersection(touch) == Rect::new(10.0, 0.0, 0.0, 10.0));
        assert!(!A.intersects(Rect::new(10.0, 10.0, 5.0, 5.0)), "corner");
        let point = Rect::new(5.0, 5.0, 0.0, 0.0);
        assert!(
            A.intersects(point) && A.contains(point) && point.intersects(A) && !point.contains(A)
        );
        let on_low_edge = Rect::new(0.0, 2.0, 0.0, 4.0);
        let on_high_edge = Rect::new(10.0, 2.0, 0.0, 4.0);
        assert!(
            A.intersects(on_low_edge) && !A.intersects(on_high_edge) && A.contains(on_high_edge)
        );
        assert!(Rect::ZERO.intersects(Rect::ZERO) && Rect::ZERO.contains(Rect::ZERO));
        assert!(!Rect::new(0.0, 0.0, 0.0, 0.0).intersects(Rect::new(1.0, 1.0, 0.0, 0.0)));
        let cross = (
            Rect::new(0.0, 3.0, 10.0, 0.0),
            Rect::new(5.0, 0.0, 0.0, 10.0),
        );
        assert!(cross.0.intersects(cross.1) && !cross.0.contains(cross.1));
        assert!(
            !Rect::new(0.0, 3.0, 10.0, 0.0).intersects(Rect::new(10.0, 3.0, 10.0, 0.0)),
            "touching lines"
        );
        assert!(
            A.intersects(Rect::new(9.999, 0.0, 10.0, 10.0))
                && !A.contains(Rect::new(0.0, 0.0, 11.0, 10.0))
        );
        assert!(A.intersection(Rect::new(10.0001, 0.0, 2.0, 10.0)).is_null());

        let neg = Rect::new(10.0, 0.0, -10.0, 5.0);
        assert!(
            neg.min_x() == 0.0 && neg.max_x() == 10.0 && neg.width() == 10.0 && neg.mid_x() == 5.0
        );
        assert!(neg == Rect::new(0.0, 0.0, 10.0, 5.0) && A.contains(neg));
        assert!(
            A == Rect::new(10.0, 10.0, -10.0, -10.0)
                && Rect::new(-0.0, 0.0, 1.0, 1.0) == Rect::new(0.0, 0.0, 1.0, 1.0)
        );
        assert!(Rect::new(1.0, 1.0, 0.0, 0.0) != Rect::new(2.0, 2.0, 0.0, 0.0));

        let null = Rect::NULL;
        assert!(
            null.is_null()
                && null.is_empty()
                && A.contains(null)
                && !null.contains(A)
                && null.contains(null)
        );
        assert!(!A.intersects(null) && !null.intersects(null) && null.intersection(A).is_null());
        assert!(null == Rect::new(f64::INFINITY, f64::INFINITY, 5.0, 5.0));
        assert!(Rect::new(0.0, f64::INFINITY, 5.0, 5.0).is_null());

        let nan = Rect::new(f64::NAN, 0.0, 10.0, 10.0);
        assert!(nan != nan && !A.contains(nan) && !nan.is_null());
        assert!(A.offset_by(20.0, 0.0) == Rect::new(20.0, 0.0, 10.0, 10.0));
    }

    #[test]
    fn float_order_like_swift() {
        assert_eq!(cmp_f64(-0.0, 0.0), Ordering::Equal);
        assert_eq!(cmp_f64(1.0, 2.0), Ordering::Less);
        assert_eq!(cmp_f64(f64::NAN, 1.0), Ordering::Greater);
    }
}
