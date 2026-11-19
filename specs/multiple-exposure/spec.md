# Multiple exposure

## Authority

| Capability | Owns |
| --- | --- |
| This capability | The multiple-exposure stack, method mapping, and Film and RGB composition. |
| Film chain | Film record exposure, camera-side operations, development, print, and scan. |
| LUT transform | Linear working space, per-shot Diffusion and RGB Halation, LUT, LUT Grade, output encoding, and Grain. |
| CLI | Command surface, recipe schema, and stack errors. |
| GUI product design | No multiple-exposure inspector. |
| Presets | No preset ownership. |

## Scope

Multiple exposure composes two or more photographs on one output frame.
Two schemes exist: Film Multiple Exposure and RGB Multiple Exposure.
The schemes share a stack shape but not a blend law.

**Film Multiple Exposure** composes film record exposure before development.
**RGB Multiple Exposure** composes linear light before the active RGB look and output stages.
A multiple-exposure stack is the ordered set of two to eight exposures.

[Simulation methods](../simulation-method/spec.md#declared-tracks) declares the capability dual-track.

## Shared contract

| Rule | Requirement |
| --- | --- |
| Exposure count | Two to eight. |
| Method | Every exposure in one stack MUST use the same scheme. |
| Input | A stack MUST NOT accept a positional input, stage artifact, or preset. |
| Geometry | Every exposure MUST share the working geometry. |
| Transform | A per-exposure crop, resize, rotation, or other transform is outside this capability. |
| Exposure offset | Each exposure MUST declare one finite offset in stops, within the declared control domain. |
| Composition point | Composition MUST precede every stage that runs once per stack. |
| Output crop | The output crop MUST apply after composition. |
| Headroom | The stack MUST preserve floating headroom until the ordinary chain boundary. |
| Determinism | The stack MUST be deterministic for the same sources, offsets, controls, seed, and backend. |
| Backend parity | CPU and GPU MUST use the same composition semantics. |
| Memory | A failed memory check MUST publish no output. |
| Preview | A preview MUST use the same composition and effect order at a reduced long edge. |

## Film Multiple Exposure

- Each exposure MUST pass source interpretation, metering, camera conversion, and the camera-side film exposure operations.
- The camera-side operations include camera diffusion, lens blur, camera light leak, and Film Halation.
- Every exposure MUST share the selected film, print reference, route, source interpretation, working geometry, and working pitch.
- Camera RAW, lens correction, channel swap, route, scan profile, scan output, and direct scan MUST apply to the whole stack.
- A per-exposure RAW or scan option is outside this capability.
- The offset MUST apply after metering and before the camera-side optical operations.
- Composition MUST add record exposure before the density curve. It MUST NOT add density.
- The density curve, DIR couplers, and material grain MUST run once after the last exposure.
- The selected downstream print and scan operations MUST run once after composition.
- The stack MUST accept film-forming image entries, including camera RAW.
- A completed material state MUST NOT be a stack input or output.
- A nonzero global camera exposure compensation MUST fail. The per-exposure offsets own the exposure decision.

| Metering policy | Behavior |
| --- | --- |
| `each` | Meter each exposure on its own input. |
| `lock-to-first` | Meter the first exposure and reuse its decision for the stack. |
| `manual` | Apply the declared offsets without automatic metering. |

Film Multiple Exposure MUST follow the [Film chain](../film-chain/spec.md) stage definitions.

## RGB Multiple Exposure

- Each exposure MUST decode to the linear sRGB working space under [LUT transform](../lut-transform/spec.md#lut-input).
- The offset MUST apply after decoding and before any spatial effect.
- Each exposure MUST clamp negative channel values to zero after the offset. Upper headroom MUST remain unclamped.
- Each exposure MUST run the active pre-LUT effects at their [LUT transform](../lut-transform/spec.md#color-order) placement.
- Composition MUST add linear light. It MUST NOT add density, log exposure, or encoded values.
- RGB Multiple Exposure MUST NOT run film metering, camera light leak, film development, DIR couplers, material grain, print, or scan.
- The active LUT and LUT Grade MUST run once after composition.
- Output gamut and output encoding MUST run once after composition.
- Active Effects Grain MUST run once after composition.
- RGB Halation MUST see each exposure before composition.
- An active LUT MUST see the composite.
- RGB Multiple Exposure runs in the RGB workflow.
- The LUT, LUT Grade, Diffusion, RGB Halation, and Grain modules are independently active.
- An inactive module MUST be an exact bypass.
- RGB Multiple Exposure MUST accept raster input only. It MUST NOT accept camera RAW.
- A metering policy is outside RGB Multiple Exposure. Per-exposure offsets are manual.
- RGB Multiple Exposure MUST NOT claim Film visual equivalence. Equivalence belongs to a declared [look pair](../presets/spec.md#look-pairs).

## Parameters

| Parameter | Requirement |
| --- | --- |
| Method | `film` or `rgb`; one value per stack. |
| Exposures | Two to eight ordered inputs. |
| Exposure offset | One finite value per exposure. |
| Source interpretation | One input color space and CTF decoding for the stack. |
| Output crop | Optional output rectangle. |
| Seed | One deterministic seed for the stack. |
| Memory bound | Optional host-memory bound. |
| Film group | Film selection, print reference, metering, route, scan, RAW, and channel-swap values. |
| RGB group | Active LUT/LUT Grade values and Effects values. |
| Weights | Not admitted; offsets express relative exposure. |
| Per-exposure transform | Not admitted. |
| Per-exposure preset | Not admitted. |

## Recipe

A stack recipe MUST carry the method, exposures, offsets, source interpretation, seed, output crop, and the selected method's group.
A recipe MUST NOT carry a per-exposure transform or a preset.

The [CLI](../cli/spec.md#multiple-exposure) owns the schema version and field names.

## Failure

A stack MUST fail before rendering when any of the following is true:

- The exposure count is outside two to eight.
- The method is missing or mixed.
- A required method group is missing or the other group is present.
- A per-exposure offset is nonfinite or outside the control domain.
- The inputs do not share geometry.
- A per-exposure transform or preset is requested.
- The memory bound cannot hold the stack.

A failed stack MUST publish no output.

## GUI

A multiple-exposure inspector is outside [GUI product design](../gui-product-design/spec.md#non-goals).
The [CLI](../cli/spec.md#multiple-exposure) is the product surface for both methods.

## Acceptance

1. A Film stack sums record exposure and develops once.
2. An RGB stack sums per-shot linear light; active pre-LUT effects run per shot; active post-composition modules run once.
3. A mixed-method stack or a per-exposure transform fails before output.
