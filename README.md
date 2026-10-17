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

### Fit through your display transform (recommended)

If a display transform follows the DCTL in your grade (a CST to Rec.709, OpenDRT, ACES…),
fit **through** it, otherwise the DCTL and the DRT both add rendering and the result
over-shoots. In Resolve, right-click the node(s) that follow the DCTL (on a DWG /
DaVinci Intermediate timeline) → *Generate LUT* → 65 Point Cube, then:

```sh
lutmatrix export --through "dwgdi_to_rec709G24_opendrt.cube" "My LUT.cube"
```

The report shows the error of the DRT alone vs with the matrix. Use the resulting DCTL
directly before that same transform. Options: `--through-input` (default
`da_vinci_intermediate_dwg`), `--through-gamut`, `--through-transfer` (default: the LUT's
own output).

### Export a DCTL

```sh
lutmatrix export "My LUT.cube"                 # writes My_LUT_MXYZ.dctl/.json/.txt next to the LUT
lutmatrix export -o ~/Desktop --white preserve "My LUT.cube"
```

The `.dctl` takes **DaVinci Wide Gamut / DaVinci Intermediate** in and out:
DI → linear DWG → XYZ (`DWG_TO_XYZ`) → `M_XYZ` → linear DWG (`XYZ_TO_DWG`) → DI.
A **Matrix Amount** slider (0–1) blends between no change and the full matrix, in XYZ.
The `.json` and `.txt` files hold the same matrices (plus the least-squares matrix and
the equivalent single linear-DWG matrix) for other tools.

**Install in Resolve:** Project Settings → Color Management → Lookup Tables →
*Open LUT Folder*, copy the `.dctl` there, then *Update Lists*. Apply it with the
**DCTL** ResolveFX (or as a node LUT) on a node where the image is DWG / DaVinci
Intermediate, before your output transform. Works with CUDA, OpenCL and Metal.

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
