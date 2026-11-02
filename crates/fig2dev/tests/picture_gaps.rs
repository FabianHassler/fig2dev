#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Picture-handling gaps found by black-box testing: `sc` in PostScript JPEG output, 2- and 4-bit
//! grey TIFFs, JPEGs over the pixel cap, CMYK JPEGs without an Adobe marker.

use fig2dev::{convert, drivers, read::readfp_fig, Ctx};
use std::path::PathBuf;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("picgaps-{name}"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A figure with one picture 1 inch wide at 1200,1200 (aspect 1:1).
fn fig(file: &str) -> String {
    format!(
        "#FIG 3.2\nLandscape\nCenter\nInches\nLetter\n100.00\nSingle\n-2\n1200 2\n\
2 5 0 1 0 -1 50 -1 -1 0.000 0 0 -1 0 0 5\n\t 0 {file}\n\t 1200 1200 2400 1200 2400 2400 1200 2400 1200 1200\n"
    )
}

/// (output bytes, messages)
fn run(lang: &str, dir: &std::path::Path, file: &str) -> (Vec<u8>, Vec<String>) {
    let mut ctx = Ctx {
        lang: lang.into(),
        dir: Some(dir.to_path_buf()),
        ..Ctx::default()
    };
    ctx.font_size *= ctx.fontmag;
    let mut dev = drivers::driver(lang).unwrap();
    let objects = readfp_fig(&mut ctx, fig(file).as_bytes()).map_err(|_| "read").unwrap();
    let out = convert(&mut ctx, dev.as_mut(), &objects).unwrap().output;
    (out, ctx.messages)
}

fn gray_jpeg(w: u32, h: u32) -> Vec<u8> {
    let px = vec![128u8; (w * h) as usize];
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
        .encode(&px, w, h, image::ExtendedColorType::L8)
        .unwrap();
    out
}

/// BUG 1: the "ps" and "eps" drivers write `W H sc` for a JPEG (genps's
/// shorthand) but their prologue never defines `sc`: ghostscript stops with
/// "/undefined in sc". Either define `/sc {scale} bind def` or write `scale`.
#[test]
fn jpeg_in_ps_and_eps_only_uses_defined_operators() {
    let d = dir("sc");
    std::fs::write(d.join("a.jpg"), gray_jpeg(16, 16)).unwrap();
    for lang in ["ps", "eps"] {
        let (out, _) = run(lang, &d, "a.jpg");
        let ps = String::from_utf8_lossy(&out);
        let uses_sc = ps.lines().any(|l| l.ends_with(" sc") || l == "sc");
        let defines_sc = ps.contains("/sc {scale}") || ps.contains("/sc{scale}");
        assert!(!uses_sc || defines_sc, "{lang}: `sc` used but never defined");
    }
}

/// Minimal little-endian uncompressed grey TIFF, `bits` per sample
/// (1, 2, 4 or 8), `w` x `h`, every row `row` (already packed), min-is-black.
fn grey_tiff(w: u32, h: u32, bits: u16, photometric: u16, row: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    for _ in 0..h {
        data.extend_from_slice(row);
    }
    let entries: [(u16, u16, u32, u32); 9] = [
        (256, 4, 1, w),                      // ImageWidth
        (257, 4, 1, h),                      // ImageLength
        (258, 3, 1, u32::from(bits)),        // BitsPerSample
        (259, 3, 1, 1),                      // Compression: none
        (262, 3, 1, u32::from(photometric)), // Photometric: 0 min-is-white, 1 min-is-black
        (273, 4, 1, 8),                      // StripOffsets
        (277, 3, 1, 1),                      // SamplesPerPixel
        (278, 4, 1, h),                      // RowsPerStrip
        (279, 4, 1, data.len() as u32),      // StripByteCounts
    ];
    let mut t = b"II*\0".to_vec();
    t.extend_from_slice(&((8 + data.len()) as u32).to_le_bytes()); // IFD after the data
    t.extend_from_slice(&data);
    t.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, typ, count, val) in entries {
        t.extend_from_slice(&tag.to_le_bytes());
        t.extend_from_slice(&typ.to_le_bytes());
        t.extend_from_slice(&count.to_le_bytes());
        if typ == 3 {
            t.extend_from_slice(&(val as u16).to_le_bytes());
            t.extend_from_slice(&[0, 0]);
        } else {
            t.extend_from_slice(&val.to_le_bytes());
        }
    }
    t.extend_from_slice(&0u32.to_le_bytes());
    t
}

/// BUG 2: 2- and 4-bit grey TIFFs are rejected ("Bad TIFF format"); C fig2dev
/// reads them through tifftopnm, which expands any bits per sample. 1 and 8 work.
#[test]
fn two_and_four_bit_grey_tiffs_are_read() {
    let d = dir("tiff");
    // 4 pixels wide, 2 bits: values 0,1,2,3 -> 0b00_01_10_11
    std::fs::write(d.join("g2.tif"), grey_tiff(4, 4, 2, 1, &[0b0001_1011])).unwrap();
    // 4 pixels wide, 4 bits: values 0,5,10,15
    std::fs::write(d.join("g4.tif"), grey_tiff(4, 4, 4, 1, &[0x05, 0xaf])).unwrap();
    for f in ["g2.tif", "g4.tif"] {
        let (_, msgs) = run("eps", &d, f);
        assert!(msgs.is_empty(), "{f}: {msgs:?}");
    }
}

/// BUG 3: a JPEG over MAX_PIXELS (here 5793 x 5793 > 2^25) is replaced by a
/// grey placeholder in PNG and PDF output *without any message* (PNG files
/// over the cap do say "Bad PNG format"; EPS embeds the JPEG regardless).
/// Expect a message naming the file.
#[test]
fn over_cap_jpeg_reports_a_message_in_png_and_pdf() {
    let d = dir("bigjpg");
    std::fs::write(d.join("big.jpg"), gray_jpeg(5793, 5793)).unwrap();
    for lang in ["png", "pdf"] {
        let (_, msgs) = run(lang, &d, "big.jpg");
        assert!(
            msgs.iter().any(|m| m.contains("big.jpg")),
            "{lang}: silent failure, messages: {msgs:?}"
        );
    }
}

/// An 8 x 8 CMYK JPEG, JFIF first, no Adobe marker, samples stored as
/// C=0 M=127 Y=190 K=55 (plain, not inverted).
const CMYK_NO_ADOBE: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
    0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0xff, 0xc0, 0x00, 0x14, 0x08, 0x00,
    0x08, 0x00, 0x08, 0x04, 0x43, 0x11, 0x00, 0x4d, 0x11, 0x00, 0x59, 0x11, 0x00, 0x4b, 0x11, 0x00, 0xff, 0xc4, 0x00,
    0x1f, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0xff, 0xc4, 0x00, 0xb5, 0x10, 0x00, 0x02, 0x01,
    0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7d, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11,
    0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23,
    0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a,
    0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73,
    0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95,
    0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6,
    0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7,
    0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6,
    0xf7, 0xf8, 0xf9, 0xfa, 0xff, 0xda, 0x00, 0x0e, 0x04, 0x43, 0x00, 0x4d, 0x00, 0x59, 0x00, 0x4b, 0x00, 0x00, 0x3f,
    0x00, 0xff, 0x00, 0x3f, 0xfa, 0xaf, 0x5f, 0xbe, 0x15, 0xfc, 0xdb, 0xd7, 0xff, 0xd9,
];

/// BUG 4 (inconsistency; arguably ambiguous): for a CMYK JPEG without an
/// Adobe marker the PS/PDF path follows readjpg.c (values are plain CMYK, so
/// the picture is (200,100,50)-ish) but the PNG path treats them as inverted
/// and draws near black. Both outputs should agree.
#[test]
fn cmyk_jpeg_without_adobe_marker_same_in_png_and_ps() {
    let d = dir("cmyk");
    std::fs::write(d.join("c.jpg"), CMYK_NO_ADOBE).unwrap();
    let (ps, _) = run("eps", &d, "c.jpg");
    let ps = String::from_utf8_lossy(&ps);
    // readjpg.c: no `/Decode [1 0 ...]` without the Adobe marker
    assert!(!ps.contains("/Decode [1 0 1 0 1 0 1 0]"));
    let (png, _) = run("png", &d, "c.jpg");
    let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let f = r.next_frame(&mut buf).unwrap();
    let (cx, cy) = (f.width as usize / 2, f.height as usize / 2);
    let n = f.buffer_size() / (f.width as usize * f.height as usize);
    let p = &buf[(cy * f.width as usize + cx) * n..][..3];
    // plain CMYK (0,127,190,55) is (200,100,50) as the PS path shows it
    assert!(p[0] > 150 && p[2] < 100, "PNG draws {p:?}, expected about (200,100,50)");
}

/// The grey of each of four pixel columns, at mid height, of a PNG output
/// of the 4-pixel-wide picture `file`.
fn png_row(d: &std::path::Path, file: &str) -> Vec<u8> {
    let (png, msgs) = run("png", d, file);
    assert!(msgs.is_empty(), "{file}: {msgs:?}");
    let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let f = r.next_frame(&mut buf).unwrap();
    let n = f.buffer_size() / (f.width as usize * f.height as usize);
    let y = f.height as usize / 2;
    // the picture fills the figure
    (0..4)
        .map(|i| buf[(y * f.width as usize + f.width as usize * (2 * i + 1) / 8) * n])
        .collect()
}

/// The 2- and 4-bit grey TIFFs also come out with the right grey levels
/// (samples stretched to 0..255), and min-is-white is inverted.
#[test]
fn low_bit_depth_grey_tiffs_have_the_right_levels() {
    let d = dir("tiff-levels");
    std::fs::write(d.join("g2.tif"), grey_tiff(4, 4, 2, 1, &[0b0001_1011])).unwrap();
    std::fs::write(d.join("w2.tif"), grey_tiff(4, 4, 2, 0, &[0b0001_1011])).unwrap();
    std::fs::write(d.join("g4.tif"), grey_tiff(4, 4, 4, 1, &[0x05, 0xaf])).unwrap();
    // (the canvas resamples a little)
    let near = |got: Vec<u8>, want: [u8; 4]| {
        assert!(
            got.iter().zip(want).all(|(&g, w)| g.abs_diff(w) <= 4),
            "{got:?} != {want:?}"
        );
    };
    near(png_row(&d, "g2.tif"), [0, 85, 170, 255]);
    near(png_row(&d, "w2.tif"), [255, 170, 85, 0]);
    near(png_row(&d, "g4.tif"), [0, 85, 170, 255]);
}
