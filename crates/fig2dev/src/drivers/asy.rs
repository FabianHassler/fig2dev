//! The `asy` driver: the figure as an Asymptote program (`asy -f pdf
//! figure.asy`). fig2dev has no such driver; this one is original to this
//! crate, built on the same driver framework as the others.
//!
//! Coordinates are Fig units with y negated, drawn at `unitsize(S bp)`
//! (`S = 72 * magnification / ppi`, so 0.06 bp for a 1200 ppi figure at
//! 100 %); line widths, dashes and font sizes are in bp. Polylines, boxes,
//! polygons, rounded boxes, arcs, circles and ellipses are `fill`s and
//! `draw`s of paths with explicit cap, join and dash pens; X-splines
//! arrive from the reader as polylines, older splines are converted the
//! same way. Arrowheads are paths computed as `calc_arrow` does, and the
//! shaft is clipped (even-odd) so that it does not poke through a hollow or
//! pointed head. Colours are `rgb("rrggbb")` (user colours are pens named
//! `xfigcN`), shades and tints are mixed here, patterns 41 to 62 are
//! tilings (`patterns.asy`, one per pattern and pen colour, drawn from the
//! PostScript driver's tiles). Texts are `label`s set on their baseline:
//! PostScript fonts as `font(encoding, family, series, shape)` with the
//! same `\usefont` arguments the TeX drivers use, LaTeX fonts as
//! `fontcommand`s, special (TeX) texts unquoted and all others with
//! LaTeX's special characters quoted. Pictures are `graphic(…)` labels.
//!
//! Options: `-b` border, `-F` no font family/series/shape, `-f` default
//! font, `-i`/`-w` for picture names, `-O` all texts are TeX, `-o` no font
//! size, `-T` only TeX fonts, `-G` grid.

use std::collections::HashSet;
use std::f64::consts::PI;

use figbase::color::{area_fill_color, Rgb};

use crate::bound::{calc_arrow, compute_arcarrow_angle, ArrowPoints};
use crate::gendev::{strtol, Ctx};
use crate::object::{
    Arc, Arrow, Compound, Ellipse, Line, Point, Spline, Text, DASH_2_DOTS_LINE, DASH_3_DOTS_LINE, DASH_DOT_LINE,
    DASH_LINE, DOTTED_LINE, MAX_FONT, MAX_PSFONT, P_EPS, SOLID_LINE, SPECIAL_TEXT, T_ARC_BOX, T_BOX, T_LEFT_JUSTIFIED,
    T_OPEN_ARC, T_PIC_BOX, T_PIE_WEDGE_ARC, T_POLYGON, T_POLYLINE, T_RIGHT_JUSTIFIED, UNFILLED,
};
use crate::scene::build::color;
use crate::scene::pattern::tile;
use crate::trans_spline::{create_line_with_spline, make_control_factors};
use crate::util::psfonts::{unpsfont_clear, PS_FONT_NAMES};
use crate::util::texfonts::{put_string, tex_font_index, TexFonts};
use crate::{fprintf, Driver, Out};

/// Lines of points are broken after this many characters.
const MAX_LINE: usize = 78;
/// Paths with more points than this are built from an array.
const LONG_PATH: usize = 200;
/// Kappa for a quarter circle as a Bézier curve.
const KAPPA: f64 = 0.552_284_749_830_793_4;
/// How far the clip box around a shaft reaches beyond the figure, in Fig
/// units.
const CLIP_MARGIN: i32 = 4000;

/// The `asy` driver's state.
#[derive(Debug)]
pub struct Asy {
    fonts: TexFonts,
    select_fontsize: bool,
    select_fontname: bool,
    only_texfonts: bool,
    allspecial: bool,
    removesuffix: bool,
    prepend: Option<String>,
    /// `-b`: the blank border in bp.
    border: f64,
    /// bp per Fig unit.
    unit: f64,
    thick_scale: f64,
    fontmag: f64,
    /// Patterns (number, pen colour) already defined.
    defined: HashSet<(i32, [u8; 3])>,
    /// The figure's bounds in Fig units, with the border.
    bounds: (i32, i32, i32, i32),
}

impl Default for Asy {
    fn default() -> Asy {
        Asy {
            fonts: TexFonts::new(),
            select_fontsize: true,
            select_fontname: true,
            only_texfonts: false,
            allspecial: false,
            removesuffix: false,
            prepend: None,
            border: 0.0,
            unit: 0.06,
            thick_scale: 1.0,
            fontmag: 1.0,
            defined: HashSet::new(),
            bounds: (0, 0, 0, 0),
        }
    }
}

/// The driver for `lang`.
pub fn new(lang: &str) -> Option<Box<dyn Driver>> {
    (lang == "asy").then(|| Box::new(Asy::default()) as Box<dyn Driver>)
}

/// What fills a shape.
enum Fill {
    None,
    Solid(Rgb),
    Pattern { kind: i32, fg: Rgb, bg: Rgb },
}

/// `atoi`: the leading integer, 0 if none.
fn atoi(s: &str) -> i32 {
    strtol(s).0
}

/// A number with at most `prec` decimals, trailing zeros and a lone minus
/// zero removed.
fn num(v: f64, prec: usize) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    let s = format!("{v:.prec$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s.as_str()
    };
    if s == "-0" || s.is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// `THICKNESS(T)` of fig2dev: the drawn width of a line of thickness `t`
/// (both in Fig units).
fn drawn_thickness(t: f64, thick_scale: f64) -> f64 {
    if t <= thick_scale {
        0.5 * t
    } else {
        t - thick_scale
    }
}

/// The comments of an object as `//` lines.
fn comments(out: &mut Out, comments: &[crate::object::Comment]) {
    for c in comments {
        out.puts("// ");
        out.bytes(&c.raw);
        out.puts("\n");
    }
}

/// A Fig point as an Asymptote pair (y turned upwards).
fn pt(p: Point) -> String {
    format!("({},{})", p.x, p.y.wrapping_neg())
}

/// The pair for real coordinates in Fig units.
fn pt_f(x: f64, y: f64) -> String {
    format!("({},{})", num(x, 3), num(-y, 3))
}

/// `rgb("rrggbb")`.
fn rgb_lit(c: Rgb) -> String {
    format!("rgb(\"{:02x}{:02x}{:02x}\")", c.r, c.g, c.b)
}

/// Points joined by `sep`, broken into lines.
fn join_points(items: impl Iterator<Item = String>, sep: &str, indent: &str) -> String {
    let mut s = String::new();
    let mut line = indent.len();
    for (i, it) in items.enumerate() {
        if i > 0 {
            s.push_str(sep);
            line += sep.len();
        }
        if line > MAX_LINE {
            s.push('\n');
            s.push_str(indent);
            line = indent.len();
        }
        line += it.len();
        s.push_str(&it);
    }
    s
}

/// A polygon or polyline through `pts`.
fn poly_path(pts: &[Point], closed: bool, indent: &str) -> String {
    // `--cycle` closes the path: its repeated first point is left out.
    let pts = match (pts.first(), pts.last()) {
        (Some(a), Some(b)) if closed && pts.len() > 2 && a == b => pts.get(..pts.len() - 1).unwrap_or(pts),
        _ => pts,
    };
    // asy's parser recurses once per `--`: a long chain overflows its
    // stack (at about 1,000 points), so long paths are built from an array.
    let mut s = if pts.len() > LONG_PATH {
        let list = join_points(pts.iter().map(|&p| pt(p)), ",", indent);
        format!("operator --(...new pair[]{{{list}}})")
    } else {
        join_points(pts.iter().map(|&p| pt(p)), "--", indent)
    };
    if closed {
        s.push_str("--cycle");
    }
    s
}

/// An Asymptote string literal (single quotes: C escapes, so the TeX
/// backslashes are doubled).
fn asy_string(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('\'');
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\'' => o.push_str("\\'"),
            '\n' | '\r' | '\t' => o.push(' '),
            c if c.is_control() => {}
            c => o.push(c),
        }
    }
    o.push('\'');
    o
}

/// The arguments of `\usefont{enc}{family}{series}{shape}` as an Asymptote
/// `font(…)` call.
fn font_call(usefont: &str) -> Option<String> {
    let inner = usefont.strip_prefix('{')?.strip_suffix('}')?;
    let parts: Vec<&str> = inner.split("}{").collect();
    if parts.len() != 4 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    Some(format!("font(\"{}\")", parts.join("\",\"")))
}

/// Whether a picture is PostScript (the reader leaves the subtype alone,
/// so the file name says).
fn is_eps(p: &crate::object::Pic) -> bool {
    if p.subtype != P_EPS {
        return false;
    }
    let name = p.file.to_ascii_lowercase();
    name.ends_with(".eps") || name.ends_with(".epsi") || name.ends_with(".epsf") || name.ends_with(".ps")
}

/// What the figure needs from the header: patterns, pictures that LaTeX
/// can only include with pdfLaTeX.
fn scan(com: &Compound, pats: &mut bool, pdf_pics: &mut bool) {
    *pats |= com.lines.iter().any(|l| l.fill_style > 40)
        || com.ellipses.iter().any(|e| e.fill_style > 40)
        || com.arcs.iter().any(|a| a.fill_style > 40)
        || com.splines.iter().any(|s| s.fill_style > 40);
    *pdf_pics |= com
        .lines
        .iter()
        .any(|l| l.type_ == T_PIC_BOX && l.pic.as_ref().is_some_and(|p| !is_eps(p)));
    for c in &com.compounds {
        scan(c, pats, pdf_pics);
    }
}

impl Asy {
    fn bp(&self, fig: f64) -> f64 {
        fig * self.unit
    }

    /// The drawn width of a line, in bp.
    fn width_bp(&self, thickness: f64) -> f64 {
        self.bp(drawn_thickness(thickness, self.thick_scale))
    }

    /// The dash pattern of a line style in bp, empty for solid.
    fn dashes(&self, ctx: &Ctx, style: i32, style_val: f64) -> Vec<f64> {
        if style == SOLID_LINE || style_val <= 0.0 || !style_val.is_finite() {
            return Vec::new();
        }
        let v = style_val * ctx.ppi / 80.0;
        let dot = ctx.ppi / 80.0;
        let d: Vec<f64> = match style {
            DASH_LINE => vec![v, v],
            DOTTED_LINE => vec![dot, v],
            DASH_DOT_LINE => vec![v, 0.5 * v, dot, 0.5 * v],
            DASH_2_DOTS_LINE => vec![v, 0.45 * v, dot, 0.333 * v, dot, 0.45 * v],
            DASH_3_DOTS_LINE => vec![v, 0.4 * v, dot, 0.3 * v, dot, 0.3 * v, dot, 0.4 * v],
            _ => Vec::new(),
        };
        d.into_iter().map(|x| self.bp(x)).collect()
    }

    /// The pen of a stroked path.
    #[allow(clippy::too_many_arguments)]
    fn stroke_pen(
        &self,
        ctx: &Ctx,
        thickness: i32,
        pen_color: i32,
        style: i32,
        style_val: f64,
        cap: i32,
        join: i32,
    ) -> String {
        let mut p = format!(
            "{}+linewidth({})+linecap({})+linejoin({})",
            self.pen_color(color(ctx, pen_color), pen_color),
            num(self.width_bp(f64::from(thickness)), 4),
            cap.clamp(0, 2),
            join.clamp(0, 2)
        );
        let d = self.dashes(ctx, style, style_val);
        if !d.is_empty() {
            let v: Vec<String> = d.iter().map(|&x| num(x, 3)).collect();
            p.push_str(&format!("+linetype(new real[]{{{}}},0,false,false)", v.join(",")));
        }
        p
    }

    /// The pen for a colour: a user colour by its name, a standard one as
    /// a literal.
    fn pen_color(&self, c: Rgb, number: i32) -> String {
        if number >= crate::gendev::NUM_STD_COLS as i32 {
            format!("xfigc{number}")
        } else {
            rgb_lit(c)
        }
    }

    /// The fill of a shape.
    fn fill_of(&self, ctx: &Ctx, area_fill: i32, pen: i32, fill_color: i32) -> Fill {
        if area_fill == UNFILLED || area_fill < 0 {
            return Fill::None;
        }
        if area_fill > 40 {
            return if area_fill <= 62 {
                Fill::Pattern {
                    kind: area_fill,
                    fg: color(ctx, pen),
                    bg: color(ctx, fill_color),
                }
            } else {
                Fill::None
            };
        }
        area_fill_color(color(ctx, fill_color), fill_color <= 0, fill_color == 7, area_fill)
            .map_or(Fill::None, Fill::Solid)
    }

    /// The definition of the tiling for pattern `kind` in colour `fg`, once.
    fn pattern_def(&mut self, out: &mut Out, kind: i32, fg: Rgb) -> String {
        let name = format!("xfigp{}_{:02x}{:02x}{:02x}", kind - 41, fg.r, fg.g, fg.b);
        if !self.defined.insert((kind, [fg.r, fg.g, fg.b])) {
            return name;
        }
        let Some(t) = tile(kind) else {
            return name;
        };
        fprintf!(out, "// Fill pattern {kind}\n{{\n  picture t;\n");
        let pen = format!(
            "{}+linewidth({})+linecap(0)+linejoin(0)",
            rgb_lit(fg),
            num(t.line_width, 3)
        );
        for l in &t.lines {
            let p = join_points(
                l.iter().map(|&(x, y)| format!("({},{})", num(x, 3), num(y, 3))),
                "--",
                "    ",
            );
            fprintf!(out, "  draw(t,{p},{pen});\n");
        }
        let (w, h) = (num(t.width, 3), num(t.height, 3));
        fprintf!(out, "  draw(t,box((0,0),({w},{h})),invisible);\n");
        fprintf!(out, "  clip(t,box((0,0),({w},{h})));\n");
        fprintf!(out, "  add(\"{name}\",t);\n}}\n");
        name
    }

    /// The statements that fill path `g`.
    fn fill_stmts(&mut self, out: &mut Out, fill: &Fill, closed: bool, stmts: &mut Vec<String>) {
        // Asymptote fills cyclic paths only; PostScript closes them.
        let g = if closed { "g" } else { "g--cycle" };
        match fill {
            Fill::None => {}
            Fill::Solid(c) => stmts.push(format!("fill({g},{});", rgb_lit(*c))),
            Fill::Pattern { kind, fg, bg } => {
                let name = self.pattern_def(out, *kind, *fg);
                stmts.push(format!("fill({g},{});", rgb_lit(*bg)));
                stmts.push(format!("fill({g},pattern(\"{name}\"));"));
            }
        }
    }

    /// An arrowhead from `from` to `to` on a line of `thickness`.
    fn arrow_points(&self, ctx: &Ctx, from: Point, to: Point, thickness: i32, a: &Arrow) -> ArrowPoints {
        calc_arrow(ctx, from.x, from.y, to.x, to.y, thickness, a)
    }

    /// The statements that draw an arrowhead.
    fn head_stmts(&self, ap: &ArrowPoints, a: &Arrow, pen_color: Rgb, pen_number: i32, stmts: &mut Vec<String>) {
        if ap.points.len() < 2 {
            return;
        }
        let kind = a.type_;
        let closed = kind != 0 && kind < 13 && kind != 6;
        let pen = self.pen_color(pen_color, pen_number);
        let draw_pen = format!(
            "{pen}+linewidth({})+linecap(0)+linejoin(0)+miterlimit(8)",
            num(self.width_bp(a.thickness), 4)
        );
        let path = poly_path(&ap.points, closed, "    ");
        // The outline of the open styles is not filled; a hollow head is
        // white, a full one has the pen colour.
        let fill = match kind {
            0 | 13 | 14 => None,
            9 => Some("white".to_string()),
            _ if a.style == 1 => Some(pen.clone()),
            _ => Some("white".to_string()),
        };
        match fill {
            Some(f) if closed => stmts.push(format!("filldraw({path},{f},{draw_pen});")),
            Some(f) => {
                // The half circle: filled across its chord, stroked open.
                stmts.push(format!("fill({path}--cycle,{f});"));
                stmts.push(format!("draw({path},{draw_pen});"));
            }
            None => stmts.push(format!("draw({path},{draw_pen});")),
        }
        if !ap.fillpoints.is_empty() {
            let part = poly_path(&ap.fillpoints, true, "    ");
            stmts.push(format!("filldraw({part},{pen},{draw_pen});"));
        }
    }

    /// The clip path that removes the arrowheads' areas from a shaft.
    fn clip_path(&self, heads: &[&ArrowPoints]) -> Option<String> {
        let polys: Vec<String> = heads
            .iter()
            .filter(|h| h.clippts.len() > 2)
            .map(|h| format!("({})", poly_path(&h.clippts, true, "    ")))
            .collect();
        if polys.is_empty() {
            return None;
        }
        let (llx, lly, urx, ury) = self.bounds;
        let m = CLIP_MARGIN;
        let outer = format!(
            "box(({},{}),({},{}))",
            llx.saturating_sub(m),
            ury.saturating_neg().saturating_sub(m),
            urx.saturating_add(m),
            lly.saturating_neg().saturating_add(m)
        );
        Some(format!("{outer}^^{}", polys.join("^^")))
    }

    /// Writes one object: the fill, the stroke (clipped when arrowheads
    /// need it) and the arrowheads. `path` is an Asymptote path expression.
    #[allow(clippy::too_many_arguments)]
    fn shape(
        &mut self,
        out: &mut Out,
        path: &str,
        closed: bool,
        fill: &Fill,
        stroke: Option<&str>,
        heads: Vec<(ArrowPoints, Arrow)>,
        pen_color: Rgb,
        pen_number: i32,
    ) {
        let mut stmts = Vec::new();
        self.fill_stmts(out, fill, closed, &mut stmts);
        let refs: Vec<&ArrowPoints> = heads.iter().map(|h| &h.0).collect();
        let clip = if stroke.is_some() { self.clip_path(&refs) } else { None };
        if let Some(pen) = stroke {
            if let Some(clip) = &clip {
                stmts.push(format!(
                    "{{\n    picture p;\n    draw(p,g,{pen});\n    clip(p,{clip},evenodd);\n    add(p);\n  }}"
                ));
            } else {
                stmts.push(format!("draw(g,{pen});"));
            }
            for (ap, a) in &heads {
                self.head_stmts(ap, a, pen_color, pen_number, &mut stmts);
            }
        }
        match stmts.as_slice() {
            [] => {}
            [one] if one.starts_with("draw(g,") => {
                // A single statement on the path itself.
                out.puts(&one.replacen("(g,", &format!("({path},"), 1));
                out.puts("\n");
            }
            _ => {
                fprintf!(out, "{{\n  path g={path};\n");
                for s in &stmts {
                    fprintf!(out, "  {s}\n");
                }
                out.puts("}\n");
            }
        }
    }

    /// A rounded rectangle with corner radius `r` through the corners
    /// `p` and `q` (Fig points), as a path.
    fn round_box(p: Point, q: Point, r: i32) -> String {
        let (x0, x1) = (f64::from(p.x.min(q.x)), f64::from(p.x.max(q.x)));
        let (y0, y1) = (-f64::from(p.y.max(q.y)), -f64::from(p.y.min(q.y)));
        let r = f64::from(r).min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
        let k = KAPPA * r;
        let c = |x: f64, y: f64| format!("({},{})", num(x, 2), num(y, 2));
        let mut s = String::new();
        s.push_str(&format!("{}--{}\n    ", c(x0 + r, y0), c(x1 - r, y0)));
        s.push_str(&format!(
            "..controls {} and {}..{}--{}\n    ",
            c(x1 - r + k, y0),
            c(x1, y0 + r - k),
            c(x1, y0 + r),
            c(x1, y1 - r)
        ));
        s.push_str(&format!(
            "..controls {} and {}..{}--{}\n    ",
            c(x1, y1 - r + k),
            c(x1 - r + k, y1),
            c(x1 - r, y1),
            c(x0 + r, y1)
        ));
        s.push_str(&format!(
            "..controls {} and {}..{}--{}\n    ",
            c(x0 + r - k, y1),
            c(x0, y1 - r + k),
            c(x0, y1 - r),
            c(x0, y0 + r)
        ));
        s.push_str(&format!(
            "..controls {} and {}..{}--cycle",
            c(x0, y0 + r - k),
            c(x0 + r - k, y0),
            c(x0 + r, y0)
        ));
        s
    }

    /// A picture: a `graphic` label centred in its box.
    fn picture(&self, out: &mut Out, p: Point, r: Point, pic: &crate::object::Pic) {
        let mut dx = r.x.wrapping_sub(p.x);
        let mut dy = r.y.wrapping_sub(p.y);
        let mut rot = 0;
        if dx < 0 && dy < 0 {
            dx = dx.wrapping_neg();
            dy = dy.wrapping_neg();
            rot = 180;
        } else if dx < 0 && dy >= 0 {
            rot = dy;
            dy = dx.wrapping_neg();
            dx = rot;
            rot = 270;
        } else if dy < 0 && dx >= 0 {
            rot = dx;
            dx = dy.wrapping_neg();
            dy = rot;
            rot = 90;
        }
        if pic.flipped != 0 {
            rot += 90;
            std::mem::swap(&mut dx, &mut dy);
        }
        let name = pic.name_bytes().split(|&b| b == 0).next().unwrap_or_default();
        let name = if self.removesuffix {
            name.iter()
                .rposition(|&c| c == b'.')
                .map_or(name, |i| name.get(..i).unwrap_or(name))
        } else {
            name
        };
        let name = format!(
            "{}{}",
            self.prepend.as_deref().unwrap_or(""),
            String::from_utf8_lossy(name)
        );
        let opts = format!(
            "width={}bp,height={}bp",
            num(self.bp(f64::from(dx)), 3),
            num(self.bp(f64::from(dy)), 3)
        );
        let mut g = format!("graphic({},{})", asy_string(&name), asy_string(&opts));
        if pic.flipped != 0 {
            g = format!("{}+{g}+{}", asy_string("\\reflectbox{"), asy_string("}"));
        }
        // The picture turns about the centre of its box, which stays where
        // the corners `p` and `r` put it.
        let rot = rot.rem_euclid(360);
        let mid = pt_f(
            (f64::from(p.x) + f64::from(r.x)) / 2.0,
            (f64::from(p.y) + f64::from(r.y)) / 2.0,
        );
        let label = format!("Label({g},{mid},NoAlign)");
        if rot != 0 {
            fprintf!(out, "label(rotate({rot})*{label});\n");
        } else {
            fprintf!(out, "label({label});\n");
        }
    }

    /// The shaft's end points for the arrowheads of a polyline.
    fn line_heads(
        &self,
        ctx: &Ctx,
        pts: &[Point],
        thickness: i32,
        fwd: Option<&Arrow>,
        back: Option<&Arrow>,
    ) -> Vec<(ArrowPoints, Arrow)> {
        let mut heads = Vec::new();
        let n = pts.len();
        if let (Some(a), Some(&from), Some(&to)) = (fwd, n.checked_sub(2).and_then(|i| pts.get(i)), pts.last()) {
            heads.push((self.arrow_points(ctx, from, to, thickness, a), *a));
        }
        if let (Some(a), Some(&from), Some(&to)) = (back, pts.get(1), pts.first()) {
            heads.push((self.arrow_points(ctx, from, to, thickness, a), *a));
        }
        heads
    }
}

impl Driver for Asy {
    fn option(&mut self, ctx: &mut Ctx, opt: char, arg: Option<&str>) -> Result<(), String> {
        let a = arg.unwrap_or("");
        match opt {
            // blank border around the figure (1/72 inch)
            'b' => self.border = f64::from(atoi(a).max(0)),
            'F' => self.select_fontname = false,
            // prepend this string to included graphics files
            'i' => self.prepend = Some(a.to_string()),
            // the default text font
            'f' => {
                let tex = (1..=MAX_FONT as usize).find(|&i| {
                    self.fonts.names.get(i).is_some_and(|n| n == a) || self.fonts.fonts.get(i).is_some_and(|n| n == a)
                });
                if let Some(i) = tex {
                    if let Some(v) = self.fonts.fonts.get(i).cloned() {
                        self.fonts.fonts[0] = v;
                    }
                } else if let Some(i) = (1..=MAX_PSFONT as usize).find(|&i| PS_FONT_NAMES.get(i) == Some(&a)) {
                    // (The C drivers store the font's name where the
                    // `\usefont` arguments belong.)
                    if let Some(v) = self.fonts.psfonts.get(i).cloned() {
                        self.fonts.psfonts[0] = v;
                    }
                } else {
                    ctx.warn(format!("warning: non-standard font name {a} ignored"));
                }
            }
            'O' => self.allspecial = true,
            'o' => self.select_fontsize = false,
            'T' => self.only_texfonts = true,
            // remove the suffix from included graphics files
            'w' => self.removesuffix = true,
            'G' | 'L' => {}
            _ => return Err(format!("Argument -{opt} unknown to asy driver.")),
        }
        Ok(())
    }

    fn start(&mut self, ctx: &mut Ctx, out: &mut Out, objects: &Compound) -> Result<(), String> {
        self.unit = 72.0 * ctx.mag / ctx.ppi;
        self.thick_scale = f64::from(ctx.thick_scale);
        self.fontmag = ctx.fontmag;
        let border_fig = if self.unit > 0.0 {
            (self.border / self.unit).round() as i32
        } else {
            0
        };
        ctx.llx = ctx.llx.saturating_sub(border_fig);
        ctx.lly = ctx.lly.saturating_sub(border_fig);
        ctx.urx = ctx.urx.saturating_add(border_fig);
        ctx.ury = ctx.ury.saturating_add(border_fig);
        self.bounds = (ctx.llx, ctx.lly, ctx.urx, ctx.ury);
        let (mut pats, mut pdf_pics) = (ctx.pats_used, false);
        scan(objects, &mut pats, &mut pdf_pics);

        out.puts("// Asymptote picture written by fig2dev (language asy).\n");
        out.puts("// Run with: asy -f pdf <file>.asy  (texts and pictures are set by TeX).\n");
        if !objects.comments.is_empty() {
            comments(out, &objects.comments);
        }
        if pats {
            out.puts("import patterns;\n");
        }
        if pdf_pics {
            // PNG, JPEG and the other bitmaps need pdfLaTeX.
            out.puts("settings.tex=\"pdflatex\";\n");
        }
        fprintf!(out, "unitsize({}bp);\n", num(self.unit, 6));
        out.puts("labelmargin=0;\n");
        for (i, u) in ctx.user_colors.iter().enumerate() {
            fprintf!(
                out,
                "pen xfigc{}=rgb(\"{:02x}{:02x}{:02x}\");\n",
                i + crate::gendev::NUM_STD_COLS,
                u.r.clamp(0, 255),
                u.g.clamp(0, 255),
                u.b.clamp(0, 255)
            );
        }
        let (llx, lly, urx, ury) = self.bounds;
        fprintf!(
            out,
            "// The figure's bounds.\ndraw(box(({},{}),({},{})),invisible);\n",
            llx,
            ury.saturating_neg(),
            urx,
            lly.saturating_neg()
        );
        Ok(())
    }

    fn grid(&mut self, ctx: &mut Ctx, out: &mut Out, mut major: f32, mut minor: f32) -> Result<(), String> {
        if minor == 0.0 && major == 0.0 {
            return Ok(());
        }
        if ctx.metric {
            // Cancel out the factor by which the figure is multiplied.
            minor = (f64::from(minor) * (3.81 / 4.)) as f32;
            major = (f64::from(major) * (3.81 / 4.)) as f32;
        }
        let (llx, lly, urx, ury) = self.bounds;
        let ts = self.thick_scale;
        for (step, w) in [(minor, 0.5 * ts), (major, 1.25 * ts)] {
            if step <= 0.0 || !step.is_finite() {
                continue;
            }
            let pen = format!("gray(0.7)+linewidth({})", num(self.bp(w), 3));
            fprintf!(
                out,
                "for(real x={llx}; x<={urx}; x+={}) draw((x,{})--(x,{}),{pen});\n",
                num(f64::from(step), 3),
                ury.saturating_neg(),
                lly.saturating_neg()
            );
            fprintf!(
                out,
                "for(real y={lly}; y<={ury}; y+={}) draw(({llx},-y)--({urx},-y),{pen});\n",
                num(f64::from(step), 3)
            );
        }
        Ok(())
    }

    fn arc(&mut self, ctx: &mut Ctx, out: &mut Out, a: &Arc) -> Result<(), String> {
        comments(out, &a.comments);
        let heads_wanted = a.thickness > 0 && (a.for_arrow.is_some() || a.back_arrow.is_some());
        if a.thickness <= 0 && a.fill_style == UNFILLED {
            return Ok(());
        }
        let [p0, _, p2] = a.point;
        let (cx, cy) = (a.center.x, a.center.y);
        let (d1x, d1y) = (f64::from(p0.x) - cx, cy - f64::from(p0.y));
        let (d2x, d2y) = (f64::from(p2.x) - cx, cy - f64::from(p2.y));
        let rad = 0.5 * (d1x.hypot(d1y) + d2x.hypot(d2y));
        if !(rad.is_finite() && rad > 0.0) {
            return Ok(());
        }
        let a1 = d1y.atan2(d1x) * 180.0 / PI;
        let a2 = d2y.atan2(d2x) * 180.0 / PI;
        let dir = if a.direction != 0 { "CCW" } else { "CW" };
        let mut path = format!(
            "arc({},{},{},{},{dir})",
            pt_f(cx, cy),
            num(rad, 3),
            num(a1, 4),
            num(a2, 4)
        );
        if a.type_ == T_PIE_WEDGE_ARC {
            path.push_str(&format!("--{}--cycle", pt_f(cx, cy)));
        }
        let fill = self.fill_of(ctx, a.fill_style, a.pen_color, a.fill_color);
        let stroke = (a.thickness > 0)
            .then(|| self.stroke_pen(ctx, a.thickness, a.pen_color, a.style, a.style_val, a.cap_style, 0));
        let mut heads = Vec::new();
        if heads_wanted && a.type_ == T_OPEN_ARC {
            if let Some(fa) = &a.for_arrow {
                let (x, y) = compute_arcarrow_angle(ctx, cx, cy, f64::from(p2.x), f64::from(p2.y), a.direction, fa);
                heads.push((self.arrow_points(ctx, Point::new(x, y), p2, a.thickness, fa), *fa));
            }
            if let Some(ba) = &a.back_arrow {
                let (x, y) = compute_arcarrow_angle(ctx, cx, cy, f64::from(p0.x), f64::from(p0.y), a.direction ^ 1, ba);
                heads.push((self.arrow_points(ctx, Point::new(x, y), p0, a.thickness, ba), *ba));
            }
        }
        let pen = color(ctx, a.pen_color);
        self.shape(
            out,
            &path,
            a.type_ == T_PIE_WEDGE_ARC,
            &fill,
            stroke.as_deref(),
            heads,
            pen,
            a.pen_color,
        );
        Ok(())
    }

    fn ellipse(&mut self, ctx: &mut Ctx, out: &mut Out, e: &Ellipse) -> Result<(), String> {
        comments(out, &e.comments);
        if e.thickness <= 0 && e.fill_style == UNFILLED {
            return Ok(());
        }
        let (rx, ry) = (e.radiuses.x, e.radiuses.y);
        let c = pt_f(f64::from(e.center.x), f64::from(e.center.y));
        let path = if e.angle == 0.0 || rx == ry {
            if rx == ry {
                format!("shift({c})*scale({rx})*unitcircle")
            } else {
                format!("shift({c})*scale({rx},{ry})*unitcircle")
            }
        } else {
            format!(
                "shift({c})*rotate({})*scale({rx},{ry})*unitcircle",
                num(e.angle * 180.0 / PI, 3)
            )
        };
        let fill = self.fill_of(ctx, e.fill_style, e.pen_color, e.fill_color);
        let cap = i32::from(e.style == DOTTED_LINE);
        let stroke =
            (e.thickness > 0).then(|| self.stroke_pen(ctx, e.thickness, e.pen_color, e.style, e.style_val, cap, 0));
        let pen = color(ctx, e.pen_color);
        self.shape(out, &path, true, &fill, stroke.as_deref(), Vec::new(), pen, e.pen_color);
        Ok(())
    }

    fn line(&mut self, ctx: &mut Ctx, out: &mut Out, l: &Line) -> Result<(), String> {
        comments(out, &l.comments);
        let Some(&p) = l.points.first() else {
            return Ok(());
        };
        let q = l.points.get(1).copied();
        let r = l.points.get(2).copied();
        let pen_rgb = color(ctx, l.pen_color);

        // One or two points: only a polyline is drawn.
        let (Some(_), Some(r)) = (q, r) else {
            if l.thickness <= 0 || l.type_ != T_POLYLINE {
                return Ok(());
            }
            let stroke = self.stroke_pen(
                ctx,
                l.thickness,
                l.pen_color,
                l.style,
                l.style_val,
                l.cap_style,
                l.join_style,
            );
            let end = q.unwrap_or(p);
            // A zero-length line shows only with round caps.
            if end == p && l.cap_style != 1 {
                return Ok(());
            }
            let heads = self.line_heads(ctx, &[p, end], l.thickness, l.for_arrow.as_ref(), l.back_arrow.as_ref());
            let path = format!("{}--{}", pt(p), pt(end));
            self.shape(
                out,
                &path,
                false,
                &Fill::None,
                Some(&stroke),
                heads,
                pen_rgb,
                l.pen_color,
            );
            return Ok(());
        };
        if l.type_ == T_PIC_BOX {
            if let Some(pic) = &l.pic {
                self.picture(out, p, r, pic);
            }
            return Ok(());
        }
        let has_arrow = l.for_arrow.is_some() || l.back_arrow.is_some();
        if l.thickness <= 0 && l.fill_style == UNFILLED && !has_arrow {
            return Ok(());
        }
        let fill = self.fill_of(ctx, l.fill_style, l.pen_color, l.fill_color);
        let stroke = (l.thickness > 0).then(|| {
            self.stroke_pen(
                ctx,
                l.thickness,
                l.pen_color,
                l.style,
                l.style_val,
                l.cap_style,
                l.join_style,
            )
        });
        let mut heads = Vec::new();
        let path = match l.type_ {
            T_BOX => format!("box({},{})", pt(p), pt(r)),
            T_ARC_BOX => Asy::round_box(p, r, l.radius),
            T_POLYGON => {
                // Polygons repeat the first point at the end.
                let n = if l.points.last() == Some(&p) && l.points.len() > 1 {
                    l.points.len() - 1
                } else {
                    l.points.len()
                };
                poly_path(l.points.get(..n).unwrap_or(&l.points), true, "    ")
            }
            _ => {
                if l.thickness > 0 {
                    heads = self.line_heads(ctx, &l.points, l.thickness, l.for_arrow.as_ref(), l.back_arrow.as_ref());
                }
                poly_path(&l.points, false, "    ")
            }
        };
        self.shape(
            out,
            &path,
            l.type_ != T_POLYLINE,
            &fill,
            stroke.as_deref(),
            heads,
            pen_rgb,
            l.pen_color,
        );
        Ok(())
    }

    /// Splines of files before 3.2 (the reader turns the others into
    /// polylines): drawn as the X-spline of their kind.
    fn spline(&mut self, ctx: &mut Ctx, out: &mut Out, s: &Spline) -> Result<(), String> {
        let mut s = s.clone();
        make_control_factors(&mut s);
        match create_line_with_spline(ctx, &s) {
            Some(l) => self.line(ctx, out, &l),
            None => {
                comments(out, &s.comments);
                ctx.warn("Unable to convert spline to line, dropped.");
                Ok(())
            }
        }
    }

    fn text(&mut self, ctx: &mut Ctx, out: &mut Out, t: &Text) -> Result<(), String> {
        comments(out, &t.comments);
        if t.hidden_text() || t.raw.is_empty() {
            return Ok(());
        }
        let mut t = t.clone();
        if self.allspecial {
            t.flags |= SPECIAL_TEXT;
        }
        if self.only_texfonts {
            unpsfont_clear(ctx, &mut t);
        }
        let mut pen = self.pen_color(color(ctx, t.color), t.color);
        if self.select_fontsize {
            let size = t.size * if t.rigid_text() { 1.0 } else { self.fontmag };
            if size.is_finite() && size > 0.0 {
                pen.push_str(&format!("+fontsize({})", num(size, 2)));
            }
        }
        if self.select_fontname {
            if t.psfont_text() {
                let i = if t.font <= MAX_PSFONT {
                    usize::try_from(t.font + 1).unwrap_or(0)
                } else {
                    0
                };
                if let Some(f) = self.fonts.psfonts.get(i).and_then(|f| font_call(f)) {
                    pen.push('+');
                    pen.push_str(&f);
                }
            } else if let Some(f) = self.fonts.fonts.get(tex_font_index(t.font)).filter(|f| !f.is_empty()) {
                pen.push_str(&format!("+fontcommand({})", asy_string(f)));
            }
        }
        let mut tmp = Out::new();
        put_string(&mut tmp, ctx, &t.raw, t.font, t.special_text());
        let text = String::from_utf8_lossy(&tmp.buf);
        // A box without height or depth puts the baseline on the anchor.
        let s = asy_string(&format!("\\raisebox{{0pt}}[0pt][0pt]{{{text}}}"));
        let align = match t.type_ {
            T_LEFT_JUSTIFIED => "E",
            T_RIGHT_JUSTIFIED => "W",
            _ => "NoAlign",
        };
        let pos = pt(Point::new(t.base_x, t.base_y));
        if t.angle != 0.0 && t.angle.is_finite() {
            fprintf!(
                out,
                "label(rotate({})*Label({s},{pos},{align},{pen}));\n",
                num(t.angle * 180.0 / PI, 2)
            );
        } else {
            fprintf!(out, "label({s},{pos},{align},{pen});\n");
        }
        Ok(())
    }

    fn end(&mut self, _ctx: &mut Ctx, _out: &mut Out) -> Result<i32, String> {
        Ok(0)
    }

    fn text_include(&self) -> bool {
        true
    }
}
