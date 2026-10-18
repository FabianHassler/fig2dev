# Changelog

## Unreleased

- Header metadata (audit pass 16): eps/ps write `%%Title` (`-n`, else the input name, else stdin), `%%Creator: fig2dev Version 3.2.9a`, `%%CreationDate` (`SOURCE_DATE_EPOCH`, else UTC now), `%%Magnification`, the `%%DocumentNeededResources` CMap lines for non-ASCII text and the whole-figure comments before the prolog, in genps's order (no `%%HiResBoundingBox`/`%%LanguageLevel`); svg writes gensvg's preamble, Creator/CreationDate/Magnification comments, `<svg>` with `width`/`height` in points, `<desc>` comments and the `<g fill="none">` wrapper; pdf has an Info dictionary (Title, Creator, Producer, CreationDate, ModDate). Header blocks equal fig2dev's on 311/311 fixtures and sample figures (tests/header_meta.rs).
- Split out of the RustFig repository with its history: crates `figbase` and `fig2dev` (library) and the `fig2dev` program (package `fig2dev-cli`).
- FMA parity with the reference binary (audit pass 12): `ellipse_bound` (2 fused ops), `arc_bound` radius and trans_spline (blend functions, point weights, float step angle) fuse `a*b+c` as clang does; exact against oracle programs on 412,800 ellipses and 20,000 splines (tests/fma_oracle.rs). The "closed spline with less than three points" message has no newline, as in C.
