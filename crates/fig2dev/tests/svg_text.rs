#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! SVG text as gensvg.c writes it: Symbol and ZapfDingbats through the
//! Unicode tables, special text as `tspan`s, and `-z`.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};

fn run(font: i32, flags: i32, text: &str, extra: &[(char, Option<&str>)]) -> Result<String, String> {
    let fig = format!(
        "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
4 0 0 50 -1 {font} 12 0.0 {flags} 150 900 600 600 {text}\\001\n"
    );
    let mut ctx = Ctx {
        lang: "svg".into(),
        ..Ctx::default()
    };
    let mut dev = drivers::driver("svg").unwrap();
    for &(c, a) in extra {
        if !ctx.general_option(c, a)? {
            dev.option(&mut ctx, c, a)?;
        }
    }
    ctx.font_size *= ctx.fontmag;
    let objects = readfp_fig(&mut ctx, fig.as_bytes()).map_err(|e| e.to_string())?;
    let out = convert(&mut ctx, dev.as_mut(), &objects)?.output;
    Ok(String::from_utf8(out).unwrap())
}

/// What lies between `<text ...>` and `</text>`.
fn body(svg: &str) -> &str {
    let a = svg.find("<text ").unwrap();
    let a = a + svg[a..].find('>').unwrap() + 1;
    &svg[a..svg.find("</text>").unwrap()]
}

#[test]
fn symbol_codes_become_unicode() {
    let s = run(32, 4, "abAB\\242", &[]).unwrap();
    // alpha beta Alpha Beta; 0242 is the prime.
    assert_eq!(body(&s), "&#945;&#946;&#913;&#914;&#8242;");
}

#[test]
fn dingbat_codes_become_unicode() {
    let s = run(34, 4, "ab", &[]).unwrap();
    assert_eq!(body(&s), "&#10049;&#10050;");
}

#[test]
fn plain_text_is_unchanged() {
    let s = run(0, 4, "a^2 <b> &", &[]).unwrap();
    assert_eq!(body(&s), "a^2 &lt;b&gt; &amp;");
}

#[test]
fn special_text_shifts_with_tspans() {
    let s = run(0, 6, "x^2 y_{ab}z$q$", &[]).unwrap();
    assert_eq!(
        body(&s),
        "x<tspan font-size=\"96\" baseline-shift=\"super\">2</tspan> y<tspan font-size=\"96\" baseline-shift=\"sub\">ab</tspan>zq"
    );
}

#[test]
fn special_text_edge_cases_stay_well_formed() {
    // C leaves the tspan open at the end of the text, and writes a NUL
    // after a final `}`; here both are closed.
    let s = run(0, 6, "e^b", &[]).unwrap();
    assert_eq!(body(&s), "e<tspan font-size=\"96\" baseline-shift=\"super\">b</tspan>");
    let s = run(0, 6, "e^{b}", &[]).unwrap();
    assert_eq!(body(&s), "e<tspan font-size=\"96\" baseline-shift=\"super\">b</tspan>");
    // A trailing `^` is dropped; nesting closes at the innermost `}`.
    let s = run(0, 6, "d^", &[]).unwrap();
    assert_eq!(body(&s), "d");
    let s = run(0, 6, "a^{b_{c}}d", &[]).unwrap();
    assert_eq!(
        body(&s),
        "a<tspan font-size=\"96\" baseline-shift=\"super\">b<tspan font-size=\"96\" baseline-shift=\"sub\">c</tspan>}d</tspan>"
    );
}

#[test]
fn paper_size_sets_width_and_height() {
    let s = run(0, 4, "x", &[('z', Some("A4"))]).unwrap();
    // The figure is landscape: 842 x 595 points, viewBox kept.
    assert!(s.contains("width=\"842pt\" height=\"595pt\""), "{s}");
    let e = run(0, 4, "x", &[('z', Some("nosuch"))]).unwrap_err();
    assert_eq!(e, "Unknown paper size `nosuch'");
}

#[test]
fn text_attributes_as_gensvg_writes_them() {
    // Family by font / 4, italic by font % 2, bold by font % 4.
    let s = run(18, 4, "x", &[]).unwrap();
    assert!(
        s.contains(
            "<!-- Text -->\n<text xml:space=\"preserve\" x=\"600\" y=\"600\" fill=\"#000000\" font-family=\"Helvetica\" font-style=\"normal\" font-weight=\"bold\" font-size=\"200\" text-anchor=\"start\">x</text>\n"
        ),
        "{s}"
    );
    let s = run(32, 4, "a", &[]).unwrap();
    assert!(
        s.contains("font-family=\"Times,Symbol\" font-style=\"normal\" font-weight=\"normal\""),
        "{s}"
    );
}

#[test]
fn symbol_text_is_looked_up_by_character() {
    // U+00B1 in UTF-8 is the bytes C2 B1. C looks each byte up (0xC2 is
    // the real-part sign); a UTF-8 text is one code per character here, as
    // the PostScript driver writes it, so RustFig's Symbol texts come out
    // right. A Latin-1 file still goes byte by byte.
    let s = run(32, 4, "\\302\\261", &[]).unwrap();
    assert_eq!(body(&s), "&#177;");
    let s = run(32, 4, "\\345", &[]).unwrap();
    assert_eq!(body(&s), "&#8721;");
}

#[test]
fn elements_as_gensvg_writes_them() {
    let fig = "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
2 2 0 3 0 7 50 -1 20 0.0 0 0 -1 0 0 5\n\t 100 100 400 100 400 400 100 400 100 100\n\
2 1 1 1 1 7 50 -1 -1 3.0 1 1 0 0 0 2\n\t 100 100 400 100\n\
1 3 0 0 0 4 50 -1 10 0.0 1 0.0 1000 1000 200 200 1000 1000 1200 1000\n";
    let mut ctx = Ctx {
        lang: "svg".into(),
        ..Ctx::default()
    };
    let mut dev = drivers::driver("svg").unwrap();
    let objects = readfp_fig(&mut ctx, fig.as_bytes()).unwrap();
    let svg = String::from_utf8(convert(&mut ctx, dev.as_mut(), &objects).unwrap().output).unwrap();
    // Box: a `rect`; the reader scales the thickness 3 to 45 (1200 ppi), the
    // stroke is 45 - 15 = 30 px.
    assert!(
        svg.contains(
            "<!-- Line -->\n<rect x=\"100\" y=\"100\" width=\"300\" height=\"300\" fill=\"#ffffff\"\n\tstroke=\"#000000\" stroke-width=\"30px\"/>\n"
        ),
        "{svg}"
    );
    // Dashed polyline with round joins and caps.
    assert!(
        svg.contains(
            "<!-- Line -->\n<polyline points=\" 100,100 400,100\"\n\tstroke=\"#0000ff\" stroke-width=\"8px\" stroke-linejoin=\"round\" stroke-linecap=\"round\" stroke-dasharray=\"30 30\"/>\n"
        ),
        "{svg}"
    );
    // A circle with no outline, filled with half of colour 4 (red).
    assert!(
        svg.contains("<!-- Circle -->\n<circle cx=\"1000\" cy=\"1000\" r=\"200\" fill=\"#7f0000\"/>\n"),
        "{svg}"
    );
}
