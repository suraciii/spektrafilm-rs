#!/usr/bin/env python3
"""Fast numerical preparation regressions. Requires NumPy and SciPy."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

import prepare_polymax as preparation
import extract_polymax as extraction


class PolymaxPreparationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = preparation.ROOT/'data'
        cls.source = cls.data/'profile_sources/kodak_polymax_fine_art.json'
        cls.payloads, cls.report = preparation.prepare(cls.source, cls.data)

    def test_rebuild_matches_shipped_profiles_and_repeats(self):
        repeated, report = preparation.prepare(self.source, self.data)
        self.assertEqual(self.payloads, repeated)
        self.assertEqual(self.report, report)
        self.assertEqual(set(self.payloads), {'kodak_polymax_fine_art'})
        self.assertEqual(
            {p.stem for p in (self.data/'profiles').glob('kodak_polymax*.json')},
            set(self.payloads))
        for stock, payload in self.payloads.items():
            self.assertEqual(payload, (self.data/'profiles'/f'{stock}.json').read_text())

    def test_curve_identity_rejects_reordered_thresholds_and_conditions(self):
        source = json.loads(self.source.read_text())
        extraction.validate_curve_identity(source['characteristic'], source['log_sensitivity'])
        for field, first, second in [('log_sensitivity', '0.6', '1.6'), ('characteristic', '0', '5')]:
            changed = copy.deepcopy(source)
            changed[field][first], changed[field][second] = changed[field][second], changed[field][first]
            with self.assertRaises(ValueError):
                extraction.validate_curve_identity(changed['characteristic'], changed['log_sensitivity'])

    def test_isotonic_repair_is_minimal_and_monotone(self):
        np.testing.assert_array_equal(preparation.monotone([0., 2., 1., 3.]), [0., 1.5, 1.5, 3.])
        np.testing.assert_array_equal(preparation.monotone([0., 1., 2.]), [0., 1., 2.])

    def test_invalid_source_axis_is_rejected(self):
        source = json.loads(self.source.read_text())
        source['characteristic']['unfiltered'][1][0] = source['characteristic']['unfiltered'][0][0]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'source.json'
            path.write_text(json.dumps(source))
            with self.assertRaises(ValueError):
                preparation.prepare(path, self.data)

    def test_source_positions_and_paper_density_are_preserved(self):
        source = json.loads(self.source.read_text())
        origin = self.report['log10_lux_second_origin']
        for stock, payload in self.payloads.items():
            profile = json.loads(payload)
            data = profile['data']
            grid = np.array(data['log_exposure']) + origin
            base = data['base_density'][0][0]
            total = np.array(data['density_curves'])[:, 0] + base
            label = self.report['profiles'][stock]['filter_label']
            x, y = np.array(source['characteristic'][label]).T
            self.assertLess(np.max(abs(np.interp(x, grid, total)-y)), .001)
            self.assertTrue(np.all(np.diff(total) >= 0))
            self.assertAlmostEqual(total[0], y[0])
            self.assertAlmostEqual(total[-1], y[-1])
            self.assertEqual(profile['info']['channel_model'], 'bw')
            self.assertIsNone(data['development_time'])
            self.assertIsNone(data['density_curves_model'])
            self.assertTrue(np.isfinite(np.array(data['log_sensitivity'])).all())


if __name__ == '__main__':
    unittest.main()
