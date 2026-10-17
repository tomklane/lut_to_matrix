# LUT Matrix

Extract the colour transform of a camera-log → display LUT as a 3x3 matrix in linear
CIE XYZ, and export it as a DaVinci Resolve DCTL (DaVinci Wide Gamut / DaVinci
Intermediate in and out).

> Status: in development (stage 1 of 5: LUT parsing and colour math).

## Build

Requirements: Rust (stable, 1.80+).

```sh
cargo build --release
cargo test --workspace
```

## Usage (CLI)

```sh
cargo run --release -p lutmatrix-cli -- inspect "path/to/My LUT.cube" [more.cube ...]
cargo run --release -p lutmatrix-cli -- inspect --json "path/to/My LUT.cube"
```

`inspect` reports the LUT format, the detected camera input and display output
(with confidence and reasons), and where 18% gray lands on the display.

```sh
lutmatrix fit "My LUT.cube"                       # detected spaces, D65 white (default)
lutmatrix fit --white preserve "My LUT.cube"      # keep the LUT's white cast in the matrix
lutmatrix fit --white preserve --lock-neutral "My LUT.cube"  # …with gray exactly the LUT's white
lutmatrix fit --input arri_log_c3_awg3 --ei 1280 --gamut rec709 --transfer 2.4 "x.cube"
lutmatrix fit --anchor display --mid-gray-nits 12.8 --bracket 0.5 "x.cube"
lutmatrix fit --json "My LUT.cube"                # full analysis incl. per-sample swatches
```

`fit` prints the least-squares and perceptually refined matrices (linear XYZ, D65),
mean/RMS/p95/max error (ΔE2000 for SDR, ΔE ITP for PQ), neutral-axis error, the
worst hues, and warnings about the LUT's tone curve.

Input ids: `arri_log_c3_awg3`, `arri_log_c4_awg4`, `sony_s_log3_s_gamut3`,
`sony_s_log3_s_gamut3_cine`, `red_log3_g10_rwg`, `canon_log2_cinema_gamut`,
`canon_log3_cinema_gamut`, `panasonic_v_log_v_gamut`, `bmd_film_gen5`,
`fuji_f_log_f_gamut`, `fuji_f_log2_f_gamut`, `da_vinci_intermediate_dwg`, `aces_cct_ap1`.

## Tests

```sh
cargo test --workspace
# also parse a folder of your own LUTs (they are never committed):
LUTMATRIX_SAMPLE_LUTS="/path/to/luts" cargo test --workspace
```

Reference values come from colour-science; regenerate with
`python3 tools/gen_fixtures.py` (requires `pip install colour-science`).

Installer, app and DCTL usage instructions arrive with later stages.
