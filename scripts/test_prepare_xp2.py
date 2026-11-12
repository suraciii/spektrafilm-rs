#!/usr/bin/env python3
"""Source and deterministic preparation checks for Ilford XP2 Super 400."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
import prepare_xp2 as preparation


class Xp2PreparationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = preparation.ROOT / 'data'
        cls.source = cls.data / 'profile_sources/ilford_xp2_super_400.json'
        cls.payload, cls.report = preparation.prepare(cls.source, cls.data)

    def test_single_profile_reproduces_shipped_bytes(self):
        repeated, report = preparation.prepare(self.source, self.data)
        self.assertEqual(self.payload, repeated)
        self.assertEqual(self.report, report)
        self.assertEqual(self.payload, (self.data / 'profiles/ilford_xp2_super_400.json').read_text())
        self.assertEqual([p.name for p in (self.data / 'profiles').glob('ilford_xp2_super_400*.json')],
                         ['ilford_xp2_super_400.json'])

    def test_density_accounting_and_single_record(self):
        profile = json.loads(self.payload)
        d = profile['data']
        self.assertEqual(profile['info']['type'], 'negative')
        self.assertEqual(profile['info']['use'], 'still')
        self.assertEqual(profile['info']['channel_model'], 'bw')
        self.assertEqual(d['development_time'], [0.0])
        self.assertEqual(profile['info']['log_sensitivity_density_over_min'], 1.0)
        density = np.asarray(d['density_curves'])[:, 0]
        self.assertTrue(np.all(np.diff(density) >= 0))
        np.testing.assert_array_equal(np.asarray(d['density_curves_layers'])[:, 0, 0], density)
        self.assertIsNone(d['density_curves_model'])
        self.assertGreater(density[-1], 1.5)
        self.assertAlmostEqual(float(np.min(np.asarray(d['base_density']))), 0.2685185185185164, places=12)

    def test_source_identity_raster_axes_and_spectral_assumption(self):
        source = json.loads(self.source.read_text())
        self.assertEqual(source['stock'], 'ilford_xp2_super_400')
        self.assertEqual(source['primary_sha256'], preparation.PDF_HASH)
        self.assertEqual(source['primary_pages'], {'characteristic': 3, 'spectral': 2})
        self.assertEqual(source['extraction']['geometry_sha256'], preparation.GEOMETRY_HASH)
        self.assertIn('linear', source['axis_mapping']['spectral']['ordinate'])
        for name in ('characteristic', 'spectral'):
            image = source['extraction']['images'][name]
            self.assertEqual(len(image['image_sha256']), 64)
            self.assertLess(image['source_domain'][0], image['source_domain'][1])
        characteristic = np.asarray(source['characteristic'])
        self.assertTrue(np.all(np.diff(characteristic[:, 0]) > 0))
        self.assertTrue(np.all(np.diff(characteristic[:, 1]) >= 0))
        sensitivity = np.asarray(source['relative_sensitivity'])
        self.assertTrue(np.all(np.diff(sensitivity[:, 0]) > 0))
        self.assertGreater(sensitivity[:, 1].min(), -1.0)
        self.assertGreater(sensitivity[:, 1].max(), 0.8)
        self.assertGreater(sensitivity[0, 1], sensitivity[-1, 1])
        self.assertIn('2856 K', source['limitations'][2])
        self.assertIn('380-355', source['limitations'][3])
        self.assertIn('chromogenic', source['limitations'][5])

    def test_rejects_non_monotone_characteristic(self):
        source = json.loads(self.source.read_text())
        broken = copy.deepcopy(source)
        broken['characteristic'][10][1] = broken['characteristic'][9][1] - 0.1
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'source.json'
            path.write_text(json.dumps(broken))
            with self.assertRaises(ValueError):
                preparation.prepare(path, self.data)

    def test_rejects_swapped_stock_or_pdf_or_geometry(self):
        source = json.loads(self.source.read_text())
        for path in (('stock', 'kodak_tmax_100'), ('primary_sha256', '0' * 64)):
            broken = copy.deepcopy(source)
            broken[path[0]] = path[1]
            with tempfile.TemporaryDirectory() as directory:
                target = Path(directory) / 'source.json'
                target.write_text(json.dumps(broken))
                with self.assertRaises(ValueError):
                    preparation.prepare(target, self.data)
        broken = copy.deepcopy(source)
        broken['extraction']['geometry_sha256'] = '0' * 64
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / 'source.json'
            target.write_text(json.dumps(broken))
            with self.assertRaises(ValueError):
                preparation.prepare(target, self.data)

    def test_adapter_is_refitted_and_holdout_is_measured(self):
        adaptation = self.report['adaptation']
        self.assertEqual(adaptation['training'], '99 CIE 2017 TCS under D55')
        self.assertEqual(adaptation['validation'], '24 BabelColor Average patches under D55')
        self.assertGreater(adaptation['validation_error']['rmse_stops_rgb'][0], 0)
        self.assertEqual(len(adaptation['validation_error']['rmse_stops_rgb']), 1)
        self.assertEqual(self.report['spectral_baseline'], 'linear normalized wedge sensitivity, explicit interpretation')
        self.assertAlmostEqual(self.report['estimated_base_density'], 0.2685185185185164, places=12)
        self.assertIn('No RMS granularity or MTF is published', ' '.join(self.report['limitations']))


if __name__ == '__main__':
    unittest.main()
