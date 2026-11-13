#!/usr/bin/env python3
"""Prepare one Kodak T-MAX 100 / 100TMX 135 D-76 7.5-minute profile.

The F-4016 p8 curves are vector; the characteristic curve is the printed
D-76 20 C 7.5-minute family member, and the spectral baseline is the published
0.3-above-D-min curve. The 1.0 curve is retained for ablation only. No Tri-X or
Acros coefficients are reused.
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
STOCK = 'kodak_tmax_100'
PDF_HASH = '00518d7d1e296d6065f6e41ea3db999e5d8714679fb3d2b76d9a49eb207113be'
GEOMETRY_HASH = 'becde9deead3a7ed67d13488c1fd2d764579060e36ee182b7bf2ddca2a20bda3'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(source_path, data_dir):
    source = json.loads(source_path.read_text())
    if source.get('stock') != STOCK or source.get('primary_sha256') != PDF_HASH:
        raise ValueError('unexpected source stock or PDF identity')
    if source.get('extraction', {}).get('geometry_sha256') != GEOMETRY_HASH:
        raise ValueError('unexpected source geometry')
    characteristic = np.asarray(source['characteristic'], dtype=float)
    sensitivity = np.asarray(source['log_sensitivity']['0.3'], dtype=float)
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
    # Endpoint clamping: do not invent extra latitude outside the published plot.
    grid = np.unique(np.r_[np.linspace(x[0], x[-1], 1025), x, reference])
    density = curve(grid) - base
    if np.any(density < -1e-12):
        raise ValueError('negative characteristic density')
    logs = PchipInterpolator(sx, sy, extrapolate=False)(WL)
    for mask, edge, neighbor in ((WL < sx[0], 0, 1), (WL > sx[-1], -1, -2)):
        slope = (sy[edge] - sy[neighbor]) / (sx[edge] - sx[neighbor])
        logs[mask] = sy[edge] + slope * (WL[mask] - sx[edge])
    logs = np.maximum(logs, -12.)
    cmfs, daylight = observer_and_illuminant(data_dir, 'D55')
    normalized, window, surface, adaptation = calibrate_sensitivity(
        (10. ** logs)[:, None], data_dir, cmfs, daylight)
    surface = np.repeat(surface, 3, axis=0)
    dense = PchipInterpolator(x, total)(np.linspace(x[0], x[-1], 4096))
    reconstruction = float(np.max(abs(np.interp(np.linspace(x[0], x[-1], 4096), grid, density) + base - dense)))
    limitations = list(dict.fromkeys(source['limitations'] + [
        'The selected 7.5-minute p8 curve is not the p3 table value (6.5 minutes at 68 F); the table is a starting-point recommendation and was not silently substituted.',
        'The 0.3-above-D-min spectral curve is the baseline; the 1.0 curve is retained for ablation, not treated as a second measured layer.',
        'The spectral log offset is normalized by the existing engine-matched adapter; the absolute sensitivity scale is not an ISO calibration.',
        'Spectral tails outside the plotted domain are endpoint-slope assumptions, not measured cutoffs.',
        'One aggregate silver record is used; this is not a measured emulsion sublayer decomposition.',
        'Exposure zero is the first plotted density plus 1.0; 0.213071 is not a measured D-min, and D55 RGB midgray mapping is not absolute ISO or output-brightness calibration.',
        'No measured base spectrum, silver grain, halation PSF or scanner response is supplied by the publication.',
        'The antihalation metadata tag is a conservative weak default; the sheet does not publish a halation PSF.',
        'The preset grain is an approximation based on RMS 8 at 48 micrometers and D-min+1.0; it is not a per-channel grain calibration.',
        'Real scans are used for qualitative review only and cannot identify a scanner-independent material response.',
        'The characteristic curve clamps outside the plotted range; no inferred shoulder or extra latitude.']))
    profile = {
        'metadata': {
            'version': '0.3.4', 'created': '2026-11-08',
            'copyright': 'Copyright (c) 2026 SpektraFilm contributors (coordinate extraction and reconstruction).',
            'license': 'New contributions: CC BY-SA 4.0, https://creativecommons.org/licenses/by-sa/4.0/. Kodak technical material remains the property of its rights holder. See data/license/TMAX_100.txt.',
            'citation': 'Kodak, KODAK PROFESSIONAL T-MAX 100 Film, F-4016, June 2018, page 8; D-76 20 C 7.5-minute characteristic curve and 0.3-above-D-min log-sensitivity curve.',
            'datasource': f'{source["primary_url"]}; PDF SHA256 {source["primary_sha256"]}; source SHA256 {digest(source_path)}. {source["process"]} Exposure origin {reference:.12g} log10(lux seconds). Rebuild scripts/prepare_tmax100.py; see docs/tmax100.md. ' + ' '.join(limitations)
        },
        'info': {
            'stock': STOCK, 'name': 'Kodak T-MAX 100 / 100TMX 135 (D-76 7.5 min)',
            'type': 'negative', 'support': 'film', 'stage': 'filming', 'use': 'still',
            'antihalation': 'weak', 'target_print': None, 'channel_model': 'bw',
            'densitometer': 'diffuse_visual', 'log_sensitivity_density_over_min': 0.3,
            'reference_illuminant': 'D55'
        },
        'data': {
            'wavelengths': WL.tolist(), 'log_sensitivity': normalized[:, :1].tolist(),
            'channel_density': np.ones((len(WL), 1)).tolist(),
            'base_density': np.full((len(WL), 1), base).tolist(),
            'midscale_neutral_density': np.full(len(WL), base + 1.).tolist(),
            'log_exposure': (grid - reference).tolist(), 'density_curves': density[:, None].tolist(),
            'density_curves_layers': density[:, None, None].tolist(),
            'density_curves_model': None, 'development_time': [7.5],
            'hanatos2025_adaptation_window_params': window.tolist(),
            'hanatos2025_adaptation_surface_params': surface.tolist()
        }
    }
    payload = json.dumps(profile, indent=2, allow_nan=False) + '\n'
    report = {
        'stock': STOCK, 'source_sha256': digest(source_path), 'generator_sha256': digest(Path(__file__)),
        'shared_calibrator_sha256': digest(ROOT / 'scripts/prepare_velvia50.py'),
        'extractor_sha256': digest(ROOT / 'scripts/extract_tmax100.py'),
        'cmfs_sha256': digest(data_dir / 'illuminants/cmfs.json'),
        'illuminants_sha256': digest(data_dir / 'illuminants/catalog.json'),
        'versions': {'numpy': np.__version__, 'scipy': scipy.__version__, 'colour': colour.__version__},
        'profile_sha256': hashlib.sha256(payload.encode()).hexdigest(),
        'reference_log10_lux_seconds': reference, 'estimated_base_density': base,
        'source_domain': [float(x[0]), float(x[-1])], 'runtime_samples': len(grid),
        'runtime_vs_source_max_density': reconstruction,
        'spectral_baseline': '0.3 above D-min', 'spectral_ablation': '1.0 above D-min retained in source',
        'adaptation': adaptation, 'limitations': limitations
    }
    return payload, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--data-dir', type=Path, default=ROOT / 'data')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    payload, report = prepare(args.source, args.data_dir)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(payload)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, allow_nan=False) + '\n')
    print(json.dumps({k: report[k] for k in ['reference_log10_lux_seconds', 'estimated_base_density', 'runtime_vs_source_max_density', 'adaptation']}, indent=2))


if __name__ == '__main__':
    main()
