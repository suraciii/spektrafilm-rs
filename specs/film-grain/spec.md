# Film grain

## Contract

- Grain is optional. `active = false` is an exact bypass.
- The grain engine is selected explicitly. V1 is the default for existing recipes; V2 is procedural grain.
- V1 operates in film-density space and uses film-profile chemistry. V2 operates on native scanner RGB after the scanner color encoding.
- A grain seed makes a realization repeatable for the same input, parameters, and implementation. CPU and WGPU realizations must share semantics, not bit identity.
- V1 and V2 are separate engines. Their controls, processing domain, and backend requirements must not be conflated.
- V2 provides `Analogue` and `Noise` modes, Negative and Positive film types, named format/speed profiles, and a `custom` profile with explicit controls.
- V2 profile controls are bounded: Size 1–48; Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution 0–100.

Engine-specific contracts live in [V1](v1/spec.md) and [V2](v2/spec.md).
