# fig2dev (Rust)

A Rust port of [fig2dev](https://mcj.sourceforge.net/) 3.2.9a, the converter that turns xfig's Fig files into PostScript, PDF, SVG, LaTeX and many other formats. The `fig2dev` program here is a drop-in replacement: it follows fig2dev's command line, messages and exit codes. It has no C dependencies and builds on macOS, Linux and Windows; the library also compiles to WebAssembly.

It was developed as part of [RustFig](https://github.com/FabianHassler/rustfig), a Rust reimplementation of xfig, which uses it as a library.

## Contents

| Crate | Path | Role |
|---|---|---|
| `figbase` | `crates/figbase` | what fig2dev and RustFig share: geometry, X-splines, arrowheads, Fig fonts (tables, metrics, glyph outlines), colours, styles |
| `fig2dev-rs-lib` | `crates/fig2dev` | the library (crate `fig2dev` in code): Fig reader (1.3 to 3.2), options, drivers, scene (display list), rasteriser (tiny-skia), fill patterns |
| `fig2dev-rs` | `apps/fig2dev` | the command line; the binary is called `fig2dev` |
| `xtask` | `xtask` | `cargo xtask licences` |

The name `fig2dev` belongs to the C program, so the packages are called `fig2dev-rs` (the program, `[[bin]] name = "fig2dev"`) and `fig2dev-rs-lib` (the library, `[lib] name = "fig2dev"`): `cargo install fig2dev-rs` installs `fig2dev`, and `fig2dev-rs-lib = "0.9"` gives the library, used as `fig2dev::…`.

## Installation

You need Rust 1.92 or newer from [rustup](https://rustup.rs). There are no C libraries to install.

From crates.io:

```
cargo install --locked fig2dev-rs
```

Straight from GitHub, a given release:

```
cargo install --locked --git https://github.com/FabianHassler/fig2dev --tag v0.9.3 fig2dev-rs
```

As a library: `fig2dev-rs-lib = "0.9"` (used as `fig2dev::…`; and `figbase = "0.9"` for the shared geometry, fonts and colours) in your `Cargo.toml`.

Or from a clone:

```
git clone https://github.com/FabianHassler/fig2dev
cd fig2dev
cargo install --locked --path apps/fig2dev
```

Either way the program is installed as `fig2dev` in `~/.cargo/bin` (on Windows `%USERPROFILE%\.cargo\bin`), which rustup puts on your `PATH`. If the C fig2dev is installed too (for example from Homebrew or a Linux package), the one that comes first on `PATH` runs; `which -a fig2dev` lists both. Remove it again with `cargo uninstall fig2dev-rs`.

## Languages and status

Every fig2dev language is ported: PostScript, EPS, PDF, SVG, PNG, GIF, JPEG, TIFF, PPM, PCX, XBM, XPM, LaTeX picture, epic, eepic, eepicemu, PiCTeX, TeXtyl, tpic, pic, pstex, pdftex (and their text halves pstex_t, pdftex_t), pict2e, TikZ, PSTricks, LaTeX box, CGM, HPGL, DXF, MetaPost, METAFONT, ge, image map, Gerber, shape, EMF, Tk, Perl-Tk and AutoCAD slides (`sld`); beyond C's list, Asymptote (`asy`). Input encodings (`-E`) include UTF-8, ISO-8859-N, KOI8, CP125x, Mac and CJK, through `encoding_rs`.

How this program differs from fig2dev 3.2.9a (C), what it adds, what it does on purpose and what is not ported: [docs/differences.md](docs/differences.md). fig2dev's own test suite passes: 88 pass, 0 fail, 1 skipped.

## Building and testing

```
cargo build --release          # target/release/fig2dev
cargo test --workspace
cargo xtask licences
```

Text uses installed fonts (URW/Nimbus, macOS and Windows system faces, TeX Gyre from a TeX installation, found with `fontdb`) and falls back to Ubuntu Light and Hack from the `epaint_default_fonts` crate. `FIG2DEV_BUILTIN_FONTS=1` (or `figbase::text::set_builtin_fonts(true)`) forces the built-in fonts, which tests and reference comparisons use; `figbase::text::add_font_dirs` adds font folders. No font files belong in this repository.

Tests that compare with the C program or its test data skip when these are missing:

- `FIG2DEV_SOURCES=/path/to/dir` must contain `fig2dev-3.2.9a/` (the unpacked source from <https://sourceforge.net/projects/mcj/files/>) and, for the sample-figure tests, `xfig-3.2.9a/`. The default is `old/` in the repository root (gitignored).
- fig2dev's own suite, which needs those sources: `FIG2DEV_SOURCES=... cargo test -p fig2dev-rs --test fig2dev_suite -- --ignored --nocapture` (report in `target/fig2dev-suite.txt`).
- Tests that call a reference fig2dev look for `/opt/homebrew/bin/fig2dev` and skip without it.

## Licence

Original code: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). Parts ported from xfig and fig2dev keep the xfig licence (`LICENSE-xfig`, which lists those files); see `NOTICE`.
