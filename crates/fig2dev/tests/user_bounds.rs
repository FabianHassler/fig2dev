#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `Ctx::user_bounds`: a program's fixed export box replaces the objects'
//! bounds (RustFig's bounding box); without it the output is fig2dev's.

use fig2dev::{read, Ctx};

const FIG: &str = "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
2 2 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 5\n\
\t 1200 1200 2400 1200 2400 2400 1200 2400 1200 1200\n";

fn convert(lang: &str, bounds: Option<(i32, i32, i32, i32)>) -> String {
    let mut dev = fig2dev::drivers::driver(lang).unwrap();
    let mut ctx = Ctx::default();
    ctx.general_option('L', Some(lang)).unwrap();
    ctx.user_bounds = bounds;
    let objects = read::readfp_fig(&mut ctx, FIG.as_bytes()).unwrap();
    let status = fig2dev::convert(&mut ctx, dev.as_mut(), &objects).unwrap();
    String::from_utf8_lossy(&status.output).into_owned()
}

fn line<'a>(out: &'a str, prefix: &str) -> &'a str {
    out.lines()
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("{prefix} in {out}"))
}

#[test]
fn the_box_sizes_the_page() {
    // fig2dev's own bounds: the square, 1 inch.
    assert_eq!(
        line(&convert("eps", None), "%%BoundingBox:"),
        "%%BoundingBox: 0 0 74 74"
    );
    // A 3 by 2 inch box around it.
    let eps = convert("eps", Some((0, 0, 3600, 2400)));
    assert_eq!(line(&eps, "%%BoundingBox:"), "%%BoundingBox: 0 0 216 144");
    let svg = convert("svg", Some((0, 0, 3600, 2400)));
    assert!(
        line(&svg, "<svg").contains("viewBox=\"0 0 3600 2400\"") || svg.contains("viewBox=\"0 0 3600 2400\""),
        "{svg}"
    );
    // tikz clips to the box.
    let tikz = convert("tikz", Some((0, 0, 3600, 2400)));
    assert!(tikz.contains("\\clip("), "{tikz}");
}

#[test]
fn eps_clips_to_a_fixed_box() {
    // The square reaches past a box over its left half.
    let eps = convert("eps", Some((0, 0, 1800, 3600)));
    assert!(eps.contains("rectclip"), "{eps}");
    assert!(!convert("eps", None).contains("rectclip"));
}
