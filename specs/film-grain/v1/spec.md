# Spektra grain
## Contract
- Spektra MUST sample developed film density and return film density for the remaining print or scan stages.
- The reference algorithm is `andreavolpato/spektrafilm` experimental commit `28bf883e1672e884307edc75852549376e13644e`.
- Spektra MUST use the profile's available density-curve layers.
- A profile without layer curves MUST use its total density curve as one active layer of the same sampler.
- `rms_granularity` MUST size the particle population from the maximum of the profile's summed layer-variance curves at the reference 48 µm diameter aperture. It MUST NOT be treated as a direct particle area or a measured final-scan RMS guarantee.
- Spektra MUST retain per-layer density floors, `particle_scale_sublayers`, uniformity, dye-cloud blur, micro-structure, and multiplicative unsharp masking.
- CLI source composition and complete looks MUST use `[6, 8, 10]` for omitted Spektra RMS before stock defaults apply.
- Authored RMS values MUST retain precedence; the shared Dye default MUST NOT become Spektra's omitted-strength fallback.
- Direct runtime callers that select Spektra MUST supply a positive RMS target; shared typed defaults remain Dye defaults.
- The configured seed MUST control repeatable sampling.
- The sequential sampler MUST draw the complete Poisson plane before the complete binomial plane.
- Its micro-structure field MUST continue the final active plane's NumPy random state.
- `settings.use_fast_stats` MUST select the reference distribution branches with deterministic per-element streams. It MUST NOT claim pixel identity with upstream's schedule-dependent numba streams.
- Active Spektra grain MUST run on the selected executor under [GPU algorithm alignment](../../gpu-execution/spec.md#algorithm-alignment). Missing device precision or resources MUST fail explicitly rather than substitute another engine or CPU image work.
- `particle_area_um2`, `particle_scale`, `particle_scale_layers`, `sublayers_active`, and `n_sub_layers` are obsolete controls. Parameter sources MUST reject them rather than map them to the RMS model.
