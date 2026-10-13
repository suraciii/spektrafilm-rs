# Film grain

## Route identity

- The three grain routes have stable serialized values and names: `spektra` (Spektra), `procedural` (Procedural), and `dye` (Dye).
- Spektra is the upstream experimental original and the default route. Procedural is derived from Dehancer; its behavior is not claimed to have pixel parity with Dehancer. Dye is self-developed and experimental; it is opt-in.
- The routes coexist independently. Selecting one route does not imply an upgrade, migration, or behavioral equivalence to another route.
- CLI and serialized `film_render.grain.engine` values are exactly `spektra`, `procedural`, and `dye`; legacy numeric selectors `v1`, `v2`, and `v3` are invalid.

## Contract

- Grain is optional. `active = false` is an exact bypass.
- The grain engine is selected explicitly. Spektra is the default; Procedural and Dye are opt-in.
- Spektra operates in film-density space and uses film-profile chemistry. Dye's material-density boundary is defined in its [design](v3/design.md#processing-boundary). Procedural is a post-scan grain appearance on native encoded scanner RGB, not material-grain formation. Its [contract](v2/spec.md) owns the exact boundary.
- Existing density-sensitive grain continues to consume its own stage's material or scanner-domain input. Preflash does not alter upstream film density; changing a material control does not make grain engines share a domain. [Output mixing](../material-response/spec.md#output-mix) blends the completed result without regenerating or reseeding any grain engine.
- A grain seed makes a realization repeatable for the same input, parameters, and implementation. CPU and WGPU realizations must share semantics, not bit identity.
- Spektra, Procedural, and Dye are separate engines. Their controls, processing domains, and backend requirements must not be conflated.
- GUI and CLI descriptions must distinguish material grain (Spektra and Dye) from post-scan grain appearance (Procedural). Shared selection and serialized ownership do not imply a shared execution domain. Procedural's Analogue mode and Film Resolution controls must not be described as upstream material formation or scanner lens optics.
- Procedural provides `Analogue` and `Noise` modes, Negative and Positive film types, named format/speed profiles, and a `custom` profile with explicit controls.
- Procedural profile controls are bounded: Size 1–48; Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution 0–100.
- Dye targets convincing, visually pleasing grain formed by a film-plane particle-equivalent dye field read by output samples. Real film scans define its reference. Its [contract](v3/spec.md) defines the initial validation scope and acceptance criteria.

Engine-specific contracts live in [Spektra](v1/spec.md), [Procedural](v2/spec.md), and [Dye](v3/spec.md).
