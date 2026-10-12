"""Consumer-visible benchmark invariants and metric boundary regressions."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import struct
import zlib

import numpy as np
from PIL import Image, ImageCms

MODULES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(MODULES))
from documents import DocumentError, load_json_document, safe_artifact_path, protocol_digest
from image_data import ImageData, display_rgb
from metrics import aggregate_seed_measurements, ciede2000, compare_images
from references import compare_reference
from spektrafilm_bench import rule_result
import png16


class BenchmarkTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.base = ImageData(np.full((24, 32, 3), .2), "sRGB", "srgb")
        self.candidate = ImageData(np.full((24, 32, 3), .25), "sRGB", "srgb")
        self.protocol = {"case_id": "probe", "seed_population": [0], "measurements": ["numeric", "color", "tone", "flip"],
                         "pixels_per_degree": 60., "region_sampling_scales": [1.], "hotspots": {"count": 4, "min_area_px": 16}}

    def tearDown(self):
        self.temporary.cleanup()

    def test_display_crop_preserves_sub_8bit_channel_values(self):
        values = np.arange(11 * 9 * 3, dtype=np.uint16).reshape(9, 11, 3) + 30000
        png16.save(self.root / "full.png", values)
        png16.crop(self.root / "full.png", self.root / "crop.png", [3, 2, 5, 4])
        data = (self.root / "crop.png").read_bytes()
        offset, compressed, header = 8, bytearray(), None
        while offset < len(data):
            size, kind = struct.unpack_from(">I4s", data, offset)
            chunk = data[offset + 8:offset + 8 + size]
            if kind == b"IHDR":
                header = struct.unpack(">IIBBBBB", chunk)
            elif kind == b"IDAT":
                compressed.extend(chunk)
            offset += size + 12
        self.assertEqual(header, (5, 4, 16, 2, 0, 0, 0))
        rows = np.frombuffer(zlib.decompress(compressed), dtype=np.uint8).reshape(4, 31)
        recovered = rows[:, 1:].copy().view(">u2").reshape(4, 5, 3)
        np.testing.assert_array_equal(recovered, values[2:6, 3:8])

    def test_duplicate_and_nonfinite_json_rejected(self):
        for text, code in (("{\"x\":1,\"x\":2}", "duplicate_key"), ("{\"x\":NaN}", "nonfinite_number"), ("{\"x\":1e999}", "nonfinite_number")):
            path = self.root / "input.json"
            path.write_text(text)
            with self.assertRaises(DocumentError) as caught:
                load_json_document(path)
            self.assertEqual(caught.exception.code, code)

    def test_artifact_path_escape_and_symlink_rejected(self):
        for path in ("../escape", "/absolute", "C:/escape", "bad\\path"):
            with self.assertRaises(DocumentError):
                safe_artifact_path(self.root, path, must_exist=False)
        target = self.root / "target"
        target.write_text("evidence")
        (self.root / "alias").symlink_to(target)
        with self.assertRaises(DocumentError):
            safe_artifact_path(self.root, "alias")

    def test_identical_master_has_zero_perceptual_and_numeric_difference(self):
        result = compare_images(self.base, self.base, [], self.protocol, self.root / "same")
        self.assertEqual(result["execution_status"], "completed")
        rows = {row["metric"]: row for row in result["measurements"]}
        for name in ("numeric.rmse", "numeric.max_abs_difference", "delta_e00.max", "flip.mean", "delta_l_star.mean"):
            self.assertAlmostEqual(rows[name]["value"], 0., places=7)

    def test_exposure_change_has_signed_lightness_and_real_flip(self):
        result = compare_images(self.base, self.candidate, [], self.protocol, self.root / "changed")
        rows = {row["metric"]: row for row in result["measurements"]}
        self.assertGreater(rows["delta_l_star.mean"]["value"], 0)
        self.assertGreater(rows["flip.mean"]["value"], 0)
        self.assertLessEqual(rows["flip.mean"]["value"], 1)
        self.assertTrue(result["automatic_regions"])

    def test_nonfinite_and_size_fail_execution(self):
        invalid = self.base.samples.copy()
        invalid[0, 0, 0] = np.nan
        for name, image in (("nan", ImageData(invalid, "sRGB", "srgb")), ("size", ImageData(np.ones((12, 12, 3)), "sRGB", "srgb"))):
            result = compare_images(self.base, image, [], self.protocol, self.root / name)
            self.assertEqual(result["execution_status"], "failed")
            self.assertTrue(result["findings"])
            self.assertTrue(any(row["availability"] == "unavailable" for row in result["measurements"]))

    def test_rgb_icc_conversion_preserves_native_float_precision(self):
        profile = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
        path = self.root / "source.icc"
        path.write_bytes(profile)
        native = np.full((3, 4, 3), .200123456, dtype=np.float64)
        image = ImageData(native.copy(), None, None, metadata={"icc": {"path": str(path), "sha256": hashlib.sha256(profile).hexdigest()}})
        converted = display_rgb(image)
        np.testing.assert_array_equal(image.samples, native)
        np.testing.assert_allclose(converted, native, atol=2e-4)
        self.assertGreater(abs(converted[0, 0, 0]*255-round(converted[0, 0, 0]*255)), .001)

    def test_ciede2000_known_hue_wrap_pair_and_symmetry(self):
        first = np.array([[50., 2.6772, -79.7751], [50., 0., 0.]])
        second = np.array([[50., 0., -82.7485], [50., 0., 0.]])
        result = ciede2000(first, second)
        np.testing.assert_allclose(result, [2.0425, 0.], atol=5e-5)
        np.testing.assert_allclose(result, ciede2000(second, first), atol=1e-12)

    def test_zero_variance_grain_correlation_is_unavailable(self):
        region = {"id": "flat", "role": "flat_field", "geometry": [0, 0, 32, 24], "measurements": ["grain"]}
        protocol = {**self.protocol, "measurements": [], "grain": {"residual": {"method": "flat_mean", "border_px": 0, "high_pass_radius_px": 2}, "spectrum": {"window": "hann"}}}
        zero = ImageData(np.zeros((24, 32, 3)), "sRGB", "linear")
        result = compare_images(zero, zero, [region], protocol, self.root / "grain")
        correlations = [row for row in result["measurements"] if "correlation" in row["metric"]]
        self.assertTrue(correlations)
        self.assertTrue(all(row["availability"] == "unavailable" for row in correlations))
        self.assertTrue(any(artifact["role"] == "grain_radial_spectrum" for artifact in result["artifacts"]))

    def test_seed_aggregation_reports_actual_population(self):
        first = compare_images(self.base, self.candidate, [], self.protocol, self.root / "seed0")
        second = compare_images(self.base, self.candidate, [], {**self.protocol, "seed_population": [42]}, self.root / "seed42")
        rows = aggregate_seed_measurements([first, second])
        target = next(row for row in rows if row["metric"] == "delta_l_star.mean.seed_std")
        self.assertEqual(target["seed_population"], [0, 42])
        self.assertEqual(target["seed_count"], 2)
        self.assertAlmostEqual(target["value"], 0)

    def test_failed_seed_prevents_reduced_population_aggregate(self):
        observed = {"case_id": "probe", "seed": 0, "measurements": [{"case_id": "probe", "region_id": "flat", "metric": "grain.rms", "domain": "linear", "units": "RMS", "estimator": "flat", "availability": "available", "value": 2., "seed_population": [0], "artifact_refs": []}]}
        missing = {"case_id": "probe", "seed": 42, "status": "failed", "measurements": []}
        rows = aggregate_seed_measurements([observed, missing])
        self.assertTrue(all(row["availability"] == "unavailable" and row["reason"] == "missing_seed_measurement" for row in rows))
        self.assertEqual(rows[0]["expected_seed_population"], [0, 42])

    def test_observed_violation_precedes_missing_seed(self):
        rule = {"id": "protect", "selector": {"case_id": "probe", "region_id": "skin"}, "metric": "delta_e00.p95", "required": True, "max": 1.}
        rows = [{"case_id": "probe", "region_id": "skin", "metric": "delta_e00.p95", "availability": "available", "value": 2.}, {"case_id": "probe", "region_id": "skin", "metric": "delta_e00.p95", "availability": "unavailable"}]
        self.assertEqual(rule_result(rule, rows)["status"], "failed")

    def test_protocol_identity_changes_with_mask_and_seed(self):
        suite = {"protocol": {}, "cases": [{"id": "probe", "seeds": [0], "regions": [{"id": "skin", "geometry": {"path": "a.png", "sha256": "a"*64}}]}]}
        before = protocol_digest(suite)
        suite["cases"][0]["regions"][0]["geometry"]["path"] = "other.png"
        self.assertEqual(protocol_digest(suite), before)
        suite["cases"][0]["seeds"].append(42)
        self.assertNotEqual(protocol_digest(suite), before)
        suite["cases"][0]["seeds"] = [0]
        suite["cases"][0]["regions"][0]["geometry"]["sha256"] = "b"*64
        self.assertNotEqual(protocol_digest(suite), before)

    def reference_case(self, scope):
        path = self.root / "reference.png"
        Image.fromarray(np.full((24, 32, 3), 64, np.uint8)).save(path)
        return {"id": "probe", "regions": [{"id": "patch", "geometry": [0, 0, 32, 24], "measurements": []}],
                "reference": {"id": "scan", "path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "scope": scope,
                              "color": {"color_space": "sRGB", "cctf_decoding": True},
                              "processing": {"grading": "none", "histogram_matching": "none"}, "provenance": {"redistribution": "permitted"}}}

    def test_visual_reference_emits_no_fidelity_score(self):
        result = compare_reference(self.base, self.candidate, self.reference_case("visual"), self.protocol, self.root / "visual", self.root)
        self.assertEqual(result["execution_status"], "completed")
        self.assertEqual(result["measurements"], [])
        self.assertTrue(result["reference_review"]["display_artifact_id"])

    def test_regional_reference_distance_direction(self):
        case = self.reference_case("regional")
        case["reference"]["regions"] = [{"id": "patch", "geometry": [0, 0, 32, 24]}]
        result = compare_reference(self.base, self.candidate, case, self.protocol, self.root / "regional", self.root)
        self.assertEqual(result["execution_status"], "completed")
        metric = next(row for row in result["measurements"] if row["metric"] == "reference.patch_delta_e00.distance_delta")
        self.assertLess(metric["value"], 0)

    def test_aligned_tone_matched_reference_does_not_score_tone(self):
        case = self.reference_case("aligned_scene")
        mask = self.root / "mask.png"
        Image.fromarray(np.full((24, 32), 255, np.uint8)).save(mask)
        transform = self.root / "transform.json"
        transform.write_text(json.dumps({"matrix": np.eye(3).tolist(), "direction": "reference_to_render", "coordinate_system": "pixel_centers", "interpolation": "nearest"}))
        review = self.root / "review.json"
        review.write_text(json.dumps({"status": "approved", "reviewer": "fixture", "excluded_regions": [], "limitations": []}))
        for key, path in (("transform", transform), ("common_area_mask", mask), ("correspondence_review", review)):
            case["reference"][key] = {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        case["reference"]["processing"]["histogram_matching"] = "brightness_only"
        result = compare_reference(self.base, self.candidate, case, self.protocol, self.root / "aligned", self.root)
        tones = [row for row in result["measurements"] if "tone_l_star" in row["metric"] or "delta_e00" in row["metric"]]
        self.assertTrue(tones)
        self.assertTrue(all(row["availability"] == "unavailable" for row in tones))

    def test_gate_requires_policy_without_loading_any_images(self):
        result = subprocess.run([sys.executable, str(MODULES / "spektrafilm_bench.py"), "compare", "--baseline", "missing", "--candidate", "missing", "--out", str(self.root / "out"), "--gate"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("--gate requires --policy", result.stderr)
        self.assertFalse((self.root / "out").exists())


if __name__ == "__main__":
    unittest.main()
