#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! SVG arrowheads: each line is clipped by its own arrowheads only.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};

fn svg(fig: &str) -> String {
    let mut ctx = Ctx {
        lang: "svg".into(),
        ..Ctx::default()
    };
    let mut dev = drivers::driver("svg").unwrap();
    let objects = readfp_fig(&mut ctx, fig.as_bytes()).unwrap();
    String::from_utf8(convert(&mut ctx, dev.as_mut(), &objects).unwrap().output).unwrap()
}

#[test]
fn a_line_is_not_clipped_by_the_previous_lines_arrowhead() {
    // A line with a backward arrow, then one with a forward arrow only. C
    // kept the first line's backward arrowhead in the second's clip path.
    let s = svg(
        "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
2 1 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 1 2\n\t1 1 1.00 60.00 120.00\n\t600 600 3000 600\n\
2 1 0 1 0 7 50 -1 -1 0.000 0 0 -1 1 0 2\n\t1 1 1.00 60.00 120.00\n\t600 1800 3000 1800\n",
    );
    let clips: Vec<&str> = s
        .split("<clipPath")
        .skip(1)
        .map(|c| &c[..c.find("</clipPath>").unwrap()])
        .collect();
    assert_eq!(clips.len(), 2, "{s}");
    for c in clips {
        // The page frame and one arrowhead.
        assert_eq!(c.matches('M').count(), 2, "{c}");
    }
}
