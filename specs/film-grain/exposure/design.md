# Exposure grain design

## Boundary

Exposure owns a reconstructed post-scan appearance model. Core owns parameter resolution, the encoded-RGB stage boundary, dispatch, and observation. The model MUST NOT consume film or print density or alter a material-profile decision.

The frozen reference is the reconstructed Exposure X7 luma path. Its gradient-noise generator, automatic-size model, smoothstep zone partition, roughness S curve, and additive composition are the model authority. The report and executable reconstruction identify the source; no Exposure binary, preset asset, or table is redistributed.

The reconstruction does not claim pixel parity with Exposure X7 or its presets. It is an authored appearance model with the controls in this contract. The decoded noise law (gradient dot plus a value term) is the reconstruction authority; the Color Variation channel-to-format mapping remains a reconstruction inference.

## Parameter resolution

The selected engine resolves its own nested controls before rendering. The default engine remains Dye. An omitted engine MUST NOT be rewritten to Exposure.

The source-preset values used for defaults are the reconstructed Regular Grain 100 percent values. The fixed zone thresholds are those default thresholds; vendor presets that varied the threshold keys are outside this contract.

The format constants are engine values; they do not read `camera.film_format_mm`. The product control ranges include every observed factory value; they are authoring bounds, not vendor calibration.

Automatic size MUST be resolved from the actual output dimensions. A full-frame and a reduced preview therefore have different sampling geometry; the preview result MUST NOT be described as a pixel-identical full render.

The effective size and amplitude are image-dimension dependent. Prepared tables depend on the seed and the effective frequency size, not on image pixels.

A change to seed, automatic-size mode, size controls, format, render scale, or push MUST invalidate prepared tables. A pure strength change MUST NOT require table regeneration.

## Noise construction

The scalar field uses the frozen 516-entry spectral tables and two 8192-entry permutation tables.

The minstd state advances with Schrage's recurrence. The shared 64-bit seed is reduced to the reference domain by taking its low 31 bits and clamping to `0x7ffffffe`; the constructor adds one and advances once.

Table phase draws use the canonical unit value; shuffle indices use the integer remainder. The spectral shuffle uses bound `515` (`N - 1`), and the permutation shuffle uses bound `8192`; this off-by-one is part of the frozen reference.

The spectral tables share one 258-phase draw per index: T1 and T2 are identical at `size_low` amplitude, and the third table uses half amplitude with swapped cosine/sine order. One Fisher-Yates index per entry shuffles all three tables together, then a second seed-derived stream shuffles the two permutation tables.

The five octave weights are `[1, 0.5, 0.25, 0.375, 0.1875]`, normalized. Weights below `0.03` are omitted.

The octave offsets are a single array `off = [23.1, 8.15, 57.0, 69.2, 117.4]` added to both the x and y coordinates. The spectral table entries use `size_low` amplitude; the third table uses half amplitude with swapped components.

The frequencies are `fx = 1 / size_low` and `fy = 1 / size_high`. The x coordinate scales by `2^octave` per octave; the y coordinate is not scaled per octave. `size_low` also sets the spectral amplitude and the output clamp `±|size_low|`.

The frozen scalar evaluator initializes `perm[i] = i & 511` and `perm2[i] = i`. It hashes each integer lattice corner as `h = perm[perm2[ix & 8191] ^ (iy & 8191)]`.

A corner value is a gradient dot plus a value term. It is `(px - cx) * t1[h] + (py - cy) * t2[h] + t3[h]`. Here `(cx, cy)` is the corner's integer lattice position and `(px, py)` is the sample point.

The four corners use offsets `(fx, fy)`, `(fx, fy - 1)`, `(fx - 1, fy)`, and `(fx - 1, fy - 1)`. Here `fx = px - ix` and `fy = py - iy`.

Bilinear smoothstep interpolation evaluates `(ix, iy)`, `(ix, iy + 1)`, `(ix + 1, iy)`, and `(ix + 1, iy + 1)` in that order. The result is clamped to `±|size_low|`.

The color evaluator runs a second, independent noise generator. Its seed is offset by `0x1f` from the luma generator. It adds three raw chromatic fields on top of the achromatic base.

Each channel receives its own field from consecutive gradient-table slots (`g+0`, `g+1`, `g+2`). The per-zone amplitude uses `0.4 * colorvar` for shadow and midtone and `0.8 * colorvar` for highlight. It is scaled by the zone amplitudes and frequency gain.

The raw noise is not S-curved. `colorvar = 0` reproduces the frozen monochrome scalar field exactly. The pixel-format flags rotate only the byte offset of each channel (RGBA vs BGRA); the gain weights are fixed per zone.

## Value shaping

The zone weights are adjacent differences of the smoothstep partition at `[-0.5, 0, midtone, highlight, 1.5]`. They MUST sum to one in the represented display range.

The roughness S curve uses the reconstructed convention:

```text
roughness_effective = clamp(0.01 * roughness + 0.3 * push / 3, 0, 1)
if abs(roughness_effective - 0.5) < 1e-5:
    identity
else:
    t = (1 - 0.5) * roughness_effective / (roughness_effective - 0.5)
    a = t * t - t
    b = t
    shaped = sign(g) * clamp(a / (1 - abs(g) - b) + b, 0, 1)
```

The division guard is part of the law. When `1 - abs(g) - b` is exactly zero, the shaped magnitude MUST be `1`.

Other nonzero denominators MUST use the reconstructed expression. The guard MUST NOT replace valid negative-denominator behavior with a different curve.

The displacement is `sum(zone_weight * zone_amplitude) * frequency_gain * shaped`. It is added to luma as the achromatic base, and that base is applied uniformly to the encoded RGB channels.

Color Variation is a separate pass over the luma result. A second noise generator (seed offset `0x1f`) supplies three raw chromatic fields from consecutive gradient-table slots. The raw noise is not S-curved.

With `colorvar = clamp(color_variation / 100, 0, 1)`, the per-zone amplitude is `k1 = 0.4 * colorvar` for the shadow and midtone zones and `k2 = 0.8 * colorvar` for the highlight zone. The color delta per channel is `field_c * color_gain * frequency_gain`. Here `color_gain = (w_sh*amp_sh + w_md*amp_md)*k1 + w_hi*amp_hi*k2`.

The delta is applied additively on top of the luma-modified pixels. Each channel is clamped to the encoded range. At `color_variation = 0` the color pass is skipped and the frozen scalar path is unchanged.

The Rec.709 appearance luma and the uniform base application are authored SpektraFilm choices. The frozen scalar reference does not establish vendor channel weights.

The Color Variation law is recovered from the Exposure X7 binary. It uses a separate generator, per-zone `0.4`/`0.8` amplitude, and three chromatic fields from consecutive gradient slots. The pixel-format flags rotate only the byte offset of each channel (RGBA vs BGRA), not the gain weights.

No channel-specific grain is part of this contract beyond this reconstructed Color Variation.

## Executor boundary

CPU and WGPU consume the same prepared table values and scalar control coefficients. The host MAY construct the bounded tables because they are image-independent. It MUST NOT construct per-pixel fields or per-image random arrays for device execution.

The device evaluates one output sample from tables and scalar coefficients. Absolute pixel coordinates MUST include the requested raster origin. Source/output pages, tiles, and dispatch windows affect resources only.

A missing device path, unsupported precision, or resource limit MUST fail explicitly. It MUST NOT demote an explicit GPU request to another engine or to CPU image arithmetic.

The recorded luma path is bounded and does not require a spatial blur. If a future change adds spatial filtering or a color path, it MUST define its own device residency, cancellation, and evidence.

## Pipeline integration

The engine is one post-scan appearance branch. Core MUST use one authoritative predicate for post-scan encoded engines so the transfer encode, output-transfer decode, CMYK ordering, resident tail, and telemetry cannot disagree.

The stage order is the existing scanner appearance order. Exposure MUST NOT move material grain, halation, DIR, print exposure, or the negative interpretation.

The source image is encoded before Exposure and decoded after it when the caller requests linear output. The encoded samples passed to Exposure MUST be the same samples for encoded and linear exports.

A supported GPU configuration MUST execute the complete Exposure path on the selected device. The shared grain parameter object and all five engine selectors retain independent control spaces.

## Evidence

Reference vectors MUST cover table construction, size and push scaling, zone partition boundaries, roughness identity and endpoints, magnitude clamping, and repeated execution.

CPU/GPU comparisons MUST use identical prepared tables, dimensions, seeds, and controls. Budgets MUST identify the metric, domain, sample population, and measured source. Generic statistical agreement MUST NOT replace table and coordinate checks.

Real CLI and native GUI runs MUST exercise the encoded route, a linear export, direct scan and negative interpretation, preview, inactive grain, and strength zero. Vendor output is not an oracle for this contract; any future comparison MUST remain separate from the frozen reconstruction evidence.
