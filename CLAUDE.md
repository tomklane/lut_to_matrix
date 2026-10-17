# CLAUDE.md — LUT Matrix

Guidance for future sessions working in this repo. Keep it current.

## What this is
A cross-platform (Windows + macOS) desktop app for a colorist. It reads a 3D LUT that
converts camera log → display, measures how the LUT transforms colour around
middle-gray exposure, fits a 3x3 matrix in linear CIE XYZ that best approximates that
colour transform, and exports a DaVinci Resolve DCTL (DWG/DaVinci Intermediate in and out,
with explicit DWG_TO_XYZ, M_XYZ, XYZ_TO_DWG steps).

The full original brief is in `docs/BRIEF.md`. Colour-science and architecture choices
and their reasons are in `DECISIONS.md` — read it before changing any colour math.

## Layout
```
Cargo.toml                 workspace; [workspace.package].version is THE version
crates/lutmatrix-core      all colour math, LUT I/O, fitting, DCTL generation. No UI deps.
  src/math.rs              Mat3/Vec3 (f64), primaries→XYZ, chromatic adaptation
  src/spaces/              colour space registry
    camera.rs              camera log curves + gamuts (one entry per space; add new ones here)
    display.rs             display gamuts + transfer functions (gamma, PQ)
    davinci.rs             DaVinci Wide Gamut + DaVinci Intermediate
  src/lut/                 .cube / .3dl parsing, 1D/3D apply, tetrahedral/trilinear
  src/detect.rs            guess input/output spaces from filename/TITLE/comments
crates/lutmatrix-cli       `lutmatrix` binary (inspect, fit, export)
tools/                     Python (colour-science) scripts that generate test fixtures
crates/lutmatrix-core/tests/fixtures   committed reference values
app/                       Tauri v2 app (stage 4)
```

## Conventions
- All colour/fitting math in `f64`. The only `f32` code is the DCTL-pipeline emulation test.
- Every constant from a published spec carries a source comment (document name, version,
  section/page). If you add a camera space, add: curve encode/decode, primaries, white,
  a source comment, detection keywords, a fixture row and a round-trip test.
- Ambiguous colour-science choices become settings, not hard-coded values; log them in
  `DECISIONS.md`.
- Run `tools/check.sh` (clippy -D warnings + tests) before every commit; it exits
  non-zero on any failure.
- Small commits, one logical change each. Update `CHANGELOG.md` (Keep a Changelog) with
  every change. Run `cargo test` and `cargo clippy --all-targets -- -D warnings` before
  every commit; never commit with failures.
- Version lives only in the root `Cargo.toml` (`[workspace.package] version`). Code reads
  it via `lutmatrix_core::VERSION`.

## Build & test
```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p lutmatrix-cli -- inspect path/to/file.cube
```
Regenerate fixtures (needs `pip install colour-science`):
```
python3 tools/gen_fixtures.py
```

## Status
See `CHANGELOG.md` "Unreleased" for what's done. Build stages:
1. LUT parsing + colour space / log curve math + tests
2. Sampling + fitting
3. DCTL export
4. GUI (Tauri v2 + Svelte)
5. Installers + CI
