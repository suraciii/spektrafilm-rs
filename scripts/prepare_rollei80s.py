#!/usr/bin/env python3
"""Prepare the Rollei Retro 80S 35 mm / Neofin Blau profile.

The characteristic curve is a 10-zone third-party densitometry set, not a
manufacturer plot. The spectral curve is the 2021 Rollei page-3 vector path.
No 120 or ID-11 claim is made.
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
STOCK = 'rollei_retro_80s_35mm_neofin_blau'
SOURCE_HTML_HASH = '8da29c06a10b430d4c70534550fb96a6ac242b3c8f90259e46f9e07fa68137c9'
SPECTRAL_PDF_HASH = 'f37754ce4548ceab6e6902143118cea712d5bf4f1dee32488bfa37dfb02e0f45'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(source_path, data_dir):
    source = json.loads(source_path.read_text())
    if source.get('stock') != STOCK:
        raise ValueError('unexpected source stock')
    if source.get('primary_sha256') != SOURCE_HTML_HASH:
        raise ValueError('unexpected APHOG page identity')
    if source.get('spectral_sha256') != SPECTRAL_PDF_HASH:
        raise ValueError('unexpected Rollei PDF identity')

    characteristic = np.asarray(source['characteristic'], dtype=float)
    sensitivity = np.asarray(source['spectral_points'], dtype=float)
    if not (np.isfinite(characteristic).all() and np.isfinite(sensitivity).all()):
        raise ValueError('source contains non-finite values')
    x, total = characteristic.T
    if not (np.all(np.diff(x) > 0) and np.all(np.diff(total) >= 0)):
        raise ValueError('characteristic must have increasing exposure and monotone density')
    sx, sy = sensitivity.T
    if not np.all(np.diff(sx) > 0):
        raise ValueError('sensitivity wavelength must increase')
    if not (sx[0] <= WL[0] and sx[-1] >= WL[-1]):
        raise ValueError('spectral coverage does not span the runtime grid')

    curve = PchipInterpolator(x, total, extrapolate=False)
    base = float(total[0])
    reference = float(brentq(lambda h: curve(h) - base - 1., x[0], x[-1]))
    grid = np.unique(np.r_[np.linspace(x[0], x[-1], 1025), x, reference])
    density = curve(grid) - base
    if np.any(density < -1e-12):
        raise ValueError('negative characteristic density')

    # The Rollei plot is a relative log-sensitivity ordinate with an arbitrary
    # zero. A constant offset is removed before linearisation; the shared
    # calibrator then normalises the spectral shape.
    log_s = PchipInterpolator(sx, sy, extrapolate=False)(WL)
    relative = 10.0 ** (log_s - np.max(log_s))
    relative = np.maximum(relative, 1e-12)
    cmfs, daylight = observer_and_illuminant(data_dir, 'D55')
    normalized, window, surface, adaptation = calibrate_sensitivity(
        relative[:, None], data_dir, cmfs, daylight)
    surface = np.repeat(surface, 3, axis=0)

    dense = PchipInterpolator(x, total)(np.linspace(x[0], x[-1], 4096))
    reconstruction = float(np.max(abs(
        np.interp(np.linspace(x[0], x[-1], 4096), grid, density) + base - dense)))

    limitations = list(dict.fromkeys(source['limitations'] + [
        'No 120-specific or ID-11/D-76 curve is available; this profile is 35 mm + Neofin Blau only.',
        'The first measured zone is a base-density proxy; no measured D-min is supplied.',
        'The toe below Zone I and shoulder above Zone X are clamped, not inferred.',
        'The spectral ordinate is relative, not absolute; the ISO anchor is model calibration.',
        'The Wallner MC 505 reading is relative logD, not a calibrated diffuse density measurement.',
        'The built-in grain and halation are class-level effects, not measured RMS/PSF.',
    ]))
    profile = {
        'metadata': {
            'version': '0.3.4', 'created': '2026-11-09',
            'copyright': 'Copyright (c) 2026 SpektraFilm contributors (third-party curve transcription and reconstruction).',
            'license': 'New contributions: CC BY-SA 4.0, https://creativecommons.org/licenses/by-sa/4.0/. Rollei/Maco technical material and the APHOG measurements remain the property of their respective holders. See data/license/ROLLEI_RETRO_80S.txt.',
            'citation': 'Rollei Retro 80S data sheet, 2021-01-15, page 3 spectral sensitivity; APHOG forum post #40-41 (2011-06-05), 10-zone Neofin Blau densitometry. Process: Tetenal Neofin Blau, one flask to 500 ml, 6 min at 20 C; EI 80/20.',
            'datasource': f'{source["primary_url"]}; archived HTML SHA256 {source["primary_sha256"]}; {source["spectral_url"]}; PDF SHA256 {source["spectral_sha256"]}; source SHA256 {digest(source_path)}. Source-relative log-exposure reference {reference:.12g} on the Zone I-X axis; this is not an absolute lux-seconds origin. Rebuild scripts/prepare_rollei80s.py; see docs/rollei80s.md. ' + ' '.join(limitations)
        },
        'info': {
            'stock': STOCK,
            'name': 'Rollei Retro 80S 35mm (Neofin Blau 6 min, EI 80)',
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
            'log_exposure': (grid - reference).tolist(),
            'density_curves': density[:, None].tolist(),
            'density_curves_layers': density[:, None, None].tolist(),
            'density_curves_model': None, 'development_time': [6.0],
            'hanatos2025_adaptation_window_params': window.tolist(),
            'hanatos2025_adaptation_surface_params': surface.tolist()
        }
    }
    payload = json.dumps(profile, indent=2, allow_nan=False) + '\n'
    report = {
        'stock': STOCK, 'source_sha256': digest(source_path), 'generator_sha256': digest(Path(__file__)),
        'shared_calibrator_sha256': digest(ROOT / 'scripts/prepare_velvia50.py'),
        'source_html_sha256': source['primary_sha256'], 'spectral_pdf_sha256': source['spectral_sha256'],
        'cmfs_sha256': digest(data_dir / 'illuminants/cmfs.json'),
        'illuminants_sha256': digest(data_dir / 'illuminants/catalog.json'),
        'versions': {'numpy': np.__version__, 'scipy': scipy.__version__, 'colour': colour.__version__},
        'profile_sha256': hashlib.sha256(payload.encode()).hexdigest(),
        'reference_source_log_exposure': reference, 'estimated_base_density': base,
        'source_domain': [float(x[0]), float(x[-1])], 'runtime_samples': len(grid),
        'runtime_vs_source_max_density': reconstruction,
        'spectral_baseline': 'Rollei page-3 relative log sensitivity, arbitrary zero',
        'spectral_axis': source['spectral_axis'],
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
    print(json.dumps({k: report[k] for k in [
        'reference_source_log_exposure', 'estimated_base_density',
        'runtime_vs_source_max_density', 'adaptation']}, indent=2))


if __name__ == '__main__':
    main()
