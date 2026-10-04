# Parity matrix (summary)

Machine-readable source of truth: [`parity_matrix.json`](parity_matrix.json)
(regenerate with `python3 scripts/parity/gen_matrix.py --report
/tmp/spektrafilm-current-parity/parity_report.json`). This page separates the
original audited state from current integration contracts. Field entries
retain implementation provenance without claiming universal coverage;
`differential_evidence` records the exercised scenarios and tap metrics.

## Historical coverage at the audited baseline

| Issue | Scope | State at audited baseline |
|-------|-------|---------------------------|
| #3 [02] runtime parameter/preview/debug/tap contracts | `debug.*`, `taps.*` groups absent; `digest_params` step absent (preview_mode, lut_mode, deactivate_spatial/stochastic); stock-specific DIR presets (velvia, provia) and halation presets unported; `use_enlarger_lut` + `lut_resolution` unread | 12+ fields affected |
| #4 [03] color spaces & gamut | 4 input/output spaces supported; unknown names silently fall back; input CCTF decode ignored; jzazbz/oklrab + non-sRGB gamut fallback; output clipping semantics | 6 divergence clusters |
| #5 [04] UV/IR + Hanatos adaptation | `filter_uv`/`filter_ir` unread; `apply_hanatos2025_adaptation_surface` + `spectral_gaussian_blur` unread (window adaptation **is** ported) | 4 fields |
| #6 [05] layered grain | whole layers path (sublayers_active, particle_scale_layers, blur_dye_clouds_um, micro_structure, use_fast_stats, density_curves_layers data) unported | default-effect path |
| #7 [06] crop/resize | crop controls unread; AE/rescale order differs from upstream preprocess | 4 items |
| #8 [07] scan/glare semantics | direct-scan glare applied by Rust (upstream: none); glare RNG fixed seed 42 | 2 clusters |
| #9 [08] formats/ICC/metadata | 166 ICC + license not bundled; OIIO float-depth/metadata preservation absent | 167 files |
| #10 [09] RAW decode | white-balance modes + lensfun corrections absent; rawler as-shot only | 3 workflows |
| #11–#12 [10–11] GUI | state files, startup defaults, viewer/display/paper presentation unported | 21 actions |
| #13–#15 [12–14] LUT creator | whole product absent: 32-entry color-space registry, bundles, delivery targets, OCIO emission, QA; CLI exports via the wrong (simplified) constructor | 32 registry entries |
| #16 [15] CPU/GPU budgets | preview approximation budget provisional; GPU paths unverifiable until parity lands | budgets defined |

## Verified matches (hash evidence)

- 28/28 upstream film profiles byte-identical
- spectral upsampling LUT assets (2/2) byte-identical
- dichroic/heat-absorbing/lens-transmission filter CSVs (16/16) byte-identical
- `neutral_print_filters.json`: verified superset (upstream 160 combos unchanged)
- All 214 pinned Python asset paths are present: 212 byte-identical,
  neutral-filter database verified superset, ICC README adapted for Rust.
- Rust-only extras (out of 0.3.4 scope): `arctic2026alpha02` LUT dir,
  `kodak_2302` / `kodak_doublex` / `kodak_trix` profiles, B&W papers in the
  neutral-filter database, `settings.use_cat16`, film/paper
  `development_time`

## Upstream no-op fields (port omissions they are not)

- `enlarger.lens_blur` — zeroed by digest, applied by no upstream stage
- `film_render.glare.*` on the print path — upstream reads
  `print_render.glare` there; on `scan_film` it passes `glare=None`
  (the historical Rust scan-glare divergence is retained under audit_baseline)

## Scenario catalog

68 expanded rows in `scripts/parity/scenarios.py` cover the preserved bare
chain, print/direct-scan topology, both upsamplers, spectral LUT reduction,
color spaces/encoding/gamut, crop, UV/IR, layered grain, preview/digest,
stock presets, spatial controls, print morph, preflash and exposure. The
profile coverage consists of 22 film-support runs and six explicit
paper-as-film rejection checks. Preview evidence separates arithmetic
digestion with glare disabled from stochastic 512² glare moments under
the unchanged 1% mean / 5% standard-deviation budgets.

The current report `/tmp/spektrafilm-current-parity/parity_report.json` passes
all 68 exercised rows with no unsupported prerequisites. Full per-tap
metrics and reference environment are embedded under `differential_evidence`
in the machine-readable matrix. This verifies those scenarios; isolated
dark-grain regimes and other unexercised controls require their own evidence.
Historical baseline failures remain metadata and never excuse current drift.

## Native GUI evidence

The native driver exercises the five sidebar tabs, persistent Preview/Scan controls, quarter-turn pipeline input, exact pixel zoom, state/restart restoration, RAW loading and export cancellation. Viewer scenarios additionally record Paper back, interpolation, canvas/border changes, animation frames, float inspection, profile selection, non-sRGB state and decoded Save/Export isolation. Generate the matrix with `--gui-report` only after the complete native run succeeds. GUI action rows become `verified_exercised_path` only when their named assertions are present; all other rows retain their unverified status.

Current Linux evidence is `/tmp/spektrafilm-current-package-evidence-3/gui-acceptance/observations.json`.
Scan-for-print forced settings/restoration, Input interpolation isolation,
preview-size refresh/full-resolution Export and persistent RAW correction status
passed in the native run. Nonlinear toe fitting and interactive scientific plots
remain explicitly separate research API scope.

The evidence is platform-specific. Linux Xvfb does not verify a physical monitor ICC profile or Windows/macOS window interaction. A rotated export's native TIFF staging write cannot be interrupted; cancellation is checked around it and temporary files are removed before the worker finishes.
