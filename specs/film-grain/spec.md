# Film grain

## Route identity

- The three grain routes have stable serialized values and names: `spektra` (Spektra), `procedural` (Procedural), and `dye` (Dye).
- Spektra is the upstream experimental original and the default route. Procedural is derived from Dehancer; its behavior is not claimed to have pixel parity with Dehancer. Dye is self-developed and experimental; it is opt-in.
- The routes coexist independently. Selecting one route does not imply an upgrade, migration, or behavioral equivalence to another route.
- CLI and serialized `film_render.grain.engine` values are exactly `spektra`, `procedural`, and `dye`; legacy numeric selectors `v1`, `v2`, and `v3` are invalid.

## Contract

- Grain is optional. `active = false` is an exact bypass.
- The grain engine is selected explicitly. Spektra is the default; Procedural and Dye are opt-in.
- Spektra operates in film-density space and uses film-profile chemistry. Procedural operates on native scanner RGB after scanner color encoding. Dye's processing boundary is defined in its [design](v3/design.md#processing-boundary).
- [Film and print digital response](../material-response/spec.md#product-model) changes developed physical record density at its owning material stage, before scanning. Density-sensitive grain must respond to the changed density in its existing engine domain; unchanged grain fields must not be assumed. Response changes must not reseed the grain realization or move Spektra, Procedural, or Dye into a common domain. Procedural continues to consume native encoded scanner RGB, including the consequences of eligible upstream material response. [Output mixing](../material-response/spec.md#output-mix) blends the completed result without regenerating or reseeding any grain engine.
- A grain seed makes a realization repeatable for the same input, parameters, and implementation. CPU and WGPU realizations must share semantics, not bit identity.
- Spektra, Procedural, and Dye are separate engines. Their controls, processing domains, and backend requirements must not be conflated.
- Procedural provides `Analogue` and `Noise` modes, Negative and Positive film types, named format/speed profiles, and a `custom` profile with explicit controls.
- Procedural profile controls are bounded: Size 1–48; Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution 0–100.
- Dye targets convincing, visually pleasing grain formed by a film-plane particle-equivalent dye field read by output samples. Real film scans define its reference. Its [contract](v3/spec.md) defines the initial validation scope and acceptance criteria.

Engine-specific contracts live in [Spektra](v1/spec.md), [Procedural](v2/spec.md), and [Dye](v3/spec.md).
