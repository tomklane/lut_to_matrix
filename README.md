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
cargo run -p lutmatrix-cli -- inspect "path/to/My LUT.cube"
```

Installer, app and DCTL usage instructions arrive with later stages.
