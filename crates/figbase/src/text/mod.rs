//! Fig text: the Fig font tables, font selection, text measurement and
//! glyph outlines.
//!
//! Fonts: each PostScript font is looked up among the installed fonts
//! (`fontdb`; see [`sysfonts`](self) for the candidate families and TeX Gyre
//! search); without a match, or with [`set_builtin_fonts`] /
//! `FIG2DEV_BUILTIN_FONTS=1` (always on wasm32), figbase uses the open fonts
//! shipped in the `epaint_default_fonts` crate and synthesises bold and
//! italic styles.

mod fonts;
mod local;
mod symbols;
mod sysfonts;

pub use fonts::{
    latex_font_name, ps_font_display_name, ps_font_name, ps_font_number, FontRequest, Generic, LATEX_FONT_COUNT,
    PS_FONT_COUNT,
};
pub use local::{find_local_font, parse_comment as parse_local_font_comment, LocalFont};
pub use symbols::{DINGBAT_CHAR, SYMBOL_CHAR};
pub use sysfonts::{add_font_dirs, builtin_fonts, set_builtin_fonts};

use std::sync::OnceLock;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

/// Fig units per point.
pub const FIG_PER_POINT: f64 = 1200.0 / 72.0;
/// Line spacing as a multiple of the font size.
pub const LINE_SPACING: f64 = 1.2;
/// Upper bound on characters laid out at once (longer strings are truncated).
pub const MAX_CHARS: usize = 100_000;

/// A path command in text-local Fig units (origin on the baseline at the
/// left end of the first line, y down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathOp {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// Measured extent of a (possibly multi-line) string, in Fig units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Metrics {
    /// Advance width of the widest line.
    pub width: f64,
    /// Ascent above the first baseline.
    pub ascent: f64,
    /// Descent below the last baseline (positive).
    pub descent: f64,
    /// Number of lines.
    pub lines: usize,
}

/// Laid-out glyph outlines for one string.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shaped {
    /// Outlines, one sub-list per line so callers can justify each line.
    pub lines: Vec<Vec<PathOp>>,
    pub metrics: Metrics,
    /// Per-line advance widths (for justification).
    pub line_widths: Vec<f64>,
    /// The face has no bold variant: the renderer should embolden by stroking.
    pub synthetic_bold: bool,
}

pub(crate) struct LoadedFace {
    pub(crate) face: Face<'static>,
    /// Lower-case family + file name, for matching.
    pub(crate) key: String,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    /// An installed font (not one of the built-in fallbacks).
    pub(crate) system: bool,
}

struct Library {
    faces: Vec<LoadedFace>,
}

fn library() -> &'static Library {
    static LIB: OnceLock<Library> = OnceLock::new();
    LIB.get_or_init(|| {
        let mut faces = Vec::new();
        for (key, bytes) in [
            ("fallback-sans ubuntu", epaint_default_fonts::UBUNTU_LIGHT),
            ("fallback-mono hack", epaint_default_fonts::HACK_REGULAR),
        ] {
            let Ok(face) = Face::parse(bytes, 0) else {
                continue;
            };
            let (bold, italic) = (face.is_bold(), face.is_italic() || face.is_oblique());
            faces.push(LoadedFace {
                face,
                key: key.to_string(),
                bold,
                italic,
                system: false,
            });
        }
        Library { faces }
    })
}

/// Number of usable font faces (the built-in fallbacks).
pub fn face_count() -> usize {
    library().faces.len()
}

/// Pick the best built-in face for a request; returns (face, needs
/// synthetic bold, needs synthetic italic).
fn pick_builtin(req: &FontRequest) -> Option<(&'static LoadedFace, bool, bool)> {
    let lib = library();
    let mut best: Option<(i32, usize)> = None;
    for (i, f) in lib.faces.iter().enumerate() {
        let mut score = 0;
        if req.keywords.iter().any(|k| f.key.contains(k)) {
            score += 100;
        } else if req.generic.keywords().iter().any(|k| f.key.contains(k)) {
            score += 40;
        } else if f.key.contains("fallback") {
            let mono = f.key.contains("mono");
            score += if mono == (req.generic == Generic::Mono) { 20 } else { 5 };
        }
        if f.bold == req.bold {
            score += 10;
        }
        if f.italic == req.italic {
            score += 8;
        }
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, i));
        }
    }
    let (_, i) = best?;
    let f = lib.faces.get(i)?;
    Some((f, req.bold && !f.bold, req.italic && !f.italic))
}

/// The face for a request: an installed one when found, else built-in.
fn select(req: &FontRequest) -> Option<(&'static LoadedFace, bool, bool)> {
    if let Some(p) = sysfonts::pick_system(req) {
        return Some((p.face, p.synth_bold, p.synth_italic));
    }
    pick_builtin(req)
}

/// The glyph of `ch` in `face`; for Symbol-encoded fonts (a (3,0) cmap) the
/// private-use slot 0xF000 + `code`.
fn glyph_of(face: &Face<'_>, ch: char, code: Option<u8>) -> Option<GlyphId> {
    if let Some(g) = face.glyph_index(ch) {
        return Some(g);
    }
    let code = code?;
    let cmap = face.tables().cmap?;
    cmap.subtables
        .into_iter()
        .find_map(|st| st.glyph_index(0xF000 + u32::from(code)))
}

pub(crate) struct Builder<'a> {
    pub(crate) ops: &'a mut Vec<PathOp>,
    pub(crate) scale: f64,
    pub(crate) dx: f64,
    pub(crate) dy: f64,
    pub(crate) skew: f64,
}

impl Builder<'_> {
    fn map(&self, x: f32, y: f32) -> (f64, f64) {
        let (x, y) = (f64::from(x) * self.scale, -f64::from(y) * self.scale);
        (self.dx + x - y * self.skew, self.dy + y)
    }
}

impl OutlineBuilder for Builder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.ops.push(PathOp::Move(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.ops.push(PathOp::Line(x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (a, b) = self.map(x1, y1);
        let (x, y) = self.map(x, y);
        self.ops.push(PathOp::Quad(a, b, x, y));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b) = self.map(x1, y1);
        let (c, d) = self.map(x2, y2);
        let (x, y) = self.map(x, y);
        self.ops.push(PathOp::Cubic(a, b, c, d, x, y));
    }
    fn close(&mut self) {
        self.ops.push(PathOp::Close);
    }
}

fn layout(req: &FontRequest, size_pt: f64, text: &str, outlines: bool) -> Shaped {
    let size = if size_pt.is_finite() {
        size_pt.clamp(0.1, 5000.0)
    } else {
        12.0
    };
    let em = size * FIG_PER_POINT;
    let mut out = Shaped::default();
    let Some((lf, synth_bold, synth_italic)) = select(req) else {
        // No font at all: estimate.
        let widths: Vec<f64> = text.split('\n').map(|l| l.chars().count() as f64 * em * 0.5).collect();
        out.metrics = Metrics {
            width: widths.iter().copied().fold(0.0, f64::max),
            ascent: em * 0.75,
            descent: em * 0.25,
            lines: widths.len(),
        };
        out.line_widths = widths;
        return out;
    };
    let face = &lf.face;
    // Symbol and ZapfDingbats codes are Unicode in an installed font.
    let table: Option<&[u32; 256]> = match (lf.system, req.ps_font) {
        (true, Some(32)) => Some(&SYMBOL_CHAR),
        (true, Some(34)) => Some(&DINGBAT_CHAR),
        _ => None,
    };
    let chain: std::cell::OnceCell<Vec<&'static LoadedFace>> = std::cell::OnceCell::new();
    let ubuntu = library().faces.iter().find(|f| f.key.contains("fallback-sans"));
    let scale = em / f64::from(face.units_per_em().max(1));
    let ascent = f64::from(face.ascender()) * scale;
    let descent = -f64::from(face.descender()) * scale;
    let mut count = 0usize;
    let mut y = 0.0;
    let mut nlines = 0usize;
    for line in text.split('\n') {
        nlines += 1;
        let mut ops = Vec::new();
        let mut x = 0.0;
        let mut prev: Option<GlyphId> = None;
        for ch in line.chars() {
            count += 1;
            if count > MAX_CHARS {
                break;
            }
            let (ch, code) = match (table, u8::try_from(u32::from(ch))) {
                (Some(t), Ok(b)) => match t.get(usize::from(b)).copied().and_then(char::from_u32) {
                    Some(u) if u != '\0' => (u, Some(b)),
                    _ => (ch, Some(b)),
                },
                _ => (ch, None),
            };
            let (f, gid): (&Face<'static>, GlyphId) = match glyph_of(face, ch, code) {
                Some(g) => (face, g),
                None => {
                    let sys = chain.get_or_init(|| {
                        if lf.system {
                            sysfonts::symbol_chain().iter().copied().collect()
                        } else {
                            Vec::new()
                        }
                    });
                    sys.iter()
                        .copied()
                        .chain(ubuntu)
                        .find_map(|fb| fb.face.glyph_index(ch).map(|g| (&fb.face, g)))
                        .unwrap_or((face, GlyphId(0)))
                }
            };
            let fscale = em / f64::from(f.units_per_em().max(1));
            if let (Some(p), true) = (prev, std::ptr::eq(f, face)) {
                if let Some(kern) = face.tables().kern {
                    for st in kern.subtables {
                        if st.horizontal {
                            if let Some(k) = st.glyphs_kerning(p, gid) {
                                x += f64::from(k) * fscale;
                                break;
                            }
                        }
                    }
                }
            }
            if outlines {
                let mut b = Builder {
                    ops: &mut ops,
                    scale: fscale,
                    dx: x,
                    dy: y,
                    skew: if synth_italic { 0.2 } else { 0.0 },
                };
                let _ = f.outline_glyph(gid, &mut b);
            }
            x += f64::from(f.glyph_hor_advance(gid).unwrap_or(0)) * fscale;
            prev = Some(gid);
        }
        if synth_bold {
            x += em * 0.04;
        }
        out.line_widths.push(x);
        out.lines.push(ops);
        y += em * LINE_SPACING;
    }
    let nlines = nlines.max(1);
    out.metrics = Metrics {
        width: out.line_widths.iter().copied().fold(0.0, f64::max),
        ascent,
        descent: descent + (nlines - 1) as f64 * em * LINE_SPACING,
        lines: nlines,
    };
    out.synthetic_bold = synth_bold;
    out
}

/// Measure a string set in the requested font at `size_pt` points.
pub fn measure(req: &FontRequest, size_pt: f64, text: &str) -> Metrics {
    layout(req, size_pt, text, false).metrics
}

/// Glyph outlines of a string. Each line starts at x = 0 and lines are
/// spaced [`LINE_SPACING`] em apart; justification is up to the caller (see
/// [`Shaped::line_widths`]).
pub fn shape(req: &FontRequest, size_pt: f64, text: &str) -> Shaped {
    layout(req, size_pt, text, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_and_shapes() {
        let req = FontRequest::for_fig(16, true);
        let m = measure(&req, 12.0, "Hello");
        assert!(m.width > 0.0 && m.ascent > 0.0);
        let wide = measure(&req, 24.0, "Hello");
        assert!((wide.width / m.width - 2.0).abs() < 0.05);
        let s = shape(&req, 12.0, "Hi\nthere");
        assert!(s.lines.iter().any(|l| !l.is_empty()));
        assert_eq!(s.metrics.lines, 2);
        assert_eq!(s.line_widths.len(), 2);
    }

    #[test]
    fn hostile_inputs() {
        let req = FontRequest::for_fig(9999, true);
        let _ = measure(&req, f64::NAN, "\u{0}\u{10FFFF}");
        let _ = measure(&FontRequest::for_fig(-5, false), -3.0, "");
        let big = "x".repeat(MAX_CHARS + 10);
        let m = measure(&req, 12.0, &big);
        assert!(m.width.is_finite());
    }

    #[test]
    fn fallback_faces_present() {
        assert!(face_count() >= 2);
    }

    #[test]
    fn courier_is_monospaced() {
        let req = FontRequest::for_fig(12, true);
        let a = measure(&req, 12.0, "iiii");
        let b = measure(&req, 12.0, "MMMM");
        assert!((a.width - b.width).abs() < 1.0);
    }
}
