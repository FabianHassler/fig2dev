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
