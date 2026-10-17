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

## Tests

```sh
cargo test --workspace
# also parse a folder of your own LUTs (they are never committed):
LUTMATRIX_SAMPLE_LUTS="/path/to/luts" cargo test --workspace
```

Reference values come from colour-science; regenerate with
`python3 tools/gen_fixtures.py` (requires `pip install colour-science`).

Installer, app and DCTL usage instructions arrive with later stages.
