#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The header metadata of the eps, ps, pdf and svg outputs (genps_start,
//! gensvg_start, creationdate.c): `%%Title`, `%%Creator`, `%%CreationDate`,
//! `%%Magnification`, the whole-figure comments and the PDF Info. The
//! expected lines are fig2dev 3.2.9a's with `SOURCE_DATE_EPOCH=86400`.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};
use std::sync::Once;

static INIT: Once = Once::new();

fn run(lang: &str, fig: &str, opts: &[(char, Option<&str>)]) -> Vec<u8> {
    INIT.call_once(|| std::env::set_var("SOURCE_DATE_EPOCH", "86400"));
    let mut ctx = Ctx {
        lang: lang.into(),
        ..Ctx::default()
    };
    let mut dev = drivers::driver(lang).unwrap();
    for &(c, a) in opts {
        if !ctx.general_option(c, a).unwrap() {
            dev.option(&mut ctx, c, a).unwrap();
        }
    }
    ctx.font_size *= ctx.fontmag;
    let objects = readfp_fig(&mut ctx, fig.as_bytes()).map_err(|_| "read").unwrap();
    convert(&mut ctx, dev.as_mut(), &objects).unwrap().output
}

fn text(lang: &str, fig: &str, opts: &[(char, Option<&str>)]) -> String {
    String::from_utf8_lossy(&run(lang, fig, opts)).into_owned()
}

const BODY: &str = "2 2 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 5\n\t 0 0 2250 0 2250 1125 0 1125 0 0\n";

fn fig(unit: &str, comments: &str, extra: &str) -> String {
    format!("#FIG 3.2\n{comments}Portrait\nCenter\n{unit}\nA4\n100.00\nSingle\n-2\n1200 2\n{extra}{BODY}")
}

fn head(s: &str) -> String {
    let end = s.find("%%EndComments\n").unwrap() + "%%EndComments\n".len();
    s[..end].to_string()
}

#[test]
fn eps_header_as_genps() {
    let s = text("eps", &fig("Inches", "", ""), &[('n', Some("Name"))]);
    assert_eq!(
        head(&s),
        "%!PS-Adobe-3.0 EPSF-3.0\n%%Title: Name\n%%Creator: fig2dev Version 3.2.9a\n\
         %%CreationDate: 1970-01-02 00:00:00\n%%BoundingBox: 0 0 137 69\n\
         %%Magnification: 1.0000\n%%EndComments\n"
    );
}

#[test]
fn title_is_stdin_without_a_name() {
    let s = text("eps", &fig("Inches", "", ""), &[]);
    assert!(s.contains("%%Title: stdin\n"), "{}", head(&s));
}

#[test]
fn ps_header_has_pages_paper_and_metric_magnification() {
    let s = text("ps", &fig("Metric", "", ""), &[('m', Some("1.5"))]);
    assert_eq!(
        head(&s),
        "%!PS-Adobe-3.0\n%%Title: stdin\n%%Creator: fig2dev Version 3.2.9a\n\
         %%CreationDate: 1970-01-02 00:00:00\n%%Pages: 1\n%%BoundingBox: 0 0 595 842\n\
         %%DocumentPaperSizes: a4\n%%Magnification: 1.5000\n%%EndComments\n"
    );
}

#[test]
fn whole_figure_comments_follow_the_header() {
    let s = text("ps", &fig("Inches", "# skipped\n# first <&>\n# second\n", ""), &[]);
    let after = s.split("%%EndSetup\n").nth(1).unwrap();
    assert!(
        after.starts_with("%\n%  first <&> \n%  second \n%\n%%BeginProlog\n"),
        "{after:.80}"
    );
    let s = text("eps", &fig("Inches", "# skipped\n# one\n", ""), &[]);
    assert!(s.contains("%%EndComments\n%\n%  one \n%\n%%BeginProlog\n"));
}

#[test]
fn non_ascii_text_names_the_cmap() {
    let t = "4 0 0 50 -1 0 12 0.0000 4 135 300 100 500 h\\351\\001\n";
    let s = text("eps", &fig("Inches", "", t), &[]);
    assert!(head(&s).contains(
        "%%DocumentNeededResources: ProcSet (CIDInit)\n%%+ CMap (Xfig-UTF8-H)\n\
         %%DocumentSuppliedResources: CMap (Xfig-UTF8-H)\n%%EndComments\n"
    ));
}

#[test]
fn svg_header_as_gensvg() {
    let s = text(
        "svg",
        &fig("Inches", "# skipped\n# a <b> & c\n", ""),
        &[('m', Some("0.5"))],
    );
    let want = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n\
                <!-- Creator: fig2dev Version 3.2.9a -->\n\
                <!-- CreationDate: 1970-01-02 00:00:00 -->\n\
                <!-- Magnification: 0.5 -->\n\
                <svg\txmlns=\"http://www.w3.org/2000/svg\"\n\
                \txmlns:xlink=\"http://www.w3.org/1999/xlink\"\n\
                \twidth=\"69pt\" height=\"35pt\"\n\
                \tviewBox=\"-12 -12 2274 1149\">\n\
                <desc>a &lt;b&gt; &amp; c</desc>\n\
                <g fill=\"none\">\n";
    assert!(s.starts_with(want), "{s:.400}");
    assert!(s.ends_with("</g>\n</svg>\n"));
}

#[test]
fn svg_paper_size_in_points() {
    let s = text("svg", &fig("Inches", "", ""), &[('z', Some("a4"))]);
    assert!(s.contains("\twidth=\"595pt\" height=\"842pt\"\n"), "{s:.300}");
}

#[test]
fn pdf_info_has_title_creator_and_dates() {
    let b = run("pdf", &fig("Inches", "", ""), &[('n', Some("T1"))]);
    let s = String::from_utf8_lossy(&b);
    for want in [
        "/Title (T1)",
        "/Creator (fig2dev Version 3.2.9a)",
        "/CreationDate (D:19700102000000)",
        "/ModDate (D:19700102000000)",
    ] {
        assert!(s.contains(want), "{want}");
    }
}
