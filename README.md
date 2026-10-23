# fig2dev (Rust)

A Rust port of [fig2dev](https://mcj.sourceforge.net/) 3.2.9a, the converter that turns xfig's Fig files into PostScript, PDF, SVG, LaTeX and many other formats. The `fig2dev` program here is a drop-in replacement: it follows fig2dev's command line, messages and exit codes. It has no C dependencies and builds on macOS, Linux and Windows; the library also compiles to WebAssembly.

It was developed as part of [RustFig](https://github.com/FabianHassler/rustfig), a Rust reimplementation of xfig, which uses it as a library.

## Contents

| Crate | Path | Role |
|---|---|---|
| `figbase` | `crates/figbase` | what fig2dev and RustFig share: geometry, X-splines, arrowheads, Fig fonts (tables, metrics, glyph outlines), colours, styles |
| `fig2dev` | `crates/fig2dev` | the library: Fig reader (1.3 to 3.2), options, drivers, scene (display list), rasteriser (tiny-skia), fill patterns |
| `fig2dev-cli` | `apps/fig2dev` | the command line; the binary is called `fig2dev` |
| `xtask` | `xtask` | `cargo xtask licences` |

The library crate and the program share the name `fig2dev`; the program lives in the package `fig2dev-cli` with `[[bin]] name = "fig2dev"`, so `cargo install --path apps/fig2dev` installs `fig2dev` and `fig2dev = { git = ... }` gives the library.

## Installation

You need Rust 1.92 or newer from [rustup](https://rustup.rs). There are no C libraries to install, and the crates are not on crates.io.

Straight from GitHub, a given release:

```
cargo install --locked --git https://github.com/FabianHassler/fig2dev --tag v0.6.0 fig2dev-cli
```

Or from a clone:

```
git clone https://github.com/FabianHassler/fig2dev
cd fig2dev
cargo install --locked --path apps/fig2dev
```

Either way the program is installed as `fig2dev` in `~/.cargo/bin` (on Windows `%USERPROFILE%\.cargo\bin`), which rustup puts on your `PATH`. If the C fig2dev is installed too (for example from Homebrew or a Linux package), the one that comes first on `PATH` runs; `which -a fig2dev` lists both. Remove it again with `cargo uninstall fig2dev-cli`.

## Languages and status

Every fig2dev language is ported: PostScript, EPS, PDF, SVG, PNG, GIF, JPEG, TIFF, PPM, PCX, XBM, XPM, LaTeX picture, epic, eepic, eepicemu, PiCTeX, TeXtyl, tpic, pic, pstex, pdftex, pspdftex, pict2e, TikZ, PSTricks, LaTeX box, CGM, HPGL, DXF, MetaPost, METAFONT, ge, image map, Gerber, shape, EMF, Tk, Perl-Tk and AutoCAD slides (`sld`). Input encodings (`-E`) include UTF-8, ISO-8859-N, KOI8, CP125x, Mac and CJK, through `encoding_rs`.

Against fig2dev 3.2.9a (C):

- fig2dev's own test suite: 82 pass, 6 fail (`epsi` and two embedded-PostScript bitmap tests that need a PostScript interpreter, the `/MediaBox` helper program, a version-2 font size, the KOI8-R case whose `iconv KOI8R` macOS lacks), 6 need unported languages.
- TeX family, plotters, EMF, Tk, pic, map: byte-identical to fig2dev on the test figures and on nearly all of xfig's ≈2,550 sample figures (the rest: Latin-1 text bytes, a few out-of-bounds reads in the C code).
- EPS/PDF/SVG/bitmaps: same geometry as fig2dev. The C program writes PostScript and has ghostscript turn it into PDF and bitmaps; this port writes PDF itself and rasterises with tiny-skia, so PNG, GIF, JPEG etc. differ from ghostscript's pixels at the edges (antialiasing, font hinting) and are compared visually, not byte for byte.
- PDF and EPS picture objects: a PDF file as a picture stays a vector. PDF output embeds its first page as a Form XObject (`hayro-write`); PostScript and EPS output get it as EPS written by a PostScript device on `hayro-interpret` (C: `pdftops`), as an image only when the page uses transparency, shadings or patterns (with a message); the bitmap languages render it with `hayro`; SVG links the file as gensvg does. An EPS picture is copied in as before in PostScript and EPS output (and pstex/pspdftex); in PDF and bitmap output it is drawn by ghostscript when `gs` is on `PATH` (run without a shell, `-dSAFER`, with a time and size limit, never on wasm), else from the preview image the file carries (bitmaps), or left out (PDF, with fig2dev's kind of message). JPEG pictures in PDF keep their data (`/DCTDecode`). Known limits: encrypted PDFs, PDF blend modes, knockout groups; CMYK colours are converted to RGB.

## Building and testing

```
cargo build --release          # target/release/fig2dev
cargo test --workspace
cargo xtask licences
```

Text uses installed fonts (URW/Nimbus, macOS and Windows system faces, TeX Gyre from a TeX installation, found with `fontdb`) and falls back to Ubuntu Light and Hack from the `epaint_default_fonts` crate. `FIG2DEV_BUILTIN_FONTS=1` (or `figbase::text::set_builtin_fonts(true)`) forces the built-in fonts, which tests and reference comparisons use; `figbase::text::add_font_dirs` adds font folders. No font files belong in this repository.

Tests that compare with the C program or its test data skip when these are missing:

- `FIG2DEV_SOURCES=/path/to/dir` must contain `fig2dev-3.2.9a/` (the unpacked source from <https://sourceforge.net/projects/mcj/files/>) and, for the sample-figure tests, `xfig-3.2.9a/`. The default is `old/` in the repository root (gitignored).
- fig2dev's own suite, which needs those sources: `FIG2DEV_SOURCES=... cargo test -p fig2dev-cli --test fig2dev_suite -- --ignored --nocapture` (report in `target/fig2dev-suite.txt`).
- Tests that call a reference fig2dev look for `/opt/homebrew/bin/fig2dev` and skip without it.

## Licence

Original code: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). Parts ported from xfig and fig2dev keep the xfig licence (`LICENSE-xfig`, which lists those files); see `NOTICE`.
