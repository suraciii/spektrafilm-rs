#!/usr/bin/env python3
"""Extract G-24 published vector centerlines, not laboratory observations.

Requires pdftocairo and NumPy. See docs/polymax.md for source and scope.
"""
import argparse
import ast
import hashlib
import json
import re
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

import numpy as np

PDF_HASH = 'd3ea3fe1487d34c2fd4684194b2863c93653e2c73ec175f68805fcafe3e703b6'
# Independently visually audited curve identities and coordinates, not merely order.
COORDINATE_HASH = '11dfb34f5c7de54c650f90372b1a5507a7cfb2507bb68f034e686ddc7990298d'


def validate_curve_identity(curves, sensitivity):
    payload = json.dumps({'characteristic': curves, 'log_sensitivity': sensitivity},
                         sort_keys=True, separators=(',', ':'), allow_nan=False)
    require(hashlib.sha256(payload.encode()).hexdigest() == COORDINATE_HASH,
            'published curve identity changed; inspect converter and axis mapping before accepting')

CHECK_HASH = 'c005c7c15cd5a821a9063330b5c02f844af548f8db21b49adf2305a41a49bca5'
CHECK_URL = ('https://raw.githubusercontent.com/JanLohse/spectral_film_lut/'
             '6307a98f3cf4f2d7b6d09bab82fbf6f64356ccfe/'
             'src/spectral_film_lut/bw_print_film/kodak_polymax_fine_art.py')
FRAMES = {
    'odd': (95.714844, 280.105469, 493.847656, 678.28125),
    'even': (95.71875, 280.113281, 261.628906, 446.070312),
    'none': (375.421875, 559.820312, 495.253906, 679.65625),
}
# Order and labels were visually matched to the leaders on printed page 5.
CURVES = [('5', 0, 'odd'), ('1', 1, 'odd'), ('3', 2, 'odd'),
          ('4', 3, 'even'), ('2', 4, 'even'), ('0', 5, 'even'),
          ('unfiltered', 6, 'none'), ('-1', 7, 'none')]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def extract(pdf, digitization):
    require(digest(pdf) == PDF_HASH, 'G-24 source hash mismatch')
    require(digest(digitization) == CHECK_HASH, 'cross-check source hash mismatch')
    with tempfile.TemporaryDirectory() as directory:
        svg = Path(directory)/'page.svg'
        subprocess.run(['pdftocairo', '-f', '5', '-l', '5', '-svg', str(pdf), str(svg)], check=True)
        paths = []
        for element in ET.parse(svg).iter():
            if not element.tag.endswith('path') or element.get('stroke-width') not in ('1.08', '1.079'):
                continue
            require(element.get('transform') == 'matrix(1, 0, 0, -1, 0, 792)', 'unexpected transform')
            drawing = element.get('d')
            require(set(re.findall('[A-Za-z]', drawing)) == {'M', 'L'}, 'unexpected path commands')
            for part in drawing.split('M ')[1:]:
                values = list(map(float, re.findall(r'-?\d+(?:\.\d+)?', part)))
                paths.append(np.array(values).reshape(-1, 2))
    require(len(paths) == 11, 'unexpected curve count')
    curves = {}
    for label, index, frame in CURVES:
        a, b, c, d = FRAMES[frame]
        points = (paths[index]-[a, c])/[b-a, d-c]*4
        require(np.all(np.diff(points[:, 0]) > 0), 'non-increasing exposure coordinates')
        curves[label] = points.tolist()
    sensitivity = {}
    for threshold, index in [('0.6', 8), ('1.6', 9), ('0.3', 10)]:
        points = paths[index]
        # Labeled 400/550 nm ticks and 0/-1 log-sensitivity grid lines.
        converted = np.c_[400+(points[:, 0]-426.894531)*150/(487.179688-426.894531),
                          (points[:, 1]-400.050781)/(400.050781-362.351562)]
        require(np.all(np.diff(converted[:, 0]) > 0), 'non-increasing spectral coordinates')
        sensitivity[threshold] = converted.tolist()
    validate_curve_identity(curves, sensitivity)
    converter = subprocess.run(['pdftocairo', '-v'], check=True, capture_output=True, text=True)
    converter_version = (converter.stdout + converter.stderr).splitlines()[0]
    original = {}
    # Parse literals only. Downloaded Python is never imported or executed.
    for node in ast.parse(digitization.read_text()).body:
        if not isinstance(node, ast.Assign):
            continue
        names = [target.id for target in node.targets if isinstance(target, ast.Name)]
        if 'curve' in names:
            original['curves'] = {('unfiltered' if k is None else str(k)): v
                                 for k, v in ast.literal_eval(node.value).items()}
        if 'KODAK_POLYMAX' in names:
            value = next(kw.value for kw in node.value.keywords if kw.arg == 'log_sensitivity')
            original['log_sensitivity'] = ast.literal_eval(value)
    comparison = {}
    for label, points in curves.items():
        points = np.array(points)
        other = np.array(sorted((float(k), v) for k, v in original['curves'][label].items()))
        mask = (other[:, 0] >= points[0, 0]) & (other[:, 0] <= points[-1, 0])
        error = np.interp(other[mask, 0], points[:, 0], points[:, 1])-other[mask, 1]
        comparison[label] = {'max_density_difference': float(max(abs(error))),
                             'median_density_difference': float(np.median(abs(error))),
                             'points': len(error)}
    other = np.array(sorted((float(k), v) for k, v in original['log_sensitivity'][0].items()))
    for threshold, points in sensitivity.items():
        points = np.array(points)
        mask = (other[:, 0] >= points[0, 0]) & (other[:, 0] <= points[-1, 0])
        error = np.interp(other[mask, 0], points[:, 0], points[:, 1])-other[mask, 1]
        comparison['sensitivity_'+threshold] = {'max_log_difference': float(max(abs(error))),
                                                'median_log_difference': float(np.median(abs(error)))}
    return {
        'primary_url': 'https://125px.com/docs/paper/kodak/g24.pdf',
        'primary_title': 'KODAK PROFESSIONAL POLYMAX Fine-Art Paper',
        'primary_revision': 'G-24, June 2005', 'primary_sha256': PDF_HASH, 'page': 5,
        'conditions': {'surface': 'F', 'developer': 'DEKTOL 1:2', 'time_seconds': 90,
                       'temperature_c': 20, 'characteristic_exposure_unit': 'log10(lux second)',
                       'density': 'reflection density; exact measurement geometry unspecified',
                       'spectral_sensitivity': 'log10 reciprocal exposure in erg/cm2; 2 seconds; thresholds above D-min',
                       'illuminant': 'Tungsten/tungsten-halogen recommended on p.1; exact plotted lamp spectrum unspecified'},
        'extraction': {'method': 'pdftocairo SVG polyline centerlines, axis-affine mapping; no raster tracing',
                       'converter_version': converter_version, 'coordinate_sha256': COORDINATE_HASH,
                       'command': 'pdftocairo -f 5 -l 5 -svg g24.pdf g24-p5.svg',
                       'characteristic_frames_pdf_points': FRAMES,
                       'sensitivity_calibration': {'wavelength_ticks_nm': [400, 550],
                           'x_ticks_pdf_points': [426.894531, 487.179688],
                           'log_sensitivity_ticks': [0, -1],
                           'y_ticks_pdf_points': [400.050781, 362.351562]},
                       'script_sha256': digest(Path(__file__)),
                       'reproduction': 'scripts/extract_polymax.py --pdf /path/to/g24.pdf --digitization /path/to/kodak_polymax_fine_art.py --output /path/to/source.json'},
        'characteristic': curves, 'log_sensitivity': sensitivity,
        'crosscheck': {'url': CHECK_URL, 'sha256': CHECK_HASH, 'differences': comparison},
        'limitations': ['Published representative curves, not measurements of a specific box.',
                       'Vector vertices are plot drawing coordinates, not independent experimental samples.',
                       'No full measured paper or silver-image reflectance spectra.',
                       'Graph label #5 is preserved; table uses 5+.',
                       'Manufacturer PDF is not redistributed.'],
        'license': 'New coordinate extraction contributions: CC BY-SA 4.0. Original Kodak technical data remain the property of their rights holder. PDF not redistributed. Jan Lohse coordinates used only for cross-check, not copied into these curves.',
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pdf', type=Path, required=True)
    parser.add_argument('--digitization', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    source = extract(args.pdf, args.digitization)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(source, indent=2, allow_nan=False)+'\n')
    print(json.dumps(source['crosscheck']['differences'], indent=2))


if __name__ == '__main__':
    main()
