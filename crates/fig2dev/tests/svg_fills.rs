#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! SVG shades and tints of colours with red in them.

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
fn shades_and_tints_of_red() {
    // gensvg.c let red into the blue part: a shade of red came out #7f807f.
    let box_with = |fill: i32| {
        format!(
            "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
2 2 0 1 0 4 50 -1 {fill} 0.000 0 0 -1 0 0 5\n\t 600 600 1200 600 1200 1200 600 1200 600 600\n"
        )
    };
    assert!(
        svg(&box_with(10)).contains("fill=\"#7f0000\""),
        "{}",
        svg(&box_with(10))
    );
    assert!(
        svg(&box_with(30)).contains("fill=\"#ff7f7f\""),
        "{}",
        svg(&box_with(30))
    );
}
