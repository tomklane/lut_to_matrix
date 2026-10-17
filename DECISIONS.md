# Decisions

Colour-science and architecture choices, with reasons. Newest at the bottom.
Each entry: what, why, alternatives considered, and whether it is a user setting.

## D1 — GUI framework: Tauri v2 + Svelte 5 + TypeScript
- **Why:** one web frontend can follow Apple's HIG closely on both macOS and Windows
  (system font stack, segmented controls, pop-up menus, `prefers-color-scheme`), with
  native file drag-and-drop, small bundles, and WebView2 already present on Windows 10/11.
- **Alternatives:** egui / iced (immediate/retained Rust UIs that can't credibly look
  HIG-native), Slint (possible, but every control custom-built), SwiftUI + WinUI (two UIs).

## D2 — Repository workflow
- GitHub (`tomklane/lut_to_matrix`) is the source of truth; work happens in a cloud
  workspace and a snapshot is copied to the user's Dropbox folder after each stage.
- **Why:** a live `.git` inside Dropbox risks sync corruption; CI needs GitHub anyway.

## D3 — Numeric precision
- All colour math and fitting in `f64`. DCTL constants are written with 10 significant
  decimals, more than float32 can hold, so the DCTL rounds once at compile time.

## D4 — Signal range
- LUT input/output are assumed **full-range** (data levels), as Resolve applies `.cube`
  files. A legal-range override is available per LUT (setting).
- Camera curves are implemented to output full-range normalised code values. Where a
  manufacturer publishes the curve in legal-range terms (e.g. Canon "IRE"), the
  conversion is done explicitly and noted at the curve.

## D5 — Middle-gray anchor: user-selectable (A or B)
- **A (default):** sample at scene-linear 0.18 (setting). The display middle-gray setting
  is used to report where the LUT places 18% gray relative to it (in stops).
- **B:** solve for the scene exposure that the LUT maps to the display middle-gray
  setting, and sample there.
- In both modes the fitted matrix is normalised so neutral gray has unity gain, because
  it runs on scene-referred DWG footage.

## D6 — ACES (AP1) white
- AP1 uses the ACES white (≈D60). Input XYZ is chromatically adapted to D65 (Bradford)
  so "neutral in" is always D65, matching Resolve's colour space transform behaviour.
  Option to use unadapted XYZ.

## D7 — Default display middle gray
- **PQ:** 26 cd/m² — ITU-R BT.2408-8 (11/2024) §2.2 Table 1, "Grey Card (18%)", 38% PQ.
  (HDR Reference White 203 cd/m², 58% PQ.)
- **SDR gamma:** no ITU value exists. Default = 26/203 of SDR peak ≈ 12.81% (12.8 cd/m²
  at 100 nits; code ≈ 0.393 / 0.425 / 0.453 at gamma 2.2 / 2.4 / 2.6). Same placement
  relative to reference white as BT.2408. A convention, so it is a setting.
- SDR outputs carry a nominal peak (default 100 cd/m²) so all display-linear values
  are in cd/m².

## D8 — RGB↔XYZ matrices derived from primaries
- Every gamut's NPM is computed in f64 from its published xy primaries and white
  (SMPTE RP 177), not copied from rounded published matrices. Tests confirm the derived
  matrices agree with every manufacturer-published matrix to its printed precision
  (DWG to 8 decimals per the BMD document; AWG4 to 10 decimals).

## D9 — Manufacturer constants kept verbatim, including their small gaps
- Panasonic V-Log's decode breakpoint (0.181) is a rounding of the encode breakpoint
  (0.1809997); Fujifilm's F-Log/F-Log2 decode breakpoints don't meet the encode
  breakpoints exactly (~1e-5 overlap in deep shadow). We use the published values as
  printed; tests allow exactly these gaps at the breakpoints. Effect on fitting: nil
  (samples sit around middle gray).

## D10 — Camera curve variants
- ARRI LogC3: SUP 3.x, "linear scene exposure factor" parameter table (EI 160–1600);
  EI is a setting (default 800).
- Canon Log 2/3: v1.2 ("Full %") formulas, which output full-range code values directly.
- RED Log3G10: v3 (with linear segment below −0.01), the current IPP2 definition.
- Fujifilm F-Gamut = BT.2020 primaries (F-Log data sheet Ver.1.1).
- Blackmagic WG Gen 5 uses BMD's own white (0.3127170, 0.3290312), adapted to D65 like ACES.

## D11 — LUT application
- Inputs outside a LUT's domain are clamped (as Resolve does), but the sampler checks
  `Lut::in_domain` first and skips such samples rather than measuring clamped output.
- Tetrahedral interpolation is the default (Resolve's default; exact on the neutral
  diagonal). Trilinear is an option.
- In a Resolve shaper+3D `.cube`, DOMAIN_MIN/MAX (or LUT_1D_INPUT_RANGE) belong to the
  shaper; the 3D table uses LUT_3D_INPUT_RANGE or [0, 1].

## D12 — Detection
- Evidence = weighted keywords (per camera space in the registry; per display gamut and
  transfer in `detect.rs`) matched on lower-case, separator-free text from filename,
  TITLE and comments. A keyword inside a longer matched keyword isn't counted twice.
- Gray probe: each text-matched camera candidate's 18% gray is run through the LUT; up
  to +1.5 evidence if it lands within 1.5 stops of display middle gray. With no text
  evidence the probe alone picks a candidate and confidence is reported as "none".
- Output never above 0.80 → +2 evidence for PQ; PQ peak from "1K"/"1000nits" in the
  name, else estimated from the maximum output code and snapped to a preset.
- ARRI's HDR LUTs are named "DW100" and place 18% gray at ≈13 nits (diffuse white at
  100 nits), a stop below BT.2408's 26 nits. When "DW<n>" is in the name we suggest
  display middle gray = n × 26/203.

## D13 — Test colours
- Hue ring in CIELAB (D65, Yn = 1) at constant Y = anchor gray (0.18 in mode A), default
  36 hues × saturations {25, 50, 75, 95}% + neutral. Saturation is a fraction of the
  per-hue maximum chroma that stays inside **both** the camera gamut and the display
  gamut (setting: camera only). Rationale: measure colours the display can show, not the
  LUT's gamut compression, which no 3×3 matrix can model.
- Optional exposure bracket ±N stops (setting, default off) reuses the same
  chromaticities scaled in luminance.
- Skipped: camera RGB < 0, LUT input outside its domain, output code ≤ 0.0005 or within
  0.001 of the top code (1.0, or PQ(peak) for HDR). Thresholds are settings.

## D14 — Exposure normalisation
- On (default): per exposure level, the LUT output is scaled by k = Y_in(gray) /
  Y_out(gray), so the matrix has unity gain on neutral and captures colour only.
- Off: a fixed k = scene middle gray / display middle gray, so the matrix also carries
  the LUT's exposure placement relative to the display middle-gray setting (reported).

## D15 — D65 white mode: chromatic adaptation **and** a neutral constraint
- Chosen approach: (1) measure the white the LUT renders for scene neutral; (2) adapt
  the LUT's output from that white to D65 with a CAT (Bradford default, CAT02 option),
  keeping luminance; (3) fit with the exact linear constraint M·D65 = D65 (Lagrange
  least squares, then LM in the constraint's 6-D null space).
- Why both: a hard constraint alone forces neutrals to D65 while still matching the
  LUT's white-shifted colours, which bends every hue toward the old white; a CAT alone
  removes the cast perceptually but leaves a small residual neutral error from the fit.
  CAT + constraint gives colours "as if the LUT were white-balanced to D65" and an exact
  D65 neutral.
- Note: if the LUT's white shift is a scaling in display RGB (e.g. P3 rendered on a DCI
  projector), Bradford doesn't undo it exactly; the result is the Bradford-adapted look,
  not the identity. Tests assert exactly that.

## D16 — Neutral handling in preserve mode
- Preserve mode is an unconstrained fit, as specified. With 1 neutral among 145
  samples the unconstrained fit tinted neutrals by ~1.8 ΔE2000 on ARRI's Rec.709 LUT,
  so neutral samples are weighted (default: one neutral = one whole hue ring).
- Option `lock_neutral`: constrain M·D65 = the LUT's measured white exactly. On ARRI's
  LUTs this costs almost nothing in mean error (1.36 locked vs 1.39 weighted) and makes
  neutral error exactly 0. Off by default to follow the brief; candidate default.

## D17 — Error metrics and reference whites
- ΔE2000 (SDR) uses CIELAB with reference white = D65 × SDR peak (100 cd/m²);
  ΔE ITP (PQ) uses absolute cd/m² via BT.2100 ICtCp and BT.2124. Errors are measured in
  display units: prediction = M·x / k vs the (adapted, in D65 mode) LUT output.
- Least squares runs in XYZ (fast, closed form) and seeds a Levenberg–Marquardt
  refinement minimising Σ ΔE² (RMS). The LS-only result is kept for comparison.

## D18 — Measuring any transform; test tolerances
- Measurement works on a `Transform` trait (LUT files, or exact analytic transforms).
- Synthetic tests check exact transforms to 1e-7 per matrix element; 65³ grid LUTs
  are checked against a tolerance derived from that LUT's own measured interpolation
  error (≤ 3× grid ΔE + 0.05 mean / 0.1 max), because the perceptual fit follows the
  LUT as interpolated — which is what Resolve applies.

## D19 — "Poor fit" thresholds
- Warning when mean error > 3 ΔE2000 (SDR) or > 15 ΔE ITP (HDR). On ARRI's PQ LUTs the
  same refined matrix scores mean 1.64 ΔE2000 but 7.6 ΔE ITP: ITP is ~5× more sensitive
  around 13–26 cd/m² than ΔE2000 with a 203 cd/m² reference white, so the HDR threshold
  is scaled accordingly. `--metric de2000` gives a like-for-like comparison.

## D20 — D65 is the default white mode
- User decision (2026-10-16): D65 white mode is the default; "preserve LUT white" is the
  option. The matrix then never introduces a neutral cast in DWG; a LUT's white cast
  (e.g. P3-DCI white, a warm/cool look) is removed by the CAT while its colour
  relationships are kept.
- `lock_neutral` stays off by default; it only affects preserve mode.

## D21 — DCTL portability and precision
- Matrices are separate `__DEVICE__ Mat3x3 DWG_TO_XYZ() / M_XYZ() / XYZ_TO_DWG()`
  functions returning a `typedef struct` of three `float3` rows, multiplied by an explicit
  per-component `mat_mul`. No `__CONSTANT__` arrays (Metal needs `__CONSTANTREF__` to pass
  them, a common source of backend-specific breakage) and no float3 operators (not
  available on every backend).
- Literals: 10 decimals with an `f` suffix (float32 resolves ~7.2 significant digits).
- The file is ASCII-only, including comments.
- Blend: XYZ' = XYZ + (M·XYZ − XYZ)·amount in linear XYZ, slider 0–1, default 1.
- Verification without Resolve: (1) a float32 Rust emulation reads the coefficients back
  out of the generated text and matches the f64 pipeline to ~1e-6 (relative, DI code);
  (2) the generated file is compiled with `-Wall -Werror` as C99 and C++14 against a
  shim for the DCTL built-ins and executed, and its output checked the same way (C99 ≈
  OpenCL C, C++ ≈ CUDA / Metal Shading Language); (3) a golden file catches unintended
  changes. Final confirmation in Resolve (CUDA, OpenCL, Metal) is by the user.
- Header date is UTC.

## D22 — Validation in Resolve (2026-10-16) and the double-counting problem
- User test: synthetic ColorChecker in LogC3 EI800, Resolve on macOS (Metal). K1S1 and
  Ulook4b LUTs vs our D65 DCTLs followed by a Resolve CST (with tone mapping) or by
  OpenDRT v1.1.0 (Standard look, ACES-2.0 tonescale, Rec.1886/709, 100 nit, grey 10 nit).
- Implementation verified: our tetrahedral LUT evaluation, our compiled build of the user's
  OpenDRT DCTL, and our prediction of "DCTL → OpenDRT" all match the Resolve exports to
  ≈0.15 ΔE2000 mean (16-bit export noise level). The DCTL runs as designed.
- Finding: fitting M against plain colorimetry (scene XYZ → LUT display XYZ) captures
  the LUT's whole rendering, including saturation its tone curve adds. A downstream DRT
  then adds its own, so the result can be *further* from the LUT than the DRT alone
  (K1S1 + OpenDRT, colour patches, exposure matched: 2.83 ΔE00 with matrix vs 2.13 without).
- Fitting M **through** the downstream DRT — minimise ΔE(DRT(M·x), LUT(x)), exposure
  matched per level, D65 neutral locked, hue ring at −1/0/+1 stops — fixes this on the
  held-out chart: K1S1 1.76 (vs 2.13 without matrix), Ulook4b 4.61 (vs 5.44).
- Confirmed in Resolve (v2 exports, same day): the through-OpenDRT DCTLs render within
  0.16 ΔE00 of our prediction; colour patches vs the LUT (exposure matched) improved
  from 2.13 → 1.79 (K1S1) and 5.44 → 4.59 (Ulook4b) against OpenDRT alone.
- Direction (pending user confirmation): add a "reference display transform" input —
  a LUT the user exports from Resolve (DWG/DI → display) for whatever CST/DRT follows the
  DCTL — and fit through it. Ulook4b's split tone (warm highlights, cool shadows) and its
  yellow handling remain out of reach of any 3×3 matrix.

## D23 — Fit through a reference display transform (user decision 2026-10-16)
- `--through <LUT>` (CLI) / `Reference` (core): a LUT of the node that follows the DCTL
  (DWG/DI in → display out by default; generated in Resolve from a CST, OpenDRT, …).
  The fit minimises ΔE(reference(M·x) × k_level, LUT(x)), where k_level matches the
  reference's gray to the LUT's gray at each bracket level (exposure belongs to the grade,
  not the matrix). D65 mode keeps M·D65 = D65; preserve mode is free, then scaled to unity
  neutral luminance gain (the scale is otherwise undetermined under exposure matching).
- Bracket defaults to ±1 stop in this mode (0 is treated as 1): the LUT's and the DRT's
  tone curves differ with exposure, so one exposure under-constrains M.
- Start: the better of identity (DRT alone) and the colorimetric LS matrix. Two passes:
  smooth component residuals (720·(ΔI, ½ΔCt, ΔCp) for ITP — exactly ΔE ITP — or ΔL*a*b*
  for SDR), then a polish on the scalar metric (ΔE2000's kink at zero stalls
  Gauss–Newton from the start; seen as a 1.6e-4 recovery error, fixed to < 1e-5).
- Report: error of the reference alone, of the colorimetric matrix through it, of the
  fitted matrix, the % gain, and the reference/LUT gray offset. Warning if gain < 5 %.
- The user's Resolve exports of OpenDRT and the CST as 65³ LUTs match our models to
  7e-5 (OpenDRT) and reproduce the chart exports to 0.17 ΔE00 (CST).
- Analysis JSON now writes matrices row-major (`[[r0],[r1],[r2]]`); nalgebra's default is
  a flat column-major list, which a reader transposed during this work.

## D24 — OpenDRT "LUT Match" presets (user request 2026-10-16)
- Separate from the matrix app: OpenDRT's full look parameter set was fitted directly to
  each LUT (tone and colour, across exposure), via a stickshift build of the user's
  OpenDRT v1.1.0 DCTL compiled as C++. Tooling in `tools/opendrt_presets/`; OpenDRT
  itself (GPLv3) is not in this repo.
- Held-out ColorChecker (user's Resolve exports), mean ΔE00 vs the LUT, no exposure
  matching: K1S1 2.40 (Standard/ACES-2.0) → 0.65; Ulook4b 10.62 → 2.92. Far closer than a
  3×3 matrix can get, because the tonescale and hue/purity behaviour are fitted too.
- Delivered as a modified DCTL with look presets 9–13 and a per-preset grey scale
  (`lg_scale`, so Display Grey Luminance 10 = the fitted grey); original presets are
  bit-identical. The preset grey (lg_scale) applies only with Tonescale Preset = USE LOOK
  PRESET (user decision): any stock tonescale preset gives every look the same exposure,
  set by Display Grey Luminance, exactly as for the stock looks.

## D25 — 5207 pull 2 / 2393 preset from a linear-ACES LUT
- `ACES2065_to_5207_pull2_K2393_D65_709G24.cube` (Resolve-generated, 65³, no shaper) takes
  **linear ACES2065-1** in 0–1, i.e. only up to +2.5 stops over 18% gray (output there
  ≈ 58 nits); brighter footage is clipped by the LUT itself. Identified from the gray
  axis: linear gives 18% → 9.5 nits; any log reading gives 18–32 nits.
- Fit input "aces2065": scene XYZ (D65) → Bradford to ACES white → AP0, linear; only
  samples inside the LUT domain are used (469 of 531).
- With all parameters free the fit (chart 2.08 ΔE00) left brilliance unconstrained in the
  unsampled highlights: a 0.35 % luminance reversal on saturated highlights in exposure
  sweeps. Freezing brilliance at the start preset (`ODRT_FIT_FREEZE=_brl`) removes it
  (chart 2.31 ΔE00, vs 4.99 for Standard/ACES-2.0); that version ships. Exposure sweeps
  (−4…+10 stops, saturated hues) are now part of the preset safety check.
- A LUT exported with a log input (e.g. from an ACEScct node, or with a shaper) would let
  the highlight roll-off be fitted too.

## D26 — GPLv3 and an in-app OpenDRT port (user decision 2026-10-16)
- To fit OpenDRT presets inside the app, OpenDRT v1.1.0 (Jed Smith, GPLv3) is ported to
  Rust (`crates/lutmatrix-opendrt`). Distributing that makes the app a derivative work, so
  the whole workspace is now GPL-3.0-or-later (OpenDRT's own code stays GPLv3). The
  alternatives — compiling the user's DCTL at runtime (needs a C++ toolchain on every
  machine) or a separate companion tool — were declined.

## D27 — How OpenDRT is ported
- **Which OpenDRT.** The port follows the DCTL the user runs in Resolve,
  `OpenDRT_v1.1.0.1.dctl` (bundled unmodified in `crates/lutmatrix-opendrt/opendrt/`). It is
  upstream commit `af68332` (2026-02-09) with three local changes: an extra "Standard 2"
  look, a different "DaGrinchi Tonegroan" tonescale, and surround compensation off (dark)
  for every display encoding preset (upstream uses dim for Rec.1886, bright for sRGB). We
  match what the user sees, not upstream.
- **Exactness over speed.** The transform is ported operation for operation in `f32`:
  same evaluation order, no FMA, the same library calls (`powf(2, x)` stays `powf`,
  `_exp10f` is `powf(10, x)` as in our C++ shim), literals copied digit for digit. Presets,
  matrices, ranges and labels are generated from the DCTL (`tools/gen_opendrt_tables.py`)
  and re-checked against it by a test. Result: bit-identical to the DCTL compiled as C++
  on Linux for 479 configurations (every look × tonescale, creative whites, display
  encodings, inputs, HDR sliders, 150 random stickshift sets). The C++ build matched
  Resolve exports to ~0.15 ΔE00 earlier (D22), so the port inherits that validation.
- **Cross-platform check.** The compile-and-compare test needs a C++ compiler (Linux and
  macOS CI have one); a committed fixture of compiled-DCTL outputs (tolerance 1e-4
  relative, for libm differences) checks Windows too.
- **Not ported:** the tonescale overlay (`crv_enable`), which draws a curve on screen.
- `Config::grey_scale` models the `lg_scale` that the LUT Match presets add (D24); it is 1
  for stock OpenDRT.
