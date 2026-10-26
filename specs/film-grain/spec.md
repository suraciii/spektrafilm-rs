# Film grain

## Route identity

- The four grain routes have stable serialized values and names: `spektra` (Spektra), `procedural` (Procedural), `dye` (Dye), and `fotufilm` (Fotufilm Film).
- Spektra is the upstream experimental original.
- Procedural is derived from Dehancer; its behavior is not claimed to have pixel parity with Dehancer.
- Dye is self-developed and experimental.
- The routes coexist independently.
- Selecting one route does not imply an upgrade, migration, or behavioral equivalence to another route.
- CLI and serialized `film_render.grain.engine` values are exactly `spektra`, `procedural`, `dye`, and `fotufilm`; legacy numeric selectors `v1`, `v2`, and `v3` are invalid.

## Defaults and compatibility

New runtime parameters, GUI factory state, and Grain module resets MUST select Dye.

An omitted engine in an unversioned sparse source MUST use the current default. A caller that requires Spektra MUST specify `film_render.grain.engine = spektra`.

GUI state version 6 owns this default cutover. Loading a version 1–5 GUI document that omits the engine MUST preserve its historical Spektra intent as an explicit engine. A current or unversioned GUI document that omits the engine MUST use Dye.

An explicit engine in a state, preset, or recipe MUST remain authoritative. The cutover MUST NOT convert existing explicitly authored looks or insert an engine into a sparse section preset.

This default change does not preserve pixels for an omitted engine in a new sparse source. It MUST NOT imply stock calibration, physical-fidelity acceptance, or equivalence between engines.


## Contract

- Grain is optional. `active = false` is an exact bypass.
- The grain engine is selected through the shared engine parameter.
- Spektra operates in film-density space and uses film-profile chemistry.
- Dye's processing boundary is defined in its [design](v3/design.md#processing-boundary).
- Fotufilm Film adds a fitted-population residual at the film-density boundary under [its contract](fotufilm/spec.md).
- Procedural is a post-scan appearance on native encoded scanner RGB, not material-grain formation, under [its contract](v2/spec.md).
- Existing density-sensitive grain continues to consume its own stage's material or scanner-domain input.
- Preflash does not alter upstream film density; changing a material control does not make grain engines share a domain.
- [Output mixing](../output-mix/spec.md#output-mix) blends the completed result without regenerating or reseeding any grain engine.
- A grain seed makes a realization repeatable for the same input, parameters, and implementation.
- CPU and WGPU executors MUST implement the same grain algorithm and share seed, draw, count, and coordinate semantics under [GPU algorithm alignment](../gpu-execution/spec.md#algorithm-alignment) and [stochastic alignment](../gpu-execution/design.md#stochastic-alignment).
- Float outputs need not be bit-identical.
- All four engines are independent.
- Their controls, processing domains, and backend requirements MUST NOT be conflated.
- GUI and CLI descriptions MUST distinguish material grain from post-scan grain appearance.
- Shared selection and serialized ownership do not imply a shared execution domain.
- Procedural's Analogue mode and Film Resolution controls MUST NOT be described as upstream material formation or scanner lens optics.
- Procedural provides `Analogue` and `Noise` modes, Negative and Positive film types, named format/speed profiles, and a `custom` profile with explicit controls.
- Procedural profile controls are bounded: Size 1–48; Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution 0–100.
- Dye targets convincing color-film grain and supplies an effective single-record monochrome model, formed in film coordinates and read through output footprints.
- Its [contract](v3/spec.md) defines the product boundary and acceptance gates.
- Calibration evidence is separate and does not lock algorithm values.

Engine-specific contracts live in [Spektra](v1/spec.md), [Procedural](v2/spec.md), [Dye](v3/spec.md), and [Fotufilm Film](fotufilm/spec.md).
