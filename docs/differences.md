# fig2dev (Rust) and fig2dev 3.2.9a (C)

This program is a Rust port of fig2dev 3.2.9a: the same command line, the same options, the same output on the languages it shares with C. This page lists what it adds or does on purpose differently. The comparison is with the C sources of 3.2.9a (`fig2dev` from [mcj on SourceForge](https://sourceforge.net/projects/mcj/files/)).

Contents: [How close](#how-close) · [Languages beyond C](#languages-beyond-c) · [sld](#sld) · [Options beyond C](#options-beyond-c) · [Behavior on purpose](#behavior-on-purpose) · [Not ported](#not-ported) · [Library API beyond the program](#library-api-beyond-the-program)

## How close

- fig2dev's own test suite (`make check` of 3.2.9a, run with `FIG2DEV_SOURCES=... cargo test -p fig2dev-rs --test fig2dev_suite -- --ignored --nocapture`, see the README): 88 pass, 0 fail, 1 skipped (`input.at`'s test of `japanese.ps`, which the suite itself skips unless the installed copy is checked).
- TeX family, plotters, EMF, Tk, pic, map: byte-identical to fig2dev on the test figures and on nearly all of xfig's roughly 2,550 sample figures. The rest differ in Latin-1 text bytes.
- EPS, PDF, SVG and bitmaps: the same geometry as fig2dev. C writes PostScript and has ghostscript turn it into PDF and bitmaps; this program writes PDF itself and rasterizes with tiny-skia, so PNG, GIF, JPEG and the other bitmaps differ from ghostscript's pixels at the edges (antialiasing, font hinting). They are compared visually, not byte for byte.

## Languages beyond C

| Language | Output |
|---|---|
| `asy` | [Asymptote](https://asymptote.sourceforge.io/): a `.asy` file with the figure's objects as Asymptote paths, fills, arrowheads and labels. Splines are written as vector paths, not as polylines. |

`asy` takes these options (its language is beyond C, so each of them is):

| Language | Option | Meaning |
|---|---|---|
| `asy` | `-b` | width of a blank border around the figure (1/72 inch) |
| `asy` | `-F` | do not set font family, series and shape |
| `asy` | `-f` | default text font |
| `asy` | `-i` | directory name to prepend to included graphics files |
| `asy` | `-O` | do not quote characters special to TeX (set the text flag "TeX" for all text) |
| `asy` | `-o` | do not set the font size |
| `asy` | `-S` | how splines are written: `guide` (default), `hobby` or `exact`, see below |
| `asy` | `-T` | only use TeX fonts, even when PostScript fonts are specified |
| `asy` | `-w` | remove the suffix from included graphics files |

`-S guide|hobby|exact`:

- `guide` (default) keeps the Fig points in the file. An approximated spline is `bspline(new pair[]{...}, closed)`, the uniform cubic B-spline of the points (a helper defined once in the file). An interpolated spline, or one with mixed shape factors, is one cubic `..controls a and b..` per X-spline segment.
- `hobby`: as `guide`, but an interpolated spline is Asymptote's own `..` curve through the points (Hobby's curve, which can look rounder than xfig's; a spline with arrowheads keeps the cubics).
- `exact`: every spline is written as cubics, split until they are within 0.5 Fig units of xfig's curve. Long paths are joined from pieces (`xfigjoin`).

## sld

`-L sld` (AutoCAD slide) is ported. C pipes ghostscript's output into netpbm's `ppmtoacad`. Here the built-in rasterizer's pixels are encoded as `ppmtoacad` does with fig2dev's options (the slide header, each color mapped to the nearest of AutoCAD's 256 colors, one vector per horizontal run of equal color), so neither ghostscript nor netpbm is needed.

## Options beyond C

Apart from `asy` (above), no language takes a letter that C's driver for it does not take.

`-E` (the input encoding) is in C too; it converts with iconv there. Here [`encoding_rs`](https://crates.io/crates/encoding_rs) does it, so the charsets (UTF-8, ISO-8859-N, KOI8, CP125x, Mac, CJK, and iconv's usual spellings of their names) are the same on every platform and need no iconv.

The test `crates/fig2dev/tests/differences_doc.rs` derives the options beyond C from the drivers and the options table, compares them with C's option inventory (`crates/fig2dev/tests/c_options.rs`) and fails when an option is missing from the `asy` table above, or a row there has no option behind it. It prints the rows to paste.

## Behavior on purpose

- **Drawing order.** Objects sharing a depth come in xfig's order (arcs, compounds, ellipses, lines, splines, texts, each in file order), as xfig shows them, in every language and the same on every platform.
- **Texts.** Leading blanks of a text are kept (all but the separating one), as xfig reads them.
- **No external programs.** PDF is written natively; the bitmaps are drawn by tiny-skia; `sld` needs no `ppmtoacad`; text comes from installed fonts with built-in fallbacks (`FIG2DEV_BUILTIN_FONTS=1` forces those).
- **PDF pictures stay vectors.** PDF output embeds the first page of a PDF picture as a Form XObject (`hayro-write`); PostScript and EPS output get it as EPS written by a PostScript device on `hayro-interpret` (C: `pdftops`), as an image only when the page uses transparency, shadings or patterns (with a message); the bitmap languages render it with `hayro`; SVG links the file. Known limits: encrypted PDFs, PDF blend modes, knockout groups; CMYK colors are converted to RGB. JPEG pictures in PDF keep their data (`/DCTDecode`).
- **EPS pictures.** In PostScript and EPS output an EPS picture is copied in, as before. In PDF and bitmap output it is drawn by ghostscript when `gs` is on `PATH` (run without a shell, with `-dSAFER`, a time and a size limit, never on wasm), else from the preview image the file carries (bitmaps), or left out (PDF, with fig2dev's kind of message). This is the only place ghostscript is used, and it is optional.

## Not ported

`fig2ps2tex`, `pic2tpic` and `transfig` (shell scripts and a front end around fig2dev). Everything `fig2dev` itself does is ported.

## Library API beyond the program

The crates `figbase` and `fig2dev-rs-lib` (what RustFig uses) have settings the `fig2dev` program never changes, so its output never depends on them:

- `Ctx::user_bounds`: a fixed export box (llx, lly, urx, ury in Fig units) that replaces the objects' bounds. EPS then clips to its `%%BoundingBox`; PDF, the bitmaps and SVG crop to their page.
- `Ctx::typesetter` (`scene::typeset::TextTypesetter`): typesets special texts (LaTeX) outside fig2dev and gives the page each text's box; without one the bounds are fig2dev's.
- `gendev::latex_preamble` and `gendev::LATEX_PREAMBLE_MARKER`: a figure's LaTeX preamble, saved as whole-figure comment lines `# RustFigLaTeXPreamble <line>`. The complete documents of `tikz`, `pict2e` and `pstricks` (`-P`) and `asy` use it; everywhere else the lines stay comments.
- `Ctx::keep_splines` (`Driver::keeps_splines`): splines are not flattened for a driver that writes them as paths (`asy`).
- `trans_spline::spline_to_bezier`: an X-spline as a path of cubic Béziers.
