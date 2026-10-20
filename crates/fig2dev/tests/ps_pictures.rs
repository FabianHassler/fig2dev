#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Pictures in the scene languages as genps and gensvg treat them: a file
//! that can't be read draws nothing in eps/ps/pdf/bitmaps and is a link in
//! svg (gensvg doesn't read it); the canvas's default keeps a placeholder;
//! an indexed PNG with one transparent colour stays opaque, as genps's
//! `indexed_image` writes no `/MaskColor` for it.

use fig2dev::scene::build::{build, TextOptions};
use fig2dev::scene::Prim;
use fig2dev::{convert, drivers, read::readfp_fig, Ctx};

fn fig(pic: &str, flipped: i32, pts: &str) -> String {
    format!(
        "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
         2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t{flipped} {pic}\n\t{pts}\n"
    )
}

const PTS: &str = "1200 1200 3600 1200 3600 3600 1200 3600 1200 1200";

fn run(lang: &str, src: &str) -> (String, Vec<String>) {
    let mut ctx = Ctx {
        lang: lang.into(),
        ..Ctx::default()
    };
    let mut dev = drivers::driver(lang).unwrap();
    ctx.font_size *= ctx.fontmag;
    let objects = readfp_fig(&mut ctx, src.as_bytes()).map_err(|_| "read").unwrap();
    let out = convert(&mut ctx, dev.as_mut(), &objects).unwrap().output;
    (String::from_utf8_lossy(&out).into_owned(), ctx.messages)
}

#[test]
fn a_missing_picture_draws_nothing_in_postscript() {
    let (eps, msgs) = run("eps", &fig("/nonexistent/x.png", 0, PTS));
    assert_eq!(msgs, ["No such picture file: /nonexistent/x.png"]);
    assert!(!eps.contains("stroke"), "no box, no cross, no frame");
    assert!(!eps.contains("0.502 0.502 0.502 setrgbcolor"));
}

#[test]
fn an_unknown_picture_format_draws_nothing_in_postscript() {
    let dir = std::env::temp_dir().join(format!("f2d-pic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let junk = dir.join("junk.bin");
    std::fs::write(&junk, b"junk").unwrap();
    let name = junk.to_str().unwrap();
    let (eps, msgs) = run("ps", &fig(name, 0, PTS));
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(msgs, [format!("{name}: Unknown image format")]);
    assert!(!eps.contains("stroke"));
}

#[test]
fn svg_links_the_file_without_reading_it() {
    let (svg, msgs) = run("svg", &fig("/nonexistent/x.png", 0, PTS));
    assert!(msgs.is_empty(), "{msgs:?}");
    assert!(svg.contains(
        "<!-- Image -->\n<image xlink:href=\"file:/nonexistent/x.png\" preserveAspectRatio=\"none\"\n\
         x=\"1200\" y=\"1200\" width=\"2400\" height=\"2400\"/>\n"
    ));
    assert!(!svg.contains("<path"), "no placeholder");
}

#[test]
fn svg_link_rotation_and_flip() {
    // gensvg: the corner order gives the rotation, a flipped picture
    // mirrors about the box's centre.
    let rev = "3600 3600 1200 3600 1200 1200 3600 1200 3600 3600";
    let (svg, _) = run("svg", &fig("a.png", 1, rev));
    assert!(
        svg.contains("transform=\"rotate(90 2400 2400) scale(-1,1) translate(-4800,0)\"\n"),
        "{svg}"
    );
    let (svg, _) = run("svg", &fig("a.png", 0, rev));
    assert!(svg.contains("transform=\"rotate(180 2400 2400)\"\n"), "{svg}");
}

#[test]
fn the_canvas_keeps_a_placeholder() {
    let mut ctx = Ctx::default();
    let objects = readfp_fig(&mut ctx, fig("/nonexistent/x.png", 0, PTS).as_bytes())
        .map_err(|_| "read")
        .unwrap();
    let (scene, _) = build(&mut ctx, &objects, TextOptions::default());
    assert!(scene.prims.iter().any(|p| matches!(p, Prim::PictureRef { .. })));
    assert!(
        scene.prims.iter().any(|p| matches!(p, Prim::Path { .. })),
        "crossed box"
    );
    let (scene, _) = build(
        &mut ctx,
        &objects,
        TextOptions {
            genps: true,
            ..TextOptions::default()
        },
    );
    assert!(scene.prims.is_empty(), "{:?}", scene.prims);
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// An 8-bit PNG of colour type `ct`, stored (uncompressed) deflate.
fn png(w: u32, h: u32, ct: u8, rows: &[u8], extra: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut raw = Vec::new();
    for row in rows.chunks(rows.len() / h as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    let mut z = vec![0x78, 0x01, 1];
    z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
    z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
    z.extend_from_slice(&raw);
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, ct, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    for (k, d) in extra {
        chunk(&mut out, k, d);
    }
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn eps_of_png(name: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("f2d-png-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.png");
    std::fs::write(&path, bytes).unwrap();
    let (eps, msgs) = run("eps", &fig(path.to_str().unwrap(), 0, PTS));
    std::fs::remove_dir_all(&dir).ok();
    assert!(msgs.is_empty(), "{msgs:?}");
    eps
}

#[test]
fn an_indexed_picture_with_one_transparent_colour_is_not_masked() {
    // grey + alpha and grey with a tRNS colour: genps writes a colour map
    // (`/ImageType 1`); the pixels are what libpng composited over white
    let ga = png(2, 2, 4, &[0, 255, 100, 0, 200, 255, 50, 128], &[]);
    let eps = eps_of_png("ga", &ga);
    assert!(eps.contains("colorimage") && !eps.contains("MaskColor"), "{eps}");
    let gt = png(2, 2, 0, &[0, 40, 80, 120], &[(b"tRNS", vec![0, 40])]);
    let eps = eps_of_png("gt", &gt);
    assert!(eps.contains("colorimage") && !eps.contains("MaskColor"), "{eps}");
    // RGB + alpha keeps the white colour key
    let rgba = png(2, 1, 6, &[10, 20, 30, 255, 40, 50, 60, 0], &[]);
    let eps = eps_of_png("rgba", &rgba);
    assert!(eps.contains("/MaskColor [255 255 255]"), "{eps}");
}
