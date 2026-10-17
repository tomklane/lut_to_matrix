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
