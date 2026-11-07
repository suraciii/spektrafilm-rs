"""Fast checks of source identity and the exported physical representation."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
from scipy.interpolate import PchipInterpolator

from extract_astia100f import coordinates
from prepare_astia100f import ROOT, reconstruct, validated_source

SOURCE = ROOT/'data/profile_sources/fujifilm_astia_100f.json'


class AstiaPreparationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = validated_source(SOURCE)
        cls.profile = json.loads((ROOT/'data/profiles/fujifilm_astia_100f.json').read_text())

    def test_modified_coordinates_axes_and_instrument_are_rejected(self):
        for field in ('coordinates', 'axis_ticks', 'status_a', 'primary_sha256'):
            source = copy.deepcopy(self.source)
            if field == 'coordinates':
                source[field]['characteristic'][0][0][1] += .1
            elif field == 'axis_ticks':
                source[field]['characteristic'][1][0] += 1
            elif field == 'status_a':
                source[field][0]['620'] += .1
            else:
                source[field] = 'wrong'
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory)/'source.json'
                path.write_text(json.dumps(source))
                with self.assertRaises(ValueError, msg=field):
                    validated_source(path)

    def test_scalar_transmittance_reconstructs_dense_source(self):
        data = self.profile['data']
        source = coordinates(self.source['vector_paths'], 2048)['characteristic']
        grid = np.array(data['log_exposure'])
        origin = source[0][0][0]-grid[0]
        dyes, base = np.array(data['channel_density']), np.array(data['base_density'])
        # Independent 1nm scalar Simpson accumulation into 5nm transmission.
        fine = np.arange(380., 781.)
        quadrature = np.full(401, 2./3.)
        quadrature[1::2] = 4./3.
        quadrature[[0, -1]] = 1./3.
        weights = []
        for table in self.source['status_a']:
            x = np.array([float(k) for k in table])
            y = np.array(list(table.values()))
            v = np.interp(fine, x, y)
            for mask, e, n in ((fine < x[0], 0, 1), (fine > x[-1], -1, -2)):
                v[mask] = y[e]+(y[e]-y[n])/(x[e]-x[n])*(fine[mask]-x[e])
            acc = np.zeros(81)
            for i, wl in enumerate(fine):
                lo = min(int((wl-380)//5), 79)
                fraction = (wl-(380+lo*5))/5
                acc[lo] += 10**v[i]*quadrature[i]*(1-fraction)
                acc[lo+1] += 10**v[i]*quadrature[i]*fraction
            weights.append(acc/acc.sum())
        amounts = np.array(data['density_curves'])
        for channel, curve in enumerate(source):
            xy = np.array(curve)
            a = np.stack([np.interp(xy[:, 0]-origin, grid, amounts[:, c]) for c in range(3)], axis=1)
            transmittance = 10**(-base-a@dyes.T)
            actual = -np.log10(np.array([sum(t*w for t, w in zip(row, weights[channel])) for row in transmittance]))
            self.assertLess(float(np.max(abs(actual-xy[:, 1]))), .005)

    def test_sampled_records_have_no_model_override_or_fabricated_sublayers(self):
        data = self.profile['data']
        a = np.array(data['density_curves'])
        layers = np.array(data['density_curves_layers'])
        self.assertIsNone(data['density_curves_model'])
        self.assertEqual(layers.shape, (len(a), 1, 3))
        np.testing.assert_array_equal(layers[:, 0], a)
        self.assertTrue(np.isfinite(a).all() and (a >= 0).all())
        self.assertTrue((np.diff(a, axis=0) <= 0).all())
        np.testing.assert_array_equal(a[-1], [0., 0., 0.])

    def test_reconstruction_matches_committed_runtime_arrays(self):
        grid, amounts, base, dyes, report = reconstruct(self.source)
        data = self.profile['data']
        for expected, stored in ((grid, data['log_exposure']), (amounts, data['density_curves']),
                                 (base, data['base_density']), (dyes, data['channel_density'])):
            np.testing.assert_allclose(expected, stored, atol=1e-12, rtol=0)
        self.assertLess(max(report['max_density_error_rgb']), .005)

    def test_labelled_graph_checkpoints(self):
        # Coarse independent visual checkpoints; not a precision measurement.
        ranges = [(3.25, 3.36), (3.38, 3.50), (3.45, 3.56)]
        for curve, bounds in zip(self.source['coordinates']['characteristic'], ranges):
            xy = np.array(curve)
            self.assertTrue(bounds[0] < PchipInterpolator(xy[:, 0], xy[:, 1])(-3.) < bounds[1])
        for curve, peak in zip(self.source['coordinates']['log_sensitivity'], (645, 550, 445)):
            xy = np.array(curve)
            self.assertLess(abs(xy[np.argmax(xy[:, 1]), 0]-peak), 5.)


if __name__ == '__main__':
    unittest.main()
