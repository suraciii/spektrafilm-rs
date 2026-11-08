#!/usr/bin/env python3
"""Source and deterministic preparation checks for Acros II 135."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
import prepare_acros100ii as preparation


class AcrosIIPreparationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = preparation.ROOT / 'data'
        cls.source = cls.data / 'profile_sources/fujifilm_acros_100_ii.json'
        cls.payload, cls.report = preparation.prepare(cls.source, cls.data)

    def test_single_profile_reproduces_shipped_bytes(self):
        repeated, report = preparation.prepare(self.source, self.data)
        self.assertEqual(self.payload, repeated)
        self.assertEqual(self.report, report)
        self.assertEqual(self.payload, (self.data / 'profiles/fujifilm_acros_100_ii.json').read_text())
        self.assertEqual([p.name for p in (self.data / 'profiles').glob('fujifilm_acros_100_ii*.json')],
                         ['fujifilm_acros_100_ii.json'])

    def test_density_accounting_and_single_record(self):
        profile = json.loads(self.payload)
        d = profile['data']
        self.assertEqual(profile['info']['type'], 'negative')
        self.assertEqual(profile['info']['use'], 'still')
        self.assertEqual(profile['info']['channel_model'], 'bw')
        self.assertEqual(d['development_time'], [10.])
        density = np.asarray(d['density_curves'])[:, 0]
        self.assertTrue(np.all(np.diff(density) >= 0))
        np.testing.assert_array_equal(np.asarray(d['density_curves_layers'])[:, 0, 0], density)
        self.assertIsNone(d['density_curves_model'])
        self.assertGreater(density[-1], 1.5)
        self.assertAlmostEqual(float(np.min(np.asarray(d['base_density']))), 0.13486842105263158, places=12)

    def test_source_identity_axis_anchors_and_spectral_floor(self):
        source = json.loads(self.source.read_text())
        self.assertEqual(source['stock'], 'fujifilm_acros_100_ii')
        self.assertEqual(source['primary_sha256'], preparation.PDF_HASH)
        anchors = np.asarray(source['characteristic_anchors'])
        self.assertEqual(anchors.shape, (13, 2))
        self.assertTrue(np.all(np.diff(anchors[:, 0]) > 0))
        self.assertTrue(np.all(np.diff(anchors[:, 1]) <= 0))
        sensitivity = np.asarray(source['log_sensitivity'])
        self.assertTrue(np.all(np.diff(sensitivity[:, 0]) > 0))
        self.assertGreater(sensitivity[0, 1], -12)
        self.assertGreater(sensitivity[-1, 1], -12)

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
        self.assertEqual(self.report['characteristic_anchors'][0], [300, 535.5])


if __name__ == '__main__':
    unittest.main()
