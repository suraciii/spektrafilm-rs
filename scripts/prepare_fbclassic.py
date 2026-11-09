#!/usr/bin/env python3
"""Prepare the fixed grade-2 ILFORD MULTIGRADE FB CLASSIC Glossy profile.

The source is the Oct 2013 HARMAN/ILFORD technical sheet's page-2 vector
characteristic curve. Only the filter-2/grade-2 curve is shipped. The page-1
spectral chart has no numeric ordinate, so the profile carries a flat neutral
spectral placeholder and must not be described as a measured spectrum.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np
from scipy.interpolate import PchipInterpolator

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'data/profile_sources/ilford_multigrade_fb_classic_glossy_g2.json'
PDF_SHA = 'b4fe0ea11aad13ef4210c59bb42f49bc121ea213cf1a9ee96faf35832473dd43'
STOCK = 'ilford_multigrade_fb_classic_glossy_g2'
WL = np.arange(380., 781., 5.)
BASE_REFLECTANCE = .184
MONOTONE_TOL = 1e-12


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def monotone(values):
    """Equal-weight isotonic projection, used only for drawing reversals."""
    blocks = []
    for value in values:
        blocks.append([float(value), 1])
        while len(blocks) > 1 and blocks[-2][0] > blocks[-1][0]:
            b, a = blocks.pop(), blocks.pop()
            n = a[1] + b[1]
            blocks.append([(a[0] * a[1] + b[0] * b[1]) / n, n])
    return np.concatenate([np.full(n, v) for v, n in blocks])


def prepare(source_path, output_dir):
    source = json.loads(source_path.read_text())
    if source['primary_sha256'] != PDF_SHA:
        raise ValueError('source record does not pin the expected PDF')
    points = np.asarray(source['characteristic']['2'], dtype=float)
    if points.ndim != 2 or points.shape[1] != 2 or len(points) < 10:
        raise ValueError('invalid grade-2 points')
    x, raw = points[:, 0], points[:, 1]
    if not np.all(np.diff(x) > 0):
        raise ValueError('grade-2 x values must be strictly increasing')
    y = monotone(raw)
    repair = float(np.max(np.abs(y - raw)))
    curve = PchipInterpolator(x, y, extrapolate=False)
    base = max(float(y[0]), 0.0)
    density = np.maximum(y - base, 0.0)
    reference_density = float(-math.log10(BASE_REFLECTANCE))
    reference = float(np.interp(reference_density, density, x))
    grid = np.unique(np.r_[np.linspace(x[0], x[-1], 1001), x])
    total = curve(grid)
    profile_density = np.maximum(total - base, 0.0)
    if not np.all(np.diff(profile_density) >= -MONOTONE_TOL):
        raise ValueError('profile density curve is not monotone')
    if not np.isfinite(reference):
        raise ValueError('reference exposure is not finite')
    dense = np.linspace(x[0], x[-1], 200001)
    runtime = np.interp(dense, grid, total)
    runtime_error = float(np.max(np.abs(runtime - curve(dense))))
    vertex_error = float(np.max(np.abs(np.interp(x, grid, total) - raw)))
    if runtime_error > 1e-3 or vertex_error > 1e-4:
        raise ValueError(f'curve round-trip failed: runtime={runtime_error}, vertex={vertex_error}')

    limitations = source['limitations']
    datasource = (
        f"{source['primary_url']}; SHA256 {PDF_SHA}; source record {digest(source_path)}. "
        f"Glossy 1K; filter 2 / grade 2; MULTIGRADE 1+9; 2 min at 20 C. "
        'PDF vector centerline; plotting coordinates, not measured samples. '
        f'Exposure origin is the paper-white-relative 18% reflectance convention at source logE {reference:.12g}; '
        'the plotted x-axis is not calibrated to absolute lux-seconds and ISO P230 is not used as the origin. '
        'Flat 0.0 log-sensitivity is a placeholder: page 1 has no numeric ordinate and no grade-specific curve. '
        'No measured base spectrum, Dmax, batch, fluorescence, BRDF or scanner match. '
        'One grade only; no 00-5 variable-contrast prediction.'
    )
    profile = {
        'metadata': {
            'version': '0.3.4',
            'created': '2026-11-09',
            'copyright': 'Copyright (c) 2026 SpektraFilm contributors (coordinate extraction and reconstruction).',
            'license': 'New contributions: CC BY-SA 4.0, https://creativecommons.org/licenses/by-sa/4.0/. Original ILFORD technical data remain the property of their rights holder. See data/license/ILFORD_MULTIGRADE_FB_CLASSIC.txt.',
            'citation': 'HARMAN/ILFORD, MULTIGRADE FB CLASSIC Technical Information, Oct 2013, page 2 (filter 2 / grade 2 curve).',
            'datasource': datasource,
        },
        'info': {
            'stock': STOCK,
            'name': 'Ilford MULTIGRADE FB CLASSIC Glossy / Filter 2 (grade 2) (source curve)',
            'type': 'negative',
            'support': 'paper',
            'stage': 'printing',
            'use': 'still',
            'antihalation': 'no',
            'target_print': None,
            'channel_model': 'bw',
            'densitometer': 'reflection_visual',
            'log_sensitivity_density_over_min': .6,
            'reference_illuminant': 'T',
        },
        'data': {
            'wavelengths': WL.tolist(),
            'log_sensitivity': np.zeros((len(WL), 1)).tolist(),
            'channel_density': np.ones((len(WL), 1)).tolist(),
            'base_density': np.full((len(WL), 1), base).tolist(),
            'midscale_neutral_density': np.full(len(WL), reference_density).tolist(),
            'log_exposure': (grid - reference).tolist(),
            'density_curves': profile_density[:, None].tolist(),
            'density_curves_layers': None,
            'density_curves_model': None,
            'development_time': None,
            'hanatos2025_adaptation_window_params': None,
            'hanatos2025_adaptation_surface_params': None,
        },
    }
    payload = json.dumps(profile, indent=2, allow_nan=False) + '\n'
    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / f'{STOCK}.json').write_text(payload)
    report = {
        'source_sha256': digest(source_path),
        'source_pdf_sha256': PDF_SHA,
        'generator_sha256': digest(Path(__file__)),
        'profile_sha256': hashlib.sha256(payload.encode()).hexdigest(),
        'source_vertices': len(x),
        'base_density_proxy': base,
        'reference_reflection_density': reference_density,
        'reference_source_loge': reference,
        'profile_loge_range': [float((grid - reference)[0]), float((grid - reference)[-1])],
        'density_range': [float(profile_density.min()), float(profile_density.max())],
        'monotone_repair_max_density': repair,
        'runtime_linear_vs_pchip_max_density': runtime_error,
        'runtime_linear_vs_source_vertex_max_density': vertex_error,
        'flat_spectrum_placeholder': True,
        'limitations': limitations,
    }
    return {STOCK: payload}, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=SOURCE)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    profiles, report = prepare(args.source, args.output_dir)
    for stock, payload in profiles.items():
        (args.output_dir / f'{stock}.json').write_text(payload)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, allow_nan=False) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
