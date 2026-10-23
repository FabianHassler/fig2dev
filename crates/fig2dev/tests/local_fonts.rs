#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Installed fonts named by a text's `# rustfig-font: <Family Style>` comment
//! (RustFig plan F7): PDF and PostScript embed a subset, SVG names the
//! family, a missing family falls back to the Fig font with a warning. The
//! tests use a font found on the machine and skip without one.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};

fn fig(comment: &str) -> String {
    format!(
        "#FIG 3.2\nPortrait\nCenter\nInches\nA4\n100.00\nSingle\n-2\n1200 2\n{comment}4 0 0 50 -1 16 12 0.0 4 135 600 100 300 Hello\\001\n"
    )
}

fn run(lang: &str, fig: &str) -> (Vec<u8>, Vec<String>) {
    let mut ctx = Ctx {
        lang: lang.into(),
        ..Ctx::default()
    };
    let mut dev = drivers::driver(lang).unwrap();
    ctx.font_size *= ctx.fontmag;
    let objects = readfp_fig(&mut ctx, fig.as_bytes()).map_err(|_| "read").unwrap();
    let out = convert(&mut ctx, dev.as_mut(), &objects).unwrap().output;
    (out, ctx.messages.clone())
}

/// The name of an installed TrueType face that may be embedded.
fn installed_truetype() -> Option<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut names: Vec<String> = db
        .faces()
        .filter(|i| i.weight == fontdb::Weight::NORMAL && i.style == fontdb::Style::Normal)
        .filter(|i| i.stretch == fontdb::Stretch::Normal)
        .filter_map(|i| i.families.first().map(|(f, _)| f.clone()))
        .collect();
    names.sort();
    names
        .into_iter()
        .find(|n| figbase::text::find_local_font(n).is_some_and(|f| f.embeddable && !f.cff && f.face().is_some()))
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn installed_font_is_embedded_in_pdf_and_named_in_svg() {
    let Some(name) = installed_truetype() else {
        eprintln!("skipped: no installed TrueType font found");
        return;
    };
    let f = fig(&format!("# rustfig-font: {name}\n"));
    let (pdf, msgs) = run("pdf", &f);
    assert!(msgs.is_empty(), "{msgs:?}");
    assert!(contains(&pdf, "/FontFile2"), "no embedded font file");
    assert!(contains(&pdf, "/Identity-H"));
    let ps = figbase::text::find_local_font(&name).unwrap().post_script_name.clone();
    assert!(contains(&pdf, &format!("+{ps}")), "subset name +{ps} missing");
    let (svg, _) = run("svg", &f);
    let svg = String::from_utf8(svg).unwrap();
    assert!(svg.contains(&format!("font-family=\"'{name}', ")), "{svg}");
    let (eps, _) = run("eps", &f);
    let eps = String::from_utf8_lossy(&eps);
    assert!(
        eps.contains("/FontType 42 def") && eps.contains("/RFLocal0 findfont"),
        "{eps}"
    );
    // The LaTeX family keeps the Fig number.
    let (tex, _) = run("pstex_t", &f);
    assert!(!tex.is_empty());
}

#[test]
fn missing_family_falls_back_with_a_warning() {
    let f = fig("# rustfig-font: No Such Family Anywhere\n");
    for lang in ["pdf", "svg", "eps", "ps", "pstex_t"] {
        let (out, msgs) = run(lang, &f);
        assert!(!out.is_empty(), "{lang}");
        if lang == "pdf" {
            assert!(msgs.iter().any(|m| m.contains("not installed")), "{msgs:?}");
            assert!(contains(&out, "/Helvetica"), "Fig font expected");
            assert!(!contains(&out, "/FontFile2"));
        }
    }
}
