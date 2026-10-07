"""Shared scenario catalog for the Python 0.3.4 <-> Rust parity harness.

Single source of truth consumed by:

* ``py_reference.py``  — builds fixtures + per-tap f64 dumps with the pinned
  Python core (no GUI / LUT-creator imports);
* ``run_parity.py``    — drives the Rust f64 CLI over the same fixtures and
  compares per-stage / final max & mean errors against the budgets below.

Conventions
-----------
``params``
    Dotted-path overrides applied on top of ``init_params(film, paper)``
    defaults, BEFORE ``digest_params`` on the Python side. Paths use the
    *upstream 0.3.4 field names* (``params_schema.py``). The Rust runner
    forwards every control unchanged to strict runtime validation.

``taps``
    Pipeline boundaries compared. Python names come from
    ``spektrafilm.runtime.topology.Tap``. Rust produces them through the
    ``SPEKTRAFILM_DUMP_*`` env probes + ``--raw-out``:

    ============ ======================================================
    tap          Rust producer
    ============ ======================================================
    log_e_film   ``SPEKTRAFILM_DUMP_FILM_LOG_RAW``
    cmy_film     ``SPEKTRAFILM_DUMP_FILM_DENSITY``
    log_e_print  ``SPEKTRAFILM_DUMP_PRINT_LOG_RAW``
    cmy_print    ``SPEKTRAFILM_DUMP_PRINT_DENSITY``
    rgb_out      ``--raw-out``
    ============ ======================================================

    ``rgb_in`` is the shared fixture file itself (identical by
    construction) and ``rgb_pre`` equals it whenever auto-exposure and
    crop are off — the scenarios that keep them on compare at later taps
    only.

``budget``
    Key into ``BUDGETS``. Budgets are *targets with provenance*, not
    assertions of universal bit identity — see ``docs/parity/baseline_evidence.md``.

``status``
    ``expected_parity``  — both sides implement the control; the run must
                          meet the budget.
    Historical gaps are stored under ``audit_baseline``; all migrated
    scenarios now require parity and exceeding a budget fails the run.
    ``historical``       — measured evidence already exists; the run
                          reproduces it (regression guard).
"""

from __future__ import annotations

# --- pinned references -------------------------------------------------------
PY_COMMIT = "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc"  # spektrafilm 0.3.4
PY_VERSION = "0.3.4"
RS_COMMIT = "9dd59b0380194b93686aaa230a8bb9680aa270a4"  # audited Rust baseline

# The 28 upstream 0.3.4 film profiles (data/profiles, verified byte-identical
# in both trees — see docs/parity/parity_matrix.json assets).
UPSTREAM_PROFILES = [
    "fujifilm_c200",
    "fujifilm_crystal_archive_typeii",
    "fujifilm_pro_400h",
    "fujifilm_provia_100f",
    "fujifilm_velvia_100",
    "fujifilm_xtra_400",
    "kodak_2383",
    "kodak_2393",
    "kodak_ektachrome_100",
    "kodak_ektacolor_edge",
    "kodak_ektar_100",
    "kodak_endura_premier",
    "kodak_gold_200",
    "kodak_kodachrome_64",
    "kodak_portra_160",
    "kodak_portra_400",
    "kodak_portra_800",
    "kodak_portra_800_push1",
    "kodak_portra_800_push2",
    "kodak_portra_endura",
    "kodak_supra_endura",
    "kodak_ultra_endura",
    "kodak_ultramax_400",
    "kodak_verita_200d",
    "kodak_vision3_200t",
    "kodak_vision3_250d",
    "kodak_vision3_500t",
    "kodak_vision3_50d",
]

# Integration runtime uses canonical Python 0.3.4 wire names. Unsupported
# controls must reach Rust validation, rather than being dropped here.

# --- budgets -----------------------------------------------------------------
# Provenance for every number is recorded in docs/parity/baseline_evidence.md.
BUDGETS = {
    # Deterministic (non-stochastic) chain, f64 both sides. Historical audit
    # measured 1.14e-8 max abs on the 4x4 bare chain; the shipped regression
    # test keeps the 1e-6 f64 budget.
    "arithmetic": {"max_abs": 1e-6, "basis": "measured 1.14e-8 (2026-09-30 audit)"},
    # Deterministic chain through an FFT/interp-heavy spatial stage where
    # the two implementations may order reductions differently. Target,
    # to be measured on first run.
    "arithmetic_spatial": {"max_abs": 5e-5, "basis": "provisional — measure on first run"},
    # Deterministic chain evaluated through a PCHIP LUT reduction (both
    # sides reduce; the interpolation kernels are claimed bit-parity —
    # verify on first run).
    "lut_reduction": {"max_abs": 1e-5, "basis": "provisional — measure on first run"},
    # Seeded stochastic chain (grain seeds 0/1/2 are fixed upstream). Rust
    # ports the legacy np.random stream; target is the arithmetic budget,
    # bit-exactness expected but not asserted universally.
    "rng_stream": {"max_abs": 1e-6, "basis": "seeded stream; measure on first run"},
    # Statistical grain appearance: only distribution moments are comparable
    # (per-pixel values may diverge without visual drift).
    "statistical_texture": {
        "max_abs": None,
        "mean_rel": 0.01,
        "std_rel": 0.05,
        "basis": "moment-level budget — per-pixel comparison not applicable",
    },
    # Both implementations must refuse the same unsupported configurations.
    "rejection": {"both_reject": True,
                  "basis": "pinned 0.3.4 raises ValueError in eval_erf4_spectral_bandpass for empty window params (verified 2026-10-02)"},
}

# --- input generators --------------------------------------------------------
INPUTS = {
    # EXACTLY the fixed bare-chain reference grid from
    # crates/spektrafilm-core/src/stages/mod.rs::full_chain_matches_python_0_3_4_defaults
    # (v = 0.05 + 0.5 * ((i * 37) % 256) / 255, i = 0..w*h*3). Reusing the
    # formula preserves the lineage of the preserved 1.14e-8 evidence.
    "gradient4x4": {"kind": "mod37", "width": 4, "height": 4},
    "gradient32x32": {"kind": "mod37", "width": 32, "height": 32},
    "gradient512x512": {"kind": "mod37", "width": 512, "height": 512},
    "midgray1x1": {"kind": "const", "width": 1, "height": 1, "value": 0.184},
}

# Effects-off base: the "bare chain" configuration used by the preserved
# evidence and every arithmetic scenario.
BARE = {
    "camera.auto_exposure": False,
    "film_render.grain.active": False,
    "film_render.halation.active": False,
    "film_render.dir_couplers.active": False,
    "print_render.glare.active": False,
    "scanner.unsharp_mask": [0.0, 0.0],
    "io.input_color_space": "sRGB",
    "io.input_cctf_decoding": False,
    "io.output_color_space": "sRGB",
    "io.output_cctf_encoding": False,
}

PRINT_TAPS = ["log_e_film", "cmy_film", "log_e_print", "cmy_print", "rgb_out"]
SCAN_TAPS = ["log_e_film", "cmy_film", "rgb_out"]


def _scn(
    name,
    description,
    film,
    paper,
    inp,
    params,
    taps,
    budget,
    status,
    owner_issue=None,
    notes="",
    expand=None,
    expect_reject=False,
):
    return {
        "name": name,
        "description": name.replace("_", " ") + " parity check",
        "film": film,
        "paper": paper,
        "input": inp,
        "params": params,
        "taps": taps,
        "budget": budget,
        "status": "expected_parity" if status == "known_gap" else status,
        "owner_issue": owner_issue,
        "notes": "Integration implementation must meet the unchanged budget.",
        "audit_baseline": {"status": status, "notes": notes,
                           "description": description},
        "expand": expand,
        "expect_reject": expect_reject,
    }


def _bare_with(extra):
    d = dict(BARE)
    d.update(extra)
    return d


def build_scenarios():
    """Return the ordered scenario list (dicts)."""
    sc = []

    # ---- preserved baseline -------------------------------------------------
    sc.append(_scn(
        "bare_chain_4x4",
        "Fixed 4x4 bare-chain reference (effects off, sRGB linear in/out). "
        "Reproduces the preserved audit evidence (max abs ~1.14e-8, budget "
        "1e-6) — see docs/parity/baseline_evidence.md.",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({}), PRINT_TAPS, "arithmetic", "historical", None,
        "Same input formula and params as "
        "full_chain_matches_python_0_3_4_defaults in crates/spektrafilm-core/"
        "src/stages/mod.rs.",
    ))

    # ---- topology: print path vs direct film scan --------------------------
    sc.append(_scn(
        "scan_film_negative_4x4",
        "Direct film scan (scan_film=True) of a negative stock; printing "
        "topology skipped.",
        "kodak_portra_400", "kodak_portra_400", "gradient4x4",
        _bare_with({"io.scan_film": True}), SCAN_TAPS,
        "arithmetic", "expected_parity", None,
        "Paper profile is the film itself on the scan path (Rust CLI "
        "--scan-film uses the film profile for the scan stage).",
    ))
    sc.append(_scn(
        "positive_print_4x4",
        "Positive (slide) stock through the print path — exercises the "
        "positive DIR-coupler preset and slide scanning corrections.",
        "kodak_ektachrome_100", "kodak_portra_endura", "gradient4x4",
        _bare_with({}), PRINT_TAPS, "arithmetic", "expected_parity", None,
    ))
    sc.append(_scn(
        "positive_scan_film_4x4",
        "Direct scan of a positive cine stock (scan_film=True).",
        "kodak_vision3_50d", "kodak_vision3_50d", "gradient4x4",
        _bare_with({"io.scan_film": True}), SCAN_TAPS,
        "arithmetic", "expected_parity", None,
    ))

    # ---- upsamplers ---------------------------------------------------------
    sc.append(_scn(
        "upsampler_mallett2019_4x4",
        "settings.rgb_to_raw_method='mallett2019' reflectance-basis "
        "upsampler instead of the hanatos2025 tc LUT.",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({"settings.rgb_to_raw_method": "mallett2019"}),
        PRINT_TAPS, "arithmetic", "expected_parity", None,
    ))

    # ---- runtime LUT reductions --------------------------------------------
    sc.append(_scn(
        "enlarger_lut_reduction",
        "settings.use_enlarger_lut=True — printing expose evaluated through "
        "the 17^3 PCHIP enlarger LUT.",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({"settings.use_enlarger_lut": True}),
        PRINT_TAPS, "lut_reduction", "known_gap", 3,
        "Rust RuntimeParams accepts the field but never reads it: the "
        "printing stage always evaluates the direct spectral path "
        "(crates/spektrafilm-core/src/stages/printing.rs has no "
        "use_enlarger_lut branch; Python printing.py:51).",
    ))
    sc.append(_scn(
        "scanner_lut_reduction",
        "settings.use_scanner_lut=True — scan evaluated through the 17^3 "
        "PCHIP scanner LUT.",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({"settings.use_scanner_lut": True}),
        PRINT_TAPS, "lut_reduction", "expected_parity", None,
        "Rust scanning.rs:243 routes through scan_spectral_via_lut when "
        "use_scanner_lut is set.",
    ))

    # ---- color spaces ------------------------------------------------------
    for cs in ("ACES2065-1", "ProPhoto RGB", "ITU-R BT.2020"):
        sc.append(_scn(
            f"input_colorspace_{cs.lower().replace('.', '').replace(' ', '_').replace('-', '')}",
            f"io.input_color_space='{cs}' (linear in, no cctf decode).",
            "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
            _bare_with({"io.input_color_space": cs}),
            PRINT_TAPS, "arithmetic", "expected_parity", None,
        ))
    for cs in ("ProPhoto RGB", "ITU-R BT.2020", "ACES2065-1"):
        sc.append(_scn(
            f"output_colorspace_{cs.lower().replace('.', '').replace(' ', '_').replace('-', '')}",
            f"io.output_color_space='{cs}' (linear out, no cctf encode).",
            "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
            _bare_with({"io.output_color_space": cs}),
            PRINT_TAPS, "arithmetic", "expected_parity", None,
        ))
    sc.append(_scn(
        "output_encoding_srgb",
        "io.output_cctf_encoding=True — sRGB transfer applied at the scan "
        "stage.",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({"io.output_cctf_encoding": True}),
        ["rgb_out"], "arithmetic", "expected_parity", None,
        "Encoded-domain tap only; earlier taps are identical to "
        "bare_chain_4x4.",
    ))
    sc.append(_scn(
        "gamut_output_jzazbz",
        "io.output_gamut_compress.algorithm='jzazbz' on out-of-gamut input.",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({
            "io.output_gamut_compress.algorithm": "jzazbz",
            "io.output_gamut_compress.knee": [0.0, 1.0, 6.0],
            "io.output_gamut_compress.lightness_compression": [0.7, 1.0, 2.2],
        }),
        ["rgb_out"], "arithmetic", "known_gap", 4,
        "Rust gamut_compression.rs:280 logs 'jzazbz not yet ported' and "
        "falls back to identity; Python compresses "
        "(gamut_compression.py OutputGamutCompressSpec).",
    ))
    sc.append(_scn(
        "input_cctf_decoding",
        "io.input_cctf_decoding=True with sRGB input — Python decodes the "
        "sRGB transfer before the RGB->tc projection.",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({"io.input_cctf_decoding": True}),
        ["log_e_film", "rgb_out"], "arithmetic", "known_gap", 4,
        "io.input_cctf_decoding is accepted by Rust RuntimeParams but never "
        "read (grep over crates/spektrafilm-core: only tests and the CLI "
        "png/raw auto-detect set it).",
    ))

    # ---- ignored controls (baseline reproductions) --------------------------
    sc.append(_scn(
        "crop_controls",
        "io.crop=True with a centered 50% crop — Python's "
        "ResizingService.crop_and_rescale crops before filming.",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        _bare_with({
            "io.crop": True,
            "io.crop_center": [0.5, 0.5],
            "io.crop_size": [0.5, 0.5],
        }),
        ["rgb_out"], "arithmetic", "known_gap", 7,
        "io.crop/crop_center/crop_size are never read by the Rust core; "
        "output geometry itself differs (Python emits 16x16, Rust 32x32) — "
        "the harness reports a shape mismatch as the evidence.",
    ))
    sc.append(_scn(
        "uv_ir_filters",
        "camera.filter_uv=(0.5, 410, 8) — Python band-passes the film "
        "sensitivity before the tc-LUT integration (filming.py:99-104).",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({"camera.filter_uv": [0.5, 410.0, 8.0]}),
        ["log_e_film", "rgb_out"], "arithmetic", "known_gap", 5,
        "camera.filter_uv/filter_ir never read by the Rust core.",
    ))
    sc.append(_scn(
        "layered_grain",
        "Default grain (sublayers_active=True) on a 32x32 frame — Python "
        "develops through interp_density_cmy_layers + "
        "apply_grain_to_density_layers.",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        _bare_with({
            "film_render.grain.active": True,
            "film_render.grain.sublayers_active": True,
            "film_render.halation.active": False,
            "film_render.dir_couplers.active": False,
            "print_render.glare.active": False,
        }),
        ["cmy_film"], "rng_stream", "expected_parity", None,
        "Rust uses the same layered interpolation, per-layer particle scales, "
        "and seeded legacy np.random stream as the Python implementation.",
    ))
    sc.append(_scn(
        "grain_single_curve_seeded",
        "Grain with sublayers_active=False (the ported path), deterministic "
        "seeds 0/1/2 (grain.py:87).",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        _bare_with({
            "film_render.grain.active": True,
            "film_render.grain.sublayers_active": False,
            "film_render.halation.active": False,
            "film_render.dir_couplers.active": False,
            "print_render.glare.active": False,
        }),
        ["cmy_film"], "rng_stream", "expected_parity", None,
        "Rust ports the legacy np.random stream (spektrafilm-math "
        "numpy_rng) used by scipy.stats poisson/binomial rvs.",
    ))
    sc.append(_scn(
        "scan_film_glare",
        "Direct film scan with default glare flags — Python passes "
        "glare=None on the scan_film path (scanning.py:54-55); Rust applies "
        "film_render.glare (scanning.rs:284-288) with a fixed seed 42.",
        "kodak_portra_400", "kodak_portra_400", "gradient32x32",
        _bare_with({
            "io.scan_film": True,
            "film_render.grain.active": False,
            "film_render.halation.active": False,
            "film_render.dir_couplers.active": False,
            "film_render.glare.active": True,
        }),
        ["rgb_out"], "arithmetic", "known_gap", 8,
        "film_render.glare is a distinct upstream group from "
        "print_render.glare; upstream never applies it anywhere (no-op on "
        "the print path, explicitly None on scan_film).",
    ))
    sc.append(_scn(
        "preview_mode_digest",
        "settings.preview_mode=True — digest_params zeroes every spatial "
        "blur and disables grain before the pipeline is built.",
        "kodak_portra_400", "kodak_portra_endura", "gradient512x512",
        {
            "camera.auto_exposure": False,
            "film_render.grain.active": True,
            "film_render.halation.active": True,
            "film_render.dir_couplers.active": True,
            "print_render.glare.active": True,
            "scanner.unsharp_mask": [0.7, 0.7],
            "settings.preview_mode": True,
            "io.input_color_space": "sRGB",
            "io.input_cctf_decoding": False,
            "io.output_color_space": "sRGB",
            "io.output_cctf_encoding": False,
        },
        ["rgb_out"], "statistical_texture", "expected_parity", 3,
        "Rust accepts settings.preview_mode but has no digest step; the "
        "spatial/stochastic effects stay active.",
    ))
    sc.append(_scn(
        "preview_digest_deterministic",
        "Preview digestion with spatial controls enabled and stochastic glare off.",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        _bare_with({"settings.preview_mode": True,
                    "film_render.grain.active": True,
                    "film_render.halation.active": True,
                    "film_render.dir_couplers.active": True,
                    "camera.lens_blur_um": 8.0,
                    "scanner.unsharp_mask": [0.7, 0.7]}),
        PRINT_TAPS, "arithmetic", "expected_parity", 3,
    ))

    # ---- stock-specific overrides ------------------------------------------
    sc.append(_scn(
        "velvia_dir_preset",
        "fujifilm_velvia_100 with DIR couplers active — digest_params "
        "applies the velvia-specific DIR gammas (params_builder.py:202-206).",
        "fujifilm_velvia_100", "fujifilm_crystal_archive_typeii", "gradient32x32",
        _bare_with({
            "film_render.grain.active": False,
            "film_render.dir_couplers.active": True,
        }),
        ["cmy_film"], "arithmetic_spatial", "known_gap", 3,
        "Rust apply_film_specific_params covers only the generic "
        "positive/negative presets (pipeline.rs:55-66); the velvia/provia "
        "stock overrides are not ported.",
    ))
    sc.append(_scn(
        "halation_preset",
        "Halation active on a stock with use/antihalation tags — "
        "_apply_halation_preset seeds halation_first_sigma_um / "
        "halation_strength from the profile (params_builder.py:243-252).",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        _bare_with({
            "film_render.halation.active": True,
            "film_render.grain.active": False,
            "film_render.dir_couplers.active": False,
        }),
        ["cmy_film"], "arithmetic_spatial", "known_gap", 3,
        "No _HALATION_PRESETS port in Rust (grep over crates: only the "
        "profile.antihalation string is parsed).",
    ))

    # ---- non-default spatial settings --------------------------------------
    sc.append(_scn(
        "camera_lens_blur",
        "camera.lens_blur_um=8.0 — gaussian blur in um before halation.",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        _bare_with({
            "camera.lens_blur_um": 8.0,
            "camera.film_format_mm": 35.0,
        }),
        ["log_e_film", "rgb_out"], "arithmetic_spatial", "expected_parity", None,
    ))
    sc.append(_scn(
        "scanner_unsharp",
        "scanner.unsharp_mask=(1.2, 0.8) — non-default unsharp on the scan "
        "stage.",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        _bare_with({"scanner.unsharp_mask": [1.2, 0.8]}),
        ["rgb_out"], "arithmetic_spatial", "expected_parity", None,
    ))
    sc.append(_scn(
        "enlarger_diffusion_filter",
        "enlarger.diffusion_filter black_pro_mist 1/4 during print "
        "expose.",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        _bare_with({
            "enlarger.diffusion_filter.active": True,
            "enlarger.diffusion_filter.filter_family": "black_pro_mist",
            "enlarger.diffusion_filter.strength": 0.25,
        }),
        ["log_e_print", "cmy_print"], "arithmetic_spatial", "expected_parity", None,
    ))
    sc.append(_scn(
        "dir_couplers_spatial",
        "DIR couplers active (deterministic spatial diffusion, no RNG) on "
        "the default portra pair.",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        _bare_with({"film_render.dir_couplers.active": True}),
        ["cmy_film"], "arithmetic_spatial", "expected_parity", None,
    ))
    sc.append(_scn(
        "print_curves_morph",
        "print_render.density_curves_morph active with coupled-gamma "
        "morphing.",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({
            "print_render.density_curves_morph.active": True,
            "print_render.density_curves_morph.gamma_factor": 0.85,
            "print_render.density_curves_morph.gamma_factor_fast": 0.9,
            "print_render.density_curves_morph.gamma_factor_slow": 1.1,
            "print_render.density_curves_morph.developer_exhaustion": 0.2,
        }),
        ["cmy_print"], "arithmetic", "expected_parity", None,
    ))
    sc.append(_scn(
        "preflash",
        "enlarger.preflash_exposure=0.05 with pre-flash filter shifts.",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({
            "enlarger.preflash_exposure": 0.05,
            "enlarger.preflash_y_filter_shift": 5.0,
            "enlarger.preflash_m_filter_shift": -5.0,
        }),
        ["log_e_print", "cmy_print"], "arithmetic", "expected_parity", None,
    ))
    sc.append(_scn(
        "exposure_compensation",
        "camera.exposure_compensation_ev=-1 with print exposure "
        "compensation on (factor_midgray_comp branch).",
        "kodak_portra_400", "kodak_portra_endura", "gradient4x4",
        _bare_with({
            "camera.exposure_compensation_ev": -1.0,
            "enlarger.print_exposure": 1.5,
        }),
        PRINT_TAPS, "arithmetic", "expected_parity", None,
    ))

    # ---- full stochastic chain ----------------------------------------------
    sc.append(_scn(
        "defaults_full_chain_32x32",
        "Every default effect active (grain layers + halation + DIR + "
        "print glare + unsharp) — the shipped consumer default.",
        "kodak_portra_400", "kodak_portra_endura", "gradient32x32",
        {
            "io.input_color_space": "sRGB",
            "io.input_cctf_decoding": False,
            "io.output_color_space": "sRGB",
            "io.output_cctf_encoding": False,
        },
        ["cmy_film", "cmy_print", "rgb_out"], "statistical_texture",
        "known_gap", 6,
        "Layered grain (default sublayers_active=True) is unported, so "
        "per-pixel parity is not expected; moments are recorded for the "
        "statistical budget. Glare RNG streams also differ (Rust fixed "
        "seed 42 vs Python's continued np.random stream).",
    ))

    # ---- 28-profile sweep ----------------------------------------------------
    sc.append(_scn(
        "profiles_sweep",
        "All 28 upstream film profiles through their print topology "
        "(paper = target_print when present, else the film itself in scan "
        "mode), bare chain. Expands to one row per profile.",
        None, None, "gradient4x4", dict(BARE), PRINT_TAPS, "arithmetic",
        "expected_parity", None,
        "Catches profile-driven divergences (B&W vs colour n_channels "
        "semantics, positive vs negative presets, fitted print-curve "
        "models). Expansion uses UPSTREAM_PROFILES + each profile's "
        "info.target_print.",
        expand="profiles",
    ))

    for name, controls, budget in [
        ("hanatos_surface", {"settings.apply_hanatos2025_adaptation_surface": True}, "arithmetic"),
        ("hanatos_spectral_blur", {"settings.spectral_gaussian_blur": 2.0}, "arithmetic"),
        ("debug_spatial", {"debug.deactivate_spatial_effects": True}, "arithmetic"),
        ("debug_stochastic", {"debug.deactivate_stochastic_effects": True}, "arithmetic"),
        ("debug_lut_mode", {"debug.lut_mode": True}, "arithmetic"),
        ("lut_resolution_9", {"settings.use_enlarger_lut": True, "settings.use_scanner_lut": True, "settings.lut_resolution": 9}, "lut_reduction"),
        ("layered_fast_stats", {"film_render.grain.active": True, "film_render.grain.sublayers_active": True, "settings.use_fast_stats": True}, "statistical_texture"),
    ]:
        sc.append(_scn(name, name, "kodak_portra_400", "kodak_portra_endura",
                       "gradient32x32", _bare_with(controls), PRINT_TAPS,
                       budget, "expected_parity"))
    # Paper-support stocks are print targets only upstream: constructing
    # SimulationPipeline with one as the film raises ValueError in
    # eval_erf4_spectral_bandpass (empty window params) — verified against
    # the pinned venv. kodak_2383/2393 are film-support stocks whose
    # profiles ship empty window params; the upstream-supported
    # configuration for them disables the window adaptation (GUI toggle).
    sc.append(_scn(
        "paper_as_film_rejected",
        "Each paper-support stock used as --film must be rejected with an "
        "actionable error, matching the pinned Python ValueError.",
        None, None, "gradient4x4", dict(BARE), [], "rejection",
        "expected_parity", 3,
        expand="papers", expect_reject=True,
    ))

    return sc


def expand_scenarios(py_repo):
    """Expand sweep scenarios into concrete rows.

    ``py_repo`` is the pinned Python checkout; each profile's
    ``info.target_print`` selects the paper (print topology) or forces
    ``io.scan_film`` (film as its own scan target). Row names use
    ``"<sweep>::<film>"`` and fixture directories use ``<sweep>/<film>``
    — both drivers agree on this mapping via :func:`scenario_dir_name`.
    """
    import json
    from pathlib import Path

    rows = []
    profiles_dir = Path(py_repo) / "src" / "spektrafilm" / "data" / "profiles"
    for scn in SCENARIOS:
        if scn.get("expand") == "profiles":
            for film in UPSTREAM_PROFILES:
                profile = json.loads((profiles_dir / f"{film}.json").read_text())
                info = profile.get("info", {})
                data = profile.get("data", {})
                if info.get("support") == "paper":
                    continue  # papers are print targets, never films
                params = dict(scn["params"])
                target = info.get("target_print")
                if target:
                    paper = target
                else:
                    paper = film
                    params["io.scan_film"] = True
                    if not data.get("hanatos2025_adaptation_window_params"):
                        # kodak_2383/2393: default window=True raises
                        # upstream; window-off is the supported config.
                        params["settings.apply_hanatos2025_adaptation_window"] = False
                rows.append({
                    **scn,
                    "name": f"{scn['name']}::{film}",
                    "film": film,
                    "paper": paper,
                    "params": params,
                    "taps": SCAN_TAPS if not target else scn["taps"],
                    "expand": None,
                })
        elif scn.get("expand") == "papers":
            for film in UPSTREAM_PROFILES:
                profile = json.loads((profiles_dir / f"{film}.json").read_text())
                if profile.get("info", {}).get("support") != "paper":
                    continue
                rows.append({
                    **scn,
                    "name": f"{scn['name']}::{film}",
                    "film": film,
                    "paper": film,
                    "params": dict(scn["params"]),
                    "expand": None,
                })
        else:
            rows.append(scn)
    return rows


def scenario_dir_name(name):
    """Fixture sub-directory for a scenario row name (sweep-safe)."""
    return name.replace("::", "/")


SCENARIOS = build_scenarios()


def scenario_by_name(name):
    for s in SCENARIOS:
        if s["name"] == name:
            return s
    raise KeyError(f"unknown scenario {name!r}")


if __name__ == "__main__":
    for s in SCENARIOS:
        print(f"{s['name']:32s} {s['status']:16s} budget={s['budget']:20s} "
              f"owner={s['owner_issue']}")
