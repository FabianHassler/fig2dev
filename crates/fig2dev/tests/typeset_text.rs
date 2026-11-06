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
    // The source is kept on the page, for reading the text back.
    assert!(pdf.contains("/RustFigLaTeX"), "the page lists its typeset texts");
    assert!(
        pdf.contains(r#""text":"$x^2$","font":0,"ps":false,"size":12"#),
        "{}",
        &pdf[pdf.find("/RustFigLaTeX").unwrap_or(0)..][..200.min(pdf.len())]
    );
    assert!(!plain.contains("/RustFigLaTeX"));
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
    assert!(svg.contains("data-rustfig-x=\"1200\" data-rustfig-y=\"1200\""));
    assert!(svg.contains("data-rustfig-justify=\"0\""));
}

/// With a typesetter, a special text counts in the page with the box TeX
/// gave it (72 x 36 pt here), not with its stored length (the width of the
/// source, or nothing for a text with `$` or `\`, as fig2dev's bounds do).
#[test]
fn the_page_fits_the_typeset_box() {
    const ONLY: &str = "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
4 0 0 50 -1 0 12 0.0000 2 150 4800 1200 1200 $\\\\alpha$\\001\n\
2 1 0 1 0 7 50 -1 -1 0.000 0 0 -1 0 0 2\n\t 1200 1200 1260 1200\n";
    let media = |pdf: &[u8]| -> (f64, f64) {
        let s = String::from_utf8_lossy(pdf);
        let i = s.find("/MediaBox [").unwrap() + 11;
        let v: Vec<f64> = s[i..]
            .split(']')
            .next()
            .unwrap()
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        (v[2] - v[0], v[3] - v[1])
    };
    let run_only = |ts: Option<Arc<Fixed>>| {
        let mut ctx = Ctx {
            lang: "pdf".into(),
            typesetter: ts.map(|t| t as Arc<dyn TextTypesetter>),
            ..Ctx::default()
        };
        let mut dev = drivers::driver("pdf").unwrap();
        let objects = readfp_fig(&mut ctx, ONLY.as_bytes()).map_err(|_| "read").unwrap();
        convert(&mut ctx, dev.as_mut(), &objects).unwrap().output
    };
    let (w, h) = media(&run_only(Some(Arc::new(Fixed::default()))));
    assert!((72.0..=75.0).contains(&w) && (36.0..=39.0).contains(&h), "{w} x {h}");
    // Without one: the line's box (the text's width is not counted).
    let (w, _) = media(&run_only(None));
    assert!(w < 10.0, "{w}");
    // SVG: the viewBox holds the box too (1200 + 72 pt in Fig units).
    let mut ctx = Ctx {
        lang: "svg".into(),
        typesetter: Some(Arc::new(Fixed::default()) as Arc<dyn TextTypesetter>),
        ..Ctx::default()
    };
    let mut dev = drivers::driver("svg").unwrap();
    let objects = readfp_fig(&mut ctx, ONLY.as_bytes()).map_err(|_| "read").unwrap();
    let svg = String::from_utf8_lossy(&convert(&mut ctx, dev.as_mut(), &objects).unwrap().output).into_owned();
    let i = svg.find("viewBox=\"").unwrap() + 9;
    let v: Vec<f64> = svg[i..]
        .split('"')
        .next()
        .unwrap()
        .split_whitespace()
        .map(|x| x.parse().unwrap())
        .collect();
    assert!(
        (1200.0..=1260.0).contains(&v[2]) && (600.0..=660.0).contains(&v[3]),
        "{v:?}"
    );
}

/// Hands out the pages of one two-page file (`rich.pdf`'s page twice), in
/// turn, and records what `prepare` was given.
#[derive(Debug)]
struct Shared {
    file: Arc<Vec<u8>>,
    next: Mutex<usize>,
    prepared: Mutex<Vec<String>>,
}

impl TextTypesetter for Shared {
    fn typeset(&self, _req: &TypesetRequest<'_>) -> Option<Typeset> {
        let mut n = self.next.lock().unwrap();
        let pdf = PdfPicture::open_page(self.file.clone(), *n % 2)?;
        *n += 1;
        Some(Typeset {
            pdf: Arc::new(pdf),
            width_pt: 72.0,
            height_pt: 36.0,
            depth_pt: 0.0,
            margin_pt: 0.0,
        })
    }

    fn prepare(&self, requests: &[TypesetRequest<'_>]) {
        self.prepared
            .lock()
            .unwrap()
            .extend(requests.iter().map(|r| r.text.to_string()));
    }
}

/// The special texts reach `prepare` before the first is typeset, and the
/// pages of one shared file keep their fonts once in the PDF.
#[test]
fn texts_are_prepared_together_and_share_their_fonts() {
    const TWO: &str = "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
4 0 0 50 -1 0 12 0.0000 2 150 300 1200 1200 $a$\\001\n\
4 0 0 50 -1 0 12 0.0000 2 150 300 1200 2400 $b$\\001\n";
    let rich = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdf_pictures/rich.pdf")).unwrap();
    let doc = hayro::hayro_syntax::Pdf::new(Arc::new(rich)).unwrap();
    let two_pages = Arc::new(hayro_write::extract_pages_to_pdf(&doc, &[0, 0]));
    let ts = Arc::new(Shared {
        file: two_pages,
        next: Mutex::new(0),
        prepared: Mutex::new(Vec::new()),
    });
    let mut ctx = Ctx {
        lang: "pdf".into(),
        typesetter: Some(ts.clone() as Arc<dyn TextTypesetter>),
        ..Ctx::default()
    };
    let mut dev = drivers::driver("pdf").unwrap();
    let objects = readfp_fig(&mut ctx, TWO.as_bytes()).map_err(|_| "read").unwrap();
    let pdf = String::from_utf8_lossy(&convert(&mut ctx, dev.as_mut(), &objects).unwrap().output).into_owned();
    let mut prepared = ts.prepared.lock().unwrap().clone();
    prepared.sort();
    assert_eq!(prepared, ["$a$", "$b$"]);
    assert_eq!(*ts.next.lock().unwrap(), 2, "both typeset");
    assert_eq!(
        pdf.matches("/BaseFont /Courier-Bold").count(),
        1,
        "one font object for both pages"
    );
}
