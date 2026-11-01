//! Composite fonts for Japanese and Korean text (`-j`): `genps.c`'s
//! `append_find_composite` and the text branch of `genps_text`.
//!
//! The PostScript prologue comes from the locale (`LC_ALL`, `LC_CTYPE`,
//! `LANG`): fig2dev reads `<locale>.ps` from its i18n directory, where
//! `japanese.ps` and `korean.ps` are linked under the names of the EUC
//! locales (`i18n/Makefile.am`). Both files are built in here. Texts in the
//! composite fonts switch between the ASCII font and the EUC font with
//! `\377\001` and `\377\000`.
//
// Ported from fig2dev dev/genps.c and i18n/{japanese,korean}.ps (xfig
// licence, see LICENSE-xfig).

use std::fmt::Write as _;

/// The EUC charset of the locale's composite fonts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    EucJp,
    EucKr,
}

/// What `-j` needs once the locale's prologue is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Composite {
    /// The prologue (`japanese.ps` or `korean.ps`).
    pub prolog: &'static str,
    pub charset: Charset,
    /// The figure has no input encoding (no `-E`, no `#encoding:` line):
    /// a text's characters are the bytes of the file, written as they are
    /// (fig2dev's way). Otherwise the text is Unicode and goes into the
    /// locale's charset.
    pub raw_bytes: bool,
}

const JAPANESE: &str = include_str!("i18n/japanese.ps");
const KOREAN: &str = include_str!("i18n/korean.ps");

/// The prologue for a locale name, by the links `i18n/Makefile.am` makes.
pub fn for_locale(locale: &str, raw_bytes: bool) -> Option<Composite> {
    let (prolog, charset) = match locale {
        "ja_JP.eucJP" | "ja_JP.EUC" | "ja_JP" | "ja_JP.ujis" | "ja" | "japanese" => (JAPANESE, Charset::EucJp),
        "ko_KR.eucKR" | "ko_KR.EUC" | "ko_KR" | "ko" | "korean" => (KOREAN, Charset::EucKr),
        _ => return None,
    };
    Some(Composite {
        prolog,
        charset,
        raw_bytes,
    })
}

/// `setlocale(LC_CTYPE, "")`'s name: `LC_ALL`, else `LC_CTYPE`, else `LANG`
/// (an empty variable counts as unset); `"C"` if none is set.
pub fn locale_from_env(get: impl Fn(&str) -> Option<String>) -> String {
    for var in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Some(v) = get(var).filter(|v| !v.is_empty()) {
            return v;
        }
    }
    "C".to_string()
}

impl Composite {
    /// The bytes of a text line in the locale's charset.
    pub fn encode(&self, line: &str) -> Vec<u8> {
        if self.raw_bytes && line.chars().all(|c| u32::from(c) <= 0xff) {
            return line.chars().map(|c| c as u32 as u8).collect();
        }
        let enc = match self.charset {
            Charset::EucJp => encoding_rs::EUC_JP,
            Charset::EucKr => encoding_rs::EUC_KR,
        };
        let mut out = Vec::new();
        let mut buf = [0u8; 4];
        for c in line.chars() {
            let (bytes, _, bad) = enc.encode(c.encode_utf8(&mut buf));
            // A character the charset lacks is left out (iconv would stop).
            if !bad {
                out.extend_from_slice(&bytes);
            }
        }
        out
    }

    /// The PostScript string of a text line (`genps_text`): `(`, `)` and
    /// `\` escaped, `\377\001` before a run of bytes beyond ASCII (the EUC
    /// font) and `\377\000` after it. Bytes beyond ASCII are octal escapes
    /// where fig2dev writes them raw. The second value is false if a run
    /// has an odd number of bytes (an incomplete multibyte character, which
    /// gets a `?` as in fig2dev).
    pub fn ps_string(&self, line: &str) -> (String, bool) {
        let bytes = self.encode(line);
        let mut o = String::from("(");
        let (mut gr, mut gr_chars, mut ok) = (false, 0usize, true);
        let mut chars = 0usize;
        for &b in &bytes {
            if chars > 200 {
                o.push_str("\\\n");
                chars = 0;
            }
            if b & 0x80 != 0 {
                if !gr {
                    o.push_str("\\377\\001");
                    chars += 8;
                    gr = true;
                    gr_chars = 0;
                }
                gr_chars += 1;
            } else if gr {
                if gr_chars % 2 == 1 {
                    ok = false;
                    o.push('?');
                }
                o.push_str("\\377\\000");
                chars += 8;
                gr = false;
            }
            if matches!(b, b'(' | b')' | b'\\') {
                o.push('\\');
                chars += 1;
            }
            if b & 0x80 != 0 || b == b'\r' || b == 0 {
                let _ = write!(o, "\\{b:03o}");
                chars += 4;
            } else {
                o.push(char::from(b));
                chars += 1;
            }
        }
        if gr && gr_chars % 2 == 1 {
            ok = false;
            o.push('?');
        }
        o.push(')');
        (o, ok)
    }
}

/// Whether a text is set in a composite font (`genps_text`): a PostScript
/// font Times-Roman, Times-Bold or the default, or a LaTeX font up to Bold.
/// The returned flag is true for the bold one.
pub fn font_of(postscript: bool, font: i32) -> Option<bool> {
    // fig2dev sets LaTeX font 1 (roman) in CompositeBold (its test is
    // `font <= 0` for roman); roman is meant.
    let composite = if postscript { font <= 0 || font == 2 } else { font <= 2 };
    composite.then_some(font == 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales() {
        assert!(for_locale("ja_JP.eucJP", true).is_some_and(|c| c.charset == Charset::EucJp));
        assert!(for_locale("ko_KR", true).is_some_and(|c| c.charset == Charset::EucKr));
        assert!(for_locale("ja_JP.UTF-8", true).is_none());
        assert!(for_locale("C", true).is_none());
    }

    #[test]
    fn env_order() {
        let env = |k: &str| match k {
            "LANG" => Some("ko".to_string()),
            "LC_CTYPE" => Some(String::new()),
            _ => None,
        };
        assert_eq!(locale_from_env(env), "ko");
        assert_eq!(locale_from_env(|_| None), "C");
    }

    #[test]
    fn strings() {
        let c = for_locale("ja", false).unwrap();
        let (s, ok) = c.ps_string("a(\u{3042})");
        assert_eq!(s, "(a\\(\\377\\001\\244\\242\\377\\000\\))");
        assert!(ok);
        let raw = for_locale("ja", true).unwrap();
        let (s, _) = raw.ps_string("\u{a4}\u{a2}x");
        assert_eq!(s, "(\\377\\001\\244\\242\\377\\000x)");
        let (s, ok) = raw.ps_string("\u{a4}");
        assert_eq!(s, "(\\377\\001\\244?)");
        assert!(!ok);
    }
}
