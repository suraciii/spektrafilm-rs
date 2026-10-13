# Preset parameters

One parameter set serves every input surface. A leaf is settable through the GUI, the CLI, a preset document, and the API together, or it is settable nowhere: a leaf that no control edits must not keep a command-line path, and a leaf that has a control must not be reachable only from a file. The table is generated from the parameter metadata rather than maintained by hand, and a new leaf must arrive with a class, a control decision, and a capture attribute.

| Class | Leaves | Settable | Meaning |
|---|---|---|---|
| `parameter` | 198 | yes, by every surface | A user control edits it |
| `internal` | 20 | no | No control edits it, it is derived, or the pipeline never reads it |

`capture` is a second attribute of a `parameter` leaf: a whole-look capture writes the leaves with `capture=yes`, which are the values a user tunes while simulating. A `capture=no` leaf is a source, geometry, delivery, viewer, or environment value; a document may still carry it explicitly.

## Review items

### No control, and no settable path (must lose `--set`, preset, and API access)

| Path | Why |
|---|---|
| `debug.deactivate_spatial_effects` | diagnostics |
| `debug.deactivate_stochastic_effects` | diagnostics |
| `debug.lut_mode` | diagnostics |
| `debug.print_timings` | diagnostics |
| `enlarger.c_filter_neutral` | measured paper neutral; no control edits it |
| `enlarger.m_filter_neutral` | measured paper neutral; no control edits it |
| `enlarger.y_filter_neutral` | measured paper neutral; no control edits it |
| `film_render.glare.active` | parity-only: never read by the pipeline, no control |
| `film_render.glare.blur` | parity-only: never read by the pipeline, no control |
| `film_render.glare.percent` | parity-only: never read by the pipeline, no control |
| `film_render.glare.roughness` | parity-only: never read by the pipeline, no control |
| `settings.lut_resolution` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.neutral_print_filters_from_database` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.preview_mode` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.spectral_shape` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.use_enlarger_lut` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.use_fast_stats` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.use_scanner_lut` | grid, LUT wiring, preview, performance, calibration policy |
| `taps.collect` | spectral tap test hook |
| `taps.inject` | spectral tap test hook |

### No control, but a taking-side style value (needs a control, or removal)

| Path | Why |
|---|---|
| `camera.filter_ir` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.filter_uv` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.lens_blur_um` | taking side: exposure, metering, format, filters, optics, diffusion |

## Settable parameters

| Path | Capture | GUI control |
|---|---|---|
| `camera.auto_exposure` | yes | controls.rs:157 |
| `camera.auto_exposure_method` | yes | controls.rs:159 |
| `camera.color_filter` | yes | controls.rs:160 |
| `camera.diffusion_filter.active` | yes | not audited |
| `camera.diffusion_filter.bloom_intensity` | yes | not audited |
| `camera.diffusion_filter.bloom_size` | yes | not audited |
| `camera.diffusion_filter.core_intensity` | yes | not audited |
| `camera.diffusion_filter.core_size` | yes | not audited |
| `camera.diffusion_filter.filter_family` | yes | not audited |
| `camera.diffusion_filter.halo_intensity` | yes | not audited |
| `camera.diffusion_filter.halo_size` | yes | not audited |
| `camera.diffusion_filter.halo_warmth` | yes | not audited |
| `camera.diffusion_filter.spatial_scale` | yes | not audited |
| `camera.diffusion_filter.strength` | yes | not audited |
| `camera.exposure_compensation_ev` | yes | not audited |
| `camera.film_format_mm` | yes | controls.rs:158 |
| `camera.filter_ir` | yes | none found |
| `camera.filter_uv` | yes | none found |
| `camera.lens_blur_um` | yes | none found |
| `enlarger.diffusion_filter.active` | yes | not audited |
| `enlarger.diffusion_filter.bloom_intensity` | yes | not audited |
| `enlarger.diffusion_filter.bloom_size` | yes | not audited |
| `enlarger.diffusion_filter.core_intensity` | yes | not audited |
| `enlarger.diffusion_filter.core_size` | yes | not audited |
| `enlarger.diffusion_filter.filter_family` | yes | not audited |
| `enlarger.diffusion_filter.halo_intensity` | yes | not audited |
| `enlarger.diffusion_filter.halo_size` | yes | not audited |
| `enlarger.diffusion_filter.halo_warmth` | yes | not audited |
| `enlarger.diffusion_filter.spatial_scale` | yes | not audited |
| `enlarger.diffusion_filter.strength` | yes | not audited |
| `enlarger.illuminant` | yes | not audited |
| `enlarger.lens_blur` | yes | not audited |
| `enlarger.m_filter_shift` | yes | not audited |
| `enlarger.normalize_print_exposure` | yes | not audited |
| `enlarger.preflash_exposure` | yes | not audited |
| `enlarger.preflash_m_filter_shift` | yes | not audited |
| `enlarger.preflash_y_filter_shift` | yes | not audited |
| `enlarger.print_exposure` | yes | not audited |
| `enlarger.print_exposure_compensation` | yes | not audited |
| `enlarger.y_filter_shift` | yes | not audited |
| `film.stock` | yes | not audited |
| `film_render.base.active` | yes | not audited |
| `film_render.base.cyan` | yes | not audited |
| `film_render.base.magenta` | yes | not audited |
| `film_render.base.scale` | yes | not audited |
| `film_render.base.tilt` | yes | not audited |
| `film_render.base.yellow` | yes | not audited |
| `film_render.chemistry.active` | yes | not audited |
| `film_render.chemistry.developer_exhaustion` | yes | not audited |
| `film_render.chemistry.gamma_factor` | yes | not audited |
| `film_render.chemistry.gamma_factor_blue` | yes | not audited |
| `film_render.chemistry.gamma_factor_fast` | yes | not audited |
| `film_render.chemistry.gamma_factor_green` | yes | not audited |
| `film_render.chemistry.gamma_factor_red` | yes | not audited |
| `film_render.chemistry.gamma_factor_slow` | yes | not audited |
| `film_render.convert.base_percentile` | yes | not audited |
| `film_render.convert.calibration` | yes | not audited |
| `film_render.convert.exposure_compensation_ev` | yes | not audited |
| `film_render.convert.scan_illuminant` | yes | not audited |
| `film_render.density_curve_gamma` | yes | not audited |
| `film_render.development_time` | yes | not audited |
| `film_render.dir_couplers.active` | yes | not audited |
| `film_render.dir_couplers.amount` | yes | not audited |
| `film_render.dir_couplers.diffusion_size_um` | yes | not audited |
| `film_render.dir_couplers.diffusion_tail_um` | yes | not audited |
| `film_render.dir_couplers.diffusion_tail_weight` | yes | not audited |
| `film_render.dir_couplers.gamma_interlayer_b_to_rg` | yes | not audited |
| `film_render.dir_couplers.gamma_interlayer_g_to_rb` | yes | not audited |
| `film_render.dir_couplers.gamma_interlayer_r_to_gb` | yes | not audited |
| `film_render.dir_couplers.gamma_samelayer_rgb` | yes | not audited |
| `film_render.dir_couplers.inhibition_interlayer` | yes | not audited |
| `film_render.dir_couplers.inhibition_samelayer` | yes | not audited |
| `film_render.dir_couplers.langmuir_donor_k_rgb` | yes | not audited |
| `film_render.dir_couplers.langmuir_receiver_k_rgb` | yes | not audited |
| `film_render.grain.active` | yes | not audited |
| `film_render.grain.blur` | yes | not audited |
| `film_render.grain.blur_dye_clouds_um` | yes | not audited |
| `film_render.grain.density_min` | yes | not audited |
| `film_render.grain.engine` | yes | not audited |
| `film_render.grain.fotufilm.amount` | yes | not audited |
| `film_render.grain.fotufilm.clump_radius_um` | yes | not audited |
| `film_render.grain.fotufilm.coating_density_scale` | yes | not audited |
| `film_render.grain.fotufilm.color` | yes | not audited |
| `film_render.grain.fotufilm.fog_density` | yes | not audited |
| `film_render.grain.fotufilm.layer_size_ratio` | yes | not audited |
| `film_render.grain.fotufilm.layer_weights` | yes | not audited |
| `film_render.grain.fotufilm.layers` | yes | not audited |
| `film_render.grain.fotufilm.material` | yes | not audited |
| `film_render.grain.fotufilm.radius_span` | yes | not audited |
| `film_render.grain.fotufilm.rms_density` | yes | not audited |
| `film_render.grain.fotufilm.size` | yes | not audited |
| `film_render.grain.fotufilm.softness` | yes | not audited |
| `film_render.grain.fotufilm.sublayer_shares` | yes | not audited |
| `film_render.grain.micro_structure` | yes | not audited |
| `film_render.grain.micro_sublayers` | yes | not audited |
| `film_render.grain.mult_usm_amount` | yes | not audited |
| `film_render.grain.mult_usm_sigma` | yes | not audited |
| `film_render.grain.n_sub_layers` | yes | not audited |
| `film_render.grain.particle_area_um2` | yes | not audited |
| `film_render.grain.particle_scale` | yes | not audited |
| `film_render.grain.particle_scale_sublayers` | yes | not audited |
| `film_render.grain.rms_granularity` | yes | not audited |
| `film_render.grain.sublayers_active` | yes | not audited |
| `film_render.grain.uniformity` | yes | not audited |
| `film_render.grain.v2_amount` | yes | not audited |
| `film_render.grain.v2_chroma` | yes | not audited |
| `film_render.grain.v2_film_type` | yes | not audited |
| `film_render.grain.v2_highlights` | yes | not audited |
| `film_render.grain.v2_midtones` | yes | not audited |
| `film_render.grain.v2_mode` | yes | not audited |
| `film_render.grain.v2_profile` | yes | not audited |
| `film_render.grain.v2_resolution_factor` | yes | not audited |
| `film_render.grain.v2_resolution_type` | yes | not audited |
| `film_render.grain.v2_shadows` | yes | not audited |
| `film_render.grain.v2_size` | yes | not audited |
| `film_render.grain.v2_timer` | yes | not audited |
| `film_render.grain.v3_dye_support_um` | yes | not audited |
| `film_render.grain.v3_event_area_um2` | yes | not audited |
| `film_render.halation.active` | yes | not audited |
| `film_render.halation.boost_ev` | yes | not audited |
| `film_render.halation.boost_range` | yes | not audited |
| `film_render.halation.halation_amount` | yes | not audited |
| `film_render.halation.halation_bounce_decay` | yes | not audited |
| `film_render.halation.halation_first_sigma_um` | yes | not audited |
| `film_render.halation.halation_n_bounces` | yes | not audited |
| `film_render.halation.halation_renormalize` | yes | not audited |
| `film_render.halation.halation_spatial_scale` | yes | not audited |
| `film_render.halation.halation_strength` | yes | not audited |
| `film_render.halation.protect_ev` | yes | not audited |
| `film_render.halation.scatter_amount` | yes | not audited |
| `film_render.halation.scatter_core_um` | yes | not audited |
| `film_render.halation.scatter_spatial_scale` | yes | not audited |
| `film_render.halation.scatter_tail_um` | yes | not audited |
| `film_render.halation.scatter_tail_weight` | yes | not audited |
| `io.crop` | no | controls.rs:337 |
| `io.crop_center` | no | controls.rs:338 |
| `io.crop_size` | no | controls.rs:339 |
| `io.input_cctf_decoding` | no | not audited |
| `io.input_color_space` | no | not audited |
| `io.input_gamut_compress.active` | no | not audited |
| `io.input_gamut_compress.algorithm` | no | not audited |
| `io.input_gamut_compress.boundary` | no | not audited |
| `io.input_gamut_compress.hull_detail` | no | not audited |
| `io.input_gamut_compress.knee` | no | not audited |
| `io.output_cctf_encoding` | no | not audited |
| `io.output_color_space` | no | not audited |
| `io.output_gamut_compress.algorithm` | no | not audited |
| `io.output_gamut_compress.knee` | no | not audited |
| `io.upscale_factor` | no | controls.rs:336 |
| `magazine_print_color.active` | yes | not audited |
| `magazine_print_color.strength` | yes | not audited |
| `output_mix.amount` | yes | not audited |
| `print.stock` | yes | not audited |
| `print_render.base.active` | yes | not audited |
| `print_render.base.cyan` | yes | not audited |
| `print_render.base.magenta` | yes | not audited |
| `print_render.base.scale` | yes | not audited |
| `print_render.base.yellow` | yes | not audited |
| `print_render.density_curve_gamma` | yes | not audited |
| `print_render.density_curves_morph.active` | yes | not audited |
| `print_render.density_curves_morph.developer_exhaustion` | yes | not audited |
| `print_render.density_curves_morph.gamma_factor` | yes | not audited |
| `print_render.density_curves_morph.gamma_factor_blue` | yes | not audited |
| `print_render.density_curves_morph.gamma_factor_fast` | yes | not audited |
| `print_render.density_curves_morph.gamma_factor_green` | yes | not audited |
| `print_render.density_curves_morph.gamma_factor_red` | yes | not audited |
| `print_render.density_curves_morph.gamma_factor_slow` | yes | not audited |
| `print_render.development_time` | yes | not audited |
| `print_render.glare.active` | yes | controls.rs:298 |
| `print_render.glare.blur` | yes | controls.rs:301 |
| `print_render.glare.percent` | yes | controls.rs:299 |
| `print_render.glare.roughness` | yes | controls.rs:300 |
| `scanner.auto_setup` | yes | not audited |
| `scanner.black_correction` | yes | not audited |
| `scanner.black_level` | yes | not audited |
| `scanner.black_point` | yes | not audited |
| `scanner.finish` | yes | not audited |
| `scanner.finish_strength` | yes | not audited |
| `scanner.lens_blur` | yes | not audited |
| `scanner.positive_balance_ev` | yes | not audited |
| `scanner.positive_contrast` | yes | not audited |
| `scanner.positive_density_ev` | yes | not audited |
| `scanner.positive_dmax` | yes | not audited |
| `scanner.positive_interpretation` | yes | not audited |
| `scanner.receiver_profile` | yes | not audited |
| `scanner.reference_balance_ev` | yes | not audited |
| `scanner.reference_exposure_ev` | yes | not audited |
| `scanner.scan_output` | yes | not audited |
| `scanner.unsharp_mask` | yes | not audited |
| `scanner.white_correction` | yes | not audited |
| `scanner.white_level` | yes | not audited |
| `scanner.white_point` | yes | not audited |
| `settings.apply_hanatos2025_adaptation_surface` | yes | controls.rs:387 |
| `settings.apply_hanatos2025_adaptation_window` | yes | controls.rs:386 |
| `settings.preview_max_size` | no | controls.rs:538 |
| `settings.rgb_to_raw_method` | yes | controls.rs:385 |
| `settings.spectral_gaussian_blur` | yes | controls.rs:388 |
| `settings.use_cat16` | yes | not audited |

## Internal leaves

| Path | Why it is not a parameter |
|---|---|
| `debug.deactivate_spatial_effects` | diagnostics |
| `debug.deactivate_stochastic_effects` | diagnostics |
| `debug.lut_mode` | diagnostics |
| `debug.print_timings` | diagnostics |
| `enlarger.c_filter_neutral` | measured paper neutral; no control edits it |
| `enlarger.m_filter_neutral` | measured paper neutral; no control edits it |
| `enlarger.y_filter_neutral` | measured paper neutral; no control edits it |
| `film_render.glare.active` | parity-only: never read by the pipeline, no control |
| `film_render.glare.blur` | parity-only: never read by the pipeline, no control |
| `film_render.glare.percent` | parity-only: never read by the pipeline, no control |
| `film_render.glare.roughness` | parity-only: never read by the pipeline, no control |
| `settings.lut_resolution` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.neutral_print_filters_from_database` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.preview_mode` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.spectral_shape` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.use_enlarger_lut` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.use_fast_stats` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.use_scanner_lut` | grid, LUT wiring, preview, performance, calibration policy |
| `taps.collect` | spectral tap test hook |
| `taps.inject` | spectral tap test hook |

## Source

Generated from `spektrafilm describe` metadata, the GUI control surface, and the shipped capability contracts. Totals: 198 parameters, 20 internal leaves, of which 20 lose an existing settable path and 3 need a control decision.
