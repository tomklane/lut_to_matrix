#!/usr/bin/env python3
"""Generate reference values for lutmatrix-core tests using colour-science.

Run once (or whenever a curve/space is added) and commit the output:

    pip install colour-science==0.4.7
    python3 tools/gen_fixtures.py

Writes crates/lutmatrix-core/tests/fixtures/reference.json.
"""

import json
import pathlib

import colour
import numpy as np
from colour.models import (
    log_encoding_ACEScct,
    log_encoding_ARRILogC3,
    log_encoding_ARRILogC4,
    log_encoding_CanonLog2,
    log_encoding_CanonLog3,
    log_encoding_FLog,
    log_encoding_FLog2,
    log_encoding_Log3G10,
    log_encoding_SLog3,
    log_encoding_VLog,
    oetf_BlackmagicFilmGeneration5,
    oetf_DaVinciIntermediate,
)

OUT = pathlib.Path(__file__).resolve().parent.parent / "crates/lutmatrix-core/tests/fixtures/reference.json"

LIN = [-0.005, 0.0, 0.0005, 0.002, 0.01, 0.05, 0.18, 0.5, 1.0, 2.0, 8.0, 30.0]

CURVES = {
    "arri_logc4": log_encoding_ARRILogC4,
    "slog3": log_encoding_SLog3,
    "log3g10": lambda x: log_encoding_Log3G10(x, method="v3"),
    "canon_log2": lambda x: log_encoding_CanonLog2(x, method="v1.2"),
    "canon_log3": lambda x: log_encoding_CanonLog3(x, method="v1.2"),
    "vlog": log_encoding_VLog,
    "bmd_film_gen5": oetf_BlackmagicFilmGeneration5,
    "flog": log_encoding_FLog,
    "flog2": log_encoding_FLog2,
    "davinci_intermediate": oetf_DaVinciIntermediate,
    "acescct": log_encoding_ACEScct,
}
for ei in [160, 200, 250, 320, 400, 500, 640, 800, 1000, 1280, 1600]:
    CURVES[f"arri_logc3_ei{ei}"] = (lambda e: lambda x: log_encoding_ARRILogC3(x, EI=e))(ei)

GAMUTS = {
    "awg3": "ARRI Wide Gamut 3",
    "awg4": "ARRI Wide Gamut 4",
    "s_gamut3": "S-Gamut3",
    "s_gamut3_cine": "S-Gamut3.Cine",
    "rwg": "REDWideGamutRGB",
    "cinema_gamut": "Cinema Gamut",
    "v_gamut": "V-Gamut",
    "bmd_wg_gen5": "Blackmagic Wide Gamut",
    "f_gamut": "F-Gamut",
    "dwg": "DaVinci Wide Gamut",
    "ap1": "ACEScct",
    "ap0": "ACES2065-1",
    "rec709": "ITU-R BT.709",
    "p3_d65": "P3-D65",
    "p3_dci": "DCI-P3",
    "rec2020": "ITU-R BT.2020",
}


def main():
    out = {"colour_version": colour.__version__, "curves": {}, "npm": {}, "published_npm": {}}

    for name, f in CURVES.items():
        out["curves"][name] = [[x, float(f(x))] for x in LIN]

    for key, cs_name in GAMUTS.items():
        cs = colour.RGB_COLOURSPACES[cs_name]
        npm = colour.normalised_primary_matrix(cs.primaries, cs.whitepoint)
        out["npm"][key] = np.asarray(npm).tolist()
        # colour-science stores the manufacturer-published (rounded) matrix
        # where one exists; record it so tests can compare against it too.
        out["published_npm"][key] = np.asarray(cs.matrix_RGB_to_XYZ).tolist()

    # DaVinci Wide Gamut Intermediate v1.1 published matrices (8 decimals).
    out["dwg_doc"] = {
        "rgb_to_xyz": [
            [0.70062239, 0.14877482, 0.10105872],
            [0.27411851, 0.87363190, -0.14775041],
            [-0.09896291, -0.13789533, 1.32591599],
        ],
        "xyz_to_rgb": [
            [1.51667204, -0.28147805, -0.14696363],
            [-0.46491710, 1.25142378, 0.17488461],
            [0.06484905, 0.10913934, 0.76141462],
        ],
    }

    aces = colour.CCS_ILLUMINANTS["CIE 1931 2 Degree Standard Observer"]["ACES"]
    d65 = np.array([0.3127, 0.3290])
    dci = np.array([0.314, 0.351])
    xyz = colour.xy_to_XYZ
    out["cat"] = {
        "bradford_aces_to_d65": colour.adaptation.matrix_chromatic_adaptation_VonKries(
            xyz(aces), xyz(d65), "Bradford").tolist(),
        "cat02_aces_to_d65": colour.adaptation.matrix_chromatic_adaptation_VonKries(
            xyz(aces), xyz(d65), "CAT02").tolist(),
        "bradford_dci_to_d65": colour.adaptation.matrix_chromatic_adaptation_VonKries(
            xyz(dci), xyz(d65), "Bradford").tolist(),
    }

    nits = [0.0, 0.1, 1.0, 26.0, 100.0, 203.0, 1000.0, 4000.0, 10000.0]
    out["pq"] = [[n, float(colour.models.eotf_inverse_ST2084(n))] for n in nits]

    # Colour differences on pseudo-random pairs (fixed seed).
    rng = np.random.default_rng(20261016)
    d65_100 = colour.xy_to_XYZ(d65) * 100.0
    de = []
    for _ in range(40):
        a = rng.uniform([1, 0.5, 0.5], [90, 95, 100])
        b = a * rng.uniform(0.85, 1.15, 3)
        la = colour.XYZ_to_Lab(a / 100.0, d65)
        lb = colour.XYZ_to_Lab(b / 100.0, d65)
        de.append({"xyz1": a.tolist(), "xyz2": b.tolist(), "white": d65_100.tolist(),
                   "lab1": la.tolist(), "de2000": float(colour.delta_E(la, lb, method="CIE 2000"))})
    out["delta_e_2000"] = de

    rec2020 = colour.RGB_COLOURSPACES["ITU-R BT.2020"]
    itp = []
    for _ in range(40):
        a = rng.uniform(0.5, 800.0, 3)
        b = a * rng.uniform(0.9, 1.1, 3)
        xa = rec2020.matrix_RGB_to_XYZ @ a
        xb = rec2020.matrix_RGB_to_XYZ @ b
        ia = colour.RGB_to_ICtCp(a)
        ib = colour.RGB_to_ICtCp(b)
        itp.append({"xyz1": xa.tolist(), "xyz2": xb.tolist(), "ictcp1": ia.tolist(),
                    "de_itp": float(colour.difference.delta_E_ITP(ia, ib))})
    out["delta_e_itp"] = itp

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(out, indent=1))
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
