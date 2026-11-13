#!/usr/bin/env python3
"""Prepare the unfiltered Polymax source profile; see docs/polymax.md."""
import argparse
import hashlib
import json
import re
from pathlib import Path

import numpy as np
import scipy
from scipy.interpolate import PchipInterpolator
from scipy.optimize import brentq

ROOT = Path(__file__).resolve().parents[1]
WL = np.arange(380., 781., 5.)
STOCK = 'kodak_polymax_fine_art'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def monotone(values):
    """Equal-weight isotonic regression removes only publication drawing reversals."""
    blocks = []
    for value in values:
        blocks.append([float(value), 1])
        while len(blocks) > 1 and blocks[-2][0] > blocks[-1][0]:
            b, a = blocks.pop(), blocks.pop()
            n = a[1] + b[1]
            blocks.append([(a[0]*a[1] + b[0]*b[1])/n, n])
    return np.concatenate([np.full(n, v) for v, n in blocks])


def sensitivity(source):
    # Use the explicitly identified D-min + 0.6 curve, not inferred emulsion layers.
    x, y = np.array(source['log_sensitivity']['0.6']).T
    logs = PchipInterpolator(x, y, extrapolate=False)(WL)
    for mask, edge, neighbor in ((WL < x[0], 0, 1), (WL > x[-1], -1, -2)):
        slope = (y[edge]-y[neighbor])/(x[edge]-x[neighbor])
        logs[mask] = y[edge] + slope*(WL[mask]-x[edge])
    logs = np.maximum(logs, -12.)
    lamp_path = ROOT/'crates/spektrafilm-core/src/spectral_service/illuminant.rs'
    match = re.search(r'pub const ILLUMINANT_T_F64[^=]*= \[(.*?)\];', lamp_path.read_text(), re.S)
    if match is None:
        raise ValueError('pinned runtime T illuminant not found')
    lamp = np.array([float(v) for v in match[1].split(',') if v.strip()])
    if lamp.shape != WL.shape or not np.all(lamp > 0):
        raise ValueError('runtime T illuminant has invalid samples')
    normalization = float(lamp @ (10.**logs))
    return logs - np.log10(normalization), {
        'threshold_above_dmin': .6, 'measured_interval_nm': [float(x[0]), float(x[-1])],
        'tail_policy': 'Endpoint log slopes to numerical floor -12; extrapolated, not measured.',
        'normalization': normalization, 'runtime_illuminant_source_sha256': digest(lamp_path),
        'reference': 'Unit integrated response under bundled T; relative engine coordinate, not absolute ISO or timer calibration.',
    }


def prepare(source_path, data_dir):
    source = json.loads(source_path.read_text())
    x, raw = np.array(source['characteristic']['unfiltered']).T
    if not np.all(np.diff(x) > 0) or not np.all(np.isfinite(raw)):
        raise ValueError('invalid unfiltered source curve')
    y = monotone(raw)
    curve = PchipInterpolator(x, y, extrapolate=False)
    # Relative engine origin: unfiltered paper-white-relative midgray reflectance.
    reference_density = float(y[0] - np.log10(.184))
    reference = float(brentq(lambda t: curve(t)-reference_density, x[0], x[-1]))
    # Keep every source vertex as well as a dense grid. Runtime linear lookup
    # reproduces the publication vertices and bounds inter-vertex error.
    grid = np.unique(np.r_[np.linspace(-1., 4., 1001), x])
    logs, spectral_report = sensitivity(source)
    report = {'source_sha256': digest(source_path), 'generator_sha256': digest(Path(__file__)),
              'illuminants_sha256': digest(data_dir/'illuminants/catalog.json'),
              'versions': {'numpy': np.__version__, 'scipy': scipy.__version__},
              'log10_lux_second_origin': reference,
              'unfiltered_reference_reflection_density': reference_density,
              'sensitivity': spectral_report, 'profiles': {},
              'limitations': source['limitations'] + [
                  'Fixed-filtration neutral-silver approximation; no arbitrary spectral variable-contrast prediction.',
                  'Neutral base and silver are not measured spectral reflectances; no fluorescence or surface model.',
                  'Unfiltered source condition with one effective sensitivity; supported for a fixed source and spectrally neutral negative.',
                  'Reference T lamp is a convention; exact published lamp/filter exposure calibration is unavailable.',
                  'Scanner normalizes paper white: output is paper-white-relative, not absolute reflectance.',
                  'No stock-specific grain, texture, gloss, toning, reciprocity or aging calibration.']}
    total = curve(np.clip(grid, x[0], x[-1]))
    base = float(y[0])
    density = total - base
    dense_x = np.linspace(x[0], x[-1], 100001)
    runtime = np.interp(dense_x, grid, total)
    error = float(np.max(abs(runtime-curve(dense_x))))
    polyline_error = float(np.max(abs(runtime-np.interp(dense_x, x, raw))))
    if error > .001 or np.min(np.diff(density)) < -1e-12:
        raise ValueError(f'unfiltered: runtime interpolation failed ({error})')
    stock = STOCK
    display = 'No filter'
    name = f'Kodak Polymax Fine-Art F / {display} (source curve)'
    profile = {
        'metadata': {'version': '0.3.4', 'created': '2026-11-06',
            'copyright': 'Copyright (c) 2026 SpektraFilm contributors (coordinate extraction and reconstruction).',
            'license': 'New contributions: CC BY-SA 4.0, https://creativecommons.org/licenses/by-sa/4.0/. Original Kodak technical data remain the property of their rights holder. See data/license/POLYMAX.txt.',
            'citation': 'Kodak, KODAK PROFESSIONAL POLYMAX Fine-Art Paper, G-24, June 2005, page 5.',
            'datasource': f'{source["primary_url"]}; SHA256 {source["primary_sha256"]}. PDF vector centerlines; source SHA256 {digest(source_path)}. F surface, DEKTOL 1:2, 90 seconds, 20 C; {display}. Exposure origin {reference:.12g} log10(lux seconds). D-min + 0.6 sensitivity. Rebuild scripts/prepare_polymax.py; see docs/polymax.md. ' + ' '.join(report['limitations'])},
        'info': {'stock': stock, 'name': name, 'type': 'negative', 'support': 'paper',
            'stage': 'printing', 'use': 'still', 'antihalation': 'no', 'target_print': None,
            'channel_model': 'bw', 'densitometer': 'reflection_visual',
            'log_sensitivity_density_over_min': .6, 'reference_illuminant': 'T'},
        'data': {'wavelengths': WL.tolist(), 'log_sensitivity': logs[:, None].tolist(),
            'channel_density': np.ones((len(WL), 1)).tolist(),
            'base_density': np.full((len(WL), 1), base).tolist(),
            'midscale_neutral_density': np.full(len(WL), float(curve(np.clip(reference, x[0], x[-1])))).tolist(),
            'log_exposure': (grid-reference).tolist(), 'density_curves': density[:, None].tolist(),
            'density_curves_layers': None, 'density_curves_model': None,
            'development_time': None, 'hanatos2025_adaptation_window_params': None,
            'hanatos2025_adaptation_surface_params': None}}
    payload = json.dumps(profile, indent=2, allow_nan=False)+'\n'
    profiles = {stock: payload}
    report['profiles'][stock] = {'filter_label': 'unfiltered', 'source_vertices': len(x),
        'runtime_samples': len(grid), 'base_density': base, 'maximum_density': float(y[-1]),
        'monotonic_repair_max_density': float(np.max(abs(y-raw))),
        'runtime_linear_vs_pchip_max_density': error,
        'runtime_linear_vs_pdf_polyline_max_density': polyline_error,
        'comparison_samples': len(dense_x), 'comparison_domain': [float(x[0]), float(x[-1])],
        'source_vertex_max_density': float(np.max(abs(np.interp(x, grid, total)-raw))),
        'profile_sha256': hashlib.sha256(payload.encode()).hexdigest()}
    return profiles, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data-dir', type=Path, default=ROOT/'data')
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    profiles, report = prepare(args.data_dir/'profile_sources/kodak_polymax_fine_art.json', args.data_dir)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for stock, payload in profiles.items():
        (args.output_dir/f'{stock}.json').write_text(payload)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, allow_nan=False)+'\n')
    print(json.dumps(report['profiles'], indent=2))


if __name__ == '__main__':
    main()
