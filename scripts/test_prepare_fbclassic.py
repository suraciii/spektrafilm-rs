#!/usr/bin/env python3
"""Fast numerical preparation regressions for FB CLASSIC grade 2."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

import prepare_fbclassic as preparation


class FbClassicPreparationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = preparation.SOURCE
        cls.payloads, cls.report = preparation.prepare(cls.source, Path('/tmp'))

    def test_rebuild_matches_shipped_profile_and_repeats(self):
        repeated, report = preparation.prepare(self.source, Path('/tmp'))
        self.assertEqual(self.payloads, repeated)
        self.assertEqual(self.report, report)
        self.assertEqual(set(self.payloads), {preparation.STOCK})
        shipped = preparation.ROOT/'data/profiles'/f'{preparation.STOCK}.json'
        self.assertTrue(shipped.exists())
        self.assertEqual(self.payloads[preparation.STOCK], shipped.read_text())

    def test_source_pdf_hash_and_curve_order_are_enforced(self):
        source = json.loads(self.source.read_text())
        changed = copy.deepcopy(source)
        changed['primary_sha256'] = '0'*64
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'source.json'
            path.write_text(json.dumps(changed))
            with self.assertRaises(ValueError):
                preparation.prepare(path, Path(directory))
        changed = copy.deepcopy(source)
        changed['characteristic']['2'][1][0] = changed['characteristic']['2'][0][0]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'source.json'
            path.write_text(json.dumps(changed))
            with self.assertRaises(ValueError):
                preparation.prepare(path, Path(directory))

    def test_isotonic_repair_is_minimal_and_monotone(self):
        np.testing.assert_array_equal(preparation.monotone([0., 2., 1., 3.]), [0., 1.5, 1.5, 3.])
        np.testing.assert_array_equal(preparation.monotone([0., 1., 2.]), [0., 1., 2.])

    def test_source_positions_and_density_are_preserved(self):
        source = json.loads(self.source.read_text())
        origin = self.report['reference_source_loge']
        profile = json.loads(self.payloads[preparation.STOCK])
        data = profile['data']
        grid = np.array(data['log_exposure']) + origin
        base = data['base_density'][0][0]
        total = np.array(data['density_curves'])[:, 0] + base
        x, y = np.array(source['characteristic']['2']).T
        self.assertLess(np.max(abs(np.interp(x, grid, total)-y)), .001)
        self.assertTrue(np.all(np.diff(total) >= 0))
        self.assertAlmostEqual(total[0], y[0])
        self.assertAlmostEqual(total[-1], y[-1])
        self.assertEqual(profile['info']['channel_model'], 'bw')
        self.assertIsNone(data['development_time'])
        self.assertIsNone(data['density_curves_model'])
        self.assertTrue(np.isfinite(np.array(data['log_sensitivity'])).all())

    def test_flat_spectrum_is_explicit_placeholder(self):
        profile = json.loads(self.payloads[preparation.STOCK])
        data = profile['data']
        self.assertTrue(np.all(np.array(data['log_sensitivity']) == 0.0))
        self.assertTrue(np.all(np.array(data['base_density']) == 0.0))
        self.assertIn('Flat 0.0 log-sensitivity is a placeholder', profile['metadata']['datasource'])
        self.assertIn('no 00-5 variable-contrast prediction', profile['metadata']['datasource'])


if __name__ == '__main__':
    unittest.main()
