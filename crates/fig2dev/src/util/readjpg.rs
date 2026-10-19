//! JPEG pictures as genps embeds them: fig2dev does not decode a JPEG, it
//! reads the header ([`analyze_jpeg`]) and passes the compressed (DCT) data
//! on to the PostScript interpreter, ASCII85-encoded ([`ascii85_encode`],
//! [`jpeg_to_ps`]).
//!
//! Differences from C: a file without a further marker after the last one
//! it read (C loops forever on the end of file) and a marker whose length
//! is below 2 (C counts down from a huge unsigned number) end in "unexpected
//! end of JPEG file!". The JFIF resolution is ignored, as in C (the flag
//! that asks for it is never set). C keeps the size of the previous picture
//! in a static when a file has no SOF marker; here it stays 0.
//
// Ported from fig2dev dev/readjpg.c and encode.c (xfig licence, see
// LICENSE-xfig). jpeg2ps is (C) Thomas Merz 1994-2002, used with permission.

use crate::gendev::PACKAGE_VERSION;
use std::fmt::Write as _;

/// What `AnalyzeJPEG` finds out (C's `struct imagedata`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegInfo {
    /// Pixels per line.
    pub width: i32,
    /// Rows.
    pub height: i32,
    /// 1 (grey), 3 (RGB) or 4 (CMYK).
    pub components: i32,
    /// Bits per colour component, as the SOF marker says.
    pub bits_per_component: i32,
    /// An Adobe APP14 marker is present.
    pub adobe: bool,
}

// Markers (the IJG enum).
const M_SOF0: i32 = 0xc0;
const M_SOF1: i32 = 0xc1;
const M_RST0: i32 = 0xd0;
const M_RST7: i32 = 0xd7;
const M_SOI: i32 = 0xd8;
const M_EOI: i32 = 0xd9;
const M_APP0: i32 = 0xe0;
const M_APP14: i32 = 0xee;
const M_TEM: i32 = 0x01;
const M_ERROR: i32 = 0x100;
/// C keeps at most this many bytes of an APPn marker.
const APP_MAX: usize = 255;

/// A `FILE *` read byte by byte: `getc` gives -1 at the end and then `feof`
/// is true.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    eof: bool,
}

impl<'a> Reader<'a> {
    fn getc(&mut self) -> i32 {
        match self.data.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                i32::from(b)
            }
            None => {
                self.eof = true;
                -1
            }
        }
    }

    /// `get_2bytes`: two bytes, MSB first, with C's unsigned arithmetic
    /// (a missing byte reads as -1).
    fn get_2bytes(&mut self) -> u32 {
        let hi = (self.getc() as u32) << 8;
        hi.wrapping_add(self.getc() as u32)
    }

    /// `n` times `getc`; the bytes read, at most `max` of them.
    fn read(&mut self, n: u64, max: usize) -> &'a [u8] {
        let start = self.pos;
        let avail = (self.data.len() - start) as u64;
        if n > avail {
            self.eof = true;
        }
        let take = n.min(avail) as usize;
        self.pos = start + take;
        &self.data[start..start + take.min(max)]
    }

    /// `next_marker`: the next marker's code, `M_ERROR` at the end.
    fn next_marker(&mut self) -> i32 {
        if self.eof {
            return M_ERROR;
        }
        loop {
            // skip to FF
            loop {
                match self.getc() {
                    0xff => break,
                    -1 => return M_ERROR,
                    _ => {}
                }
            }
            // skip repeated FFs
            let mut c;
            loop {
                c = self.getc();
                if c != 0xff {
                    break;
                }
            }
            // repeat after FF/00
            if c != 0 {
                return c;
            }
        }
    }
}

/// `AnalyzeJPEG`: the size and colour space of JPEG `data`. The `Err` is
/// the message C prints (`put_msg`).
pub fn analyze_jpeg(data: &[u8]) -> Result<JpegInfo, String> {
    let mut r = Reader {
        data,
        pos: 0,
        eof: false,
    };
    let mut info = JpegInfo {
        width: 0,
        height: 0,
        components: 0,
        bits_per_component: 0,
        adobe: false,
    };
    loop {
        let c = r.next_marker();
        if c == M_EOI {
            break;
        }
        match c {
            M_ERROR => return Err("Error: unexpected end of JPEG file!".into()),
            // the SOF markers; PostScript level 2 supports only SOF0 and SOF1
            0xc2..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf | M_SOF0 | M_SOF1 => {
                let _length = r.get_2bytes();
                info.bits_per_component = r.getc();
                info.height = r.get_2bytes() as i32;
                info.width = r.get_2bytes() as i32;
                info.components = r.getc();
                break;
            }
            // JFIF: C would read the resolution, but never asks for it
            M_APP0 => {
                let length = u64::from(r.get_2bytes());
                r.read(length.saturating_sub(2), APP_MAX);
                if length < 2 {
                    r.read(u64::MAX, 0);
                }
            }
            // Adobe: "Adobe" at the start of the marker
            M_APP14 => {
                let length = u64::from(r.get_2bytes());
                let app = r.read(length.saturating_sub(2), APP_MAX);
                if length < 2 {
                    r.read(u64::MAX, 0);
                }
                if length >= 12 && app.starts_with(b"Adobe") {
                    info.adobe = true;
                }
            }
            // markers without parameters
            M_SOI | M_TEM | M_RST0..=M_RST7 => {}
            // skip the variable length markers
            _ => {
                let length = u64::from(r.get_2bytes());
                r.read(if length < 2 { u64::MAX } else { length - 2 }, 0);
            }
        }
    }
    if info.height <= 0 || info.width <= 0 || info.components <= 0 {
        return Err("Error: DNL marker not supported in PostScript Level 2!".into());
    }
    if !matches!(info.components, 1 | 3 | 4) {
        return Err(format!(
            "Error: unknown color space ({} components)!\n",
            info.components
        ));
    }
    Ok(info)
}

/// `convertfourbytes`: four bytes as five characters (`z` for zero).
fn four_bytes(out: &mut String, b: &[u8; 4]) {
    const POWER85: [u32; 4] = [85 * 85 * 85 * 85, 85 * 85 * 85, 85 * 85, 85];
    let mut word = u32::from_be_bytes(*b);
    if word == 0 {
        out.push('z');
        return;
    }
    for p in POWER85 {
        let o = word / p;
        word -= o * p;
        out.push(char::from(b'!' + o as u8));
    }
    out.push(char::from(b'!' + word as u8));
}

/// `ascii85encode` of all of `data`, as one call (C feeds it blocks of 64
/// KiB, a multiple of the 64-byte lines): lines of 16 groups, the last
/// group padded with zeros and cut to `len + 1` characters. No `~>`.
pub fn ascii85_encode(out: &mut String, data: &[u8]) {
    const POWER85: [u32; 4] = [85 * 85 * 85 * 85, 85 * 85 * 85, 85 * 85, 85];
    let (lines, rest) = data.as_chunks::<64>();
    for line in lines {
        for g in line.as_chunks::<4>().0 {
            four_bytes(out, g);
        }
        out.push('\n');
    }
    let (groups, tail) = rest.as_chunks::<4>();
    for g in groups {
        four_bytes(out, g);
    }
    // `convertremainder`
    if !tail.is_empty() {
        let mut word = 0u32;
        for (i, &b) in tail.iter().enumerate() {
            word += u32::from(b) << (8 * (3 - i));
        }
        for p in POWER85.iter().take(tail.len() + 1) {
            let o = word / p;
            word -= o * p;
            out.push(char::from(b'!' + o as u8));
        }
    }
}

/// `JPEGtoPS` in its ASCII85 mode: the EPS wrapper with the compressed
/// data. `filename` is the picture's name in the figure, `date` the
/// creation date (`None` leaves the line out), `version` the version
/// fig2dev reports.
pub fn jpeg_to_ps(info: &JpegInfo, filename: &str, date: Option<&str>, data: &[u8]) -> String {
    let mut o = String::with_capacity(data.len() / 4 * 5 + 1500);
    let (w, h) = (info.width, info.height);
    let space = match info.components {
        1 => "Gray",
        3 => "RGB",
        _ => "CMYK",
    };
    o.push_str("%!PS-Adobe-3.0 EPSF-3.0\n");
    let _ = writeln!(o, "%%Creator: jpeg2ps by Thomas Merz in fig2dev {}", PACKAGE_VERSION);
    let _ = writeln!(o, "%%Title: {filename}");
    if let Some(d) = date {
        let _ = writeln!(o, "%%CreationDate: {d}");
    }
    let _ = writeln!(o, "%%BoundingBox: 0 0 {w} {h}");
    o.push_str("%%DocumentData: Clean7Bit\n%%LanguageLevel: 2\n%%EndComments\n%%BeginProlog\n%%EndProlog\n");
    o.push_str("/languagelevel where {pop languagelevel 2 lt}{true} ifelse {\n");
    let _ = writeln!(
        o,
        "  (JPEG file '{filename}' needs PostScript Level 2!\\n) dup print flush"
    );
    o.push_str("  /Helvetica findfont 20 scalefont setfont 100 100 moveto show showpage stop\n} if\n");
    o.push_str("save\n/RawData currentfile /ASCII85Decode filter def\n");
    o.push_str("/Data RawData << >> /DCTDecode filter def\n");
    let _ = writeln!(o, "/Device{space} setcolorspace");
    o.push_str("{ << /ImageType 1\n");
    let _ = writeln!(o, "\t/Width {w}\n\t/Height {h}");
    let _ = writeln!(o, "\t/ImageMatrix [ {w} 0 0 {} 0 {h} ]", h.wrapping_neg());
    o.push_str("\t/DataSource Data\n");
    let _ = writeln!(o, "\t/BitsPerComponent {}", info.bits_per_component);
    // Adobe's CMYK files are inverted
    if info.adobe && info.components == 4 {
        o.push_str("\t  /Decode [1 0 1 0 1 0 1 0]\n");
    } else {
        o.push_str("\t  /Decode [0 1");
        for _ in 1..info.components {
            o.push_str(" 0 1");
        }
        o.push_str("]\n");
    }
    o.push_str("  >> image\n  Data closefile\n  RawData flushfile\n  restore\n} exec\n");
    ascii85_encode(&mut o, data);
    o.push_str("~>\n");
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a85(data: &[u8]) -> String {
        let mut s = String::new();
        ascii85_encode(&mut s, data);
        s
    }

    #[test]
    fn ascii85_groups_and_remainders() {
        assert_eq!(a85(b""), "");
        assert_eq!(a85(&[0, 0, 0, 0]), "z");
        assert_eq!(a85(b"Man "), "9jqo^");
        assert_eq!(a85(b"M"), "9`");
        assert_eq!(a85(b"Ma"), "9jn");
        assert_eq!(a85(b"Man"), "9jqo");
        assert_eq!(a85(&[255; 4]), "s8W-!");
        // zero groups in a remainder are not abbreviated
        assert_eq!(a85(&[0]), "!!");
    }

    #[test]
    fn ascii85_lines_hold_sixteen_groups() {
        let s = a85(&[0x41; 130]);
        let lines: Vec<&str> = s.split('\n').collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].len(), 80);
        assert_eq!(lines[1].len(), 80);
        // 2 bytes left: 32 groups of 4 bytes consumed in two lines = 128
        assert_eq!(lines[2].len(), 3);
        // a multiple of 64 ends with a newline
        assert!(a85(&[1; 64]).ends_with('\n'));
    }

    /// SOI, an APP0 marker, SOF0.
    fn jpeg(sof: u8, comps: u8, w: u16, h: u16, extra: &[u8]) -> Vec<u8> {
        let mut d = vec![0xff, 0xd8, 0xff, 0xe0, 0, 16];
        d.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        d.extend_from_slice(extra);
        d.extend_from_slice(&[0xff, sof, 0, 8 + 3 * u16::from(comps) as u8, 8]);
        d.extend_from_slice(&h.to_be_bytes());
        d.extend_from_slice(&w.to_be_bytes());
        d.push(comps);
        d
    }

    #[test]
    fn analyze_reads_size_and_components() {
        let i = analyze_jpeg(&jpeg(0xc0, 3, 37, 23, &[])).unwrap();
        assert_eq!((i.width, i.height, i.components, i.bits_per_component), (37, 23, 3, 8));
        assert!(!i.adobe);
        // progressive is accepted too
        assert_eq!(analyze_jpeg(&jpeg(0xc2, 1, 5, 6, &[])).unwrap().components, 1);
    }

    #[test]
    fn analyze_finds_the_adobe_marker() {
        let mut adobe = vec![0xff, 0xee, 0, 14];
        adobe.extend_from_slice(b"Adobe\0d\0\0\0\0\0");
        assert!(analyze_jpeg(&jpeg(0xc0, 4, 2, 2, &adobe)).unwrap().adobe);
        // too short a marker
        let mut short = vec![0xff, 0xee, 0, 7];
        short.extend_from_slice(b"Adobe");
        assert!(!analyze_jpeg(&jpeg(0xc0, 4, 2, 2, &short)).unwrap().adobe);
    }

    #[test]
    fn analyze_messages() {
        assert_eq!(
            analyze_jpeg(&[0xff, 0xd8, 0xff, 0xe0, 0, 16, 1]).unwrap_err(),
            "Error: unexpected end of JPEG file!"
        );
        assert_eq!(
            analyze_jpeg(&[0xff, 0xd8]).unwrap_err(),
            "Error: unexpected end of JPEG file!"
        );
        assert_eq!(
            analyze_jpeg(&[0xff, 0xd8, 0xff, 0xd9]).unwrap_err(),
            "Error: DNL marker not supported in PostScript Level 2!"
        );
        assert_eq!(
            analyze_jpeg(&jpeg(0xc0, 2, 3, 3, &[])).unwrap_err(),
            "Error: unknown color space (2 components)!\n"
        );
        assert!(analyze_jpeg(&jpeg(0xc0, 3, 0, 3, &[])).is_err());
    }

    #[test]
    fn analyze_hostile_input_ends() {
        for len in [0u8, 1, 2, 3] {
            let _ = analyze_jpeg(&[0xff, 0xd8, 0xff, 0xe0, 0, len]);
            let _ = analyze_jpeg(&[0xff, 0xd8, 0xff, 0xee, 0, len]);
            let _ = analyze_jpeg(&[0xff, 0xd8, 0xff, 0xdb, 0, len]);
        }
        let _ = analyze_jpeg(&[0xff; 100]);
        let _ = analyze_jpeg(&[0xff, 0xff, 0, 0xff, 0]);
    }

    #[test]
    fn wrapper_text() {
        let info = JpegInfo {
            width: 4,
            height: 3,
            components: 4,
            bits_per_component: 8,
            adobe: true,
        };
        let s = jpeg_to_ps(&info, "a.jpg", Some("D"), &[0, 0, 0, 0, 1]);
        assert!(s.contains("/DeviceCMYK setcolorspace\n"));
        assert!(s.contains("\t/ImageMatrix [ 4 0 0 -3 0 3 ]\n"));
        assert!(s.contains("\t  /Decode [1 0 1 0 1 0 1 0]\n"));
        assert!(s.contains("%%CreationDate: D\n"));
        assert!(s.ends_with("} exec\nz!<~>\n"));
    }
}
