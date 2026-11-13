#!/usr/bin/env python3
"""Extract the pinned ASTIA 100F bulletin; see docs/astia100f.md."""
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

PDF_HASH = 'a3ed5826d934743b91d750a7ce6c79041c8b3ba0b1afc05ff3cf0b8d175e0809'
PRIMARY_URL = 'https://125px.com/docs/film/fuji/astia_100f_datasheet.pdf'
STATUS_HASH = 'c04ea67f57bfef6852ac50d0306291f98b59c1509ff3a4dbd98ea2bc9564ba75'
GEOMETRY_HASH = 'dd6e6aa8a328cc07f38f7599b532710dfbeff9d426d642a697262e166c394b0f'
STATUS_TABLE_HASH = 'da654eb0fbd7d14b9dc21e2ebb88c7974f575e0515a64b005cc556aa2225fa29'
STATUS_URL = 'https://raw.githubusercontent.com/JanLohse/spectral_film_lut/6307a98f3cf4f2d7b6d09bab82fbf6f64356ccfe/src/spectral_film_lut/densiometry.py'
# Reviewed labelled tick centers in the PDF's upward-y coordinate system.
TICKS = {
    'characteristic': [[105.785156, 149.843750, 193.046875, 236.531250, 280.007812],
                       [-3, -2, -1, 0, 1],
                       [596.093750, 574.933594, 552.847656, 531.011719, 509.453125,
                        487.828125, 465.699219, 444.273438, 423.066406],
                       [4, 3.5, 3, 2.5, 2, 1.5, 1, .5, 0]],
    'log_sensitivity': [[347.992188, 407.441406, 466.324219, 525.179688],
                        [400, 500, 600, 700], [565.761719, 507.664062, 449.843750], [1, 0, -1]],
    'dye_density': [[348.574219, 408.023438, 466.906250, 525.761719],
                    [400, 500, 600, 700], [293.652344, 234.125, 174.597656], [1, .5, 0]],
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sample_path(path, subdivisions=64):
    require(path['transform'] == 'matrix(1, 0, 0, -1, 0, 794)', 'unexpected curve transform')
    tokens = re.findall(r'[A-Za-z]|[-+]?(?:\d*\.)?\d+(?:[eE][-+]?\d+)?', path['d'])
    points, i, last = [], 0, None
    while i < len(tokens):
        op = tokens[i]
        i += 1
        require(op in ('M', 'L', 'C'), f'unsupported SVG command: {op}')
        count = {'M': 2, 'L': 2, 'C': 6}[op]
        values = np.array(list(map(float, tokens[i:i+count]))).reshape(-1, 2)
        i += count
        if op in ('M', 'L'):
            require(op != 'M' or last is None, 'discontinuous curve')
            points.append(values[0])
        else:
            require(last is not None, 'cubic without start')
            for t in np.linspace(0, 1, subdivisions+1)[1:]:
                points.append((1-t)**3*last+3*(1-t)**2*t*values[0]
                              +3*(1-t)*t*t*values[1]+t**3*values[2])
        last = values[-1]
    result = np.array(points)
    require(np.isfinite(result).all() and np.all(np.diff(result[:, 0]) > 0), 'invalid curve coordinates')
    return result


def map_axes(points, kind):
    ticks = TICKS[kind]
    result = points.copy()
    for axis, (positions, labels) in enumerate(((ticks[0], ticks[1]), (ticks[2], ticks[3]))):
        slope, intercept = np.linalg.lstsq(np.array([positions, np.ones(len(positions))]).T,
                                          labels, rcond=None)[0]
        result[:, axis] = points[:, axis]*slope+intercept
    return result


def coordinates(paths, subdivisions=64):
    return {kind: [map_axes(sample_path(p, subdivisions), kind).tolist() for p in curves]
            for kind, curves in paths.items()}


def extract(pdf, status_file):
    require(digest(pdf) == PDF_HASH, 'unexpected primary PDF bytes')
    require(digest(status_file) == STATUS_HASH, 'unexpected Status A source bytes')
    with tempfile.TemporaryDirectory() as directory:
        svg = Path(directory)/'page8.svg'
        subprocess.run(['pdftocairo', '-f', '8', '-l', '8', '-svg', str(pdf), str(svg)], check=True)
        candidates = [e.attrib for e in ET.parse(svg).iter('{http://www.w3.org/2000/svg}path')
                      if e.get('stroke-width') == '1' and len(e.get('d', '')) > 250]
    require(len(candidates) == 9, 'expected nine labelled material curves')
    selected = {'characteristic': [None]*3, 'log_sensitivity': [None]*3, 'dye_density': [None]*3}
    for p in candidates:
        xy = sample_path(p)
        x, y = xy[0]
        if x < 100 and y > 550:
            kind = 'characteristic'
            channel = {'': 0, '9 1.5 2.5 1.5': 1, '3 1.5': 2}[p.get('stroke-dasharray', '')]
        elif y > 450:
            kind = 'log_sensitivity'
            channel = 2 if x < 360 else (1 if x < 410 else 0)
        else:
            kind = 'dye_density'
            channel = 2 if y > 240 else (1 if y < 182 else 0)
        require(selected[kind][channel] is None, 'duplicate curve label')
        selected[kind][channel] = {k: p.get(k, '') for k in ('d', 'transform', 'stroke-dasharray')}
    require(all(all(c is not None for c in curves) for curves in selected.values()), 'missing curve')
    require(fingerprint(selected) == GEOMETRY_HASH, 'labelled curve geometry changed')
    # Parse only literal instrument response data; never execute downloaded code.
    tables = [ast.literal_eval(n.value) for n in ast.parse(status_file.read_text()).body
              if isinstance(n, ast.Assign) and isinstance(n.targets[0], ast.Name)
              and n.targets[0].id == 'STATUS_A' and isinstance(n.value, ast.List)]
    require(len(tables) == 1, 'Status A literal missing or ambiguous')
    require(fingerprint(tables[0]) == STATUS_TABLE_HASH, 'Status A table changed')
    return {'stock': 'fujifilm_astia_100f', 'primary_url': PRIMARY_URL, 'primary_sha256': PDF_HASH,
            'primary_revision': 'AF3-149E (EIGI-03.11-HB)', 'primary_page': 8,
            'process': 'ASTIA 100F RAP100F; daylight, 1/50 second; E-6/CR-56; Fuji FAD-30S (Status A).',
            'channel_order': 'characteristic and sensitivity RGB; dye spectra CMY',
            'axis_ticks': TICKS, 'vector_paths': selected,
            'geometry_sha256': fingerprint(selected),
            'coordinates': coordinates(selected),
            'status_a': tables[0], 'status_a_url': STATUS_URL, 'status_a_sha256': STATUS_HASH,
            'status_a_table_sha256': fingerprint(tables[0]),
            'method': 'Bézier centerlines, 64 subdivisions; least-squares affine axes through labelled ticks.',
            'limitations': ['Publication drawing, not batch-specific measurements.',
                           'Separated-light dye shapes act as additive basis spectra; base removal not specified.',
                           'No independently measured clear-film spectrum or nonneutral layer coupling.',
                           'Outside plotted domains, spectral tails require assumptions.']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pdf', type=Path, required=True)
    parser.add_argument('--status-source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = extract(args.pdf, args.status_source)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, allow_nan=False)+'\n')


if __name__ == '__main__':
    main()
