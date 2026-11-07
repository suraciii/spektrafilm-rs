#!/usr/bin/env python3
"""Extract Kodak F-4017 (October 2021) page 8 vectors; see docs/trix400.md."""
import argparse
import hashlib
import json
import re
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

import numpy as np

PDF_HASH = '1d86b030cf7a8502e6f1414c630fa2cda30b68df53f49bccccce73c9c6562078'
PRIMARY_URL = 'https://kodakprofessional.com/sites/default/files/wysiwyg/film/f4017_trix_320400.pdf'
# Bound the labelled curves to reviewed SVG geometry, independent of XML order.
GEOMETRY_HASH = 'a73ce61e28bf600e7e7f47e8e2b075c9df51c47efec8181bdf371c6b537b999c'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sample_path(path, subdivisions=64):
    tokens = re.findall(r'[A-Za-z]|[-+]?(?:\d*\.)?\d+(?:[eE][-+]?\d+)?', path['d'])
    points, i, last = [], 0, None
    while i < len(tokens):
        op = tokens[i]; i += 1
        require(op in ('M', 'L', 'C'), f'unsupported SVG command {op}')
        count = {'M': 2, 'L': 2, 'C': 6}[op]
        values = np.array(list(map(float, tokens[i:i+count]))).reshape(-1, 2)
        i += count
        if op in ('M', 'L'):
            require(op != 'M' or last is None, 'discontinuous source curve')
            points.append(values[0]); last = values[0]
        else:
            require(last is not None, 'cubic without start')
            for t in np.linspace(0, 1, subdivisions+1)[1:]:
                points.append((1-t)**3*last + 3*(1-t)**2*t*values[0] +
                              3*(1-t)*t*t*values[1] + t**3*values[2])
            last = values[-1]
    matrix = list(map(float, re.findall(r'[-+]?(?:\d*\.)?\d+', path['transform'])))
    require(len(matrix) == 6 and matrix[:4] == [1., 0., 0., -1.], 'unexpected SVG transform')
    a, b, c, d, e, f = matrix
    result = np.array(points) @ np.array([[a, b], [c, d]]) + [e, f]
    require(np.all(np.isfinite(result)) and np.all(np.diff(result[:, 0]) > 0), 'invalid source coordinates')
    return result


def map_axes(points, kind):
    result = points.copy()
    if kind == 'characteristic':
        result[:, 0] = (points[:, 0]-354.4140625)/(538.8515625-354.4140625)*5-4
        result[:, 1] = (247.9765625-points[:, 1])/(247.9765625-63.48046875)*4
    else:
        result[:, 0] = (points[:, 0]-74.9453125)/(275.37109375-74.9453125)*500+250
        result[:, 1] = (442.97265625-points[:, 1])/(442.97265625-291.66796875)*4
    return result


def extract(pdf):
    require(hashlib.sha256(pdf.read_bytes()).hexdigest() == PDF_HASH, 'unexpected primary PDF bytes')
    with tempfile.TemporaryDirectory() as directory:
        svg = Path(directory)/'page8.svg'
        subprocess.run(['pdftocairo', '-f', '8', '-l', '8', '-svg', str(pdf), str(svg)], check=True)
        candidates = []
        for element in ET.parse(svg).iter('{http://www.w3.org/2000/svg}path'):
            p = element.attrib
            if float(p.get('stroke-width', 0)) > .6 and len(p.get('d', '')) > 350:
                candidates.append({k: p.get(k, '') for k in ('d', 'transform', 'stroke-dasharray')})
    selected = {}
    for p in candidates:
        xy = sample_path(p)
        if 370 < xy[0, 0] < 380 and 230 < xy[0, 1] < 240 and not p['stroke-dasharray']:
            selected['characteristic'] = p
        elif 94 < xy[0, 0] < 96 and 340 < xy[0, 1] < 355:
            selected['sensitivity_1.0'] = p
        elif 94 < xy[0, 0] < 96 and 390 < xy[0, 1] < 405:
            selected['sensitivity_0.3'] = p
    require(set(selected) == {'characteristic', 'sensitivity_1.0', 'sensitivity_0.3'}, 'source curve identities missing')
    fingerprint = hashlib.sha256(json.dumps(selected, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    require(fingerprint == GEOMETRY_HASH, 'source geometry changed; inspect labelled curves before accepting')
    result = {'stock': 'kodak_trix_400', 'primary_url': PRIMARY_URL, 'primary_sha256': PDF_HASH,
              'primary_revision': 'October 2021', 'primary_page': 8,
              'process': '400TX 35 mm; T-MAX Developer 1:4, 6 minutes, small tank, 20 C, agitation every 30 seconds; diffuse visual density.',
              'axis_mapping': {'characteristic': {'svg_bounds': [354.4140625, 63.48046875, 538.8515625, 247.9765625], 'x': [-4, 1], 'y': [4, 0]},
                               'sensitivity': {'svg_bounds': [74.9453125, 291.66796875, 275.37109375, 442.97265625], 'x': [250, 750], 'y': [4, 0]}},
              'extraction': {'method': 'SVG M/L/C centerlines; 64 uniform parameter subdivisions per cubic',
                             'geometry_sha256': fingerprint, 'converter_version': subprocess.run(['pdftocairo', '-v'], check=True, capture_output=True, text=True).stderr.splitlines()[0]},
              'vector_paths': selected,
              'characteristic': map_axes(sample_path(selected['characteristic']), 'characteristic').tolist(),
              'log_sensitivity': {label: map_axes(sample_path(selected['sensitivity_'+label]), 'sensitivity').tolist() for label in ('0.3', '1.0')},
              'limitations': ['Publication drawing, not batch-specific measurement.',
                              'Left plotted density is an estimated base+fog floor, not an unexposed-strip measurement.',
                              'D-min+1.0 effective sensitivity; other density threshold retained for comparison, not a separate layer.',
                              'No measured silver/base spectrum, matched T-MAX grain, halation, or angular-scattering calibration.']}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pdf', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    data = extract(args.pdf)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2, allow_nan=False)+'\n')


if __name__ == '__main__':
    main()
