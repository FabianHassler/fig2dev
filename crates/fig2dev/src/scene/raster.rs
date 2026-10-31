//! Raster backend (tiny-skia): canvas drawing and bitmap export.

use super::pattern::{coverage, PatternFrame};
use super::{textcache, Fill, Prim, Rgba, Scene, Stroke, SubPath, TextPrim};
use crate::util::pdfpic::PdfPicture;
use figbase::geom::{Affine, Point, Rect};
use figbase::image::RasterImage;
use figbase::style::{CapStyle, JoinStyle, Justify};
use figbase::text::PathOp;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

pub use tiny_skia::Pixmap;
use tiny_skia::{
    Color, FillRule, IntSize, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, PixmapPaint, Shader, StrokeDash,
    Transform,
};

/// Largest bitmap edge produced by exports, in pixels.
pub const MAX_EDGE: u32 = 20_000;
/// Largest bitmap area produced by exports, in pixels.
pub const MAX_PIXELS: u64 = 200_000_000;

/// ghostscript's default `fill_adjust` (pixels): hard-edged shapes grow by
/// this much on every side.
const GS_FILL_ADJUST: f64 = 0.3;

/// Hard-edged pictures: the least alpha (of 255) that makes a pixel of a
/// rendered PDF picture opaque; the rest is dropped. Calibrated so that
/// lines and glyphs come out as many black pixels as ghostscript's own
/// hard-edged output (fig2dev's `bitmaps.at` counts them).
const HARD_EDGE_MIN_ALPHA: u8 = 6;

/// Rasterisation parameters.
#[derive(Debug, Clone, Copy)]
pub struct RasterOptions {
    /// Fig units → device pixels.
    pub transform: Affine,
    /// Thinnest stroke drawn, in pixels (1.0 on screen so hairlines show).
    pub min_line_px: f64,
    /// Hard-edged output (no smoothing): shapes grow by this many pixels
    /// on every side, as ghostscript's `fill_adjust` makes every pixel a
    /// shape touches black; 0 when smoothed.
    pub adjust_px: f64,
    /// Where fill patterns are anchored and how large their tiles are.
    pub pattern: PatternFrame,
    /// The pixmap is this many times larger than the bitmap (supersampled
    /// `-S 2`/`-S 4`; 1 otherwise). Ghostscript draws a pattern's lines
    /// hard-edged at the bitmap's own resolution, whatever the smoothing, so
    /// with more than 1 they are drawn at that resolution and enlarged.
    pub pattern_k: u32,
}

fn color(c: Rgba) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn paint(c: Rgba, anti_alias: bool) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(color(c)),
        anti_alias,
        ..Paint::default()
    }
}

fn f(v: f64) -> f32 {
    if v.is_finite() {
        v.clamp(-1.0e7, 1.0e7) as f32
    } else {
        0.0
    }
}

fn build_path(subpaths: &[SubPath], t: &Affine) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for s in subpaths {
        let mut it = s.pts.iter().map(|p| t.apply(*p));
        let Some(first) = it.next() else { continue };
        pb.move_to(f(first.x), f(first.y));
        for p in it {
            pb.line_to(f(p.x), f(p.y));
        }
        if s.closed {
            pb.close();
        }
    }
    pb.finish()
}

fn device_bbox(subpaths: &[SubPath], t: &Affine) -> Rect {
    let mut r = Rect::EMPTY;
    for s in subpaths {
        for p in &s.pts {
            r.add_point(t.apply(*p));
        }
    }
    r
}

fn sk_stroke(s: &Stroke, scale: f64, min_px: f64) -> tiny_skia::Stroke {
    let width = (s.width * scale).max(min_px);
    let dash = if s.dash.is_empty() {
        None
    } else {
        let d: Vec<f32> = s.dash.iter().map(|v| f((v * scale).max(0.01))).collect();
        StrokeDash::new(d, 0.0)
    };
    tiny_skia::Stroke {
        width: f(width),
        // genps.c: `10 setmiterlimit`.
        miter_limit: 10.0,
        line_cap: match s.cap {
            CapStyle::Butt => LineCap::Butt,
            CapStyle::Round => LineCap::Round,
            CapStyle::Projecting => LineCap::Square,
        },
        line_join: match s.join {
            JoinStyle::Miter => LineJoin::Miter,
            JoinStyle::Round => LineJoin::Round,
            JoinStyle::Bevel => LineJoin::Bevel,
        },
        dash,
    }
}

fn draw_path(
    pm: &mut Pixmap,
    subpaths: &[SubPath],
    fill: Option<&Fill>,
    stroke: Option<&Stroke>,
    opts: &RasterOptions,
    aa: bool,
) {
    let t = &opts.transform;
    let scale = t.mean_scale();
    let bb = device_bbox(subpaths, t);
    let sw = stroke.map(|s| (s.width * scale).max(opts.min_line_px)).unwrap_or(0.0);
    let view = Rect::new(Point::ZERO, Point::new(f64::from(pm.width()), f64::from(pm.height())));
    if !bb.expand(sw + 2.0).intersects(&view) {
        return;
    }
    let Some(path) = build_path(subpaths, t) else {
        return;
    };
    match fill {
        Some(Fill::Solid(c)) => pm.fill_path(&path, &paint(*c, aa), FillRule::EvenOdd, Transform::identity(), None),
        Some(Fill::Pattern { kind, fg, bg, nonzero }) => {
            let rule = if *nonzero { FillRule::Winding } else { FillRule::EvenOdd };
            pm.fill_path(&path, &paint(*bg, aa), rule, Transform::identity(), None);
            if let Some(mut mask) = Mask::new(pm.width(), pm.height()) {
                mask.fill_path(&path, rule, aa, Transform::identity());
                if opts.pattern_k > 1 {
                    draw_pattern_at_bitmap_resolution(pm, &mask, bb, *kind, *fg, opts);
                } else if let Some(inv) = t.invert() {
                    // Pattern lines over the visible part of the shape, in Fig units.
                    let vis = Rect::from_points(bb.intersection_or_empty(&view).corners().map(|p| inv.apply(p)));
                    let (lines, line_width) = coverage(*kind, &vis, &opts.pattern);
                    let subs: Vec<SubPath> = lines
                        .into_iter()
                        .map(|pts| SubPath {
                            pts,
                            closed: false,
                            curve: None,
                        })
                        .collect();
                    if let Some(lp) = build_path(&subs, t) {
                        let st = tiny_skia::Stroke {
                            width: f((line_width * scale).max(0.5) + opts.adjust_px),
                            ..tiny_skia::Stroke::default()
                        };
                        pm.stroke_path(&lp, &paint(*fg, aa), &st, Transform::identity(), Some(&mask));
                    }
                }
            }
        }
        None => {}
    }
    if let Some(s) = stroke {
        let mut st = sk_stroke(s, scale, opts.min_line_px);
        st.width += f(2.0 * opts.adjust_px);
        pm.stroke_path(&path, &paint(s.color, aa), &st, Transform::identity(), None);
    }
}

/// A pattern's lines as ghostscript draws them into a smoothed bitmap: hard
/// pixels at the bitmap's resolution (`opts.pattern_k` pixels of `pm` make
/// one), shown where `mask` (the shape, at `pm`'s resolution) lets them
/// through. `bb` is the shape's box in `pm` pixels.
fn draw_pattern_at_bitmap_resolution(
    pm: &mut Pixmap,
    mask: &Mask,
    bb: Rect,
    kind: i32,
    fg: Rgba,
    opts: &RasterOptions,
) {
    let k = opts.pattern_k;
    let kf = f64::from(k);
    // The part of the bitmap the shape can touch, in bitmap pixels.
    let (bw, bh) = (pm.width() / k, pm.height() / k);
    let clamp = |v: f64, hi: u32| v.clamp(0.0, f64::from(hi));
    let x0 = clamp((bb.min.x / kf).floor() - 1.0, bw) as u32;
    let y0 = clamp((bb.min.y / kf).floor() - 1.0, bh) as u32;
    let x1 = clamp((bb.max.x / kf).ceil() + 1.0, bw) as u32;
    let y1 = clamp((bb.max.y / kf).ceil() + 1.0, bh) as u32;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let t1 = opts
        .transform
        .then(&Affine::scale(1.0 / kf, 1.0 / kf))
        .then(&Affine::translate(-f64::from(x0), -f64::from(y0)));
    let Some(inv) = t1.invert() else {
        return;
    };
    let Some(mut tile) = Pixmap::new(x1 - x0, y1 - y0) else {
        return;
    };
    let region = Rect::new(Point::ZERO, Point::new(f64::from(x1 - x0), f64::from(y1 - y0)));
    let vis = Rect::from_points(region.corners().map(|p| inv.apply(p)));
    let (lines, line_width) = coverage(kind, &vis, &opts.pattern);
    let subs: Vec<SubPath> = lines
        .into_iter()
        .map(|pts| SubPath {
            pts,
            closed: false,
            curve: None,
        })
        .collect();
    let Some(lp) = build_path(&subs, &t1) else {
        return;
    };
    let st = tiny_skia::Stroke {
        width: f((line_width * t1.mean_scale()).max(0.5) + GS_FILL_ADJUST),
        ..tiny_skia::Stroke::default()
    };
    tile.stroke_path(&lp, &paint(fg, false), &st, Transform::identity(), None);
    // Enlarge by pixel repetition to `pm`'s resolution.
    let Some(mut big) = Pixmap::new((x1 - x0) * k, (y1 - y0) * k) else {
        return;
    };
    let (tw, bgw, kk) = (tile.width() as usize, big.width() as usize, k as usize);
    let src = tile.data();
    for (y, row) in big.data_mut().chunks_exact_mut(bgw * 4).enumerate() {
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let i = ((y / kk) * tw + x / kk) * 4;
            if let Some(p) = src.get(i..i + 4) {
                px.copy_from_slice(p);
            }
        }
    }
    pm.draw_pixmap(
        (x0 * k) as i32,
        (y0 * k) as i32,
        big.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        Some(mask),
    );
}

/// Text outlines in Fig units, justified, rotated and placed at the origin.
pub(crate) fn text_ops_fig(t: &TextPrim) -> (Vec<PathOp>, bool, f64) {
    let shaped = textcache::shaped(t);
    let rot = Affine::rotate(t.angle);
    let mut ops = Vec::new();
    let map = |x: f64, y: f64, dx: f64| {
        let p = t.origin + rot.apply_vec(Point::new(x + dx, y));
        (p.x, p.y)
    };
    for (line, w) in shaped.lines.iter().zip(shaped.line_widths.iter()) {
        let dx = match t.justify {
            Justify::Left => 0.0,
            Justify::Center => -w / 2.0,
            Justify::Right => -w,
        };
        for op in line {
            ops.push(match *op {
                PathOp::Move(x, y) => {
                    let (x, y) = map(x, y, dx);
                    PathOp::Move(x, y)
                }
                PathOp::Line(x, y) => {
                    let (x, y) = map(x, y, dx);
                    PathOp::Line(x, y)
                }
                PathOp::Quad(a, b, x, y) => {
                    let (a, b) = map(a, b, dx);
                    let (x, y) = map(x, y, dx);
                    PathOp::Quad(a, b, x, y)
                }
                PathOp::Cubic(a, b, c, d, x, y) => {
                    let (a, b) = map(a, b, dx);
                    let (c, d) = map(c, d, dx);
                    let (x, y) = map(x, y, dx);
                    PathOp::Cubic(a, b, c, d, x, y)
                }
                PathOp::Close => PathOp::Close,
            });
        }
    }
    (ops, shaped.synthetic_bold, super::font_px(t.size) * 0.04)
}

fn draw_text(pm: &mut Pixmap, t: &TextPrim, opts: &RasterOptions, aa: bool) {
    let tr = &opts.transform;
    let view = Rect::new(Point::ZERO, Point::new(f64::from(pm.width()), f64::from(pm.height())));
    let bb = Rect::from_points(super::Prim::Text(t.clone()).bbox().corners().map(|p| tr.apply(p)));
    if !bb.intersects(&view) {
        return;
    }
    let (ops, bold, bold_w) = text_ops_fig(t);
    let mut pb = PathBuilder::new();
    let m = |x: f64, y: f64| {
        let p = tr.apply(Point::new(x, y));
        (f(p.x), f(p.y))
    };
    for op in ops {
        match op {
            PathOp::Move(x, y) => {
                let (x, y) = m(x, y);
                pb.move_to(x, y);
            }
            PathOp::Line(x, y) => {
                let (x, y) = m(x, y);
                pb.line_to(x, y);
            }
            PathOp::Quad(a, b, x, y) => {
                let (a, b) = m(a, b);
                let (x, y) = m(x, y);
                pb.quad_to(a, b, x, y);
            }
            PathOp::Cubic(a, b, c, d, x, y) => {
                let (a, b) = m(a, b);
                let (c, d) = m(c, d);
                let (x, y) = m(x, y);
                pb.cubic_to(a, b, c, d, x, y);
            }
            PathOp::Close => pb.close(),
        }
    }
    let Some(path) = pb.finish() else { return };
    pm.fill_path(
        &path,
        &paint(t.color, aa),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    if bold {
        let st = tiny_skia::Stroke {
            width: f(bold_w * tr.mean_scale()),
            line_join: LineJoin::Round,
            ..tiny_skia::Stroke::default()
        };
        pm.stroke_path(&path, &paint(t.color, aa), &st, Transform::identity(), None);
    }
}

thread_local! {
    static IMAGES: RefCell<HashMap<usize, (Arc<RasterImage>, Pixmap)>> = RefCell::new(HashMap::new());
}

/// Premultiplied pixmap for an image (cached by identity).
fn image_pixmap(img: &Arc<RasterImage>) -> Option<Pixmap> {
    let key = Arc::as_ptr(img) as usize;
    IMAGES.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((_, pm)) = c.get(&key) {
            return Some(pm.clone());
        }
        let size = IntSize::from_wh(img.width, img.height)?;
        let expected = (img.width as usize).checked_mul(img.height as usize)?.checked_mul(4)?;
        if img.rgba.len() != expected {
            return None;
        }
        let mut data = img.rgba.clone();
        for [r, g, b, a] in data.as_chunks_mut::<4>().0.iter_mut() {
            let al = u16::from(*a);
            *r = ((u16::from(*r) * al + 127) / 255) as u8;
            *g = ((u16::from(*g) * al + 127) / 255) as u8;
            *b = ((u16::from(*b) * al + 127) / 255) as u8;
        }
        let pm = Pixmap::from_vec(data, size)?;
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, (img.clone(), pm.clone()));
        Some(pm)
    })
}

fn draw_image(pm: &mut Pixmap, corners: &[Point; 4], img: &Arc<RasterImage>, flipped: bool, opts: &RasterOptions) {
    let t = &opts.transform;
    let view = Rect::new(Point::ZERO, Point::new(f64::from(pm.width()), f64::from(pm.height())));
    let bb = Rect::from_points(corners.iter().map(|p| t.apply(*p)));
    if !bb.intersects(&view) || img.width == 0 || img.height == 0 {
        return;
    }
    let Some(src) = image_pixmap(img) else { return };
    let [tl, tr, _br, bl] = corners.map(|p| t.apply(p));
    // Image x axis → top edge, y axis → left edge (swapped when flipped).
    let (ex, ey) = if flipped {
        (bl - tl, tr - tl)
    } else {
        (tr - tl, bl - tl)
    };
    let sx = ex * (1.0 / f64::from(img.width));
    let sy = ey * (1.0 / f64::from(img.height));
    let ts = Transform::from_row(f(sx.x), f(sx.y), f(sy.x), f(sy.y), f(tl.x), f(tl.y));
    let paint = PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    pm.draw_pixmap(0, 0, src.as_ref(), &paint, ts, None);
}

/// A rendered PDF picture: (file id, width, height) and the pixels.
type PdfPage = ((u64, u32, u32), Arc<RasterImage>);

thread_local! {
    /// Rendered PDF pictures, by file, pixel size and nothing else (the
    /// canvas draws the same page every frame).
    static PDF_PAGES: RefCell<Vec<PdfPage>> = const { RefCell::new(Vec::new()) };
}

/// A PDF picture: its first page rendered to the box's size in pixels, then
/// drawn like an image.
fn draw_pdf(pm: &mut Pixmap, corners: &[Point; 4], flipped: bool, pdf: &Arc<PdfPicture>, opts: &RasterOptions) {
    let t = &opts.transform;
    let view = Rect::new(Point::ZERO, Point::new(f64::from(pm.width()), f64::from(pm.height())));
    if !Rect::from_points(corners.iter().map(|p| t.apply(*p))).intersects(&view) {
        return;
    }
    let [tl, tr, _br, bl] = corners.map(|p| t.apply(p));
    let (ex, ey) = if flipped {
        (bl - tl, tr - tl)
    } else {
        (tr - tl, bl - tl)
    };
    let px = |v: f64| {
        if v.is_finite() {
            v.abs().ceil().clamp(1.0, 16_000.0) as u32
        } else {
            1
        }
    };
    let (w, h) = (px(ex.len()), px(ey.len()));
    let key = (pdf.id(), w, h);
    let cached = PDF_PAGES.with(|c| c.borrow().iter().find(|e| e.0 == key).map(|e| e.1.clone()));
    let img = match cached {
        Some(i) => i,
        None => {
            let Some(i) = pdf.render(w, h) else { return };
            let i = Arc::new(i);
            PDF_PAGES.with(|c| {
                let mut c = c.borrow_mut();
                if c.len() >= 6 {
                    c.remove(0);
                }
                c.push((key, i.clone()));
            });
            i
        }
    };
    if opts.adjust_px <= 0.0 {
        draw_image(pm, corners, &img, flipped, opts);
    } else {
        // Hard edges, as ghostscript draws the embedded picture: almost
        // every pixel the picture's shapes touch is black.
        let mut hard = (*img).clone();
        for px in hard.rgba.as_chunks_mut::<4>().0.iter_mut() {
            px[3] = if px[3] >= HARD_EDGE_MIN_ALPHA { 255 } else { 0 };
        }
        draw_image(pm, corners, &Arc::new(hard), flipped, opts);
    }
}

/// Draw a scene onto a pixmap, antialiased.
pub fn rasterize(scene: &Scene, pm: &mut Pixmap, opts: &RasterOptions) {
    rasterize_aa(scene, pm, opts, true);
}

/// Draw a scene onto a pixmap, antialiased or not (fig2dev's bitmaps
/// without smoothing are not).
pub fn rasterize_aa(scene: &Scene, pm: &mut Pixmap, opts: &RasterOptions, aa: bool) {
    for p in &scene.prims {
        match p {
            Prim::Path { subpaths, fill, stroke } => draw_path(pm, subpaths, fill.as_ref(), stroke.as_ref(), opts, aa),
            Prim::Text(t) => draw_text(pm, t, opts, aa),
            Prim::PictureRef { .. } | Prim::LinkedImage { .. } => {}
            Prim::PdfPicture { corners, flipped, pdf } => draw_pdf(pm, corners, *flipped, pdf, opts),
            Prim::Image {
                corners,
                image,
                flipped,
                ..
            } => draw_image(pm, corners, image, *flipped, opts),
        }
    }
}

/// Render the region `bounds` (Fig units) at `px_per_fig` into a pixmap,
/// with fill patterns in `pattern`.
pub fn render_region(
    scene: &Scene,
    bounds: &Rect,
    px_per_fig: f64,
    background: Option<Rgba>,
    pattern: PatternFrame,
) -> Result<Pixmap, String> {
    if bounds.is_empty() || !(px_per_fig.is_finite() && px_per_fig > 0.0) {
        return Err("nothing to render".into());
    }
    let w = (bounds.width() * px_per_fig).ceil();
    let h = (bounds.height() * px_per_fig).ceil();
    let (w, h) = checked_size(w, h)?;
    render_sized(
        scene,
        bounds,
        px_per_fig,
        (w, h),
        background,
        pattern,
        Smoothing::Antialias,
    )
}

/// A bitmap size in pixels, checked against the export limits.
pub fn checked_size(w: f64, h: f64) -> Result<(u32, u32), String> {
    if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
        return Err("image would be empty".into());
    }
    if w > f64::from(MAX_EDGE) || h > f64::from(MAX_EDGE) || w * h > MAX_PIXELS as f64 {
        return Err(format!("image too large ({w}x{h} pixels); lower the resolution"));
    }
    Ok((w as u32, h as u32))
}

/// Ghostscript with `-dGraphicsAlphaBits=k` draws strokes wider than
/// PostScript says, by `k/(k*k-1)` pixel in all (measured on 1..8 thick
/// lines for k = 2 and 4); in pixels of the `k`-times raster, per side.
fn gs_smooth_adjust(k: u32) -> f64 {
    let k = f64::from(k);
    if k <= 1.0 {
        return 0.0;
    }
    0.5 * k * k / (k * k - 1.0)
}

/// How a bitmap's edges are smoothed (fig2dev's `-S`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Smoothing {
    /// tiny-skia's antialiasing (RustFig's default).
    Antialias,
    /// Hard edges (fig2dev without `-S`, or `-S 1`).
    None,
    /// Hard edges drawn at this many times the resolution, then averaged
    /// (`-S 2`, `-S 4`: ghostscript's `TextAlphaBits`/`GraphicsAlphaBits`).
    Supersample(u32),
}

/// Render the region `bounds` (Fig units) at `px_per_fig` into a pixmap of
/// `size` pixels, smoothed as asked.
pub fn render_sized(
    scene: &Scene,
    bounds: &Rect,
    px_per_fig: f64,
    size: (u32, u32),
    background: Option<Rgba>,
    pattern: PatternFrame,
    smoothing: Smoothing,
) -> Result<Pixmap, String> {
    if bounds.is_empty() || !(px_per_fig.is_finite() && px_per_fig > 0.0) {
        return Err("nothing to render".into());
    }
    let (w, h) = checked_size(f64::from(size.0), f64::from(size.1))?;
    let (k, aa) = match smoothing {
        Smoothing::Antialias => (1, true),
        Smoothing::None => (1, false),
        Smoothing::Supersample(k) => (k.clamp(1, 4), false),
    };
    let (kw, kh) = (w.saturating_mul(k), h.saturating_mul(k));
    if u64::from(kw) * u64::from(kh) > MAX_PIXELS {
        return Err(format!(
            "image too large to smooth ({w}x{h} pixels at {k}x); lower the resolution or the smoothing"
        ));
    }
    let mut pm = Pixmap::new(kw, kh).ok_or("cannot allocate image")?;
    if let Some(bg) = background {
        pm.fill(color(bg));
    }
    let ks = f64::from(k);
    let transform =
        Affine::translate(-bounds.min.x, -bounds.min.y).then(&Affine::scale(px_per_fig * ks, px_per_fig * ks));
    rasterize_aa(
        scene,
        &mut pm,
        &RasterOptions {
            transform,
            min_line_px: 0.5 * ks,
            adjust_px: match smoothing {
                Smoothing::None => GS_FILL_ADJUST,
                Smoothing::Supersample(_) => gs_smooth_adjust(k),
                Smoothing::Antialias => 0.0,
            },
            pattern,
            pattern_k: k,
        },
        aa,
    );
    if k == 1 {
        Ok(pm)
    } else {
        downsample(&pm, k, w, h)
    }
}

/// Average `k`×`k` blocks of premultiplied pixels into a `w`×`h` pixmap.
fn downsample(src: &Pixmap, k: u32, w: u32, h: u32) -> Result<Pixmap, String> {
    let mut out = Pixmap::new(w, h).ok_or("cannot allocate image")?;
    let sw = src.width() as usize;
    let data = src.data();
    let (k, w) = (k as usize, w as usize);
    let n = (k * k) as u32;
    let dst = out.data_mut();
    for (y, row) in dst.chunks_exact_mut(w * 4).enumerate() {
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let mut sum = [0u32; 4];
            for sy in y * k..(y + 1) * k {
                let start = (sy * sw + x * k) * 4;
                let Some(block) = data.get(start..start + k * 4) else {
                    continue;
                };
                for p in block.as_chunks::<4>().0 {
                    for (s, v) in sum.iter_mut().zip(p) {
                        *s += u32::from(*v);
                    }
                }
            }
            for (d, s) in px.iter_mut().zip(sum) {
                // Averages of premultiplied values stay premultiplied.
                *d = ((s + n / 2) / n) as u8;
            }
        }
    }
    Ok(out)
}

/// Un-premultiplied RGBA bytes of a pixmap.
pub fn pixmap_rgba(pm: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.data().len());
    for p in pm.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

/// Encode a pixmap as PNG.
pub fn encode_png(pm: &Pixmap) -> Result<Vec<u8>, String> {
    pm.encode_png().map_err(|e| e.to_string())
}

/// Encode straight (un-premultiplied) RGBA pixels as PNG.
pub fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let size = IntSize::from_wh(width, height).ok_or("empty image")?;
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("image too large")?;
    if rgba.len() != expected {
        return Err("pixel buffer has the wrong size".into());
    }
    let mut data = rgba.to_vec();
    for [r, g, b, a] in data.as_chunks_mut::<4>().0.iter_mut() {
        let al = u16::from(*a);
        *r = ((u16::from(*r) * al + 127) / 255) as u8;
        *g = ((u16::from(*g) * al + 127) / 255) as u8;
        *b = ((u16::from(*b) * al + 127) / 255) as u8;
    }
    let pm = Pixmap::from_vec(data, size).ok_or("cannot build image")?;
    encode_png(&pm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use figbase::color::Rgb;

    #[test]
    fn draws_a_filled_box() {
        let corners = [
            Point::new(0.0, 0.0),
            Point::new(1200.0, 0.0),
            Point::new(1200.0, 1200.0),
            Point::new(0.0, 1200.0),
        ];
        let scene = Scene {
            prims: vec![Prim::Path {
                subpaths: vec![SubPath {
                    pts: corners.to_vec(),
                    closed: true,
                    curve: None,
                }],
                fill: Some(Fill::Solid(Rgba::opaque(Rgb::new(255, 0, 0)))),
                stroke: None,
            }],
        };
        let pm = render_region(
            &scene,
            &Rect::from_points(corners),
            0.05,
            Some(Rgba::opaque(Rgb::WHITE)),
            PatternFrame::screen(),
        )
        .unwrap();
        let mid = pm.pixel(pm.width() / 2, pm.height() / 2).unwrap();
        assert_eq!((mid.red(), mid.green(), mid.blue()), (255, 0, 0));
        assert!(!encode_png(&pm).unwrap().is_empty());
    }

    /// Ghostscript draws a pattern's lines hard at the bitmap's resolution
    /// whatever `-S` says: inside the shape a smoothed bitmap has the same
    /// pixels as the unsmoothed one.
    #[test]
    fn pattern_pixels_are_hard_when_smoothed() {
        let corners = [
            Point::new(0.0, 0.0),
            Point::new(2400.0, 0.0),
            Point::new(2400.0, 2400.0),
            Point::new(0.0, 2400.0),
        ];
        let scene = Scene {
            prims: vec![Prim::Path {
                subpaths: vec![SubPath {
                    pts: corners.to_vec(),
                    closed: true,
                    curve: None,
                }],
                fill: Some(Fill::Pattern {
                    kind: 44,
                    fg: Rgba::opaque(Rgb::new(0, 0, 0)),
                    bg: Rgba::opaque(Rgb::new(255, 0, 0)),
                    nonzero: false,
                }),
                stroke: None,
            }],
        };
        let r = Rect::from_points(corners);
        let render = |sm| {
            render_sized(
                &scene,
                &r,
                0.1,
                (240, 240),
                Some(Rgba::opaque(Rgb::WHITE)),
                PatternFrame::screen(),
                sm,
            )
            .unwrap()
        };
        let hard = render(Smoothing::None);
        for k in [2, 4] {
            let soft = render(Smoothing::Supersample(k));
            let mut same = 0;
            for y in 8..232 {
                for x in 8..232 {
                    let (a, b) = (hard.pixel(x, y).unwrap(), soft.pixel(x, y).unwrap());
                    assert!(
                        matches!((b.red(), b.green(), b.blue()), (255, 0, 0) | (0, 0, 0)),
                        "{x},{y}"
                    );
                    same += usize::from((a.red(), a.green()) == (b.red(), b.green()));
                }
            }
            assert_eq!(same, 224 * 224, "k = {k}");
        }
    }

    #[test]
    fn refuses_huge_images() {
        let scene = Scene::default();
        let r = Rect::new(Point::ZERO, Point::new(1e9, 1e9));
        assert!(render_region(&scene, &r, 1.0, None, PatternFrame::screen()).is_err());
    }
}

#[cfg(test)]
mod miter_tests {
    use super::*;

    #[test]
    fn miter_limit_is_the_postscript_one() {
        // genps.c: `10 setmiterlimit`.
        let s = Stroke {
            color: Rgba::default(),
            width: 1.0,
            dash: Vec::new(),
            cap: CapStyle::Butt,
            join: JoinStyle::Miter,
        };
        assert!((sk_stroke(&s, 1.0, 0.0).miter_limit - 10.0).abs() < f32::EPSILON);
    }
}
