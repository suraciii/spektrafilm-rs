"""Offline animation acceptance criterion; no native libraries or display."""
import json
import tempfile
from types import SimpleNamespace
from unittest.mock import patch
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts' / 'parity'))
from gui_viewer_acceptance import transition_evidence, reveal_evidence, _capture_to_target


class TransitionCriterionTests(unittest.TestCase):
    def test_direct_replacement_is_not_animation(self):
        self.assertFalse(transition_evidence([0, 20, 40], [[0, 20, 40], [100, 120, 140]],
                                             [100, 120, 140])['passed'])

    def test_real_intermediate_frames_pass(self):
        result = transition_evidence([0, 20, 40],
                                     [[0, 20, 40], [25, 45, 65], [60, 80, 100], [100, 120, 140]],
                                     [100, 120, 140])
        self.assertTrue(result['passed'])
        self.assertEqual([item['frame'] for item in result['intermediate_frames']], [1, 2])

    def test_unchanged_endpoints_fail(self):
        self.assertFalse(transition_evidence([10, 20], [[15, 25], [20, 30]], [10, 20])['passed'])

    def test_unrelated_repaint_is_not_a_blend(self):
        self.assertFalse(transition_evidence([0, 100], [[25, 20], [60, 0]], [100, 0])['passed'])

    def test_single_or_repeated_intermediate_frame_fails(self):
        for frames in ([[50]], [[50], [50]]):
            self.assertFalse(transition_evidence([0], frames, [100])['passed'])

    def test_near_endpoint_noise_fails(self):
        self.assertFalse(transition_evidence([0], [[1], [99]], [100])['passed'])

    def test_reveal_rejects_direct_replacement_and_repeated_intermediate(self):
        for frames in ([[0, 0], [100, 100]], [[30, 90], [30, 90], [100, 100]]):
            self.assertFalse(reveal_evidence([0, 0], frames, [100, 100])['passed'])

    def test_late_nonlinear_reveal_has_distinct_intermediates(self):
        frames = [[0, 0]] * 40 + [[30, 90], [80, 20], [100, 100]]
        result = reveal_evidence([0, 0], frames, [100, 100])
        self.assertTrue(result['passed'])
        self.assertEqual(result['intermediate_frames'], [40, 41])

    def test_capture_ignores_repeated_intermediate_until_known_endpoint(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'operation-2.json').write_text(json.dumps({'operation': {
                'id': 2, 'kind': 'preview', 'outcome': 'succeeded'}}))
            driver = SimpleNamespace(report_directory=root, last_report_id=1)
            frames = [(0,), (10,), (10,), (20,), (100,)]
            with (patch('gui_viewer_acceptance._record_frame', side_effect=frames),
                  patch('gui_viewer_acceptance._sample', side_effect=lambda value: value),
                  patch('gui_viewer_acceptance.time.sleep')):
                captured, endpoint = _capture_to_target(driver, [], 'crossfade', (100,))
            self.assertEqual(captured, frames)
            self.assertEqual(endpoint, (100,))
            self.assertTrue(transition_evidence((0,), captured, endpoint)['passed'])

    def test_matching_endpoint_does_not_replace_new_render_completion(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / 'operation-1.json'
            report.write_text(json.dumps({'operation': {
                'id': 1, 'kind': 'preview', 'outcome': 'succeeded'}}))
            driver = SimpleNamespace(report_directory=root, last_report_id=1)
            calls = []
            def frame(*args):
                calls.append(None)
                if len(calls) == 3:
                    (root / 'operation-2.json').write_text(json.dumps({'operation': {
                        'id': 2, 'kind': 'preview', 'outcome': 'succeeded'}}))
                return (100,)
            with (patch('gui_viewer_acceptance._record_frame', side_effect=frame),
                  patch('gui_viewer_acceptance._sample', side_effect=lambda value: value),
                  patch('gui_viewer_acceptance.time.sleep')):
                captured, _ = _capture_to_target(driver, [], 'reveal', (100,))
            self.assertEqual(len(captured), 3)


if __name__ == '__main__':
    unittest.main()
