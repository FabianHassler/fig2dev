#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Where fig2dev 3.2.9a's drivers were wrong and this port is not: arc
//! arrowheads in PiCTeX, TextYL and epic, TikZ's cap style on open arcs,
//! turned ellipses in epic and PiCTeX.

mod tex_common;
use tex_common::run_with;

const HEAD: &str = "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n";

fn run(lang: &str, body: &str) -> String {
    run_with(lang, format!("{HEAD}{body}").as_bytes(), None, &[], false)
        .unwrap()
        .0
}

/// The numbers in `s` (signs and decimals kept).
fn numbers(s: &str) -> Vec<f64> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter_map(|t| t.parse().ok())
        .collect()
}

// A counter-clockwise arc from the right over the top to the left, with a
// forward arrow at the left end: the arc arrives there going down the page,
// so the arrowhead's base lies above its tip (C put it below).
const ARC: &str = "5 1 0 1 0 7 50 -1 -1 0.000 0 1 1 0 3000 3000 4000 3000 3000 2000 2000 3000\n\
\t1 1 1.00 120.00 240.00\n";

#[test]
fn pictex_arc_arrowhead_lies_on_the_arc() {
    let out = run("pictex", ARC);
    let line = out.lines().find(|l| l.starts_with("\\plot")).unwrap();
    // \plot wing tip wing /, y up.
    let n = numbers(line);
    assert!(n[1] > n[3] && n[5] > n[3], "{line}");
}

#[test]
fn textyl_arc_arrowhead_lies_on_the_arc() {
    let out = run("textyl", ARC);
    let line = out.lines().find(|l| l.contains("tyl line")).unwrap();
    // tyl line S <thickness> wing; tip, y up.
    let n = numbers(line);
    assert!(n[2] > n[4], "{line}");
}

#[test]
fn epic_small_arc_arrowhead_lies_on_the_arc() {
    // Radius 100, an arrowhead 400 long: the tangent case.
    let out = run(
        "epic",
        "5 1 0 1 0 7 50 -1 -1 0.000 0 1 1 0 3000 3000 3100 3000 3000 2900 2900 3000\n\t1 1 1.00 120.00 400.00\n",
    );
    let line = out.lines().find(|l| l.starts_with("\\blacken\\drawline")).unwrap();
    let n = numbers(line);
    // wing, tip, wing; y up.
    assert!(n[1] > n[3] && n[5] > n[3], "{line}");
}

#[test]
fn tikz_open_arc_keeps_its_round_cap() {
    let out = run(
        "tikz",
        "5 1 0 1 0 7 50 -1 -1 0.000 1 1 0 0 3000 3000 4000 3000 3000 2000 2000 3000\n",
    );
    assert!(out.contains("\\pgfsetroundcap"), "{out}");
}

#[test]
fn turned_ellipses_are_turned() {
    // 45°: a polygon; 90°: the radii swapped.
    let e45 = "1 1 0 1 0 7 50 -1 -1 0.000 1 0.7854 3000 3000 800 400 3000 3000 3800 3400\n";
    let e90 = "1 1 0 1 0 7 50 -1 -1 0.000 1 1.5708 3000 3000 800 400 3000 3000 3800 3400\n";
    let out = run("epic", e45);
    assert!(!out.contains("\\ellipse") && out.contains("\\drawline"), "{out}");
    assert!(run("pictex", e45).contains("POLYLINE"));
    // Radii 800 and 400 turned a quarter: taller than wide, \oval(w,h).
    let out = run("epic", e90);
    let l = out.lines().find(|l| l.contains("\\oval")).unwrap();
    let n = numbers(l);
    assert!(n.len() >= 2 && n[n.len() - 1] > n[n.len() - 2], "{l}");
}

#[test]
fn metafont_shaded_polyline_and_turned_ellipse() {
    let out = run(
        "mf",
        "2 2 0 1 0 7 50 -1 10 0.000 0 0 -1 0 0 5\n\t 600 600 1200 600 1200 1200 600 1200 600 600\n",
    );
    assert!(out.contains("unfilled polyline"), "{out}");
    assert!(!out.contains("unfilledpolyline"), "{out}");
    let out = run(
        "mf",
        "1 1 0 1 0 7 50 -1 -1 0.000 1 0.5236 3000 3000 600 300 3000 3000 3600 3300\n",
    );
    let l = out.lines().find(|l| l.contains("ellipse((")).unwrap();
    assert!(l.ends_with(",30.000000);") || l.contains(",30.0000"), "{l}");
}

#[test]
fn ge_splines_are_scaled_like_lines() {
    use fig2dev::object::{Compound, Point, Spline};
    // The reader turns splines into lines first, so the driver is called
    // directly: 1200 Fig units at 1200 ppi are 254 tenths of a millimetre
    // (genge.c wrote splines in Fig units).
    let mut ctx = fig2dev::Ctx {
        lang: "ge".into(),
        ..fig2dev::Ctx::default()
    };
    let mut dev = fig2dev::drivers::driver("ge").unwrap();
    let mut out = fig2dev::Out::new();
    dev.start(&mut ctx, &mut out, &Compound::default()).unwrap();
    let s = Spline {
        thickness: 1,
        points: vec![Point::new(1200, 1200), Point::new(2400, 1200), Point::new(2400, 2400)],
        ..Spline::default()
    };
    dev.spline(&mut ctx, &mut out, &s).unwrap();
    let text = String::from_utf8(out.buf).unwrap();
    assert!(text.contains("n 254.0 254.0 m"), "{text}");
}

#[test]
fn pictex_black_box_keeps_its_border_width() {
    // Thickness 3 is 2pt in PiCTeX; after the black fill the border gets it
    // back (genpictex.c wrote the Fig thickness as points: 3pt).
    let out = run(
        "pictex",
        "2 2 0 3 0 7 50 -1 20 0.000 0 0 -1 0 0 5\n\t 600 600 1800 600 1800 1200 600 1200 600 600\n",
    );
    let lines: Vec<&str> = out.lines().collect();
    let fill = lines
        .iter()
        .position(|l| l.contains("\\linethickness= 1.270cm"))
        .unwrap();
    assert_eq!(lines.get(fill - 2), Some(&"\\linethickness=2pt"), "{out}");
    assert_eq!(lines.get(fill + 2), Some(&"\\linethickness=2pt"), "{out}");
}
