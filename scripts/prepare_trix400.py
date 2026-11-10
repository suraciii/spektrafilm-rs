#!/usr/bin/env python3
"""Prepare one 400TX 35mm/T-MAX six-minute profile; see docs/trix400.md."""
import argparse
import hashlib
import json
from pathlib import Path

import colour
import numpy as np
import scipy
from scipy.interpolate import PchipInterpolator
from scipy.optimize import brentq

# Reuse the existing engine-matched RGB spectral calibration, not another stock's
# fitted values. All three targets are the same effective silver record.
from prepare_velvia50 import calibrate_sensitivity, observer_and_illuminant
from extract_trix400 import sample_path, map_axes, PDF_HASH, GEOMETRY_HASH

ROOT = Path(__file__).resolve().parents[1]
WL = np.arange(380., 781., 5.)
STOCK = 'kodak_trix_400'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(source_path, data_dir):
    source = json.loads(source_path.read_text())
    geometry = hashlib.sha256(json.dumps(source['vector_paths'], sort_keys=True,
                                         separators=(',', ':')).encode()).hexdigest()
    if source['primary_sha256'] != PDF_HASH or geometry != GEOMETRY_HASH:
        raise ValueError('unexpected source PDF identity or labelled geometry')
    for label, values in [('characteristic', source['characteristic']),
                          *[(f'sensitivity_{d}', source['log_sensitivity'][d])
                            for d in ('0.3', '1.0')]]:
        expected = map_axes(sample_path(source['vector_paths'][label]),
                            'characteristic' if label == 'characteristic' else 'sensitivity')
        if not np.array_equal(np.asarray(values), expected):
            raise ValueError(f'source coordinates differ from pinned vector: {label}')
    x, total = np.array(source['characteristic']).T
    if not (np.isfinite(x).all() and np.isfinite(total).all() and
            np.all(np.diff(x) > 0) and np.all(np.diff(total) >= 0)):
        raise ValueError('invalid monotone source characteristic')
    curve = PchipInterpolator(x, total, extrapolate=False)
    base = float(total[0])
    reference = float(brentq(lambda h: curve(h)-base-1., x[0], x[-1]))
    # Endpoints clamp: no invented shoulder/extra highlight latitude beyond graph.
    grid = np.unique(np.r_[np.linspace(x[0], x[-1], 1025), x, reference])
    density = curve(grid)-base
    sx, sy = np.array(source['log_sensitivity']['1.0']).T
    if not (np.isfinite(sx).all() and np.isfinite(sy).all() and np.all(np.diff(sx) > 0)):
        raise ValueError('invalid source sensitivity')
    logs = PchipInterpolator(sx, sy, extrapolate=False)(WL)
    for mask, edge, neighbor in ((WL < sx[0], 0, 1), (WL > sx[-1], -1, -2)):
        slope = (sy[edge]-sy[neighbor])/(sx[edge]-sx[neighbor])
        logs[mask] = sy[edge]+slope*(WL[mask]-sx[edge])
    logs = np.maximum(logs, -12.)
    cmfs, daylight = observer_and_illuminant(data_dir, 'D55')
    normalized, window, surface, adaptation = calibrate_sensitivity(
        (10.**logs)[:, None], data_dir, cmfs, daylight)
    # Fit one effective record and broadcast exactly. Separate identical target
    # fits are needlessly underdetermined and can acquire channel solver noise.
    surface = np.repeat(surface, 3, axis=0)
    dense = map_axes(sample_path(source['vector_paths']['characteristic'], 4096), 'characteristic')
    error = float(np.max(abs(np.interp(dense[:, 0], grid, density)+base-dense[:, 1])))
    vertex_error = float(np.max(abs(np.interp(x, grid, density)+base-total)))
    if error > .001 or vertex_error > 1e-10:
        raise ValueError(f'publication reconstruction budget exceeded: {error}, {vertex_error}')
    limitations = source['limitations'] + [
        'Published upper solid sensitivity labelled D=1.0; threshold ordering is unresolved, labels not silently corrected.',
        '2007 Kodak edition attributes sensitivity/MTF to older 5063; later omission of caveat does not prove remeasurement.',
        'Endpoint log-slope spectral tails to -12 are extrapolated, not measured hard cutoffs.',
        'Exposure zero is estimated base+fog plus 1.0 density; D55 RGB midgray mapping is not absolute ISO calibration.',
        'Neutral silver and base spectra; no angular scattering, fluorescence or real scanner calibration.',
        'No real Tri-X photographs calibrate the RGB adaptation; CIE training and BabelColor holdout test only spectral reconstruction.',
        'Characteristic curve clamps outside plotted range; no inferred shoulder or extra latitude.',
        'One aggregate density record, not a measured layer decomposition.',
        'No stock-specific grain, halation or MTF calibration; builtin preset uses class-level priors, not measured effects.']
    profile = {
        'metadata': {'version': '0.3.4', 'created': '2026-11-07',
            'copyright': 'Copyright (c) 2026 SpektraFilm contributors (coordinate extraction and reconstruction).',
            'license': 'New contributions: CC BY-SA 4.0, https://creativecommons.org/licenses/by-sa/4.0/. Kodak technical material remains the property of its rights holder. See data/license/TRIX_400.txt.',
            'citation': 'Kodak, KODAK PROFESSIONAL TRI-X 320 and 400 Films, F-4017, October 2021, page 8; process table page 3.',
            'datasource': f'{source["primary_url"]}; PDF SHA256 {source["primary_sha256"]}; source SHA256 {digest(source_path)}. {source["process"]} Exposure origin {reference:.12g} log10(lux seconds). Rebuild scripts/prepare_trix400.py; see docs/trix400.md. '+' '.join(limitations)},
        'info': {'stock': STOCK, 'name': 'Kodak Tri-X 400 / 400TX (T-MAX 6 min)',
            'type': 'negative', 'support': 'film', 'stage': 'filming', 'use': 'still',
            'antihalation': 'strong', 'target_print': None, 'channel_model': 'bw',
            'densitometer': 'diffuse_visual', 'log_sensitivity_density_over_min': 1.0,
            'reference_illuminant': 'D55'},
        'data': {'wavelengths': WL.tolist(), 'log_sensitivity': normalized[:, :1].tolist(),
            'channel_density': np.ones((len(WL), 1)).tolist(),
            'base_density': np.full((len(WL), 1), base).tolist(),
            'midscale_neutral_density': np.full(len(WL), base+1.).tolist(),
            'log_exposure': (grid-reference).tolist(), 'density_curves': density[:, None].tolist(),
            # One aggregate image record satisfies existing grain consumers; it
            # is not a decomposition into measured emulsion sublayers.
            'density_curves_layers': density[:, None, None].tolist(),
            'density_curves_model': None, 'development_time': [6.],
            'hanatos2025_adaptation_window_params': window.tolist(),
            'hanatos2025_adaptation_surface_params': surface.tolist()}}
    payload = json.dumps(profile, indent=2, allow_nan=False)+'\n'
    report = {'stock': STOCK, 'source_sha256': digest(source_path),
              'generator_sha256': digest(Path(__file__)),
              'shared_calibrator_sha256': digest(ROOT/'scripts/prepare_velvia50.py'),
              'extractor_and_axis_mapping_sha256': digest(ROOT/'scripts/extract_trix400.py'),
              'cmfs_sha256': digest(data_dir/'illuminants/cmfs.json'),
              'illuminants_sha256': digest(data_dir/'illuminants/catalog.json'),
              'versions': {'numpy': np.__version__, 'scipy': scipy.__version__, 'colour': colour.__version__},
              'profile_sha256': hashlib.sha256(payload.encode()).hexdigest(),
              'reference_log10_lux_seconds': reference, 'estimated_base_plus_fog': base,
              'source_domain': [float(x[0]), float(x[-1])], 'runtime_samples': len(grid),
              'runtime_vs_vector_max_density': error, 'source_point_max_density': vertex_error,
              'reference_sampling': '4096 subdivisions per source cubic; all source line endpoints',
              'adaptation': adaptation, 'limitations': limitations}
    return payload, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data-dir', type=Path, default=ROOT/'data')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    payload, report = prepare(args.data_dir/'profile_sources/kodak_trix_400.json', args.data_dir)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(payload)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, allow_nan=False)+'\n')
    print(json.dumps({k: report[k] for k in ['reference_log10_lux_seconds', 'runtime_vs_vector_max_density', 'adaptation']}, indent=2))


if __name__ == '__main__':
    main()
