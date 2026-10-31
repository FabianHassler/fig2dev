//! PDF backend (pdf-writer). Text uses the standard 14 PDF fonts; characters
//! beyond WinAnsi are set in Type 3 fonts built from RustFig's glyph outlines.

use super::localfont::{subset_tag, GlyphSet};
use super::output::{Meta, CREATOR};
use super::pattern::{coverage, PatternFrame};
use super::{Fill, PageLayout, Prim, Rgba, Scene, Stroke, SubPath, TextPrim};
use crate::util::pdfpic::PdfPicture;
use figbase::geom::{Point, Rect, Seg};
use figbase::image::RasterImage;
use figbase::style::{CapStyle, JoinStyle, Justify};
use pdf_writer::types::{LineCapStyle, LineJoinStyle};
use pdf_writer::types::{SystemInfo, UnicodeCmap};
use pdf_writer::{Content, Date, Filter, Finish, Name, Pdf, Rect as PdfRect, Ref, Str, TextStr};
use std::collections::BTreeMap;

/// The standard fonts used for text, by resource name.
const BASE14: [(&str, &str); 14] = [
    ("F0", "Times-Roman"),
    ("F1", "Times-Italic"),
    ("F2", "Times-Bold"),
    ("F3", "Times-BoldItalic"),
    ("F4", "Helvetica"),
    ("F5", "Helvetica-Oblique"),
    ("F6", "Helvetica-Bold"),
    ("F7", "Helvetica-BoldOblique"),
    ("F8", "Courier"),
    ("F9", "Courier-Oblique"),
    ("F10", "Courier-Bold"),
    ("F11", "Courier-BoldOblique"),
    ("F12", "Symbol"),
    ("F13", "ZapfDingbats"),
];

fn base14_index(t: &TextPrim) -> usize {
    let req = t.request();
    let style = usize::from(req.italic) + 2 * usize::from(req.bold);
    if t.postscript {
        match t.font {
            32 => return 12,
            34 => return 13,
            12..=15 => return 8 + style,
            4..=7 | 16..=23 => return 4 + style,
            _ => {}
        }
    } else {
        match t.font {
            4 => return 4,
            5 => return 8,
            _ => {}
        }
    }
    style
}

/// The WinAnsi code of a character, if it has one.
fn win_ansi_byte(c: char) -> Option<u8> {
    let u = c as u32;
    if (0x20..0x7f).contains(&u) || (0xa0..=0xff).contains(&u) {
        return u8::try_from(u).ok();
    }
    match c {
        '€' => Some(0x80),
        '‘' => Some(0x91),
        '’' => Some(0x92),
        '“' => Some(0x93),
        '”' => Some(0x94),
        '•' => Some(0x95),
        '–' => Some(0x96),
        '—' => Some(0x97),
        _ => None,
    }
}

/// Encode a string in WinAnsi (other characters become '?'; text that has
/// any is set in a Type 3 font instead, see [`T3Font`]).
fn win_ansi(s: &str) -> Vec<u8> {
    s.chars().map(|c| win_ansi_byte(c).unwrap_or(b'?')).collect()
}

/// A Type 3 font for characters beyond WinAnsi: the glyph outlines of
/// RustFig's own fonts ([`figbase::text`]: installed fonts, else
/// the fallback faces), 1000 units to the em, with a `ToUnicode` map so the
/// text stays selectable. One font holds up to 255 characters of one Fig
/// font.
struct T3Font {
    font: i32,
    postscript: bool,
    glyphs: Vec<T3Glyph>,
}

struct T3Glyph {
    ch: char,
    /// Advance in glyph units (1000 per em).
    adv: f64,
    /// The glyph's content stream.
    proc_: Vec<u8>,
    /// Bounding box, glyph units.
    bbox: [f32; 4],
}

impl T3Glyph {
    fn new(t: &TextPrim, ch: char) -> T3Glyph {
        use figbase::text::PathOp;
        let em_pt = 1000.0 / figbase::text::FIG_PER_POINT;
        let shaped = figbase::text::shape(&t.request(), em_pt, &ch.to_string());
        let adv = shaped.line_widths.first().copied().unwrap_or(0.0);
        let mut c = Content::new();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        let mut grow = |x: f64, y: f64| {
            x0 = x0.min(x);
            y0 = y0.min(-y);
            x1 = x1.max(x);
            y1 = y1.max(-y);
        };
        let mut body = Content::new();
        let mut cur = (0.0, 0.0);
        let mut any = false;
        for op in shaped.lines.first().map(Vec::as_slice).unwrap_or(&[]) {
            any = true;
            match *op {
                PathOp::Move(x, y) => {
                    grow(x, y);
                    body.move_to(f(x), f(-y));
                    cur = (x, y);
                }
                PathOp::Line(x, y) => {
                    grow(x, y);
                    body.line_to(f(x), f(-y));
                    cur = (x, y);
                }
                PathOp::Quad(qx, qy, x, y) => {
                    grow(qx, qy);
                    grow(x, y);
                    let (c1x, c1y) = (cur.0 + 2.0 / 3.0 * (qx - cur.0), cur.1 + 2.0 / 3.0 * (qy - cur.1));
                    let (c2x, c2y) = (x + 2.0 / 3.0 * (qx - x), y + 2.0 / 3.0 * (qy - y));
                    body.cubic_to(f(c1x), f(-c1y), f(c2x), f(-c2y), f(x), f(-y));
                    cur = (x, y);
                }
                PathOp::Cubic(ax, ay, bx, by, x, y) => {
                    grow(ax, ay);
                    grow(bx, by);
                    grow(x, y);
                    body.cubic_to(f(ax), f(-ay), f(bx), f(-by), f(x), f(-y));
                    cur = (x, y);
                }
                PathOp::Close => {
                    body.close_path();
                }
            }
        }
        let bbox = if any && x0 <= x1 {
            [f(x0), f(y0), f(x1), f(y1)]
        } else {
            [0.0; 4]
        };
        c.start_shape_glyph(f(adv), bbox[0], bbox[1], bbox[2], bbox[3]);
        if any {
            if shaped.synthetic_bold {
                // The face has no bold: embolden by stroking, as the raster does.
                c.set_line_width(40.0).set_line_join(LineJoinStyle::RoundJoin);
            }
            let mut bytes = c.finish().to_vec();
            bytes.push(b'\n');
            bytes.extend_from_slice(&body.finish());
            bytes.push(b'\n');
            bytes.extend_from_slice(if shaped.synthetic_bold { b"B\n" } else { b"f\n" });
            return T3Glyph {
                ch,
                adv,
                proc_: bytes,
                bbox,
            };
        }
        T3Glyph {
            ch,
            adv,
            proc_: c.finish().to_vec(),
            bbox,
        }
    }
}

fn f(v: f64) -> f32 {
    if v.is_finite() {
        v.clamp(-1.0e7, 1.0e7) as f32
    } else {
        0.0
    }
}

struct Ctx {
    c: Content,
    k: f64,
    min_x: f64,
    max_y: f64,
    t3: Vec<T3Font>,
    /// Installed fonts (`rustfig-font:` comments), embedded as subsets.
    lf: Vec<GlyphSet>,
}

impl Ctx {
    /// The Type 3 font number and code of `ch` in the font of `t`
    /// (registering the glyph); `None` when 255 codes per font run out of
    /// reach (never: a new font is started).
    fn t3_code(&mut self, t: &TextPrim, ch: char) -> (usize, u8) {
        for (i, fnt) in self.t3.iter().enumerate() {
            if fnt.font == t.font && fnt.postscript == t.postscript {
                if let Some(k) = fnt.glyphs.iter().position(|g| g.ch == ch) {
                    return (i, u8::try_from(k + 1).unwrap_or(1));
                }
            }
        }
        let room = self
            .t3
            .iter()
            .position(|fnt| fnt.font == t.font && fnt.postscript == t.postscript && fnt.glyphs.len() < 255);
        let i = room.unwrap_or_else(|| {
            self.t3.push(T3Font {
                font: t.font,
                postscript: t.postscript,
                glyphs: Vec::new(),
            });
            self.t3.len() - 1
        });
        let glyph = T3Glyph::new(t, ch);
        let Some(fnt) = self.t3.get_mut(i) else {
            return (0, 1);
        };
        fnt.glyphs.push(glyph);
        (i, u8::try_from(fnt.glyphs.len()).unwrap_or(1))
    }

    fn p(&self, p: Point) -> (f32, f32) {
        (f((p.x - self.min_x) * self.k), f((self.max_y - p.y) * self.k))
    }

    fn path(&mut self, subs: &[SubPath]) {
        for s in subs {
            if let Some(curve) = &s.curve {
                for seg in curve {
                    match *seg {
                        Seg::Move(q) => {
                            let (x, y) = self.p(q);
                            self.c.move_to(x, y);
                        }
                        Seg::Line(q) => {
                            let (x, y) = self.p(q);
                            self.c.line_to(x, y);
                        }
                        Seg::Cubic(a, b, q) => {
                            let ((x1, y1), (x2, y2), (x3, y3)) = (self.p(a), self.p(b), self.p(q));
                            self.c.cubic_to(x1, y1, x2, y2, x3, y3);
                        }
                    }
                }
            } else {
                for (i, q) in s.pts.iter().enumerate() {
                    let (x, y) = self.p(*q);
                    if i == 0 {
                        self.c.move_to(x, y);
                    } else {
                        self.c.line_to(x, y);
                    }
                }
            }
            if s.closed && !s.pts.is_empty() {
                self.c.close_path();
            }
        }
    }

    fn fill_rgb(&mut self, c: Rgba) {
        self.c
            .set_fill_rgb(f32::from(c.r) / 255.0, f32::from(c.g) / 255.0, f32::from(c.b) / 255.0);
    }

    fn stroke(&mut self, s: &Stroke) {
        let c = s.color;
        self.c
            .set_stroke_rgb(f32::from(c.r) / 255.0, f32::from(c.g) / 255.0, f32::from(c.b) / 255.0);
        self.c.set_line_width(f(s.width * self.k));
        let dash: Vec<f32> = s.dash.iter().map(|v| f((v * self.k).max(0.001))).collect();
        self.c.set_dash_pattern(dash, 0.0);
        self.c.set_line_cap(match s.cap {
            CapStyle::Butt => LineCapStyle::ButtCap,
            CapStyle::Round => LineCapStyle::RoundCap,
            CapStyle::Projecting => LineCapStyle::ProjectingSquareCap,
        });
        self.c.set_line_join(match s.join {
            JoinStyle::Miter => LineJoinStyle::MiterJoin,
            JoinStyle::Round => LineJoinStyle::RoundJoin,
            JoinStyle::Bevel => LineJoinStyle::BevelJoin,
        });
        self.c.stroke();
    }
}

/// The text's glyph outlines, filled (for an installed font that may not be
/// embedded).
fn outline_text(c: &mut Ctx, t: &TextPrim) {
    use figbase::text::PathOp;
    let (ops, _, _) = super::raster::text_ops_fig(t);
    c.fill_rgb(t.color);
    let mut cur = (0.0f32, 0.0f32);
    let mut start = cur;
    for op in ops {
        match op {
            PathOp::Move(x, y) => {
                cur = c.p(Point::new(x, y));
                start = cur;
                c.c.move_to(cur.0, cur.1);
            }
            PathOp::Line(x, y) => {
                cur = c.p(Point::new(x, y));
                c.c.line_to(cur.0, cur.1);
            }
            PathOp::Quad(a, b, x, y) => {
                let (q, e) = (c.p(Point::new(a, b)), c.p(Point::new(x, y)));
                let c1 = (cur.0 + 2.0 / 3.0 * (q.0 - cur.0), cur.1 + 2.0 / 3.0 * (q.1 - cur.1));
                let c2 = (e.0 + 2.0 / 3.0 * (q.0 - e.0), e.1 + 2.0 / 3.0 * (q.1 - e.1));
                c.c.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
                cur = e;
            }
            PathOp::Cubic(a, b, cc, d, x, y) => {
                let (p1, p2, e) = (c.p(Point::new(a, b)), c.p(Point::new(cc, d)), c.p(Point::new(x, y)));
                c.c.cubic_to(p1.0, p1.1, p2.0, p2.1, e.0, e.1);
                cur = e;
            }
            PathOp::Close => {
                c.c.close_path();
                cur = start;
            }
        }
    }
    c.c.fill_nonzero();
}

/// Text in an installed font: a CID font (Identity-H) with a subset of the
/// font embedded; the widths come from the font.
fn local_text(c: &mut Ctx, t: &TextPrim, font: &std::sync::Arc<figbase::text::LocalFont>) {
    let size = super::font_px(t.size) * c.k;
    let (x, y) = c.p(t.origin);
    let (s, co) = t.angle.sin_cos();
    c.fill_rgb(t.color);
    let fi = match c.lf.iter().position(|g| g.font.name == font.name) {
        Some(i) => i,
        None => {
            c.lf.push(GlyphSet::new(font.clone()));
            c.lf.len() - 1
        }
    };
    for (i, line) in t.text.split('\n').enumerate() {
        let mut bytes: Vec<u8> = Vec::new();
        let mut adv = 0.0;
        if let Some(set) = c.lf.get_mut(fi) {
            for ch in line.chars() {
                // 65535 glyph ids is far beyond what a figure uses.
                let Some((gid, a)) = set.code(ch, 60_000) else {
                    continue;
                };
                bytes.extend_from_slice(&gid.to_be_bytes());
                adv += a;
            }
        }
        let w = adv / 1000.0 * size;
        let dx = match t.justify {
            Justify::Left => 0.0,
            Justify::Center => -w / 2.0,
            Justify::Right => -w,
        };
        let dy = -(i as f64) * size * figbase::text::LINE_SPACING;
        let ox = f64::from(x) + dx * co - dy * s;
        let oy = f64::from(y) + dx * s + dy * co;
        c.c.begin_text();
        c.c.set_font(Name(format!("LF{fi}").as_bytes()), f(size));
        c.c.set_text_matrix([f(co), f(s), f(-s), f(co), f(ox), f(oy)]);
        c.c.show(Str(&bytes));
        c.c.end_text();
    }
}

/// The objects of an embedded installed font: a Type0 font over a CID font
/// (TrueType or CFF), its descriptor, the subset file and a ToUnicode map.
fn local_font_objects(pdf: &mut Pdf, set: &GlyphSet, ids: [Ref; 5], data: &[u8]) {
    use pdf_writer::types::{CidFontType, FontFlags};
    let [type0, cid, desc, file, tou] = ids;
    let font = &set.font;
    let ps_name: String = font
        .post_script_name
        .chars()
        .filter(|ch| ch.is_ascii_graphic() && !"()<>[]{}/%".contains(*ch))
        .collect();
    let base = format!(
        "{}+{}",
        subset_tag(&font.name, &set.used),
        if ps_name.is_empty() { "LocalFont" } else { &ps_name }
    );
    let (upm, asc, dsc, bbox, ital, cap) = match font.face() {
        Some(face) => {
            let u = f32::from(face.units_per_em().max(1)) / 1000.0;
            let b = face.global_bounding_box();
            (
                u,
                f32::from(face.ascender()),
                f32::from(face.descender()),
                [
                    f32::from(b.x_min),
                    f32::from(b.y_min),
                    f32::from(b.x_max),
                    f32::from(b.y_max),
                ],
                face.italic_angle(),
                face.capital_height().map_or(f32::from(face.ascender()), f32::from),
            )
        }
        None => (1.0, 800.0, -200.0, [0.0, -200.0, 1000.0, 900.0], 0.0, 700.0),
    };
    let sc = |v: f32| v / upm;
    pdf.type0_font(type0)
        .base_font(Name(base.as_bytes()))
        .encoding_predefined(Name(b"Identity-H"))
        .descendant_font(cid)
        .to_unicode(tou);
    {
        let mut c = pdf.cid_font(cid);
        c.subtype(if font.cff {
            CidFontType::Type0
        } else {
            CidFontType::Type2
        });
        c.base_font(Name(base.as_bytes()));
        c.system_info(SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"Identity"),
            supplement: 0,
        });
        c.font_descriptor(desc);
        c.default_width(0.0);
        if !font.cff {
            c.cid_to_gid_map_predefined(Name(b"Identity"));
        }
        let mut w = c.widths();
        for &(gid, _, adv) in &set.used {
            w.consecutive(gid, [f(adv)]);
        }
    }
    let mut flags = FontFlags::NON_SYMBOLIC;
    if font.italic {
        flags |= FontFlags::ITALIC;
    }
    {
        let mut d = pdf.font_descriptor(desc);
        d.name(Name(base.as_bytes()));
        d.flags(flags);
        d.bbox(PdfRect::new(sc(bbox[0]), sc(bbox[1]), sc(bbox[2]), sc(bbox[3])));
        d.italic_angle(ital);
        d.ascent(sc(asc));
        d.descent(sc(dsc));
        d.cap_height(sc(cap));
        d.stem_v(if font.bold { 140.0 } else { 80.0 });
        if font.cff {
            d.font_file3(file);
        } else {
            d.font_file2(file);
        }
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
    let mut stream = pdf.stream(file, &packed);
    stream.filter(Filter::FlateDecode);
    if font.cff {
        stream.pair(Name(b"Subtype"), Name(b"OpenType"));
    } else {
        stream.pair(Name(b"Length1"), i32::try_from(data.len()).unwrap_or(i32::MAX));
    }
    stream.finish();
    let mut cmap = UnicodeCmap::<u16>::new(
        Name(b"Adobe-Identity-UCS"),
        SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"UCS"),
            supplement: 0,
        },
    );
    for &(gid, ch, _) in &set.used {
        cmap.pair(gid, ch);
    }
    pdf.stream(tou, &cmap.finish());
}

fn text(c: &mut Ctx, t: &TextPrim) {
    if let Some(font) = t.local() {
        if font.embeddable {
            local_text(c, t, &font);
        } else {
            outline_text(c, t);
        }
        return;
    }
    let idx = base14_index(t);
    let Some((res, _)) = BASE14.get(idx) else {
        return;
    };
    let size = super::font_px(t.size) * c.k;
    let (x, y) = c.p(t.origin);
    let (s, co) = t.angle.sin_cos();
    c.fill_rgb(t.color);
    let shaped = (idx >= 12).then(|| super::textcache::shaped(t));
    for (i, line) in t.text.split('\n').enumerate() {
        // Characters beyond WinAnsi: the line is set in Type 3 fonts.
        let t3_line: Option<Vec<(usize, u8)>> = if idx < 12 && line.chars().any(|ch| win_ansi_byte(ch).is_none()) {
            Some(line.chars().map(|ch| c.t3_code(t, ch)).collect())
        } else {
            None
        };
        // Type 3 lines use the glyph advances; WinAnsi lines in the standard
        // fonts the Adobe AFM widths, which is what the viewer draws them
        // with; Symbol and ZapfDingbats RustFig's own font metrics.
        let w = match &t3_line {
            Some(codes) => {
                codes
                    .iter()
                    .map(|&(fi, code)| {
                        c.t3.get(fi)
                            .and_then(|fnt| fnt.glyphs.get(usize::from(code) - 1))
                            .map_or(0.0, |g| g.adv)
                    })
                    .sum::<f64>()
                    / 1000.0
                    * size
            }
            None if idx < 12 => f64::from(super::afm::string_width(idx, &win_ansi(line))) / 1000.0 * size,
            None => {
                shaped
                    .as_ref()
                    .and_then(|sh| sh.line_widths.get(i).copied())
                    .unwrap_or(0.0)
                    * c.k
            }
        };
        let dx = match t.justify {
            Justify::Left => 0.0,
            Justify::Center => -w / 2.0,
            Justify::Right => -w,
        };
        let dy = -(i as f64) * size * figbase::text::LINE_SPACING;
        // Rotate (dx, dy) by the text angle (counter-clockwise, y up).
        let ox = f64::from(x) + dx * co - dy * s;
        let oy = f64::from(y) + dx * s + dy * co;
        c.c.begin_text();
        if let Some(codes) = t3_line {
            c.c.set_text_matrix([f(co), f(s), f(-s), f(co), f(ox), f(oy)]);
            let mut k = 0;
            while let Some(&(fi, _)) = codes.get(k) {
                let run: Vec<u8> = codes
                    .iter()
                    .skip(k)
                    .take_while(|(g, _)| *g == fi)
                    .map(|&(_, b)| b)
                    .collect();
                k += run.len();
                c.c.set_font(Name(format!("T3_{fi}").as_bytes()), f(size));
                c.c.show(Str(&run));
            }
        } else {
            c.c.set_font(Name(res.as_bytes()), f(size));
            c.c.set_text_matrix([f(co), f(s), f(-s), f(co), f(ox), f(oy)]);
            c.c.show(Str(&win_ansi(line)));
        }
        c.c.end_text();
    }
}

struct PendingImage {
    name: String,
    rgb: Vec<u8>,
    alpha: Option<Vec<u8>>,
    w: u32,
    h: u32,
    /// A JPEG file the PDF carries as it is (DCTDecode), as ghostscript
    /// does for genps's output.
    jpeg: Option<super::EmbeddedJpeg>,
}

fn image(
    c: &mut Ctx,
    corners: &[Point; 4],
    img: &RasterImage,
    flipped: bool,
    jpeg: Option<&super::EmbeddedJpeg>,
    images: &mut Vec<PendingImage>,
) {
    let (w, h) = (img.width, img.height);
    let expected = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    if w == 0 || h == 0 || img.rgba.len() != expected {
        return;
    }
    let name = format!("Im{}", images.len());
    let mut rgb = Vec::with_capacity(expected / 4 * 3);
    let mut alpha = Vec::with_capacity(expected / 4);
    for &[r, g, b, a] in img.rgba.as_chunks::<4>().0 {
        rgb.extend_from_slice(&[r, g, b]);
        alpha.push(a);
    }
    let opaque = alpha.iter().all(|a| *a == 255);
    let [tl, tr, _, bl] = corners.map(|p| {
        let (x, y) = c.p(p);
        Point::new(f64::from(x), f64::from(y))
    });
    let (ex, ey) = if flipped {
        (bl - tl, tr - tl)
    } else {
        (tr - tl, bl - tl)
    };
    // PDF images map the unit square with row 0 at the top (v = 1).
    c.c.save_state();
    c.c.transform([f(ex.x), f(ex.y), f(-ey.x), f(-ey.y), f(tl.x + ey.x), f(tl.y + ey.y)]);
    c.c.x_object(Name(name.as_bytes()));
    c.c.restore_state();
    images.push(PendingImage {
        name,
        rgb,
        alpha: if opaque { None } else { Some(alpha) },
        w,
        h,
        jpeg: jpeg.cloned(),
    });
}

/// A PDF picture used on the page: the name of its XObject.
struct PendingPdf {
    name: String,
    pic: std::sync::Arc<PdfPicture>,
}

/// A PDF picture: its first page as a Form XObject, whose lower left corner
/// and size (`W` x `H` points) are mapped onto the picture's box.
fn pdf_picture(
    c: &mut Ctx,
    corners: &[Point; 4],
    flipped: bool,
    pic: &std::sync::Arc<PdfPicture>,
    pdfs: &mut Vec<PendingPdf>,
) {
    let (pw, ph) = pic.size();
    let name = match pdfs.iter().find(|p| p.pic.id() == pic.id()) {
        Some(p) => p.name.clone(),
        None => {
            let name = format!("Pdf{}", pdfs.len());
            pdfs.push(PendingPdf {
                name: name.clone(),
                pic: pic.clone(),
            });
            name
        }
    };
    let [tl, tr, _, bl] = corners.map(|p| {
        let (x, y) = c.p(p);
        Point::new(f64::from(x), f64::from(y))
    });
    let (ex, ey) = if flipped {
        (bl - tl, tr - tl)
    } else {
        (tr - tl, bl - tl)
    };
    // The page's (x, y), y up, goes to tl + x/W ex + (1 - y/H) ey.
    c.c.save_state();
    c.c.transform([
        f(ex.x / pw),
        f(ex.y / pw),
        f(-ey.x / ph),
        f(-ey.y / ph),
        f(tl.x + ey.x),
        f(tl.y + ey.y),
    ]);
    c.c.x_object(Name(name.as_bytes()));
    c.c.restore_state();
}

/// Render a scene as a one-page PDF covering `bounds` (Fig units) at `scale`.
/// `title` becomes the document's `/Title` (fig2dev's `-n`).
pub fn to_pdf(scene: &Scene, bounds: &Rect, scale: f64, background: Option<Rgba>, title: Option<&str>) -> Vec<u8> {
    let meta = Meta {
        title: title.unwrap_or_default().to_string(),
        ..Meta::default()
    };
    to_pdf_pages(scene, bounds, scale, background, &meta, None, 5, &BTreeMap::new())
}

/// Like [`to_pdf`]; with a `layout` the figure goes on its pages (fig2dev's
/// page mode) instead of one page of the figure's size. `minor` is the PDF
/// version 1.`minor` (`-Y`).
#[allow(clippy::too_many_arguments)]
pub fn to_pdf_pages(
    scene: &Scene,
    bounds: &Rect,
    scale: f64,
    background: Option<Rgba>,
    meta: &Meta<'_>,
    layout: Option<&PageLayout>,
    minor: u8,
    embeds: &BTreeMap<String, super::EmbeddedEps>,
) -> Vec<u8> {
    let b = if bounds.is_empty() {
        Rect::new(Point::ZERO, Point::new(1.0, 1.0))
    } else {
        *bounds
    };
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let k = 72.0 / 1200.0 * scale;
    let (w, h) = (b.width() * k, b.height() * k);
    let mut c = Ctx {
        lf: Vec::new(),
        c: Content::new(),
        k,
        min_x: b.min.x,
        max_y: b.max.y,
        t3: Vec::new(),
    };
    if let (Some(bg), None) = (background, layout) {
        c.fill_rgb(bg);
        c.c.rect(0.0, 0.0, f(w.ceil()), f(h.ceil()));
        c.c.fill_even_odd();
    }
    let mut images = Vec::new();
    let mut pdfs: Vec<PendingPdf> = Vec::new();
    for p in &scene.prims {
        match p {
            Prim::Path { subpaths, fill, stroke } => {
                match fill {
                    Some(Fill::Solid(col)) => {
                        c.fill_rgb(*col);
                        c.path(subpaths);
                        c.c.fill_even_odd();
                    }
                    Some(Fill::Pattern { kind, fg, bg, nonzero }) => {
                        c.fill_rgb(*bg);
                        c.path(subpaths);
                        if *nonzero {
                            c.c.fill_nonzero();
                        } else {
                            c.c.fill_even_odd();
                        }
                        c.c.save_state();
                        c.path(subpaths);
                        if *nonzero {
                            c.c.clip_nonzero();
                        } else {
                            c.c.clip_even_odd();
                        }
                        c.c.end_path();
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
                        c.c.restore_state();
                    }
                    None => {}
                }
                if let Some(s) = stroke {
                    c.path(subpaths);
                    c.stroke(s);
                }
            }
            Prim::Text(t) => text(&mut c, t),
            Prim::PictureRef { .. } | Prim::LinkedImage { .. } => {}
            Prim::PdfPicture { corners, flipped, pdf } => pdf_picture(&mut c, corners, *flipped, pdf, &mut pdfs),
            Prim::Image {
                corners,
                image: img,
                flipped,
                dct,
            } => {
                let jpeg = dct.as_ref().and_then(|f| embeds.get(f)).and_then(|e| e.jpeg.as_ref());
                image(&mut c, corners, img, *flipped, jpeg, &mut images)
            }
        }
    }

    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let tree_id = Ref::new(2);
    let page_id = Ref::new(3);
    let content_id = Ref::new(4);
    let font_base = 5;
    let mut next = font_base + BASE14.len() as i32;
    let mut image_ids = Vec::new();
    for im in &images {
        let id = Ref::new(next);
        next += 1;
        let mask = if im.alpha.is_some() && im.jpeg.is_none() {
            let m = Ref::new(next);
            next += 1;
            Some(m)
        } else {
            None
        };
        image_ids.push((im.name.clone(), id, mask));
    }
    // The PDF pictures' pages: their objects come after ours; one that
    // can't be copied is an empty form, so the name still resolves.
    let mut pdf_objs: Vec<(String, Ref, Option<pdf_writer::Chunk>)> = Vec::new();
    for p in &pdfs {
        match p.pic.xobject(next) {
            Some((chunk, root, after)) => {
                next = after;
                pdf_objs.push((p.name.clone(), root, Some(chunk)));
            }
            None => {
                pdf_objs.push((p.name.clone(), Ref::new(next), None));
                next += 1;
            }
        }
    }
    // One page of the figure's size, or fig2dev's pages on the paper.
    let figure_page = [(0.0, 0.0)];
    let (page_w, page_h, places): (f64, f64, &[(f64, f64)]) = match layout {
        Some(l) => (f64::from(l.width), f64::from(l.height), &l.pages),
        // ps2pdf crops to the EPS's integer BoundingBox
        None => (w.ceil(), h.ceil(), &figure_page),
    };
    let mut page_ids = vec![(page_id, content_id)];
    for _ in 1..places.len() {
        page_ids.push((Ref::new(next), Ref::new(next + 1)));
        next += 2;
    }
    // Type 3 fonts: the font, its ToUnicode map and one id per glyph.
    let t3_ids: Vec<(Ref, Ref, Vec<Ref>)> =
        c.t3.iter()
            .map(|fnt| {
                let font = Ref::new(next);
                let tou = Ref::new(next + 1);
                next += 2;
                let glyphs = fnt
                    .glyphs
                    .iter()
                    .map(|_| {
                        next += 1;
                        Ref::new(next - 1)
                    })
                    .collect();
                (font, tou, glyphs)
            })
            .collect();
    // Installed fonts: subset, Type0 font, CID font, descriptor, file, ToUnicode.
    let lf_subsets: Vec<Option<Vec<u8>>> = c.lf.iter().map(GlyphSet::subset).collect();
    let lf_ids: Vec<[Ref; 5]> =
        c.lf.iter()
            .map(|_| {
                let r = [0, 1, 2, 3, 4].map(|i| Ref::new(next + i));
                next += 5;
                r
            })
            .collect();
    let minor = if c.lf.iter().any(|g| g.font.cff) {
        minor.max(6)
    } else {
        minor
    };
    pdf.set_version(1, minor);
    let xmp_id = Ref::new(next + 1);
    {
        let mut cat = pdf.catalog(catalog_id);
        cat.pages(tree_id);
        cat.metadata(xmp_id);
    }
    pdf.pages(tree_id)
        .kids(page_ids.iter().map(|p| p.0))
        .count(i32::try_from(page_ids.len()).unwrap_or(i32::MAX));
    for &(page_id, content_id) in &page_ids {
        let mut page = pdf.page(page_id);
        page.media_box(PdfRect::new(0.0, 0.0, f(page_w), f(page_h)));
        page.parent(tree_id);
        page.contents(content_id);
        let mut res = page.resources();
        {
            let mut fonts = res.fonts();
            for (i, (name, _)) in BASE14.iter().enumerate() {
                fonts.pair(Name(name.as_bytes()), Ref::new(font_base + i as i32));
            }
            for (i, (font, _, _)) in t3_ids.iter().enumerate() {
                fonts.pair(Name(format!("T3_{i}").as_bytes()), *font);
            }
            for (i, ids) in lf_ids.iter().enumerate() {
                fonts.pair(Name(format!("LF{i}").as_bytes()), ids[0]);
            }
        }
        {
            let mut xo = res.x_objects();
            for (name, id, _) in &image_ids {
                xo.pair(Name(name.as_bytes()), *id);
            }
            for (name, id, _) in &pdf_objs {
                xo.pair(Name(name.as_bytes()), *id);
            }
        }
        res.finish();
        page.finish();
    }
    for (i, (_, base)) in BASE14.iter().enumerate() {
        let mut font = pdf.type1_font(Ref::new(font_base + i as i32));
        font.base_font(Name(base.as_bytes()));
        if !matches!(*base, "Symbol" | "ZapfDingbats") {
            font.encoding_predefined(Name(b"WinAnsiEncoding"));
        }
    }
    for (fnt, (font_id, tou_id, glyph_ids)) in c.t3.iter().zip(&t3_ids) {
        let names: Vec<String> = fnt.glyphs.iter().map(|g| format!("uni{:04X}", g.ch as u32)).collect();
        let bbox = fnt.glyphs.iter().fold([0.0f32, -300.0, 1000.0, 1000.0], |a, g| {
            [
                a[0].min(g.bbox[0]),
                a[1].min(g.bbox[1]),
                a[2].max(g.bbox[2]),
                a[3].max(g.bbox[3]),
            ]
        });
        let mut font = pdf.type3_font(*font_id);
        font.bbox(PdfRect::new(bbox[0], bbox[1], bbox[2], bbox[3]));
        font.matrix([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
        {
            let mut cp = font.char_procs();
            for (name, id) in names.iter().zip(glyph_ids) {
                cp.pair(Name(name.as_bytes()), *id);
            }
        }
        font.encoding_custom()
            .differences()
            .consecutive(1, names.iter().map(|n| Name(n.as_bytes())));
        font.first_char(1);
        font.last_char(u8::try_from(fnt.glyphs.len()).unwrap_or(255));
        font.widths(fnt.glyphs.iter().map(|g| f(g.adv)));
        font.to_unicode(*tou_id);
        font.finish();
        let mut cmap = UnicodeCmap::<u8>::new(
            Name(b"Adobe-Identity-UCS"),
            SystemInfo {
                registry: Str(b"Adobe"),
                ordering: Str(b"UCS"),
                supplement: 0,
            },
        );
        for (k, g) in fnt.glyphs.iter().enumerate() {
            cmap.pair(u8::try_from(k + 1).unwrap_or(1), g.ch);
        }
        pdf.stream(*tou_id, &cmap.finish());
        for (g, id) in fnt.glyphs.iter().zip(glyph_ids) {
            pdf.stream(*id, &g.proc_);
        }
    }
    for ((set, ids), subset) in c.lf.iter().zip(&lf_ids).zip(&lf_subsets) {
        if let Some(data) = subset {
            local_font_objects(&mut pdf, set, *ids, data);
        }
    }
    let body = c.c.finish();
    for ((_, content_id), (i, &(tx, ty))) in page_ids.iter().zip(places.iter().enumerate()) {
        if layout.is_none() {
            pdf.stream(*content_id, &body);
            continue;
        }
        // The paper: background on the first page, clipped to the page,
        // the figure's corner at its place.
        let mut content = String::new();
        if let (0, Some(bg)) = (i, background) {
            let (r, g, b) = (
                f64::from(bg.r) / 255.0,
                f64::from(bg.g) / 255.0,
                f64::from(bg.b) / 255.0,
            );
            content.push_str(&format!("{r:.4} {g:.4} {b:.4} rg 0 0 {page_w} {page_h} re f\n"));
        }
        content.push_str(&format!("q 0 0 {page_w} {page_h} re W n 1 0 0 1 {tx:.3} {ty:.3} cm\n"));
        let mut bytes = content.into_bytes();
        bytes.extend_from_slice(&body);
        bytes.extend_from_slice(b"\nQ\n");
        pdf.stream(*content_id, &bytes);
    }
    for (im, (_, id, mask)) in images.iter().zip(image_ids.iter()) {
        if let Some(j) = &im.jpeg {
            let mut x = pdf.image_xobject(*id, &j.data);
            x.filter(Filter::DctDecode);
            x.width(j.info.width);
            x.height(j.info.height);
            match j.info.components {
                1 => x.color_space().device_gray(),
                4 => x.color_space().device_cmyk(),
                _ => x.color_space().device_rgb(),
            }
            x.bits_per_component(j.info.bits_per_component);
            if j.info.adobe && j.info.components == 4 {
                // Adobe's CMYK files are inverted
                x.decode([1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
            }
            x.finish();
            continue;
        }
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&im.rgb, 6);
        let mut x = pdf.image_xobject(*id, &data);
        x.filter(Filter::FlateDecode);
        x.width(im.w as i32);
        x.height(im.h as i32);
        x.color_space().device_rgb();
        x.bits_per_component(8);
        if let Some(m) = mask {
            x.s_mask(*m);
        }
        x.finish();
        if let (Some(m), Some(alpha)) = (mask, &im.alpha) {
            let data = miniz_oxide::deflate::compress_to_vec_zlib(alpha, 6);
            let mut s = pdf.image_xobject(*m, &data);
            s.filter(Filter::FlateDecode);
            s.width(im.w as i32);
            s.height(im.h as i32);
            s.color_space().device_gray();
            s.bits_per_component(8);
            s.finish();
        }
    }
    for (_, id, chunk) in &pdf_objs {
        match chunk {
            Some(chunk) => {
                pdf.extend(chunk);
            }
            None => {
                let mut x = pdf.form_xobject(*id, b"");
                x.bbox(PdfRect::new(0.0, 0.0, 1.0, 1.0));
            }
        }
    }
    // What ghostscript's pdfwrite takes from genps's DSC comments.
    let mut info = pdf.document_info(Ref::new(next));
    info.title(TextStr(&meta.title.replace(['\r', '\n'], " ")));
    info.creator(TextStr(CREATOR));
    info.producer(TextStr("fig2dev (Rust)"));
    if let Some(d) = &meta.date {
        let date = || {
            Date::new(u16::try_from(d.year).unwrap_or(0))
                .month(d.month as u8)
                .day(d.day as u8)
                .hour(d.hour as u8)
                .minute(d.minute as u8)
                .second(d.second as u8)
        };
        info.creation_date(date());
        info.modified_date(date());
    }
    info.finish();
    pdf.stream(xmp_id, xmp_packet(meta).as_bytes())
        .pair(Name(b"Type"), Name(b"Metadata"))
        .pair(Name(b"Subtype"), Name(b"XML"));
    pdf.finish()
}

/// The XMP packet ghostscript's pdfwrite adds (Producer, dates, creator tool,
/// title), as `ps2pdf` output has it.
fn xmp_packet(meta: &Meta<'_>) -> String {
    fn esc(s: &str) -> String {
        s.chars()
            .filter(|c| !c.is_control() || *c == '\t')
            .map(|c| match c {
                '&' => "&amp;".to_string(),
                '<' => "&lt;".to_string(),
                '>' => "&gt;".to_string(),
                c => c.to_string(),
            })
            .collect()
    }
    let mut x = String::from(
        "<?xpacket begin='\u{feff}' id='W5M0MpCehiHzreSzNTczkc9d'?>\n\
         <x:xmpmeta xmlns:x='adobe:ns:meta/'>\n\
         <rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'>\n\
         <rdf:Description rdf:about='' xmlns:pdf='http://ns.adobe.com/pdf/1.3/' pdf:Producer='fig2dev (Rust)'/>\n",
    );
    let dates = match &meta.date {
        Some(d) => {
            let iso = format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                d.year, d.month, d.day, d.hour, d.minute, d.second
            );
            format!(
                "<xmp:ModifyDate>{iso}</xmp:ModifyDate><xmp:CreateDate>{iso}</xmp:CreateDate>\
                 <xmp:MetadataDate>{iso}</xmp:MetadataDate>"
            )
        }
        None => String::new(),
    };
    x.push_str(&format!(
        "<rdf:Description rdf:about='' xmlns:xmp='http://ns.adobe.com/xap/1.0/'>{dates}\
         <xmp:CreatorTool>{}</xmp:CreatorTool></rdf:Description>\n\
         <rdf:Description rdf:about='' xmlns:dc='http://purl.org/dc/elements/1.1/' dc:format='application/pdf'>\
         <dc:title><rdf:Alt><rdf:li xml:lang='x-default'>{}</rdf:li></rdf:Alt></dc:title></rdf:Description>\n\
         </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end='w'?>",
        esc(CREATOR),
        esc(&meta.title)
    ));
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pdf_is_valid_header() {
        let pdf = to_pdf(&Scene::default(), &Rect::EMPTY, 1.0, None, Some("T"));
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn media_box_is_the_integer_bounding_box_and_xmp_follows() {
        let b = Rect::new(Point::ZERO, Point::new(2000.0, 1000.0));
        let pdf = to_pdf(&Scene::default(), &b, 1.0, None, Some("A&B"));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/MediaBox [0 0 120 60]"), "{text}");
        assert!(text.contains("/Type /Metadata"));
        assert!(text.contains("<dc:title><rdf:Alt><rdf:li xml:lang='x-default'>A&amp;B</rdf:li>"));
    }

    /// The x of the `Tm` operator of each text line in a PDF.
    fn tm_x(pdf: &[u8]) -> Vec<f64> {
        let text = String::from_utf8_lossy(pdf);
        text.lines()
            .filter_map(|l| {
                let n: Vec<&str> = l.strip_suffix(" Tm")?.split(' ').collect();
                n.get(4)?.parse().ok()
            })
            .collect()
    }

    #[test]
    fn centred_and_right_text_uses_the_standard_font_widths() {
        let line = "Wave iiii MMMM, fi.";
        let bounds = Rect::new(Point::ZERO, Point::new(12000.0, 3000.0));
        let size = 24.0;
        let x_of = |font: i32, justify: Justify| {
            let t = TextPrim {
                origin: Point::new(6000.0, 1500.0),
                angle: 0.0,
                size,
                justify,
                font,
                postscript: true,
                text: line.to_string(),
                color: Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                special: false,
                local_font: None,
            };
            let idx = base14_index(&t);
            let pdf = to_pdf(
                &Scene {
                    prims: vec![Prim::Text(t)],
                },
                &bounds,
                1.0,
                None,
                None,
            );
            let xs = tm_x(&pdf);
            (idx, xs.first().copied().unwrap_or(f64::NAN))
        };
        let mut seen = std::collections::BTreeSet::new();
        for font in [0, 1, 2, 3, 12, 13, 14, 15, 16, 17, 18, 19] {
            let (idx, left) = x_of(font, Justify::Left);
            seen.insert(idx);
            let w = f64::from(super::super::afm::string_width(idx, &win_ansi(line))) / 1000.0 * size;
            for (j, shift) in [(Justify::Center, w / 2.0), (Justify::Right, w)] {
                let (_, x) = x_of(font, j);
                assert!(
                    (left - x - shift).abs() < 0.01 * w,
                    "font {font} {j:?}: moved {} of {shift}",
                    left - x
                );
            }
        }
        assert_eq!(seen.len(), 12);
    }

    #[test]
    fn win_ansi_encoding() {
        assert_eq!(win_ansi("aé€☃"), vec![b'a', 0xe9, 0x80, b'?']);
    }
}
