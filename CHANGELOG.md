# Changelog

## Unreleased

- Split out of the RustFig repository with its history: crates `figbase` and `fig2dev` (library) and the `fig2dev` program (package `fig2dev-cli`).
- FMA parity with the reference binary (audit pass 12): `ellipse_bound` (2 fused ops), `arc_bound` radius and trans_spline (blend functions, point weights, float step angle) fuse `a*b+c` as clang does; exact against oracle programs on 412,800 ellipses and 20,000 splines (tests/fma_oracle.rs). The "closed spline with less than three points" message has no newline, as in C.
