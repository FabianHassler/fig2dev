#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! PDF and EPS files as picture objects: a PDF page stays a vector picture
//! in PDF output (a Form XObject), becomes EPS in PostScript output (C's
//! `pdftops`), is rendered in the bitmap languages; EPS files go through
//! ghostscript where it is installed. `rich.pdf` (text, a dashed curve, a
//! sampled image, clips) and `shade.pdf` (a shading, which PostScript
//! Level 2 can't draw) were written for these tests with ghostscript's
//! `pdfwrite` from `rich.eps` and a `shfill` program.

use fig2dev::util::pdfpic::PdfPicture;
use fig2dev::{convert, drivers, read::readfp_fig, Ctx};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdf_pictures")
}

fn fig(file: &str, flipped: u8) -> String {
    format!(
        "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t {flipped} {file}\n\t 1200 1200 3600 1200 3600 3600 1200 3600 1200 1200\n"
    )
}

/// (output bytes, messages)
fn run(lang: &str, file: &str, flipped: u8) -> (Vec<u8>, Vec<String>) {
    let mut ctx = Ctx {
        lang: lang.into(),
        dir: Some(fixtures()),
        ..Ctx::default()
    };
    ctx.font_size *= ctx.fontmag;
    let mut dev = drivers::driver(lang).unwrap();
    let objects = readfp_fig(&mut ctx, fig(file, flipped).as_bytes())
        .map_err(|_| "read")
        .unwrap();
    let out = convert(&mut ctx, dev.as_mut(), &objects).unwrap().output;
    (out, ctx.messages)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// The pixel (x, y) of a rendering of `pdf`, as RGBA.
fn render(pdf: Vec<u8>, w: u32, h: u32) -> impl Fn(u32, u32) -> [u8; 4] {
    let img = PdfPicture::open(pdf).expect("a PDF").render(w, h).expect("renders");
    move |x, y| {
        let i = ((y * img.width + x) * 4) as usize;
        [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
    }
}

#[test]
fn pdf_picture_stays_a_vector_in_pdf_output() {
    let (pdf, msgs) = run("pdf", "rich.pdf", 0);
    assert!(msgs.is_empty(), "{msgs:?}");
    let t = text(&pdf);
    assert!(t.contains("/Subtype /Form"), "the page is a Form XObject");
    assert!(t.contains("Pdf0"));
    // the output is a PDF of its own: the picture's background is there
    let at = render(pdf, 100, 100);
    // the figure is the picture's box (2 in = 144 pt) at 100 px: its light
    // blue ground covers the box
    let px = at(50, 50);
    assert!(px[3] > 0 && px[2] > 200, "{px:?}");
}

#[test]
fn pdf_picture_in_postscript_is_converted_to_eps() {
    let (ps, msgs) = run("eps", "rich.pdf", 0);
    assert!(msgs.is_empty(), "{msgs:?}");
    let t = text(&ps);
    assert!(t.contains("%%BeginDocument: rich.pdf"));
    assert!(t.contains("%%BoundingBox: 0 0 200 150"));
    assert!(t.contains("curveto") && t.contains("setdash") && t.contains("colorimage"));
}

#[test]
fn a_shading_makes_postscript_draw_the_page_as_an_image_with_a_message() {
    let (ps, msgs) = run("eps", "shade.pdf", 0);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(msgs[0].contains("shade.pdf") && msgs[0].contains("image"));
    assert!(text(&ps).contains("colorimage"));
}

#[test]
fn pdf_picture_is_drawn_in_bitmaps() {
    let (png, msgs) = run("png", "rich.pdf", 0);
    assert!(msgs.is_empty(), "{msgs:?}");
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    // the light blue ground of the page fills the middle of the box
    let p = img.get_pixel(img.width() / 2, img.height() / 2).0;
    assert!(p[2] > 200 && p[0] < 250, "{p:?}");
    // and a flipped picture is drawn too
    let (png2, _) = run("png", "rich.pdf", 1);
    assert_ne!(png, png2);
}

#[test]
fn svg_still_links_the_file() {
    let (svg, _) = run("svg", "rich.pdf", 0);
    assert!(text(&svg).contains("<image xlink:href=\"file:rich.pdf\""));
}

#[test]
fn eps_in_postscript_is_copied_as_before() {
    let (ps, msgs) = run("eps", "rich.eps", 0);
    assert!(msgs.is_empty(), "{msgs:?}");
    assert!(text(&ps).contains("%%BeginDocument: rich.eps"));
}

#[test]
fn eps_in_pdf_and_bitmaps_goes_through_ghostscript_when_installed() {
    let (pdf, msgs) = run("pdf", "rich.eps", 0);
    if fig2dev::util::gs::find().is_none() {
        assert!(!text(&pdf).contains("/Subtype /Form"));
        return;
    }
    assert!(msgs.is_empty(), "{msgs:?}");
    assert!(text(&pdf).contains("/Subtype /Form"));
    let (png, _) = run("png", "rich.eps", 0);
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    let p = img.get_pixel(img.width() / 2, img.height() / 2).0;
    assert!(p[2] > 200 && p[0] < 250, "{p:?}");
}

#[test]
fn a_page_that_is_not_a_pdf_leaves_the_picture_out() {
    let dir = std::env::temp_dir().join(format!("pdfpic-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bad.pdf"), b"%PDF-1.4\nthis is not a PDF\n").unwrap();
    let mut ctx = Ctx {
        lang: "pdf".into(),
        dir: Some(dir.clone()),
        ..Ctx::default()
    };
    ctx.font_size *= ctx.fontmag;
    let mut dev = drivers::driver("pdf").unwrap();
    let objects = readfp_fig(&mut ctx, fig("bad.pdf", 0).as_bytes())
        .map_err(|_| "read")
        .unwrap();
    let out = convert(&mut ctx, dev.as_mut(), &objects).unwrap().output;
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.starts_with(b"%PDF-"));
    assert!(!text(&out).contains("/Subtype /Form"));
}

/// Hostile files: the PDF crates must give `None` or a picture, never
/// bring the program down.
#[test]
fn mutated_pdfs_never_panic() {
    let original = std::fs::read(fixtures().join("rich.pdf")).unwrap();
    let mut seed = 0x9e3779b97f4a7c15u64;
    let mut next = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for _ in 0..400 {
        let mut f = original.clone();
        for _ in 0..1 + next() % 6 {
            let i = next() % f.len();
            match next() % 3 {
                0 => f[i] = next() as u8,
                1 => f.truncate(i.max(8)),
                _ => {
                    f.remove(i);
                }
            }
        }
        if let Some(p) = PdfPicture::open(f) {
            let _ = p.to_eps();
            let _ = p.render(20, 20);
            let _ = p.xobject(10);
        }
    }
    // sizes that make no sense
    let p = PdfPicture::open(original).unwrap();
    assert!(p.render(0, 5).is_none());
    assert!(p.render(1_000_000, 1_000_000).is_none());
}

/// A JPEG picture is carried as it is in PDF output (DCTDecode), where it
/// was decoded pixels before.
#[test]
fn jpeg_in_pdf_keeps_its_data() {
    let dir = std::env::temp_dir().join(format!("pdfpic-jpg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let px: Vec<u8> = (0..16 * 16 * 3).map(|i| (i * 7 % 256) as u8).collect();
    let mut jpg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 80)
        .encode(&px, 16, 16, image::ExtendedColorType::Rgb8)
        .unwrap();
    std::fs::write(dir.join("p.jpg"), &jpg).unwrap();
    let mut ctx = Ctx {
        lang: "pdf".into(),
        dir: Some(dir.clone()),
        ..Ctx::default()
    };
    ctx.font_size *= ctx.fontmag;
    let mut dev = drivers::driver("pdf").unwrap();
    let objects = readfp_fig(&mut ctx, fig("p.jpg", 0).as_bytes())
        .map_err(|_| "read")
        .unwrap();
    let out = convert(&mut ctx, dev.as_mut(), &objects).unwrap().output;
    std::fs::remove_dir_all(&dir).ok();
    assert!(text(&out).contains("/DCTDecode"));
    assert!(
        out.windows(jpg.len()).any(|w| w == jpg.as_slice()),
        "the file's bytes are in the PDF"
    );
}
