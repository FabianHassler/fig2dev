use super::*;

const TOP: &str = "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n";

fn read(src: &str) -> (Result<Compound, i32>, Vec<String>, Ctx) {
    let mut ctx = Ctx::default();
    let r = readfp_fig(&mut ctx, src.as_bytes());
    let m = ctx.messages.clone();
    (r, m, ctx)
}

#[test]
fn header_sets_globals() {
    let src = "#FIG 3.2\nPortrait\nFlush left\nMetric\nA4 \n50.00\nMultiple\n3\n# fig comment\n1200 2\n";
    let (r, m, ctx) = read(src);
    let c = r.unwrap();
    assert!(m.is_empty(), "{m:?}");
    assert!(!ctx.landscape && !ctx.center && ctx.metric && ctx.multi_page);
    assert_eq!(ctx.papersize, "A4");
    assert_eq!((ctx.mag, ctx.fontmag), (0.5, 0.5));
    assert_eq!(ctx.gif_transparent, "#00ffff");
    assert_eq!((ctx.ppi, ctx.thick_scale), (1200.0, 15.0));
    assert!(ctx.v2_flag && ctx.v21_flag && ctx.v30_flag && ctx.v32_flag);
    assert_eq!(c.comments, vec!["fig comment".to_string()]);
}

#[test]
fn options_override_the_file() {
    let mut ctx = Ctx {
        orientspec: true,
        magspec: true,
        paperspec: true,
        mag: 2.0,
        papersize: "B5".into(),
        ..Ctx::default()
    };
    readfp_fig(&mut ctx, TOP.as_bytes()).unwrap();
    assert!(!ctx.landscape);
    assert_eq!((ctx.mag, ctx.papersize.as_str()), (2.0, "B5"));
}

#[test]
fn user_colours_are_renumbered() {
    let src = format!("{TOP}0 32 #ff8000\n0 40 #102030\n2 1 0 1 40 32 50 -1 20 0.0 0 0 -1 0 0 2\n 0 0 600 600\n");
    let (r, m, ctx) = read(&src);
    let c = r.unwrap();
    assert!(m.is_empty(), "{m:?}");
    let l = &c.lines[0];
    assert_eq!((l.pen_color, l.fill_color), (33, 32));
    assert_eq!(ctx.user_col_indx, vec![32, 40]);
    assert_eq!(ctx.color_rgb(33), Some((0x10, 0x20, 0x30)));
}

#[test]
fn texts_decode_escapes_and_encodings() {
    let src = format!("{TOP}4 0 0 50 -1 0 12 0.0 4 150 405 0 0 a\\\\b\\344\\001x\\001\n");
    let (r, m, _) = read(&src);
    let c = r.unwrap_or_else(|e| panic!("{e} {m:?}"));
    assert_eq!(c.texts[0].cstring, "a\\bä");
    // UTF-8 in a UTF-8 file.
    let src = format!("{TOP}4 0 0 50 -1 0 12 0.0 4 150 405 0 0 Grüße\\001\n");
    assert_eq!(read(&src).0.unwrap().texts[0].cstring, "Grüße");
    // An escaped end mark does not end the string.
    let src = format!("{TOP}4 0 0 50 -1 0 12 0.0 4 150 405 0 0 a\\\\001b\\001\n");
    assert_eq!(read(&src).0.unwrap().texts[0].cstring, "a\\001b");
    // Latin-2 by -E.
    let mut ctx = Ctx {
        input_encoding: Some("ISO-8859-2".into()),
        ..Ctx::default()
    };
    let mut src = format!("{TOP}4 0 0 50 -1 0 12 0.0 4 150 405 0 0 ").into_bytes();
    src.extend_from_slice(b"\xb1\\001\n");
    assert_eq!(readfp_fig(&mut ctx, &src).unwrap().texts[0].cstring, "ą");
}

#[test]
fn multi_line_text_keeps_newlines() {
    let src = format!("{TOP}4 0 0 50 -1 0 12 0.0 4 150 405 0 0 one\ntwo\\001\n");
    let c = read(&src).0.unwrap();
    assert_eq!(c.texts[0].cstring, "one\ntwo");
}

#[test]
fn picture_names_follow_the_fig_file() {
    let src = format!("{TOP}2 5 0 1 0 -1 50 -1 -1 0.0 0 0 -1 0 0 5\n 1 line.eps\n 0 0 600 0 600 300 0 300 0 0\n");
    let mut ctx = Ctx {
        from: Some("dir/sub/a.fig".into()),
        ..Ctx::default()
    };
    let c = readfp_fig(&mut ctx, src.as_bytes()).unwrap();
    let pic = c.lines[0].pic.as_ref().unwrap();
    assert_eq!((pic.file.as_str(), pic.flipped), ("dir/sub/line.eps", 1));
    let c = read(&src).0.unwrap();
    assert_eq!(c.lines[0].pic.as_ref().unwrap().file, "line.eps");
}

#[test]
fn arcs_are_sanitized() {
    let src = format!("{TOP}5 1 0 1 0 7 50 -1 -1 0.0 0 0 0 0 0.0 0.0 600 0 0 600 300 0\n");
    let c = read(&src).0.unwrap();
    assert_eq!(c.arcs[0].point[2], Point::new(600, 0));
    let src = format!("{TOP}5 1 0 1 0 7 50 -1 -1 0.0 0 0 0 0 0.0 0.0 0 0 0 600 300 0\n");
    let (r, m, _) = read(&src);
    assert_eq!(r, Err(-1));
    assert_eq!(
        m,
        vec!["Invalid arc object at line 10: arc radius smaller than one Fig unit."]
    );
}

#[test]
fn compounds_nest_and_cap() {
    let src = format!("{TOP}6 0 0 100 100\n6 0 0 100 100\n1 3 0 1 0 7 50 -1 -1 0.0 1 0.0 0 0 60 60 0 0 60 0\n-6\n-6\n");
    let c = read(&src).0.unwrap();
    assert_eq!(c.compounds[0].compounds[0].ellipses.len(), 1);
    let deep = format!("{TOP}{}", "6 0 0 1 1\n".repeat(MAX_NESTING + 1));
    let (r, m, _) = read(&deep);
    assert_eq!(r, Err(-1));
    assert!(m[0].starts_with("Compound objects nested deeper"));
}

#[test]
fn old_splines_keep_controls() {
    let src = "#FIG 3.1\nLandscape\nCenter\nInches\n1200 2\n3 2 0 1 0 7 50 -1 -1 0.0 0 0 0 2\n 0 0 1200 0\n 1 2 3 4\n 5 6 7 8\n";
    let (r, m, ctx) = read(src);
    let c = r.unwrap();
    assert!(m.is_empty(), "{m:?}");
    assert_eq!(ctx.papersize, "Letter");
    assert_eq!(c.splines[0].controls.len(), 2);
    assert_eq!(c.splines[0].controls[1].rx, 7.0);
}

#[test]
fn editing_mode_keeps_splines_blanks_and_the_background_transparency() {
    let src = format!(
        "{}\n4 0 0 50 -1 0 12 0.0000 4 135 405 600 600   two\\001\n3 4 0 1 0 7 50 -1 -1 0.000 0 0 0 3\n\t 0 0 600 600 1200 0\n\t 0.000 1.000 0.000\n",
        TOP.replace("-2\n", "-1\n").trim_end()
    );
    // fig2dev: the spline becomes a line, the text loses its blanks.
    let (r, _, ctx) = read(&src);
    let c = r.unwrap();
    assert_eq!((c.splines.len(), c.lines.len()), (0, 1));
    assert_eq!(c.texts[0].cstring, "two");
    assert_eq!(ctx.header_transparent, -1);
    // An editor: the spline stays, the text keeps its blanks beyond one.
    let mut ctx = Ctx {
        editing: true,
        ..Ctx::default()
    };
    let c = readfp_fig(&mut ctx, src.as_bytes()).unwrap();
    assert_eq!((c.splines.len(), c.lines.len()), (1, 0));
    assert_eq!(c.texts[0].cstring, "  two");
}

#[test]
fn editing_mode_reads_as_xfig_does() {
    // Kept as written: an arc off its circle, an open polygon, a distorted
    // box, an ellipse of radius 0, a 2-point polyline's fill; a NUL ends a
    // line; angles become 0 to 2 pi; a polygon of 2 points is deleted.
    let body = "5 1 0 1 0 7 50 -1 -1 0.000 0 1 0 0 100.000 0.000 200 0 100 100 0 0\n\
1 3 0 1 0 7 50 -1 -1 0.000 1 -1.0000 600 600 0 100 600 600 600 700\n\
2 3 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 3\n\t 0 0 100 0 100 100\n\
2 2 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 5\n\t 0 0 100 0 100 50 20 50 0 0\n\
2 1 0 1 0 7 50 -1 20 0.000 0 0 -1 0 0 2\n\t 0 0 100 0\n\
2 3 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 2\n\t 0 0 100 0\n";
    let src = format!("{TOP}{body}");
    let mut ctx = Ctx {
        editing: true,
        ..Ctx::default()
    };
    let c = readfp_fig(&mut ctx, src.as_bytes()).unwrap();
    assert_eq!(c.arcs.len(), 1);
    assert_eq!(c.arcs[0].point[2], Point::new(0, 0));
    assert_eq!(c.ellipses.len(), 1);
    assert!((c.ellipses[0].angle - (2.0 * std::f64::consts::PI - 1.0)).abs() < 1e-9);
    let kinds: Vec<(i32, usize, i32)> = c
        .lines
        .iter()
        .map(|l| (l.type_, l.points.len(), l.fill_style))
        .collect();
    // The open polygon (3 points) and the 5-point box stay, the filled
    // 2-point polyline stays filled, the 2-point polygon is deleted.
    assert_eq!(
        kinds,
        [(T_POLYGON, 3, -1), (T_BOX, 5, -1), (T_POLYLINE, 2, 20)],
        "{:?}",
        ctx.messages
    );
    assert!(
        ctx.messages.iter().any(|m| m.contains("Deleting polygon")),
        "{:?}",
        ctx.messages
    );
    // fig2dev refuses the radius 0 and repairs the others.
    let (r, _, _) = read(&src);
    assert!(r.is_err());

    let nul = format!("{TOP}4 0 0 50 -1 0 12 0.0000 4 135 405 600 600 ab\0cd\\001\n");
    let mut ctx = Ctx {
        editing: true,
        ..Ctx::default()
    };
    // The line ends at the NUL, so the text has no end mark: it takes the
    // next lines (none here), as xfig's.
    assert!(readfp_fig(&mut ctx, nul.as_bytes()).is_ok());
    assert!(read(&nul).0.is_err());
}

#[test]
fn repairs_are_functions_the_reader_uses() {
    use super::{apply_font_size, repair_arc, repair_line, trim_text_start, ArcFault, LineFault, LineNote};
    let line = |type_, pts: &[(i32, i32)]| Line {
        type_,
        fill_style: 20,
        points: pts.iter().map(|&(x, y)| Point::new(x, y)).collect(),
        ..Line::default()
    };
    // A box of three points (counting the closing one) is a polyline.
    let mut l = line(T_BOX, &[(0, 0), (5, 0), (0, 0)]);
    assert_eq!(repair_line(&mut l, 3), Ok(vec![LineNote::ToPolyline(3)]));
    assert_eq!((l.type_, l.points.len(), l.fill_style), (T_POLYLINE, 2, UNFILLED));
    // A distorted box is a polygon; an arc-box of three corners is refused.
    let mut l = line(T_BOX, &[(0, 0), (5, 1), (5, 5), (0, 5), (0, 0)]);
    assert_eq!(repair_line(&mut l, 5), Ok(vec![LineNote::ToPolygonDistorted]));
    assert_eq!(l.type_, T_POLYGON);
    let mut l = line(T_ARC_BOX, &[(0, 0), (5, 0), (5, 5), (0, 0)]);
    assert_eq!(repair_line(&mut l, 4), Err(LineFault::Corners(3)));
    // Arcs: radius below one unit, end on the centre, end moved onto the circle.
    let arc = |p: [(i32, i32); 3]| Arc {
        center: DPoint { x: 0.0, y: 0.0 },
        point: p.map(|(x, y)| Point::new(x, y)),
        ..Arc::default()
    };
    assert_eq!(
        repair_arc(&mut arc([(0, 0), (5, 5), (9, 9)])),
        Err(ArcFault::SmallRadius)
    );
    assert_eq!(
        repair_arc(&mut arc([(10, 0), (5, 5), (0, 0)])),
        Err(ArcFault::EndAtCenter)
    );
    let mut a = arc([(10, 0), (0, 10), (0, 20)]);
    assert_eq!(repair_arc(&mut a), Ok(()));
    assert_eq!(a.point[2], Point::new(0, 10));
    // Texts.
    let mut t = Text {
        size: 0.0,
        ..Text::default()
    };
    apply_font_size(0.0, &mut t);
    assert_eq!(t.size, DEFAULT_FONT_SIZE);
    t.size = 10.0;
    t.length = 100.0;
    apply_font_size(20.0, &mut t);
    assert_eq!((t.size, t.length), (20.0, 200.0));
    assert_eq!(trim_text_start(" \t x"), "x");
    assert_eq!(trim_text_start("\n x"), "\n x");
}
