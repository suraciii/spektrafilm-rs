#!/usr/bin/env python3
"""Extract Agfachrome RSX II 100 page-8 vector data; see docs/agfachrome100.md."""
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

PDF_HASH = '1a80cea6b7afca268ace723f969a5027197c8c6118483372000b98a63b35298e'
GEOMETRY_HASH = '9b38323533535606ddca0291a1fd8f35dfc787a7901bcf84b3497a352c302aa1'
PRIMARY_URL = 'https://analoguephotolab.com/wp-content/uploads/2019/04/AGFA-Films.pdf'
STATUS_HASH = 'c04ea67f57bfef6852ac50d0306291f98b59c1509ff3a4dbd98ea2bc9564ba75'
STATUS_TABLE_HASH = 'da654eb0fbd7d14b9dc21e2ebb88c7974f575e0515a64b005cc556aa2225fa29'
STATUS_URL = 'https://raw.githubusercontent.com/JanLohse/spectral_film_lut/6307a98f3cf4f2d7b6d09bab82fbf6f64356ccfe/src/spectral_film_lut/densiometry.py'
# Middle RSX II 100 panel; coordinates are PDF/SVG user space, with the
# page-8 path transform matrix(1,0,0,-1,0,765). Frame endpoints were cross-
# checked against pdftotext -bbox labels; they are graph extents, not label centers.
X_TICKS = [229.3945, 356.4414]
X_VALUES = [350., 750.]
Y_SPEC = [79.5156, 174.8047]
Y_SPEC_VALUES = [2., -1.]
Y_DYE = [220.4141, 283.9180]
Y_DYE_VALUES = [1.5, 0.]
X_CHAR = [230.6563, 356.4414]
X_CHAR_VALUES = [-3., 2.]
Y_CHAR = [461.3242, 561.9141]
Y_CHAR_VALUES = [4., 0.]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def parse_path(d, subdivisions=32):
    tokens = re.findall(r'[A-Za-z]|[-+]?(?:\d*\.)?\d+(?:[eE][-+]?\d+)?', d)
    points, i, last = [], 0, None
    while i < len(tokens):
        op = tokens[i]
        i += 1
        require(op in ('M', 'L', 'C'), f'unsupported SVG command: {op}')
        n = {'M': 2, 'L': 2, 'C': 6}[op]
        values = np.array(list(map(float, tokens[i:i + n]))).reshape(-1, 2)
        i += n
        if op in ('M', 'L'):
            require(op != 'M' or last is None, 'discontinuous curve')
            points.append(values[0])
        else:
            require(last is not None, 'cubic without start')
            for t in np.linspace(0, 1, subdivisions + 1)[1:]:
                points.append((1 - t) ** 3 * last + 3 * (1 - t) ** 2 * t * values[0]
                              + 3 * (1 - t) * t * t * values[1] + t ** 3 * values[2])
        last = values[-1]
    result = np.asarray(points)
    require(np.isfinite(result).all() and np.all(np.diff(result[:, 0]) > 0),
            'invalid curve coordinates')
    return result


def bbox(points):
    return (float(points[:, 0].min()), float(points[:, 1].min()),
            float(points[:, 0].max()), float(points[:, 1].max()))


def affine(positions, values):
    slope, intercept = np.polyfit(np.asarray(positions), np.asarray(values), 1)
    return float(slope), float(intercept)


def mapped(points, xaxis, yaxis):
    x = points[:, 0] * xaxis[0] + xaxis[1]
    y = (765. - points[:, 1]) * yaxis[0] + yaxis[1]
    return np.stack([x, y], axis=1)


def choose(paths, lo_x, hi_x, lo_y, hi_y):
    out = []
    for path in paths:
        b = path['bbox']
        if lo_x <= b[0] and b[2] <= hi_x and lo_y <= b[1] and b[3] <= hi_y:
            out.append(path)
    return out


def coordinates(paths):
    xaxis = affine(X_TICKS, X_VALUES)
    xchar = affine(X_CHAR, X_CHAR_VALUES)
    yspec = affine(Y_SPEC, Y_SPEC_VALUES)
    ydye = affine(Y_DYE, Y_DYE_VALUES)
    ychar = affine(Y_CHAR, Y_CHAR_VALUES)
    out = {}
    for name in ('blue', 'green', 'red'):
        out[name] = mapped(parse_path(paths[name]['d']), xaxis, yspec).tolist()
    for name in ('yellow', 'magenta', 'cyan', 'visual_grey'):
        out[name] = mapped(parse_path(paths[name]['d']), xaxis, ydye).tolist()
    for name in ('green', 'red', 'blue'):
        out[f'characteristic_{name}'] = mapped(
            parse_path(paths['characteristic_' + name]['d']), xchar, ychar).tolist()
    return out


def extract(pdf, status_file):
    require(digest(pdf) == PDF_HASH, 'unexpected primary PDF bytes')
    require(digest(status_file) == STATUS_HASH, 'unexpected Status A source bytes')
    with tempfile.TemporaryDirectory() as directory:
        svg = Path(directory) / 'page8.svg'
        subprocess.run(['pdftocairo', '-f', '8', '-l', '8', '-svg', str(pdf), str(svg)], check=True)
        all_paths = []
        for element in ET.parse(svg).iter('{http://www.w3.org/2000/svg}path'):
            attr = element.attrib
            d = attr.get('d', '')
            if attr.get('stroke-width') in ('0.85', '0.851') and attr.get('fill') == 'none' and len(d) > 200:
                points = parse_path(d)
                all_paths.append({'d': d, 'transform': attr.get('transform', ''),
                                  'stroke_dasharray': attr.get('stroke-dasharray', ''),
                                  'bbox': bbox(points), 'points': points})
    require(len(all_paths) > 20, 'too few labelled curves')
    xaxis = affine(X_TICKS, X_VALUES)
    xchar = affine(X_CHAR, X_CHAR_VALUES)
    yspec = affine(Y_SPEC, Y_SPEC_VALUES)
    ydye = affine(Y_DYE, Y_DYE_VALUES)
    ychar = affine(Y_CHAR, Y_CHAR_VALUES)

    spectral_paths = choose(all_paths, 230, 350, 590, 670)
    require(len(spectral_paths) == 3, f'expected 3 spectral curves, got {len(spectral_paths)}')
    spectral = []
    for path in spectral_paths:
        x = path['points'][:, 0] * xaxis[0] + xaxis[1]
        y = (765. - path['points'][:, 1]) * yspec[0] + yspec[1]
        spectral.append({'path': path, 'peak': float(x[int(np.argmax(y))])})
    spectral.sort(key=lambda item: item['peak'])
    names = ['blue', 'green', 'red']
    vector_paths = {name: {'d': item['path']['d'], 'transform': item['path']['transform'],
                           'stroke_dasharray': item['path']['stroke_dasharray']}
                    for name, item in zip(names, spectral)}

    dye_paths = choose(all_paths, 230, 350, 480, 531)
    require(len(dye_paths) == 4, f'expected 4 dye curves, got {len(dye_paths)}')
    dye = []
    visual = None
    for path in dye_paths:
        x = path['points'][:, 0] * xaxis[0] + xaxis[1]
        y = (765. - path['points'][:, 1]) * ydye[0] + ydye[1]
        item = {'path': path, 'peak': float(x[int(np.argmax(y))])}
        if path['stroke_dasharray'] == '0.85 0.85':
            visual = item
        else:
            dye.append(item)
    require(visual is not None and len(dye) == 3, 'missing visual grey or CMY curves')
    dye.sort(key=lambda item: item['peak'])
    for name, item in zip(['yellow', 'magenta', 'cyan'], dye):
        vector_paths[name] = {'d': item['path']['d'], 'transform': item['path']['transform'],
                              'stroke_dasharray': item['path']['stroke_dasharray']}
    vector_paths['visual_grey'] = {'d': visual['path']['d'], 'transform': visual['path']['transform'],
                                   'stroke_dasharray': visual['path']['stroke_dasharray']}

    char_paths = choose(all_paths, 230, 357, 205, 304)
    require(len(char_paths) == 3, f'expected 3 characteristic curves, got {len(char_paths)}')
    char = []
    for path in char_paths:
        x = path['points'][:, 0] * xchar[0] + xchar[1]
        y = (765. - path['points'][:, 1]) * ychar[0] + ychar[1]
        char.append({'path': path, 'mid': float(np.interp(-2., x, y))})
    char.sort(key=lambda item: item['mid'], reverse=True)
    for name, item in zip(['green', 'red', 'blue'], char):
        vector_paths['characteristic_' + name] = {
            'd': item['path']['d'], 'transform': item['path']['transform'],
            'stroke_dasharray': item['path']['stroke_dasharray']}

    require(len(vector_paths) == 10, 'expected 3 spectral + 4 dye + 3 characteristic paths')
    coordinates = {}
    for name in ('blue', 'green', 'red'):
        coordinates[name] = mapped(parse_path(vector_paths[name]['d']), xaxis, yspec).tolist()
    for name in ('yellow', 'magenta', 'cyan', 'visual_grey'):
        coordinates[name] = mapped(parse_path(vector_paths[name]['d']), xaxis, ydye).tolist()
    for name in ('green', 'red', 'blue'):
        coordinates[f'characteristic_{name}'] = mapped(
            parse_path(vector_paths['characteristic_' + name]['d']), xchar, ychar).tolist()

    status = [ast.literal_eval(node.value) for node in ast.parse(Path(status_file).read_text()).body
              if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name)
              and node.targets[0].id == 'STATUS_A' and isinstance(node.value, ast.List)]
    require(len(status) == 1, 'Status A literal missing or ambiguous')
    require(fingerprint(status[0]) == STATUS_TABLE_HASH, 'Status A table changed')
    result = {
        'stock': 'agfachrome_rsx_ii_100',
        'primary_url': PRIMARY_URL,
        'primary_sha256': PDF_HASH,
        'primary_revision': 'Agfa Professional Films F-PF-E4, fourth edition, August 2004',
        'primary_page': 8,
        'process': 'Agfachrome RSX II 100; daylight 1/100 second; AP44/E-6; Status A/M densitometry.',
        'channel_order': 'spectral sensitivity Blue/Green/Red; dye density Yellow/Magenta/Cyan + visual grey; characteristic Green/Red/Blue',
        'axis_ticks': {'spectral': [X_TICKS, X_VALUES, Y_SPEC, Y_SPEC_VALUES],
                       'dye': [X_TICKS, X_VALUES, Y_DYE, Y_DYE_VALUES],
                       'characteristic': [X_CHAR, X_CHAR_VALUES, Y_CHAR, Y_CHAR_VALUES]},
        'vector_paths': vector_paths,
        'geometry_sha256': fingerprint(vector_paths),
        'coordinates': coordinates,
        'status_a': status[0],
        'status_a_url': STATUS_URL,
        'status_a_sha256': STATUS_HASH,
        'status_a_table_sha256': fingerprint(status[0]),
        'method': 'Bézier centerlines, 32 subdivisions; frame-endpoint affine axes from the RSX II 100 middle panel.',
        'limitations': [
            'Publication drawing, not batch-specific measurements.',
            'CMY density curves are treated as additive basis spectra; base removal is inferred from the common clear-end segment.',
            'No independently measured clear-film spectrum or nonneutral layer coupling.',
            'Spectrograph uses 2850 K tungsten; the printed lg Sensitivity axis is explicit but no equal-energy conversion was applied by Agfa.',
            'Characteristic curves are printed at daylight 1/100 s, AP44/E-6, Status A; exact batch and scanner response unknown.',
            'RMS 10 is a 48 µm/diffuse-density-1.0 measurement, not a runtime grain calibration.',
        ],
    }
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pdf', type=Path, required=True)
    parser.add_argument('--status-source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    data = extract(args.pdf, args.status_source)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2, allow_nan=False) + '\n')


if __name__ == '__main__':
    main()
