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
