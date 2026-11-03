//! Special (LaTeX) texts typeset by TeX, for the scene languages (PDF,
//! PostScript, EPS, SVG and the bitmaps).
//!
//! fig2dev itself never starts TeX: a program that can (RustFig's export)
//! puts a [`TextTypesetter`] into [`Ctx::typesetter`](crate::gendev::Ctx),
//! and the scene builder then draws each special text as the PDF page TeX
//! made for it, a vector picture at the text's anchor, justification and
//! angle. A text the typesetter has no page for is drawn as text, as
//! before.

use super::Rgba;
use crate::util::pdfpic::PdfPicture;
use figbase::geom::Point;
use figbase::style::Justify;
use std::sync::Arc;

/// Fig units (1200 per inch) per PostScript point.
const FIG_PER_PT: f64 = 1200.0 / 72.0;

/// One special text to typeset.
#[derive(Debug, Clone, Copy)]
pub struct TypesetRequest<'a> {
    pub text: &'a str,
    pub font: i32,
    /// Fig fonts of the PostScript set rather than the LaTeX set.
    pub postscript: bool,
    /// Points, as the text is drawn.
    pub size: f64,
    pub color: Rgba,
}

/// A special text typeset by TeX: the page and the text's box on it.
#[derive(Debug, Clone)]
pub struct Typeset {
    /// TeX's page: the text's box with `margin_pt` on every side (ink may
    /// reach outside the box).
    pub pdf: Arc<PdfPicture>,
    /// Width, height (including depth) and depth below the baseline, in
    /// PostScript points.
    pub width_pt: f64,
    pub height_pt: f64,
    pub depth_pt: f64,
    /// Room around the box on the page, in PostScript points.
    pub margin_pt: f64,
}

/// Source of typeset special texts; `None` draws the text as text.
pub trait TextTypesetter: Send + Sync + std::fmt::Debug {
    fn typeset(&self, req: &TypesetRequest<'_>) -> Option<Typeset>;
}

/// The corners of a typeset text's box (top-left, top-right, bottom-right,
/// bottom-left of the unrotated text) at the text's anchor, justification
/// and angle (radians); `pad` widens it on every side. Fig units, y down.
pub fn corners(origin: Point, angle: f64, justify: Justify, ts: &Typeset, pad: f64) -> [Point; 4] {
    let angle = if angle.is_finite() { angle } else { 0.0 };
    let (w, h, d) = (
        ts.width_pt * FIG_PER_PT,
        ts.height_pt * FIG_PER_PT,
        ts.depth_pt * FIG_PER_PT,
    );
    let u0 = match justify {
        Justify::Left => 0.0,
        Justify::Center => -w / 2.0,
        Justify::Right => -w,
    };
    let (c, s) = (angle.cos(), angle.sin());
    // u along the baseline, v up (Fig's y grows downwards).
    let at = |u: f64, v: f64| Point::new(origin.x + u * c - v * s, origin.y - u * s - v * c);
    let pad = if pad.is_finite() { pad.max(0.0) } else { 0.0 };
    let (u0, u1) = (u0 - pad, u0 + w + pad);
    let (v0, v1) = (-d - pad, h - d + pad);
    [at(u0, v1), at(u1, v1), at(u1, v0), at(u0, v0)]
}

/// The corners of the whole page TeX made (the box plus its margin).
pub fn page_corners(origin: Point, angle: f64, justify: Justify, ts: &Typeset) -> [Point; 4] {
    corners(origin, angle, justify, ts, ts.margin_pt * FIG_PER_PT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_follow_justification_and_angle() {
        let pdf = PdfPicture::open(crate::util::pdfpic::tests::sample_pdf()).unwrap();
        let ts = Typeset {
            pdf: Arc::new(pdf),
            width_pt: 72.0,
            height_pt: 36.0,
            depth_pt: 0.0,
            margin_pt: 0.0,
        };
        let o = Point::new(1000.0, 1000.0);
        let [tl, tr, br, bl] = corners(o, 0.0, Justify::Left, &ts, 0.0);
        assert_eq!((bl.x, bl.y), (1000.0, 1000.0));
        assert_eq!((br.x, tr.y, tl.x), (2200.0, 400.0, 1000.0));
        let [_, _, br, bl] = corners(o, 0.0, Justify::Right, &ts, 0.0);
        assert_eq!((bl.x, br.x), (-200.0, 1000.0));
        // A quarter turn counterclockwise: the baseline runs up the page.
        let [_, _, br, _] = corners(o, std::f64::consts::FRAC_PI_2, Justify::Left, &ts, 0.0);
        assert!((br.x - 1000.0).abs() < 1e-9 && (br.y + 200.0).abs() < 1e-9);
    }
}
