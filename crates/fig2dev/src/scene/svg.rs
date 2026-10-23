//! SVG backend.
//!
//! Text follows gensvg.c: Symbol and ZapfDingbats codes become Unicode
//! character references, special text (`^`, `_`, `{`, `}`, `$`) becomes
//! `tspan`s with a baseline shift.

// Ported from fig2dev dev/gensvg.c (`gensvg_text`, `symbolchar`,
// `dingbatchar`, `gensvg_start`'s paper size) (xfig licence, see
// LICENSE-xfig).

use super::output::{Meta, CREATOR};
use super::pattern::{coverage, PatternFrame};
use super::{Fill, Prim, Rgba, Scene, Stroke, SubPath, TextPrim};
use crate::drivers::dxf::fmt_g;
use figbase::geom::{Point, Rect, Seg};
use figbase::image::RasterImage;
use figbase::style::{CapStyle, JoinStyle, Justify};
use figbase::text::{DINGBAT_CHAR, SYMBOL_CHAR};
use std::fmt::Write as _;

/// gensvg's `put_sanitized_char` over a string, as far as it matters here:
/// `<`, `>` and `&` escaped; RustFig also escapes `"` and drops control
/// characters, so the output is always well-formed XML.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

fn n(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn rgb(c: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

pub(crate) fn path_data(subs: &[SubPath]) -> String {
    let mut d = String::new();
    for s in subs {
        if let Some(curve) = &s.curve {
            for seg in curve {
                let _ = match *seg {
                    Seg::Move(p) => write!(d, "M{} {} ", n(p.x), n(p.y)),
                    Seg::Line(p) => write!(d, "L{} {} ", n(p.x), n(p.y)),
                    Seg::Cubic(a, b, p) => {
                        write!(d, "C{} {} {} {} {} {} ", n(a.x), n(a.y), n(b.x), n(b.y), n(p.x), n(p.y))
                    }
                };
            }
        } else {
            for (i, p) in s.pts.iter().enumerate() {
                let _ = write!(d, "{}{} {} ", if i == 0 { "M" } else { "L" }, n(p.x), n(p.y));
            }
        }
        if s.closed && !s.pts.is_empty() {
            d.push_str("Z ");
        }
    }
    d.trim_end().to_string()
}

fn stroke_attrs(s: &Stroke) -> String {
    let mut a = format!(" stroke=\"{}\" stroke-width=\"{}\"", rgb(s.color), n(s.width));
    if !s.dash.is_empty() {
        let d: Vec<String> = s.dash.iter().map(|v| n(v.max(0.01))).collect();
        let _ = write!(a, " stroke-dasharray=\"{}\"", d.join(" "));
    }
    a.push_str(match s.cap {
        CapStyle::Butt => "",
        CapStyle::Round => " stroke-linecap=\"round\"",
        CapStyle::Projecting => " stroke-linecap=\"square\"",
    });
    a.push_str(match s.join {
        JoinStyle::Miter => "",
        JoinStyle::Round => " stroke-linejoin=\"round\"",
        JoinStyle::Bevel => " stroke-linejoin=\"bevel\"",
    });
    a
}

/// CSS font family and style for a text.
pub(crate) fn css_font(t: &TextPrim) -> (&'static str, bool, bool) {
    let req = t.request();
    let fam = if t.postscript {
        match t.font {
            -1..=3 => "Times, 'Times New Roman', serif",
            4..=7 => "'URW Gothic', 'Avant Garde', sans-serif",
            8..=11 => "'Bookman Old Style', Bookman, serif",
            12..=15 => "Courier, 'Courier New', monospace",
            16..=23 => "Helvetica, Arial, sans-serif",
            24..=27 => "'Century Schoolbook', serif",
            28..=31 => "Palatino, 'Palatino Linotype', serif",
            32 => "Symbol",
            33 => "'Zapf Chancery', cursive",
            _ => "'Zapf Dingbats', Dingbats",
        }
    } else {
        match t.font {
            4 => "sans-serif",
            5 => "monospace",
            _ => "'Latin Modern Roman', 'Computer Modern', serif",
        }
    };
    (fam, req.bold, req.italic)
}

/// The Unicode `table` gives for each byte of a Symbol or ZapfDingbats
/// string, as `&#N;` (gensvg_text writes `&#0;` for codes without one, which
/// no XML parser accepts; those are left out).
fn table_text(table: &[u32; 256], bytes: &[u8]) -> String {
    let mut o = String::new();
    for &b in bytes {
        if let Some(&u) = table.get(usize::from(b)).filter(|&&u| u != 0) {
            let _ = write!(o, "&#{u};");
        }
    }
    o
}

/// The text as C's `cstring` bytes: the characters below 256 as themselves
/// (Latin-1, as the reader decoded them), the others as UTF-8.
fn text_bytes(line: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(line.len());
    for c in line.chars() {
        match u8::try_from(u32::from(c)) {
            Ok(v) => b.push(v),
            Err(_) => {
                let mut buf = [0u8; 4];
                b.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    b
}

/// gensvg_text's special text: `^x` and `_x` (or `^{...}`, `_{...}`) in a
/// smaller `tspan` shifted up or down, `$` dropped. As in C a `}` ends only
/// the innermost shift and a lone `}` after a one-character shift is
/// swallowed; unlike C, tspans still open at the end of the text are closed
/// (C leaves the XML broken) and a `}` at the very end writes no NUL.
fn special_body(t: &TextPrim, line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let size = (t.size * 8.0).ceil();
    let size = if size.is_finite() { size as i64 } else { 0 };
    let mut o = String::new();
    let (mut supsub, mut open) = (0, 0);
    let mut i = 0;
    while let Some(&first) = chars.get(i) {
        let mut ch = first;
        if (supsub == 2 && ch == '}') || supsub == 1 {
            o.push_str("</tspan>");
            open -= 1;
            supsub = 0;
            if ch == '}' {
                i += 1;
                let Some(&c) = chars.get(i) else { break };
                ch = c;
            }
        }
        if ch == '_' || ch == '^' {
            if i + 1 >= chars.len() {
                break;
            }
            supsub = 1;
            let shift = if ch == '_' { "sub" } else { "super" };
            let _ = write!(o, "<tspan font-size=\"{size}\" baseline-shift=\"{shift}\">");
            open += 1;
            i += 1;
            ch = chars.get(i).copied().unwrap_or(' ');
            if ch == '{' {
                if i + 1 >= chars.len() {
                    break;
                }
                supsub = 2;
                i += 1;
                ch = chars.get(i).copied().unwrap_or(' ');
            }
        }
        if ch != '$' {
            o.push_str(&esc(ch.encode_utf8(&mut [0u8; 4])));
        }
        i += 1;
    }
    for _ in 0..open {
        o.push_str("</tspan>");
    }
    o
}

/// The characters of one line of `t`.
fn text_body(t: &TextPrim, line: &str) -> String {
    match t.font {
        32 => table_text(&SYMBOL_CHAR, &text_bytes(line)),
        34 => table_text(&DINGBAT_CHAR, &text_bytes(line)),
        _ if t.special => special_body(t, line),
        _ => esc(line),
    }
}

fn text(out: &mut String, t: &TextPrim) {
    let (fam, mut bold, mut italic) = css_font(t);
    // An installed font (`rustfig-font:` comment): its family first, the
    // Fig font's families as the fallback.
    let fam_owned;
    let fam: &str = match t.local() {
        Some(lf) => {
            bold = lf.bold;
            italic = lf.italic;
            let family: String = lf
                .family
                .chars()
                .filter(|c| !matches!(c, '\'' | '"' | '<' | '>' | '&' | '\\') && !c.is_control())
                .collect();
            fam_owned = format!("'{family}', {fam}");
            &fam_owned
        }
        None => fam,
    };
    let anchor = match t.justify {
        Justify::Left => "start",
        Justify::Center => "middle",
        Justify::Right => "end",
    };
    let deg = -t.angle.to_degrees();
    let size = super::font_px(t.size);
    let _ = write!(
        out,
        "<text transform=\"translate({} {}){}\" font-family=\"{}\" font-size=\"{}\"{}{} text-anchor=\"{}\" fill=\"{}\" xml:space=\"preserve\">",
        n(t.origin.x),
        n(t.origin.y),
        if deg.abs() > 1e-9 { format!(" rotate({})", n(deg)) } else { String::new() },
        fam,
        n(size),
        if bold { " font-weight=\"bold\"" } else { "" },
        if italic { " font-style=\"italic\"" } else { "" },
        anchor,
        rgb(t.color)
    );
    let lines: Vec<&str> = t.text.split('\n').collect();
    if lines.len() == 1 {
        out.push_str(&text_body(t, &t.text));
    } else {
        for (i, l) in lines.iter().enumerate() {
            let _ = write!(
                out,
                "<tspan x=\"0\" y=\"{}\">{}</tspan>",
                n(i as f64 * size * figbase::text::LINE_SPACING),
                text_body(t, l)
            );
        }
    }
    out.push_str("</text>\n");
}

pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [
            c.first().copied().unwrap_or(0),
            c.get(1).copied().unwrap_or(0),
            c.get(2).copied().unwrap_or(0),
        ];
        let v = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let idx = |sh: u32| char::from(T.get(((v >> sh) & 63) as usize).copied().unwrap_or(b'A'));
        s.push(idx(18));
        s.push(idx(12));
        s.push(if c.len() > 1 { idx(6) } else { '=' });
        s.push(if c.len() > 2 { idx(0) } else { '=' });
    }
    s
}

pub(crate) fn image_png(img: &RasterImage) -> Option<Vec<u8>> {
    let size = tiny_skia::IntSize::from_wh(img.width, img.height)?;
    let mut data = img.rgba.clone();
    let expected = (img.width as usize).checked_mul(img.height as usize)?.checked_mul(4)?;
    if data.len() != expected {
        return None;
    }
    for [r, g, b, a] in data.as_chunks_mut::<4>().0.iter_mut() {
        let al = u16::from(*a);
        *r = ((u16::from(*r) * al + 127) / 255) as u8;
        *g = ((u16::from(*g) * al + 127) / 255) as u8;
        *b = ((u16::from(*b) * al + 127) / 255) as u8;
    }
    tiny_skia::Pixmap::from_vec(data, size)?.encode_png().ok()
}

/// Render a scene as an SVG document covering `bounds` (Fig units), sized
/// at `scale` (1.0 = actual size).
pub fn to_svg(scene: &Scene, bounds: &Rect, scale: f64, background: Option<Rgba>) -> String {
    to_svg_page(scene, bounds, scale, background, None, &Meta::default())
}

/// [`to_svg`] with `-z`: the size of the paper in points (gensvg_start sets
/// width and height to the paper's and keeps the viewBox of the figure).
pub fn to_svg_page(
    scene: &Scene,
    bounds: &Rect,
    scale: f64,
    background: Option<Rgba>,
    paper: Option<(i32, i32)>,
    meta: &Meta<'_>,
) -> String {
    let b = if bounds.is_empty() {
        Rect::new(Point::ZERO, Point::new(1.0, 1.0))
    } else {
        *bounds
    };
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let mut out = String::new();
    // gensvg_start: PREAMBLE, comments, <svg> with the size in points.
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n");
    let _ = writeln!(out, "<!-- Creator: {CREATOR} -->");
    if let Some(d) = &meta.date {
        let _ = writeln!(out, "<!-- CreationDate: {} -->", d.iso());
    }
    let _ = writeln!(out, "<!-- Magnification: {} -->", fmt_g(meta.mag, 3));
    let (w, h) = match paper {
        Some((w, h)) => (w, h),
        None => (
            (b.width() * 72.0 / 1200.0 * scale).ceil() as i32,
            (b.height() * 72.0 / 1200.0 * scale).ceil() as i32,
        ),
    };
    let _ = writeln!(
        out,
        "<svg\txmlns=\"http://www.w3.org/2000/svg\"\n\txmlns:xlink=\"http://www.w3.org/1999/xlink\"\n\twidth=\"{w}pt\" height=\"{h}pt\"\n\tviewBox=\"{} {} {} {}\">",
        n(b.min.x),
        n(b.min.y),
        n(b.width()),
        n(b.height())
    );
    for c in meta.comments {
        let _ = writeln!(out, "<desc>{}</desc>", sanitized(c));
    }
    out.push_str("<g fill=\"none\">\n");
    if let Some(bg) = background {
        let _ = writeln!(
            out,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
            n(b.min.x),
            n(b.min.y),
            n(b.width()),
            n(b.height()),
            rgb(bg)
        );
    }
    let mut clip_id = 0;
    for p in &scene.prims {
        match p {
            Prim::Path { subpaths, fill, stroke } => {
                let d = path_data(subpaths);
                if d.is_empty() {
                    continue;
                }
                match fill {
                    Some(Fill::Solid(c)) => {
                        let _ = writeln!(out, "<path d=\"{d}\" fill=\"{}\" fill-rule=\"evenodd\"/>", rgb(*c));
                    }
                    Some(Fill::Pattern { kind, fg, bg, nonzero }) => {
                        clip_id += 1;
                        let rule = if *nonzero { "nonzero" } else { "evenodd" };
                        let _ = writeln!(
                            out,
                            "<clipPath id=\"c{clip_id}\"><path d=\"{d}\" clip-rule=\"{rule}\"/></clipPath>"
                        );
                        let _ = writeln!(out, "<path d=\"{d}\" fill=\"{}\" fill-rule=\"{rule}\"/>", rgb(*bg));
                        let region = Rect::from_points(subpaths.iter().flat_map(|s| s.pts.iter().copied()));
                        let (lines, line_width) = coverage(*kind, &region, &PatternFrame::export(&b, scale));
                        let lines: Vec<SubPath> = lines
                            .into_iter()
                            .map(|pts| SubPath {
                                pts,
                                closed: false,
                                curve: None,
                            })
                            .collect();
                        let _ = writeln!(
                            out,
                            "<path clip-path=\"url(#c{clip_id})\" d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"/>",
                            path_data(&lines),
                            rgb(*fg),
                            n(line_width)
                        );
                    }
                    None => {}
                }
                if let Some(s) = stroke {
                    let _ = writeln!(out, "<path d=\"{d}\" fill=\"none\"{}/>", stroke_attrs(s));
                }
            }
            Prim::Text(t) => text(&mut out, t),
            Prim::PictureRef { .. } | Prim::PdfPicture { .. } => {}
            Prim::LinkedImage { file, p1, p3, flipped } => {
                linked_image(&mut out, file, *p1, *p3, *flipped);
            }
            Prim::Image {
                corners,
                image,
                flipped,
                ..
            } => {
                let Some(png) = image_png(image) else {
                    continue;
                };
                let [tl, tr, _, bl] = *corners;
                let (ex, ey) = if *flipped {
                    (bl - tl, tr - tl)
                } else {
                    (tr - tl, bl - tl)
                };
                let (w, h) = (f64::from(image.width.max(1)), f64::from(image.height.max(1)));
                let _ = writeln!(
                    out,
                    "<image width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" transform=\"matrix({} {} {} {} {} {})\" xlink:href=\"data:image/png;base64,{}\"/>",
                    image.width,
                    image.height,
                    n6(ex.x / w),
                    n6(ex.y / w),
                    n6(ey.x / h),
                    n6(ey.y / h),
                    n(tl.x),
                    n(tl.y),
                    base64(&png)
                );
            }
        }
    }
    out.push_str("</g>\n</svg>\n");
    out
}

/// `put_sanitized_char` over a comment.
fn sanitized(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            c => o.push(c),
        }
    }
    o
}

fn n6(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.6}")
    } else {
        "0".into()
    }
}

/// gensvg_line's picture: a link to the file, rotated and flipped by the
/// box's orientation.
fn linked_image(out: &mut String, file: &str, p1: (i32, i32), p3: (i32, i32), flipped: bool) {
    let (mut px, mut py) = p1;
    let mut width = p3.0.wrapping_sub(px);
    let mut height = p3.1.wrapping_sub(py);
    let mut rotation = match (width < 0, height < 0) {
        (true, true) => 180,
        (true, false) => 90,
        (false, true) => 270,
        (false, false) => 0,
    };
    if flipped {
        rotation -= 90;
    }
    if width < 0 {
        px = p3.0;
        width = width.wrapping_neg();
    }
    if height < 0 {
        py = p3.1;
        height = height.wrapping_neg();
    }
    let cx = px.wrapping_add(width / 2);
    let cy = py.wrapping_add(height / 2);
    let _ = writeln!(
        out,
        "<!-- Image -->\n<image xlink:href=\"file:{file}\" preserveAspectRatio=\"none\""
    );
    if flipped {
        let _ = writeln!(
            out,
            "transform=\"rotate({rotation} {cx} {cy}) scale(-1,1) translate({},0)\"",
            cx.wrapping_mul(2).wrapping_neg()
        );
    } else if rotation != 0 {
        let _ = writeln!(out, "transform=\"rotate({rotation} {cx} {cy})\"");
    }
    let _ = writeln!(out, "x=\"{px}\" y=\"{py}\" width=\"{width}\" height=\"{height}\"/>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_known() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b""), "");
    }

    #[test]
    fn escapes() {
        assert_eq!(esc("a<b>&\"c\u{1}"), "a&lt;b&gt;&amp;&quot;c");
    }
}
