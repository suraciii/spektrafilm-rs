# Simulation methods

## Authority

| Capability | Owns |
| --- | --- |
| This capability | Spectral and RGB method language; method mapping to render modes; method labels. |
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

- Every effect capability MUST declare one track model: shared, dual-track, or single-track.
- A shared model runs in both methods and MUST declare its input-domain mapping.
- A dual-track capability MUST define separate input, parameters, and output composition for each method.
- A single-track capability MUST state why the other method has no counterpart.
- A capability MUST NOT claim dual-track status by renaming one shared model.
- [Halation models](../halation/spec.md) is the reference dual-track contract.

## Presentation

Visible labels follow [GUI language](../gui-language/spec.md#simulation-method-and-halation-labels).

A method label MUST NOT change `workflow.mode`.

## Acceptance

1. Film presents Spectral and executes Film Halation; LUT and Off present RGB and execute RGB Halation.
2. RGB LUT and RGB Effects remain distinguishable at the workflow level and share RGB Halation.
3. Method labels do not change serialized mode values or activate an inactive branch.
