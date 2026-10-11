"""Offline animation acceptance criterion; no native libraries or display."""
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts' / 'parity'))
from gui_viewer_acceptance import transition_evidence


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


if __name__ == '__main__':
    unittest.main()
