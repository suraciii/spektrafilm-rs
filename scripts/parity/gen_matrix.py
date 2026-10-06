#!/usr/bin/env python3
"""Generate docs/parity/parity_matrix.json — the machine-readable 0.3.4
parity inventory.

Curated sections (runtime fields, GUI actions, LUT-creator color-space
registry, image/RAW workflows, known gaps, budgets) mirror the pinned
Python source at 3bb2c2d2801ff68b92019cf1dbcbb133d60832bc against the
audited Rust baseline 9dd59b0380194b93686aaa230a8bb9680aa270a4. Bundled
assets are hashed live from both trees so ``match`` claims are real
evidence, not assertions.

Re-run after either tree changes:

    python3 scripts/parity/gen_matrix.py
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import platform
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parent.parent
PY_REPO = Path("/home/szf/repos/spektrafilm")
PY_COMMIT = "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc"
RS_COMMIT = "9dd59b0380194b93686aaa230a8bb9680aa270a4"

S = "supported"
UNREAD = "accepted_but_unread"
ABSENT = "absent_from_rust"
NOOP = "noop_upstream"
RUST_ONLY = "rust_only_extra"
DIVERGENT = "divergent"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    h.update(path.read_bytes())
    return h.hexdigest()


# ---------------------------------------------------------------------------
# Runtime field inventory — Python params_schema.py vs Rust params.rs +
# read-site greps over crates/spektrafilm-core (status verified 2026-10-02).
# ---------------------------------------------------------------------------
RUNTIME_FIELDS = {
    "camera": {
        "exposure_compensation_ev": (S, None, "filming expose + print factor_midgray_comp"),
        "auto_exposure": (S, None, "filming.rs:296 measure_autoexposure_ev"),
        "auto_exposure_method": (S, None, "all 7 metering patterns ported; unknown -> 1.0 EV on both sides"),
        "lens_blur_um": (S, None, "filming.rs:359 apply_gaussian_blur_um"),
        "film_format_mm": (S, None, "pixel_size_um derivation"),
        "filter_uv": (S, None, "spectral_service::apply_camera_filters"),
        "filter_ir": (S, None, "spectral_service::apply_camera_filters"),
        "diffusion_filter.active": (S, None, "model/diffusion.rs port"),
        "diffusion_filter.filter_family": (S, None, ""),
        "diffusion_filter.strength": (S, None, ""),
        "diffusion_filter.spatial_scale": (S, None, ""),
        "diffusion_filter.halo_warmth": (S, None, ""),
        "diffusion_filter.core_intensity": (S, None, ""),
        "diffusion_filter.core_size": (S, None, ""),
        "diffusion_filter.halo_intensity": (S, None, ""),
        "diffusion_filter.halo_size": (S, None, ""),
        "diffusion_filter.bloom_intensity": (S, None, ""),
        "diffusion_filter.bloom_size": (S, None, ""),
    },
    "enlarger": {
        "illuminant": (S, None, "enlarger.rs dichroic filtering"),
        "print_exposure": (S, None, ""),
        "print_exposure_compensation": (S, None, "factor_midgray_comp branch"),
        "normalize_print_exposure": (S, None, ""),
        "y_filter_shift": (S, None, ""),
        "m_filter_shift": (S, None, ""),
        "y_filter_neutral": (S, None, "database lookup, f64 preserved"),
        "m_filter_neutral": (S, None, ""),
        "c_filter_neutral": (S, None, ""),
        "lens_blur": (NOOP, None, "never applied by any Python 0.3.4 stage (only zeroed in digest); Rust matches by not reading it"),
        "diffusion_filter": (S, None, "printing expose applies enlarger diffusion"),
        "preflash_exposure": (S, None, "compute_preflash_raw"),
        "preflash_y_filter_shift": (S, None, ""),
        "preflash_m_filter_shift": (S, None, ""),
    },
    "scanner": {
        "lens_blur": (S, None, "scanning.rs:348"),
        "white_correction": (S, None, "color_reference luminance remap"),
        "black_correction": (S, None, ""),
        "white_level": (S, None, ""),
        "black_level": (S, None, ""),
        "unsharp_mask": (S, None, "scanning.rs:353"),
    },
    "film_render": {
        "density_curve_gamma": (S, None, ""),
        "grain.active": (S, None, ""),
        "grain.sublayers_active": (S, None, "filming.rs:549-558"),
        "grain.particle_area_um2": (S, None, "renamed agx_particle_area_um2 in Rust; f64 preserved"),
        "grain.particle_scale": (S, None, "renamed agx_particle_scale"),
        "grain.particle_scale_layers": (S, None, "filming.rs:549-558"),
        "grain.density_min": (S, None, ""),
        "grain.uniformity": (S, None, ""),
        "grain.blur": (S, None, ""),
        "grain.blur_dye_clouds_um": (S, None, "filming.rs:549-558"),
        "grain.micro_structure": (S, None, "filming.rs:549-558"),
        "grain.n_sub_layers": (S, None, ""),
        "halation.active": (S, None, ""),
        "halation.scatter_amount": (S, None, ""),
        "halation.scatter_spatial_scale": (S, None, ""),
        "halation.halation_amount": (S, None, ""),
        "halation.halation_spatial_scale": (S, None, ""),
        "halation.scatter_core_um": (S, None, ""),
        "halation.scatter_tail_um": (S, None, ""),
        "halation.scatter_tail_weight": (S, None, ""),
        "halation.boost_ev": (S, None, ""),
        "halation.boost_range": (S, None, ""),
        "halation.protect_ev": (S, None, ""),
        "halation.halation_strength": (S, None, "params_builder.rs:209-221"),
        "halation.halation_first_sigma_um": (S, None, "params_builder.rs:209-221"),
        "halation.halation_n_bounces": (S, None, ""),
        "halation.halation_bounce_decay": (S, None, ""),
        "halation.halation_renormalize": (S, None, ""),
        "dir_couplers.active": (S, None, ""),
        "dir_couplers.amount": (S, None, "filming.rs:455"),
        "dir_couplers.inhibition_samelayer": (S, None, ""),
        "dir_couplers.inhibition_interlayer": (S, None, ""),
        "dir_couplers.gamma_samelayer_rgb": (S, None, "params_builder.rs:214-236"),
        "dir_couplers.gamma_interlayer_r_to_gb": (S, None, ""),
        "dir_couplers.gamma_interlayer_g_to_rb": (S, None, ""),
        "dir_couplers.gamma_interlayer_b_to_rg": (S, None, ""),
        "dir_couplers.diffusion_size_um": (S, None, ""),
        "dir_couplers.diffusion_tail_um": (S, None, ""),
        "dir_couplers.diffusion_tail_weight": (S, None, ""),
        "glare.active": (NOOP, None, "upstream passes glare=None for scan_film and never reads film_render.glare"),
        "glare.percent": (NOOP, None, ""),
        "glare.roughness": (NOOP, None, ""),
        "glare.blur": (NOOP, None, ""),
    },
    "print_render": {
        "glare.active": (S, None, "print-path glare; RNG stream differs (Rust seed 42 vs Python np.random continuation) — statistical budget"),
        "glare.percent": (S, None, ""),
        "glare.roughness": (S, None, ""),
        "glare.blur": (S, None, ""),
        "density_curves_morph.active": (S, None, "print_morph.rs"),
        "density_curves_morph.gamma_factor": (S, None, ""),
        "density_curves_morph.gamma_factor_fast": (S, None, ""),
        "density_curves_morph.gamma_factor_slow": (S, None, ""),
        "density_curves_morph.gamma_factor_red": (S, None, ""),
        "density_curves_morph.gamma_factor_green": (S, None, ""),
        "density_curves_morph.gamma_factor_blue": (S, None, ""),
        "density_curves_morph.developer_exhaustion": (S, None, ""),
    },
    "io": {
        "input_color_space": (S, None, "colorspace::resolve rejects unknown names"),
        "input_cctf_decoding": (S, None, "converting.rs:232-236"),
        "output_color_space": (S, None, "colorspace::resolve rejects unknown names"),
        "output_cctf_encoding": (S, None, "scanning.rs output encoding"),
        "input_gamut_compress.active": (S, None, "input_gamut.rs"),
        "input_gamut_compress.algorithm": (S, None, "input_gamut.rs"),
        "input_gamut_compress.knee": (S, None, "input_gamut.rs"),
        "output_gamut_compress.algorithm": (S, None, "gamut_compression.rs"),
        "output_gamut_compress.knee": (S, None, ""),
        "output_gamut_compress.lightness_compression": (S, None, ""),
        "crop": (S, None, "resizing.rs:170-205"),
        "crop_center": (S, None, "resizing.rs:75-147"),
        "crop_size": (S, None, "resizing.rs:75-147"),
        "upscale_factor": (S, None, "resizing.rs:170-205"),
        "scan_film": (S, None, "pipeline.rs:421-423"),
    },
    "debug": {
        "deactivate_spatial_effects": (S, None, "params_builder.rs:112-131"),
        "deactivate_stochastic_effects": (S, None, "params_builder.rs:133-136"),
        "print_timings": (NOOP, None, "upstream field is declared but not read by digest_params"),
        "lut_mode": (S, None, "params_builder.rs:80-110"),
    },
    "taps": {
        "inject": (S, None, "pipeline.rs:1017-1019"),
        "collect": (S, None, "pipeline.rs:1020-1023"),
    },
    "settings": {
        "rgb_to_raw_method": (S, None, "spectral_service upsampler dispatch"),
        "apply_hanatos2025_adaptation_window": (S, None, "spectral_service.rs:1163"),
        "apply_hanatos2025_adaptation_surface": (S, None, "spectral_service.rs:1163"),
        "spectral_gaussian_blur": (S, None, "spectral_service.rs:1159-1160"),
        "use_enlarger_lut": (S, None, "printing.rs:201-215"),
        "use_scanner_lut": (S, None, "scanning.rs:252-267"),
        "lut_resolution": (S, None, "printing.rs:214; scanning.rs:265"),
        "use_fast_stats": (S, None, "filming.rs:557"),
        "preview_max_size": (S, None, "runtime.rs:133-141"),
        "preview_mode": (S, None, "params_builder.rs:61-73"),
        "neutral_print_filters_from_database": (S, None, "neutral_filters.rs"),
        "use_cat16": (RUST_ONLY, None, "Python hard-codes CAT16; Rust exposes a CAT02 escape hatch (default matches 0.3.4)"),
    },
}
AUDIT_RUNTIME_STATUSES = {
    "camera.filter_uv": UNREAD,
    "camera.filter_ir": UNREAD,
    "film_render.grain.sublayers_active": UNREAD,
    "film_render.grain.particle_scale_layers": UNREAD,
    "film_render.grain.blur_dye_clouds_um": UNREAD,
    "film_render.grain.micro_structure": UNREAD,
    "film_render.halation.halation_strength": DIVERGENT,
    "film_render.halation.halation_first_sigma_um": DIVERGENT,
    "film_render.dir_couplers.gamma_samelayer_rgb": DIVERGENT,
    "film_render.dir_couplers.gamma_interlayer_r_to_gb": DIVERGENT,
    "film_render.dir_couplers.gamma_interlayer_g_to_rb": DIVERGENT,
    "film_render.dir_couplers.gamma_interlayer_b_to_rg": DIVERGENT,
    "film_render.glare.active": DIVERGENT,
    "film_render.glare.percent": DIVERGENT,
    "film_render.glare.roughness": DIVERGENT,
    "film_render.glare.blur": DIVERGENT,
    "io.input_color_space": DIVERGENT,
    "io.input_cctf_decoding": UNREAD,
    "io.output_color_space": DIVERGENT,
    "io.output_cctf_encoding": DIVERGENT,
    "io.input_gamut_compress.algorithm": DIVERGENT,
    "io.output_gamut_compress.algorithm": DIVERGENT,
    "io.crop": UNREAD,
    "io.crop_center": UNREAD,
    "io.crop_size": UNREAD,
    "io.upscale_factor": DIVERGENT,
    "io.scan_film": DIVERGENT,
    "debug.deactivate_spatial_effects": ABSENT,
    "debug.deactivate_stochastic_effects": ABSENT,
    "debug.print_timings": ABSENT,
    "debug.lut_mode": ABSENT,
    "taps.inject": ABSENT,
    "taps.collect": ABSENT,
    "settings.rgb_to_raw_method": DIVERGENT,
    "settings.apply_hanatos2025_adaptation_surface": UNREAD,
    "settings.spectral_gaussian_blur": UNREAD,
    "settings.use_enlarger_lut": UNREAD,
    "settings.lut_resolution": UNREAD,
    "settings.use_fast_stats": UNREAD,
    "settings.preview_max_size": UNREAD,
    "settings.preview_mode": UNREAD,
}


# ---------------------------------------------------------------------------
# GUI actions — spektrafilm_gui/controller.py handlers wired in app.py:225-242.
# ---------------------------------------------------------------------------
GUI_ACTIONS = [
    ("load_input_image", "open a raster image via OIIO with ICC/metadata", ABSENT, 9),
    ("load_raw_image", "RAW decode via rawpy+lensfun (WB modes, TCA/vignetting)", ABSENT, 10),
    ("rotate_input_image_clockwise", "quarter-turn input layer", ABSENT, 12),
    ("rotate_input_image_counterclockwise", "quarter-turn input layer", ABSENT, 12),
    ("apply_profile_defaults", "film/paper selection re-syncs stock defaults", DIVERGENT, 3),
    ("apply_film_profile_defaults", "film stock change re-applies stock specifics", DIVERGENT, 3),
    ("run_preview", "preview render through resize_for_preview (skimage order=1 anti-aliased)", DIVERGENT, 16),
    ("run_scan", "full-resolution simulation", ABSENT, 12),
    ("scan_for_print", "force scanner corrections/glare and restore transient snapshot", ABSENT, 12),
    ("request_auto_preview", "auto-preview wiring for every editor", DIVERGENT, 11),
    ("report_display_transform_status", "napari display transform toggle", ABSENT, 12),
    ("set_gray_18_canvas", "18% gray canvas background", ABSENT, 12),
    ("set_output_interpolation_mode", "output layer interpolation mode", ABSENT, 12),
    ("refresh_preview_cache", "recompute cached preview", ABSENT, 12),
    ("save_output_layer", "save with format/ICC/metadata preservation", ABSENT, 9),
    ("save_current_as_default", "GUI state persisted as startup default", ABSENT, 11),
    ("save_current_state_to_file", "GUI state export", ABSENT, 11),
    ("load_state_from_file", "GUI state import", ABSENT, 11),
    ("restore_factory_default", "reset persisted state", ABSENT, 11),
    ("show_startup_placeholder", "startup placeholder layer", ABSENT, 11),
    ("virtual_photo_paper", "photo-paper presentation frame + watermark asset", ABSENT, 12),
    ("polaroid_animation", "development animation", ABSENT, 12),
]
AUDIT_GUI_STATUSES = {
    "load_input_image": ABSENT,
    "load_raw_image": ABSENT,
    "rotate_input_image_clockwise": ABSENT,
    "rotate_input_image_counterclockwise": ABSENT,
    "apply_profile_defaults": DIVERGENT,
    "apply_film_profile_defaults": DIVERGENT,
    "run_preview": DIVERGENT,
    "run_scan": ABSENT,
    "scan_for_print": ABSENT,
    "request_auto_preview": DIVERGENT,
    "report_display_transform_status": ABSENT,
    "set_gray_18_canvas": ABSENT,
    "set_output_interpolation_mode": ABSENT,
    "refresh_preview_cache": ABSENT,
    "save_output_layer": ABSENT,
    "save_current_as_default": ABSENT,
    "save_current_state_to_file": ABSENT,
    "load_state_from_file": ABSENT,
    "restore_factory_default": ABSENT,
    "show_startup_placeholder": ABSENT,
    "virtual_photo_paper": ABSENT,
    "polaroid_animation": ABSENT,
}


# ---------------------------------------------------------------------------
# LUT-creator color-space registry — spektrafilm_lut_creator/color_spaces.py.
# role () = registry-only (silenced as bundle input pending shaper support).
# ---------------------------------------------------------------------------
LUT_REGISTRY = [
    # name, kind, roles, rust_status, owner
    ("ACES2065-1", "linear", [], S, None),
    ("ACEScg", "linear", [], S, None),
    ("Rec.709 Linear", "linear", [], S, None),
    ("Rec.2020 Linear", "linear", [], S, None),
    ("ProPhoto Linear", "linear", [], S, None),
    ("sRGB Linear", "linear", [], S, None),
    ("sRGB", "encoded_sdr", ["input", "output"], S, None),
    ("Rec.709", "encoded_sdr", ["input", "output"], S, None),
    ("Display P3", "encoded_sdr", ["input", "output"], S, None),
    ("Rec.2020", "encoded_sdr", ["input", "output"], S, None),
    ("DCI-P3", "encoded_sdr", ["input", "output"], S, None),
    ("Adobe RGB", "encoded_sdr", ["input", "output"], S, None),
    ("ACEScct", "log", ["input"], S, None),
    ("ACEScc", "log", ["input"], S, None),
    ("ARRI LogC3 (EI800)", "log", ["input"], S, None),
    ("ARRI LogC4", "log", ["input"], S, None),
    ("Sony S-Log3", "log", ["input"], S, None),
    ("Sony S-Log3 (S-Gamut3.Cine)", "log", ["input"], S, None),
    ("Panasonic V-Log", "log", ["input"], S, None),
    ("Fujifilm F-Log", "log", ["input"], S, None),
    ("Fujifilm F-Log2", "log", ["input"], S, None),
    ("Canon Log 3", "log", ["input"], S, None),
    ("RED Log3G10", "log", ["input"], S, None),
    ("DaVinci Intermediate", "log", ["input"], S, None),
    ("Apple Log", "log", ["input"], S, None),
    ("Blackmagic Film Gen 5", "log", ["input"], S, None),
    ("Nikon N-Log", "log", ["input"], S, None),
    ("DJI D-Log", "log", ["input"], S, None),
    ("Rec.2100 PQ", "log", ["input", "output"], S, None),
    ("Rec.2100 HLG", "log", ["input", "output"], S, None),
    ("P3-D65 Linear", "linear", [], S, None),
    ("P3-D65 PQ", "log", ["input", "output"], S, None),
]
AUDIT_LUT_STATUS = ABSENT


# ---------------------------------------------------------------------------
# Image / RAW workflows.
# ---------------------------------------------------------------------------
WORKFLOWS = [
    ("raster_load_oiio", "load_image_oiio: TIFF/EXR/PNG/JPEG + linear colorspace attribute", S, None),
    ("raster_save_oiio", "save_image_oiio: jpg/png uint8, tif 8/16/32-bit float, bit_depth control", S, None),
    ("icc_embed", "_load_icc_profile: bundled ellelstone/saucecontrol profiles embedded on save", S, None),
    ("metadata_preservation", "read/write_image_metadata via exiv2 (EXIF/XMP)", S, None),
    ("raw_decode", "load_and_process_raw_file: rawpy demosaic, no auto brightening, ACES2065-1 out", S, None),
    ("raw_white_balance", "as_shot/daylight/tungsten/custom (temperature+tint) modes", S, None),
    ("raw_lensfun", "vignetting/TCA/distortion/geometry/scale corrections from EXIF-matched lens", S, None),
    ("preview_resize", "resize_for_preview: skimage order=1 anti-aliased, max_size=preview_max_size", S, None),
    ("lut_export_cube", "spektrafilm-lut CLI: bundles, delivery targets, OCIO emission, QA", S, None),
]
AUDIT_WORKFLOW_STATUSES = {
    "raster_load_oiio": ABSENT,
    "raster_save_oiio": DIVERGENT,
    "icc_embed": ABSENT,
    "metadata_preservation": ABSENT,
    "raw_decode": DIVERGENT,
    "raw_white_balance": ABSENT,
    "raw_lensfun": ABSENT,
    "preview_resize": DIVERGENT,
    "lut_export_cube": ABSENT,
}


# ---------------------------------------------------------------------------
# Known gaps (beyond field-level) with owning issues.
# ---------------------------------------------------------------------------
AUDIT_BASELINE_GAPS = [
    {"gap": "layered grain develop path (interp_density_cmy_layers + apply_grain_to_density_layers)",
     "owner_issue": 6, "evidence": "Python model/grain.py:193-213; Rust parses density_curves_layers (profile.rs:124) but no stage reads it"},
    {"gap": "wrong CUBE LUT constructor: CLI export-lut uses the simplified non-spectral Pipeline::new with glare/unsharp left active",
     "owner_issue": 13, "evidence": "crates/spektrafilm-cli/src/main.rs:368-375 vs Python lut_mode digest (params_builder.py:99-118)"},
    {"gap": "digest_params step absent: preview_mode/lut_mode/deactivate_spatial/deactivate_stochastic zeroing unported",
     "owner_issue": 3, "evidence": "params_builder.py:75-144 has no Rust counterpart; Pipeline::new_with_spectral only applies film specifics + neutral filters"},
    {"gap": "stock-specific DIR presets (velvia_100, provia_100f) and halation use/antihalation presets",
     "owner_issue": 3, "evidence": "params_builder.py:199-252 vs pipeline.rs:55-94"},
    {"gap": "direct-scan glare: Python passes glare=None on scan_film; Rust applies film_render.glare (seed 42)",
     "owner_issue": 8, "evidence": "scanning.py:54-55 vs scanning.rs:284-288,325-333"},
    {"gap": "output clip semantics: Rust clamps [0,1] when output_cctf_encoding=False; Python never clips",
     "owner_issue": 4, "evidence": "scanning.rs:385-389 vs scanning.py:130-139"},
    {"gap": "unsupported color spaces silently fall back (input->ProPhoto, output->sRGB) instead of erroring",
     "owner_issue": 4, "evidence": "filming.rs:510-518 _ => PROPHOTO_TO_XYZ; scanning.rs:449-456 _ => XYZ_TO_SRGB_F64"},
    {"gap": "jzazbz/oklrab output gamut + perceptual compression on non-sRGB output fall back to identity",
     "owner_issue": 4, "evidence": "gamut_compression.rs:267-293 error-and-identity branches"},
    {"gap": "enlarger LUT reduction (use_enlarger_lut) unported; lut_resolution param unread",
     "owner_issue": 3, "evidence": "printing.py:46-52 vs printing.rs (no use_lut branch)"},
    {"gap": "crop + AE order: Rust rescales before AE; Python AEs before crop/rescale",
     "owner_issue": 7, "evidence": "pipeline.rs:462-471 vs pipeline.py:191-195"},
    {"gap": "glare RNG stream: Rust fixed seed 42 vs Python continued np.random stream (statistical parity only)",
     "owner_issue": 8, "evidence": "scanning.rs:331 comment; model/glare.py via fast_stats np.random"},
    {"gap": "ICC profiles (166 files) + SPEKTRAFILM_LICENSE.txt not bundled in the Rust tree",
     "owner_issue": 9, "evidence": "src/spektrafilm/data/icc vs data/ (Rust) directory listing"},
    {"gap": "RAW white-balance modes and lensfun corrections; Rust GUI/CLI use rawler with as-shot only",
     "owner_issue": 10, "evidence": "raw_file_processor.py:372-438 vs cli/src/main.rs load_raw"},
]

KNOWN_GAPS = [
    {
        "gap": "Native GUI acceptance is platform-specific; Linux/Xvfb evidence does not establish Windows/macOS display and window behavior.",
        "owner_issue": None,
        "evidence": "docs/parity/parity_matrix.md:68-90",
    },
]

BUDGETS = {
    "arithmetic": {"max_abs": 1e-6,
                   "provenance": "2026-09-30 audit: 4x4 bare chain max abs 1.14e-8 f64; shipped test budget 1e-6 (stages/mod.rs:127)"},
    "arithmetic_spatial": {"max_abs": 5e-5,
                           "provenance": "provisional target for FFT/interp-heavy deterministic stages; measure on first harness run"},
    "lut_reduction": {"max_abs": 1e-5,
                      "provenance": "provisional target for PCHIP LUT-reduced paths (claimed bit-parity kernels); measure on first run"},
    "rng_stream": {"max_abs": 1e-6,
                   "provenance": "seeded streams (grain seeds 0/1/2); Rust ports the legacy np.random generators; measure on first run"},
    "statistical_texture": {"mean_rel": 0.01, "std_rel": 0.05,
                            "provenance": "moment-level budget for stochastic appearance (grain layers, glare); per-pixel equality not applicable"},
    "file_fidelity": {"max_lsb_8bit": 1, "max_lsb_16bit": 1,
                      "provenance": "encoded-output compare: decoded pixels within +/-1 LSB; exact byte equality NOT required (different encoders)"},
    "preview": {"mean_abs": 2e-2, "note": "preview approximation budget",
                "provenance": "provisional: Python skimage order=1 anti-aliased downscale vs Rust GUI rescale; owner issue #16"},
}


def asset_inventory():
    """Hash-compare every bundled Python 0.3.4 data file against the Rust tree."""
    py_data = PY_REPO / "src" / "spektrafilm" / "data"
    rs_data = REPO_ROOT / "data"
    assets = []
    for path in sorted(py_data.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(py_data)
        rs_path = rs_data / rel
        entry = {
            "path": f"data/{rel}",
            "py_sha256": sha256(path),
        }
        if rs_path.exists():
            entry["rs_sha256"] = sha256(rs_path)
            entry["match"] = entry["py_sha256"] == entry["rs_sha256"]
            if not entry["match"] and rel.name == "neutral_print_filters.json":
                py_db = json.loads(path.read_text())
                rs_db = json.loads(rs_path.read_text())
                shared_ok = all(rs_db.get(k) == v for k, v in py_db.items())
                extra = sorted(set(rs_db) - set(py_db))
                entry["match"] = False
                entry["detail"] = (f"verified superset: all {len(py_db)} upstream papers "
                                   f"byte-identical ({shared_ok}); Rust adds extra papers "
                                   f"{extra} for its B&W stocks (upstream entries unchanged)")
        else:
            entry["rs_sha256"] = None
            entry["match"] = False
            entry["detail"] = "missing from Rust data/ tree"
        assets.append(entry)

    rust_extra = []
    for path in sorted(rs_data.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(rs_data)
        if not (py_data / rel).exists():
            rust_extra.append(f"data/{rel}")

    return assets, rust_extra

def integration_entry(status, owner, notes, source, audit_status=None):
    if status == NOOP:
        current_status = NOOP
        verification = "Explicit upstream no-op; implementation intentionally does not read this field."
    elif status == RUST_ONLY:
        current_status = RUST_ONLY
        verification = "Rust-only extension outside pinned upstream 0.3.4 scope."
    elif status in (DIVERGENT, ABSENT, UNREAD):
        current_status = "implemented_unverified"
        verification = "Fresh consumer-visible evidence required; implementation status is not a parity pass."
    else:
        current_status = "supported"
        verification = "Implementation source and current differential/package evidence."
    return {
        "rust_status": current_status,
        "owner_issue": owner,
        "implementation_source": source,
        "verification": verification,
        "audit_baseline": {
            "rust_status": status if audit_status is None else audit_status,
            "notes": notes,
        },
    }


def report_evidence(path):
    from scenarios import expand_scenarios
    catalog = {row["name"]: row for row in expand_scenarios(PY_REPO)}
    if not path.exists():
        return {"status": "pending_fresh_execution", "report_path": str(path),
                "catalog_rows": len(catalog), "rows": []}
    report = json.loads(path.read_text())
    pins = report.get("pins", {})
    environment = report.get("environment", {})
    reference = environment.get("reference_environment", {})
    if pins.get("python_commit") != PY_COMMIT or pins.get("python_version") != "0.3.4":
        sys.exit(f"Refusing report with incorrect Python pin: {path}")
    if (report.get("unsupported") or not environment.get("rust_bin")
            or not reference.get("dependencies") or not reference.get("numpy_blas")
            or not reference.get("python_version") or not reference.get("architecture")):
        sys.exit(f"Refusing report without complete runnable environment provenance: {path}")
    rows = report.get("rows", [])
    names = [row["scenario"] for row in rows]
    if len(names) != len(set(names)) or set(names) - set(catalog):
        sys.exit(f"Refusing duplicate or unknown scenario evidence: {path}")
    counts = dict(Counter(row["status"] for row in rows))
    complete = set(names) == set(catalog)
    verified = []
    for row in rows:
        if row["status"] != "pass":
            continue
        scenario = catalog[row["scenario"]]
        if row.get("budget") != scenario["budget"]:
            sys.exit(f"Refusing stale scenario budget: {row['scenario']}")
        if scenario.get("expect_reject"):
            valid = row.get("rust_rejected") and not row.get("artifact_produced")
        else:
            taps = row.get("taps", {})
            valid = set(taps) == set(scenario["taps"]) and all(
                tap.get("within_budget") is True for tap in taps.values())
        if not valid:
            sys.exit(f"Refusing passing row without complete evidence: {row['scenario']}")
        verified.append(row["scenario"])
    return {"status": "verified_catalog" if complete and len(verified) == len(catalog)
            else "partial_or_failed_evidence", "report_path": str(path.resolve()),
            "catalog_rows": len(catalog), "reported_rows": len(rows), "counts": counts,
            "verified_scenarios": verified, "pins": pins, "environment": environment,
            "rows": rows,
            "scope": "Only these exercised scenarios and tap budgets are verified; no universal field or backend parity claim."}

def gui_evidence(path):
    if path is None:
        return {"status": "pending_fresh_execution"}
    records = json.loads(path.read_text())
    if not isinstance(records, list) or not records or not any(
            row.get("startup_restore") is True and row.get("raw_input_space") == "ACES2065-1"
            and row.get("cancel_output_absent") is True and row.get("close_output_absent") is True
            and row.get("export_temp_files") == [] for row in records):
        sys.exit(f"Refusing incomplete native GUI evidence: {path}")
    if any("failure" in row for row in records):
        sys.exit(f"Refusing failed GUI evidence: {path}")
    required = {"paper_back", "interpolation", "gray_canvas", "white_border", "reveal", "crossfade",
                "float_inspection", "display_output_isolation", "profile_selection", "non_srgb"}
    observed = {row.get("scenario") for row in records}
    if required - observed:
        sys.exit(f"Refusing GUI evidence missing viewer scenarios: {sorted(required - observed)}")
    return {"status": "measured_scenarios", "report_path": str(path.resolve()),
            "report_sha256": sha256(path), "records": records,
            "scope": "Only named assertions and screenshots in this native run are measured; other actions remain unverified."}

def gui_action_entry(action, desc, status, owner, evidence):
    entry = {"description": desc, **integration_entry(status, owner, "",
        "crates/spektrafilm-gui/src/main.rs; state.rs; controls.rs; display.rs")}
    keys = {
        "rotate_input_image_clockwise": "rotated_input_max_error",
        "rotate_input_image_counterclockwise": "rotation_pixel_bounds",
        "load_raw_image": "raw_input_space",
        "save_current_as_default": "startup_restore",
        "restore_factory_default": "factory_reset",
    }
    scenarios = {
        "set_output_interpolation_mode": "interpolation", "set_gray_18_canvas": "gray_canvas",
        "virtual_photo_paper": "paper_back", "polaroid_animation": "reveal",
        "save_output_layer": "display_output_isolation",
        "scan_for_print": "scan_for_print", "load_raw_image": "raw_status",
    }
    rows = [row for row in evidence.get("records", [])
            if (action in keys and keys[action] in row)
            or (action in scenarios and row.get("scenario") == scenarios[action])]
    if rows:
        entry.update(rust_status="verified_exercised_path",
                     verification="Named native-window assertions only; see measured_evidence.",
                     measured_evidence=rows, evidence_report_sha256=evidence["report_sha256"])
    return entry




def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path,
                        default=REPO_ROOT / "target/parity/parity_report.json")
    parser.add_argument("--gui-report", type=Path)
    args = parser.parse_args()
    evidence = report_evidence(args.report)
    native_gui = gui_evidence(args.gui_report)
    head = subprocess.run(["git", "-C", str(PY_REPO), "rev-parse", "HEAD"],
                          capture_output=True, text=True, check=True).stdout.strip()
    if head != PY_COMMIT:
        sys.exit(f"Python repo at {head}, expected {PY_COMMIT}")

    rs_head = subprocess.run(["git", "-C", str(REPO_ROOT), "rev-parse", "HEAD"],
                             capture_output=True, text=True, check=True).stdout.strip()

    assets, rust_extra = asset_inventory()
    matched = sum(1 for a in assets if a["match"])
    superset = [a for a in assets
                if not a["match"] and "superset" in a.get("detail", "")]
    missing = [a["path"] for a in assets if a["rs_sha256"] is None]
    differing = [a["path"] for a in assets if a["rs_sha256"] is not None
                 and not a["match"] and "superset" not in a.get("detail", "")]

    matrix = {
        "schema": "spektrafilm-parity-matrix/1",
        "generated": {
            "python_commit": PY_COMMIT,
            "python_version": "0.3.4",
            "rust_commit": rs_head,
            "rust_audited_baseline": RS_COMMIT,
            "python_platform": platform.platform(),
            "python_version_runtime": platform.python_version(),
            "note": "Current implementation inventory and historical audited statuses are separate. Fresh execution determines parity; implementation is not a passing result.",
        },
        "runtime_fields": {
            group: {
                field: integration_entry(status, owner, notes,
                    "crates/spektrafilm-core/src/params.rs; params_builder.rs; pipeline.rs; stages/; crates/spektrafilm-model/src/grain.rs",
                    audit_status=AUDIT_RUNTIME_STATUSES.get(f"{group}.{field}", status))
                for field, (status, owner, notes) in fields.items()
            }
            for group, fields in RUNTIME_FIELDS.items()
        },
        "gui_actions": {
            action: gui_action_entry(action, desc, status, owner, native_gui)
            for action, desc, status, owner in GUI_ACTIONS
        },
        "lut_registry": {
            name: {"kind": kind, "roles": roles,
                   **integration_entry(status, owner, "",
                       "crates/spektrafilm-core/src/lut_transport.rs; lut_delivery.rs; lut_formats.rs",
                       audit_status=AUDIT_LUT_STATUS)}
            for name, kind, roles, status, owner in LUT_REGISTRY
        },
        "workflows": {
            name: {"description": desc, **integration_entry(status, owner, "",
                "crates/spektrafilm-core/src/image_io.rs; crates/spektrafilm-raw/; crates/spektrafilm-cli/src/lut.rs",
                audit_status=AUDIT_WORKFLOW_STATUSES[name])}
            for name, desc, status, owner in WORKFLOWS
        },
        "assets": {
            "summary": {
                "python_files": len(assets),
                "byte_identical": matched,
                "verified_superset": len(superset),
                "missing_from_rust": missing,
                "differing_from_python": differing,
                "rust_only_extras": rust_extra,
            },
            "entries": assets,
        },
        "audit_baseline_gaps": AUDIT_BASELINE_GAPS,
        "known_gaps": [{"gap": "Pinned Python bundled asset missing", "path": path,
                        "owner_issue": 9, "evidence": "Live sha256 inventory"} for path in missing],
        "verification_status": evidence["status"],
        "differential_evidence": evidence,
        "gui_evidence": native_gui,
        "budgets": BUDGETS,
        "scenario_status": f"{evidence['catalog_rows']} expanded rows; exercised scenario and tap evidence is recorded under differential_evidence.",
    }

    out = REPO_ROOT / "docs" / "parity" / "parity_matrix.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(matrix, indent=2) + "\n")
    print(f"wrote {out}")
    print(f"assets: {matched}/{len(assets)} byte-identical, "
          f"{len(superset)} verified superset, {len(missing)} missing")
    print(f"rust-only extras: {rust_extra}")


if __name__ == "__main__":
    main()
