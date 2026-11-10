//! fig2dev's spline conversion (`trans_spline.c`): X-splines (Blanc and
//! Schlick, "X-splines: a spline model designed for the end user",
//! SIGGRAPH '95) as polylines. fig2dev's reader turns every Fig 3.2
//! spline into a line this way; some drivers convert their own shapes.
//
// Ported from fig2dev trans_spline.c and trans_spline.h (xfig licence, see
// LICENSE-xfig).

use crate::object::{
    Compound, Control, Line, Point, Spline, S_SPLINE_ANGULAR, S_SPLINE_APPROX, S_SPLINE_INTERP, T_POLYGON, T_POLYLINE,
};
use crate::Ctx;

/// At most this many points per converted spline (`MAXNUMPTS`).
pub const MAXNUMPTS: usize = 25000;
/// The precision the reader converts splines with.
pub const HIGH_PRECISION: f32 = 0.5;
/// Points dropped at an end with an arrowhead, so that the arrowhead
/// gets a good direction (`ARROW_START`).
pub const ARROW_START: usize = 4;
const MAX_SPLINE_STEP: f64 = 0.2;

/// The point array the curve is collected in (C's static `points`).
struct PointArray {
    pts: Vec<Point>,
    /// A point was refused because the array was full.
    overflow: bool,
}

impl PointArray {
    fn new() -> PointArray {
        PointArray {
            pts: Vec::with_capacity(300),
            overflow: false,
        }
    }

    /// `add_point`: identical successive points are dropped.
    fn add(&mut self, x: i32, y: i32) {
        if self.pts.last() == Some(&Point::new(x, y)) {
            return;
        }
        if self.pts.len() >= MAXNUMPTS {
            self.overflow = true;
            return;
        }
        self.pts.push(Point::new(x, y));
    }

    /// Hand over the points, reporting an overflow once (fig2dev prints the
    /// message for every point it refuses).
    fn finish(self, ctx: &mut Ctx) -> Vec<Point> {
        if self.overflow {
            ctx.warn(format!(
                "Too many points, recompile with MAXNUMPTS > {MAXNUMPTS} in trans_spline.h"
            ));
        }
        self.pts
    }
}

/// The control points of one segment, and their shape factors.
#[derive(Clone, Copy)]
struct Segment {
    p: [Point; 4],
    s1: f64,
    s2: f64,
}

/// The shape factors of `s`: its own if there is one per point, otherwise
/// those `make_control_factors` would give it.
fn shape_factors(s: &Spline) -> Vec<f64> {
    if s.controls.len() == s.points.len() {
        s.controls.iter().map(|c| c.s).collect()
    } else {
        control_factors(s).iter().map(|c| c.s).collect()
    }
}

fn segment(s: &Spline, factors: &[f64], idx: [usize; 4]) -> Option<Segment> {
    let pt = |i: usize| s.points.get(i).copied();
    Some(Segment {
        p: [pt(idx[0])?, pt(idx[1])?, pt(idx[2])?, pt(idx[3])?],
        s1: *factors.get(idx[1])?,
        s2: *factors.get(idx[2])?,
    })
}

/// `compute_open_spline`: the points of an open X-spline. `None` for a
/// spline with fewer than two points.
pub fn compute_open_spline(ctx: &mut Ctx, s: &Spline, precision: f32) -> Option<Vec<Point>> {
    let n = s.points.len();
    let mut arr = PointArray::new();
    if n < 2 {
        return None;
    }
    // a two-point spline is a straight line
    if n == 2 {
        for q in &s.points {
            arr.add(q.x, q.y);
        }
        return Some(arr.finish(ctx));
    }
    let factors = shape_factors(s);
    // The first control point counts twice for the first segment, the
    // last twice for the last segment.
    let last = n - 1;
    let mut k = 0;
    for kk in 0..=n - 3 {
        k = kk;
        let idx = [kk.saturating_sub(1), kk, kk + 1, kk + 2];
        let seg = segment(s, &factors, idx)?;
        spline_segment(&mut arr, kk as i32, &seg, precision);
        if arr.overflow {
            break;
        }
    }
    let seg = segment(s, &factors, [last - 2, last - 1, last, last])?;
    if !arr.overflow {
        spline_segment(&mut arr, k as i32, &seg, precision);
    }
    let end = seg.p[3];
    arr.add(end.x, end.y);
    Some(arr.finish(ctx))
}

/// `compute_closed_spline`: the points of a closed X-spline, the first
/// point repeated at the end. `None` (with fig2dev's message) for a
/// spline with fewer than three points.
pub fn compute_closed_spline(ctx: &mut Ctx, s: &Spline, precision: f32) -> Option<Vec<Point>> {
    let n = s.points.len();
    if n < 3 {
        // C writes this without a newline: the reader's next message continues the line.
        ctx.warn_partial("A closed spline with less than three points.");
        return None;
    }
    let factors = shape_factors(s);
    let mut arr = PointArray::new();
    // one segment per point, joining the end to the beginning
    for k in 0..n {
        let seg = segment(s, &factors, [k, (k + 1) % n, (k + 2) % n, (k + 3) % n])?;
        spline_segment(&mut arr, k as i32, &seg, precision);
        if arr.overflow {
            break;
        }
    }
    if let Some(&first) = arr.pts.first() {
        arr.add(first.x, first.y);
    }
    Some(arr.finish(ctx))
}

fn f_blend(numerator: f64, denominator: f64) -> f64 {
    let p = 2.0 * denominator * denominator;
    let u = numerator / denominator;
    let u2 = u * u;
    // clang fuses the three terms (ffp-contract=on in the reference)
    u * u2 * (6.0 - p).mul_add(u2, (2.0 * p - 15.0).mul_add(u, 10.0 - p))
}

/// p equals 2.
fn g_blend(u: f64, q: f64) -> f64 {
    u * u.mul_add(
        u.mul_add(
            u.mul_add(
                u.mul_add((-5.0f64).mul_add(q, 4.0), 14.0f64.mul_add(q, -11.0)),
                (-12.0f64).mul_add(q, 8.0),
            ),
            2.0 * q,
        ),
        q,
    )
}

fn h_blend(u: f64, q: f64) -> f64 {
    let u2 = u * u;
    u * u.mul_add(u2.mul_add((-2.0f64).mul_add(q, -(u * q)), 2.0 * q), q)
}

/// `(A0, A2)` for a negative `s1`.
fn negative_s1_influence(t: f64, s1: f64) -> (f64, f64) {
    (h_blend(-t, -s1), g_blend(t, -s1))
}

/// `(A1, A3)` for a negative `s2`.
fn negative_s2_influence(t: f64, s2: f64) -> (f64, f64) {
    (g_blend(1.0 - t, -s2), h_blend(t - 1.0, -s2))
}

/// `(A0, A2)` for a positive `s1`.
fn positive_s1_influence(k: i32, t: f64, s1: f64) -> (f64, f64) {
    let k = k as f64;
    let tk = k + 1.0 + s1;
    let a0 = if t + k + 1.0 < tk {
        f_blend(t + k + 1.0 - tk, k - tk)
    } else {
        0.0
    };
    let tk = k + 1.0 - s1;
    let a2 = f_blend(t + k + 1.0 - tk, k + 2.0 - tk);
    (a0, a2)
}

/// `(A1, A3)` for a positive `s2`.
fn positive_s2_influence(k: i32, t: f64, s2: f64) -> (f64, f64) {
    let k = k as f64;
    let tk = k + 2.0 + s2;
    let a1 = f_blend(t + k + 1.0 - tk, k + 1.0 - tk);
    let tk = k + 2.0 - s2;
    let a3 = if t + k + 1.0 > tk {
        f_blend(t + k + 1.0 - tk, k + 3.0 - tk)
    } else {
        0.0
    };
    (a1, a3)
}

/// The blending weights `[A0, A1, A2, A3]` at `t`, by the signs of the
/// shape factors, as `spline_segment_computing` picks them.
fn blend(k: i32, t: f64, s1: f64, s2: f64) -> [f64; 4] {
    let ((a0, a2), (a1, a3)) = if s1 < 0.0 {
        if s2 < 0.0 {
            (negative_s1_influence(t, s1), negative_s2_influence(t, s2))
        } else {
            (negative_s1_influence(t, s1), positive_s2_influence(k, t, s2))
        }
    } else if s2 < 0.0 {
        (positive_s1_influence(k, t, s1), negative_s2_influence(t, s2))
    } else {
        (positive_s1_influence(k, t, s1), positive_s2_influence(k, t, s2))
    };
    [a0, a1, a2, a3]
}

/// `point_computing`: the weighted mean of the four control points.
fn point_computing(a: &[f64; 4], p: &[Point; 4]) -> (i32, i32) {
    let (x, y) = point_computing_f(a, p);
    (crate::round(x), crate::round(y))
}

/// `point_computing` before the rounding.
fn point_computing_f(a: &[f64; 4], p: &[Point; 4]) -> (f64, f64) {
    let w = a[0] + a[1] + a[2] + a[3];
    let num = |f: fn(&Point) -> i32| {
        a[3].mul_add(
            f(&p[3]) as f64,
            a[2].mul_add(f(&p[2]) as f64, a[0].mul_add(f(&p[0]) as f64, a[1] * f(&p[1]) as f64)),
        )
    };
    (num(|q| q.x) / w, num(|q| q.y) / w)
}

/// `step_computing`: the parameter step for the segment from `p1` to
/// `p2`, finer for long and for strongly curved segments.
fn step_computing(k: i32, seg: &Segment, precision: f32) -> f32 {
    let Segment { p, s1, s2 } = *seg;
    // origin
    let (xstart, ystart) = if s1 > 0.0 {
        let a = if s2 < 0.0 {
            let ((a0, a2), (a1, a3)) = (positive_s1_influence(k, 0.0, s1), negative_s2_influence(0.0, s2));
            [a0, a1, a2, a3]
        } else {
            let ((a0, a2), (a1, a3)) = (positive_s1_influence(k, 0.0, s1), positive_s2_influence(k, 0.0, s2));
            [a0, a1, a2, a3]
        };
        point_computing(&a, &p)
    } else {
        (p[1].x, p[1].y)
    };
    // extremity
    let (xend, yend) = if s2 > 0.0 {
        let a = if s1 < 0.0 {
            let ((a0, a2), (a1, a3)) = (negative_s1_influence(1.0, s1), positive_s2_influence(k, 1.0, s2));
            [a0, a1, a2, a3]
        } else {
            let ((a0, a2), (a1, a3)) = (positive_s1_influence(k, 1.0, s1), positive_s2_influence(k, 1.0, s2));
            [a0, a1, a2, a3]
        };
        point_computing(&a, &p)
    } else {
        (p[2].x, p[2].y)
    };
    // middle
    let ((a0, a2), (a1, a3)) = if s2 > 0.0 {
        if s1 < 0.0 {
            (negative_s1_influence(0.5, s1), positive_s2_influence(k, 0.5, s2))
        } else {
            (positive_s1_influence(k, 0.5, s1), positive_s2_influence(k, 0.5, s2))
        }
    } else if s1 < 0.0 {
        (negative_s1_influence(0.5, s1), negative_s2_influence(0.5, s2))
    } else {
        (positive_s1_influence(k, 0.5, s1), negative_s2_influence(0.5, s2))
    };
    let (xmid, ymid) = point_computing(&[a0, a1, a2, a3], &p);

    // fig2dev computes the angle in float
    let xv1 = (xstart as i64 - xmid as i64) as f32;
    let yv1 = (ystart as i64 - ymid as i64) as f32;
    let xv2 = (xend as i64 - xmid as i64) as f32;
    let yv2 = (yend as i64 - ymid as i64) as f32;
    let scal_prod = xv1.mul_add(xv2, yv1 * yv2);
    let sides_length_prod = ((xv1.mul_add(xv1, yv1 * yv1) * xv2.mul_add(xv2, yv2 * yv2)) as f64).sqrt() as f32;
    // the cosine of the origin-middle-extremity angle approximates the
    // curvature of the segment
    let angle_cos = if sides_length_prod == 0.0 {
        0.0
    } else {
        scal_prod / sides_length_prod
    };
    let xlength = (xend as i64 - xstart as i64) as f64;
    let ylength = (yend as i64 - ystart as i64) as f64;
    let start_to_end_dist = (xlength * xlength + ylength * ylength).sqrt() as i32;
    // more steps if the segment's ends are far apart
    let mut number_of_steps = ((start_to_end_dist as f64).sqrt() as i32) / 2;
    // more steps if the curve is high
    number_of_steps = number_of_steps.saturating_add(((1.0 + angle_cos as f64) * 10.0) as i32);
    let mut step = if number_of_steps == 0 || number_of_steps > 999 {
        1.0
    } else {
        precision / number_of_steps as f32
    };
    if step as f64 > MAX_SPLINE_STEP || step == 0.0 || !step.is_finite() {
        step = MAX_SPLINE_STEP as f32;
    }
    step
}

/// `SPLINE_SEGMENT_LOOP`: the points of one segment, `t` from 0 to 1
/// (exclusive).
fn spline_segment(arr: &mut PointArray, k: i32, seg: &Segment, precision: f32) {
    let step = step_computing(k, seg, precision) as f64;
    let mut t = 0.0;
    while t < 1.0 {
        let a = blend(k, t, seg.s1, seg.s2);
        let (x, y) = point_computing(&a, &seg.p);
        arr.add(x, y);
        if arr.overflow {
            return;
        }
        t += step;
    }
}

/// `create_line_with_spline`: the spline as a polyline (open) or polygon
/// (closed) with its attributes and comments. At an end with an arrowhead,
/// the three points before the end are dropped, so that the arrowhead
/// points along the curve. `None` if the spline can't be converted (too
/// few points; fig2dev's message goes to `ctx`).
pub fn create_line_with_spline(ctx: &mut Ctx, s: &Spline) -> Option<Line> {
    let mut points = if s.open_spline() {
        compute_open_spline(ctx, s, HIGH_PRECISION)?
    } else {
        compute_closed_spline(ctx, s, HIGH_PRECISION)?
    };
    let mut start = 0;
    let mut npoints = points.len();
    if s.for_arrow.is_some() && npoints > ARROW_START {
        let last = *points.get(npoints - 1)?;
        *points.get_mut(npoints - ARROW_START)? = last;
        npoints -= ARROW_START - 1;
    }
    if s.back_arrow.is_some() && npoints > ARROW_START {
        let first = *points.first()?;
        *points.get_mut(ARROW_START - 1)? = first;
        npoints -= ARROW_START - 1;
        start = ARROW_START - 1;
    }
    let points = points.get(start..start + npoints)?.to_vec();
    Some(Line {
        type_: if s.open_spline() { T_POLYLINE } else { T_POLYGON },
        style: s.style,
        thickness: s.thickness,
        pen_color: s.pen_color,
        fill_color: s.fill_color,
        depth: s.depth,
        pen: s.pen,
        fill_style: s.fill_style,
        style_val: s.style_val,
        for_arrow: s.for_arrow,
        back_arrow: s.back_arrow,
        cap_style: s.cap_style,
        // prevents spikes in tight splines
        join_style: 2,
        radius: 0,
        points,
        pic: None,
        comments: s.comments.clone(),
    })
}

/// One cubic Bézier segment of a spline path: the control points and the
/// end point (the start is the previous segment's end).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BezierSegment {
    pub c1: (f64, f64),
    pub c2: (f64, f64),
    pub end: (f64, f64),
    /// The segment is a straight line (both controls on the chord).
    pub straight: bool,
}

/// A spline as a path of cubic Béziers.
#[derive(Clone, Debug, PartialEq)]
pub struct BezierPath {
    pub start: (f64, f64),
    pub segments: Vec<BezierSegment>,
    /// A closed spline: the last segment returns to `start`.
    pub closed: bool,
    /// Segments that were split in two or more pieces to stay within the
    /// tolerance.
    pub splits: usize,
}

/// How far a Bézier path may stray from the X-spline: `max(abs, rel *
/// chord)` for a segment whose chord is that long.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BezierTolerance {
    pub abs: f64,
    pub rel: f64,
}

/// Samples per segment and per Bézier polyline when measuring the error.
const FIT_SAMPLES: usize = 16;
const FIT_DENSE: usize = 128;
/// The deepest halving of a segment's parameter interval.
const MAX_SPLIT_DEPTH: u32 = 6;

/// The X-spline curve of one segment at parameter `t` (not rounded).
fn curve_at(k: i32, seg: &Segment, t: f64) -> (f64, f64) {
    point_computing_f(&blend(k, t, seg.s1, seg.s2), &seg.p)
}

/// The derivative of the curve at `t` by differences (one-sided at the
/// ends, where the blending functions are defined only inside `[0, 1]`).
fn curve_slope(k: i32, seg: &Segment, t: f64) -> (f64, f64) {
    const H: f64 = 1e-4;
    let at = |t: f64| curve_at(k, seg, t);
    if t < H {
        let (a, b, c) = (at(t), at(t + H), at(t + 2.0 * H));
        (
            (-3.0 * a.0 + 4.0 * b.0 - c.0) / (2.0 * H),
            (-3.0 * a.1 + 4.0 * b.1 - c.1) / (2.0 * H),
        )
    } else if t > 1.0 - H {
        let (a, b, c) = (at(t), at(t - H), at(t - 2.0 * H));
        (
            (3.0 * a.0 - 4.0 * b.0 + c.0) / (2.0 * H),
            (3.0 * a.1 - 4.0 * b.1 + c.1) / (2.0 * H),
        )
    } else {
        let (a, b) = (at(t - H), at(t + H));
        ((b.0 - a.0) / (2.0 * H), (b.1 - a.1) / (2.0 * H))
    }
}

fn dist_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let u = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - u * dx).hypot(p.1 - a.1 - u * dy)
}

/// The largest distance of a point of `pts` from the polyline `poly`.
fn max_dist(pts: &[(f64, f64)], poly: &[(f64, f64)]) -> f64 {
    pts.iter()
        .map(|&p| {
            poly.windows(2)
                .map(|w| dist_to_segment(p, w[0], w[1]))
                .fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max)
}

fn bezier_at(p0: (f64, f64), c1: (f64, f64), c2: (f64, f64), p1: (f64, f64), u: f64) -> (f64, f64) {
    let v = 1.0 - u;
    let (a, b, c, d) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
    (
        a * p0.0 + b * c1.0 + c * c2.0 + d * p1.0,
        a * p0.1 + b * c1.1 + c * c2.1 + d * p1.1,
    )
}

/// The Hausdorff-style distance between the cubic and the curve between
/// `t0` and `t1`.
fn fit_error(k: i32, seg: &Segment, t0: f64, t1: f64, b: [(f64, f64); 4]) -> f64 {
    let u = |i: usize| i as f64 / FIT_DENSE as f64;
    let curve: Vec<(f64, f64)> = (0..=FIT_DENSE)
        .map(|i| curve_at(k, seg, t0 + (t1 - t0) * u(i)))
        .collect();
    let cubic: Vec<(f64, f64)> = (0..=FIT_DENSE)
        .map(|i| bezier_at(b[0], b[1], b[2], b[3], u(i)))
        .collect();
    let every = FIT_DENSE / FIT_SAMPLES;
    let curve_samples: Vec<_> = curve.iter().step_by(every).copied().collect();
    let cubic_samples: Vec<_> = cubic.iter().step_by(every).copied().collect();
    max_dist(&curve_samples, &cubic).max(max_dist(&cubic_samples, &curve))
}

/// The unit tangent of the curve at `t0` (`forward`) or `t1`: from the
/// derivative, or, where that vanishes (a corner), from the curve nearby.
fn unit_tangent(k: i32, seg: &Segment, t: f64, forward: bool, at: (f64, f64)) -> Option<(f64, f64)> {
    let d = curve_slope(k, seg, t);
    let len = d.0.hypot(d.1);
    if len > 1e-3 {
        return Some((d.0 / len, d.1 / len));
    }
    let near = curve_at(k, seg, if forward { t + 0.05 } else { t - 0.05 });
    let (dx, dy) = (near.0 - at.0, near.1 - at.1);
    let len = dx.hypot(dy);
    (len > 1e-9).then(|| (dx / len, dy / len))
}

/// Control points on the curve's end tangents at the distances that fit
/// the curve best in the least-squares sense (Schneider, "An algorithm for
/// automatically fitting digitized curves", Graphics Gems, 1990), with the
/// samples placed by chord length. `None` if the system is singular or a
/// distance comes out non-positive.
fn fit_tangent_lengths(
    k: i32,
    seg: &Segment,
    (t0, t1): (f64, f64),
    (p0, p1): ((f64, f64), (f64, f64)),
) -> Option<[(f64, f64); 2]> {
    const N: usize = 24;
    let pts: Vec<(f64, f64)> = (0..=N)
        .map(|i| curve_at(k, seg, t0 + (t1 - t0) * i as f64 / N as f64))
        .collect();
    let mut u = vec![0.0; pts.len()];
    for i in 1..pts.len() {
        let (a, b) = (*pts.get(i - 1)?, *pts.get(i)?);
        *u.get_mut(i)? = *u.get(i - 1)? + (b.0 - a.0).hypot(b.1 - a.1);
    }
    let total = *u.last()?;
    if total < 1e-9 {
        return None;
    }
    let e0 = unit_tangent(k, seg, t0, true, p0)?;
    let e1 = unit_tangent(k, seg, t1, false, p1)?;
    // c1 = p0 + a1 * e0, c2 = p1 - a2 * e1
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (q, ui) in pts.iter().zip(&u) {
        let ui = ui / total;
        let v = 1.0 - ui;
        let (b0, b1, b2, b3) = (v * v * v, 3.0 * v * v * ui, 3.0 * v * ui * ui, ui * ui * ui);
        let a1 = (e0.0 * b1, e0.1 * b1);
        let a2 = (-e1.0 * b2, -e1.1 * b2);
        c00 += a1.0 * a1.0 + a1.1 * a1.1;
        c01 += a1.0 * a2.0 + a1.1 * a2.1;
        c11 += a2.0 * a2.0 + a2.1 * a2.1;
        let r = (
            q.0 - (p0.0 * (b0 + b1) + p1.0 * (b2 + b3)),
            q.1 - (p0.1 * (b0 + b1) + p1.1 * (b2 + b3)),
        );
        x0 += a1.0 * r.0 + a1.1 * r.1;
        x1 += a2.0 * r.0 + a2.1 * r.1;
    }
    let det = c00 * c11 - c01 * c01;
    if det.abs() < 1e-12 * (c00 * c11).abs().max(1e-30) {
        return None;
    }
    let alpha1 = (x0 * c11 - x1 * c01) / det;
    let alpha2 = (c00 * x1 - c01 * x0) / det;
    let eps = 1e-3 * total;
    if !(alpha1.is_finite() && alpha2.is_finite()) || alpha1 < eps || alpha2 < eps {
        return None;
    }
    Some([
        (p0.0 + alpha1 * e0.0, p0.1 + alpha1 * e0.1),
        (p1.0 - alpha2 * e1.0, p1.1 - alpha2 * e1.1),
    ])
}

/// Fits cubics to the parameter interval `t0..t1` of one segment (from `p0`
/// to `p1`), halving it while the error exceeds `tol`; `None` if the curve
/// is not finite.
#[allow(clippy::too_many_arguments)]
fn fit_segment(
    k: i32,
    seg: &Segment,
    (t0, t1): (f64, f64),
    (p0, p1): ((f64, f64), (f64, f64)),
    tol: BezierTolerance,
    depth: u32,
    out: &mut Vec<BezierSegment>,
) -> Option<()> {
    let hermite = {
        let (d0, d1) = (curve_slope(k, seg, t0), curve_slope(k, seg, t1));
        let h = (t1 - t0) / 3.0;
        [(p0.0 + d0.0 * h, p0.1 + d0.1 * h), (p1.0 - d1.0 * h, p1.1 - d1.1 * h)]
    };
    let mut best = hermite;
    let mut best_err = None;
    if hermite.iter().any(|c| !c.0.is_finite() || !c.1.is_finite()) {
        return None;
    }
    // The X-spline's blending is not a cubic polynomial in `t`: the
    // tangent lengths that fit best are found by least squares.
    let fitted = fit_tangent_lengths(k, seg, (t0, t1), (p0, p1));
    for cand in std::iter::once(hermite).chain(fitted) {
        let e = fit_error(k, seg, t0, t1, [p0, cand[0], cand[1], p1]);
        if best_err.is_none_or(|b| e < b) {
            best = cand;
            best_err = Some(e);
        }
    }
    let [c1, c2] = best;
    let chord = (p1.0 - p0.0).hypot(p1.1 - p0.1);
    let allowed = tol.abs.max(tol.rel * chord);
    if depth >= MAX_SPLIT_DEPTH || best_err.is_some_and(|e| e <= allowed) {
        // Straight: both controls within a quarter unit of the chord and
        // between its ends.
        let straight = if chord < 1e-9 {
            c1 == p0 && c2 == p0
        } else {
            let (ux, uy) = ((p1.0 - p0.0) / chord, (p1.1 - p0.1) / chord);
            [c1, c2].iter().all(|c| {
                let (dx, dy) = (c.0 - p0.0, c.1 - p0.1);
                let along = dx * ux + dy * uy;
                (dx * uy - dy * ux).abs() <= 0.25 && along >= -0.25 && along <= chord + 0.25
            })
        };
        // A zero-length piece (coincident points) adds nothing.
        if straight && chord < 1e-9 {
            return Some(());
        }
        out.push(BezierSegment {
            c1,
            c2,
            end: p1,
            straight,
        });
        return Some(());
    }
    let tm = 0.5 * (t0 + t1);
    let pm = curve_at(k, seg, tm);
    fit_segment(k, seg, (t0, tm), (p0, pm), tol, depth + 1, out)?;
    fit_segment(k, seg, (tm, t1), (pm, p1), tol, depth + 1, out)
}

/// The segments of the X-spline `s` as `compute_open_spline` and
/// `compute_closed_spline` walk them, each with its `k`.
fn spline_segments(s: &Spline) -> Option<Vec<(i32, Segment)>> {
    let n = s.points.len();
    let factors = shape_factors(s);
    let mut v = Vec::new();
    if s.open_spline() {
        if n < 3 {
            return None;
        }
        for kk in 0..=n - 3 {
            v.push((
                kk as i32,
                segment(s, &factors, [kk.saturating_sub(1), kk, kk + 1, kk + 2])?,
            ));
        }
        v.push(((n - 3) as i32, segment(s, &factors, [n - 3, n - 2, n - 1, n - 1])?));
    } else {
        if n < 3 {
            return None;
        }
        for k in 0..n {
            v.push((
                k as i32,
                segment(s, &factors, [k, (k + 1) % n, (k + 2) % n, (k + 3) % n])?,
            ));
        }
    }
    Some(v)
}

/// The X-spline `s` as a path of cubic Béziers: one per segment between
/// two of its points (a segment is split in halves, up to six times, while
/// the cubic strays more than `tol` from the curve). The ends of a segment
/// are the curve's points there and the controls come from its tangents.
/// `None` for a spline with fewer than two points (three if closed) or
/// non-finite shape factors.
pub fn spline_to_bezier(s: &Spline, tol: BezierTolerance) -> Option<BezierPath> {
    if s.points.len() < 2 || s.controls.iter().any(|c| !c.s.is_finite()) {
        return None;
    }
    let closed = s.closed_spline();
    if s.points.len() == 2 && !closed {
        let (a, b) = (s.points.first()?, s.points.get(1)?);
        let (a, b) = ((f64::from(a.x), f64::from(a.y)), (f64::from(b.x), f64::from(b.y)));
        return Some(BezierPath {
            start: a,
            segments: vec![BezierSegment {
                c1: a,
                c2: b,
                end: b,
                straight: true,
            }],
            closed,
            splits: 0,
        });
    }
    let segs = spline_segments(s)?;
    let (k0, first) = segs.first()?;
    let start = curve_at(*k0, first, 0.0);
    let mut path = BezierPath {
        start,
        segments: Vec::new(),
        closed,
        splits: 0,
    };
    let mut at = start;
    for (k, seg) in &segs {
        let end = curve_at(*k, seg, 1.0);
        let before = path.segments.len();
        fit_segment(*k, seg, (0.0, 1.0), (at, end), tol, 0, &mut path.segments)?;
        if path.segments.len() > before + 1 {
            path.splits += 1;
        }
        at = end;
    }
    Some(path)
}

/// A figure built in memory as the drivers expect it: every spline of
/// `com` and of its compounds becomes a line (a polyline or polygon) as the
/// reader does for a Fig 3.2 file (`ctx.v32_flag`; otherwise only X-splines,
/// the others keep their controls), appended after the compound's lines. A spline that can't be converted
/// (coincident points, non-finite shape factors, too few points) is
/// dropped, with the reader's message in `ctx.messages`. With
/// `ctx.keep_splines` the valid ones stay splines.
pub fn splines_to_lines(ctx: &mut Ctx, com: &Compound) -> Compound {
    let mut out = Compound {
        compounds: com.compounds.iter().map(|c| splines_to_lines(ctx, c)).collect(),
        splines: Vec::new(),
        ..com.clone()
    };
    for s in &com.splines {
        if !(s.x_spline() || ctx.v32_flag) {
            out.splines.push(s.clone());
            continue;
        }
        let bad = s.points.iter().all(|p| Some(p) == s.points.first())
            || s.controls.len() != s.points.len()
            || s.controls.iter().any(|c| !c.s.is_finite());
        if bad {
            ctx.warn("Invalid spline with only co-incident points or shape factors, dropped.");
            continue;
        }
        match create_line_with_spline(ctx, s) {
            // A driver that draws splines itself (`Ctx::keep_splines`)
            // gets the spline; the conversion only checked it.
            Some(_) if ctx.keep_splines => out.splines.push(s.clone()),
            Some(l) => out.lines.push(l),
            None => ctx.warn("Unable to convert spline to line, dropped."),
        }
    }
    out
}

/// The controls `make_control_factors` gives a spline: the shape factor
/// of an approximated (1) or interpolated (−1) spline at every point, the
/// ends of an open spline angular (0).
fn control_factors(s: &Spline) -> Vec<Control> {
    let type_s = if s.approx_spline() {
        S_SPLINE_APPROX
    } else {
        S_SPLINE_INTERP
    };
    let first = if s.closed_spline() { type_s } else { S_SPLINE_ANGULAR };
    let n = s.points.len();
    (0..n)
        .map(|i| Control {
            s: if i == 0 || i + 1 == n { first } else { type_s },
            ..Control::default()
        })
        .collect()
}

/// `make_control_factors`: give `s` one control per point with the shape
/// factor of its type (the reader then overwrites them with the file's).
pub fn make_control_factors(s: &mut Spline) {
    s.controls = control_factors(s);
}

#[cfg(test)]
mod tests {
    #[test]
    fn splines_of_a_figure_in_memory_become_lines() {
        use crate::object::{Compound, Control, Point, Spline};
        let mut ctx = Ctx::for_figure(1200.0, false, &[(40, 1, 2, 3)]);
        assert_eq!((ctx.thick_scale, ctx.user_col_indx.clone()), (15.0, vec![40]));
        let s = Spline {
            type_: 4,
            points: vec![Point::new(0, 0), Point::new(600, 300), Point::new(1200, 0)],
            controls: vec![
                Control {
                    s: 0.0,
                    ..Control::default()
                },
                Control {
                    s: 1.0,
                    ..Control::default()
                },
                Control {
                    s: 0.0,
                    ..Control::default()
                },
            ],
            ..Spline::default()
        };
        let com = Compound {
            splines: vec![s],
            ..Compound::default()
        };
        let out = super::splines_to_lines(&mut ctx, &com);
        assert!(out.splines.is_empty() && out.lines.len() == 1 && out.lines[0].points.len() > 3);
    }

    use super::*;
    use crate::object::{Arrow, T_CLOSED_XSPLINE, T_OPEN_XSPLINE};

    fn spline(type_: i32, pts: &[(i32, i32)], s: &[f64]) -> Spline {
        Spline {
            type_,
            thickness: 15,
            depth: 50,
            points: pts.iter().map(|&(x, y)| Point::new(x, y)).collect(),
            controls: s
                .iter()
                .map(|&s| Control {
                    s,
                    ..Control::default()
                })
                .collect(),
            comments: vec!["c".into()],
            ..Spline::default()
        }
    }

    /// The Bézier path sampled, and the farthest sample from the X-spline's
    /// own polyline.
    fn bezier_deviation(s: &Spline, path: &BezierPath) -> f64 {
        let mut ctx = Ctx::default();
        let poly: Vec<(f64, f64)> = if s.open_spline() {
            compute_open_spline(&mut ctx, s, 0.05)
        } else {
            compute_closed_spline(&mut ctx, s, 0.05)
        }
        .unwrap()
        .iter()
        .map(|p| (f64::from(p.x), f64::from(p.y)))
        .collect();
        let mut at = path.start;
        let mut worst = 0.0f64;
        for g in &path.segments {
            let samples: Vec<_> = (0..=20)
                .map(|i| bezier_at(at, g.c1, g.c2, g.end, f64::from(i) / 20.0))
                .collect();
            worst = worst.max(max_dist(&samples, &poly));
            at = g.end;
        }
        worst
    }

    fn shapes() -> Vec<Spline> {
        let five = [(1200, 1200), (3600, 1200), (4200, 3000), (2400, 4200), (900, 3000)];
        let four = [(1200, 5200), (2400, 6400), (3600, 5200), (4800, 6400)];
        let six = [
            (5200, 5200),
            (6000, 6400),
            (6800, 5200),
            (7600, 6400),
            (8000, 5200),
            (8800, 6400),
        ];
        vec![
            spline(3, &five, &[-1.0; 5]),
            spline(1, &five, &[1.0; 5]),
            spline(2, &four, &[0.0, -1.0, -1.0, 0.0]),
            spline(0, &four, &[0.0, 1.0, 1.0, 0.0]),
            spline(T_OPEN_XSPLINE, &six, &[0.0, 1.0, 0.0, -1.0, 1.0, 0.0]),
            spline(T_CLOSED_XSPLINE, &six, &[0.5, 1.0, 0.0, -1.0, 0.3, -0.5]),
        ]
    }

    #[test]
    fn bezier_paths_follow_the_xspline() {
        let exact = BezierTolerance { abs: 0.5, rel: 0.0 };
        let loose = BezierTolerance { abs: 6.0, rel: 0.02 };
        let mut splits = 0;
        for (i, s) in shapes().into_iter().enumerate() {
            let n = s.points.len();
            let segments = if s.closed_spline() { n } else { n - 1 };
            let tight = spline_to_bezier(&s, exact).unwrap();
            let dev = bezier_deviation(&s, &tight);
            assert!(dev <= 1.0, "type {} exact deviation {dev}", s.type_);
            let rough = spline_to_bezier(&s, loose).unwrap();
            let dev = bezier_deviation(&s, &rough);
            let longest = s
                .points
                .windows(2)
                .map(|w| f64::from(w[1].x - w[0].x).hypot(f64::from(w[1].y - w[0].y)))
                .fold(0.0, f64::max);
            let allowed = 6.0f64.max(0.02 * 1.5 * longest) + 1.0;
            assert!(dev <= allowed, "type {} loose deviation {dev} > {allowed}", s.type_);
            eprintln!(
                "type {}: {} segments; exact {} (deviation {:.2}), loose {} (deviation {dev:.2})",
                s.type_,
                segments,
                tight.segments.len(),
                bezier_deviation(&s, &tight),
                rough.segments.len()
            );
            assert_eq!(rough.closed, s.closed_spline());
            splits += rough.splits;
            // The closed approximated and interpolated splines never split.
            if i < 2 {
                assert_eq!(rough.splits, 0, "type {}", s.type_);
            }
            // Nodes are about the Fig points: at most 3 pieces per segment.
            assert!(rough.segments.len() <= 3 * segments);
        }
        // Only segments next to a corner (shape factor 0) may split.
        assert!(splits <= 4, "the default split {splits} segments");
    }

    #[test]
    fn interpolated_bezier_nodes_are_the_fig_points() {
        let s = &shapes()[0];
        let path = spline_to_bezier(s, BezierTolerance { abs: 6.0, rel: 0.02 }).unwrap();
        let mut nodes = vec![path.start];
        nodes.extend(path.segments.iter().map(|g| g.end));
        for p in &s.points {
            let q = (f64::from(p.x), f64::from(p.y));
            assert!(
                nodes.iter().any(|n| (n.0 - q.0).hypot(n.1 - q.1) < 0.01),
                "{q:?} in {nodes:?}"
            );
        }
    }

    #[test]
    fn bezier_of_a_two_point_spline_is_a_line_and_of_a_bad_one_none() {
        let tol = BezierTolerance { abs: 6.0, rel: 0.02 };
        let two = spline(T_OPEN_XSPLINE, &[(0, 0), (100, 50)], &[0.0, 0.0]);
        let p = spline_to_bezier(&two, tol).unwrap();
        assert_eq!((p.segments.len(), p.segments[0].straight), (1, true));
        assert!(spline_to_bezier(&spline(T_CLOSED_XSPLINE, &[(0, 0), (100, 50)], &[0.0, 0.0]), tol).is_none());
        let nan = spline(T_OPEN_XSPLINE, &[(0, 0), (1, 5), (9, 9)], &[0.0, f64::NAN, 0.0]);
        assert!(spline_to_bezier(&nan, tol).is_none());
    }

    #[test]
    fn control_factors_by_type() {
        let mut s = spline(T_OPEN_XSPLINE, &[(0, 0), (1, 1), (2, 2), (3, 3)], &[]);
        s.type_ = 0; // open approximated
        make_control_factors(&mut s);
        let f: Vec<f64> = s.controls.iter().map(|c| c.s).collect();
        assert_eq!(f, vec![0.0, 1.0, 1.0, 0.0]);
        s.type_ = 3; // closed interpolated
        make_control_factors(&mut s);
        let f: Vec<f64> = s.controls.iter().map(|c| c.s).collect();
        assert_eq!(f, vec![-1.0; 4]);
        let mut e = Spline::default();
        make_control_factors(&mut e);
        assert!(e.controls.is_empty());
    }

    #[test]
    fn two_points_are_a_straight_line() {
        let mut ctx = Ctx::default();
        let s = spline(T_OPEN_XSPLINE, &[(0, 0), (1200, 600)], &[0.0, 0.0]);
        let l = create_line_with_spline(&mut ctx, &s).unwrap();
        assert_eq!(l.points, vec![Point::new(0, 0), Point::new(1200, 600)]);
        assert_eq!((l.type_, l.join_style, l.thickness, l.depth), (T_POLYLINE, 2, 15, 50));
        assert_eq!(l.comments, vec!["c".to_string()]);
    }

    #[test]
    fn angular_spline_passes_its_points() {
        let mut ctx = Ctx::default();
        // all shape factors 0: a polyline through the points
        let s = spline(T_OPEN_XSPLINE, &[(0, 0), (1200, 0), (1200, 1200)], &[0.0, 0.0, 0.0]);
        let pts = compute_open_spline(&mut ctx, &s, HIGH_PRECISION).unwrap();
        assert_eq!(pts.first(), Some(&Point::new(0, 0)));
        assert_eq!(pts.last(), Some(&Point::new(1200, 1200)));
        assert!(pts.contains(&Point::new(1200, 0)));
        // all on the two straight segments
        assert!(pts.iter().all(|q| q.y == 0 || q.x == 1200));
        assert!(pts.len() > 20);
        assert!(ctx.messages.is_empty());
    }

    #[test]
    fn approximated_spline_point_counts() {
        let mut ctx = Ctx::default();
        let s = spline(
            T_OPEN_XSPLINE,
            &[(1200, 1200), (2400, 600), (3600, 1800), (4800, 1200)],
            &[0.0, 1.0, 1.0, 0.0],
        );
        let pts = compute_open_spline(&mut ctx, &s, HIGH_PRECISION).unwrap();
        assert_eq!(pts.first(), Some(&Point::new(1200, 1200)));
        assert_eq!(pts.last(), Some(&Point::new(4800, 1200)));
        // the curve stays inside the control polygon's hull
        assert!(pts.iter().all(|q| q.y > 600 && q.y < 1800));
        assert!(pts.len() > 10);
        let l = create_line_with_spline(&mut ctx, &s).unwrap();
        assert_eq!(l.points, pts);
    }

    #[test]
    fn closed_spline_ends_where_it_starts() {
        let mut ctx = Ctx::default();
        let s = spline(
            T_CLOSED_XSPLINE,
            &[(1200, 3000), (2400, 2400), (3600, 3600), (2400, 4200)],
            &[-1.0; 4],
        );
        let l = create_line_with_spline(&mut ctx, &s).unwrap();
        assert_eq!(l.type_, T_POLYGON);
        assert_eq!(l.points.first(), l.points.last());
        // interpolating: passes the control points
        assert!(l.points.contains(&Point::new(2400, 2400)));
        let two = spline(T_CLOSED_XSPLINE, &[(0, 0), (10, 10)], &[-1.0; 2]);
        assert!(create_line_with_spline(&mut ctx, &two).is_none());
        assert_eq!(ctx.messages.len(), 1);
    }

    #[test]
    fn arrows_drop_three_points_per_end() {
        let mut ctx = Ctx::default();
        let mut s = spline(T_OPEN_XSPLINE, &[(0, 0), (1200, 1200), (2400, 0)], &[0.0, 1.0, 0.0]);
        let plain = compute_open_spline(&mut ctx, &s, HIGH_PRECISION).unwrap();
        let a = Arrow {
            type_: 1,
            style: 1,
            thickness: 15.0,
            wid: 60.0,
            ht: 120.0,
        };
        s.for_arrow = Some(a);
        s.back_arrow = Some(a);
        let l = create_line_with_spline(&mut ctx, &s).unwrap();
        assert_eq!(l.points.len(), plain.len() - 6);
        assert_eq!(l.points.first(), plain.first());
        assert_eq!(l.points.last(), plain.last());
        assert_eq!(l.for_arrow, Some(a));
    }

    #[test]
    fn hostile_splines_dont_panic() {
        let mut ctx = Ctx::default();
        let pts: Vec<(i32, i32)> = (0..200).map(|i| (i * 1_000_000 % i32::MAX, -i * 7_000_000)).collect();
        let s = spline(T_OPEN_XSPLINE, &pts, &[f64::NAN; 200]);
        let _ = create_line_with_spline(&mut ctx, &s);
        let s = spline(
            T_CLOSED_XSPLINE,
            &[(i32::MAX, i32::MIN), (0, 0), (i32::MIN, i32::MAX)],
            &[1e300, -1e300, 0.5],
        );
        let _ = create_line_with_spline(&mut ctx, &s);
        // mismatched controls: the type's factors
        let s = spline(T_OPEN_XSPLINE, &[(0, 0), (100, 100), (200, 0)], &[]);
        assert!(create_line_with_spline(&mut ctx, &s).is_some());
        let one = spline(T_OPEN_XSPLINE, &[(0, 0)], &[0.0]);
        assert!(create_line_with_spline(&mut ctx, &one).is_none());
        // too many points: capped, one message
        let many: Vec<(i32, i32)> = (0..6000).map(|i| (i * 1000, (i % 2) * 100_000)).collect();
        let s = spline(T_OPEN_XSPLINE, &many, &vec![1.0; 6000]);
        let mut ctx = Ctx::default();
        let l = create_line_with_spline(&mut ctx, &s).unwrap();
        assert!(l.points.len() <= MAXNUMPTS);
        assert_eq!(ctx.messages.len(), 1);
    }
}
