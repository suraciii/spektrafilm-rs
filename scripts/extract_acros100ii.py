#!/usr/bin/env python3
"""Extract Acros II 135 p3 curves from the pinned raster publication.

The primary page is a vector page containing 300 dpi raster plot images. This
extractor identifies the two relevant images by size and hash, traces the
Microfine 10-minute curve and the relative log sensitivity curve, and records
the pixel-to-axis mapping. It does not invent data outside the plotted line.
"""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image
from scipy.interpolate import PchipInterpolator

PDF_HASH = 'afb5e24db0d9b7a219b6682b640237a525b7927e8cf1b95ba2299c0be6385cdd'
PRIMARY_URL = 'https://asset.fujifilm.com/www/us/files/2020-08/73080443d0289b1a7b116add14a9bdfc/films_neopan100acros2_135_01_0.pdf'
PRIMARY_REVISION = 'AF3-0258E (20.02-BX), February 2020'
PRIMARY_PAGE = 3
CHARACTERISTIC_IMAGE_SHA256 = 'ba0e8db7dec9bd4a2d5f6bb33f5815791b4edf61faf1d635941dff382cf400de'
SPECTRAL_IMAGE_SHA256 = 'a7705ce8bf7465aac546181ff5fab3b51329483c0bd034b4ba34b55fe7470029'
CHARACTERISTIC_SIZE = (913, 652)
SPECTRAL_SIZE = (808, 769)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def image_sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def runs(mask, x, lo, hi):
    """Return dark run centers in one column, ignoring one-pixel specks."""
    ys = np.where(mask[:, x])[0]
    ys = ys[(ys >= lo) & (ys <= hi)]
    out = []
    if len(ys) == 0:
        return out
    start = previous = ys[0]
    for y in ys[1:]:
        if y > previous + 1:
            if previous - start + 1 >= 2:
                out.append((start + previous) / 2)
            start = y
        previous = y
    if previous - start + 1 >= 2:
        out.append((start + previous) / 2)
    return out


def clean_grid(mask, x_grid, y_grid):
    for x in x_grid:
        for delta in range(-2, 3):
            if 0 <= x + delta < mask.shape[1]:
                mask[:, x + delta] = False
    for y in y_grid:
        for delta in range(-2, 3):
            if 0 <= y + delta < mask.shape[0]:
                mask[y + delta, :] = False


def median_smooth(values, width=5):
    values = values.copy()
    for index in range(width, len(values) - width):
        values[index] = np.median(values[index - width:index + width + 1])
    return values


def trace_characteristic(image_path):
    image = Image.open(image_path).convert('L')
    pixels = np.asarray(image)
    mask = pixels < 120
    x_grid = [107, 183, 259, 335, 411, 487, 561, 636, 712, 788, 864]
    y_grid = [100, 176, 252, 328, 404, 480, 556]
    clean_grid(mask, x_grid, y_grid)
    # Ten-minute curve anchors. At x=400..700 four curves are separated, so
    # the second-largest y is the second-lowest density (10 min). The lower
    # shoulder and the labelled right end are explicit anchors selected from
    # the unlabelled line positions; labels are never used as curve pixels.
    anchors = [(300, None), (350, None), (400, None), (450, None),
               (500, None), (550, None), (600, None), (650, None),
               (700, None), (740, 334.5), (780, 311.0), (800, 300.0), (805, 299.5)]
    points = []
    for x, fixed in anchors:
        candidates = runs(mask, x, 100, 556)
        if fixed is not None:
            y = fixed
        elif len(candidates) >= 4:
            y = sorted(candidates)[-2]
        elif candidates:
            y = sorted(candidates)[-1]
        else:
            raise ValueError(f'missing characteristic anchor at {x}')
        points.append((x, y))
    xs = np.array([point[0] for point in points], dtype=float)
    ys = np.array([point[1] for point in points], dtype=float)
    exposure = (xs - 712.2) * 4.0 / 605.0
    density = (556.0 - ys) / 152.0
    require(np.all(np.diff(exposure) > 0), 'characteristic exposure is not increasing')
    require(np.all(np.diff(density) >= 0), 'characteristic density is not monotone')
    return exposure, density, points


def trace_sensitivity(image_path):
    image = Image.open(image_path).convert('L')
    pixels = np.asarray(image)
    mask = pixels < 120
    x_grid = [89, 129, 341, 552, 764, 805]
    y_grid = [62, 269, 476, 682]
    clean_grid(mask, x_grid, y_grid)
    points = {}
    y = None
    for x in range(132, 686):
        candidates = runs(mask, x, 62, 685)
        if not candidates:
            continue
        if y is None:
            y = min(candidates, key=lambda value: abs(value - 410))
        else:
            y = min(candidates, key=lambda value: abs(value - y))
        points[x] = y
    require(len(points) > 500, 'spectral curve trace missing')
    xs = np.array(sorted(points))
    ys = median_smooth(np.array([points[x] for x in xs]))
    wavelength = 400.0 + (xs - 129.0) * 100.0 / 211.667
    log_sensitivity = (476.0 - ys) / 207.0
    require(np.all(np.diff(wavelength) > 0), 'spectral wavelength is not increasing')
    return wavelength, log_sensitivity


def extract(pdf):
    require(hashlib.sha256(pdf.read_bytes()).hexdigest() == PDF_HASH, 'unexpected primary PDF bytes')
    with tempfile.TemporaryDirectory() as directory:
        prefix = Path(directory) / 'page3'
        subprocess.run(['pdfimages', '-f', '3', '-l', '3', '-png', str(pdf), str(prefix)],
                       check=True, capture_output=True)
        candidates = {}
        for path in sorted(Path(directory).glob('page3-*.png')):
            with Image.open(path) as image:
                if image.mode == 'RGB' and image.size == CHARACTERISTIC_SIZE:
                    candidates['characteristic'] = path
                elif image.mode == 'RGB' and image.size == SPECTRAL_SIZE:
                    candidates['spectral'] = path
        require(set(candidates) == {'characteristic', 'spectral'}, 'page 3 plot images missing')
        require(image_sha(candidates['characteristic']) == CHARACTERISTIC_IMAGE_SHA256,
                'characteristic image bytes changed')
        require(image_sha(candidates['spectral']) == SPECTRAL_IMAGE_SHA256,
                'spectral image bytes changed')
        anchor_x, anchor_y, characteristic_anchors = trace_characteristic(candidates['characteristic'])
        characteristic_anchors_pixels = characteristic_anchors
        anchor_pixel_x = np.array([point[0] for point in characteristic_anchors], dtype=float)
        anchor_pixel_y = np.array([point[1] for point in characteristic_anchors], dtype=float)
        dense_pixels = np.unique(np.r_[np.arange(anchor_pixel_x[0], anchor_pixel_x[-1] + 0.1, 2.0), anchor_pixel_x[-1]])
        dense_pixel_y = PchipInterpolator(anchor_pixel_x, anchor_pixel_y)(dense_pixels)
        dense_exposure = (dense_pixels - 712.2) * 4.0 / 605.0
        characteristic_x = dense_exposure
        characteristic_y = (556.0 - dense_pixel_y) / 152.0
        spectral_x, spectral_y = trace_sensitivity(candidates['spectral'])
        version = subprocess.run(['pdfimages', '-v'], capture_output=True, text=True).stderr.splitlines()[0]
    characteristic = np.column_stack([characteristic_x, characteristic_y])
    spectral = np.column_stack([spectral_x, spectral_y])
    # Keep a compact but faithful set; PCHIP reconstructs between measured pixels.
    spectral = spectral[::2]
    spectral = np.unique(np.vstack([spectral, [spectral_x[-1], spectral_y[-1]]]), axis=0)
    return {
        'stock': 'fujifilm_acros_100_ii',
        'primary_url': PRIMARY_URL,
        'primary_sha256': PDF_HASH,
        'primary_revision': PRIMARY_REVISION,
        'primary_page': PRIMARY_PAGE,
        'process': 'NEOPAN 100 ACROS II 135; Microfine stock, small tank, 20 C, 10 minutes; diffuse visual density.',
        'axis_mapping': {
            'characteristic': {
                'raster_bounds': [107.0, 100.0, 864.0, 556.0],
                'x_grid_pixels': [107, 183, 259, 335, 411, 487, 561, 636, 712, 788, 864],
                'x_grid_values': [-4.0, -3.5, -3.0, -2.5, -2.0, -1.5, -1.0, -0.5, 0.0, 0.5, 1.0],
                'y_grid_pixels': [100, 176, 252, 328, 404, 480, 556],
                'y_grid_values': [3.0, 2.5, 2.0, 1.5, 1.0, 0.5, 0.0],
                'x_formula': '(pixel_x - 712.2) * 4 / 605',
                'y_formula': '(556 - pixel_y) / 152'
            },
            'sensitivity': {
                'raster_bounds': [89.0, 62.0, 805.0, 685.0],
                'x_grid_pixels': [129, 341, 552, 764],
                'x_grid_values': [400.0, 500.0, 600.0, 700.0],
                'y_grid_pixels': [269, 476],
                'y_grid_values': [1.0, 0.0],
                'x_formula': '400 + (pixel_x - 129) * 100 / 211.667',
                'y_formula': '(476 - pixel_y) / 207',
                'note': 'The 1.0 bracket spans the two horizontal grid lines; absolute log offset is arbitrary and is normalized by the adapter.'
            }
        },
        'extraction': {
            'method': 'pdfimages extracts the pinned 300 dpi plot rasters; grid/frame lines removed; curve traced by continuity; 5-sample median smoothing; every second pixel retained',
            'characteristic_image_sha256': CHARACTERISTIC_IMAGE_SHA256,
            'spectral_image_sha256': SPECTRAL_IMAGE_SHA256,
            'characteristic_image_size': list(CHARACTERISTIC_SIZE),
            'spectral_image_size': list(SPECTRAL_SIZE),
            'pdfimages_version': version,
            'characteristic_anchors': len(characteristic_anchors_pixels),
            'characteristic_points': len(characteristic),
            'spectral_points': len(spectral),
            'characteristic_pixel_domain': [int(characteristic_anchors[0][0]), int(characteristic_anchors[-1][0])],
            'spectral_wavelength_domain': [float(spectral_x[0]), float(spectral_x[-1])]
        },
        'characteristic': characteristic.tolist(),
        'characteristic_anchors': [[round(x, 8), round(y, 8)] for x, y in characteristic_anchors_pixels],
        'log_sensitivity': spectral.tolist(),
        'limitations': [
            'The page contains 300 dpi raster plots, not vector centerlines; graph-line width and raster sampling limit precision.',
            'The 10-minute curve is identified by its printed label and vertical order; 5/15/20-minute curves are not the selected process.',
            'The spectral curve is a wedge spectrogram relative log response; the sheet does not state its density threshold. The profile stores an explicit unspecified-threshold limitation.',
            'The x-axis labels are printed at -3 through 1; the plot frame extends left to about -4 and right to about 1; only the plotted curve domain is used.',
            'No measured base spectrum, silver grain calibration, halation PSF, or scanner response is supplied by this publication.',
            'Microfine 10 minutes is a published starting point, not a guarantee for a particular tank/agitation or aged film.'
        ]
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pdf', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    data = extract(args.pdf)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2, allow_nan=False) + '\n')


if __name__ == '__main__':
    main()
