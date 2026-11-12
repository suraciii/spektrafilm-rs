#!/usr/bin/env python3
"""Prepare one Ilford HP5 Plus 400 135 profile.

Baseline: ILFOTEC HC 1+31, 6.5 min, 20 C. The spectral ordinate is an explicit linear, zero-based interpretation of the Harman
wedge spectrogram, not a printed axis definition. No T-MAX/Acros coefficients are reused.
"""
import argparse
import hashlib
import json
from pathlib import Path

import colour
import numpy as np
import scipy
from scipy.interpolate import PchipInterpolator
from scipy.optimize import brentq

from prepare_velvia50 import calibrate_sensitivity, observer_and_illuminant

ROOT = Path(__file__).resolve().parents[1]
WL = np.arange(380., 781., 5.)
STOCK = 'ilford_hp5_plus'
PDF_HASH = 'b2006ecb9f96b1645726a90a7f15e798bdb7948b4ae1d0e6e9fa9719c93925f2'
GEOMETRY_HASH = 'e1019c98aac0cd66e8857b663cf72d7f8476050dd06d3040c766e83ce43716f4'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(source_path, data_dir):
    source = json.loads(source_path.read_text())
    if source.get('stock') != STOCK or source.get('primary_sha256') != PDF_HASH:
        raise ValueError('unexpected source stock or PDF identity')
    if source.get('extraction', {}).get('geometry_sha256') != GEOMETRY_HASH:
        raise ValueError('unexpected source geometry')
    characteristic = np.asarray(source['characteristic'], dtype=float)
    sensitivity = np.asarray(source['relative_sensitivity'], dtype=float)
    if not (np.isfinite(characteristic).all() and np.isfinite(sensitivity).all()):
        raise ValueError('source contains non-finite values')
    x, total = characteristic.T
    if not (np.all(np.diff(x) > 0) and np.all(np.diff(total) >= 0)):
        raise ValueError('characteristic must have increasing exposure and monotone density')
    sx, sy = sensitivity.T
    if not np.all(np.diff(sx) > 0):
        raise ValueError('sensitivity wavelength must increase')
    curve = PchipInterpolator(x, total, extrapolate=False)
    base = float(total[0])
    reference = float(brentq(lambda h: curve(h) - base - 1., x[0], x[-1]))
    grid = np.unique(np.r_[np.linspace(x[0], x[-1], 1025), x, reference])
    density = curve(grid) - base
    if np.any(density < -1e-12):
        raise ValueError('negative characteristic density')
    relative = PchipInterpolator(sx, sy, extrapolate=False)(WL)
    for mask, edge, neighbor in ((WL < sx[0], 0, 1), (WL > sx[-1], -1, -2)):
        slope = (sy[edge] - sy[neighbor]) / (sx[edge] - sx[neighbor])
        relative[mask] = sy[edge] + slope * (WL[mask] - sx[edge])
    relative = np.maximum(relative, 1e-12)
    cmfs, daylight = observer_and_illuminant(data_dir, 'D55')
    normalized, window, surface, adaptation = calibrate_sensitivity(
        relative[:, None], data_dir, cmfs, daylight)
    surface = np.repeat(surface, 3, axis=0)
    dense = PchipInterpolator(x, total)(np.linspace(x[0], x[-1], 4096))
    reconstruction = float(np.max(abs(np.interp(np.linspace(x[0], x[-1], 4096), grid, density) + base - dense)))
    limitations = list(dict.fromkeys(source['limitations'] + [
        'No still-film RMS, MTF or resolving-power figure is published on the selected HP5 sheet.',
        'Exposure zero is the first plotted density plus 1.0; that density is not a measured D-min.',
        'The characteristic curve clamps outside the plotted range; no inferred shoulder or extra latitude.',
        'Real scans are qualitative only and cannot establish scanner-independent physical fidelity.',
    ]))
    profile = {
        'metadata': {
            'version': '0.3.4', 'created': '2026-11-08',
            'copyright': 'Copyright (c) 2026 SpektraFilm contributors (coordinate extraction and reconstruction).',
            'license': 'New contributions: CC BY-SA 4.0, https://creativecommons.org/licenses/by-sa/4.0/. Ilford/Harman technical material remains the property of its rights holder. See data/license/HP5_PLUS.txt.',
            'citation': 'Ilford, ILFORD HP5 PLUS Technical Information, November 2018, page 5 characteristic curve and page 1 wedge spectrogram; ILFOTEC HC 1+31, 6.5 min, 20 C.',
            'datasource': f'{source["primary_url"]}; PDF SHA256 {source["primary_sha256"]}; source SHA256 {digest(source_path)}. {source["process"]} Source-relative log-exposure reference {reference:.12g} on the p5 Relative log exposure axis; this is not an absolute lux-seconds origin. Rebuild scripts/prepare_hp5.py; see docs/hp5plus.md. ' + ' '.join(limitations)
        },
        'info': {
            'stock': STOCK, 'name': 'Ilford HP5 Plus 400 (ILFOTEC HC 1+31 6.5 min)',
            'type': 'negative', 'support': 'film', 'stage': 'filming', 'use': 'still',
            'antihalation': 'weak', 'target_print': None, 'channel_model': 'bw',
            'densitometer': 'diffuse_visual', 'log_sensitivity_density_over_min': 1.0,
            'reference_illuminant': 'D55'
        },
        'data': {
            'wavelengths': WL.tolist(), 'log_sensitivity': normalized[:, :1].tolist(),
            'channel_density': np.ones((len(WL), 1)).tolist(),
            'base_density': np.full((len(WL), 1), base).tolist(),
            'midscale_neutral_density': np.full(len(WL), base + 1.).tolist(),
            'log_exposure': (grid - reference).tolist(), 'density_curves': density[:, None].tolist(),
            'density_curves_layers': density[:, None, None].tolist(),
            'density_curves_model': None, 'development_time': [6.5],
            'hanatos2025_adaptation_window_params': window.tolist(),
            'hanatos2025_adaptation_surface_params': surface.tolist()
        }
    }
    payload = json.dumps(profile, indent=2, allow_nan=False) + '\n'
    report = {
        'stock': STOCK, 'source_sha256': digest(source_path), 'generator_sha256': digest(Path(__file__)),
        'shared_calibrator_sha256': digest(ROOT / 'scripts/prepare_velvia50.py'),
        'extractor_sha256': digest(ROOT / 'scripts/extract_hp5.py'),
        'cmfs_sha256': digest(data_dir / 'illuminants/cmfs.json'),
        'illuminants_sha256': digest(data_dir / 'illuminants/catalog.json'),
        'versions': {'numpy': np.__version__, 'scipy': scipy.__version__, 'colour': colour.__version__},
        'profile_sha256': hashlib.sha256(payload.encode()).hexdigest(),
        'reference_source_log_exposure': reference, 'estimated_base_density': base,
        'source_domain': [float(x[0]), float(x[-1])], 'runtime_samples': len(grid),
        'runtime_vs_source_max_density': reconstruction,
        'spectral_baseline': 'linear normalized wedge sensitivity, explicit interpretation',
        'spectral_axis': 'Harman prints Sensitivity 0.5/1.0 without a log label; linear zero-based reading is an explicit assumption',
        'adaptation': adaptation, 'limitations': limitations
    }
    return payload, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    data_dir = ROOT / 'data'
    payload, report = prepare(args.source, data_dir)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(payload)
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ['reference_source_log_exposure', 'estimated_base_density', 'runtime_vs_source_max_density', 'adaptation']}, indent=2))


if __name__ == '__main__':
    main()
