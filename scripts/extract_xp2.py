#!/usr/bin/env python3
"""Extract Ilford XP2 Super 400 curves from the November 2018 Harman sheet.

Selected curves:
  page 2  2856 K tungsten wedge spectrogram (515 x 264 raster)
  page 3  characteristic curve, standard C-41, representative of 35mm and roll film (292 x 210 raster)

The source plots are embedded raster images. Axis ticks and labels are
separate from the plot images. The characteristic x-axis is Relative log
exposure; the spectral y-axis is printed only as "Sensitivity" with 0.5/1.0
ticks. The profile records the explicit linear, zero-based spectral reading.
"""
import argparse
import hashlib
import json
import math
import subprocess
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image

PDF_HASH = "695a2d3d54f11cb8b24f6d040a9723a16819001a38715f2bd6f71f2d05cdc549"
STOCK = "ilford_xp2_super_400"
PRIMARY_URL = "https://125px.com/docs/film/ilford/XP2-Super_201811.pdf"

# Embedded image sizes from pdfimages -list.
P2_W, P2_H = 515, 264
P3_W, P3_H = 292, 210

# Page-2 spectral frame/ticks. Same 515x264 plot geometry as the HP5 sheet.
P2_X_PX = np.array([72, 129, 187, 244, 301, 359], dtype=float)
P2_X_NM = np.array([400, 450, 500, 550, 600, 650], dtype=float)
P2_Y_PX = np.array([186, 129, 72], dtype=float)
P2_Y_SENS = np.array([0.0, 0.5, 1.0], dtype=float)

# Page-3 characteristic frame: 10 vertical grid lines, 7 horizontal grid lines.
# The printed x labels are 1, 2, 3, 4 at columns 35, 90, 145, 199.
P3_X_PX = np.array([35, 90, 145, 199], dtype=float)
P3_X_VAL = np.array([1, 2, 3, 4], dtype=float)
P3_Y_PX = np.array([168, 114, 60], dtype=float)
P3_Y_VAL = np.array([0.0, 1.0, 2.0], dtype=float)
P3_GRID_ROWS = np.array([5, 33, 60, 87, 114, 141, 168], dtype=float)
P3_GRID_COLS = np.array([8, 35, 63, 90, 117, 145, 172, 199, 227, 253], dtype=float)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def image_from_pdf(pdf, page, width, height, tmp):
    subprocess.run(
        ["pdfimages", "-f", str(page), "-l", str(page), "-png", str(pdf), str(tmp / f"p{page}")],
        check=True,
        capture_output=True,
    )
    candidates = sorted(tmp.glob(f"p{page}-*.png"))
    for path in candidates:
        with Image.open(path) as image:
            if image.size == (width, height):
                return np.asarray(image.convert("L"), dtype=np.uint8), path.name
    raise ValueError(f"page {page}: no embedded image with size {width}x{height}: {candidates}")


def largest_cluster(values):
    if len(values) == 0:
        return None
    groups = []
    start = end = int(values[0])
    for value in values[1:]:
        value = int(value)
        if value <= end + 2:
            end = value
        else:
            groups.append((start, end))
            start = end = value
    groups.append((start, end))
    groups.sort(key=lambda group: group[1] - group[0], reverse=True)
    return 0.5 * (groups[0][0] + groups[0][1])


def extract_spectral(pdf, tmp):
    image, name = image_from_pdf(pdf, 2, P2_W, P2_H, tmp)
    mask = image < 100
    mask[:12, :] = False
    mask[186:, :] = False
    mask[:, :14] = False
    mask[:, 417:] = False
    points = []
    for x in range(14, 417):
        y = largest_cluster(np.where(mask[12:186, x])[0] + 12)
        if y is not None:
            points.append((x, y))
    if len(points) < 100:
        raise ValueError("spectral curve too sparse")
    px = np.array([p[0] for p in points], dtype=float)
    py = np.array([p[1] for p in points], dtype=float)
    x_fit = np.polyfit(P2_X_PX, P2_X_NM, 1)
    y_fit = np.polyfit(P2_Y_PX, P2_Y_SENS, 1)
    nm = np.polyval(x_fit, px)
    sensitivity = np.polyval(y_fit, py)
    order = np.argsort(nm)
    nm, sensitivity = nm[order], sensitivity[order]
    lo, hi = float(nm[0]), float(nm[-1])
    anchors_nm = np.arange(math.ceil(lo / 5) * 5, math.floor(hi / 5) * 5 + 1, 5)
    anchors_s = np.interp(anchors_nm, nm, sensitivity)
    anchors = [[float(w), float(v)] for w, v in zip(anchors_nm, anchors_s)]
    anchors.append([float(nm[-1]), float(sensitivity[-1])])
    return {
        "image": name,
        "image_sha256": digest(image.tobytes()),
        "x_fit": {"slope": float(x_fit[0]), "intercept": float(x_fit[1])},
        "y_fit": {"slope": float(y_fit[0]), "intercept": float(y_fit[1])},
        "source_domain": [lo, hi],
        "points": anchors,
    }


def extract_characteristic(pdf, tmp):
    image, name = image_from_pdf(pdf, 3, P3_W, P3_H, tmp)
    mask = image < 100
    for y in P3_GRID_ROWS:
        y = int(y)
        mask[max(0, y - 1) : y + 2, :] = False
    for x in P3_GRID_COLS:
        x = int(x)
        mask[:, max(0, x - 1) : x + 2] = False
    points = []
    for x in range(9, 253):
        y = largest_cluster(np.where(mask[6:168, x])[0] + 6)
        if y is not None:
            points.append((x, y))
    if len(points) < 100:
        raise ValueError("characteristic curve too sparse")
    px = np.array([p[0] for p in points], dtype=float)
    py = np.array([p[1] for p in points], dtype=float)
    x_fit = np.polyfit(P3_X_PX, P3_X_VAL, 1)
    y_fit = np.polyfit(P3_Y_PX, P3_Y_VAL, 1)
    log_e = np.polyval(x_fit, px)
    density = np.polyval(y_fit, py)
    order = np.argsort(log_e)
    log_e, density = log_e[order], density[order]
    lo, hi = float(log_e[0]), float(log_e[-1])
    anchors_x = np.round(np.arange(math.ceil(lo * 10) / 10, math.floor(hi * 10) / 10 + 0.05, 0.1), 1)
    anchors_y = np.interp(anchors_x, log_e, density)
    anchors = [[float(log_e[0]), float(density[0])]]
    anchors += [[float(x), float(y)] for x, y in zip(anchors_x, anchors_y) if x > log_e[0] + 1e-9]
    anchors.append([float(log_e[-1]), float(density[-1])])
    while len(anchors) > 3 and anchors[-1][1] < anchors[-2][1]:
        anchors.pop()
    return {
        "image": name,
        "image_sha256": digest(image.tobytes()),
        "x_fit": {"slope": float(x_fit[0]), "intercept": float(x_fit[1])},
        "y_fit": {"slope": float(y_fit[0]), "intercept": float(y_fit[1])},
        "source_domain": [lo, hi],
        "points": anchors,
    }


def extract(pdf):
    if digest(pdf.read_bytes()) != PDF_HASH:
        raise ValueError("unexpected XP2 PDF identity")
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        characteristic = extract_characteristic(pdf, tmp)
        spectral = extract_spectral(pdf, tmp)
    geometry = digest(
        json.dumps(
            {
                "p3": [characteristic["image_sha256"], P3_GRID_ROWS.tolist(), P3_GRID_COLS.tolist(),
                       P3_X_PX.tolist(), P3_Y_PX.tolist()],
                "p2": [spectral["image_sha256"], P2_X_PX.tolist(), P2_Y_PX.tolist()],
            },
            sort_keys=True,
        ).encode()
    )
    return {
        "stock": STOCK,
        "primary_url": PRIMARY_URL,
        "primary_sha256": PDF_HASH,
        "primary_revision": "XP2 SUPER Technical Information, Nov 2018, 5 pp.",
        "primary_pages": {"spectral": 2, "characteristic": 3},
        "process": "Standard C-41 colour-negative process; curve representative of 35mm and roll film; no development time is printed.",
        "axis_mapping": {
            "characteristic": {
                "x_pixels": P3_X_PX.tolist(), "x_values": P3_X_VAL.tolist(),
                "y_pixels": P3_Y_PX.tolist(), "y_values": P3_Y_VAL.tolist(),
                "formula": f"logE={characteristic['x_fit']['slope']:.12g}*px+{characteristic['x_fit']['intercept']:.12g}; D={characteristic['y_fit']['slope']:.12g}*py+{characteristic['y_fit']['intercept']:.12g}",
            },
            "spectral": {
                "x_pixels": P2_X_PX.tolist(), "x_values": P2_X_NM.tolist(),
                "y_pixels": P2_Y_PX.tolist(), "y_values": P2_Y_SENS.tolist(),
                "formula": f"nm={spectral['x_fit']['slope']:.12g}*px+{spectral['x_fit']['intercept']:.12g}; S={spectral['y_fit']['slope']:.12g}*py+{spectral['y_fit']['intercept']:.12g}",
                "ordinate": "linear normalized wedge sensitivity, zero at frame bottom (explicit interpretation; Harman prints only 'Sensitivity' with 0.5/1.0 ticks)",
            },
        },
        "extraction": {
            "method": "embedded raster plot images; grid removal; largest dark cluster per column; axis tick polynomial fits; 0.1 logE / 5 nm anchors",
            "geometry_sha256": geometry,
            "converter_version": subprocess.run(["pdfimages", "-v"], check=True, capture_output=True, text=True).stderr.splitlines()[0],
            "images": {"characteristic": characteristic, "spectral": spectral},
        },
        "characteristic": characteristic["points"],
        "relative_sensitivity": spectral["points"],
        "limitations": [
            "Publication plots are embedded raster images, not vector curves.",
            "The spectral ordinate is printed only as 'Sensitivity' with 0.5/1.0 ticks; the linear zero-based reading is an explicit interpretation, not a Harman statement.",
            "The page-2 ordinate is a 2856 K tungsten wedge response; no equal-energy correction or source spectrum is supplied, so relative_sensitivity is not equal-energy film sensitivity.",
            "The 380-355 nm and 660-780 nm tails are endpoint-slope extrapolations, not measured.",
            "No RMS granularity or MTF figure is published on the XP2 sheet; grain must be a class-level approximation, not a measured RMS.",
            "XP2 is a chromogenic dye B&W film: the image is formed by dyes, not silver. The negative is pink/red-brown and the datasheet recommends C-41.",
            "No base spectrum, halation PSF, scanner response or absolute ISO calibration is supplied.",
            "The characteristic curve is representative of 35mm and roll film; no separate 120/sheet calibration is provided.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pdf", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    data = extract(args.pdf)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
