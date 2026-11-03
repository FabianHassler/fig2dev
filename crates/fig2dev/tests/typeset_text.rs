#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Special (LaTeX) texts drawn as the PDF page a typesetter made for them
//! (`Ctx::typesetter`): a vector picture in PDF, embedded EPS in
//! PostScript, pixels in the bitmaps; texts without a page stay text.

use fig2dev::scene::typeset::{TextTypesetter, Typeset, TypesetRequest};
use fig2dev::util::pdfpic::PdfPicture;
use fig2dev::{convert, drivers, read::readfp_fig, Ctx};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Gives `rich.pdf` (the picture tests' page) for every text, and keeps
/// the requests.
#[derive(Debug, Default)]
struct Fixed(Mutex<Vec<(String, f64)>>);

impl TextTypesetter for Fixed {
    fn typeset(&self, req: &TypesetRequest<'_>) -> Option<Typeset> {
        self.0.lock().unwrap().push((req.text.to_string(), req.size));
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdf_pictures/rich.pdf");
        let pdf = PdfPicture::open(std::fs::read(path).ok()?)?;
        Some(Typeset {
            pdf: Arc::new(pdf),
            width_pt: 72.0,
            height_pt: 36.0,
            depth_pt: 0.0,
            margin_pt: 0.0,
        })
    }
}

/// A special text ($x^2$, flags 2) and a plain one, both at 12 points.
const FIG: &str = "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
4 0 0 50 -1 0 12 0.0000 2 150 300 1200 1200 $x^2$\\001\n\
4 0 0 50 -1 0 12 0.0000 4 150 600 1200 2400 PLAINTEXT\\001\n";

fn run(lang: &str, ts: Option<Arc<Fixed>>) -> Vec<u8> {
    let mut ctx = Ctx {
        lang: lang.into(),
        typesetter: ts.map(|t| t as Arc<dyn TextTypesetter>),
        ..Ctx::default()
    };
    ctx.font_size *= ctx.fontmag;
    let mut dev = drivers::driver(lang).unwrap();
    let objects = readfp_fig(&mut ctx, FIG.as_bytes()).map_err(|_| "read").unwrap();
    convert(&mut ctx, dev.as_mut(), &objects).unwrap().output
}

#[test]
fn special_texts_become_typeset_pages() {
    let ts = Arc::new(Fixed::default());
    let pdf = String::from_utf8_lossy(&run("pdf", Some(ts.clone()))).into_owned();
    let forms = |s: &str| s.matches("/Form").count();
    let pdf_forms = forms(&pdf);
    // Only the special text was asked for, at the size it is drawn.
    let asked = ts.0.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].0, "$x^2$");
    assert!(asked[0].1 > 10.0 && asked[0].1 < 13.0, "{}", asked[0].1);
    // Without a typesetter: no picture.
    let plain = String::from_utf8_lossy(&run("pdf", None)).into_owned();
    assert!(pdf_forms > forms(&plain), "the page is a Form XObject");
}

#[test]
fn eps_embeds_the_typeset_page() {
    let ts = Arc::new(Fixed::default());
    let eps = String::from_utf8_lossy(&run("eps", Some(ts))).into_owned();
    assert!(eps.contains("% Begin typeset text"));
    assert!(eps.contains("PLAINTEXT"), "the plain text stays text");
    assert!(!eps.contains("(\\$x^2\\$)") && !eps.contains("($x^2$)"));
    let plain = String::from_utf8_lossy(&run("eps", None)).into_owned();
    assert!(!plain.contains("% Begin typeset text"));
}

#[test]
fn bitmaps_draw_the_typeset_page() {
    let ts = Arc::new(Fixed::default());
    let with = run("ppm", Some(ts));
    let without = run("ppm", None);
    assert_ne!(with, without);
}

#[test]
fn svg_draws_the_typeset_page_as_paths() {
    let ts = Arc::new(Fixed::default());
    let svg = String::from_utf8_lossy(&run("svg", Some(ts))).into_owned();
    assert!(svg.contains("matrix("), "the page is placed by a transform");
    assert!(svg.contains("PLAINTEXT"));
    assert!(
        !svg.contains(">$x^2$<"),
        "the special text is drawn as TeX's page, not as text"
    );
    // The source stays with the picture, for reading the text back.
    assert!(svg.contains("data-rustfig-text=\"$x^2$\""));
    assert!(svg.contains("data-rustfig-size=\"12\""));
}
