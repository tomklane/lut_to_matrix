# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
