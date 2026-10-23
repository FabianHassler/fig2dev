//! Encapsulated PostScript backend.
//!
//! EPS pictures the export supplies ([`EmbeddedEps`]) are embedded as
//! genps.c embeds them: scaled into the picture box, clipped to their
//! bounding box, `showpage` and `setpagedevice` disabled.
//
// The picture embedding is ported from fig2dev dev/genps.c (`genps_line`,
// picture branch) and dev/readeps.c (`read_eps`); the page structure of
// `to_ps` from `genps_start` and `genps_end` (xfig licence, see
// LICENSE-xfig). The UTF-8 text path (CMap, `composefont`, `-UTF-8` fonts)
// is ported from genps.c (`note_text_needing_cmap`, `genps_text`).

use super::localfont::{sfnts_strings, GlyphSet};
use super::output::{Meta, CREATOR};
use super::pattern::{coverage, PatternFrame};
use super::ps_cmap::{XFIG_CMAP1, XFIG_CMAP2, XFIG_CMAP3};
use super::{EmbeddedEps, PageLayout};
use super::{Fill, Prim, Rgba, Scene, Stroke, SubPath, TextPrim};
use crate::util::readjpg;
use figbase::geom::{Point, Rect, Seg};
use figbase::style::{CapStyle, JoinStyle, Justify};
use std::fmt::Write as _;

struct Ctx {
    out: String,
    /// Fig units → points.
    k: f64,
    min_x: f64,
    max_y: f64,
    /// The creation date, for the DSC header of an embedded JPEG.
    date: Option<String>,
    /// Subsets of installed TrueType fonts (Type 42), in the order of
    /// first use; at most 255 glyphs each.
    lf: Vec<GlyphSet>,
    /// Installed fonts drawn as outlines (their subset can't be written).
    lf_outline: std::collections::BTreeSet<String>,
}

/// The glyph sets of the installed fonts a scene uses, and the fonts whose
/// subset can't be made into a Type 42 font (drawn as outlines instead).
fn plan_local_fonts(scene: &Scene) -> (Vec<GlyphSet>, std::collections::BTreeSet<String>) {
    let mut sets: Vec<GlyphSet> = Vec::new();
    let mut outline = std::collections::BTreeSet::new();
    for p in &scene.prims {
        let Prim::Text(t) = p else { continue };
        let Some(font) = t.local() else { continue };
        if font.cff || !font.embeddable {
            continue;
        }
        for ch in t.text.chars().filter(|&ch| ch != '\n') {
            let have = sets
                .iter()
                .any(|g| g.font.name == font.name && g.used.iter().any(|u| u.1 == ch));
            if have {
                continue;
            }
            let idx = match sets.iter().position(|g| g.font.name == font.name && g.used.len() < 255) {
                Some(i) => i,
                None => {
                    sets.push(GlyphSet::new(font.clone()));
                    sets.len() - 1
                }
            };
            if let Some(set) = sets.get_mut(idx) {
                set.code(ch, 255);
            }
        }
    }
    for set in &sets {
        let ok = set.subset().is_some_and(|d| sfnts_strings(&d).is_some());
        if !ok {
            outline.insert(set.font.name.clone());
        }
    }
    sets.retain(|g| !outline.contains(&g.font.name));
    (sets, outline)
}

/// The Type 42 fonts of the subsets, in the prolog.
fn type42_fonts(out: &mut String, sets: &[GlyphSet]) {
    for (i, set) in sets.iter().enumerate() {
        let Some(data) = set.subset() else { continue };
        let Some(strings) = sfnts_strings(&data) else { continue };
        let (upm, bbox) = match set.font.face() {
            Some(f) => {
                let b = f.global_bounding_box();
                (
                    f64::from(f.units_per_em().max(1)),
                    [
                        f64::from(b.x_min),
                        f64::from(b.y_min),
                        f64::from(b.x_max),
                        f64::from(b.y_max),
                    ],
                )
            }
            None => (1000.0, [0.0, -200.0, 1000.0, 900.0]),
        };
        let _ = writeln!(out, "%%BeginResource: font RFLocal{i}");
        let _ = writeln!(
            out,
            "11 dict begin\n/FontName /RFLocal{i} def\n/FontType 42 def\n/FontMatrix [1 0 0 1 0 0] def\n/PaintType 0 def\n/FontBBox [{} {} {} {}] def",
            bbox[0] / upm,
            bbox[1] / upm,
            bbox[2] / upm,
            bbox[3] / upm
        );
        out.push_str("/Encoding 256 array 0 1 255 {1 index exch /.notdef put} for\n");
        for k in 1..=set.used.len() {
            let _ = writeln!(out, "dup {k} /g{k} put");
        }
        out.push_str("def\n/CharStrings ");
        let _ = writeln!(out, "{} dict dup begin\n/.notdef 0 def", set.used.len() + 1);
        for (k, (gid, _, _)) in set.used.iter().enumerate() {
            let _ = writeln!(out, "/g{} {} def", k + 1, gid);
        }
        out.push_str("end readonly def\n/sfnts [\n");
        for piece in strings {
            out.push('<');
            let mut col = 0;
            for b in piece {
                let _ = write!(out, "{b:02x}");
                col += 2;
                if col >= 64 {
                    out.push('\n');
                    col = 0;
                }
            }
            // Strings of a Type 42 font have an even length.
            if piece.len() % 2 == 1 {
                out.push_str("00");
            }
            out.push_str(">\n");
        }
        let _ = writeln!(
            out,
            "] def\ncurrentdict end /RFLocal{i} exch definefont pop\n%%EndResource"
        );
    }
}

fn n(v: f64) -> String {
    if v.is_finite() {
        let r = (v * 1000.0).round() / 1000.0;
        format!("{r}")
    } else {
        "0".into()
    }
}

impl Ctx {
    /// The part and code of `ch` in installed font `font`.
    fn lf_code(&self, font: &figbase::text::LocalFont, ch: char) -> Option<(usize, u8)> {
        for (i, g) in self.lf.iter().enumerate() {
            if g.font.name == font.name {
                if let Some(k) = g.used.iter().position(|u| u.1 == ch) {
                    return Some((i, u8::try_from(k + 1).ok()?));
                }
            }
        }
        None
    }

    fn p(&self, p: Point) -> (f64, f64) {
        ((p.x - self.min_x) * self.k, (self.max_y - p.y) * self.k)
    }

    fn path(&mut self, subs: &[SubPath]) {
        self.out.push_str("newpath\n");
        for s in subs {
            if let Some(curve) = &s.curve {
                for seg in curve {
                    let _ = match *seg {
                        Seg::Move(q) => {
                            let (x, y) = self.p(q);
                            writeln!(self.out, "{} {} m", n(x), n(y))
                        }
                        Seg::Line(q) => {
                            let (x, y) = self.p(q);
                            writeln!(self.out, "{} {} l", n(x), n(y))
                        }
                        Seg::Cubic(a, b, q) => {
                            let ((x1, y1), (x2, y2), (x3, y3)) = (self.p(a), self.p(b), self.p(q));
                            writeln!(
                                self.out,
                                "{} {} {} {} {} {} curveto",
                                n(x1),
                                n(y1),
                                n(x2),
                                n(y2),
                                n(x3),
                                n(y3)
                            )
                        }
                    };
                }
            } else {
                for (i, q) in s.pts.iter().enumerate() {
                    let (x, y) = self.p(*q);
                    let _ = writeln!(self.out, "{} {} {}", n(x), n(y), if i == 0 { "m" } else { "l" });
                }
            }
            if s.closed && !s.pts.is_empty() {
                self.out.push_str("closepath\n");
            }
        }
    }

    fn rgb(&mut self, c: Rgba) {
        let _ = writeln!(
            self.out,
            "{} {} {} setrgbcolor",
            n(f64::from(c.r) / 255.0),
            n(f64::from(c.g) / 255.0),
            n(f64::from(c.b) / 255.0)
        );
    }

    fn stroke(&mut self, s: &Stroke) {
        self.rgb(s.color);
        let _ = writeln!(self.out, "{} setlinewidth", n(s.width * self.k));
        let d: Vec<String> = s.dash.iter().map(|v| n((v * self.k).max(0.001))).collect();
        let _ = writeln!(self.out, "[{}] 0 setdash", d.join(" "));
        let _ = writeln!(
            self.out,
            "{} setlinecap {} setlinejoin",
            match s.cap {
                CapStyle::Butt => 0,
                CapStyle::Round => 1,
                CapStyle::Projecting => 2,
            },
            match s.join {
                JoinStyle::Miter => 0,
                JoinStyle::Round => 1,
                JoinStyle::Bevel => 2,
            }
        );
        self.out.push_str("stroke\n");
    }
}

/// A PostScript string literal (Latin-1; other characters become '?').
pub(crate) fn ps_string(s: &str) -> String {
    let mut o = String::from("(");
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                o.push('\\');
                o.push(c);
            }
            c if (' '..='~').contains(&c) => o.push(c),
            c if (c as u32) >= 0xa0 && (c as u32) <= 0xff => {
                let _ = write!(o, "\\{:03o}", c as u32);
            }
            _ => o.push('?'),
        }
    }
    o.push(')');
    o
}

/// The PostScript font a text is set in.
fn font_name(t: &TextPrim) -> &'static str {
    if t.postscript {
        figbase::text::ps_font_name(t.font)
    } else {
        match t.font {
            2 => "Times-Bold",
            3 => "Times-Italic",
            4 => "Helvetica",
            5 => "Courier",
            _ => "Times-Roman",
        }
    }
}

/// Whether a text goes through the `-UTF-8` composite font: it has a
/// character beyond Latin-1 and its font is not Symbol or ZapfDingbats
/// (those get the bytes as they are, see `ps_string_bytes`). (genps does so for any non-ASCII text; Latin-1 text stays on the
/// ISO-Latin-1 fonts here.)
fn needs_utf8(t: &TextPrim) -> bool {
    !matches!(font_name(t), "Symbol" | "ZapfDingbats") && t.text.chars().any(|c| u32::from(c) > 0xff)
}

/// The fonts that need a `-UTF-8` composite font (`note_text_needing_cmap`).
fn utf8_fonts(scene: &Scene) -> std::collections::BTreeSet<&'static str> {
    scene
        .prims
        .iter()
        .filter_map(|p| match p {
            Prim::Text(t) if needs_utf8(t) => Some(font_name(t)),
            _ => None,
        })
        .collect()
}

/// `%%Title`, `%%Creator` and `%%CreationDate` (genps_start).
fn dsc_head(out: &mut String, meta: &Meta<'_>) {
    let _ = writeln!(out, "%%Title: {}", meta.title.replace(['\r', '\n'], " "));
    let _ = writeln!(out, "%%Creator: {CREATOR}");
    if let Some(d) = &meta.date {
        let _ = writeln!(out, "%%CreationDate: {}", d.iso());
    }
}

/// The whole-figure comments, between `%` lines, before the prolog
/// (genps_start's `print_comments("% ", ...)`).
fn figure_comments(out: &mut String, meta: &Meta<'_>) {
    if meta.comments.is_empty() {
        return;
    }
    out.push_str("%\n");
    for c in meta.comments {
        let _ = writeln!(out, "%  {c} ");
    }
    out.push_str("%\n");
}

/// DSC resource comments, as genps writes them before `%%EndComments`
/// for any text beyond ASCII (`note_text_needing_cmap`; genps also sets
/// them where this writer keeps Latin-1 text on the ISO-Latin-1 fonts, so
/// the comment can name a CMap the prolog doesn't carry).
fn utf8_comments(out: &mut String, scene: &Scene) {
    let non_ascii = scene.prims.iter().any(|p| match p {
        Prim::Text(t) => !t.text.is_ascii() && !matches!(font_name(t), "Symbol" | "ZapfDingbats"),
        _ => false,
    });
    if non_ascii {
        out.push_str("%%DocumentNeededResources: ProcSet (CIDInit)\n%%+ CMap (Xfig-UTF8-H)\n");
        out.push_str("%%DocumentSuppliedResources: CMap (Xfig-UTF8-H)\n");
    }
}

/// The CMap, at the start of the prolog.
fn utf8_cmap(out: &mut String, fonts: &std::collections::BTreeSet<&'static str>) {
    if !fonts.is_empty() {
        out.push_str(XFIG_CMAP1);
        out.push_str(XFIG_CMAP2);
        out.push_str(XFIG_CMAP3);
    }
}

/// The composite fonts, in the prolog.
fn utf8_composefonts(out: &mut String, fonts: &std::collections::BTreeSet<&'static str>) {
    for f in fonts {
        let _ = writeln!(
            out,
            "/{f}-UTF-8\n/Xfig-AGL-UTF8-H /CMap findresource\n[/{f} ] composefont pop"
        );
    }
}

/// A PostScript string literal for UTF-8 text, as genps writes it: only
/// `(`, `)` and `\` are escaped.
fn ps_string_utf8(s: &str) -> String {
    let mut o = String::from("(");
    for c in s.chars() {
        if matches!(c, '(' | ')' | '\\') {
            o.push('\\');
        }
        o.push(c);
    }
    o.push(')');
    o
}

/// The UTF-8 bytes of a text as a string literal for a font without the
/// CMap (Symbol, ZapfDingbats): genps writes them as they are, so the font
/// shows what its own encoding makes of each byte.
fn ps_string_bytes(s: &str) -> String {
    let mut o = String::from("(");
    for b in s.bytes() {
        match b {
            b'(' | b')' | b'\\' => {
                o.push('\\');
                o.push(char::from(b));
            }
            b' '..=b'~' => o.push(char::from(b)),
            _ => {
                let _ = write!(o, "\\{b:03o}");
            }
        }
    }
    o.push(')');
    o
}

/// Text in an installed TrueType font: a Type 42 font of the subset
/// (defined in the prolog; see [`type42_fonts`]).
fn local_text(c: &mut Ctx, t: &TextPrim) -> bool {
    let Some(font) = t.local() else {
        return false;
    };
    if font.cff || !font.embeddable || c.lf_outline.contains(&font.name) {
        // Outlines: CFF fonts and fonts that may not be embedded.
        outline_text(c, t);
        return true;
    }
    let size = super::font_px(t.size) * c.k;
    let (x, y) = c.p(t.origin);
    c.rgb(t.color);
    let shaped = super::textcache::shaped(t);
    for (i, line) in t.text.split('\n').enumerate() {
        // The glyphs of one line may sit in several parts of 255 glyphs.
        let mut parts: Vec<(usize, Vec<u8>)> = Vec::new();
        for ch in line.chars() {
            if let Some((p, code)) = c.lf_code(&font, ch) {
                match parts.last_mut() {
                    Some((q, bytes)) if *q == p => bytes.push(code),
                    _ => parts.push((p, vec![code])),
                }
            }
        }
        let w = shaped.line_widths.get(i).copied().unwrap_or(0.0) * c.k;
        let dx = match t.justify {
            Justify::Left => 0.0,
            Justify::Center => -w / 2.0,
            Justify::Right => -w,
        };
        let dy = -(i as f64) * size * figbase::text::LINE_SPACING;
        let _ = write!(
            c.out,
            "gsave {} {} translate {} rotate {} {} moveto ",
            n(x),
            n(y),
            n(t.angle.to_degrees()),
            n(dx),
            n(dy)
        );
        for (p, bytes) in &parts {
            let _ = write!(c.out, "/RFLocal{p} findfont {} scalefont setfont (", n(size));
            for &b in bytes {
                let _ = write!(c.out, "\\{b:03o}");
            }
            c.out.push_str(") show ");
        }
        c.out.push_str("grestore\n");
    }
    true
}

/// The text's glyph outlines, filled.
fn outline_text(c: &mut Ctx, t: &TextPrim) {
    use figbase::text::PathOp;
    let (ops, _, _) = super::raster::text_ops_fig(t);
    c.rgb(t.color);
    c.out.push_str("newpath\n");
    let mut cur = (0.0, 0.0);
    for op in ops {
        let pt = |x: f64, y: f64| c.p(Point::new(x, y));
        match op {
            PathOp::Move(x, y) => {
                cur = pt(x, y);
                let _ = writeln!(c.out, "{} {} moveto", n(cur.0), n(cur.1));
            }
            PathOp::Line(x, y) => {
                cur = pt(x, y);
                let _ = writeln!(c.out, "{} {} lineto", n(cur.0), n(cur.1));
            }
            PathOp::Quad(a, b, x, y) => {
                let (q, e) = (pt(a, b), pt(x, y));
                let c1 = (cur.0 + 2.0 / 3.0 * (q.0 - cur.0), cur.1 + 2.0 / 3.0 * (q.1 - cur.1));
                let c2 = (e.0 + 2.0 / 3.0 * (q.0 - e.0), e.1 + 2.0 / 3.0 * (q.1 - e.1));
                let _ = writeln!(
                    c.out,
                    "{} {} {} {} {} {} curveto",
                    n(c1.0),
                    n(c1.1),
                    n(c2.0),
                    n(c2.1),
                    n(e.0),
                    n(e.1)
                );
                cur = e;
            }
            PathOp::Cubic(a, b, cc, d, x, y) => {
                let (p1, p2, e) = (pt(a, b), pt(cc, d), pt(x, y));
                let _ = writeln!(
                    c.out,
                    "{} {} {} {} {} {} curveto",
                    n(p1.0),
                    n(p1.1),
                    n(p2.0),
                    n(p2.1),
                    n(e.0),
                    n(e.1)
                );
                cur = e;
            }
            PathOp::Close => c.out.push_str("closepath\n"),
        }
    }
    c.out.push_str("fill\n");
}

fn text(c: &mut Ctx, t: &TextPrim) {
    if local_text(c, t) {
        return;
    }
    let name = font_name(t);
    let utf8 = needs_utf8(t);
    let latin = !matches!(name, "Symbol" | "ZapfDingbats");
    let size = super::font_px(t.size) * c.k;
    let (x, y) = c.p(t.origin);
    c.rgb(t.color);
    let _ = writeln!(
        c.out,
        "gsave {} {} translate {} rotate /{}{} findfont {} scalefont setfont",
        n(x),
        n(y),
        n(t.angle.to_degrees()),
        name,
        if utf8 {
            "-UTF-8"
        } else if latin {
            "-iso"
        } else {
            ""
        },
        n(size)
    );
    for (i, line) in t.text.split('\n').enumerate() {
        let dy = -(i as f64) * size * figbase::text::LINE_SPACING;
        let s = if utf8 {
            ps_string_utf8(line)
        } else if !latin && line.chars().any(|c| u32::from(c) > 0xff) {
            ps_string_bytes(line)
        } else {
            ps_string(line)
        };
        let just = match t.justify {
            Justify::Left => String::new(),
            Justify::Center => format!("{s} stringwidth pop -2 div 0 rmoveto "),
            Justify::Right => format!("{s} stringwidth pop neg 0 rmoveto "),
        };
        let _ = writeln!(c.out, "0 {} moveto {}{} show", n(dy), just, s);
    }
    c.out.push_str("grestore\n");
}

/// The pixels as hex RGB, composited over white; with a colour key, the
/// transparent pixels (alpha 0) are written as the key, which the image
/// operator leaves out.
fn push_hex_pixels(out: &mut String, img: &figbase::image::RasterImage, key: Option<[u8; 3]>) {
    let mut col = 0;
    for &[r, g, b, a] in img.rgba.as_chunks::<4>().0 {
        let blend = |v: u8| -> u8 {
            let a = u16::from(a);
            ((u16::from(v) * a + 255 * (255 - a) + 127) / 255) as u8
        };
        let [r, g, b] = match key {
            Some(k) if a == 0 => k,
            _ => [blend(r), blend(g), blend(b)],
        };
        let _ = write!(out, "{r:02x}{g:02x}{b:02x}");
        col += 6;
        if col >= 72 {
            out.push('\n');
            col = 0;
        }
    }
}

/// A standalone colour EPS file showing `img` at 1 pixel = 1 point (as
/// bmeps does), alpha composited against white. `None` for an empty or
/// inconsistent image.
pub fn raster_eps(img: &figbase::image::RasterImage) -> Option<String> {
    let (w, h) = (img.width, img.height);
    let expected = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    if w == 0 || h == 0 || img.rgba.len() != expected {
        return None;
    }
    let mut out = String::new();
    let _ = writeln!(out, "%!PS-Adobe-3.0 EPSF-3.0");
    let _ = writeln!(out, "%%BoundingBox: 0 0 {w} {h}");
    let _ = writeln!(out, "%%Creator: RustFig fig2dev");
    let _ = writeln!(out, "%%EndComments");
    let _ = writeln!(out, "gsave");
    let _ = writeln!(out, "/picstr {} string def", u64::from(w) * 3);
    // The image matrix maps the picture onto the unit square; scale it to
    // one point per pixel, as bmeps does.
    let _ = writeln!(out, "{w} {h} scale");
    let _ = writeln!(
        out,
        "{w} {h} 8 [{w} 0 0 -{h} 0 {h}] {{currentfile picstr readhexstring pop}} false 3 colorimage"
    );
    push_hex_pixels(&mut out, img, None);
    out.push_str("\ngrestore\nshowpage\n%%EOF\n");
    Some(out)
}

/// The colour that stands for transparent in an image whose pixels are
/// fully opaque or fully transparent: all transparent pixels have it, none
/// of the opaque ones has not (they would be masked too).
fn color_key(img: &figbase::image::RasterImage) -> Option<[u8; 3]> {
    let px = img.rgba.as_chunks::<4>().0;
    let key = px.iter().find(|p| p[3] == 0).map(|p| [p[0], p[1], p[2]])?;
    px.iter()
        .all(|p| match p[3] {
            0 => [p[0], p[1], p[2]] == key,
            255 => [p[0], p[1], p[2]] != key,
            _ => false,
        })
        .then_some(key)
}

fn image(c: &mut Ctx, corners: &[Point; 4], img: &figbase::image::RasterImage, flipped: bool) {
    let (w, h) = (img.width, img.height);
    let expected = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    if w == 0 || h == 0 || img.rgba.len() != expected {
        return;
    }
    let [tl, tr, _, bl] = corners.map(|p| {
        let (x, y) = c.p(p);
        Point::new(x, y)
    });
    let (ex, ey) = if flipped {
        (bl - tl, tr - tl)
    } else {
        (tr - tl, bl - tl)
    };
    // Map the unit square (image space, y down) onto the box.
    let _ = writeln!(
        c.out,
        "gsave [{} {} {} {} {} {}] concat",
        n(ex.x),
        n(ex.y),
        n(ey.x),
        n(ey.y),
        n(tl.x),
        n(tl.y)
    );
    let _ = writeln!(c.out, "/picstr {} string def", u64::from(w) * 3);
    let key = color_key(img);
    if let Some([r, g, b]) = key {
        // Transparent pixels (a colour key, as genps's `/ImageType 4
        // /MaskColor`) leave what lies below.
        let _ = writeln!(
            c.out,
            "/DeviceRGB setcolorspace\n<< /ImageType 4 /MaskColor [{r} {g} {b}] /Width {w} /Height {h}\n   \
             /ImageMatrix [{w} 0 0 {h} 0 0] /DataSource {{currentfile picstr readhexstring pop}}\n   \
             /BitsPerComponent 8 /Decode [0 1 0 1 0 1] >> image"
        );
    } else {
        let _ = writeln!(
            c.out,
            "{w} {h} 8 [{w} 0 0 {h} 0 0] {{currentfile picstr readhexstring pop}} false 3 colorimage"
        );
    }
    push_hex_pixels(&mut c.out, img, key);
    c.out.push_str("\ngrestore\n");
}

const ISO_FONTS: [&str; 39] = [
    "Times-Roman",
    "Times-Italic",
    "Times-Bold",
    "Times-BoldItalic",
    "AvantGarde-Book",
    "AvantGarde-BookOblique",
    "AvantGarde-Demi",
    "AvantGarde-DemiOblique",
    "Bookman-Light",
    "Bookman-LightItalic",
    "Bookman-Demi",
    "Bookman-DemiItalic",
    "Courier",
    "Courier-Oblique",
    "Courier-Bold",
    "Courier-BoldOblique",
    "Helvetica",
    "Helvetica-Oblique",
    "Helvetica-Bold",
    "Helvetica-BoldOblique",
    "Helvetica-Narrow",
    "Helvetica-Narrow-Oblique",
    "Helvetica-Narrow-Bold",
    "Helvetica-Narrow-BoldOblique",
    "NewCenturySchlbk-Roman",
    "NewCenturySchlbk-Italic",
    "NewCenturySchlbk-Bold",
    "NewCenturySchlbk-BoldItalic",
    "Palatino-Roman",
    "Palatino-Italic",
    "Palatino-Bold",
    "Palatino-BoldItalic",
    "ZapfChancery-MediumItalic",
    "Times-Roman",
    "Times-Roman",
    "Times-Roman",
    "Times-Roman",
    "Times-Roman",
    "Times-Roman",
];

/// The procedure and font definitions of the prolog.
fn prolog(out: &mut String) {
    out.push_str("/m {moveto} bind def /l {lineto} bind def\n");
    out.push_str(
        "/isofont { findfont dup length dict begin {1 index /FID ne {def} {pop pop} ifelse} forall \
         /Encoding ISOLatin1Encoding def currentdict end definefont pop } bind def\n",
    );
    let mut seen = std::collections::BTreeSet::new();
    for f in ISO_FONTS {
        if seen.insert(f) {
            let _ = writeln!(out, "/{f}-iso /{f} isofont");
        }
    }
}

/// The scene's primitives, in the frame of `c`.
fn draw_prims(c: &mut Ctx, scene: &Scene, embeds: &std::collections::BTreeMap<String, EmbeddedEps>) {
    let mut skip_placeholder = false;
    for p in &scene.prims {
        if std::mem::take(&mut skip_placeholder) {
            continue;
        }
        match p {
            // (the PostScript writers get PDF pages as embedded EPS)
            Prim::LinkedImage { .. } | Prim::PdfPicture { .. } => {}
            Prim::PictureRef { corners, flipped, file } => {
                if let Some(e) = embeds.get(file) {
                    embed(c, corners, *flipped, e);
                    // The next primitive is the placeholder for the picture.
                    skip_placeholder = true;
                }
            }
            Prim::Path { subpaths, fill, stroke } => {
                match fill {
                    Some(Fill::Solid(col)) => {
                        c.rgb(*col);
                        c.path(subpaths);
                        c.out.push_str("eofill\n");
                    }
                    Some(Fill::Pattern { kind, fg, bg, nonzero }) => {
                        c.rgb(*bg);
                        c.path(subpaths);
                        c.out.push_str(if *nonzero {
                            "gsave fill grestore gsave clip\n"
                        } else {
                            "gsave eofill grestore gsave eoclip\n"
                        });
                        let region = Rect::from_points(subpaths.iter().flat_map(|s| s.pts.iter().copied()));
                        let frame = PatternFrame {
                            origin: Point::new(c.min_x, c.max_y),
                            fig_per_pt: 1.0 / c.k,
                        };
                        let (lines, line_width) = coverage(*kind, &region, &frame);
                        let lines: Vec<SubPath> = lines
                            .into_iter()
                            .map(|pts| SubPath {
                                pts,
                                closed: false,
                                curve: None,
                            })
                            .collect();
                        c.path(&lines);
                        c.stroke(&Stroke {
                            width: line_width,
                            color: *fg,
                            dash: Vec::new(),
                            cap: CapStyle::Butt,
                            join: JoinStyle::Miter,
                        });
                        c.out.push_str("grestore\n");
                    }
                    None => {}
                }
                if let Some(s) = stroke {
                    c.path(subpaths);
                    c.stroke(s);
                }
            }
            Prim::Text(t) => text(c, t),
            Prim::Image {
                corners,
                image: img,
                flipped,
                dct,
            } => match dct.as_ref().and_then(|f| embeds.get(f)) {
                Some(e) => embed(c, corners, *flipped, e),
                None => image(c, corners, img, *flipped),
            },
        }
    }
}

/// Render a scene as EPS covering `bounds` (Fig units) at `scale`.
/// `meta` has the header comments (genps_start).
/// `embeds` holds the EPS pictures to embed, by file name.
pub fn to_eps(
    scene: &Scene,
    bounds: &Rect,
    scale: f64,
    background: Option<Rgba>,
    meta: &Meta<'_>,
    embeds: &std::collections::BTreeMap<String, EmbeddedEps>,
) -> String {
    let b = if bounds.is_empty() {
        Rect::new(Point::ZERO, Point::new(1.0, 1.0))
    } else {
        *bounds
    };
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let k = 72.0 / 1200.0 * scale;
    let mut c = Ctx {
        out: String::new(),
        k,
        min_x: b.min.x,
        max_y: b.max.y,
        date: meta.date.as_ref().map(|d| d.iso()),
        lf: Vec::new(),
        lf_outline: Default::default(),
    };
    (c.lf, c.lf_outline) = plan_local_fonts(scene);
    let (w, h) = (b.width() * k, b.height() * k);
    draw_prims(&mut c, scene, embeds);
    let body = std::mem::take(&mut c.out);
    c.out.push_str("%!PS-Adobe-3.0 EPSF-3.0\n");
    dsc_head(&mut c.out, meta);
    let _ = writeln!(c.out, "%%BoundingBox: 0 0 {} {}", w.ceil() as i64, h.ceil() as i64);
    let _ = writeln!(c.out, "%%Magnification: {:.4}", meta.magnification);
    let fonts = utf8_fonts(scene);
    utf8_comments(&mut c.out, scene);
    c.out.push_str("%%EndComments\n");
    figure_comments(&mut c.out, meta);
    c.out.push_str("%%BeginProlog\n");
    utf8_cmap(&mut c.out, &fonts);
    prolog(&mut c.out);
    utf8_composefonts(&mut c.out, &fonts);
    type42_fonts(&mut c.out, &c.lf);
    c.out.push_str("%%EndProlog\n%%Page: 1 1\nsave\n");
    if let Some(bg) = background {
        c.rgb(bg);
        let _ = writeln!(c.out, "0 0 {} {} rectfill", n(w), n(h));
    }
    c.out.push_str(&body);
    c.out.push_str("restore\nshowpage\n%%Trailer\n%%EOF\n");
    c.out
}

/// Render a scene as a DSC PostScript document on `layout`'s pages:
/// `bounds` (Fig units) at `scale` placed on each page as
/// [`PageLayout::pages`] says, clipped to the paper.
#[allow(clippy::too_many_arguments)]
pub fn to_ps(
    scene: &Scene,
    bounds: &Rect,
    scale: f64,
    background: Option<Rgba>,
    meta: &Meta<'_>,
    embeds: &std::collections::BTreeMap<String, EmbeddedEps>,
    layout: &PageLayout,
) -> String {
    let b = if bounds.is_empty() {
        Rect::new(Point::ZERO, Point::new(1.0, 1.0))
    } else {
        *bounds
    };
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let k = 72.0 / 1200.0 * scale;
    let mut c = Ctx {
        out: String::new(),
        k,
        min_x: b.min.x,
        max_y: b.max.y,
        date: meta.date.as_ref().map(|d| d.iso()),
        lf: Vec::new(),
        lf_outline: Default::default(),
    };
    (c.lf, c.lf_outline) = plan_local_fonts(scene);
    draw_prims(&mut c, scene, embeds);
    let body = std::mem::take(&mut c.out);
    let lf = std::mem::take(&mut c.lf);
    let (pw, ph) = (layout.width, layout.height);
    let o = &mut c.out;
    o.push_str("%!PS-Adobe-3.0\n");
    dsc_head(o, meta);
    let _ = writeln!(o, "%%Pages: {}", layout.declared_pages);
    let _ = writeln!(o, "%%BoundingBox: 0 0 {pw} {ph}");
    let _ = writeln!(o, "%%DocumentPaperSizes: {}", layout.paper.to_ascii_lowercase());
    let _ = writeln!(o, "%%Magnification: {:.4}", meta.magnification);
    let fonts = utf8_fonts(scene);
    utf8_comments(o, scene);
    o.push_str("%%EndComments\n");
    o.push_str("%%BeginSetup\n[{\n");
    let _ = writeln!(o, "%%BeginFeature: *PageSize {}", layout.paper);
    let _ = writeln!(o, "<</PageSize [{pw} {ph}]>> setpagedevice");
    o.push_str("%%EndFeature\n} stopped cleartomark\n%%EndSetup\n");
    figure_comments(o, meta);
    o.push_str("%%BeginProlog\n");
    utf8_cmap(o, &fonts);
    prolog(o);
    utf8_composefonts(o, &fonts);
    type42_fonts(o, &lf);
    o.push_str("%%EndProlog\n");
    for (i, (tx, ty)) in layout.pages.iter().enumerate() {
        let _ = writeln!(o, "%%Page: {} {}\nsave", i + 1, i + 1);
        // The background fills the first page, as fig2dev's prolog does.
        if let (0, Some(bg)) = (i, background) {
            let (r, g, bl) = (
                f64::from(bg.r) / 255.0,
                f64::from(bg.g) / 255.0,
                f64::from(bg.b) / 255.0,
            );
            let _ = writeln!(o, "{r:.4} {g:.4} {bl:.4} setrgbcolor\n0 0 {pw} {ph} rectfill");
        }
        let _ = writeln!(
            o,
            "newpath 0 0 moveto {pw} 0 lineto {pw} {ph} lineto 0 {ph} lineto closepath clip newpath"
        );
        let _ = writeln!(o, "{} {} translate", n(*tx), n(*ty));
        o.push_str(&body);
        o.push_str("restore\nshowpage\n");
    }
    o.push_str("%%Trailer\n%%EOF\n");
    c.out
}

/// Embeds an EPS file into the picture box `corners` (genps.c).
fn embed(c: &mut Ctx, corners: &[Point; 4], flipped: bool, e: &EmbeddedEps) {
    let (pllx, plly, purx, pury) = (e.llx, e.lly, e.urx, e.ury);
    let (img_w, img_h) = (f64::from(purx) - f64::from(pllx), f64::from(pury) - f64::from(plly));
    let (pic_w, pic_h) = if flipped { (img_h, img_w) } else { (img_w, img_h) };
    if !(pic_w > 0.0 && pic_h > 0.0) {
        return;
    }
    let [tl, _, br, _] = *corners;
    let jpeg = e.jpeg.as_ref();
    let date = c.date.clone();
    let o = &mut c.out;
    o.push_str("%\n% pen to black in case this eps object doesn't set color first\n0 0 0 setrgbcolor\n");
    if jpeg.is_some() {
        // readjpg.c's comment
        let _ = writeln!(o, "% Begin Imported JPEG File: {}\n", e.title);
        o.push_str("newpath gsave\n");
    } else {
        let _ = writeln!(o, "% Begin Imported EPS File: {}", e.title);
        let _ = writeln!(o, "%%BeginDocument: {}", e.title);
        o.push_str("%\nnewpath gsave\n");
    }
    // Fig coordinates, as genps works in them.
    let _ = writeln!(
        o,
        "[{} 0 0 {} {} {}] concat",
        n(c.k),
        n(-c.k),
        n(-c.min_x * c.k),
        n(c.max_y * c.k)
    );
    let _ = writeln!(o, "{} {} translate", n(tl.x), n(tl.y));
    let _ = writeln!(
        o,
        "{:.6} {:.6} scale",
        ((br.x - tl.x) / pic_w).abs(),
        -(br.y - tl.y) / pic_h
    );
    if flipped {
        let _ = writeln!(o, "{} 0 translate\n270 rotate\n1 -1 scale", n(pic_w));
    } else {
        let _ = writeln!(o, "0 {} translate", n(-pic_h));
    }
    let _ = writeln!(o, "{} {} translate", -pllx, -plly);
    o.push_str("save\n");
    if let Some(j) = jpeg {
        // genps: the picture's size in bits, then JPEGtoPS's EPS
        o.push_str("% JPEG image follows:\n");
        let _ = writeln!(o, "{purx} {pury} sc");
        o.push_str(&readjpg::jpeg_to_ps(&j.info, &e.title, date.as_deref(), &j.data));
        o.push_str("restore grestore\n%\n");
        let _ = writeln!(o, "% End Imported PIC File: {}", e.title);
        o.push_str("%\n");
        return;
    }
    let _ = writeln!(
        o,
        "newpath {pllx} {plly} moveto {purx} {plly} lineto {purx} {pury} lineto {pllx} {pury} lineto closepath clip newpath"
    );
    o.push_str("countdictstack\nmark\n/showpage {} def\n/setpagedevice {pop} def\n% EPS file follows:\n");
    o.push_str(&String::from_utf8_lossy(&e.ps));
    if !o.ends_with('\n') {
        o.push('\n');
    }
    o.push_str("\ncleartomark\ncountdictstack exch sub { end } repeat\nrestore grestore\n%\n");
    let _ = writeln!(o, "% End Imported PIC File: {}", e.title);
    o.push_str("%%EndDocument\n%\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_strings() {
        assert_eq!(ps_string("a(b)\\"), "(a\\(b\\)\\\\)");
        assert_eq!(ps_string("é€"), "(\\351?)");
    }
}
