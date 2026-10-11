#!/usr/bin/env python3
"""Prepare the source-backed Velvia 50 profile; see docs/profile-preparation.md."""
import argparse
import hashlib
import json
from pathlib import Path

import colour
import numpy as np
import scipy
from colour.quality.cfi2017 import load_TCS_CIE2017
from scipy.interpolate import PchipInterpolator
from scipy.optimize import brentq, least_squares
from scipy.special import ndtr

ROOT = Path(__file__).resolve().parents[1]
WL = np.arange(380.0, 781.0, 5.0)


def read_json(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def points(table):
    return np.array(sorted((float(x), y) for x, y in table.items())).T


def sampled(table, *, log_tail=False):
    x, y = points(table)
    out = PchipInterpolator(x, y, extrapolate=False)(WL)
    for side, edge, neighbor in ((WL < x[0], 0, 1), (WL > x[-1], -1, -2)):
        slope = (y[edge] - y[neighbor]) / (x[edge] - x[neighbor])
        out[side] = y[edge] + slope * (WL[side] - x[edge])
    return np.clip(out, -12, None) if log_tail else np.maximum(out, 0)


def observer_and_illuminant(data_dir, label):
    catalog = read_json(data_dir / 'illuminants/catalog.json')
    cmfs = read_json(data_dir / 'illuminants/cmfs.json')
    def resample(values, shape):
        grid = np.arange(shape[0], shape[1] + shape[2] / 2, shape[2])
        values = np.asarray(values)
        if values.ndim == 1:
            return np.interp(WL, grid, values)
        return np.stack([np.interp(WL, grid, c) for c in values.T], axis=1)
    return resample(cmfs['values'], cmfs['spectral_shape']), resample(catalog['sources'][label], catalog['spectral_shape'])


def cubic_lookup(cube, tc):
    """Match the runtime's Mitchell B=C=1/3 kernel and reflected boundaries."""
    tc = np.atleast_2d(tc)
    pos = np.clip(tc, 0, 1) * (cube.shape[0] - 1)
    cell = np.minimum(pos.astype(int), cube.shape[0] - 2)
    frac = pos - cell
    out = np.zeros((len(tc), cube.shape[2]))
    total = np.zeros(len(tc))
    def weight(v):
        v = np.abs(v)
        return np.where(v < 1, (7*v**3 - 12*v**2 + 16/3)/6,
                        np.where(v < 2, (-7/3*v**3 + 12*v**2 - 20*v + 32/3)/6, 0))
    def reflect(i):
        i = i % (2*cube.shape[0])
        return np.where(i >= cube.shape[0], 2*cube.shape[0]-i-1, i)
    for i in range(-1, 3):
        for j in range(-1, 3):
            w = weight(frac[:, 0]-i) * weight(frac[:, 1]-j)
            out += w[:, None] * cube[reflect(cell[:, 0]+i), reflect(cell[:, 1]+j)]
            total += w
    return out / total[:, None]


def xy_to_tc(xy):
    return np.stack(((1-xy[..., 0])**2, xy[..., 1]/(1-xy[..., 0])), axis=-1)


def polynomial(tc, center):
    x, y = (tc-center).T
    return np.stack((x, y, x*x, y*y, x*y, x**3, y**3, x*x*y, x*y*y,
                     x**4, y**4, x**3*y, x*x*y*y, x*y**3), axis=1)


def bounded(z):
    return z / np.sqrt(1+(z/2)**2)


def calibrate_sensitivity(raw_sensitivity, data_dir, cmfs, daylight):
    cube_path = data_dir / 'luts/spectral_upsampling/hanatos2025_irradiance_xy_tc.npy'
    cube = np.load(cube_path).astype(float)
    white_xyz = daylight @ cmfs
    white_xyz /= white_xyz[1]
    white_tc = xy_to_tc(white_xyz[:2]/white_xyz.sum())
    neutral = cubic_lookup(cube, white_tc)[0]
    sensitivity = raw_sensitivity / ((neutral @ raw_sensitivity) * white_xyz.sum() * .184)
    shape = colour.SpectralShape(380, 780, 5)
    training = load_TCS_CIE2017(shape).copy().align(shape).values.T
    validation = np.array([sd.copy().align(shape).values for sd in colour.SDS_COLOURCHECKERS['BabelColor Average'].values()])
    def sample_context(reflectances):
        xyz = (reflectances * daylight) @ cmfs / (daylight @ cmfs[:, 1])
        brightness = xyz.sum(1)
        tc = xy_to_tc(xyz[:, :2]/brightness[:, None])
        spectra = cubic_lookup(cube, tc)
        target = ((reflectances*daylight) @ raw_sensitivity) / (.184*(daylight @ raw_sensitivity))
        return spectra, brightness, polynomial(tc, white_tc), target
    train, holdout = sample_context(training), sample_context(validation)
    def window(p):
        return ndtr((WL-p[0])/p[1])*ndtr((p[2]-WL)/p[3])
    def prediction(context, p, coeff=None):
        spectra, brightness, design, _ = context
        w = window(p)
        norm = (neutral @ (sensitivity*w[:, None])) / (neutral @ sensitivity)
        raw = (spectra @ (sensitivity*w[:, None]/norm)) * brightness[:, None]
        if coeff is not None:
            raw *= 2**bounded(design @ coeff.T)
        return raw
    fit = least_squares(lambda p: np.log2(prediction(train, p)/train[3]).ravel(),
                        [400, 30, 700, 40], bounds=([350, 5, 620, 5], [450, 100, 800, 150]),
                        max_nfev=500, ftol=1e-11, xtol=1e-11, gtol=1e-11)
    if not fit.success:
        raise RuntimeError(f'Adaptation window did not converge: {fit.message}')
    before = np.log2(prediction(train, fit.x)/train[3])
    # Ridge penalty limits high-order excursions outside the 99 measured samples.
    surface = least_squares(lambda p: np.r_[(before+bounded(train[2] @ p.reshape(3, 14).T)).ravel(), .01*p],
                            np.zeros(42), max_nfev=500, ftol=1e-11, xtol=1e-11, gtol=1e-11)
    if not surface.success:
        raise RuntimeError(f'Adaptation surface did not converge: {surface.message}')
    coeff = surface.x.reshape(3, 14)
    def metrics(context, coefficients=coeff):
        error = np.log2(prediction(context, fit.x, coefficients)/context[3])
        return {'rmse_stops_rgb': np.sqrt(np.mean(error**2, axis=0)).tolist(),
                'max_abs_stops_rgb': np.max(abs(error), axis=0).tolist()}
    report = {'training': '99 CIE 2017 TCS under D55', 'validation': '24 BabelColor Average patches under D55',
              'training_error': metrics(train), 'validation_error': metrics(holdout),
              'default_window_only_validation_error': metrics(holdout, None),
              'window_parameters': fit.x.tolist(), 'spectra_lut_sha256': digest(cube_path),
              'training_reflectances_sha256': hashlib.sha256(training.astype('<f8').tobytes()).hexdigest(),
              'validation_reflectances_sha256': hashlib.sha256(validation.astype('<f8').tobytes()).hexdigest()}
    return np.log10(sensitivity), fit.x, np.pad(coeff, ((0, 0), (1, 0))), report


def prepare(source_path, data_dir):
    source = read_json(source_path)
    dyes = np.stack([sampled(t) for t in source['dye_density']], axis=1)
    shape = colour.SpectralShape(380, 780, 5)
    weights = []
    for table in source['status_a']:
        density = colour.SpectralDistribution({float(x): y for x, y in table.items()})
        density.extrapolate(shape, extrapolator_kwargs={'method': 'linear'})
        density.values = 10**density.values
        density.interpolate(shape)
        weights.append(density.values / density.values.sum())
    weights = np.stack(weights, axis=1)
    def measure(spectra):
        return -np.log10(10**(-spectra) @ weights)
    curves = [points(t) for t in source['characteristic']]
    floors = np.array([np.median(y[x >= .45]) for x, y in curves])
    # Smooth nonnegative base estimate constrained only by measured clear-end RGB.
    basis = np.exp(-.5*((WL[:, None]-np.array([440, 545, 660]))/65)**2)
    baseline = float(floors.min()*.75)
    base_fit = least_squares(lambda a: measure(baseline+basis @ a)-floors,
                             np.full(3, .05), bounds=(0, .5),
                             ftol=1e-13, xtol=1e-13, gtol=1e-13)
    if not base_fit.success:
        raise RuntimeError(f'Base fit did not converge: {base_fit.message}')
    base = baseline + basis @ base_fit.x
    def unpack(p):
        return p.reshape(3, 3, 3)
    def amounts(p, x):
        centers, amplitudes, sigmas = unpack(p)
        return (amplitudes[None]*ndtr((centers[None]-x[:, None, None])/sigmas[None])).sum(2)
    def prediction(p, x):
        return measure(base + amounts(p, x) @ dyes.T)
    initial = np.stack((np.tile([-1.7, -1.2, -.5], (3, 1)), np.ones((3, 3)), np.full((3, 3), .35))).ravel()
    lower = np.r_[np.full(9, -3.5), np.full(9, .001), np.full(9, .05)]
    upper = np.r_[np.full(9, 1.5), np.full(9, 5), np.full(9, 2)]
    masks = [np.arange(len(x)) % 5 != 2 for x, _ in curves]
    def residual(p):
        return np.concatenate([prediction(p, x[m])[:, c]-y[m] for c, ((x, y), m) in enumerate(zip(curves, masks))])
    fit = least_squares(residual, initial, bounds=(lower, upper), max_nfev=4000,
                        ftol=1e-10, xtol=1e-10, gtol=1e-10)
    if not fit.success:
        raise RuntimeError(f'Density fit did not converge: {fit.message}')
    centers, amp, sigma = unpack(fit.x).copy()
    for c in range(3):
        order = np.argsort(centers[c])
        centers[c], amp[c], sigma[c] = centers[c, order], amp[c, order], sigma[c, order]
    fitted = np.stack((centers, amp, sigma)).ravel()
    cmfs, daylight = observer_and_illuminant(data_dir, 'D55')
    _, viewing = observer_and_illuminant(data_dir, 'D50')
    scan_weights = viewing * cmfs[:, 1]
    scan_weights /= scan_weights.sum()
    reference = brentq(lambda h: (10**(-(base+amounts(fitted, np.array([h]))[0] @ dyes.T))) @ scan_weights-.184, -3, 1.5)
    grid = np.linspace(-3.5-reference, 1.5-reference, 256)
    density_curves = amounts(fitted, grid+reference)
    layers = amp[None]*ndtr((centers[None]-(grid+reference)[:, None, None])/sigma[None])
    sensitivity = 10**np.stack([sampled(t, log_tail=True) for t in source['log_sensitivity']], axis=1)
    logs, window, surface, adaptation_report = calibrate_sensitivity(sensitivity, data_dir, cmfs, daylight)
    errors = [prediction(fitted, x)[:, c]-y for c, (x, y) in enumerate(curves)]
    report = {'stock': source['stock'], 'source_sha256': digest(source_path),
              'versions': {'numpy': np.__version__, 'scipy': scipy.__version__, 'colour': colour.__version__},
              'generator_sha256': digest(Path(__file__)),
              'observer_sha256': digest(data_dir/'illuminants/cmfs.json'),
              'illuminants_sha256': digest(data_dir/'illuminants/catalog.json'),
              'base_floor_status_a_rgb': floors.tolist(), 'estimated_base_status_a_rgb': measure(base).tolist(),
              'reference_log10_lux_seconds': reference,
              'fit': {'rmse_rgb': [float(np.sqrt(np.mean(e**2))) for e in errors],
                      'max_abs_rgb': [float(np.max(abs(e))) for e in errors],
                      'heldout_rmse_rgb': [float(np.sqrt(np.mean(e[~m]**2))) for e, m in zip(errors, masks)],
                      'samples_rgb': [len(e) for e in errors]},
              'adaptation': adaptation_report,
              'source_samples': [{'log10_H': x.tolist(), 'density': y.tolist(), 'predicted': (y+e).tolist(),
                                  'heldout': (~m).tolist()} for (x, y), e, m in zip(curves, errors, masks)],
              'limitations': ['Base spectrum inferred from clear-end RGB densities; not independently measured.',
                              'Spectral tails outside digitized support are endpoint-slope extrapolations, clamped nonnegative.',
                              'Three CDF components are numerical fits, not measured emulsion sublayers.',
                              'D55 reference is normalized to scanned Y=0.184; no absolute ISO calibration.',
                              'Hanatos fit uses CIE test reflectances; no matched Velvia photographs establish stock fidelity.',
                              'No borrowed grain or color_masking values are imported. Stock-specific grain, DIR and halation are unmeasured.']}
    if max(report['fit']['heldout_rmse_rgb']) > .08 or max(report['fit']['max_abs_rgb']) > .15:
        raise RuntimeError(f'Density fit exceeds preparation budget: {report["fit"]}')
    profile = {'info': {'stock': source['stock'], 'name': 'Fujifilm Velvia 50 (source fit)',
                       'type': 'positive', 'support': 'film', 'stage': 'filming', 'use': 'still',
                       'antihalation': 'strong', 'target_print': None, 'channel_model': 'color',
                       'densitometer': 'status_A', 'log_sensitivity_density_over_min': 1.0,
                       'reference_illuminant': 'D55', 'viewing_illuminant': 'D50'},
               'data': {'wavelengths': WL.tolist(), 'log_sensitivity': logs.tolist(),
                        'channel_density': dyes.tolist(), 'base_density': base.tolist(),
                        'midscale_neutral_density': (base+amounts(fitted, np.array([reference]))[0] @ dyes.T).tolist(),
                        'log_exposure': grid.tolist(), 'density_curves': density_curves.tolist(),
                        'density_curves_layers': layers.transpose(0, 2, 1).tolist(),
                        'density_curves_model': {'model_type': 'norm_cdfs', 'centers': (centers-reference).tolist(),
                                                 'amplitudes': amp.tolist(), 'sigmas': sigma.tolist(), 'alphas': None},
                        'development_time': None, 'hanatos2025_adaptation_window_params': window.tolist(),
                        'hanatos2025_adaptation_surface_params': surface.tolist()},
               'metadata': {'version': '0.3.4', 'created': '2026-10-11',
                            'copyright': 'Copyright (c) 2026 Jan Lohse (digitization); SpektraFilm contributors (fit).',
                            'license': 'Digitized input coordinates: MIT, see data/license/JAN_LOHSE_MIT.txt. New fit contributions: CC BY-SA 4.0, https://creativecommons.org/licenses/by-sa/4.0/. See data/license/VELVIA_50.txt. Original manufacturer data remain the property of Fujifilm.',
                            'citation': f'Fujifilm {source["primary_title"]}, {source["primary_revision"]}, page {source["primary_printed_page"]}; Jan Lohse spectral_film_lut; Colour Science; SpektraFilm.',
                            'datasource': f'Source fit from {source["digitization_url"]}; primary {source["primary_url"]}. Source SHA256 {digest(source_path)}. Rebuild with scripts/prepare_velvia50.py. See docs/profile-preparation.md for conditions, inferred base, spectral extrapolation and calibration. No other stock profile is copied. ' + ' '.join(report['limitations'])}}
    return profile, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data-dir', type=Path, default=ROOT/'data')
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args()
    source = args.data_dir/'profile_sources/fujifilm_velvia_50.json'
    profile, report = prepare(source, args.data_dir)
    payload = json.dumps(profile, indent=2, allow_nan=False)+'\n'
    report['profile_sha256'] = hashlib.sha256(payload.encode()).hexdigest()
    for path, content in ((args.output, payload), (args.report, json.dumps(report, indent=2, allow_nan=False)+'\n')):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
    print(json.dumps({k: report[k] for k in ('stock', 'profile_sha256', 'fit', 'adaptation')}, indent=2))


if __name__ == '__main__':
    main()
