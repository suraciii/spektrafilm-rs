#!/usr/bin/env python3
"""Extract Kodak F-4016 (June 2018) page 8 vectors for T-MAX 100; see docs/tmax100.md."""
import argparse
import hashlib
import json
import re
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

import numpy as np

PDF_HASH = '00518d7d1e296d6065f6e41ea3db999e5d8714679fb3d2b76d9a49eb207113be'
PRIMARY_URL = 'https://www.kodakprofessional.com/sites/default/files/wysiwyg/pro/resources/f4016_TMax_100.pdf'
GEOMETRY_HASH = 'becde9deead3a7ed67d13488c1fd2d764579060e36ee182b7bf2ddca2a20bda3'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sample_path(path, subdivisions=64):
    tokens = re.findall(r'[A-Za-z]|[-+]?(?:\d*\.)?\d+(?:[eE][-+]?\d+)?', path['d'])
    points, i, last = [], 0, None
    while i < len(tokens):
        op = tokens[i]
        i += 1
        require(op in ('M', 'L', 'C'), f'unsupported SVG command {op}')
        count = {'M': 2, 'L': 2, 'C': 6}[op]
        values = np.array(list(map(float, tokens[i:i + count]))).reshape(-1, 2)
        i += count
        if op in ('M', 'L'):
            require(op != 'M' or last is None, 'discontinuous source curve')
            points.append(values[0])
            last = values[0]
        else:
            require(last is not None, 'cubic without start')
            for t in np.linspace(0, 1, subdivisions + 1)[1:]:
                points.append((1 - t) ** 3 * last + 3 * (1 - t) ** 2 * t * values[0] +
                              3 * (1 - t) * t * t * values[1] + t ** 3 * values[2])
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
        # Page-8 D-76 panel fitted from major tick anchors: logE=(x-503.155575)/36.887557;
        # D=(241.482838-y)/46.118738.
        result[:, 0] = (points[:, 0] - 503.155575) / 36.887557
        result[:, 1] = (241.482838 - points[:, 1]) / 46.118738
    else:
        # Page-8 spectral panel local 400-700 nm fit: nm=(x+25.890607)/0.401942;
        # logS=(574.228200-y)/37.867800.
        result[:, 0] = (points[:, 0] + 25.890607) / 0.401942
        result[:, 1] = (574.228200 - points[:, 1]) / 37.867800
    return result


def extract(pdf):
    require(hashlib.sha256(pdf.read_bytes()).hexdigest() == PDF_HASH, 'unexpected primary PDF bytes')
    with tempfile.TemporaryDirectory() as directory:
        svg = Path(directory) / 'page8.svg'
        subprocess.run(['pdftocairo', '-f', '8', '-l', '8', '-svg', str(pdf), str(svg)], check=True)
        candidates = []
        for element in ET.parse(svg).iter('{http://www.w3.org/2000/svg}path'):
            p = element.attrib
            if float(p.get('stroke-width', 0)) > .6 and len(p.get('d', '')) > 350:
                try:
                    xy = sample_path(p)
                except ValueError:
                    continue
                if (385 < xy[0, 0] < 390 and 225 < xy[0, 1] < 238 and 3000 < len(p['d'])
                        and p.get('stroke-dasharray') == '3.2392 1.6196'):
                    candidates.append({'name': 'characteristic', **{k: p.get(k, '') for k in ('d', 'transform', 'stroke-dasharray')}})
                elif 130 < xy[0, 0] < 140 and 510 < xy[0, 1] < 525 and 1200 < len(p['d']):
                    candidates.append({'name': 'sensitivity_0.3', **{k: p.get(k, '') for k in ('d', 'transform', 'stroke-dasharray')}})
                elif 130 < xy[0, 0] < 140 and 548 < xy[0, 1] < 562 and 1200 < len(p['d']):
                    candidates.append({'name': 'sensitivity_1.0', **{k: p.get(k, '') for k in ('d', 'transform', 'stroke-dasharray')}})
        # One path per role; reject duplicate/misidentified vector sets.
        selected = {}
        for p in candidates:
            require(p['name'] not in selected, f'duplicate source path: {p["name"]}')
            selected[p['name']] = {k: p[k] for k in ('d', 'transform', 'stroke-dasharray')}
        require(set(selected) == {'characteristic', 'sensitivity_0.3', 'sensitivity_1.0'}, 'source curve identities missing')
        fingerprint = hashlib.sha256(json.dumps(selected, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        require(fingerprint == GEOMETRY_HASH, 'source geometry changed; inspect labelled curves before accepting')
        characteristic = map_axes(sample_path(selected['characteristic']), 'characteristic')
        sensitivity = {label: map_axes(sample_path(selected['sensitivity_' + label]), 'sensitivity')
                       for label in ('0.3', '1.0')}
        require(np.all(np.diff(characteristic[:, 0]) > 0) and np.all(np.diff(characteristic[:, 1]) >= 0),
                'characteristic is not monotone')
        result = {
            'stock': 'kodak_tmax_100',
            'primary_url': PRIMARY_URL,
            'primary_sha256': PDF_HASH,
            'primary_revision': 'June 2018, F-4016',
            'primary_page': 8,
            'process': '100TMX 135; KODAK D-76 stock, small tank, 20 C, 7.5 min curve, intermittent agitation; diffuse visual density.',
            'axis_mapping': {
                'characteristic': {'x_pixels': [355.605469, 540.042969], 'x_values': [-4, 1],
                                   'y_pixels': [241.482838, 57.007812], 'y_values': [0, 4],
                                   'formula': 'logE=(px-503.155575)/36.887557; D=(241.482838-py)/46.118738'},
                'sensitivity': {'x_pixels': [74.602, 275.128906], 'x_values': [250, 750],
                                'y_pixels': [649.963, 498.492188], 'y_values': [-2, 2],
                                'formula': 'nm=(px+25.890607)/0.401942 (local 400-700 fit); log=(574.2282-py)/37.8678'},
            },
            'extraction': {
                'method': 'SVG M/L/C centerlines; 64 uniform parameter subdivisions per cubic; page-8 vector paths selected by page geometry.',
                'geometry_sha256': fingerprint,
                'converter_version': subprocess.run(['pdftocairo', '-v'], check=True, capture_output=True, text=True).stderr.splitlines()[0],
                'pdfimages': 'assertion from pdfimages -list: no raster image objects; page-8 curves are vector paths',
            },
            'vector_paths': selected,
            'characteristic': characteristic.tolist(),
            'log_sensitivity': {label: values.tolist() for label, values in sensitivity.items()},
            'limitations': [
                'Publication drawing, not batch-specific measurement.',
                'The first plotted density 0.213071 at logE -3.14997 is a base-density proxy, not a measured D-min.',
                'The p8 7.5-minute curve is the selected printed curve; p3 lists 6.5 minutes at 68 F as a starting-point table value. Do not silently substitute one for the other.',
                'The spectral plot gives 0.3 and 1.0 above D-min curves; 0.3 is used as the profile baseline, 1.0 is retained for ablation, not a second layer.',
                'No measured silver/base spectrum, grain calibration, halation PSF, or scanner response is supplied.',
                'The characteristic curve clamps outside the plotted range; no inferred shoulder or extra latitude.',
                'One aggregate silver record; not a measured emulsion sublayer decomposition.',
                'Scope is the 135 source identity. The built-in look preserves caller format and is not separately calibrated for 120/sheet; no 120/sheet material or 2026 Ektapan 100 alias is delivered; Ektapan identity is unverified.',
            ],
        }
        return result


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
