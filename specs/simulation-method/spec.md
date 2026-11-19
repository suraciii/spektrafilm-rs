# Simulation methods

## Authority

| Capability | Owns |
| --- | --- |
| This capability | Spectral and RGB method language; method mapping to render modes. |
| [Render modes](../render-mode/spec.md) | Film, LUT, and Off activation; stored-mode values; switching and inactive-value retention. |
| [Film chain](../film-chain/spec.md) | Spectral simulation behavior. |
| [LUT transform](../lut-transform/spec.md) | RGB simulation behavior. |
| [Halation models](../halation/spec.md) | Film Halation and RGB Halation. |
| [GUI language](../gui-language/spec.md) | Visible labels and help text. |
| [Presets](../presets/spec.md) | Preset kinds and mode selection through preset application. |

## Contract

A film simulation has one method: **Spectral** or **RGB**.

**Spectral** computes the film/print color transform from spectral distributions.

**RGB** computes the film look directly in RGB, including color transforms and film effects.

RGB names the computation and input domain.

RGB does not name the output format; a Spectral simulation also outputs RGB.

## Mapping

| Method | Render mode | Look source | Effect input | Halation |
| --- | --- | --- | --- | --- |
| Spectral | Film | Film chain | Film Input | Film Halation |
| RGB | LUT | LUT | RGB Input | RGB Halation |
| RGB | Off | Identity | RGB Input | RGB Halation |

- Film mode is the only Spectral mode.
- LUT and Off are RGB modes.
- The RGB look source is LUT or Identity.
- LUT Grade is optional and applies only in LUT.
- Method is derived from the active render mode.
- `workflow.mode` remains `film`, `lut`, or `off`.
- The product MUST NOT add a fourth render mode.

## Effect tracks

- A new or changed effect capability MUST declare one [effect track](../../CONTEXT.md#language): shared, dual-track, or single-track.
- A shared model runs in both methods and MUST declare its input-domain mapping.
- A dual-track capability MUST define separate input, parameters, and output composition for each method.
- A single-track capability MUST state why the other method has no counterpart.
- A capability MUST NOT claim dual-track status by renaming one shared model.
- [Halation models](../halation/spec.md) is the reference dual-track contract.
- Its RGB scheme is calibrated to the Film visual target; pixel identity is not required.

## Declared tracks

The table declares each capability's track. The owning specification states its behavior.

| Capability | Track | Owner | Relationship |
| --- | --- | --- | --- |
| Halation | dual-track | [Halation models](../halation/spec.md) | Separate Film and RGB schemes over one spatial response; RGB is calibrated to the Film visual target. |
| Diffusion | shared | [Film chain](../film-chain/spec.md) / [LUT transform](../lut-transform/spec.md#diffusion) | One optical response at the camera, enlarger, and pre-LUT placements. |
| Grain | dual-track | [Film grain](../film-grain/spec.md) | Film material and post-scan engines use different inputs and parameters from Effects Grain. |
| Tone and color | shared | [Finishing](../finishing/spec.md) | One response at Scan Tone & Color and LUT Grade. |
| Multiple exposure | dual-track | [Multiple exposure](../multiple-exposure/spec.md) | Film sums record exposure before development; RGB sums per-shot linear light before the LUT transform. |
| Camera light leak | single-track | [Film chain](../film-chain/spec.md#camera-light-leak) | RGB has no film record exposure; a synthetic leak would be a separate effect. |
| Print viewing glare | single-track | [Film chain](../film-chain/spec.md#scan-optics-and-print-viewing-glare) | RGB has no print readout light. |
| Scanner lens blur | single-track | [Film chain](../film-chain/spec.md#scan-optics-and-print-viewing-glare) | RGB has no optical scan acquisition. |
| Unsharp mask | single-track | [Film chain](../film-chain/spec.md#scan-optics-and-print-viewing-glare) | RGB has no scan sharpening stage; generic sharpening would be a separate effect. |

Output gamut is one shared output transform. Film material, print, scan, CMYK, and camera metering are single-track by design; their owning specifications state activation.

## Presentation

Visible labels follow [GUI language](../gui-language/spec.md#simulation-method-and-halation-labels).

A method label MUST NOT change `workflow.mode`.

## Acceptance

1. Film presents Spectral and executes Film Halation; LUT and Off present RGB and execute RGB Halation.
2. RGB LUT and RGB Effects remain distinguishable at the workflow level and share RGB Halation.
3. Method labels do not change serialized mode values or activate an inactive branch.
