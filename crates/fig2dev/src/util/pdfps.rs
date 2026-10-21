//! A PDF page as EPS: a PostScript device on `hayro-interpret`, standing in
//! for the `pdftops` step of fig2dev's C code (`readeps.c`).
//!
//! Paths, clips, text (as glyph outlines), images and stencil masks stay
//! vectors or image data. PostScript Level 2 has no transparency, so a page
//! that uses opacity, soft masks, blend modes, shadings or tiling patterns
//! is drawn as one image instead (`PdfEps::raster`, the caller says so).
//
// Ported from fig2dev dev/readeps.c (`pdftops`: the command's role) (xfig
// licence, see LICENSE-xfig).

use super::pdfpic::PdfPicture;
use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, FillRule, Image, ImageData,
    ImageDrawProps, InterpreterCache, InterpreterSettings, Paint, SoftMask, TransformExt,
};
use hayro::kurbo::{Affine, BezPath, Cap, Join, PathEl, Rect};
use std::fmt::Write as _;

/// Largest EPS written for one page, in bytes.
const MAX_EPS_BYTES: usize = 256 << 20;
/// Resolution of the image a page falls back to: points to pixels.
const RASTER_SCALE: f64 = 3.0;
/// Largest edge of that image, in pixels.
const RASTER_EDGE: f64 = 4000.0;

/// A PDF page converted to EPS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfEps {
    /// The PostScript, with `%%BoundingBox: 0 0 width height`.
    pub ps: String,
    /// The bounding box: the page's size in points, rounded up.
    pub width: i32,
    pub height: i32,
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

struct PsDevice {
    out: String,
    /// Something the device can't express; the page is drawn as an image.
    unsupported: bool,
}

impl PsDevice {
    fn over(&mut self) -> bool {
        self.unsupported || self.out.len() > MAX_EPS_BYTES
    }

    fn matrix(&mut self, m: Affine) {
        let c = m.as_coeffs();
        let _ = writeln!(
            self.out,
            "[{} {} {} {} {} {}] concat",
            num(c[0]),
            num(c[1]),
            num(c[2]),
            num(c[3]),
            num(c[4]),
            num(c[5])
        );
    }

    fn path(&mut self, p: &BezPath) {
        self.out.push_str("newpath\n");
        let mut cur = (0.0, 0.0);
        for el in p.elements() {
            match *el {
                PathEl::MoveTo(q) => {
                    cur = (q.x, q.y);
                    let _ = writeln!(self.out, "{} {} moveto", num(q.x), num(q.y));
                }
                PathEl::LineTo(q) => {
                    cur = (q.x, q.y);
                    let _ = writeln!(self.out, "{} {} lineto", num(q.x), num(q.y));
                }
                PathEl::QuadTo(c, q) => {
                    // the quadratic as the cubic with the same curve
                    let c1 = (cur.0 + 2.0 / 3.0 * (c.x - cur.0), cur.1 + 2.0 / 3.0 * (c.y - cur.1));
                    let c2 = (q.x + 2.0 / 3.0 * (c.x - q.x), q.y + 2.0 / 3.0 * (c.y - q.y));
                    let _ = writeln!(
                        self.out,
                        "{} {} {} {} {} {} curveto",
                        num(c1.0),
                        num(c1.1),
                        num(c2.0),
                        num(c2.1),
                        num(q.x),
                        num(q.y)
                    );
                    cur = (q.x, q.y);
                }
                PathEl::CurveTo(a, b, q) => {
                    cur = (q.x, q.y);
                    let _ = writeln!(
                        self.out,
                        "{} {} {} {} {} {} curveto",
                        num(a.x),
                        num(a.y),
                        num(b.x),
                        num(b.y),
                        num(q.x),
                        num(q.y)
                    );
                }
                PathEl::ClosePath => self.out.push_str("closepath\n"),
            }
        }
    }

    /// Sets the colour; `false` for a paint PostScript can't express.
    fn paint(&mut self, paint: &Paint<'_>) -> bool {
        let Paint::Color(c) = paint else {
            self.unsupported = true;
            return false;
        };
        let [r, g, b, a] = c.to_rgba().components();
        if a < 0.999 {
            self.unsupported = true;
            return false;
        }
        let _ = writeln!(
            self.out,
            "{} {} {} setrgbcolor",
            num(r.into()),
            num(g.into()),
            num(b.into())
        );
        true
    }

    fn props_ok(&mut self, props: &DrawProps<'_>) -> bool {
        if props.soft_mask.is_some() || props.blend_mode != BlendMode::Normal {
            self.unsupported = true;
            return false;
        }
        true
    }

    fn stroke_style(&mut self, s: &hayro::hayro_interpret::StrokeProps) {
        let cap = match s.line_cap {
            Cap::Butt => 0,
            Cap::Round => 1,
            Cap::Square => 2,
        };
        let join = match s.line_join {
            Join::Miter => 0,
            Join::Round => 1,
            Join::Bevel => 2,
        };
        let _ = writeln!(
            self.out,
            "{} setlinewidth {cap} setlinecap {join} setlinejoin {} setmiterlimit",
            num(s.line_width.into()),
            num(s.miter_limit.max(1.0).into())
        );
        let dash: Vec<String> = s.dash_array.iter().map(|d| num((*d).into())).collect();
        let _ = writeln!(self.out, "[{}] {} setdash", dash.join(" "), num(s.dash_offset.into()));
    }

    /// Fills and/or strokes `path` (given in the space `transform` maps to
    /// the page's).
    fn paint_path(&mut self, path: &BezPath, transform: Affine, paint: &Paint<'_>, mode: &DrawMode) {
        if matches!(mode, DrawMode::Invisible) || self.over() || !self.paint(paint) {
            return;
        }
        self.out.push_str("gsave\n");
        self.matrix(transform);
        self.path(path);
        let rule = |r: &FillRule| if *r == FillRule::EvenOdd { "eofill" } else { "fill" };
        match mode {
            DrawMode::Fill(r) => {
                let _ = writeln!(self.out, "{}", rule(r));
            }
            DrawMode::Stroke(s) => {
                self.stroke_style(s);
                self.out.push_str("stroke\n");
            }
            DrawMode::FillAndStroke(r, s) => {
                self.stroke_style(s);
                let _ = writeln!(self.out, "gsave {} grestore stroke", rule(r));
            }
            DrawMode::Invisible => {}
        }
        self.out.push_str("grestore\n");
    }

    fn hex(&mut self, data: impl Iterator<Item = u8>) {
        let mut col = 0;
        for b in data {
            let _ = write!(self.out, "{b:02x}");
            col += 2;
            if col >= 78 {
                self.out.push('\n');
                col = 0;
            }
        }
        self.out.push('\n');
    }

    /// An image whose pixel `(x, y)` is the unit square at `(x, y)` in the
    /// space `transform` maps to the page's.
    fn image_data(&mut self, transform: Affine, w: u32, h: u32, comps: u32, data: &[u8]) {
        let _ = writeln!(self.out, "gsave");
        self.matrix(transform);
        let _ = writeln!(self.out, "/picstr {} string def", w as usize * comps as usize);
        if comps == 3 {
            let _ = writeln!(
                self.out,
                "{w} {h} 8 [1 0 0 1 0 0] {{currentfile picstr readhexstring pop}} false 3 colorimage"
            );
        } else {
            let _ = writeln!(
                self.out,
                "{w} {h} 8 [1 0 0 1 0 0] {{currentfile picstr readhexstring pop}} image"
            );
        }
        self.hex(data.iter().copied());
        self.out.push_str("grestore\n");
    }
}

impl<'a> Device<'a> for PsDevice {
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
        self.out.push_str("gsave\n");
        self.path(&clip.path);
        self.out.push_str(if clip.fill == FillRule::EvenOdd {
            "eoclip\n"
        } else {
            "clip\n"
        });
        self.out.push_str("newpath\n");
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
                        if let Some(a) = &alpha {
                            if a.data.iter().any(|v| *v < 255) {
                                self.unsupported = true;
                                return;
                            }
                        }
                        let (sx, sy) = data.scale_factors();
                        let t = props.transform * Affine::scale_non_uniform(f64::from(sx), f64::from(sy));
                        match &data {
                            ImageData::Rgb(d) => self.image_data(t, d.width, d.height, 3, &d.data),
                            ImageData::Luma(d) => self.image_data(t, d.width, d.height, 1, &d.data),
                        }
                    },
                    None,
                );
            }
            Image::Stencil(s) => {
                s.with_stencil(
                    |luma, paint| {
                        let (sx, sy) = luma.scale_factors;
                        let t = props.transform * Affine::scale_non_uniform(f64::from(sx), f64::from(sy));
                        if !self.paint(paint) {
                            return;
                        }
                        let (w, h) = (luma.width as usize, luma.height as usize);
                        if w == 0 || h == 0 || luma.data.len() < w * h {
                            return;
                        }
                        self.out.push_str("gsave\n");
                        self.matrix(t);
                        let row = w.div_ceil(8);
                        let _ = writeln!(self.out, "/picstr {row} string def");
                        let _ = writeln!(
                            self.out,
                            "{w} {h} false [1 0 0 1 0 0] {{currentfile picstr readhexstring pop}} imagemask"
                        );
                        // sample 0 paints: bit 0 where the stencil is 255
                        let mut bytes = Vec::with_capacity(row * h);
                        for y in 0..h {
                            for bx in 0..row {
                                let mut b = 0xffu8;
                                for k in 0..8 {
                                    let x = bx * 8 + k;
                                    if x < w && luma.data.get(y * w + x) == Some(&255) {
                                        b &= !(0x80 >> k);
                                    }
                                }
                                bytes.push(b);
                            }
                        }
                        self.hex(bytes.into_iter());
                        self.out.push_str("grestore\n");
                    },
                    None,
                );
            }
        }
    }

    fn pop_clip(&mut self) {
        if !self.over() {
            self.out.push_str("grestore\n");
        }
    }

    fn pop_transparency_group(&mut self) {}
}

/// The page as EPS (`PdfPicture::to_eps`).
pub(crate) fn to_eps(p: &PdfPicture) -> Option<PdfEps> {
    let (w, h) = p.size();
    let (bw, bh) = (w.ceil() as i32, h.ceil() as i32);
    let head = format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 {bw} {bh}\n%%Creator: {}, from a PDF page\n%%EndComments\n",
        crate::scene::CREATOR
    );
    let device = p.with_pdf(|pdf| {
        let page = pdf.pages().first()?;
        let cache = InterpreterCache::new();
        let initial = page.initial_transform(false);
        let mut ctx = Context::new(
            initial.to_kurbo(),
            Rect::new(0.0, 0.0, w, h),
            &cache,
            page.xref(),
            InterpreterSettings::default(),
        );
        let mut dev = PsDevice {
            out: String::new(),
            unsupported: false,
        };
        interpret_page(page, &mut ctx, &mut dev);
        (!dev.over() || dev.unsupported).then_some((dev.out, dev.unsupported))
    });
    match device {
        Some((body, false)) => Some(PdfEps {
            ps: format!("{head}{body}%%EOF\n"),
            width: bw,
            height: bh,
            raster: false,
        }),
        _ => raster_eps(p, &head, bw, bh),
    }
}

/// The page as one image (over white, as PostScript has no alpha).
fn raster_eps(p: &PdfPicture, head: &str, bw: i32, bh: i32) -> Option<PdfEps> {
    let (w, h) = p.size();
    let s = RASTER_SCALE.min(RASTER_EDGE / w.max(h));
    let img = p.render((w * s).ceil() as u32, (h * s).ceil() as u32)?;
    let mut dev = PsDevice {
        out: String::new(),
        unsupported: false,
    };
    let rgb = img.rgba.as_chunks::<4>().0.iter().flat_map(|px| {
        let a = u16::from(px[3]);
        let over = |c: u8| ((u16::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
        [over(px[0]), over(px[1]), over(px[2])]
    });
    let _ = writeln!(dev.out, "gsave");
    let _ = writeln!(
        dev.out,
        "[{} 0 0 {} 0 {}] concat",
        num(w / f64::from(img.width)),
        num(-h / f64::from(img.height)),
        num(h)
    );
    let _ = writeln!(dev.out, "/picstr {} string def", img.width as usize * 3);
    let _ = writeln!(
        dev.out,
        "{} {} 8 [1 0 0 1 0 0] {{currentfile picstr readhexstring pop}} false 3 colorimage",
        img.width, img.height
    );
    dev.hex(rgb);
    dev.out.push_str("grestore\n");
    Some(PdfEps {
        ps: format!("{head}{}%%EOF\n", dev.out),
        width: bw,
        height: bh,
        raster: true,
    })
}

#[cfg(test)]
mod tests {
    use super::super::pdfpic::tests::sample_pdf;
    use super::*;

    #[test]
    fn page_becomes_vector_eps() {
        let p = PdfPicture::open(sample_pdf()).expect("opens");
        let e = to_eps(&p).expect("converts");
        assert!(!e.raster);
        assert_eq!((e.width, e.height), (100, 100));
        assert!(e
            .ps
            .starts_with("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n"));
        assert!(e.ps.contains("1 0 0 setrgbcolor"));
        assert!(e.ps.contains("lineto") && e.ps.contains("fill"));
        assert!(e.ps.contains("0 0 1 setrgbcolor") && e.ps.contains("stroke"));
    }
}
