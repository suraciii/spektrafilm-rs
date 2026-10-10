# Film grain

## Contract

- Grain is optional. `active = false` is an exact bypass.
- The grain engine is selected explicitly. V1 is the default for existing recipes; V2 is opt-in. V3 is a planned opt-in engine.
- V1 operates in film-density space and uses film-profile chemistry. V2 operates on native scanner RGB after the scanner color encoding. V3's planned processing boundary is defined in its [design](v3/design.md#processing-boundary).
- A grain seed makes a realization repeatable for the same input, parameters, and implementation. CPU and WGPU realizations must share semantics, not bit identity.
- V1, V2, and V3 are separate engines. Their controls, processing domains, and backend requirements must not be conflated.
- V2 provides `Analogue` and `Noise` modes, Negative and Positive film types, named format/speed profiles, and a `custom` profile with explicit controls.
- V2 profile controls are bounded: Size 1–48; Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution 0–100.
- V3 targets convincing, visually pleasing grain formed by a film-plane particle-equivalent dye field read by output samples. Real film scans define its reference. Its [contract](v3/spec.md) defines the initial validation scope and acceptance criteria.

Engine-specific contracts live in [V1](v1/spec.md), [V2](v2/spec.md), and [V3](v3/spec.md).
