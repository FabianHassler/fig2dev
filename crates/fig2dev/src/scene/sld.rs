//! fig2dev's `-L sld` (AutoCAD slide): the pixmap through `ppmtoacad`.
//!
//! fig2dev pipes ghostscript's `ppmraw` output into `ppmtoacad -quiet`
//! (genbitmaps.c). Here the rasteriser's pixels are encoded as netpbm's
//! `ppmtoacad` does without `-dxb`, `-poly`, `-background`, `-aspect` or
//! `-8` (fig2dev passes none): the slide header, each colour mapped to the
//! nearest of AutoCAD's 256 colours, black (colour 0) left to the screen
//! background, and one vector per horizontal run of equal colour.
//
// Ported from fig2dev dev/genbitmaps.c and netpbm converter/ppm/ppmtoacad.c
// (John Walker, Autodesk; permission to use, copy, modify and distribute
// without conditions or restrictions; see NOTICE; xfig licence, see
// LICENSE-xfig).

use std::collections::HashMap;

/// netpbm's `MAXHIST`: more colours than this and the colour depth is
/// halved until the image fits.
const MAXHIST: usize = 32767;

/// netpbm's `autocad.h`: AutoCAD's 256 standard colours as RGB.
const ACAD_COLORS: [[u8; 3]; 256] = [
    [0, 0, 0],
    [255, 0, 0],
    [255, 255, 0],
    [0, 255, 0],
    [0, 255, 255],
    [0, 0, 255],
    [255, 0, 255],
    [255, 255, 255],
    [255, 255, 255],
    [255, 255, 255],
    [255, 0, 0],
    [255, 127, 127],
    [165, 0, 0],
    [165, 82, 82],
    [127, 0, 0],
    [127, 63, 63],
    [76, 0, 0],
    [76, 38, 38],
    [38, 0, 0],
    [38, 19, 19],
    [255, 63, 0],
    [255, 159, 127],
    [165, 41, 0],
    [165, 103, 82],
    [127, 31, 0],
    [127, 79, 63],
    [76, 19, 0],
    [76, 47, 38],
    [38, 9, 0],
    [38, 23, 19],
    [255, 127, 0],
    [255, 191, 127],
    [165, 82, 0],
    [165, 124, 82],
    [127, 63, 0],
    [127, 95, 63],
    [76, 38, 0],
    [76, 57, 38],
    [38, 19, 0],
    [38, 28, 19],
    [255, 191, 0],
    [255, 223, 127],
    [165, 124, 0],
    [165, 145, 82],
    [127, 95, 0],
    [127, 111, 63],
    [76, 57, 0],
    [76, 66, 38],
    [38, 28, 0],
    [38, 33, 19],
    [255, 255, 0],
    [255, 255, 127],
    [165, 165, 0],
    [165, 165, 82],
    [127, 127, 0],
    [127, 127, 63],
    [76, 76, 0],
    [76, 76, 38],
    [38, 38, 0],
    [38, 38, 19],
    [191, 255, 0],
    [223, 255, 127],
    [124, 165, 0],
    [145, 165, 82],
    [95, 127, 0],
    [111, 127, 63],
    [57, 76, 0],
    [66, 76, 38],
    [28, 38, 0],
    [33, 38, 19],
    [127, 255, 0],
    [191, 255, 127],
    [82, 165, 0],
    [124, 165, 82],
    [63, 127, 0],
    [95, 127, 63],
    [38, 76, 0],
    [57, 76, 38],
    [19, 38, 0],
    [28, 38, 19],
    [63, 255, 0],
    [159, 255, 127],
    [41, 165, 0],
    [103, 165, 82],
    [31, 127, 0],
    [79, 127, 63],
    [19, 76, 0],
    [47, 76, 38],
    [9, 38, 0],
    [23, 38, 19],
    [0, 255, 0],
    [127, 255, 127],
    [0, 165, 0],
    [82, 165, 82],
    [0, 127, 0],
    [63, 127, 63],
    [0, 76, 0],
    [38, 76, 38],
    [0, 38, 0],
    [19, 38, 19],
    [0, 255, 63],
    [127, 255, 159],
    [0, 165, 41],
    [82, 165, 103],
    [0, 127, 31],
    [63, 127, 79],
    [0, 76, 19],
    [38, 76, 47],
    [0, 38, 9],
    [19, 38, 23],
    [0, 255, 127],
    [127, 255, 191],
    [0, 165, 82],
    [82, 165, 124],
    [0, 127, 63],
    [63, 127, 95],
    [0, 76, 38],
    [38, 76, 57],
    [0, 38, 19],
    [19, 38, 28],
    [0, 255, 191],
    [127, 255, 223],
    [0, 165, 124],
    [82, 165, 145],
    [0, 127, 95],
    [63, 127, 111],
    [0, 76, 57],
    [38, 76, 66],
    [0, 38, 28],
    [19, 38, 33],
    [0, 255, 255],
    [127, 255, 255],
    [0, 165, 165],
    [82, 165, 165],
    [0, 127, 127],
    [63, 127, 127],
    [0, 76, 76],
    [38, 76, 76],
    [0, 38, 38],
    [19, 38, 38],
    [0, 191, 255],
    [127, 223, 255],
    [0, 124, 165],
    [82, 145, 165],
    [0, 95, 127],
    [63, 111, 127],
    [0, 57, 76],
    [38, 66, 76],
    [0, 28, 38],
    [19, 33, 38],
    [0, 127, 255],
    [127, 191, 255],
    [0, 82, 165],
    [82, 124, 165],
    [0, 63, 127],
    [63, 95, 127],
    [0, 38, 76],
    [38, 57, 76],
    [0, 19, 38],
    [19, 28, 38],
    [0, 63, 255],
    [127, 159, 255],
    [0, 41, 165],
    [82, 103, 165],
    [0, 31, 127],
    [63, 79, 127],
    [0, 19, 76],
    [38, 47, 76],
    [0, 9, 38],
    [19, 23, 38],
    [0, 0, 255],
    [127, 127, 255],
    [0, 0, 165],
    [82, 82, 165],
    [0, 0, 127],
    [63, 63, 127],
    [0, 0, 76],
    [38, 38, 76],
    [0, 0, 38],
    [19, 19, 38],
    [63, 0, 255],
    [159, 127, 255],
    [41, 0, 165],
    [103, 82, 165],
    [31, 0, 127],
    [79, 63, 127],
    [19, 0, 76],
    [47, 38, 76],
    [9, 0, 38],
    [23, 19, 38],
    [127, 0, 255],
    [191, 127, 255],
    [82, 0, 165],
    [124, 82, 165],
    [63, 0, 127],
    [95, 63, 127],
    [38, 0, 76],
    [57, 38, 76],
    [19, 0, 38],
    [28, 19, 38],
    [191, 0, 255],
    [223, 127, 255],
    [124, 0, 165],
    [145, 82, 165],
    [95, 0, 127],
    [111, 63, 127],
    [57, 0, 76],
    [66, 38, 76],
    [28, 0, 38],
    [33, 19, 38],
    [255, 0, 255],
    [255, 127, 255],
    [165, 0, 165],
    [165, 82, 165],
    [127, 0, 127],
    [127, 63, 127],
    [76, 0, 76],
    [76, 38, 76],
    [38, 0, 38],
    [38, 19, 38],
    [255, 0, 191],
    [255, 127, 223],
    [165, 0, 124],
    [165, 82, 145],
    [127, 0, 95],
    [127, 63, 111],
    [76, 0, 57],
    [76, 38, 66],
    [38, 0, 28],
    [38, 19, 33],
    [255, 0, 127],
    [255, 127, 191],
    [165, 0, 82],
    [165, 82, 124],
    [127, 0, 63],
    [127, 63, 95],
    [76, 0, 38],
    [76, 38, 57],
    [38, 0, 19],
    [38, 19, 28],
    [255, 0, 63],
    [255, 127, 159],
    [165, 0, 41],
    [165, 82, 103],
    [127, 0, 31],
    [127, 63, 79],
    [76, 0, 19],
    [76, 38, 47],
    [38, 0, 9],
    [38, 19, 23],
    [84, 84, 84],
    [118, 118, 118],
    [152, 152, 152],
    [186, 186, 186],
    [220, 220, 220],
    [255, 255, 255],
];

/// The slide `ppmtoacad -quiet` writes for an image of `w` x `h` pixels
/// (`rgb`: three bytes per pixel, rows from the top).
pub fn sld(rgb: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || rgb.len() < w.saturating_mul(h).saturating_mul(3) {
        return Err("image would be empty".into());
    }
    let mut px: Vec<[u8; 3]> = rgb.as_chunks::<3>().0.iter().take(w * h).copied().collect();

    // ppm_computecolorhist fails above MAXHIST colours; ppmtoacad then
    // halves maxval (PPM_DEPTH) and tries again.
    let mut maxval: u32 = 255;
    loop {
        let mut seen: HashMap<[u8; 3], ()> = HashMap::new();
        for p in &px {
            seen.insert(*p, ());
            if seen.len() > MAXHIST {
                break;
            }
        }
        if seen.len() <= MAXHIST || maxval <= 1 {
            break;
        }
        let new = maxval / 2;
        for p in &mut px {
            for c in p.iter_mut() {
                *c = ((u32::from(*c) * new + maxval / 2) / maxval) as u8;
            }
        }
        maxval = new;
    }

    // Each distinct colour to its nearest AutoCAD colour (first wins ties).
    let mut cache: HashMap<[u8; 3], u8> = HashMap::new();
    let mut mapped = Vec::with_capacity(px.len());
    for p in &px {
        let c = *cache.entry(*p).or_insert_with(|| {
            let full = p.map(|v| {
                if maxval == 255 {
                    i64::from(v)
                } else {
                    i64::from(v) * 255 / i64::from(maxval)
                }
            });
            let mut best = 0usize;
            let mut dist = 3 * 256 * 256;
            for (j, a) in ACAD_COLORS.iter().enumerate() {
                let d: i64 = (0..3).map(|k| (full[k] - i64::from(a[k])).pow(2)).sum();
                if d < dist {
                    dist = d;
                    best = j;
                }
            }
            best as u8
        });
        mapped.push(c);
    }

    let (xsize, ysize) = (w - 1, h - 1);
    // 17 bytes: `fwrite` takes the NUL too.
    let mut out = b"AutoCAD Slide\r\n\x1a\0".to_vec();
    out.push(86); // number format indicator
    out.push(2); // file level number
    out.extend_from_slice(&(xsize as u16).to_le_bytes());
    out.extend_from_slice(&(ysize as u16).to_le_bytes());
    // `(long)((double)xsize / ysize * aspect * 1E7)`: infinity saturates,
    // NaN gives 0 (arm64), and the low 32 bits are written.
    let ratio = (xsize as f64 / ysize as f64) * 1.0 * 1e7;
    out.extend_from_slice(&(ratio as i64 as u32).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // polygon fill type
    out.extend_from_slice(&0x1234u16.to_le_bytes()); // byte order indicator

    let mut curcol: i32 = -1;
    for (y, row) in mapped.chunks(w).enumerate() {
        let mut x = 0;
        while x < w {
            let color = row[x];
            let start = x;
            while x < w && row[x] == color {
                x += 1;
            }
            if color == 0 {
                continue;
            }
            if curcol != i32::from(color) {
                out.extend_from_slice(&(0xFF00u16 | u16::from(color)).to_le_bytes());
                curcol = i32::from(color);
            }
            for v in [start, ysize - y, x - 1, ysize - y] {
                out.extend_from_slice(&(v as u16).to_le_bytes());
            }
        }
    }
    out.extend_from_slice(&0xFC00u16.to_le_bytes()); // end of file
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(b: &[u8]) -> Vec<u16> {
        b.chunks(2)
            .map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)]))
            .collect()
    }

    #[test]
    fn table_is_autocads() {
        assert_eq!(ACAD_COLORS[1], [255, 0, 0]);
        assert_eq!(ACAD_COLORS[7], [255, 255, 255]);
        assert_eq!(ACAD_COLORS[255], [255, 255, 255]);
        assert_eq!(ACAD_COLORS[250], [84, 84, 84]);
    }

    #[test]
    fn header_runs_and_end_marker() {
        // 3 x 2: red red black / blue blue blue.
        let rgb = [255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 255];
        let b = sld(&rgb, 3, 2).unwrap();
        assert_eq!(&b[..17], b"AutoCAD Slide\r\n\x1a\0");
        assert_eq!(&b[17..19], &[86, 2]);
        // xsize 2, ysize 1, aspect 2/1 * 1e7, polygon fill 2, byte order.
        assert_eq!(words(&b[19..21]), [2]);
        assert_eq!(words(&b[21..23]), [1]);
        assert_eq!(u32::from_le_bytes([b[23], b[24], b[25], b[26]]), 20_000_000);
        assert_eq!(words(&b[27..31]), [2, 0x1234]);
        // colour 1, vector (0,1)-(1,1); colour 5, vector (0,0)-(2,0); end.
        assert_eq!(words(&b[31..]), [0xFF01, 0, 1, 1, 1, 0xFF05, 0, 0, 2, 0, 0xFC00]);
    }

    #[test]
    fn black_image_is_only_the_header_and_marker() {
        let b = sld(&[0; 12], 2, 2).unwrap();
        assert_eq!(b.len(), 31 + 2);
        assert_eq!(words(&b[31..]), [0xFC00]);
    }

    #[test]
    fn nearest_colour_first_wins() {
        // Mid grey: 118 (index 251) is nearer than 84 or 152.
        let b = sld(&[120, 120, 120], 1, 1).unwrap();
        assert_eq!(words(&b[31..]), [0xFF00 | 251, 0, 0, 0, 0, 0xFC00]);
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        assert!(sld(&[], 0, 0).is_err());
        assert!(sld(&[1, 2, 3], 2, 2).is_err());
        // 1 x 1: 0/0 aspect is NaN, written as 0 as on arm64.
        let b = sld(&[255, 255, 255], 1, 1).unwrap();
        assert_eq!(&b[23..27], &[0, 0, 0, 0]);
        // Wide and one pixel high: infinity saturates.
        assert!(sld(&vec![255; 300], 100, 1).is_ok());
    }

    #[test]
    fn more_than_maxhist_colours_halve_the_depth() {
        let mut rgb = Vec::new();
        for i in 0..40000u32 {
            rgb.extend_from_slice(&[(i % 256) as u8, ((i / 256) % 256) as u8, (i / 65536 * 90) as u8 + 3]);
        }
        let b = sld(&rgb, 200, 200).unwrap();
        assert!(b.len() > 31);
    }

    /// The real `ppmtoacad -quiet` writes the same bytes (needs netpbm).
    #[test]
    #[ignore]
    fn same_as_ppmtoacad() {
        use std::io::Write;
        let mut seed = 12345u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        };
        // A patterned image, and noise with more than 32,767 colours.
        let mut smooth = Vec::new();
        for i in 0..64u32 * 48 {
            smooth.extend_from_slice(&[(i * 7 % 256) as u8, (i * 13 % 200) as u8, (i / 5 % 256) as u8]);
        }
        let noisy: Vec<u8> = (0..300 * 300 * 3).map(|_| noise()).collect();
        for (rgb, w, h) in [(smooth, 64u32, 48u32), (noisy, 300, 300)] {
            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
            ppm.extend_from_slice(&rgb);
            let Ok(mut child) = std::process::Command::new("ppmtoacad")
                .arg("-quiet")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
            else {
                return;
            };
            child.stdin.take().unwrap().write_all(&ppm).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.stdout == sld(&rgb, w, h).unwrap(), "{w}x{h}");
        }
    }
}
