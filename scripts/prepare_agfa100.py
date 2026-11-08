#!/usr/bin/env python3
"""Prepare one Agfachrome RSX II 100 source approximation; see docs/agfachrome100.md."""
import argparse
import hashlib
import json
from pathlib import Path

import colour
import numpy as np
import scipy
from scipy.interpolate import PchipInterpolator
from scipy.integrate import simpson
from scipy.optimize import brentq, isotonic_regression, least_squares

from extract_agfa100 import (GEOMETRY_HASH, PDF_HASH, STATUS_HASH, STATUS_TABLE_HASH,
                             X_CHAR, X_CHAR_VALUES, X_TICKS, X_VALUES, Y_CHAR, Y_CHAR_VALUES,
                             Y_DYE, Y_DYE_VALUES, Y_SPEC, Y_SPEC_VALUES, coordinates, digest,
                             fingerprint, require)
from prepare_velvia50 import calibrate_sensitivity, observer_and_illuminant

ROOT = Path(__file__).resolve().parents[1]
WL = np.arange(380., 781., 5.)
STOCK = 'agfachrome_rsx_ii_100'


def validated_source(path):
    source = json.loads(path.read_text())
    require(source['primary_sha256'] == PDF_HASH and source['status_a_sha256'] == STATUS_HASH,
            'unexpected source identity')
    require(source['stock'] == STOCK and source['geometry_sha256'] == GEOMETRY_HASH,
            'unexpected stock or geometry')
    require(fingerprint(source['status_a']) == STATUS_TABLE_HASH, 'Status A table changed')
    expected = coordinates(source['vector_paths'])
    require(source['coordinates'] == expected, 'coordinates differ from pinned vectors')
    return source


def resample(points, grid=WL, *, sensitivity=False):
    x, y = np.asarray(points).T
    require(np.isfinite(x).all() and np.isfinite(y).all() and np.all(np.diff(x) > 0),
            'invalid spectral coordinates')
    out = PchipInterpolator(x, y, extrapolate=False)(grid)
    for mask, edge, neighbor in ((grid < x[0], 0, 1), (grid > x[-1], -1, -2)):
        slope = (y[edge] - y[neighbor]) / (x[edge] - x[neighbor])
        slope = max(slope, 0.) if edge == 0 else min(slope, 0.)
        out[mask] = y[edge] + slope * (grid[mask] - x[edge])
    return np.maximum(out, -12. if sensitivity else 0.)


def status_weights(tables):
    fine = np.arange(380., 781.)
    hats = np.stack([np.interp(fine, WL, row) for row in np.eye(len(WL))], axis=1)
    weights = []
    for table in tables:
        x = np.array([float(k) for k in table])
        y = np.array(list(table.values()))
        log_w = np.interp(fine, x, y)
        for mask, edge, neighbor in ((fine < x[0], 0, 1), (fine > x[-1], -1, -2)):
            log_w[mask] = y[edge] + (y[edge] - y[neighbor]) / (x[edge] - x[neighbor]) * (fine[mask] - x[edge])
        response = 10 ** log_w
        projected = simpson(hats * response[:, None], x=fine, axis=0)
        weights.append(projected / projected.sum())
    return np.stack(weights, axis=1)


def measure(spectrum, weights):
    return -np.log10(10 ** (-spectrum) @ weights)


def reconstruct(source):
    values = source['coordinates']
    curves = [np.asarray(values['characteristic_' + name]) for name in ('green', 'red', 'blue')]
    low, high = max(c[0, 0] for c in curves), min(c[-1, 0] for c in curves)
    grid = np.linspace(low, high, 1025)
    target = np.stack([PchipInterpolator(c[:, 0], c[:, 1])(grid) for c in curves], axis=1)
    # RGB characteristic records in G/R/B order; convert to R/G/B before
    # applying the Status A measure.
    target = target[:, [1, 0, 2]]
    base = np.full(len(WL), float(target.min()))
    # Runtime channel_density is C/M/Y (cyan absorbs red, magenta green, yellow
    # blue); source plots are labelled Yellow/Magenta/Cyan. Keep the profile in
    # the runtime order and carry the same order through density_curves.
    dyes = np.stack([resample(values[k]) for k in ('cyan', 'magenta', 'yellow')], axis=1)
    weights = status_weights(source['status_a'])
    amounts, condition, p = [], [], np.ones(3)
    for row in target:
        fit = least_squares(lambda a: measure(base + dyes @ a, weights) - row,
                            p, bounds=(0, 10), ftol=1e-12, xtol=1e-12, gtol=1e-12)
        require(fit.success, 'nonlinear dye inversion failed')
        p = fit.x
        amounts.append(p.copy())
        condition.append(float(np.linalg.cond(fit.jac)))
    raw = np.array(amounts)
    amounts = np.stack([isotonic_regression(c, increasing=False).x for c in raw.T], axis=1)
    offset = amounts[-1].copy()
    base += dyes @ offset
    amounts -= offset
    predicted = measure(base + amounts @ dyes.T, weights)
    error = predicted - target
    require(np.max(abs(error)) < .02, 'density reconstruction exceeds 0.02 D engineering budget')
    reference = float(brentq(lambda h: np.interp(h, grid, predicted[:, 1])
                             - predicted[-1, 1] - 1., low, high))
    diagnostics = {
        'domain_log10_lux_seconds': [float(low), float(high)],
        'source_clear_end_status_a_rgb': target[-1].tolist(),
        'estimated_clear_end_status_a_rgb': measure(base, weights).tolist(),
        'neutral_base_prior': float(target.min()),
        'record_minimum_moved_to_base': offset.tolist(),
        'max_density_error_rgb': np.max(abs(error), axis=0).tolist(),
        'rmse_density_error_rgb': np.sqrt(np.mean(error ** 2, axis=0)).tolist(),
        'max_monotonicity_correction': np.max(abs(amounts + offset - raw), axis=0).tolist(),
        'max_inverse_jacobian_condition': max(condition),
        'reference_log10_lux_seconds': reference,
        'exposure_reference': 'Green approximate Status A clear-end + 1 D; not absolute ISO or output brightness.',
        'runtime_samples': len(grid),
    }
    return grid - reference, amounts, base, dyes, diagnostics


def prepare(source_path, data_dir):
    source = validated_source(source_path)
    grid, amounts, base, dyes, reconstruction = reconstruct(source)
    raw = 10 ** np.stack([resample(source['coordinates'][k], sensitivity=True)
                          for k in ('red', 'green', 'blue')], axis=1)
    cmfs, daylight = observer_and_illuminant(data_dir, 'D55')
    logs, window, surface, adaptation = calibrate_sensitivity(raw, data_dir, cmfs, daylight)
    require(np.isfinite(logs).all() and np.isfinite(surface).all(), 'nonfinite RGB calibration')
    limitations = source['limitations'] + [
        'Neutral clear-end prior plus residual dye minima is inferred, not a measured base spectrum.',
        'Status A uses legacy response tables, 1 nm log-linear interpolation and Simpson projection to the 5 nm transmission grid, not certified ISO 5-3 weights.',
        'Monotone dye amounts regularize small publication drawing reversals; no extrapolated exposure latitude.',
        'Sensitivity log-slope and dye linear tails are assumptions outside plotted support; outward growth is prevented.',
        'Each aggregate CMY record is a numerical effective dye amount, not a measured emulsion sublayer.',
        'Nonneutral exposure coupling and the fourth Agfa layer are not identified by the available one-dimensional plots.',
        'D55 is an implementation daylight convention; RGB midgray exposure normalization is not absolute ISO calibration.',
        'Input-adapter training and holdout measure spectral reconstruction, not matched physical film or scanner fidelity.',
        'No calibrated stock-specific grain, halation, DIR or MTF; builtin preset must leave unmeasured effects off or explicitly approximate.',
        'The 2850 K wedge spectrogram is printed as lg Sensitivity but no equal-energy conversion or source spectrum is supplied.',
    ]
    at_reference = np.array([np.interp(0., grid, amounts[:, c]) for c in range(3)])
    profile = {
        'info': {'stock': STOCK, 'name': 'Agfachrome RSX II 100',
                 'type': 'positive', 'support': 'film', 'stage': 'filming', 'use': 'still',
                 'antihalation': 'weak', 'target_print': None, 'channel_model': 'color',
                 'densitometer': 'status_A', 'log_sensitivity_density_over_min': 1.,
                 'reference_illuminant': 'D55', 'viewing_illuminant': 'D50'},
        'data': {'wavelengths': WL.tolist(), 'log_sensitivity': logs.tolist(),
                 'channel_density': dyes.tolist(), 'base_density': base.tolist(),
                 'midscale_neutral_density': (base + dyes @ at_reference).tolist(),
                 'log_exposure': grid.tolist(), 'density_curves': amounts.tolist(),
                 'density_curves_layers': amounts[:, None, :].tolist(),
                 'density_curves_model': None, 'development_time': None,
                 'hanatos2025_adaptation_window_params': window.tolist(),
                 'hanatos2025_adaptation_surface_params': surface.tolist()},
        'metadata': {'version': '0.3.4', 'created': '2026-11-08',
                     'copyright': 'Copyright (c) 2026 SpektraFilm contributors (extraction and reconstruction); Jan Lohse (Status A table transcription).',
                     'license': 'New contributions: CC BY-SA 4.0. Status A transcription: MIT, see JAN_LOHSE_MIT.txt. Manufacturer material remains Agfa property. See data/license/AGFA_RSX_100.txt.',
                     'citation': 'Agfa, Professional Films, F-PF-E4, fourth edition, August 2004, page 8; Agfachrome RSX II 100; Jan Lohse spectral_film_lut Status A table; Colour Science.',
                     'datasource': f'{source["primary_url"]}; PDF SHA256 {PDF_HASH}; source SHA256 {digest(source_path)}. {source["process"]} Rebuild scripts/prepare_agfa100.py; see docs/agfachrome100.md. ' + ' '.join(limitations)},
    }
    payload = json.dumps(profile, indent=2, allow_nan=False) + '\n'
    report = {'stock': STOCK, 'source_sha256': digest(source_path),
              'generator_sha256': digest(Path(__file__)),
              'extractor_sha256': digest(ROOT / 'scripts/extract_agfa100.py'),
              'shared_calibrator_sha256': digest(ROOT / 'scripts/prepare_velvia50.py'),
              'observer_sha256': digest(data_dir / 'illuminants/cmfs.json'),
              'illuminants_sha256': digest(data_dir / 'illuminants/catalog.json'),
              'versions': {'numpy': np.__version__, 'scipy': scipy.__version__, 'colour': colour.__version__},
              'profile_sha256': hashlib.sha256(payload.encode()).hexdigest(),
              'reconstruction': reconstruction, 'adaptation': adaptation,
              'limitations': limitations}
    return payload, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data-dir', type=Path, default=ROOT / 'data')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    payload, report = prepare(args.data_dir / 'profile_sources/agfachrome_rsx_ii_100.json', args.data_dir)
    for path, content in ((args.output, payload),
                          (args.report, json.dumps(report, indent=2, allow_nan=False) + '\n')):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
