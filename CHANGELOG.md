# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- License: GPL-3.0-or-later (was unlicensed), to include a port of OpenDRT (GPLv3).
- D65 is now the default white mode (CLI `--white` defaults to `d65`).
- Analysis JSON writes matrices as row-major nested arrays.

### Added
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
