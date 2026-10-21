//! Helpers the drivers share (fig2dev's `dev/` support files): TeX and
//! PostScript font tables, `\SetFigFont`, ISO-8859 → TeX, text encoding,
//! picture reading, the creation date.
//!
//! - [`texfonts`]: `dev/texfonts.c` and the parts of `dev/textconvert.c` it
//!   uses (`put_string`, `select_font`, the TeX font tables).
//! - [`textconvert`]: `dev/textconvert.c` (the bytes of a text, conversions).
//! - [`iso2tex`]: `iso2tex.c` (ISO-8859-1/-2 → TeX sequences).
//! - [`psfonts`]: `dev/psfonts.c` (PostScript font names, `unpsfont`).
//! - [`setfigfont`]: `dev/setfigfont.c` (the `\SetFigFont` macro).
//! - [`latex`]: `dev/genlatex.h` (`set_color`, `reset_color`).
//! - [`creationdate`]: `creationdate.c` (honours `SOURCE_DATE_EPOCH`).
//! - [`readjpg`]: `dev/readjpg.c`, `encode.c` (JPEG header, ASCII85, the DCT passthrough).
//! - [`readpics`]: `dev/readpics.c` and the `dev/read*.c` picture readers.
//! - [`pdfpic`], [`pdfps`]: PDF pictures through the `hayro` crates (`pdftops` of `dev/readeps.c`).
//! - `gs`: ghostscript for EPS pictures in PDF and bitmap output (not on wasm).

pub mod creationdate;
#[cfg(not(target_arch = "wasm32"))]
pub mod gs;
pub mod iso2tex;
pub mod latex;
pub mod pdfpic;
pub mod pdfps;
pub mod psfonts;
pub mod readjpg;
pub mod readpics;
pub mod setfigfont;
pub mod texfonts;
pub mod textconvert;
pub mod tkpattern;
