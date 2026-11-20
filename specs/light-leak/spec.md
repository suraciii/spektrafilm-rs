# Light leak

## Authority

| Capability | Owns |
| --- | --- |
| This capability | The dual-track relationship, shared light-leak behavior, and the RGB appearance model. |
| [Film chain](../film-chain/spec.md#camera-light-leak) | Film stray-light exposure, spectrum, geometry, and placement. |
| [LUT transform](../lut-transform/spec.md#color-order) | RGB Effects placement, composition order, and runtime storage. |
| [Multiple exposure](../multiple-exposure/spec.md) | Per-exposure leak placement within a stack. |
| [GUI workflow](../gui-workflow/spec.md#stage-and-surface-ownership) | Control placement and editing surfaces. |
| [CLI](../cli/spec.md) | Parameter discovery and command syntax. |
| [Presets](../presets/spec.md#effects-presets) | Film and Effects preset capture. |
| [Simulation methods](../simulation-method/spec.md#declared-tracks) | Track declaration. |

## Contract

Light leak is a [dual-track](../../CONTEXT.md#language) capability. Film simulates stray light that reached the film record during exposure. RGB simulates a synthetic spill on the input image; it MUST NOT claim Film visual or numerical equivalence.

| Method | Input | Model | Placement |
| --- | --- | --- | --- |
| Film | Linear film raw exposure | Spectral stray exposure | After camera diffusion and lens blur; before Film Halation. |
| RGB | Linear working RGB | Tinted additive light | After Diffusion; before RGB Halation and the LUT. |

A leak MUST add light. It MUST NOT modify density, log exposure, or an encoded output value.

A leak MUST use one compact inward falloff. A leak at one edge MUST NOT add visible light at the opposite edge.

A leak MUST preserve floating headroom. A leak strong enough to drive the Film record past its shoulder MUST read the material's white.

The Film spectrum MUST be a per-record exposure ratio. The RGB tint MUST be linear RGB. Neither method MAY apply a fixed tint after development or output encoding.

The two methods MUST NOT share one gain, tint, geometry, or texture parameter group. A shared spatial kernel MAY serve both models when each model supplies its own input, parameters, and composition.

## Film track

Film light-leak behavior is owned by [Film chain](../film-chain/spec.md#camera-light-leak).

The Film leak MUST remain additive film exposure. It MUST use physical film geometry and MUST convert lengths through the working pixel pitch.

## RGB track

RGB light leak is an appearance effect. It MUST NOT read a film profile, film density, spectral sensitivity, camera exposure, or scan response.

The RGB leak MUST be defined in the full composition frame and MUST remain stable across preview scale, crop, and output size. The output crop MUST apply after the leak and after multiple-exposure composition.

The RGB group MUST provide an active switch, a linear-light gain, a linear RGB tint, a compact leak geometry, a texture amount, a texture scale, and a seed. The gain MUST be absolute; it MUST NOT scale with scene brightness.

A zero gain, an inactive group, or an all-zero tint MUST be an exact bypass. Disabling the leak MUST retain its stored values.

The leak MUST be deterministic for the same input, parameters, geometry, and seed. It MUST NOT depend on frame chunking.

The leak MUST NOT be encoded in a LUT. A pointwise LUT cannot carry its geometry or texture.

## Multiple exposure

Film Multiple Exposure runs the Film track per exposure under [Film Multiple Exposure](../multiple-exposure/spec.md#film-multiple-exposure).

RGB Multiple Exposure runs the RGB leak per exposure before linear-light composition under [RGB Multiple Exposure](../multiple-exposure/spec.md#rgb-multiple-exposure).

A leak MUST NOT run a second time on a composed or developed output.

## GUI and presets

The Film stage MUST expose the Film light-leak controls under [GUI workflow](../gui-workflow/spec.md#complete-control-access). The Effects stage MUST expose the RGB light-leak controls. Each method MUST have one editable owner.

A Film preset MUST capture the Film group. An Effects preset MUST capture the RGB group. Built-in presets MUST NOT include an RGB light-leak preset.

## Failure

A nonfinite value, an invalid geometry, or a value outside its declared domain MUST fail the active render before output. A failed render MUST preserve the preceding accepted image and parameters.

## Acceptance

1. Film adds stray exposure after camera optics and before Film Halation; an inactive leak is an exact bypass.
2. RGB adds tinted linear light before RGB Halation and the LUT; an inactive or zero-gain leak is an exact bypass and reads no film data.
3. Multiple exposure applies the active leak per exposure before composition through the method's single GUI owner.
