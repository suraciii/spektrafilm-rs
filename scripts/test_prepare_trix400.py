#!/usr/bin/env python3
"""Source and deterministic preparation checks; NumPy/SciPy/colour required."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
import prepare_trix400 as preparation
from extract_trix400 import sample_path, map_axes


class TriXPreparationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = preparation.ROOT/'data'
        cls.source = cls.data/'profile_sources/kodak_trix_400.json'
        cls.payload, cls.report = preparation.prepare(cls.source, cls.data)

    def test_single_profile_reproduces_shipped_bytes(self):
        repeated, report = preparation.prepare(self.source, self.data)
        self.assertEqual(self.payload, repeated)
        self.assertEqual(self.report, report)
        self.assertEqual(self.payload, (self.data/'profiles/kodak_trix_400.json').read_text())
        self.assertEqual([p.name for p in (self.data/'profiles').glob('kodak_trix_400*.json')], ['kodak_trix_400.json'])

    def test_density_accounting_and_single_record(self):
        profile = json.loads(self.payload); d = profile['data']
        self.assertEqual(profile['info']['type'], 'negative')
        self.assertEqual(profile['info']['use'], 'still')
        self.assertEqual(d['development_time'], [6.])
        density = np.array(d['density_curves'])[:, 0]
        self.assertTrue(np.all(np.diff(density) >= 0))
        np.testing.assert_array_equal(np.array(d['density_curves_layers'])[:, 0, 0], density)
        self.assertIsNone(d['density_curves_model'])
        source = json.loads(self.source.read_text())
        x, y = np.array(source['characteristic']).T
        total = np.interp(x-self.report['reference_log10_lux_seconds'], d['log_exposure'], density)+d['base_density'][0][0]
        np.testing.assert_allclose(total, y, atol=1e-10, rtol=0)
        self.assertLess(self.report['runtime_vs_vector_max_density'], .001)

    def test_spectral_tail_and_adaptation_are_achromatic(self):
        d = json.loads(self.payload)['data']
        values = np.array(d['log_sensitivity'])[:, 0]
        self.assertTrue(np.isfinite(values).all())
        self.assertLess(values[-1], values[0]-6)
        surface = np.array(d['hanatos2025_adaptation_surface_params'])
        np.testing.assert_array_equal(surface, np.repeat(surface[:1], 3, axis=0))
        # Predeclared numerical acceptance for this RGB adapter, not film accuracy.
        result = self.report['adaptation']
        self.assertLess(result['validation_error']['rmse_stops_rgb'][0], .1)
        self.assertLess(result['validation_error']['rmse_stops_rgb'][0],
                        result['default_window_only_validation_error']['rmse_stops_rgb'][0])

    def test_vector_sampling_converges_and_rejects_discontinuity(self):
        source = json.loads(self.source.read_text()); p = source['vector_paths']['characteristic']
        a = map_axes(sample_path(p, 64), 'characteristic')
        b = map_axes(sample_path(p, 4096), 'characteristic')
        self.assertLess(np.max(abs(np.interp(b[:, 0], a[:, 0], a[:, 1])-b[:, 1])), .001)
        p = copy.deepcopy(p); p['d'] += ' M 0 0'
        with self.assertRaises(ValueError):
            sample_path(p)

    def test_source_substitutions_fail(self):
        original = json.loads(self.source.read_text())
        for change in ('primary_sha256', 'path', 'sensitivity'):
            source = copy.deepcopy(original)
            if change == 'primary_sha256':
                source[change] = '0'*64
            elif change == 'path':
                source['vector_paths']['sensitivity_1.0']['d'] += ' '
            else:
                source['log_sensitivity']['1.0'][10][1] += .01
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)/'bad.json'; path.write_text(json.dumps(source))
                with self.assertRaises(ValueError):
                    preparation.prepare(path, self.data)

    def test_invalid_axis_fails(self):
        source = json.loads(self.source.read_text())
        source['characteristic'][1][0] = source['characteristic'][0][0]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'bad.json'; path.write_text(json.dumps(source))
            with self.assertRaises(ValueError):
                preparation.prepare(path, self.data)


if __name__ == '__main__':
    unittest.main()
