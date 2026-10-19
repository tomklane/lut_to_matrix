# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Built OpenDRT DCTLs (app *Build OpenDRT DCTL…* and `lutmatrix opendrt build`) have a
  *DCI - 2.6 Power / P3-D65* Display Encoding Preset (P3-D65, 2.6 power, dark surround),
  appended after the stock ones so existing grades keep their setting. The OpenDRT port
  resolves it too (`Presets::display_encoding` 9); checked against the compiled DCTL.

## [0.1.0] - 2026-10-17

### Changed
- The macOS app is ad-hoc signed (`signingIdentity: "-"`), so Apple Silicon Macs offer
  *Open Anyway* instead of reporting the app as damaged.
- The Windows NSIS installer installs per user (no admin rights).
- License: GPL-3.0-or-later (was unlicensed), to include a port of OpenDRT (GPLv3).
- D65 is now the default white mode (CLI `--white` defaults to `d65`).
- Analysis JSON writes matrices as row-major nested arrays.

### Added
- Release workflow (`.github/workflows/release.yml`, replaces `Bundle`): runs the full
  test suite; builds the universal macOS .dmg, the Windows NSIS (per-user) and MSI
  installers and the command-line tools for both; checks the .dmg contents (version,
  architectures, signature) and installs/uninstalls both Windows installers; publishes
  everything with `SHA256SUMS.txt` to a draft GitHub Release with notes from this file.
  `tools/release_notes.py` extracts the notes and checks the changelog before a release.
- README: Install (first launch of unsigned builds on macOS 15+ and Windows), building
  each installer, and Releasing.
- Desktop app (Tauri v2 + Svelte 5, `app/`): drag-and-drop or open a LUT; detected input
  and output spaces with confidence, reasons and overrides (ARRI EI, PQ peak); fit
  settings (white mode, anchor, bracket, metric, fit through a display transform,
  advanced options); live matrix, error statistics, reference comparison, before/after
  swatches per sample and exposure level, report; Export DCTL… and Copy Matrix; OpenDRT
  Match with safety checks and Save to Library; preset library (rename, export, import,
  remove, build OpenDRT DCTL); About; remembered settings; light/dark mode;
  macOS-style overlay title bar. LUT path accepted on the command line.
- CI and `tools/check.sh` type-check and build the front end; Linux CI installs WebKitGTK.
- Cargo workspace with `lutmatrix-core` (library) and `lutmatrix-cli` crates; version
  defined once in the root `Cargo.toml`.
- Project docs: `CLAUDE.md`, `DECISIONS.md`, `README.md`, the original brief.
- Colour math (f64): primaries → XYZ matrices, Bradford/CAT02 chromatic adaptation.
- Camera log spaces with published constants: ARRI LogC3 (EI 160–1600) / AWG3,
  ARRI LogC4 / AWG4, Sony S-Log3 / S-Gamut3 and S-Gamut3.Cine, RED Log3G10 / RWG,
  Canon Log 2 and Log 3 / Cinema Gamut, Panasonic V-Log / V-Gamut, Blackmagic Film
  Gen 5 / BMD WG, Fujifilm F-Log and F-Log2 / F-Gamut, DaVinci Intermediate / DWG,
  ACEScct / AP1.
- Display spaces: Rec.709, P3-D65, P3-DCI, Rec.2020 with gamma 2.2/2.4/2.6 or PQ.
- BT.2408-8 middle-gray defaults (26 cd/m² PQ; 26/203 of peak for SDR).
- colour-science fixture generator (`tools/gen_fixtures.py`) and reference tests for
  every curve, gamut matrix, CAT, PQ and DWG round trips.
- LUT reading: `.cube` (1D, 3D, Resolve shaper+3D, DOMAIN_MIN/MAX, LUT_*_INPUT_RANGE,
  TITLE, comments) and `.3dl` (Lustre/Flame, 10–16-bit). Tetrahedral (default) and
  trilinear interpolation. `.cube` writer for synthetic LUTs.
- Input/output space detection from filename, TITLE and comments with confidence
  levels and reasons; ARRI EI and PQ peak from names; gray probe (where 18% gray
  lands vs display middle gray); PQ inference from output range; diffuse-white hint
  ("DW100") for display middle gray.
- `lutmatrix inspect` CLI command (text or `--json`).
- CIELAB, CIEDE2000, ICtCp (BT.2100 PQ) and ΔE ITP (BT.2124), checked against
  colour-science and Sharma et al. (2005) test data.
- ALEXA LF / Mini LF detection keywords (LogC3 / AWG3).
- Sampling: CIELAB hue ring at middle-gray luminance (36 hues × 4 saturations by
  default) bounded by camera ∩ display gamut, optional ±N-stop bracket, anchor A
  (scene 0.18) or B (exposure where the LUT hits display middle gray), skipping of
  out-of-gamut / out-of-domain / clipped samples, exposure normalisation.
- Fitting: weighted least squares, neutral-constrained least squares, and
  Levenberg–Marquardt refinement on ΔE2000 (SDR) or ΔE ITP (HDR); preserve-LUT-white
  and D65 white modes (Bradford/CAT02 + exact neutral constraint); optional neutral lock.
- Fit report: mean/RMS/p95/max error, neutral error and Δu′v′, worst hues, exposure
  placement, contrast at gray, LUT white, and warnings for exposure-dependent colour,
  neutral drift, poor fit, many skipped samples; sRGB swatches per sample.
- `lutmatrix fit` CLI command with overrides for spaces, white mode, anchor, bracket,
  metric and neutral lock.
- Synthetic tests: matrix recovery across 10 camera/display pairs (exact and 65³),
  identity LUT → identity, P3-DCI white preserved/removed, tone-curve normalisation,
  anchor B, bracket, neutral lock, sample skipping.
- DCTL export: DaVinci Intermediate → linear DWG → `DWG_TO_XYZ` → `M_XYZ` (blended by a
  "Matrix Amount" slider) → `XYZ_TO_DWG` → DaVinci Intermediate, with a header stating
  source LUT, input/output spaces, white mode, middle gray, fit error, version and date.
- JSON and plain-text exports of M_XYZ, DWG↔XYZ, the least-squares matrix and the
  equivalent single linear-DWG matrix.
- `lutmatrix export` CLI command (writes .dctl, .json, .txt).
- DCTL tests: float32 emulation vs f64, golden file, C99/C++ compile-and-run check.
- Fit through a reference display transform (`--through <LUT>`): the matrix is fitted
  so that the DRT that follows the DCTL (a CST, OpenDRT, … exported from Resolve as a
  LUT) reproduces the source LUT, exposure matched, with a report of the gain over the
  DRT alone; the DCTL header names the reference.
- GitHub Actions CI: tests and clippy on macOS, Windows and Linux.
- OpenDRT LUT matching in Rust (`lutmatrix_core::opendrt_match`, CLI `lutmatrix opendrt
  fit`): fits every OpenDRT look parameter plus grey to a LUT, with safety checks (range,
  exposure-sweep luminance reversals with automatic brilliance freeze, smoothness vs stock
  looks).
- Local OpenDRT preset library (`lutmatrix_core::preset_store`; CLI `lutmatrix opendrt
  presets list|show|remove|import|export|path`): fits are saved per user on this machine;
  no LUT Match presets ship with the app or live in the repo.
- CI requires a C++ compiler on macOS and Linux so the OpenDRT parity test cannot skip.
- `lutmatrix opendrt build`: writes the bundled OpenDRT DCTL with presets from the local
  library as LUT Match look presets (per-preset grey only with Tonescale Preset = USE LOOK
  PRESET). The written DCTL is compiled and checked against the port in tests.
- ACES2065-1 (linear AP0) input space (`aces2065_ap0`), with detection; "709G24"-style
  names detected as gamma 2.4 / 2.2.
- `lutmatrix-opendrt` crate: a Rust port of OpenDRT v1.1.0 (Jed Smith, GPLv3) as used in
  Resolve (`OpenDRT_v1.1.0.1.dctl`, bundled): all look, tonescale, creative white and
  display encoding presets, every input gamut and transfer function, stickshift
  parameters. Bit-identical to the compiled DCTL in tests (479 configurations), plus a
  committed reference fixture for platforms without a C++ compiler.

[Unreleased]: https://github.com/tomklane/lut_to_matrix/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tomklane/lut_to_matrix/releases/tag/v0.1.0
