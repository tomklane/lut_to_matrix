# LUT Matrix

Extract the colour transform of a camera-log → display LUT as a 3x3 matrix in linear
CIE XYZ, and export it as a DaVinci Resolve DCTL (DaVinci Wide Gamut / DaVinci
Intermediate in and out).

It also fits OpenDRT's look parameters to a LUT and keeps the results as presets on your
computer.

Version 0.1.0 · macOS 11+ (Apple Silicon and Intel) and Windows 10/11 (x64).

## Install

Download the latest release from the repository's
[Releases](https://github.com/tomklane/lut_to_matrix/releases) page:

| | File |
|---|---|
| macOS | `LUT-Matrix_<version>_macOS_universal.dmg`: open it and drag *LUT Matrix* to *Applications* |
| Windows | `LUT-Matrix_<version>_Windows_x64-setup.exe`: installs for your user account, no admin rights needed |
| Windows (machine-wide) | `LUT-Matrix_<version>_Windows_x64.msi`: installs to Program Files for all users (needs admin) |
| Command line | `lutmatrix-cli_<version>_macOS_universal.tar.gz` / `…_Windows_x64.zip` |

`SHA256SUMS.txt` on the release lists each file's checksum.

The builds aren't signed with an Apple Developer ID or a Windows code-signing certificate,
so each system asks once before the first launch:

- **macOS 15 and later:** open the app; when macOS says it can't verify it, click *Done*.
  Open *System Settings → Privacy & Security*, scroll down, click *Open Anyway* next to
  "LUT Matrix" and confirm. **macOS 14 and earlier:** right-click the app → *Open* → *Open*.
- **Windows:** if SmartScreen says "Windows protected your PC", click *More info* → *Run
  anyway*. WebView2, which the app needs, is part of Windows 10 and 11; the installer
  downloads it if it's missing.

After that the app opens normally. Updates: install the new version over the old one;
your settings and preset library are kept (they live in your user data folder, not in
the app). To uninstall: drag the app to the Trash on macOS; *Settings → Apps* on Windows.

## The app

Drop a LUT on the window (or *Open LUT…*). The detected camera input and display output
appear with how sure the detection is; change them with the pop-up menus. The matrix,
its error statistics and before/after swatches of every sample update as you change
anything. *Export DCTL…* writes the DCTL plus .json and .txt matrix files; *Copy Matrix*
puts the 3×3 on the clipboard. *OpenDRT Match* fits OpenDRT to the LUT and saves the
result to your local preset library; *Presets* builds an OpenDRT DCTL from it. Settings
are remembered between sessions. Light and dark mode follow the system.


## Build

Requirements: Rust (stable, 1.80+), Node.js 20+. On Linux also the WebKitGTK packages
(`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev`).

```sh
cd app && npm ci && cd ..
tools/check.sh                      # type-check + build the front end, clippy, all tests
cd app && npx tauri dev             # run the app with live reload
cd app && npx tauri build           # see below
cargo build --release -p lutmatrix-cli   # the command-line tool only
```

Installers, built on the platform they're for (paths are relative to the repo root):

- **macOS:** `rustup target add aarch64-apple-darwin x86_64-apple-darwin`, then
  `cd app && npx tauri build --target universal-apple-darwin --bundles app,dmg` →
  `target/universal-apple-darwin/release/bundle/dmg/LUT Matrix_<version>_universal.dmg`.
  Drop `--target …` for a build for your Mac's own architecture only. The app is ad-hoc
  signed (`signingIdentity: "-"` in `app/src-tauri/tauri.conf.json`); Xcode's command line
  tools (`xcode-select --install`) provide `codesign`.
- **Windows:** `cd app && npx tauri build --bundles nsis,msi` →
  `target\release\bundle\nsis\LUT Matrix_<version>_x64-setup.exe` and
  `target\release\bundle\msi\LUT Matrix_<version>_x64_en-US.msi`. Needs the Visual Studio
  C++ build tools; Tauri downloads NSIS and WiX itself.

## Releasing

The version lives only in the root `Cargo.toml` (`[workspace.package] version`).

1. Set the new version in `Cargo.toml`, run `cargo check` (updates `Cargo.lock`), and in
   `CHANGELOG.md` rename *Unreleased* to `## [<version>] - <date>` above a fresh, empty
   *Unreleased*. Run `tools/check.sh`, commit and push to `main`.
2. On GitHub: *Actions → Release → Run workflow*, tick *draft release*. (Or push a tag
   `v<version>`; it must match `Cargo.toml`.) The workflow runs the full test suite on
   macOS, Windows and Linux; builds the universal .dmg, the NSIS and MSI installers and
   the command-line tools; opens the .dmg and checks the app's version, architectures
   and signature; installs and uninstalls both Windows installers; and attaches
   everything with `SHA256SUMS.txt` to a **draft** release, with notes taken from the
   changelog.
3. Download and try the files from the draft (*Releases* page). Then *Edit → Publish
   release*, which creates the `v<version>` tag. Nothing is public before that.

Without the tick, the workflow only builds and checks, leaving the files as workflow
artifacts: use that for test builds. A version with a pre-release part (`0.2.0-beta.1`) is
marked as a pre-release and gets no MSI (WiX accepts only numeric pre-release versions).

`npm run dev` in `app/` runs the interface in a browser against a development mock of the
backend (fixtures in `app/dev-fixtures/`, not committed).

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

### Match OpenDRT to a LUT

```sh
lutmatrix opendrt fit --name "LUT Match K1S1" "K1S1.cube"   # fit and save to your preset library
lutmatrix opendrt fit --input arri_log_c3_awg3 --freeze brl "x.cube"
lutmatrix opendrt fit --neutral-gray every-level "print.cube"   # remove the LUT's gray cast first
lutmatrix opendrt fit --neutral-gray white-gain --white-stops 3 "look.cube"   # balance its white with a gain after the LUT
lutmatrix opendrt presets list                               # also: show, remove, import, export, path
lutmatrix opendrt build -o OpenDRT_LUTmatch.dctl            # OpenDRT + every preset in your library
lutmatrix opendrt build --preset "LUT Match K1S1" -o K1S1.dctl
```

### Batch: every negative × every print, plus full LUTs

```sh
lutmatrix batch luts/* --check-only          # measure and sort each LUT, name and check the pairs
lutmatrix batch --negative negs/*.cube --print prints/*.dat --full full/*.dat --neutral-gray white-gain --save
```

Each LUT is measured to tell a negative (camera log → film log), a print (film log →
display) and a full LUT (camera log → display) apart; `-n`/`-p`/`-f` set the list
yourself. Each pair is the negative LUT followed by the print LUT; full LUTs are matched
on their own. Names come from film stocks: the negative's stock number and pull/push from
its file name (or `FilmLog`), the print's from `stock-names.json` in your data folder
(`{"looks": {"DLook4": "2383v1"}, "variants": {"N": "D60"}, "flags": {"gn":
"Neutralized"}, "negatives": {"k8Film5219": "5219k8"}}`). Names that would repeat get
a, b, c….

`fit` adjusts every OpenDRT v1.1.0 look parameter and the Display Grey Luminance so that
OpenDRT (DaVinci Wide Gamut / Intermediate in, the LUT's display out) reproduces the LUT.
The report compares the stock Standard look, the best unmodified preset and the fit, and
runs safety checks (output range, luminance never falling as exposure rises, smoothness).

**Presets are yours and stay on your machine.** The app ships no LUT Match presets; each
fit is saved as a JSON file in your preset library (`lutmatrix opendrt presets path`):
`~/Library/Application Support/LUT Matrix/opendrt-presets/` on macOS,
`%APPDATA%\LUT Matrix\opendrt-presets\` on Windows (`--store DIR` or the
`LUTMATRIX_DATA_DIR` environment variable to use another folder). `presets import` /
`export` move them between machines.

`build` writes the bundled OpenDRT v1.1.0.1 with the stock presets unchanged and your
presets appended to the Look Preset menu. Each LUT Match preset carries its own
middle-gray level, used only with *Tonescale Preset = USE LOOK PRESET*; with any stock
tonescale every look has the exposure of the *Display Grey Luminance* slider. It also adds
a *DCI - 2.6 Power / P3-D65* Display Encoding Preset (P3-D65 primaries and white, 2.6
power, dark surround) at the end of that menu. Install it like any DCTL; use DaVinci
Wide Gamut / Intermediate in.

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
`fuji_f_log_f_gamut`, `fuji_f_log2_f_gamut`, `da_vinci_intermediate_dwg`, `aces_cct_ap1`,
`aces2065_ap0` (linear ACES2065-1).

## Tests

```sh
cargo test --workspace
# also parse a folder of your own LUTs (they are never committed):
LUTMATRIX_SAMPLE_LUTS="/path/to/luts" cargo test --workspace
```

Reference values come from colour-science; regenerate with
`python3 tools/gen_fixtures.py` (requires `pip install colour-science`).

## License

GPL-3.0-or-later (see `LICENSE` and `NOTICE`). The app includes a Rust port of
[OpenDRT](https://github.com/jedypod/open-display-transform) by Jed Smith (GPLv3).

