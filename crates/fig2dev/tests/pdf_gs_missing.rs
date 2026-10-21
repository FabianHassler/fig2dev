#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Without ghostscript (nothing on `PATH`) an EPS picture in PDF output is
//! left out with fig2dev's message about ghostscript, and the figure is
//! still written. One test in its own process: it changes `PATH`.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};

#[test]
fn eps_in_pdf_without_ghostscript() {
    std::env::set_var("PATH", "");
    assert!(fig2dev::util::gs::find().is_none());
    let dir = std::env::temp_dir().join(format!("pdfpic-nogs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.eps"),
        b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 50 40\n0 0 50 40 rectfill\n",
    )
    .unwrap();
    let src = "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t 0 a.eps\n\t 1200 1200 3600 1200 3600 3600 1200 3600 1200 1200\n";
    let mut ctx = Ctx {
        lang: "pdf".into(),
        dir: Some(dir.clone()),
        ..Ctx::default()
    };
    ctx.font_size *= ctx.fontmag;
    let mut dev = drivers::driver("pdf").unwrap();
    let objects = readfp_fig(&mut ctx, src.as_bytes()).map_err(|_| "read").unwrap();
    let out = convert(&mut ctx, dev.as_mut(), &objects).unwrap().output;
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.starts_with(b"%PDF-"));
    assert_eq!(ctx.messages, [fig2dev::util::gs::MISSING_MESSAGE]);
}
