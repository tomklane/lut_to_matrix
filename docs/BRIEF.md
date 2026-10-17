# Project: LUT → 3x3 Matrix Extractor (Rust, Windows + macOS, DCTL output)

You are building a cross-platform desktop app in Rust for a colorist. It must run on
Windows and macOS. Before writing any code, read this whole brief, list your questions
and assumptions, and propose a plan, including your GUI framework choice. Then build it
in stages, showing me results after each stage.

## Goal
Read a 3D LUT that converts camera log to a display space. Measure how it transforms
color around middle-gray exposure, fit the 3x3 matrix in linear CIE XYZ that best
approximates that color transform, and export it as a DaVinci Resolve DCTL that takes
DaVinci Wide Gamut / DaVinci Intermediate in, converts to linear XYZ, applies the
matrix, and converts back to DaVinci Wide Gamut / DaVinci Intermediate.

## LUT input side: camera log
The LUT expects camera log / camera gamut in. Detect it from the filename, the TITLE
line and comments in the file, and let the user confirm or override it in the UI.
Show the detected space and how confident the detection is. Support at least:
- ARRI LogC3 / ARRI Wide Gamut 3, ARRI LogC4 / ARRI Wide Gamut 4
- Sony S-Log3 / S-Gamut3 and S-Gamut3.Cine
- RED Log3G10 / REDWideGamutRGB
- Canon Log 2 and Canon Log 3 / Cinema Gamut
- Panasonic V-Log / V-Gamut
- Blackmagic Film Gen 5 / Blackmagic Wide Gamut Gen 5
- Fujifilm F-Log / F-Log2 (F-Gamut)
- DaVinci Intermediate / DaVinci Wide Gamut
- ACEScct / AP1
Use each manufacturer's published curve constants and primaries. Verify them and cite
the source in code comments. Structure the code so new camera spaces are easy to add.

## LUT output side: display space
User-selectable, with auto-guess from the filename and file contents:
- Gamut: P3-D65, P3-DCI (DCI white), Rec.709, Rec.2020
- Transfer: pure power gamma 2.2, 2.4, 2.6, or PQ (SMPTE ST 2084)
- PQ peak luminance: 100 to 10000 nits (presets: 100, 300, 600, 1000, 2000, 4000, 10000)

## LUT formats
.cube (1D and 3D) required; .3dl nice to have. Handle LUT_3D_SIZE, DOMAIN_MIN/MAX,
TITLE and comments. Apply 3D LUTs with tetrahedral interpolation (trilinear option).

## Measurement (at middle-gray exposure)
- Define test colors in scene-linear: neutral gray at 0.18 plus a ring of hues at
  several saturation levels (e.g. 24–36 hues × 3–4 saturations), all at middle-gray
  luminance. Optional small exposure bracket (e.g. ±0.5 stop) for robustness.
- Encode the samples into the LUT's camera log space, run them through the LUT, decode
  the display output to linear light, and convert both input and output to CIE XYZ.
- Skip samples outside the camera gamut, outside the LUT's input range, or that clip on
  output.
- The LUT also contains tone mapping. Normalize exposure so middle gray in maps to middle
  gray out (on by default, toggle in the UI), so the matrix captures color changes
  (hue, saturation, white) rather than the tone curve. Flag anything else about the
  tone curve that could distort the fit.
- Middle gray must be a setting, with sensible defaults for SDR gamma and for PQ
  (follow ITU-R BT.2408 for PQ; verify the reference values and cite them).

## Fitting
- Fit a 3x3 matrix M_xyz in linear XYZ, input XYZ → output XYZ. Start with least
  squares, then refine by minimizing perceptual error (ΔE2000 for SDR, ΔE ITP for HDR)
  over the samples. Keep the least-squares-only result as an option for comparison.
- Do all fitting math in double precision.
- Two white-point modes:
  1. Preserve LUT white: unconstrained fit. Any neutral shift the LUT applies (a warm
     look, or P3-DCI's white) is baked into the matrix.
  2. D65 white: match the colors, but D65 neutral input must map to D65 neutral output
     in XYZ. Use constrained fitting, or remove the LUT's white shift with chromatic
     adaptation (Bradford default, CAT02 option) before fitting. Tell me which approach
     you chose and why.
- Report fit quality: mean and max ΔE2000 (SDR) or ΔE ITP (HDR) over the samples,
  neutral-axis error, and which hues fit worst.

## Output: DCTL
The DCTL must perform these steps explicitly, in this order:
1. Decode DaVinci Intermediate → linear DaVinci Wide Gamut
2. Convert linear DWG → linear CIE XYZ (DWG→XYZ matrix)
3. Apply the fitted matrix M_xyz in linear XYZ
4. Convert linear XYZ → linear DWG (XYZ→DWG matrix)
5. Encode linear DWG → DaVinci Intermediate
Requirements:
- Keep the three matrices separate and clearly named in the DCTL (DWG_TO_XYZ, M_XYZ,
  XYZ_TO_DWG). Do not collapse them into one matrix.
- Compute the DWG↔XYZ matrices from the DWG primaries and D65 white in double precision
  in the app, and write them out with enough decimal places for float32.
- Use the exact DWG primaries and DaVinci Intermediate curve constants from Blackmagic's
  DaVinci Wide Gamut Intermediate documentation. Verify them and cite the source in
  comments.
- Valid Resolve DCTL syntax (__DEVICE__ float3 transform(...)), working in Resolve on
  Windows and macOS (CUDA, OpenCL and Metal).
- Optional UI slider in the DCTL to blend between identity and the matrix (0–1),
  applied in linear XYZ.
- Header comment in every DCTL: source LUT filename, its input and output spaces, white
  mode, middle-gray value, fit error, app version and date.
- Also export M_xyz and the conversion matrices as JSON / plain text.

## GUI: intuitive, Apple style, cross-platform
- Must run on Windows and macOS. Use Tauri v2 (Rust backend, web frontend) unless you
  have a strong reason otherwise; explain your choice before building.
- Style the interface after Apple's Human Interface Guidelines on both platforms:
  clean single window or sidebar layout, system font stack (SF Pro on macOS, Segoe UI
  Variable on Windows), segmented controls and pop-up menus, generous spacing, light
  and dark mode following the system setting, smooth but minimal animation.
- Drag and drop a LUT onto the window to load it. Show the detected input and output
  spaces right away, with clear controls to override them.
- Show: the fitted matrix, fit-error stats, before/after color swatches for the sample
  ring, and a one-click "Export DCTL" (plus "Copy matrix").
- Remember the last-used settings.
- Handle file paths correctly on both platforms.

## App structure
- Cargo workspace: a core library crate with all color math, LUT parsing, fitting and
  DCTL generation (no UI dependencies), a CLI crate, and the Tauri app.
- Use well-maintained crates (e.g. nalgebra for linear algebra). Reference values for
  tests may be generated once with Python's colour-science and committed as fixtures.
- Produce a macOS .app/.dmg and a Windows installer (.msi or NSIS .exe). Document how to
  build each, and set up GitHub Actions to build and test on both platforms.

## Correctness tests (cargo test)
- Synthetic round trip: build a LUT from a known XYZ matrix for several camera-log
  inputs and display outputs; the app must recover that matrix within a tight tolerance.
- Identity-color LUT (camera log → display with no color change) → identity matrix.
- P3-DCI white test: preserve mode keeps the DCI white shift; D65 mode removes it.
- Encode/decode round trip for every camera log curve, PQ at several peak luminances,
  and DaVinci Intermediate.
- DWG↔XYZ round trip: DWG→XYZ→DWG returns the input within float tolerance.
- DCTL pipeline test: a Rust reimplementation of the DCTL's exact steps (in float32)
  matches the app's double-precision result on the sample set.
- Golden-file test for the generated DCTL.

## Version control and change tracking
- Initialize a git repo before the first file. Make small commits, one logical change
  each, with clear messages.
- Semantic versioning, with the version defined once (Cargo.toml) and stamped into every
  exported DCTL and shown in the app's About window.
- CHANGELOG.md (Keep a Changelog format), updated with every change.
- DECISIONS.md logging color-science and architecture choices and why.
- CLAUDE.md at the repo root summarizing the project, architecture, conventions and how
  to build and test, so a future session can pick up where we left off.
- README with build, install and usage instructions.
- Run cargo test and cargo clippy before every commit; never commit with failures.

## Working style
- When a color-science choice is ambiguous, state the assumption and make it a setting
  rather than hard-coding it.
- Build in stages: (1) LUT parsing + color space and log curve math + tests,
  (2) sampling + fitting, (3) DCTL export, (4) GUI, (5) installers and CI.
  Stop and show me results after each stage.