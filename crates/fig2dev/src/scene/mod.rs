//! The display list RustFig's writers draw: a [`Scene`] is a flat,
//! depth-sorted list of drawing primitives (paths with fills and strokes,
//! texts, pictures) in Fig units. The EPS, PDF and SVG writers, the
//! rasteriser behind the bitmap languages and RustFig's canvas all draw a
//! scene, so what RustFig shows is what fig2dev writes.
//!
//! [`build`] makes a scene from fig2dev's objects (as `read.c` returns
//! them); RustFig builds its canvas scene from its own document
//! (`rustfig-render`) with the helpers here ([`shape_prims`],
//! [`picture_prims`], [`fill`], [`stroke`]).
//
// Ported from fig2dev dev/genps.c (`PSFONTMAG` text sizes, `genps_grid` in grid.rs, arrows on pie
// arcs) and colors.c (`rgb2luminance`) (xfig licence, see LICENSE-xfig).

mod afm;
pub mod bitmaps;
pub mod build;
pub mod eps;
pub mod grid;
pub mod latex_box;
mod localfont;
pub mod output;
pub mod page;
pub mod pattern;
pub mod pdf;
pub mod preview;
mod ps_cmap;
pub mod raster;
pub mod sld;
pub mod svg;
mod textcache;

pub use build::build;
pub use output::{export_bounds, write, EmbeddedEps, EmbeddedJpeg, ExportFormat, Page, CREATOR};
pub use page::{PageInput, PageLayout};

use figbase::color::{area_fill_color, Rgb};
use figbase::geom::{arrowhead, ArrowFill, ArrowShape, ArrowSpec, Point, Rect, Seg, FIG_PER_POINT};
use figbase::image::RasterImage;
use figbase::style::{CapStyle, JoinStyle, Justify, LineKind};
use std::sync::Arc;

/// A colour with alpha.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn opaque(c: Rgb) -> Rgba {
        Rgba {
            r: c.r,
            g: c.g,
            b: c.b,
            a: 255,
        }
    }
    pub fn rgb(self) -> Rgb {
        Rgb::new(self.r, self.g, self.b)
    }
}

/// How a closed path is filled.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    Solid(Rgba),
    /// Fig pattern 41–62: `fg` lines over a `bg` background.
    Pattern {
        kind: i32,
        fg: Rgba,
        bg: Rgba,
        /// Fill (and clip) with the nonzero rule, as genps does; else the
        /// even-odd rule.
        nonzero: bool,
    },
}

/// How a path is stroked. Lengths are Fig units.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub width: f64,
    pub color: Rgba,
    pub dash: Vec<f64>,
    pub cap: CapStyle,
    pub join: JoinStyle,
}

/// One polyline of a path.
#[derive(Debug, Clone, PartialEq)]
pub struct SubPath {
    pub pts: Vec<Point>,
    pub closed: bool,
    /// The exact outline as lines and curves, for the vector backends
    /// (ellipses, arcs, rounded boxes); `pts` is its flattening, which the
    /// raster backend draws.
    pub curve: Option<Vec<Seg>>,
}

/// A drawing primitive in Fig coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    Path {
        subpaths: Vec<SubPath>,
        fill: Option<Fill>,
        stroke: Option<Stroke>,
    },
    Text(TextPrim),
    Image {
        /// Top-left, top-right, bottom-right, bottom-left of the picture box.
        corners: [Point; 4],
        image: Arc<RasterImage>,
        /// Transposed (Fig "flipped" pictures).
        flipped: bool,
        /// A JPEG file the PostScript writers pass through instead of the
        /// pixels: its name in the export's embedded pictures.
        dct: Option<String>,
    },
    /// A PDF page as a vector picture (a PDF file, or an EPS file ghostscript
    /// turned into one): the PDF writer embeds it, the bitmap writers and
    /// the canvas render it. The PostScript writers get it as an
    /// [`EmbeddedEps`] converted by `util::pdfps` instead.
    PdfPicture {
        /// Top-left, top-right, bottom-right, bottom-left of the picture box.
        corners: [Point; 4],
        /// Transposed (Fig "flipped" pictures).
        flipped: bool,
        pdf: Arc<crate::util::pdfpic::PdfPicture>,
    },
    /// A picture without pixels (an EPS or PDF file, or one that could
    /// not be read). The placeholder drawn for it follows as the next
    /// primitive; the EPS backend embeds the file instead when the export
    /// supplies it ([`EmbeddedEps`]).
    PictureRef {
        /// Top-left, top-right, bottom-right, bottom-left of the picture box.
        corners: [Point; 4],
        flipped: bool,
        /// The picture's file name, as in the figure.
        file: String,
    },
    /// A picture as gensvg writes it: a link to the file (it is not read),
    /// positioned by the box's first and opposite corners (`link_pictures`).
    LinkedImage {
        file: String,
        /// The box's first point and its third (the opposite corner), rounded
        /// to integers as gensvg prints them.
        p1: (i32, i32),
        p3: (i32, i32),
        flipped: bool,
    },
}

/// A text run.
#[derive(Debug, Clone, PartialEq)]
pub struct TextPrim {
    pub origin: Point,
    /// Radians, counter-clockwise.
    pub angle: f64,
    /// Points.
    pub size: f64,
    pub justify: Justify,
    pub font: i32,
    pub postscript: bool,
    pub text: String,
    pub color: Rgba,
    /// The figure's special-text flag (`^`, `_` and `$` mean something to
    /// the SVG writer).
    pub special: bool,
    /// An installed font named by the text's `rustfig-font:` comment (found
    /// installed; `font` is still the Fig number that is the fallback).
    pub local_font: Option<String>,
}

impl TextPrim {
    /// The installed face of [`TextPrim::local_font`].
    pub fn local(&self) -> Option<Arc<figbase::text::LocalFont>> {
        figbase::text::find_local_font(self.local_font.as_deref()?)
    }

    pub fn request(&self) -> figbase::text::FontRequest {
        figbase::text::FontRequest::for_fig(self.font, self.postscript)
    }
}

impl Rgba {
    /// The light gray xfig uses for depths that are off: darker colours
    /// stay a little darker, so shapes remain readable.
    pub fn grayed(self) -> Rgba {
        let lum = (u32::from(self.r) * 299 + u32::from(self.g) * 587 + u32::from(self.b) * 114) / 1000;
        let v = (170 + lum * 70 / 255).min(255) as u8;
        Rgba {
            r: v,
            g: v,
            b: v,
            a: self.a,
        }
    }
}

impl Rgba {
    /// The gray of fig2dev's `-N` (`colors.c` `rgb2luminance`).
    pub fn luminance_gray(self) -> Rgba {
        let lum = 0.3 * f64::from(self.r) + 0.59 * f64::from(self.g) + 0.11 * f64::from(self.b);
        let v = lum.round().clamp(0.0, 255.0) as u8;
        Rgba {
            r: v,
            g: v,
            b: v,
            a: self.a,
        }
    }
}

impl Scene {
    /// Render the PDF pictures (EPS pictures drawn by ghostscript) in grey
    /// levels, for `-N`.
    pub fn gray_pictures(&mut self) {
        for p in &mut self.prims {
            if let Prim::PdfPicture { pdf, .. } = p {
                *pdf = std::sync::Arc::new(pdf.grayed());
            }
        }
    }

    /// Every stroke, fill and text colour passed through `f`. Pictures keep
    /// theirs, as in fig2dev.
    pub fn map_colors(&mut self, f: impl Fn(Rgba) -> Rgba) {
        for p in &mut self.prims {
            match p {
                Prim::Path { fill, stroke, .. } => {
                    match fill {
                        Some(Fill::Solid(c)) => *c = f(*c),
                        Some(Fill::Pattern { fg, bg, .. }) => {
                            *fg = f(*fg);
                            *bg = f(*bg);
                        }
                        None => {}
                    }
                    if let Some(s) = stroke {
                        s.color = f(s.color);
                    }
                }
                Prim::Text(t) => t.color = f(t.color),
                Prim::Image { .. } | Prim::PdfPicture { .. } | Prim::PictureRef { .. } | Prim::LinkedImage { .. } => {}
            }
        }
    }
}

impl Prim {
    /// Recolour in gray (objects on depths that are off).
    pub fn gray_out(&mut self) {
        match self {
            Prim::Path { fill, stroke, .. } => {
                match fill {
                    Some(Fill::Solid(c)) => *c = c.grayed(),
                    Some(Fill::Pattern { fg, bg, .. }) => {
                        *fg = fg.grayed();
                        *bg = bg.grayed();
                    }
                    None => {}
                }
                if let Some(s) = stroke {
                    s.color = s.color.grayed();
                }
            }
            Prim::Text(t) => t.color = t.color.grayed(),
            // Pictures keep their pixels; they can't be picked either way.
            Prim::Image { .. } | Prim::PdfPicture { .. } | Prim::PictureRef { .. } | Prim::LinkedImage { .. } => {}
        }
    }

    /// Bounding box in Fig units (including half the stroke width).
    pub fn bbox(&self) -> Rect {
        match self {
            Prim::Path { subpaths, stroke, .. } => {
                let mut r = Rect::EMPTY;
                for s in subpaths {
                    for p in &s.pts {
                        r.add_point(*p);
                    }
                }
                r.expand(stroke.as_ref().map(|s| s.width / 2.0).unwrap_or(0.0))
            }
            Prim::Text(t) => {
                let m = textcache::metrics(t);
                let x0 = match t.justify {
                    Justify::Left => 0.0,
                    Justify::Center => -m.width / 2.0,
                    Justify::Right => -m.width,
                };
                let rot = figbase::geom::Affine::rotate(t.angle);
                Rect::from_points(
                    [
                        Point::new(x0, -m.ascent),
                        Point::new(x0 + m.width, -m.ascent),
                        Point::new(x0 + m.width, m.descent),
                        Point::new(x0, m.descent),
                    ]
                    .map(|p| t.origin + rot.apply_vec(p)),
                )
            }
            Prim::Image { corners, .. } | Prim::PdfPicture { corners, .. } | Prim::PictureRef { corners, .. } => {
                Rect::from_points(corners.iter().copied())
            }
            Prim::LinkedImage { p1, p3, .. } => {
                Rect::from_points([p1, p3].map(|&(x, y)| Point::new(f64::from(x), f64::from(y))))
            }
        }
    }
}

/// A depth-sorted list of primitives.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    pub prims: Vec<Prim>,
}

impl Scene {
    pub fn bbox(&self) -> Rect {
        self.prims.iter().fold(Rect::EMPTY, |r, p| r.union(&p.bbox()))
    }
}

/// Stroke width in Fig units for a thickness in 1/80 inch (fig2dev's rule,
/// [`figbase::geom::stroke_width`]).
pub fn line_width(thickness: f64) -> f64 {
    figbase::geom::stroke_width(thickness)
}

/// Font size in Fig units for a size in points.
pub fn font_px(size: f64) -> f64 {
    size * FIG_PER_POINT
}

/// The fill of an object: `area_fill` (−1 none, 0–40 shades and tints,
/// 41–62 patterns in the pen colour over the fill colour), the fill colour
/// and whether it is the default or black, or white.
pub fn fill(area_fill: i32, pen: Rgba, fill: Rgb, default_or_black: bool, white: bool) -> Option<Fill> {
    if area_fill < 0 {
        return None;
    }
    if area_fill > 40 {
        return Some(Fill::Pattern {
            kind: area_fill,
            fg: pen,
            bg: Rgba::opaque(fill),
            nonzero: false,
        });
    }
    area_fill_color(fill, default_or_black, white, area_fill).map(|c| Fill::Solid(Rgba::opaque(c)))
}

/// The stroke of an object with line `thickness` in 1/80 inch. `round_dots`:
/// fig2dev draws dotted ellipses and closed splines with round caps (dots);
/// other objects use their cap style.
pub fn stroke(
    thickness: i32,
    line: LineKind,
    style_val: f64,
    pen: Rgba,
    cap: CapStyle,
    join: JoinStyle,
    round_dots: bool,
) -> Option<Stroke> {
    if thickness <= 0 {
        return None;
    }
    let width = line_width(f64::from(thickness));
    Some(Stroke {
        width,
        color: pen,
        // genps `set_style` writes no dash for a style value of 0 or less
        // (gensvg's "0 0" array draws solid too), whatever the line style.
        dash: if style_val > 0.0 {
            line.dash_pattern(style_val, width)
        } else {
            Vec::new()
        },
        cap: if round_dots && line == LineKind::Dotted {
            CapStyle::Round
        } else {
            cap
        },
        join,
    })
}

/// A point at least `min` away from the end of `pts`, walking backwards, to
/// orient arrowheads.
fn arrow_from(pts: &[Point], tip: Point, min: f64) -> Option<Point> {
    let mut last = None;
    for p in pts.iter().rev() {
        if p.dist(tip) > 1e-6 {
            last = Some(*p);
            if p.dist(tip) >= min {
                break;
            }
        }
    }
    last
}

/// The arrowhead at the end of `pts`. Polylines point the head along their
/// last segment, as fig2dev does; curves along the chord of the head's length.
pub fn head_at_end(pts: &[Point], a: &ArrowSpec, line_thickness: f64, poly: bool) -> Option<ArrowShape> {
    let tip = *pts.last()?;
    let from = if poly {
        pts.iter().rev().find(|p| p.dist(tip) > 1e-6).copied()?
    } else {
        arrow_from(pts, tip, a.height)?
    };
    arrowhead(tip, from, a, line_thickness)
}

fn arrow_prims(pen: Rgba, a: &ArrowSpec, shape: ArrowShape, out: &mut Vec<Prim>) {
    let fill = match shape.fill {
        ArrowFill::None => None,
        ArrowFill::White => Some(Fill::Solid(Rgba::opaque(Rgb::WHITE))),
        ArrowFill::Pen => Some(Fill::Solid(pen)),
    };
    let stroke = Stroke {
        width: line_width(a.thickness).max(1.0),
        color: pen,
        dash: Vec::new(),
        cap: CapStyle::Butt,
        join: JoinStyle::Miter,
    };
    out.push(Prim::Path {
        subpaths: vec![SubPath {
            pts: shape.outline,
            closed: shape.closed,
            curve: None,
        }],
        fill,
        stroke: Some(stroke.clone()),
    });
    if let Some(part) = shape.pen_part {
        out.push(Prim::Path {
            subpaths: vec![SubPath {
                pts: part,
                closed: true,
                curve: None,
            }],
            fill: Some(Fill::Solid(pen)),
            stroke: Some(stroke),
        });
    }
}

/// A flattened line object (polyline, box, polygon, spline, arc, ellipse)
/// ready to draw.
#[derive(Debug, Clone, Default)]
pub struct Shape {
    /// The flattened outlines and whether each is closed; the first carries
    /// the arrowheads.
    pub paths: Vec<(Vec<Point>, bool)>,
    /// The exact outline as lines and curves (ellipses, arcs, rounded
    /// boxes), for the vector writers.
    pub exact: Option<Vec<Seg>>,
    pub fill: Option<Fill>,
    pub stroke: Option<Stroke>,
    /// The pen colour, for the arrowheads.
    pub pen: Rgba,
    /// Line thickness in 1/80 inch.
    pub thickness: f64,
    pub forward_arrow: Option<ArrowSpec>,
    pub backward_arrow: Option<ArrowSpec>,
    /// A polyline: arrowheads point along the last segment.
    pub poly: bool,
    /// A pie-wedge arc: its arrowheads sit on the open arc, given here
    /// (fig2dev draws them).
    pub pie_arc: Option<Vec<Point>>,
}

/// The primitives of a line object: its path, shortened where arrowheads
/// sit, then the arrowheads.
pub fn shape_prims(shape: Shape, out: &mut Vec<Prim>) {
    let Shape {
        mut paths,
        exact,
        fill,
        stroke,
        pen,
        thickness,
        forward_arrow: fwd,
        backward_arrow: back,
        poly,
        pie_arc,
    } = shape;
    let pie = pie_arc.is_some();
    let mut heads = Vec::new();
    if let Some((pts, closed)) = paths.first_mut() {
        let open = !*closed && pts.len() >= 2;
        let src = if open { Some(pts.clone()) } else { pie_arc };
        if let Some(src) = src.filter(|s| s.len() >= 2) {
            if let Some(a) = fwd {
                if let Some(sh) = head_at_end(&src, &a, thickness, poly) {
                    if let Some(l) = pts.last_mut().filter(|_| open) {
                        *l = sh.line_end;
                    }
                    heads.push((a, sh));
                }
            }
            if let Some(a) = back {
                let rev: Vec<Point> = src.iter().rev().copied().collect();
                if let Some(sh) = head_at_end(&rev, &a, thickness, poly) {
                    if let Some(f) = pts.first_mut().filter(|_| open) {
                        *f = sh.line_end;
                    }
                    heads.push((a, sh));
                }
            }
        }
    }
    let closed_any = paths.iter().any(|p| p.1);
    // The exact outline for the vector backends, unless arrowheads
    // shortened the line (then the flattened, shortened one is drawn).
    let shortened = !heads.is_empty() && !pie;
    let mut exact = if shortened { None } else { exact };
    let subpaths: Vec<SubPath> = paths
        .into_iter()
        .filter(|p| !p.0.is_empty())
        .map(|(mut pts, closed)| {
            if pts.len() == 1 {
                // A single point: a dot.
                if let Some(q) = pts.first().copied() {
                    pts.push(q + Point::new(0.01, 0.0));
                }
            }
            SubPath {
                pts,
                closed,
                curve: exact.take(),
            }
        })
        .collect();
    let single_dot = subpaths.iter().all(|s| match s.pts.as_slice() {
        [a, b] => a.dist(*b) < 0.1,
        _ => false,
    });
    let stroke = stroke.map(|mut s| {
        if single_dot {
            s.cap = CapStyle::Round;
        }
        s
    });
    // Open shapes can still be filled (xfig fills open polylines and splines).
    let fill = if closed_any || subpaths.iter().any(|s| s.pts.len() > 2) {
        fill
    } else {
        None
    };
    out.push(Prim::Path { subpaths, fill, stroke });
    for (a, sh) in heads {
        arrow_prims(pen, &a, sh, out);
    }
}

/// The primitives of a picture in the box `corners` (top-left, top-right,
/// bottom-right, bottom-left): its pixels, or a reference to the file and
/// a crossed box when it has none; then its frame (`frame`: the picture
/// object's own stroke).
pub fn picture_prims(
    corners: [Point; 4],
    image: Option<Arc<RasterImage>>,
    flipped: bool,
    file: &str,
    frame: Option<Stroke>,
    out: &mut Vec<Prim>,
) {
    picture_prims_dct(corners, image, flipped, false, file, frame, out);
}

/// [`picture_prims`] for a picture the PostScript writers may pass through
/// as it is (`dct`: a JPEG in the export's embedded pictures under `file`).
pub fn picture_prims_dct(
    corners: [Point; 4],
    image: Option<Arc<RasterImage>>,
    flipped: bool,
    dct: bool,
    file: &str,
    frame: Option<Stroke>,
    out: &mut Vec<Prim>,
) {
    match image {
        Some(image) => out.push(Prim::Image {
            corners,
            image,
            flipped,
            dct: dct.then(|| file.to_string()),
        }),
        None => {
            out.push(Prim::PictureRef {
                corners,
                flipped,
                file: file.to_string(),
            });
            // Missing picture: box with a cross.
            let [a, b, c, d] = corners;
            out.push(Prim::Path {
                subpaths: vec![
                    SubPath {
                        pts: corners.to_vec(),
                        closed: true,
                        curve: None,
                    },
                    SubPath {
                        pts: vec![a, c],
                        closed: false,
                        curve: None,
                    },
                    SubPath {
                        pts: vec![b, d],
                        closed: false,
                        curve: None,
                    },
                ],
                fill: None,
                stroke: Some(Stroke {
                    width: line_width(1.0),
                    color: Rgba::opaque(Rgb::new(128, 128, 128)),
                    dash: Vec::new(),
                    cap: CapStyle::Butt,
                    join: JoinStyle::Miter,
                }),
            });
        }
    }
    if let Some(stroke) = frame {
        out.push(Prim::Path {
            subpaths: vec![SubPath {
                pts: corners.to_vec(),
                closed: true,
                curve: None,
            }],
            fill: None,
            stroke: Some(stroke),
        });
    }
}

#[cfg(test)]
mod dash_tests {
    use super::*;

    #[test]
    fn zero_style_value_draws_solid() {
        let pen = Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        let st = |v| stroke(1, LineKind::Dashed, v, pen, CapStyle::Butt, JoinStyle::Miter, false);
        assert!(st(0.0).is_some_and(|s| s.dash.is_empty()));
        assert!(st(4.0).is_some_and(|s| !s.dash.is_empty()));
    }
}
