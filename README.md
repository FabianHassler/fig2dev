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

## Languages and status

Every fig2dev language is ported except `sld` (slides): PostScript, EPS, PDF, SVG, PNG, GIF, JPEG, TIFF, PPM, PCX, XBM, XPM, LaTeX picture, epic, eepic, eepicemu, PiCTeX, TeXtyl, tpic, pic, pstex, pdftex, pspdftex, pict2e, TikZ, PSTricks, LaTeX box, CGM, HPGL, DXF, MetaPost, METAFONT, ge, image map, Gerber, shape, EMF, Tk and Perl-Tk. Input encodings (`-E`) include UTF-8, ISO-8859-N, KOI8, CP125x, Mac and CJK, through `encoding_rs`.

Against fig2dev 3.2.9a (C):

- fig2dev's own test suite: 82 pass, 6 fail (`epsi` and two embedded-PostScript bitmap tests that need a PostScript interpreter, the `/MediaBox` helper program, a version-2 font size, the KOI8-R case whose `iconv KOI8R` macOS lacks), 6 need unported languages.
- TeX family, plotters, EMF, Tk, pic, map: byte-identical to fig2dev on the test figures and on nearly all of xfig's ≈2,550 sample figures (the rest: Latin-1 text bytes, a few out-of-bounds reads in the C code).
- EPS/PDF/SVG/bitmaps: same geometry as fig2dev under ghostscript; the rasterised outputs differ slightly (antialiasing), and pictures in PDF and EPS pictures in SVG/bitmaps are not complete yet.

## Building and testing

```
cargo build --release          # target/release/fig2dev
cargo test --workspace
cargo xtask licences
```

Optional: `CRAFT_FONTS_DIR=/abs/path/to/craft-fonts cargo build` embeds the [craft-fonts](https://github.com/storytold/craft-fonts) collection (see `crates/figbase/build.rs`). Without it, Ubuntu Light and Hack from the `epaint_default_fonts` crate are used. No font files belong in this repository.

Tests that compare with the C program or its test data skip when these are missing:

- `FIG2DEV_SOURCES=/path/to/dir` must contain `fig2dev-3.2.9a/` (the unpacked source from <https://sourceforge.net/projects/mcj/files/>) and, for the sample-figure tests, `xfig-3.2.9a/`. The default is `old/` in the repository root (gitignored).
- fig2dev's own suite, which needs those sources: `FIG2DEV_SOURCES=... cargo test -p fig2dev-cli --test fig2dev_suite -- --ignored --nocapture` (report in `target/fig2dev-suite.txt`).
- Tests that call a reference fig2dev look for `/opt/homebrew/bin/fig2dev` and skip without it.

## Licence

Original code: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). Parts ported from xfig and fig2dev keep the xfig licence (`LICENSE-xfig`, which lists those files); see `NOTICE`.
