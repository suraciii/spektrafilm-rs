#!/usr/bin/env python3
"""Source and deterministic preparation checks for T-MAX 100 135."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
import prepare_tmax100 as preparation


class TMax100PreparationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = preparation.ROOT / 'data'
        cls.source = cls.data / 'profile_sources/kodak_tmax_100.json'
        cls.payload, cls.report = preparation.prepare(cls.source, cls.data)

    def test_single_profile_reproduces_shipped_bytes(self):
        repeated, report = preparation.prepare(self.source, self.data)
        self.assertEqual(self.payload, repeated)
        self.assertEqual(self.report, report)
        self.assertEqual(self.payload, (self.data / 'profiles/kodak_tmax_100.json').read_text())
        self.assertEqual([p.name for p in (self.data / 'profiles').glob('kodak_tmax_100*.json')],
                         ['kodak_tmax_100.json'])

    def test_density_accounting_and_single_record(self):
        profile = json.loads(self.payload)
        d = profile['data']
        self.assertEqual(profile['info']['type'], 'negative')
        self.assertEqual(profile['info']['use'], 'still')
        self.assertEqual(profile['info']['channel_model'], 'bw')
        self.assertEqual(d['development_time'], [7.5])
        self.assertEqual(profile['info']['log_sensitivity_density_over_min'], 0.3)
        density = np.asarray(d['density_curves'])[:, 0]
        self.assertTrue(np.all(np.diff(density) >= 0))
        np.testing.assert_array_equal(np.asarray(d['density_curves_layers'])[:, 0, 0], density)
        self.assertIsNone(d['density_curves_model'])
        self.assertGreater(density[-1], 1.5)
        self.assertAlmostEqual(float(np.min(np.asarray(d['base_density']))), 0.2130714851737701, places=12)

    def test_source_identity_vector_axes_and_two_spectral_curves(self):
        source = json.loads(self.source.read_text())
        self.assertEqual(source['stock'], 'kodak_tmax_100')
        self.assertEqual(source['primary_sha256'], preparation.PDF_HASH)
        self.assertEqual(source['primary_page'], 8)
        self.assertEqual(source['extraction']['geometry_sha256'], preparation.GEOMETRY_HASH)
        self.assertEqual(set(source['vector_paths']), {'characteristic', 'sensitivity_0.3', 'sensitivity_1.0'})
        characteristic = np.asarray(source['characteristic'])
        self.assertTrue(np.all(np.diff(characteristic[:, 0]) > 0))
        self.assertTrue(np.all(np.diff(characteristic[:, 1]) >= 0))
        for label in ('0.3', '1.0'):
            sensitivity = np.asarray(source['log_sensitivity'][label])
            self.assertTrue(np.all(np.diff(sensitivity[:, 0]) > 0))
            self.assertGreater(sensitivity[0, 1], -12)
            self.assertGreater(sensitivity[-1, 1], -12)
        self.assertGreater(source['log_sensitivity']['0.3'][0][1], source['log_sensitivity']['1.0'][0][1])

    def test_rejects_non_monotone_characteristic(self):
        source = json.loads(self.source.read_text())
        broken = copy.deepcopy(source)
        broken['characteristic'][10][1] = broken['characteristic'][9][1] - 0.1
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'source.json'
            path.write_text(json.dumps(broken))
            with self.assertRaises(ValueError):
                preparation.prepare(path, self.data)

    def test_rejects_swapped_stock_or_pdf(self):
        source = json.loads(self.source.read_text())
        for key, value in [('stock', 'kodak_trix_400'), ('primary_sha256', '0' * 64)]:
            broken = copy.deepcopy(source)
            broken[key] = value
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'source.json'
                path.write_text(json.dumps(broken))
                with self.assertRaises(ValueError):
                    preparation.prepare(path, self.data)

    def test_adapter_is_refitted_and_holdout_is_measured(self):
        adaptation = self.report['adaptation']
        self.assertEqual(adaptation['training'], '99 CIE 2017 TCS under D55')
        self.assertEqual(adaptation['validation'], '24 BabelColor Average patches under D55')
        self.assertGreater(adaptation['validation_error']['rmse_stops_rgb'][0], 0)
        self.assertEqual(len(adaptation['validation_error']['rmse_stops_rgb']), 1)
        self.assertEqual(self.report['spectral_baseline'], '0.3 above D-min')
        self.assertAlmostEqual(self.report['estimated_base_density'], 0.2130714851737701, places=12)


if __name__ == '__main__':
    unittest.main()
