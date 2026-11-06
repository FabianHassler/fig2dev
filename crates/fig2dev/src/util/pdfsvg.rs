//! A PDF page as SVG markup: an SVG device on `hayro-interpret`, the
//! sibling of [`super::pdfps`] for the SVG writer.
//!
//! Paths, clips, text (as glyph outlines), strokes (width, caps, joins,
//! dashes), images and stencil masks stay vectors or embedded PNG data, and
//! so do axial and radial shadings (as gradients) used as the fill of a
//! path, provided they extend past both ends. Constant transparency of a
//! colour becomes `fill-opacity` / `stroke-opacity`. A page that uses
//! transparency groups, soft masks or blend modes, another kind of shading
//! (function-based, meshes, shadings that don't extend, with a background
//! or clip), a shading used as a stroke, or a tiling pattern, is drawn as
//! one embedded PNG instead (`PdfSvg::raster`).
//!
//! The markup is in the page's own units (points, origin at the upper left
//! corner, y down); the caller maps the page box onto the picture's
//! corners. Every call into the PDF crates is guarded: a hostile file gives
//! `None`, never a panic.
//
// Original code for fig2dev-rs (modelled on util/pdfps.rs).

use super::pdfpic::PdfPicture;
use super::pdfps::sample_function;
use crate::scene::raster::write_png;
use crate::scene::svg::base64;
use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::pattern::{Pattern, ShadingPattern};
use hayro::hayro_interpret::shading::ShadingType;
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, FillRule, Image, ImageData,
    ImageDrawProps, InterpreterCache, InterpreterSettings, Paint, SoftMask, StrokeProps, TransformExt,
};
use hayro::kurbo::{Affine, BezPath, Cap, Join, PathEl, Rect};
use std::fmt::Write as _;

/// Largest SVG markup written for one page, in bytes.
const MAX_SVG_BYTES: usize = 128 << 20;
/// Resolution of the image a page falls back to: points to pixels.
const RASTER_SCALE: f64 = 3.0;
/// Largest edge of that image, in pixels.
const RASTER_EDGE: f64 = 4000.0;
/// Every this many samples of a shading's colour function is a gradient stop.
const STOP_STRIDE: usize = 3;
/// Most clip paths (and shadings) of one page.
const MAX_DEFS: u32 = 1_000_000;

/// A PDF page converted to SVG.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfSvg {
    /// The markup: a `<g>` clipped to the page box, in points, y down.
    pub svg: String,
    /// The page's size in points.
    pub width: f64,
    pub height: f64,
    /// The page was drawn as an image (see the module's comment).
    pub raster: bool,
}

fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let r = (v * 10000.0).round() / 10000.0;
    if r == 0.0 {
        "0".into()
    } else {
        format!("{r}")
    }
}

fn hex(c: [f32; 3]) -> String {
    let b = |v: f32| (if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 } * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

fn matrix(m: Affine) -> String {
    let c = m.as_coeffs();
    format!(
        "matrix({} {} {} {} {} {})",
        num(c[0]),
        num(c[1]),
        num(c[2]),
        num(c[3]),
        num(c[4]),
        num(c[5])
    )
}

fn path_data(p: &BezPath) -> String {
    let mut d = String::new();
    for el in p.elements() {
        let _ = match *el {
            PathEl::MoveTo(q) => write!(d, "M{} {}", num(q.x), num(q.y)),
            PathEl::LineTo(q) => write!(d, "L{} {}", num(q.x), num(q.y)),
            PathEl::QuadTo(c, q) => write!(d, "Q{} {} {} {}", num(c.x), num(c.y), num(q.x), num(q.y)),
            PathEl::CurveTo(a, b, q) => write!(
                d,
                "C{} {} {} {} {} {}",
                num(a.x),
                num(a.y),
                num(b.x),
                num(b.y),
                num(q.x),
                num(q.y)
            ),
            PathEl::ClosePath => write!(d, "Z"),
        };
    }
    d
}

struct SvgDevice {
    out: String,
    prefix: String,
    /// Something the device can't express; the page is drawn as an image.
    unsupported: bool,
    defs: u32,
    depth: u32,
}

impl SvgDevice {
    fn over(&mut self) -> bool {
        if self.defs > MAX_DEFS {
            self.unsupported = true;
        }
        self.unsupported || self.out.len() > MAX_SVG_BYTES
    }

    fn next_id(&mut self, kind: &str) -> String {
        self.defs = self.defs.saturating_add(1);
        format!("{}{kind}{}", self.prefix, self.defs)
    }

    /// The colour of a plain paint and its opacity; `None` (and
    /// `unsupported`) for a paint SVG attributes can't express.
    fn color(&mut self, paint: &Paint<'_>) -> Option<(String, f32)> {
        let Paint::Color(c) = paint else {
            self.unsupported = true;
            return None;
        };
        let [r, g, b, a] = c.to_rgba().components();
        if !a.is_finite() {
            self.unsupported = true;
            return None;
        }
        Some((hex([r, g, b]), a.clamp(0.0, 1.0)))
    }

    fn props_ok(&mut self, props: &DrawProps<'_>) -> bool {
        if props.soft_mask.is_some() || props.blend_mode != BlendMode::Normal {
            self.unsupported = true;
            return false;
        }
        true
    }

    fn stroke_attrs(&self, s: &StrokeProps, color: &str, opacity: f32) -> String {
        let mut a = format!(" stroke=\"{color}\" stroke-width=\"{}\"", num(s.line_width.into()));
        if opacity < 0.999 {
            let _ = write!(a, " stroke-opacity=\"{}\"", num(opacity.into()));
        }
        match s.line_cap {
            Cap::Butt => {}
            Cap::Round => a.push_str(" stroke-linecap=\"round\""),
            Cap::Square => a.push_str(" stroke-linecap=\"square\""),
        }
        match s.line_join {
            Join::Miter => {
                let _ = write!(a, " stroke-miterlimit=\"{}\"", num(s.miter_limit.max(1.0).into()));
            }
            Join::Round => a.push_str(" stroke-linejoin=\"round\""),
            Join::Bevel => a.push_str(" stroke-linejoin=\"bevel\""),
        }
        if !s.dash_array.is_empty() && s.dash_array.iter().all(|d| d.is_finite() && *d >= 0.0) {
            let dash: Vec<String> = s.dash_array.iter().map(|d| num((*d).into())).collect();
            let _ = write!(a, " stroke-dasharray=\"{}\"", dash.join(" "));
            if s.dash_offset != 0.0 {
                let _ = write!(a, " stroke-dashoffset=\"{}\"", num(s.dash_offset.into()));
            }
        }
        a
    }

    /// Fills and/or strokes `path` (given in the space `transform` maps to
    /// the page's).
    fn paint_path(&mut self, path: &BezPath, transform: Affine, paint: &Paint<'_>, mode: &DrawMode) {
        if matches!(mode, DrawMode::Invisible) || self.over() {
            return;
        }
        if let Paint::Pattern(p) = paint {
            if let (Pattern::Shading(sp), DrawMode::Fill(rule)) = (&**p, mode) {
                self.shading_fill(&(transform * path.clone()), *rule, sp);
            } else {
                self.unsupported = true;
            }
            return;
        }
        let Some((color, opacity)) = self.color(paint) else {
            return;
        };
        let fill = |rule: &FillRule| {
            let mut a = format!(" fill=\"{color}\"");
            if opacity < 0.999 {
                let _ = write!(a, " fill-opacity=\"{}\"", num(opacity.into()));
            }
            if *rule == FillRule::EvenOdd {
                a.push_str(" fill-rule=\"evenodd\"");
            }
            a
        };
        let attrs = match mode {
            DrawMode::Fill(r) => fill(r),
            DrawMode::Stroke(s) => format!(" fill=\"none\"{}", self.stroke_attrs(s, &color, opacity)),
            DrawMode::FillAndStroke(r, s) => format!("{}{}", fill(r), self.stroke_attrs(s, &color, opacity)),
            DrawMode::Invisible => return,
        };
        let _ = writeln!(
            self.out,
            "<path transform=\"{}\" d=\"{}\"{attrs}/>",
            matrix(transform),
            path_data(path)
        );
    }

    /// Fills `path` (in page space) with an axial or radial shading as a
    /// gradient; anything else sets `unsupported`.
    fn shading_fill(&mut self, path: &BezPath, rule: FillRule, sp: &ShadingPattern) {
        let sh = &sp.shading;
        if sp.opacity < 0.999 || sp.transfer_function.is_some() || sh.background.is_some() || sh.clip_path.is_some() {
            self.unsupported = true;
            return;
        }
        let ShadingType::RadialAxial {
            coords,
            domain,
            function,
            extend,
            axial,
        } = &*sh.shading_type
        else {
            // a dummy shading paints nothing; the others are not supported
            self.unsupported = !matches!(&*sh.shading_type, ShadingType::Dummy);
            return;
        };
        // SVG gradients always pad
        if !(extend[0] && extend[1]) {
            self.unsupported = true;
            return;
        }
        let Some(samples) = sample_function(function, sh, *domain) else {
            self.unsupported = true;
            return;
        };
        let c = coords.map(f64::from);
        let id = self.next_id("g");
        let m = matrix(sp.matrix);
        let head = if *axial {
            format!(
                "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"",
                num(c[0]),
                num(c[1]),
                num(c[2]),
                num(c[3])
            )
        } else {
            // coords: x0 y0 r0 x1 y1 r1; SVG's focal circle must start at
            // radius 0 and lie inside the end circle
            let dist = (c[3] - c[0]).hypot(c[4] - c[1]);
            if c[2] != 0.0 || c[5] <= 0.0 || dist >= c[5] {
                self.unsupported = true;
                return;
            }
            format!(
                "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" cy=\"{}\" r=\"{}\" fx=\"{}\" fy=\"{}\"",
                num(c[3]),
                num(c[4]),
                num(c[5]),
                num(c[0]),
                num(c[1])
            )
        };
        let tag = if *axial { "linearGradient" } else { "radialGradient" };
        let _ = write!(self.out, "<defs>{head} gradientTransform=\"{m}\">");
        let count = samples.len() / 3;
        let last = count.saturating_sub(1).max(1);
        let mut i = 0;
        loop {
            let at = i.min(count.saturating_sub(1));
            if let Some(px) = samples.get(at * 3..at * 3 + 3) {
                let _ = write!(
                    self.out,
                    "<stop offset=\"{}\" stop-color=\"{}\"/>",
                    num(at as f64 / last as f64),
                    hex([px[0], px[1], px[2]].map(|v| f32::from(v) / 255.0))
                );
            }
            if at + 1 >= count {
                break;
            }
            i += STOP_STRIDE;
        }
        let _ = writeln!(self.out, "</{tag}></defs>");
        let _ = writeln!(
            self.out,
            "<path d=\"{}\" fill=\"url(#{id})\"{}/>",
            path_data(path),
            if rule == FillRule::EvenOdd {
                " fill-rule=\"evenodd\""
            } else {
                ""
            }
        );
    }

    /// An RGBA image whose pixel `(x, y)` is the unit square at `(x, y)` in
    /// the space `transform` maps to the page's.
    fn image(&mut self, transform: Affine, w: u32, h: u32, rgba: &[u8]) {
        let Ok(png) = write_png(w, h, rgba) else {
            self.unsupported = true;
            return;
        };
        let _ = writeln!(
            self.out,
            "<image width=\"{w}\" height=\"{h}\" preserveAspectRatio=\"none\" transform=\"{}\" \
             xlink:href=\"data:image/png;base64,{}\"/>",
            matrix(transform),
            base64(&png)
        );
    }
}

impl<'a> Device<'a> for SvgDevice {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, mode: &DrawMode) {
        if !self.props_ok(&props) {
            return;
        }
        self.paint_path(path, props.transform, &props.paint, mode);
    }

    fn push_clip_path(&mut self, clip: &ClipPath) {
        if self.over() {
            return;
        }
        let id = self.next_id("c");
        let rule = if clip.fill == FillRule::EvenOdd {
            "evenodd"
        } else {
            "nonzero"
        };
        let _ = writeln!(
            self.out,
            "<clipPath id=\"{id}\"><path d=\"{}\" clip-rule=\"{rule}\"/></clipPath>\n<g clip-path=\"url(#{id})\">",
            path_data(&clip.path)
        );
        self.depth += 1;
    }

    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask<'a>>, blend: BlendMode) {
        if opacity < 0.999 || mask.is_some() || blend != BlendMode::Normal {
            self.unsupported = true;
        }
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        if matches!(mode, DrawMode::Invisible) || !self.props_ok(&props) {
            return;
        }
        for g in run.glyphs() {
            if self.over() {
                return;
            }
            let gt = g.transform();
            match &**g {
                Glyph::Outline(o) => {
                    let outline = o.outline();
                    match mode {
                        DrawMode::Fill(_) => self.paint_path(&outline, props.transform * gt, &props.paint, mode),
                        _ => self.paint_path(&(gt * outline), props.transform, &props.paint, mode),
                    }
                }
                Glyph::Type3(t3) => {
                    t3.interpret(self, props.transform, gt, &props.paint);
                }
            }
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        if props.soft_mask.is_some() || props.blend_mode != BlendMode::Normal {
            self.unsupported = true;
            return;
        }
        if self.over() {
            return;
        }
        match image {
            Image::Raster(r) => {
                r.with_rgba(
                    |data, alpha| {
                        let (sx, sy) = data.scale_factors();
                        let t = props.transform * Affine::scale_non_uniform(f64::from(sx), f64::from(sy));
                        let (w, h) = (data.width(), data.height());
                        let n = (w as usize).saturating_mul(h as usize);
                        if w == 0 || h == 0 || n > 1 << 27 {
                            self.unsupported = true;
                            return;
                        }
                        if let Some(a) = &alpha {
                            if a.width != w || a.height != h || a.data.len() != n {
                                self.unsupported = true;
                                return;
                            }
                        }
                        let mut rgba = Vec::with_capacity(n * 4);
                        for i in 0..n {
                            let px = match &data {
                                ImageData::Rgb(d) => d.data.get(i * 3..i * 3 + 3).map(|p| [p[0], p[1], p[2]]),
                                ImageData::Luma(d) => d.data.get(i).map(|v| [*v; 3]),
                            };
                            let Some(px) = px else {
                                self.unsupported = true;
                                return;
                            };
                            let a = alpha.as_ref().and_then(|a| a.data.get(i)).copied().unwrap_or(255);
                            rgba.extend([px[0], px[1], px[2], a]);
                        }
                        self.image(t, w, h, &rgba);
                    },
                    None,
                );
            }
            Image::Stencil(s) => {
                s.with_stencil(
                    |luma, paint| {
                        let (sx, sy) = luma.scale_factors;
                        let t = props.transform * Affine::scale_non_uniform(f64::from(sx), f64::from(sy));
                        let Some((color, opacity)) = self.color(paint) else {
                            return;
                        };
                        let (w, h) = (luma.width as usize, luma.height as usize);
                        if w == 0 || h == 0 || w.saturating_mul(h) > 1 << 27 || luma.data.len() < w * h {
                            return;
                        }
                        let rgb =
                            [1, 3, 5].map(|i| u8::from_str_radix(color.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0));
                        let mut rgba = Vec::with_capacity(w * h * 4);
                        for v in luma.data.iter().take(w * h) {
                            let a = (f32::from(*v) * opacity).round().clamp(0.0, 255.0) as u8;
                            rgba.extend([rgb[0], rgb[1], rgb[2], a]);
                        }
                        self.image(t, luma.width, luma.height, &rgba);
                    },
                    None,
                );
            }
        }
    }

    fn pop_clip(&mut self) {
        if self.depth > 0 && !self.over() {
            self.out.push_str("</g>\n");
            self.depth -= 1;
        }
    }

    fn pop_transparency_group(&mut self) {}
}

/// The page as SVG (`PdfPicture::to_svg`). `id_prefix` starts every `id`
/// the markup defines, so several pictures in one document don't clash.
pub(crate) fn to_svg(p: &PdfPicture, id_prefix: &str) -> Option<PdfSvg> {
    let (w, h) = p.size();
    let wrap = |body: &str| {
        format!(
            "<clipPath id=\"{id_prefix}page\"><rect width=\"{}\" height=\"{}\"/></clipPath>\n\
             <g clip-path=\"url(#{id_prefix}page)\">\n{body}</g>\n",
            num(w),
            num(h)
        )
    };
    // the vector markup has the page's own colours; a greyed picture is
    // rendered (in grey) instead
    let device = if p.is_gray() {
        None
    } else {
        p.with_pdf(|pdf| {
            let page = pdf.pages().get(p.page())?;
            let cache = InterpreterCache::new();
            let initial = page.initial_transform(true);
            let mut ctx = Context::new(
                initial.to_kurbo(),
                Rect::new(0.0, 0.0, w, h),
                &cache,
                page.xref(),
                InterpreterSettings::default(),
            );
            let mut dev = SvgDevice {
                out: String::new(),
                prefix: id_prefix.to_string(),
                unsupported: false,
                defs: 0,
                depth: 0,
            };
            interpret_page(page, &mut ctx, &mut dev);
            while dev.depth > 0 {
                dev.out.push_str("</g>\n");
                dev.depth -= 1;
            }
            (!dev.over()).then_some(dev.out)
        })
    };
    match device {
        Some(body) => Some(PdfSvg {
            svg: wrap(&body),
            width: w,
            height: h,
            raster: false,
        }),
        None => raster_svg(p, &wrap),
    }
}

/// The page as one embedded PNG (transparent where nothing is drawn).
fn raster_svg(p: &PdfPicture, wrap: &dyn Fn(&str) -> String) -> Option<PdfSvg> {
    let (w, h) = p.size();
    let s = RASTER_SCALE.min(RASTER_EDGE / w.max(h));
    let img = p.render((w * s).ceil() as u32, (h * s).ceil() as u32)?;
    let png = write_png(img.width, img.height, &img.rgba).ok()?;
    let body = format!(
        "<image width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" transform=\"matrix({} 0 0 {} 0 0)\" \
         xlink:href=\"data:image/png;base64,{}\"/>\n",
        img.width,
        img.height,
        num(w / f64::from(img.width)),
        num(h / f64::from(img.height)),
        base64(&png)
    );
    Some(PdfSvg {
        svg: wrap(&body),
        width: w,
        height: h,
        raster: true,
    })
}

#[cfg(test)]
mod tests {
    use super::super::pdfpic::tests::sample_pdf;
    use super::*;

    #[test]
    fn page_becomes_vector_svg() {
        let p = PdfPicture::open(sample_pdf()).expect("opens");
        let s = to_svg(&p, "t_").expect("converts");
        assert!(!s.raster && !s.svg.contains("<image"));
        assert_eq!((s.width, s.height), (100.0, 100.0));
        assert!(s.svg.contains("id=\"t_page\""));
        // the red triangle and the blue square outline
        assert!(s.svg.contains("fill=\"#ff0000\"") && s.svg.contains("L"));
        assert!(s.svg.contains("stroke=\"#0000ff\"") && s.svg.contains("stroke-width=\"4\""));
        // the paths are in the PDF's space, y up; the matrix flips them
        assert!(
            s.svg.contains("matrix(1 0 0 -1 0 100)") && s.svg.contains("L50 90"),
            "{}",
            s.svg
        );
    }

    #[test]
    fn grey_pictures_are_rendered() {
        let p = PdfPicture::open(sample_pdf()).expect("opens").grayed();
        let s = to_svg(&p, "t_").expect("converts");
        assert!(s.raster && s.svg.contains("data:image/png;base64,"));
    }
}
