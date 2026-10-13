# Preset parameters

Every runtime parameter leaf carries exactly one class. The class decides whether a preset document may carry the leaf, and it is a property of the leaf in the parameter metadata, next to its type, bounds, default, unit, and description, so this table is generated rather than maintained by hand.

| Class | Leaves | A preset may carry it | Meaning |
|---|---|---|---|
| `look` | 182 | yes | A user control edits it and it changes the rendered look |
| `context` | 23 | no | A user control edits it, but it describes the source, the geometry, the delivery, the viewer, or the runtime environment |
| `internal` | 13 | no | No control edits it, or the pipeline never reads it |

The three `enlarger.*_filter_neutral` axes keep their command-line pin as the single documented exception: they are `internal`, a preset must reject them with the path named, and a sparse parameter source may still set them explicitly to pin a measured neutral.

## Look class

| Path | Class reason |
|---|---|
| `camera.auto_exposure` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.auto_exposure_method` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.color_filter` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.active` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.bloom_intensity` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.bloom_size` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.core_intensity` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.core_size` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.filter_family` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.halo_intensity` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.halo_size` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.halo_warmth` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.spatial_scale` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.diffusion_filter.strength` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.exposure_compensation_ev` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.film_format_mm` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.filter_ir` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.filter_uv` | taking side: exposure, metering, format, filters, optics, diffusion |
| `camera.lens_blur_um` | taking side: exposure, metering, format, filters, optics, diffusion |
| `enlarger.diffusion_filter.active` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.bloom_intensity` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.bloom_size` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.core_intensity` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.core_size` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.filter_family` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.halo_intensity` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.halo_size` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.halo_warmth` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.spatial_scale` | print exposure, M/Y shifts, diffusion |
| `enlarger.diffusion_filter.strength` | print exposure, M/Y shifts, diffusion |
| `enlarger.illuminant` | print exposure, M/Y shifts, diffusion |
| `enlarger.lens_blur` | print exposure, M/Y shifts, diffusion |
| `enlarger.m_filter_shift` | print exposure, M/Y shifts, diffusion |
| `enlarger.normalize_print_exposure` | print exposure, M/Y shifts, diffusion |
| `enlarger.preflash_exposure` | print exposure, M/Y shifts, diffusion |
| `enlarger.preflash_m_filter_shift` | print exposure, M/Y shifts, diffusion |
| `enlarger.preflash_y_filter_shift` | print exposure, M/Y shifts, diffusion |
| `enlarger.print_exposure` | print exposure, M/Y shifts, diffusion |
| `enlarger.print_exposure_compensation` | print exposure, M/Y shifts, diffusion |
| `enlarger.y_filter_shift` | print exposure, M/Y shifts, diffusion |
| `film.stock` | film profile selection by stock name |
| `film_render.base.active` | film grade, development, coupler, grain |
| `film_render.base.cyan` | film grade, development, coupler, grain |
| `film_render.base.magenta` | film grade, development, coupler, grain |
| `film_render.base.scale` | film grade, development, coupler, grain |
| `film_render.base.tilt` | film grade, development, coupler, grain |
| `film_render.base.yellow` | film grade, development, coupler, grain |
| `film_render.chemistry.active` | film grade, development, coupler, grain |
| `film_render.chemistry.developer_exhaustion` | film grade, development, coupler, grain |
| `film_render.chemistry.gamma_factor` | film grade, development, coupler, grain |
| `film_render.chemistry.gamma_factor_blue` | film grade, development, coupler, grain |
| `film_render.chemistry.gamma_factor_fast` | film grade, development, coupler, grain |
| `film_render.chemistry.gamma_factor_green` | film grade, development, coupler, grain |
| `film_render.chemistry.gamma_factor_red` | film grade, development, coupler, grain |
| `film_render.chemistry.gamma_factor_slow` | film grade, development, coupler, grain |
| `film_render.convert.base_percentile` | film grade, development, coupler, grain |
| `film_render.convert.calibration` | film grade, development, coupler, grain |
| `film_render.convert.exposure_compensation_ev` | film grade, development, coupler, grain |
| `film_render.convert.scan_illuminant` | film grade, development, coupler, grain |
| `film_render.density_curve_gamma` | film grade, development, coupler, grain |
| `film_render.development_time` | film grade, development, coupler, grain |
| `film_render.dir_couplers.active` | film grade, development, coupler, grain |
| `film_render.dir_couplers.amount` | film grade, development, coupler, grain |
| `film_render.dir_couplers.diffusion_size_um` | film grade, development, coupler, grain |
| `film_render.dir_couplers.diffusion_tail_um` | film grade, development, coupler, grain |
| `film_render.dir_couplers.diffusion_tail_weight` | film grade, development, coupler, grain |
| `film_render.dir_couplers.gamma_interlayer_b_to_rg` | film grade, development, coupler, grain |
| `film_render.dir_couplers.gamma_interlayer_g_to_rb` | film grade, development, coupler, grain |
| `film_render.dir_couplers.gamma_interlayer_r_to_gb` | film grade, development, coupler, grain |
| `film_render.dir_couplers.gamma_samelayer_rgb` | film grade, development, coupler, grain |
| `film_render.dir_couplers.inhibition_interlayer` | film grade, development, coupler, grain |
| `film_render.dir_couplers.inhibition_samelayer` | film grade, development, coupler, grain |
| `film_render.dir_couplers.langmuir_donor_k_rgb` | film grade, development, coupler, grain |
| `film_render.dir_couplers.langmuir_receiver_k_rgb` | film grade, development, coupler, grain |
| `film_render.grain.active` | film grade, development, coupler, grain |
| `film_render.grain.blur` | film grade, development, coupler, grain |
| `film_render.grain.blur_dye_clouds_um` | film grade, development, coupler, grain |
| `film_render.grain.density_min` | film grade, development, coupler, grain |
| `film_render.grain.engine` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.amount` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.clump_radius_um` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.coating_density_scale` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.color` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.fog_density` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.layer_size_ratio` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.layer_weights` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.layers` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.material` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.radius_span` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.rms_density` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.size` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.softness` | film grade, development, coupler, grain |
| `film_render.grain.fotufilm.sublayer_shares` | film grade, development, coupler, grain |
| `film_render.grain.micro_structure` | film grade, development, coupler, grain |
| `film_render.grain.micro_sublayers` | film grade, development, coupler, grain |
| `film_render.grain.mult_usm_amount` | film grade, development, coupler, grain |
| `film_render.grain.mult_usm_sigma` | film grade, development, coupler, grain |
| `film_render.grain.n_sub_layers` | film grade, development, coupler, grain |
| `film_render.grain.particle_area_um2` | film grade, development, coupler, grain |
| `film_render.grain.particle_scale` | film grade, development, coupler, grain |
| `film_render.grain.particle_scale_sublayers` | film grade, development, coupler, grain |
| `film_render.grain.rms_granularity` | film grade, development, coupler, grain |
| `film_render.grain.sublayers_active` | film grade, development, coupler, grain |
| `film_render.grain.uniformity` | film grade, development, coupler, grain |
| `film_render.grain.v2_amount` | film grade, development, coupler, grain |
| `film_render.grain.v2_chroma` | film grade, development, coupler, grain |
| `film_render.grain.v2_film_type` | film grade, development, coupler, grain |
| `film_render.grain.v2_highlights` | film grade, development, coupler, grain |
| `film_render.grain.v2_midtones` | film grade, development, coupler, grain |
| `film_render.grain.v2_mode` | film grade, development, coupler, grain |
| `film_render.grain.v2_profile` | film grade, development, coupler, grain |
| `film_render.grain.v2_resolution_factor` | film grade, development, coupler, grain |
| `film_render.grain.v2_resolution_type` | film grade, development, coupler, grain |
| `film_render.grain.v2_shadows` | film grade, development, coupler, grain |
| `film_render.grain.v2_size` | film grade, development, coupler, grain |
| `film_render.grain.v2_timer` | film grade, development, coupler, grain |
| `film_render.grain.v3_dye_support_um` | film grade, development, coupler, grain |
| `film_render.grain.v3_event_area_um2` | film grade, development, coupler, grain |
| `film_render.halation.active` | film grade, development, coupler, grain |
| `film_render.halation.boost_ev` | film grade, development, coupler, grain |
| `film_render.halation.boost_range` | film grade, development, coupler, grain |
| `film_render.halation.halation_amount` | film grade, development, coupler, grain |
| `film_render.halation.halation_bounce_decay` | film grade, development, coupler, grain |
| `film_render.halation.halation_first_sigma_um` | film grade, development, coupler, grain |
| `film_render.halation.halation_n_bounces` | film grade, development, coupler, grain |
| `film_render.halation.halation_renormalize` | film grade, development, coupler, grain |
| `film_render.halation.halation_spatial_scale` | film grade, development, coupler, grain |
| `film_render.halation.halation_strength` | film grade, development, coupler, grain |
| `film_render.halation.protect_ev` | film grade, development, coupler, grain |
| `film_render.halation.scatter_amount` | film grade, development, coupler, grain |
| `film_render.halation.scatter_core_um` | film grade, development, coupler, grain |
| `film_render.halation.scatter_spatial_scale` | film grade, development, coupler, grain |
| `film_render.halation.scatter_tail_um` | film grade, development, coupler, grain |
| `film_render.halation.scatter_tail_weight` | film grade, development, coupler, grain |
| `magazine_print_color.active` | finishing that changes the retained output |
| `magazine_print_color.strength` | finishing that changes the retained output |
| `output_mix.amount` | full-result blend amount |
| `print.stock` | print profile selection by stock name |
| `print_render.base.active` | print grade, chemistry, base, viewing glare |
| `print_render.base.cyan` | print grade, chemistry, base, viewing glare |
| `print_render.base.magenta` | print grade, chemistry, base, viewing glare |
| `print_render.base.scale` | print grade, chemistry, base, viewing glare |
| `print_render.base.yellow` | print grade, chemistry, base, viewing glare |
| `print_render.density_curve_gamma` | print grade, chemistry, base, viewing glare |
| `print_render.density_curves_morph.active` | print grade, chemistry, base, viewing glare |
| `print_render.density_curves_morph.developer_exhaustion` | print grade, chemistry, base, viewing glare |
| `print_render.density_curves_morph.gamma_factor` | print grade, chemistry, base, viewing glare |
| `print_render.density_curves_morph.gamma_factor_blue` | print grade, chemistry, base, viewing glare |
| `print_render.density_curves_morph.gamma_factor_fast` | print grade, chemistry, base, viewing glare |
| `print_render.density_curves_morph.gamma_factor_green` | print grade, chemistry, base, viewing glare |
| `print_render.density_curves_morph.gamma_factor_red` | print grade, chemistry, base, viewing glare |
| `print_render.density_curves_morph.gamma_factor_slow` | print grade, chemistry, base, viewing glare |
| `print_render.development_time` | print grade, chemistry, base, viewing glare |
| `print_render.glare.active` | print grade, chemistry, base, viewing glare |
| `print_render.glare.blur` | print grade, chemistry, base, viewing glare |
| `print_render.glare.percent` | print grade, chemistry, base, viewing glare |
| `print_render.glare.roughness` | print grade, chemistry, base, viewing glare |
| `scanner.auto_setup` | receiver, conversion, finish, acquisition |
| `scanner.black_correction` | receiver, conversion, finish, acquisition |
| `scanner.black_level` | receiver, conversion, finish, acquisition |
| `scanner.black_point` | receiver, conversion, finish, acquisition |
| `scanner.finish` | receiver, conversion, finish, acquisition |
| `scanner.finish_strength` | receiver, conversion, finish, acquisition |
| `scanner.lens_blur` | receiver, conversion, finish, acquisition |
| `scanner.positive_balance_ev` | receiver, conversion, finish, acquisition |
| `scanner.positive_contrast` | receiver, conversion, finish, acquisition |
| `scanner.positive_density_ev` | receiver, conversion, finish, acquisition |
| `scanner.positive_dmax` | receiver, conversion, finish, acquisition |
| `scanner.positive_interpretation` | receiver, conversion, finish, acquisition |
| `scanner.receiver_profile` | receiver, conversion, finish, acquisition |
| `scanner.reference_balance_ev` | receiver, conversion, finish, acquisition |
| `scanner.reference_exposure_ev` | receiver, conversion, finish, acquisition |
| `scanner.scan_output` | receiver, conversion, finish, acquisition |
| `scanner.unsharp_mask` | receiver, conversion, finish, acquisition |
| `scanner.white_correction` | receiver, conversion, finish, acquisition |
| `scanner.white_level` | receiver, conversion, finish, acquisition |
| `scanner.white_point` | receiver, conversion, finish, acquisition |
| `settings.apply_hanatos2025_adaptation_surface` | look-relevant processing switch |
| `settings.apply_hanatos2025_adaptation_window` | look-relevant processing switch |
| `settings.rgb_to_raw_method` | look-relevant processing switch |
| `settings.spectral_gaussian_blur` | look-relevant processing switch |
| `settings.use_cat16` | look-relevant processing switch |

## Context class

| Path | Class reason |
|---|---|
| `io.crop` | source declaration, geometry, gamut, output |
| `io.crop_center` | source declaration, geometry, gamut, output |
| `io.crop_size` | source declaration, geometry, gamut, output |
| `io.input_cctf_decoding` | source declaration, geometry, gamut, output |
| `io.input_color_space` | source declaration, geometry, gamut, output |
| `io.input_gamut_compress.active` | source declaration, geometry, gamut, output |
| `io.input_gamut_compress.algorithm` | source declaration, geometry, gamut, output |
| `io.input_gamut_compress.boundary` | source declaration, geometry, gamut, output |
| `io.input_gamut_compress.hull_detail` | source declaration, geometry, gamut, output |
| `io.input_gamut_compress.knee` | source declaration, geometry, gamut, output |
| `io.output_cctf_encoding` | source declaration, geometry, gamut, output |
| `io.output_color_space` | source declaration, geometry, gamut, output |
| `io.output_gamut_compress.algorithm` | source declaration, geometry, gamut, output |
| `io.output_gamut_compress.knee` | source declaration, geometry, gamut, output |
| `io.upscale_factor` | source declaration, geometry, gamut, output |
| `settings.lut_resolution` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.neutral_print_filters_from_database` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.preview_max_size` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.preview_mode` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.spectral_shape` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.use_enlarger_lut` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.use_fast_stats` | grid, LUT wiring, preview, performance, calibration policy |
| `settings.use_scanner_lut` | grid, LUT wiring, preview, performance, calibration policy |

## Internal class

| Path | Class reason |
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
| `taps.collect` | spectral tap test hook |
| `taps.inject` | spectral tap test hook |

## Review items

These leaves change the rendered result and the shipped contract excludes them, so each needs an owner decision:

- `camera.color_filter` — taking filter with measured transmission curves, shipped look match ignores it
- `camera.film_format_mm` — capture-rig value; decides pixel pitch and therefore grain scale, shipped look match ignores it
- `camera.filter_ir` — taking IR band pass, shipped look match ignores it
- `camera.filter_uv` — taking UV band pass, shipped look match ignores it
- `camera.lens_blur_um` — taking optics blur, shipped look match ignores it
- `film_render.glare.active` — parity-only, never read, no control; four --set paths would be removed
- `film_render.glare.blur` — parity-only, never read, no control
- `film_render.glare.percent` — parity-only, never read, no control
- `film_render.glare.roughness` — parity-only, never read, no control
- `magazine_print_color.active` — finishing control, shipped contract excludes the whole group from a preset
- `magazine_print_color.strength` — finishing control, shipped contract excludes the whole group from a preset
- `settings.neutral_print_filters_from_database` — database-calibration policy; changes the print neutral, shipped contract excludes it
- `settings.rgb_to_raw_method` — spectral reconstruction method; changes the rendered color, shipped contract excludes it

## Source

Generated from `spektrafilm describe` metadata plus the shipped capability contracts. A new leaf that reaches the runtime without a class must fail the build rather than default into one.
