//! Geometry helpers ported verbatim from C# `Library/GeometryHelper.cs`.
//!
//! - [`lines_intersect`] — orientation-based segment intersection (incl. the
//!   collinear `IsBetween` fallbacks).
//! - [`convex_hull`] — monotone chain over the (x, y) plane. Like the C#, a
//!   single input point yields an *empty* hull, and output `z` is always 0.
//! - [`pythagoras`] / [`pythagoras_coords`] — the two C# overloads.
//! - [`centroid`] — mean x/y. Note: the C# `Average()` throws on empty input;
//!   here an empty slice yields a `NaN` coord instead of panicking.

use crate::model::Coord;

/// Orientation of ordered triplet (a, b, c): 0 = collinear, 1 = clockwise,
/// -1 = counter-clockwise. Mirrors C# `GetOrientation` (`Math.Sign`).
fn orientation(a: Coord, b: Coord, c: Coord) -> i32 {
    let value = ((b.y - a.y) * (c.x - b.x)) - ((b.x - a.x) * (c.y - b.y));
    if value > 0.0 {
        1
    } else if value < 0.0 {
        -1
    } else {
        0
    }
}

/// Whether `test` lies within the bounding box of `a`..`b`.
/// Assumes collinearity (see [`orientation`]). Mirrors C# `IsBetween`.
fn is_between(a: Coord, b: Coord, test: Coord) -> bool {
    test.x >= a.x.min(b.x)
        && test.x <= a.x.max(b.x)
        && test.y >= a.y.min(b.y)
        && test.y <= a.y.max(b.y)
}

/// Whether the segments a1–a2 and b1–b2 intersect. Mirrors C#
/// `GeometryHelper.LinesIntersect` verbatim.
pub fn lines_intersect(a1: Coord, a2: Coord, b1: Coord, b2: Coord) -> bool {
    let orient_a = orientation(a1, a2, b1);
    let orient_b = orientation(a1, a2, b2);
    let orient_c = orientation(b1, b2, a1);
    let orient_d = orientation(b1, b2, a2);

    // Lines intersect.
    if orient_a != orient_b && orient_c != orient_d {
        return true;
    }

    // Lines are collinear; return whether they touch.
    (orient_a == 0 && is_between(a1, b1, a2))
        || (orient_b == 0 && is_between(a1, b2, a2))
        || (orient_c == 0 && is_between(b1, a1, b2))
        || (orient_d == 0 && is_between(b1, a2, b2))
}

/// Cross product of vectors o→a and o→b. Mirrors C# `VectorCrossProduct`.
fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - o.0) * (b.1 - o.1)) - ((a.1 - o.1) * (b.0 - o.0))
}

/// Points defining the convex hull of all given points, via monotone chain
/// (https://en.wikibooks.org/wiki/Algorithm_Implementation/Geometry/Convex_hull/Monotone_chain).
/// Operates on (x, y); output `z` is 0. Mirrors the C# edge case: fewer than
/// 2 input points yields an empty vec.
pub fn convex_hull(points: &[Coord]) -> Vec<Coord> {
    if points.len() <= 1 {
        return Vec::new();
    }

    let mut verts: Vec<(f64, f64)> = points.iter().map(|p| (p.x, p.y)).collect();
    verts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));

    let n = verts.len();
    let mut hull = vec![(0.0, 0.0); 2 * n];
    let mut k = 0usize;

    // Build lower hull.
    for vert in verts.iter().take(n) {
        while k >= 2 && cross(hull[k - 2], hull[k - 1], *vert) <= 0.0 {
            k -= 1;
        }
        hull[k] = *vert;
        k += 1;
    }

    // Build upper hull.
    let t = k + 1;
    for vert in verts.iter().take(n - 1).rev() {
        while k >= t && cross(hull[k - 2], hull[k - 1], *vert) <= 0.0 {
            k -= 1;
        }
        hull[k] = *vert;
        k += 1;
    }

    hull.iter()
        .take(k - 1)
        .map(|&(x, y)| Coord::new(x, y))
        .collect()
}

/// Mirrors C# `GeometryHelper.Pythagoras(double, double)`.
pub fn pythagoras(a: f64, b: f64) -> f64 {
    ((a * a) + (b * b)).sqrt()
}

/// Mirrors C# `GeometryHelper.Pythagoras(Coord, Coord)` (x/y plane only).
pub fn pythagoras_coords(a: Coord, b: Coord) -> f64 {
    pythagoras(a.x - b.x, a.y - b.y)
}

/// Mean x/y of the coords. Mirrors C# `GetCentroid(IEnumerable<Coord>)`.
pub fn centroid(coords: &[Coord]) -> Coord {
    let n = coords.len() as f64;
    Coord::new(
        coords.iter().map(|c| c.x).sum::<f64>() / n,
        coords.iter().map(|c| c.y).sum::<f64>() / n,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_and_intersection() {
        use crate::model::Coord;
        let a = Coord {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let b = Coord {
            x: 2.0,
            y: 2.0,
            z: 0.0,
        };
        let c = Coord {
            x: 0.0,
            y: 2.0,
            z: 0.0,
        };
        let d = Coord {
            x: 2.0,
            y: 0.0,
            z: 0.0,
        };
        assert!(lines_intersect(a, b, c, d));
        assert!(!lines_intersect(a, c, b, d));
        let pts = vec![
            a,
            b,
            c,
            d,
            Coord {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        assert_eq!(convex_hull(&pts).len(), 4);
    }

    #[test]
    fn centroid_and_pythagoras() {
        let pts = vec![Coord::new(0.0, 0.0), Coord::new(4.0, 4.0)];
        let center = centroid(&pts);
        assert_eq!((center.x, center.y), (2.0, 2.0));
        assert_eq!(pythagoras(3.0, 4.0), 5.0);
        assert_eq!(
            pythagoras_coords(Coord::new(0.0, 0.0), Coord::new(3.0, 4.0)),
            5.0
        );
        // Collinear overlap still counts as intersecting (IsBetween fallback).
        assert!(lines_intersect(
            Coord::new(0.0, 0.0),
            Coord::new(4.0, 0.0),
            Coord::new(2.0, 0.0),
            Coord::new(6.0, 0.0),
        ));
        // Single point → empty hull, mirroring the C# `Count == 1` early-out.
        assert!(convex_hull(&[Coord::new(1.0, 1.0)]).is_empty());
    }
}
