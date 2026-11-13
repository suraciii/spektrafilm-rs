# Exposure grain

## Identity and scope

`exposure` MUST select the independent Exposure grain engine. The native GUI MUST display `Exposure Grain`. Engine defaults follow the [shared grain contract](../spec.md#defaults-and-compatibility).

Exposure is a post-scan appearance model reconstructed from Exposure X7. It MUST consume and return display-encoded RGB. It MUST NOT read film density, film-profile chemistry, RMS granularity, or film coordinates.

Selecting Exposure MUST NOT change the selected film or paper profile. Changing a material control MUST affect the image only through the upstream film chain.

The reconstruction does not claim pixel parity with Exposure X7 or its presets. The engine is an authored appearance model with the controls in this contract.

## Parameters

`film_render.grain.exposure` MUST contain the independent engine's controls. Controls belonging to other engines MUST NOT change this model. All scalar values MUST be finite.

`strength` MUST range from 0 to 150 and default to 100. Values are percentages of the reconstructed grain amplitude.

`size` MUST range from 0.25 to 4.5 and default to 1. It is a relative multiplier for automatic size.

`auto_size` MUST be a boolean and default to `true`.

`format` MUST accept `35mm`, `medium_format`, or `large_format` and default to `35mm`. The automatic-size short-edge constants are 24.0, 56.0, and 101.6 millimeters. These are engine constants, not a declaration about the camera or film format.

`absolute_size_px` MUST range from 0.5 to 64 and default to 1.5. It applies only when `auto_size` is false; a stored value remains valid while automatic size is active.

`roughness` MUST range from 0 to 100 and default to 50.

`shadows`, `midtones`, and `highlights` MUST range from 0 to 100 and default to 50, 70, and 30.

`push` MUST range from 0 to 3 and default to 0. It is expressed in Exposure push units, not exposure stops.

`color_variation` MUST range from 0 to 100 and default to 20. That default is the reconstructed Regular Grain 100 percent value. It controls per-channel grain decorrelation.

Zero MUST reproduce the frozen monochrome scalar field exactly. Nonzero values MUST run a second noise generator (seed offset `0x1f`). It adds three raw chromatic fields, one per channel.

The per-zone amplitude is `0.4 * colorvar` for shadow and midtone and `0.8 * colorvar` for highlight. The law is recovered from the Exposure X7 binary.

The midtone and highlight thresholds are engine constants. The midtone threshold MUST be `0.424672424793243` and the highlight threshold MUST be `0.849344849586487`.

The Rec.709 appearance luma is an authored SpektraFilm choice. The frozen scalar reference does not establish vendor channel weights. The three channel fields MUST read consecutive gradient-table slots at the same lattice corner.

`color_variation = 0` MUST leave the frozen scalar displacement applied uniformly to RGB. Larger values MUST add per-channel chroma terms from the second generator (seed offset `0x1f`). The per-zone amplitude is `0.4 * colorvar` for shadow and midtone and `0.8 * colorvar` for highlight.

The pixel-format flags MUST rotate only the byte offset of each channel (RGBA vs BGRA), not the gain weights.

## Grain law

The engine MUST derive effective size from the output raster and the selected format when `auto_size` is true:

```text
effective = min(width, height) / format_mm / 125 * (size * 2.5)
effective = max(effective, 1.5)
```

When `auto_size` is false, `effective` MUST be `absolute_size_px`.

The push-adjusted size MUST be:

```text
x = (1 + 0.4 * push / 3) * effective / render_scale
size_low = min(1, x)
size_high = max(1, x)
```

`render_scale` MUST be 1 for the final delivered raster. A preview MAY use the full-to-preview ratio on a reduced raster; it MUST NOT claim full-render equivalence.

The scalar field MUST use `1 / size_low` as the x frequency and `1 / size_high` as the y frequency, and MUST clamp to `±|size_low|`. The x coordinate scales by `2^octave` per octave; the y coordinate is not scaled per octave.

The per-zone amplitude MUST use:

```text
zone_gain(value) = clamp(0.01 * value + 0.2 * push / 3, 0, 1)
amplitude[zone] = (strength / 100) * zone_gain(zone_value) * size_low
```

The roughness and frequency gain MUST use:

```text
roughness_effective = clamp(0.01 * roughness + 0.3 * push / 3, 0, 1)
frequency_gain = 0.3 / (2 * (0.3 * roughness_effective + 0.35))
```

Roughness MUST also shape the grain value with the reconstructed S curve. Roughness `50` at `push = 0` MUST be the identity case. The S-curve parameters, the exact zero-denominator continuation, and the clamp are defined in the [design](design.md#value-shaping).

The display luma MUST use the fixed appearance coefficients `[0.2126, 0.7152, 0.0722]` on encoded RGB. These coefficients are an authored SpektraFilm choice; the frozen reconstruction does not establish vendor channel weights.

The zone weights MUST be the adjacent smoothstep differences over the intervals `[-0.5, 0]`, `[0, midtone]`, `[midtone, highlight]`, and `[highlight, 1.5]`. The completed luminance displacement forms the achromatic base added uniformly to each RGB channel.

Each channel MAY add a Color Variation chroma term from the second generator (seed offset `0x1f`). The base field is frequency-gained and S-curved. The three chroma fields are raw, not S-curved.

The chroma fields carry per-zone amplitude `k1 = 0.4 * colorvar` for shadow and midtone and `k2 = 0.8 * colorvar` for highlight. The color delta per channel is `field_c * color_gain * frequency_gain`. Here `color_gain = (w_sh*amp_sh + w_md*amp_md)*k1 + w_hi*amp_hi*k2`.

Each channel MUST be clamped to the encoded range. The pixel-format flags rotate only the byte offset of each channel, not the gain weights.

The random field MUST use the reconstructed minstd permutation and gradient-noise tables. The three spectral tables MUST share one phase draw and one shuffle index per entry, with T1 and T2 identical and T3 carrying the same components swapped at half amplitude. The same input, seed, parameters, dimensions, and executor MUST produce the same realization.

The seed is the shared `random_seed`; Exposure's additional run counter MUST NOT introduce a second phase input. The shared 64-bit seed MUST be reduced to the reference's nonnegative 31-bit domain by taking its low 31 bits and clamping to `0x7ffffffe`; reference vectors MUST use a seed in that domain.

## Geometry and execution

Exposure MUST run after scan acquisition, output gamut processing, digital unsharp masking, and output transfer encoding. It MUST run before CMYK ink style and output mixing. A linear export MUST decode only the selected transfer after Exposure; it MUST NOT rerun the grain in linear RGB.

Exposure MUST execute only when the selected route executes a scan. Passthrough, material continuation, and convert-film routes MUST NOT synthesize new grain.

`active = false`, `strength = 0`, and runtime stochastic-effect deactivation MUST be exact bypasses.

The selected executor MUST implement the same model on CPU and WGPU. CPU and GPU MUST use the same prepared tables, octave weights, size, zone amplitudes, S curve, and coordinates. Floating outputs need not be bit-identical.

The prepared tables are bounded, image-independent coefficients. Host preparation MAY resolve them.

GPU execution MUST NOT use image-sized host noise arrays, CPU grain arithmetic, a generic stochastic substitute, or another grain engine. Resource limits and missing device support MUST fail explicitly under [GPU execution](../../gpu-execution/spec.md#backend-selection).

The grain field uses output-raster coordinates. A crop or resize selects the raster it receives; it MUST NOT restart the realization. Tiling and source-page boundaries MUST NOT change the coordinate or the table values.

## Acceptance

The frozen reconstruction reference MUST validate table generation, automatic size, manual size, push scaling, zone partitions, the S curve, and deterministic reproduction. Reference vectors MUST identify the seed and input dimensions.

Aligned executor evidence MUST compare identical tables, controls, geometry, and seed. Statistical agreement alone MUST NOT establish algorithm alignment. Follow [GPU execution acceptance](../../gpu-execution/spec.md#acceptance).

A real CLI run and a native GUI smoke run MUST exercise the encoded-RGB placement, a linear export, a negative scan route, a direct scan route, inactive grain, and zero strength. Full-frame output MUST be compared with the CPU reference using declared budgets.

Exposure X7 output is not an acceptance oracle for this contract. A future vendor comparison MUST identify the executable build, preset, input, export settings, and comparison domain. This contract MUST NOT be described as pixel-aligned with Exposure X7.
