#!/usr/bin/env python3
"""Source and deterministic preparation checks for Rollei Retro 80S / Neofin Blau."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
import prepare_rollei80s as preparation


class Rollei80sPreparationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = preparation.ROOT / 'data'
        cls.source = cls.data / 'profile_sources/rollei_retro_80s_35mm_neofin_blau.json'
        cls.payload, cls.report = preparation.prepare(cls.source, cls.data)

    def test_single_profile_reproduces_shipped_bytes(self):
        repeated, report = preparation.prepare(self.source, self.data)
        self.assertEqual(self.payload, repeated)
        self.assertEqual(self.report, report)
        self.assertEqual(
            self.payload,
            (self.data / 'profiles/rollei_retro_80s_35mm_neofin_blau.json').read_text(),
        )
        self.assertEqual(
            [p.name for p in (self.data / 'profiles').glob('rollei_retro_80s_35mm_neofin_blau*.json')],
            ['rollei_retro_80s_35mm_neofin_blau.json'],
        )

    def test_density_accounting_and_source_zones(self):
        profile = json.loads(self.payload)
        d = profile['data']
        source = json.loads(self.source.read_text())
        self.assertEqual(profile['info']['type'], 'negative')
        self.assertEqual(profile['info']['use'], 'still')
        self.assertEqual(profile['info']['channel_model'], 'bw')
        self.assertEqual(d['development_time'], [6.0])
        self.assertEqual(profile['info']['log_sensitivity_density_over_min'], 1.0)
        density = np.asarray(d['density_curves'])[:, 0]
        self.assertTrue(np.all(np.diff(density) >= 0))
        np.testing.assert_array_equal(np.asarray(d['density_curves_layers'])[:, 0, 0], density)
        self.assertIsNone(d['density_curves_model'])
        self.assertAlmostEqual(float(np.min(np.asarray(d['base_density']))), 0.05, places=12)
        self.assertAlmostEqual(float(density[0]), 0.0, places=12)
        self.assertGreater(float(density[-1]), 1.35)
        source_x = np.asarray(source['characteristic'])[:, 0]
        source_density = np.asarray(source['characteristic'])[:, 1]
        grid_x = np.asarray(d['log_exposure']) + self.report['reference_source_log_exposure']
        for i, (x, value) in enumerate(zip(source_x, source_density)):
            reconstructed = np.interp(x, grid_x, density) + 0.05
            self.assertAlmostEqual(
                float(reconstructed), float(value), places=5,
                msg=f'Zone {i + 1} density mismatch',
            )

    def test_source_identity_zone_and_spectral_shape(self):
        source = json.loads(self.source.read_text())
        self.assertEqual(source['stock'], 'rollei_retro_80s_35mm_neofin_blau')
        self.assertEqual(source['primary_sha256'], preparation.SOURCE_HTML_HASH)
        self.assertEqual(source['spectral_sha256'], preparation.SPECTRAL_PDF_HASH)
        self.assertEqual(source['measurement']['format'], '35 mm')
        self.assertEqual(source['measurement']['exposure_index'], 'EI 80/20')
        self.assertIn('Neofin Blau', source['process'])
        self.assertEqual(
            [round(float(v), 2) for v in source['characteristic'][0]],
            [0.0, 0.05],
        )
        spectral = np.asarray(source['spectral_points'])
        self.assertEqual(spectral.shape, (15, 2))
        self.assertTrue(np.all(np.diff(spectral[:, 0]) > 0))
        self.assertEqual(float(spectral[0, 0]), 350.0)
        self.assertEqual(float(spectral[-1, 0]), 800.0)
        self.assertAlmostEqual(float(spectral[:, 1].max()), 1.6872, places=4)
        self.assertAlmostEqual(float(spectral[-1, 1]), 0.0, places=12)

    def test_rejects_non_monotone_characteristic(self):
        source = json.loads(self.source.read_text())
        broken = copy.deepcopy(source)
        broken['characteristic'][5][1] = broken['characteristic'][4][1] - 0.1
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'source.json'
            path.write_text(json.dumps(broken))
            with self.assertRaises(ValueError):
                preparation.prepare(path, self.data)

    def test_rejects_swapped_stock_or_hashes(self):
        source = json.loads(self.source.read_text())
        for key, value in (
            ('stock', 'kodak_tmax_100'),
            ('primary_sha256', '0' * 64),
            ('spectral_sha256', '0' * 64),
        ):
            broken = copy.deepcopy(source)
            broken[key] = value
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'source.json'
                path.write_text(json.dumps(broken))
                with self.assertRaises(ValueError):
                    preparation.prepare(path, self.data)

    def test_spectral_adaptation_is_explicitly_reported(self):
        adaptation = self.report['adaptation']
        self.assertEqual(adaptation['training'], '99 CIE 2017 TCS under D55')
        self.assertEqual(adaptation['validation'], '24 BabelColor Average patches under D55')
        self.assertGreater(adaptation['validation_error']['rmse_stops_rgb'][0], 0)
        self.assertLess(adaptation['validation_error']['rmse_stops_rgb'][0], 0.5)
        self.assertEqual(len(adaptation['validation_error']['rmse_stops_rgb']), 1)
        self.assertAlmostEqual(self.report['estimated_base_density'], 0.05, places=12)
        self.assertIn('Rollei page-3', self.report['spectral_baseline'])
        self.assertIn('No 120-specific or ID-11/D-76', ' '.join(self.report['limitations']))
        self.assertLess(self.report['runtime_vs_source_max_density'], 1e-5)


if __name__ == '__main__':
    unittest.main()
