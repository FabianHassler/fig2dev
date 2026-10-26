//! fig2dev's `svg` driver: the figure as an SVG document, object by object
//! as `gensvg.c` writes it (whole-number coordinates in Fig units, a
//! `viewBox` of the figure, `<!-- Line -->` style comments, `<polyline>`,
//! `<polygon>`, `<rect>`, `<circle>`, `<ellipse>` and `<path>` elements,
//! `<pattern>` tiles for pattern fills, `<clipPath>`s that keep a line out of
//! its arrowheads, one `<text>` per text with a Fig font's family, style and
//! weight, Symbol and ZapfDingbats by their Unicode tables).
//!
//! Text is written byte by byte, as C does: in a Symbol or ZapfDingbats text
//! every byte of the string is looked up (a UTF-8 file's multibyte
//! characters give one reference per byte). Where C writes broken XML, this
//! port keeps the document well-formed: `tspan`s left open at the end of a
//! special text are closed, and a `}` at the very end of one writes no NUL
//! byte.
//
// Ported from fig2dev dev/gensvg.c (xfig licence, see LICENSE-xfig).

use crate::bound::{calc_arrow, compute_arcarrow_angle, ArrowPoints};
use crate::drivers::dxf::fmt_g;
use crate::gendev::{Ctx, NUM_STD_COLS};
use crate::object::{
    Arc, Arrow, Compound, Ellipse, Line, Point, Spline, Text, BLACK_COLOR, DEFAULT, NUMFILLS, NUMPATTERNS, NUMSHADES,
    SOLID_LINE, T_ARC_BOX, T_BOX, T_CIRCLE_BY_DIA, T_CIRCLE_BY_RAD, T_PIC_BOX, T_PIE_WEDGE_ARC, T_POLYGON, T_POLYLINE,
    UNFILLED, WHITE_COLOR,
};
use crate::scene::output::CREATOR;
use crate::scene::page::paper;
use crate::util::creationdate::creation_time;
use crate::{fprintf, Driver, Out};
use figbase::text::{DINGBAT_CHAR, SYMBOL_CHAR};

/// `SVG_LINEWIDTH`: where point lists break their lines.
const SVG_LINEWIDTH: usize = 76;

/// The standard colours as `rgbColorVal` has them (`rgbColors[]`).
const RGB_COLORS: [u32; NUM_STD_COLS] = [
    0x000000, 0x0000ff, 0x00ff00, 0x00ffff, 0xff0000, 0xff00ff, 0xffff00, 0xffffff, 0x00008f, 0x0000b0, 0x0000d1,
    0x87cfff, 0x008f00, 0x00b000, 0x00d100, 0x008f8f, 0x00b0b0, 0x00d1d1, 0x8f0000, 0xb00000, 0xd10000, 0x8f008f,
    0xb000b0, 0xd100d1, 0x803000, 0xa14000, 0xb46100, 0xff8080, 0xffa1a1, 0xffbfbf, 0xffe0e0, 0xffd600,
];

/// One `pattern[]` entry of `generate_tile`: size attributes and path data.
const PATTERNS: [(&str, &str); NUMPATTERNS as usize] = [
    // 30 degrees left diagonal
    ("width=\"134\" height=\"67\">", "\"M -7,30 73,70 M 61,-3 141,37\""),
    // 30 degrees right diagonal
    ("width=\"134\" height=\"67\">", "\"M -7,37 73,-3 M 61,70 141,30\""),
    // 30 degrees crosshatch
    (
        "width=\"134\" height=\"67\">",
        "\"M -7,30 73,70 M 61,-3 141,37 M -7,37 73,-3 M 61,70 141,30\"",
    ),
    // 45 degrees left diagonal
    ("width=\"134\" height=\"134\">", "\"M -4,63 71,138 M 63,-4 138,71\""),
    // 45 degrees right diagonal
    ("width=\"134\" height=\"134\">", "\"M -4,71 71,-4 M 63,138 138,63\""),
    // 45 degrees crosshatch
    (
        "width=\"134\" height=\"134\">",
        "\"M-4,63 71,138 M63,-4 138,71 M-4,71 71,-4 M63,138 138,63\"",
    ),
    // horizontal bricks
    (
        "width=\"268\" height=\"268\">",
        "\"M-1,67 H269 M-1,201 H269 M67,-1 V67 M67,201 V269 M201,67 V201\"",
    ),
    // vertical bricks
    (
        "width=\"268\" height=\"268\">",
        "\"M67,-1 V269 M201,-1 V269 M-1,67 H67 M201,67 H269 M67,201 H201\"",
    ),
    // horizontal lines
    ("width=\"268\" height=\"67\">", "\"M -1,30 H 269\""),
    // vertical lines
    ("width=\"67\" height=\"268\">", "\"M 30,-1 V 269\""),
    // crosshatch
    ("width=\"67\" height=\"67\">", "\"M -1,30 H 68 M 30,-1 V 68\""),
    // left-pointing shingles
    (
        "width=\"402\" height=\"402\">",
        "\"M-1,30 H403 M-1,164 H403 M-1,298 H403 M238,30 l-67,134 M372,164 l-67,134 M104,298 l-60,120 M37,30 l20,-40\"",
    ),
    // right-pointing shingles
    (
        "width=\"402\" height=\"402\">",
        "\"M-1,30 H403 M-1,164 H403 M-1,298 H403 M164,30 l67,134 M30,164 l67,134 M298,298 l60,120 M365,30 l-20,-40\"",
    ),
    // vertical left-pointing shingles
    (
        "width=\"402\" height=\"402\">",
        "\"M30,-1 V403 M164,-1 V403 M298,-1 V403 M30,164 l134,67 M164,30 l134,67 M298,298 l120,60 M30,365 l-40,-20\"",
    ),
    // vertical right-pointing shingles
    (
        "width=\"402\" height=\"402\">",
        "\"M30,-1 V403 M164,-1 V403 M298,-1 V403 M30,238 l134,-67 M164,372 l134,-67 M298,104 l120,-60 M30,37 l-40,20\"",
    ),
    // fish scales
    (
        "width=\"268\" height=\"140\">",
        "\"M-104,-30 a167.5,167.5 0 0,0 268,0 a167.5,167.5 0 0,0 134,67 m0,3 a167.5,167.5 0 0,1 -268,0 a167.5,167.5 0 0,1 -134,67 m134,70 a167.5,167.5 0 0,0 134,-67 a167.5,167.5 0 0,0 134,67\"",
    ),
    // small fish scales
    (
        "width=\"134\" height=\"134\">",
        "\"M164,-30 a67,67 0 0,1 -134,0 a67,67 0 0,1 -67,67 a67,67 0 0,0 134,0 a67,67 0 0,0 67,67 a67,67 0 0,1 -134,0 a67,67 0 0,1 -67,67\"",
    ),
    // circles
    (
        "width=\"268\" height=\"268\">",
        "\"M0,134 a134,134 0 0,0 134,-134 a134,134 0 0,0 134,134 a134,134 0 0,0 -134,134 a134,134 0 0,0 -134,-134\"",
    ),
    // hexagons
    (
        "width=\"402\" height=\"232\">",
        "\"m97,-86 -67,116 67,116 -67,116 M231,-86 l67,116 l-67,116 l67,116 M-1,30 h31 m268,0 h105 M97,146 h134\"",
    ),
    // octagons
    (
        "width=\"280\" height=\"280\">",
        "\"m-1,140 59,0 82,-82 82,82 -82,82 -82,-82 m82,82 v59 m0,-282 v59 m82,82 h59\"",
    ),
    // horizontal sawtooth
    ("width=\"134\" height=\"134\">", "\"m-4,63 67,67 67,-67 20,20\""),
    // vertical sawtooth
    ("width=\"134\" height=\"134\">", "\"m63,-4 67,67 -67,67 20,20\""),
];

/// `family[]` of `gensvg_text`, by `font / 4`.
const FAMILY: [&str; 9] = [
    "Times",
    "AvantGarde",
    "Bookman",
    "Courier",
    "Helvetica",
    "Helvetica Narrow",
    "New Century Schoolbook",
    "Palatino",
    "Times,Symbol",
];

/// What `svg_arrows` is asked to do (`INIT`, `CLIP`, or a pen colour).
#[derive(Clone, Copy)]
enum ArrowMode {
    Init,
    Clip,
    Pen(i32),
}

/// The arrowheads of one object: the points `svg_arrows` computes when it is
/// initialised and keeps (C: function statics, so an object with one arrow
/// sees the other's leftovers from the previous object).
#[derive(Default)]
struct ArrowState {
    fwd: ArrowPoints,
    back: ArrowPoints,
}

/// The ends of the arrows, `forw1 -> forw2` and `back1 -> back2`.
#[derive(Clone, Copy, Default)]
struct ArrowEnds {
    forw1: Point,
    forw2: Point,
    back1: Point,
    back2: Point,
}

pub struct SvgDriver {
    /// `tileno`, `pathno`, `clipno`: the numbers last used.
    tileno: i32,
    pathno: i32,
    clipno: i32,
    arrows: ArrowState,
}

/// The driver for `svg`.
pub fn new(lang: &str) -> Option<Box<dyn Driver>> {
    (lang == "svg").then(|| {
        Box::new(SvgDriver {
            tileno: -1,
            pathno: -1,
            clipno: -1,
            arrows: ArrowState::default(),
        }) as Box<dyn Driver>
    })
}

/// Whether `lang` is written by this driver.
pub fn writes(lang: &str) -> bool {
    lang == "svg"
}

/// `rgbColorVal`.
fn rgb_color_val(ctx: &Ctx, idx: i32) -> u32 {
    if idx == DEFAULT {
        return RGB_COLORS[0];
    }
    let Ok(i) = usize::try_from(idx) else {
        return 0;
    };
    if i < NUM_STD_COLS {
        return RGB_COLORS.get(i).copied().unwrap_or(0);
    }
    // C reads past the table for a colour the file doesn't define.
    let Some(u) = ctx.user_colors.get(i - NUM_STD_COLS) else {
        return 0;
    };
    (((u.r & 0xff) as u32) << 16) | (((u.g & 0xff) as u32) << 8) | (u.b & 0xff) as u32
}

/// `rgbFillVal`: the colour of a shade or tint. C's masks (`~0xFFFF00` for
/// the blue part) let the red byte into the blue computation; that is kept,
/// and so are C's saturating float conversions as on arm64.
fn rgb_fill_val(ctx: &Ctx, color_index: i32, area_fill: i32) -> u32 {
    let (mut color_index, mut area_fill) = (color_index, area_fill);
    if color_index == BLACK_COLOR || color_index == DEFAULT {
        if area_fill > NUMSHADES - 1 {
            area_fill = NUMSHADES - 1;
        }
        area_fill = NUMSHADES - 1 - area_fill;
        color_index = WHITE_COLOR;
    }
    let rgb = rgb_color_val(ctx, color_index);
    let mut tint = false;
    if area_fill > NUMSHADES - 1 {
        tint = true;
        area_fill -= NUMSHADES - 1;
    }
    let t = f64::from(area_fill) / f64::from(NUMSHADES - 1);
    let (r, g, b) = ((rgb & 0xffff_0000) >> 16, (rgb & 0xff00) >> 8, rgb & 0xff00_00ff);
    if tint {
        let up = |c: u32| (f64::from(c) + t * f64::from(0xffu32.wrapping_sub(c))) as u32;
        ((up(r) & 0xff) << 16)
            .wrapping_add((up(g) & 0xff) << 8)
            .wrapping_add(up(b) & 0xff)
    } else {
        let down = |c: u32| (t * f64::from(c)) as i32;
        (down(r).wrapping_shl(16))
            .wrapping_add(down(g).wrapping_shl(8))
            .wrapping_add(down(b)) as u32
    }
}

/// `degrees`: the angle as SVG's `rotate` wants it.
fn degrees(angle: f64) -> f64 {
    -angle / std::f64::consts::PI * 180.0
}

/// `linewidth_adj`: thin lines get thinner, as in genps.
fn linewidth_adj(ctx: &Ctx, linewidth: i32) -> f64 {
    let ts = f64::from(ctx.thick_scale);
    let lw = f64::from(linewidth);
    if lw <= ts {
        lw / 2.0
    } else {
        lw - ts
    }
}

/// `(int) ceil(linewidth_adj(w))`.
fn stroke_px(ctx: &Ctx, w: i32) -> i32 {
    linewidth_adj(ctx, w).ceil() as i32
}

/// `lround`.
fn lround(v: f64) -> i64 {
    v.round() as i64
}

/// `put_sanitized_char` over bytes.
fn put_sanitized(out: &mut Out, bytes: &[u8]) {
    for &c in bytes {
        put_sanitized_char(out, c);
    }
}

fn put_sanitized_char(out: &mut Out, c: u8) {
    match c {
        b'<' => out.puts("&lt;"),
        b'>' => out.puts("&gt;"),
        b'&' => out.puts("&amp;"),
        c => out.bytes(&[c]),
    }
}

/// The bytes C writes for a string: the raw bytes of the file when they are
/// UTF-8 (as the document declares), else the decoded characters, so the
/// output stays valid XML and RustFig's embedding can read it.
fn xml_bytes<'a>(raw: &'a [u8], decoded: &'a str) -> &'a [u8] {
    if std::str::from_utf8(raw).is_ok() {
        raw
    } else {
        decoded.as_bytes()
    }
}

/// The codes of a Symbol or ZapfDingbats text: C's bytes (a Latin-1 file),
/// or, when the bytes are UTF-8 (a UTF-8 file, or a text RustFig built), the
/// characters below 256 as codes and the others left out, so `å` is one
/// code (∑), not the two bytes of its UTF-8.
fn font_codes(raw: &[u8], decoded: &str) -> Vec<u8> {
    if std::str::from_utf8(raw).is_ok() {
        decoded
            .chars()
            .filter_map(|c| u8::try_from(u32::from(c)).ok())
            .collect()
    } else {
        raw.to_vec()
    }
}

/// The family list of a text with a `rustfig-font:` comment naming an
/// installed font: that family first, then the Fig font's, and the style
/// the installed face has (RustFig's F7; not in gensvg).
fn installed_font(ctx: &mut Ctx, t: &Text, fig_family: &str) -> Option<(String, bool, bool)> {
    let name = t
        .comments
        .iter()
        .find_map(|c| figbase::text::parse_local_font_comment(&c.text))?;
    let Some(lf) = figbase::text::find_local_font(name) else {
        ctx.warn(format!(
            "Font '{name}' (comment of a text) is not installed, using the Fig font instead"
        ));
        return None;
    };
    let family: String = lf
        .family
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '<' | '>' | '&' | '\\') && !c.is_control())
        .collect();
    Some((format!("'{family}', {fig_family}"), lf.bold, lf.italic))
}

/// `print_svgcomments`.
fn print_svgcomments(out: &mut Out, s1: &str, comments: &[crate::object::Comment], s2: &str) {
    for c in comments {
        out.puts(s1);
        put_sanitized(out, xml_bytes(&c.raw, &c.text));
        out.puts(s2);
    }
}

fn put_capstyle(out: &mut Out, c: i32) {
    if c == 1 {
        out.puts(" stroke-linecap=\"round\"");
    } else if c == 2 {
        out.puts(" stroke-linecap=\"square\"");
    }
}

fn put_joinstyle(out: &mut Out, j: i32) {
    if j == 1 {
        out.puts(" stroke-linejoin=\"round\"");
    } else if j == 2 {
        out.puts(" stroke-linejoin=\"bevel\"");
    }
}

/// `svg_dash`.
fn svg_dash(out: &mut Out, style: i32, val: f64) {
    out.puts(" stroke-dasharray=\"");
    let l = |m: f64| lround(val * m);
    match style {
        2 => fprintf!(out, "10 {}\"", l(10.0)),
        3 => fprintf!(out, "{} {} 10 {}\"", l(10.0), l(5.0), l(5.0)),
        4 => fprintf!(out, "{} {} 10 {} 10 {}\"", l(10.0), l(3.0), l(3.0), l(3.0)),
        5 => fprintf!(
            out,
            "{} {} 10 {} 10 {} 10 {}\"",
            l(10.0),
            l(3.0),
            l(3.0),
            l(3.0),
            l(3.0)
        ),
        _ => fprintf!(out, "{} {}\"", l(10.0), l(10.0)),
    }
}

/// `chars += fprintf(...)` for the point lists: a line break once a line
/// holds more than `SVG_LINEWIDTH` characters, the tag included (`chars` is
/// what `fputs` returned there).
fn point_list<'a>(out: &mut Out, tag: &str, pts: impl Iterator<Item = &'a Point>) {
    out.puts(tag);
    let mut chars = tag.len();
    for p in pts {
        let s = format!(" {},{}", p.x, p.y);
        chars += s.len();
        out.puts(&s);
        if chars > SVG_LINEWIDTH {
            out.puts("\n");
            chars = 0;
        }
    }
}

impl SvgDriver {
    /// `generate_tile`.
    fn generate_tile(&mut self, ctx: &Ctx, out: &mut Out, number: i32, color_index: i32) {
        self.tileno += 1;
        fprintf!(
            out,
            "<pattern id=\"tile{}\" patternUnits=\"userSpaceOnUse\"\n",
            self.tileno
        );
        out.puts("\tx=\"0\" y=\"0\" ");
        let Some((size, code)) = usize::try_from(number - 1).ok().and_then(|i| PATTERNS.get(i)) else {
            return;
        };
        out.puts(size);
        // Pattern lines are .45 bp wide (7.5 Fig units at 1200 ppi).
        fprintf!(
            out,
            "\n<g stroke-width=\"{}\" stroke=\"#{:06x}\" fill=\"none\">\n",
            fmt_g(0.5 * ctx.ppi / 80.0, 2),
            rgb_color_val(ctx, color_index)
        );
        out.puts("<path d=");
        out.puts(code);
        out.puts("/>\n</g>\n</pattern>\n");
    }

    /// `INIT_PAINT`.
    fn init_paint(out: &mut Out, fill_style: i32) {
        if fill_style > NUMFILLS {
            out.puts("<defs>\n");
        }
    }

    /// `INIT_PAINT_W_CLIP`.
    fn init_paint_w_clip(
        &mut self,
        ctx: &Ctx,
        out: &mut Out,
        fill_style: i32,
        thickness: i32,
        arrows: (Option<&Arrow>, Option<&Arrow>),
        ends: &ArrowEnds,
    ) {
        out.puts("<defs>\n");
        self.svg_arrows(ctx, out, thickness, arrows, ends, ArrowMode::Clip);
        if fill_style == UNFILLED {
            out.puts("</defs>\n");
        }
    }

    /// `continue_paint`.
    fn continue_paint(&mut self, ctx: &Ctx, out: &mut Out, fill_style: i32, pen_color: i32, fill_color: i32) {
        if fill_style > NUMFILLS {
            self.pathno += 1;
            fprintf!(out, " id=\"p{}\"/>\n", self.pathno);
            self.generate_tile(ctx, out, fill_style - NUMFILLS, pen_color);
            out.puts("</defs>\n");
            fprintf!(
                out,
                "<use xlink:href=\"#p{}\" fill=\"#{:06x}\"/>\n",
                self.pathno,
                rgb_color_val(ctx, fill_color)
            );
            fprintf!(
                out,
                "<use xlink:href=\"#p{}\" fill=\"url(#tile{})\"",
                self.pathno,
                self.tileno
            );
        } else if fill_style > UNFILLED {
            fprintf!(out, " fill=\"#{:06x}\"", rgb_fill_val(ctx, fill_color, fill_style));
        }
    }

    /// `continue_paint_w_clip`.
    fn continue_paint_w_clip(&mut self, ctx: &Ctx, out: &mut Out, fill_style: i32, pen_color: i32, fill_color: i32) {
        if fill_style > UNFILLED {
            self.pathno += 1;
            fprintf!(out, " id=\"p{}\"/>\n", self.pathno);
            if fill_style > NUMFILLS {
                self.generate_tile(ctx, out, fill_style - NUMFILLS, pen_color);
            }
            out.puts("</defs>\n");
            fprintf!(out, "<use xlink:href=\"#p{}\" ", self.pathno);
            if fill_style > NUMFILLS {
                fprintf!(out, "fill=\"#{:06x}\"/>\n", rgb_color_val(ctx, fill_color));
                fprintf!(
                    out,
                    "<use xlink:href=\"#p{}\" fill=\"url(#tile{})\"/> ",
                    self.pathno,
                    self.tileno
                );
            } else {
                fprintf!(out, "fill=\"#{:06x}\"/>\n", rgb_fill_val(ctx, fill_color, fill_style));
            }
            fprintf!(out, "<use xlink:href=\"#p{}\"", self.pathno);
        }
        fprintf!(out, " clip-path=\"url(#cp{})\"", self.clipno);
    }

    /// `arrow_path`.
    fn arrow_path(ctx: &Ctx, out: &mut Out, arrow: &Arrow, arrow2: Point, pen_color: i32, pts: &ArrowPoints) {
        fprintf!(out, " to point {},{} -->\n", arrow2.x, arrow2.y);
        let closed = pts.points.first() == pts.points.last();
        point_list(
            out,
            if closed {
                "<polygon points=\""
            } else {
                "<polyline points=\""
            },
            pts.points.iter(),
        );
        let px = stroke_px(ctx, arrow.thickness as i32);
        let color = rgb_color_val(ctx, pen_color);
        fprintf!(
            out,
            "\"\n\tstroke=\"#{color:06x}\" stroke-width=\"{px}px\" stroke-miterlimit=\"8\""
        );
        if arrow.type_ < 13 && (arrow.style != 0 || !pts.fillpoints.is_empty()) {
            if pts.fillpoints.is_empty() {
                fprintf!(out, " fill=\"#{color:06x}\"/>\n");
            } else {
                // fill the special area
                out.puts("/>\n<path d=\"M ");
                for p in &pts.fillpoints {
                    fprintf!(out, "{},{} ", p.x, p.y);
                }
                fprintf!(out, "z\"\n\tstroke=\"#{color:06x}\" stroke-width=\"{px}px\"");
                fprintf!(out, " stroke-miterlimit=\"8\" fill=\"#{color:06x}\"/>\n");
            }
        } else {
            out.puts("/>\n");
        }
    }

    /// `svg_arrows`: computes the arrowheads (`Init`: true if one has a clip
    /// area), writes their clip path (`Clip`) or draws them (`Pen`).
    fn svg_arrows(
        &mut self,
        ctx: &Ctx,
        out: &mut Out,
        line_thickness: i32,
        arrows: (Option<&Arrow>, Option<&Arrow>),
        ends: &ArrowEnds,
        mode: ArrowMode,
    ) -> bool {
        let (for_arrow, back_arrow) = arrows;
        match mode {
            ArrowMode::Init => {
                if let Some(a) = for_arrow {
                    self.arrows.fwd = calc_arrow(
                        ctx,
                        ends.forw1.x,
                        ends.forw1.y,
                        ends.forw2.x,
                        ends.forw2.y,
                        line_thickness,
                        a,
                    );
                }
                if let Some(a) = back_arrow {
                    self.arrows.back = calc_arrow(
                        ctx,
                        ends.back1.x,
                        ends.back1.y,
                        ends.back2.x,
                        ends.back2.y,
                        line_thickness,
                        a,
                    );
                }
                !self.arrows.fwd.clippts.is_empty() || !self.arrows.back.clippts.is_empty()
            }
            ArrowMode::Clip => {
                self.clipno += 1;
                fprintf!(out, "<clipPath id=\"cp{}\">\n", self.clipno);
                fprintf!(
                    out,
                    "\t<path clip-rule=\"evenodd\" d=\"M {},{} H {} V {} H {} z",
                    ctx.llx,
                    ctx.lly,
                    ctx.urx,
                    ctx.ury,
                    ctx.llx
                );
                for clip in [&self.arrows.fwd.clippts, &self.arrows.back.clippts] {
                    if let Some(first) = clip.first() {
                        fprintf!(out, "\n\t\tM {},{}", first.x, first.y);
                        for p in clip.iter().skip(1) {
                            fprintf!(out, " {},{}", p.x, p.y);
                        }
                        out.puts("z");
                    }
                }
                out.puts("\"/>\n</clipPath>\n");
                true
            }
            ArrowMode::Pen(pen_color) => {
                if let Some(a) = for_arrow.filter(|_| self.arrows.fwd.points.len() > 1) {
                    out.puts("<!-- Forward arrow");
                    Self::arrow_path(ctx, out, a, ends.forw2, pen_color, &self.arrows.fwd);
                }
                if let Some(a) = back_arrow.filter(|_| self.arrows.back.points.len() > 1) {
                    out.puts("<!-- Backward arrow");
                    Self::arrow_path(ctx, out, a, ends.back2, pen_color, &self.arrows.back);
                }
                true
            }
        }
    }

    /// The stroke attributes of lines, polygons and arcs.
    fn stroke(ctx: &Ctx, out: &mut Out, pen_color: i32, thickness: i32) {
        fprintf!(
            out,
            "\n\tstroke=\"#{:06x}\" stroke-width=\"{}px\"",
            rgb_color_val(ctx, pen_color),
            stroke_px(ctx, thickness)
        );
    }
}

impl Driver for SvgDriver {
    /// `gensvg_option`: `-G` and `-L` are ignored, `-z` names the paper.
    fn option(&mut self, ctx: &mut Ctx, opt: char, arg: Option<&str>) -> Result<(), String> {
        match opt {
            'G' | 'L' => Ok(()),
            'z' => {
                ctx.papersize = arg.unwrap_or("").to_string();
                ctx.paperspec = true;
                Ok(())
            }
            c => Err(format!("Argument -{c} unknown to svg driver.")),
        }
    }

    /// `gensvg_start`.
    fn start(&mut self, ctx: &mut Ctx, out: &mut Out, objects: &Compound) -> Result<(), String> {
        out.puts("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n");
        fprintf!(out, "<!-- Creator: {CREATOR} -->\n");
        if let Some(d) = creation_time(ctx) {
            fprintf!(out, "<!-- CreationDate: {} -->\n", d.iso());
        }
        fprintf!(out, "<!-- Magnification: {} -->\n", fmt_g(ctx.mag, 3));
        let (vw, vh);
        if ctx.paperspec {
            let (_, w, h) = paper(&ctx.papersize)?;
            if ctx.landscape {
                (vw, vh) = (h, w);
            } else {
                (vw, vh) = (w, h);
            }
        } else {
            vw = (f64::from(ctx.urx.wrapping_sub(ctx.llx)) * 72.0 * ctx.mag / ctx.ppi).ceil() as i32;
            vh = (f64::from(ctx.ury.wrapping_sub(ctx.lly)) * 72.0 * ctx.mag / ctx.ppi).ceil() as i32;
        }
        out.puts("<svg\txmlns=\"http://www.w3.org/2000/svg\"\n");
        out.puts("\txmlns:xlink=\"http://www.w3.org/1999/xlink\"\n");
        fprintf!(
            out,
            "\twidth=\"{vw}pt\" height=\"{vh}pt\"\n\tviewBox=\"{} {} {} {}\">\n",
            ctx.llx,
            ctx.lly,
            ctx.urx.wrapping_sub(ctx.llx),
            ctx.ury.wrapping_sub(ctx.lly)
        );
        print_svgcomments(out, "<desc>", &objects.comments, "</desc>\n");
        out.puts("<g fill=\"none\">\n");
        Ok(())
    }

    /// `gensvg_arc`.
    fn arc(&mut self, ctx: &mut Ctx, out: &mut Out, a: &Arc) -> Result<(), String> {
        let mut has_clip = false;
        if a.fill_style == UNFILLED && a.thickness <= 0 && a.for_arrow.is_none() && a.back_arrow.is_none() {
            return Ok(());
        }
        out.puts("<!-- Arc -->\n");
        print_svgcomments(out, "<!-- ", &a.comments, " -->\n");
        let arrows = (a.for_arrow.as_ref(), a.back_arrow.as_ref());
        let mut ends = ArrowEnds::default();
        if a.for_arrow.is_some() || a.back_arrow.is_some() {
            if let Some(fa) = &a.for_arrow {
                ends.forw2 = a.point[2];
                let (x, y) = compute_arcarrow_angle(
                    ctx,
                    a.center.x,
                    a.center.y,
                    f64::from(ends.forw2.x),
                    f64::from(ends.forw2.y),
                    a.direction,
                    fa,
                );
                ends.forw1 = Point::new(x, y);
            }
            if let Some(ba) = &a.back_arrow {
                ends.back2 = a.point[0];
                let (x, y) = compute_arcarrow_angle(
                    ctx,
                    a.center.x,
                    a.center.y,
                    f64::from(ends.back2.x),
                    f64::from(ends.back2.y),
                    a.direction ^ 1,
                    ba,
                );
                ends.back1 = Point::new(x, y);
            }
            has_clip = self.svg_arrows(ctx, out, a.thickness, arrows, &ends, ArrowMode::Init);
            if a.fill_style == UNFILLED && a.thickness <= 0 {
                self.svg_arrows(ctx, out, a.thickness, arrows, &ends, ArrowMode::Pen(a.pen_color));
                return Ok(());
            }
        }
        let dx = f64::from(a.point[0].x) - a.center.x;
        let dy = f64::from(a.point[0].y) - a.center.y;
        let radius = (dx * dx + dy * dy).sqrt();
        let x = (f64::from(a.point[0].x) - a.center.x) * (f64::from(a.point[2].x) - a.center.x)
            + (f64::from(a.point[0].y) - a.center.y) * (f64::from(a.point[2].y) - a.center.y);
        let y = (f64::from(a.point[0].x) - a.center.x) * (f64::from(a.point[2].y) - a.center.y)
            - (f64::from(a.point[0].y) - a.center.y) * (f64::from(a.point[2].x) - a.center.x);
        let mut angle = if x == 0.0 && y == 0.0 { 0.0 } else { y.atan2(x) };
        if angle < 0.0 {
            angle += 2.0 * std::f64::consts::PI;
        }
        angle *= 180.0 / std::f64::consts::PI;
        if a.direction == 1 {
            angle = 360.0 - angle;
        }
        if has_clip {
            self.init_paint_w_clip(ctx, out, a.fill_style, a.thickness, arrows, &ends);
        } else {
            Self::init_paint(out, a.fill_style);
        }
        // paint the object
        out.puts("<path d=\"M");
        if a.type_ == T_PIE_WEDGE_ARC {
            fprintf!(out, " {},{} L", lround(a.center.x), lround(a.center.y));
        }
        fprintf!(
            out,
            " {},{} A {} {} {} {} {} {} {}",
            a.point[0].x,
            a.point[0].y,
            lround(radius),
            lround(radius),
            0,
            i32::from(angle.abs() > 180.0),
            i32::from(angle.abs() > 0.0 && a.direction == 0),
            a.point[2].x,
            a.point[2].y
        );
        if a.type_ == T_PIE_WEDGE_ARC {
            out.puts(" z");
        }
        out.puts("\"");
        if has_clip {
            self.continue_paint_w_clip(ctx, out, a.fill_style, a.pen_color, a.fill_color);
        } else {
            self.continue_paint(ctx, out, a.fill_style, a.pen_color, a.fill_color);
        }
        if a.thickness != 0 {
            Self::stroke(ctx, out, a.pen_color, a.thickness);
            put_capstyle(out, a.cap_style);
            if a.style > SOLID_LINE {
                svg_dash(out, a.style, a.style_val);
            }
        }
        out.puts("/>\n");
        if a.for_arrow.is_some() || a.back_arrow.is_some() {
            self.svg_arrows(ctx, out, a.thickness, arrows, &ends, ArrowMode::Pen(a.pen_color));
        }
        Ok(())
    }

    /// `gensvg_ellipse`.
    fn ellipse(&mut self, ctx: &mut Ctx, out: &mut Out, e: &Ellipse) -> Result<(), String> {
        let (cx, cy) = (e.center.x, e.center.y);
        if e.type_ == T_CIRCLE_BY_RAD || e.type_ == T_CIRCLE_BY_DIA {
            out.puts("<!-- Circle -->\n");
            print_svgcomments(out, "<!-- ", &e.comments, " -->\n");
            Self::init_paint(out, e.fill_style);
            fprintf!(out, "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{}\"", e.radiuses.x);
        } else {
            out.puts("<!-- Ellipse -->\n");
            print_svgcomments(out, "<!-- ", &e.comments, " -->\n");
            Self::init_paint(out, e.fill_style);
            if e.angle == 0.0 {
                fprintf!(out, "<ellipse cx=\"{cx}\" cy=\"{cy}\"");
            } else {
                fprintf!(
                    out,
                    "<ellipse transform=\"translate({cx},{cy}) rotate({:.0})\"",
                    degrees(e.angle)
                );
            }
            fprintf!(out, " rx=\"{}\" ry=\"{}\"", e.radiuses.x, e.radiuses.y);
        }
        self.continue_paint(ctx, out, e.fill_style, e.pen_color, e.fill_color);
        if e.thickness != 0 {
            Self::stroke(ctx, out, e.pen_color, e.thickness);
            if e.style > SOLID_LINE {
                svg_dash(out, e.style, e.style_val);
            }
        }
        out.puts("/>\n");
        Ok(())
    }

    /// `gensvg_line`.
    fn line(&mut self, ctx: &mut Ctx, out: &mut Out, l: &Line) -> Result<(), String> {
        if l.type_ == T_PIC_BOX {
            let (Some(pic), Some(p1), Some(p3)) = (&l.pic, l.points.first(), l.points.get(2)) else {
                return Ok(());
            };
            out.puts("<!-- Image -->\n");
            out.puts("<image xlink:href=\"file:");
            out.bytes(xml_bytes(pic.name_bytes(), &pic.file));
            out.puts("\" preserveAspectRatio=\"none\"\n");
            let (mut px, mut py) = (p1.x, p1.y);
            let mut width = p3.x.wrapping_sub(px);
            let mut height = p3.y.wrapping_sub(py);
            let mut rotation = match (width < 0, height < 0) {
                (true, true) => 180,
                (true, false) => 90,
                (false, true) => 270,
                (false, false) => 0,
            };
            if pic.flipped != 0 {
                rotation -= 90;
            }
            if width < 0 {
                px = p3.x;
                width = width.wrapping_neg();
            }
            if height < 0 {
                py = p3.y;
                height = height.wrapping_neg();
            }
            let cx = px.wrapping_add(width / 2);
            let cy = py.wrapping_add(height / 2);
            if pic.flipped != 0 {
                fprintf!(
                    out,
                    "transform=\"rotate({rotation} {cx} {cy}) scale(-1,1) translate({},{})\"\n",
                    cx.wrapping_mul(2).wrapping_neg(),
                    0
                );
            } else if rotation != 0 {
                fprintf!(out, "transform=\"rotate({rotation} {cx} {cy})\"\n");
            }
            fprintf!(out, "x=\"{px}\" y=\"{py}\" width=\"{width}\" height=\"{height}\"/>\n");
            return Ok(());
        }
        if l.thickness <= 0 && l.fill_style == UNFILLED && l.for_arrow.is_none() && l.back_arrow.is_none() {
            return Ok(());
        }
        // l->type == T_BOX, T_ARC_BOX, T_POLYGON or T_POLYLINE
        out.puts("<!-- Line -->\n");
        print_svgcomments(out, "<!-- ", &l.comments, " -->\n");

        if l.type_ == T_BOX || l.type_ == T_ARC_BOX || l.type_ == T_POLYGON {
            Self::init_paint(out, l.fill_style);
            if l.type_ == T_POLYGON {
                let n = l.points.len().saturating_sub(1);
                point_list(out, "<polygon points=\"", l.points.iter().take(n));
                out.puts("\"");
            } else {
                let (Some(p1), Some(p3)) = (l.points.first(), l.points.get(2)) else {
                    return Ok(());
                };
                let (mut px, mut py) = (p3.x, p3.y);
                let mut width = p1.x.wrapping_sub(px);
                let mut height = p1.y.wrapping_sub(py);
                if width < 0 {
                    px = p1.x;
                    width = width.wrapping_neg();
                }
                if height < 0 {
                    py = p1.y;
                    height = height.wrapping_neg();
                }
                fprintf!(out, "<rect x=\"{px}\" y=\"{py}\" width=\"{width}\" height=\"{height}\"");
                if l.type_ == T_ARC_BOX {
                    fprintf!(out, " rx=\"{}\"", l.radius);
                }
            }
            self.continue_paint(ctx, out, l.fill_style, l.pen_color, l.fill_color);
            // jwatt.org's SVG authoring guidelines recommend px units.
            if l.thickness != 0 {
                Self::stroke(ctx, out, l.pen_color, l.thickness);
                put_joinstyle(out, l.join_style);
                put_capstyle(out, l.cap_style);
                if l.style > SOLID_LINE {
                    svg_dash(out, l.style, l.style_val);
                }
            }
            out.puts("/>\n");
            return Ok(());
        }

        if l.type_ == T_POLYLINE {
            let mut has_clip = false;
            let arrows = (l.for_arrow.as_ref(), l.back_arrow.as_ref());
            // last[0] is the last point, last[1] the penultimate one.
            let n = l.points.len();
            let at = |i: Option<usize>| i.and_then(|i| l.points.get(i)).copied().unwrap_or_default();
            let ends = ArrowEnds {
                forw1: at(n.checked_sub(2)),
                forw2: at(n.checked_sub(1)),
                back1: at(Some(1)),
                back2: at(Some(0)),
            };
            let has_arrow = l.for_arrow.is_some() || l.back_arrow.is_some();
            if has_arrow {
                has_clip = self.svg_arrows(ctx, out, l.thickness, arrows, &ends, ArrowMode::Init);
                if l.fill_style == UNFILLED && l.thickness <= 0 {
                    self.svg_arrows(ctx, out, l.thickness, arrows, &ends, ArrowMode::Pen(l.pen_color));
                    return Ok(());
                }
            }
            if has_clip {
                self.init_paint_w_clip(ctx, out, l.fill_style, l.thickness, arrows, &ends);
            } else {
                Self::init_paint(out, l.fill_style);
            }
            point_list(out, "<polyline points=\"", l.points.iter());
            out.puts("\"");
            if has_clip {
                self.continue_paint_w_clip(ctx, out, l.fill_style, l.pen_color, l.fill_color);
            } else {
                self.continue_paint(ctx, out, l.fill_style, l.pen_color, l.fill_color);
            }
            if l.thickness != 0 {
                Self::stroke(ctx, out, l.pen_color, l.thickness);
                put_joinstyle(out, l.join_style);
                put_capstyle(out, l.cap_style);
                if l.style > SOLID_LINE {
                    svg_dash(out, l.style, l.style_val);
                }
            }
            out.puts("/>\n");
            if has_arrow {
                self.svg_arrows(ctx, out, l.thickness, arrows, &ends, ArrowMode::Pen(l.pen_color));
            }
        }
        Ok(())
    }

    /// `gensvg_spline` ("not used by fig2dev": the reader turns splines
    /// into lines).
    fn spline(&mut self, ctx: &mut Ctx, out: &mut Out, s: &Spline) -> Result<(), String> {
        out.puts("<!-- Spline -->\n");
        print_svgcomments(out, "<!-- ", &s.comments, " -->\n");
        fprintf!(
            out,
            "<path style=\"stroke:#{:06x};stroke-width:{}\" d=\"",
            rgb_color_val(ctx, s.pen_color),
            stroke_px(ctx, s.thickness)
        );
        let first = s.points.first().copied().unwrap_or_default();
        fprintf!(out, "M {},{}\n C", first.x, first.y);
        for p in &s.points {
            fprintf!(out, "{},{}\n", p.x, p.y);
        }
        out.puts("\"/>\n");
        Ok(())
    }

    /// `gensvg_text`.
    fn text(&mut self, ctx: &mut Ctx, out: &mut Out, t: &Text) -> Result<(), String> {
        const ANCHOR: [&str; 3] = ["start", "middle", "end"];
        let (mut x, mut y) = (t.base_x, t.base_y);
        out.puts("<!-- Text -->\n");
        print_svgcomments(out, "<!-- ", &t.comments, " -->\n");
        if t.angle != 0.0 {
            fprintf!(
                out,
                "<g transform=\"translate({x},{y}) rotate({:.0})\" >\n",
                degrees(t.angle)
            );
            x = 0;
            y = 0;
        }
        out.puts("<text xml:space=\"preserve\" ");
        let family = usize::try_from(t.font / 4)
            .ok()
            .and_then(|i| FAMILY.get(i))
            .copied()
            .unwrap_or("Times");
        let local = installed_font(ctx, t, family);
        let (family, local_style) = match local {
            Some((f, b, i)) => (f, Some((b, i))),
            None => (family.to_string(), None),
        };
        fprintf!(
            out,
            "x=\"{x}\" y=\"{y}\" fill=\"#{:06x}\" font-family=\"{family}\" ",
            rgb_color_val(ctx, t.color)
        );
        let anchor = usize::try_from(t.type_)
            .ok()
            .and_then(|i| ANCHOR.get(i))
            .copied()
            .unwrap_or("start");
        fprintf!(
            out,
            "font-style=\"{}\" font-weight=\"{}\" font-size=\"{}\" text-anchor=\"{anchor}\">",
            match local_style {
                Some((_, i)) if i => "italic",
                Some(_) => "normal",
                None if t.font % 2 == 0 || t.font > 31 => "normal",
                None => "italic",
            },
            match local_style {
                Some((b, _)) if b => "bold",
                Some(_) => "normal",
                None if t.font % 4 < 2 || t.font > 31 => "normal",
                None => "bold",
            },
            (t.size * ctx.ppi / 72.0 + 0.5) as i32
        );
        // C works on the bytes of the string.
        let bytes = xml_bytes(&t.raw, &t.cstring);
        if t.font == 32 || t.font == 34 {
            let table = if t.font == 32 { &SYMBOL_CHAR } else { &DINGBAT_CHAR };
            for b in font_codes(&t.raw, &t.cstring) {
                fprintf!(out, "&#{};", table.get(usize::from(b)).copied().unwrap_or(0));
            }
        } else if t.special_text() {
            special_text_body(out, t, bytes);
        } else {
            put_sanitized(out, bytes);
        }
        out.puts("</text>\n");
        if t.angle != 0.0 {
            out.puts("</g>");
        }
        Ok(())
    }

    /// `gensvg_end`.
    fn end(&mut self, _ctx: &mut Ctx, out: &mut Out) -> Result<i32, String> {
        out.puts("</g>\n</svg>\n");
        Ok(0)
    }

    fn text_include(&self) -> bool {
        true
    }
}

/// The special-text branch of `gensvg_text`: `^x` and `_x` (or `^{...}`,
/// `_{...}`) in a smaller `tspan` shifted up or down, `$` dropped.
fn special_text_body(out: &mut Out, t: &Text, s: &[u8]) {
    let size = (t.size * 8.0).ceil() as i32;
    let mut supsub = 0;
    let mut open = 0;
    let mut i = 0;
    while let Some(&first) = s.get(i) {
        let mut ch = first;
        if (supsub == 2 && ch == b'}') || supsub == 1 {
            out.puts("</tspan>");
            open -= 1;
            supsub = 0;
            if ch == b'}' {
                i += 1;
                let Some(&c) = s.get(i) else { break };
                ch = c;
            }
        }
        if ch == b'_' || ch == b'^' {
            if i + 1 >= s.len() {
                break;
            }
            supsub = 1;
            fprintf!(
                out,
                "<tspan font-size=\"{size}\" baseline-shift=\"{}\">",
                if ch == b'_' { "sub" } else { "super" }
            );
            open += 1;
            i += 1;
            ch = s.get(i).copied().unwrap_or(0);
            if ch == b'{' {
                if i + 1 >= s.len() {
                    break;
                }
                supsub = 2;
                i += 1;
                ch = s.get(i).copied().unwrap_or(0);
            }
        }
        if ch != b'$' {
            put_sanitized_char(out, ch);
        }
        i += 1;
    }
    // C leaves these open, which no XML parser accepts.
    for _ in 0..open {
        out.puts("</tspan>");
    }
}

#[cfg(test)]
mod tests {
    use super::font_codes;

    #[test]
    fn symbol_codes_from_utf8_text_are_the_characters() {
        // ∑ is Symbol code 0o345, written by RustFig as the character `å`.
        assert_eq!(font_codes("å£a".as_bytes(), "å£a"), vec![0o345, 0o243, b'a']);
        // A Latin-1 file: C's bytes as they are.
        assert_eq!(font_codes(&[0o345, b'a'], "åa"), vec![0o345, b'a']);
    }
}
