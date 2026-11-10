# LUT look design

The [product contract](spec.md) owns observable LUT mode behavior. This document owns the mode boundary, evaluation order, shared and independent implementations, and invariants.

## Ownership

| Boundary | Owner |
| --- | --- |
| Film, print, spectral exposure, development, scan | [Film chain](../film-chain/spec.md) |
| Film-chain LUT export, bundle metadata, bake QA | [LUT delivery](../lut-delivery/spec.md) |
| LUT import, external LUT declaration, LUT stage | This capability |
| Halation, diffusion, and grain parameter models | This capability |
| Grade response and Oklab evaluation | [Tone and color](../finishing/spec.md) |
| Film grain engines | [Film grain](../film-grain/spec.md) |
| CPU/GPU evaluation | [GPU execution](../gpu-execution/spec.md) |
| GUI controls and mode presentation | [GUI workflow](../gui-workflow/spec.md) |
| CLI syntax and parameter carriers | [CLI](../cli/spec.md) |
| Film look presets | [Presets](../presets/spec.md) |

LUT delivery exports a film-chain transform. LUT mode imports an external transform. They MUST share file parsing and interpolation primitives without sharing a runtime contract.

## Implementation boundaries

| Area | Boundary |
| --- | --- |
| External LUT parsing | One shared parser reads `.cube` data; LUT look owns its declaration and validation |
| LUT look parameters | Core runtime parameters own the mode, LUT declaration, effects, and grade |
| Mode dispatch | The pipeline selects one film or LUT stage graph; no per-stage mode flags |
| Halation and grain | LUT-specific effect models; shared PSF and noise primitives only |
| Diffusion | Shared optical PSF; placed before the LUT |
| Grade | Existing finishing response; optional post-LUT stage |
| GUI and CLI | Adapters over the same core declaration; no private color transform |

The implementation MUST NOT create a second LUT reader, a second color-space table, or a second finishing response.

## Mode boundary

```text
RenderMode
├── Film → existing spectral film/print/scan pipeline
└── Lut  → LUT look pipeline
```

The mode is a top-level dispatch. LUT mode MUST NOT call film exposure, development, print, scan, spectral preparation, or film-grain engines.

Film mode MUST NOT load an external LUT look. A LUT look is not a film profile and MUST NOT populate `film_profile` or `print_profile`.

Inactive mode values are retained as stored parameters. They are not consumed and MUST NOT cause material preparation.

## Evaluation order

LUT mode MUST evaluate in this order:

```text
1. Decode input and convert to linear working RGB.
2. Diffusion in linear light.
3. Halation in linear light.
4. Shaper and LUT input transform.
5. 3D LUT.
6. Decode LUT output to linear output RGB.
7. Optional grade.
8. Existing output gamut processing.
9. Output encoding.
10. Grain in output encoding.
11. Publish output.
```

The order is fixed. No effect reorders around the LUT.

Diffusion models lens scatter; halation models film and base scatter. Diffusion therefore precedes halation.

Both effects see scene-linear highlights before the LUT compresses them. Grain sees final output encoding, so it is independent of LUT contrast.

## LUT stage

### Declaration

```rust
pub struct LutSpec {
    pub path: PathBuf,
    pub sha256: String,
    pub input_space: LutColorSpace,
    pub output_space: LutColorSpace,
    pub domain_min: [f64; 3],
    pub domain_max: [f64; 3],
    pub domain_policy: LutDomainPolicy,
    pub shaper: LutShaper,
    pub strength: f64,
}

pub enum LutColorSpace {
    LinearSrgb,
    DisplaySrgb,
    DisplayP3,
}
// DisplaySrgb and DisplayP3 carry display encoding.

pub enum LutDomainPolicy {
    Error,
    Clamp,
}

pub enum LutShaper {
    None,
    Cube1d,
}
```

The declaration is authoritative. The implementation MUST NOT infer a color space from a filename, a film profile, image metadata, or image content.

### Domain and shaper

A 3D LUT maps a normalized domain to output values. The domain MUST be finite and strictly ordered on every axis. The domain policy MUST be declared; `Error` is the import default, not an implicit omitted value.

If the input exceeds the domain, the LUT look MUST either apply a declared 1D shaper or select an explicit clamp. Silent clipping is a contract violation. An explicit clamp MUST report its clipped fraction through execution reporting.

The 1D shaper, when present, maps the working input into the 3D domain. It is not a grade and MUST NOT be described as one.

### Interpolation and strength

The 3D LUT is evaluated pointwise with tetrahedral interpolation. LUT resolution is the resolution declared by the file; the implementation MUST NOT silently resample it.

`strength = 0` MUST bypass the LUT input transform, shaper, and 3D lookup. `strength = 1` MUST return the LUT result exactly. Intermediate strength blends in the declared output space.

The LUT stage MUST NOT allocate a spatial neighborhood, compute a histogram, or inspect neighboring pixels.

### File identity

The declaration carries a SHA-256 content hash. A changed file, missing file, or parse failure invalidates the declaration.

The implementation MUST NOT search for a similarly named file. The user MUST reselect the LUT.

## Effects

### Placement contract

```rust
pub enum EffectPlacement {
    PreLut,
    PostLut,
    OutputEncoding,
}
```

| Effect | Placement | Working domain | Shared implementation | Independent semantics |
| --- | --- | --- | --- | --- |
| Diffusion | PreLut | Linear scene RGB | Camera diffusion PSF/models | No film or print stage |
| Halation | PreLut | Linear scene RGB | PSF primitive | Highlight mask, tint, threshold, no film density |
| Grade | PostLut | Linear output RGB / Oklab | `finishing` | Optional, LUT-independent |
| Grain | OutputEncoding | Encoded output RGB | Noise/hash primitive | Appearance grain, no film density or RMS |

The table does not permit sharing a parameter object. LUT effects use their own parameter groups.

### Halation

Halation MUST build a highlight mask in linear light. A representative response is:

\[
m(x)=\operatorname{smoothstep}(t,1,L(x)), \qquad
h(x)=a\,T\,(G_\sigma * m)(x),
\]

where `t` is the threshold, `L` is linear luminance, `G` is the declared spatial kernel, `a` is intensity, and `T` is a tint triplet.

The implementation MUST protect the brightest source pixels from an unbounded additive wash. It MUST NOT read film exposure, record density, or a film profile.

The PSF primitive MAY be shared with film halation. The mask, tint, threshold, and placement MUST remain LUT-specific.

### Diffusion

Diffusion MUST reuse the selected optical PSF and model from camera diffusion. It MUST run before the LUT.

The effect MUST be off by default. A LUT that already carries lens character MUST NOT acquire diffusion implicitly.

### Grade

Grade MUST call the existing finishing response on linear output RGB. It MUST preserve its existing neutral bypass and parameter validation.

Grade MUST be optional. LUT mode MUST NOT add a second tone curve, a second white balance, or a second output transform around it.

### Grain

Grain MUST evaluate after output encoding. It MUST NOT call Spektra, Dye, Fotufilm Film, or Procedural grain engines.

A representative response is:

\[
g(x)=n(x)\,A\,w(L(x)), \qquad
w(L)=w_s(1-L)^2+w_m4L(1-L)+w_hL^2,
\]

where `n` is a seeded noise field, `A` is amount, and `w_s`, `w_m`, `w_h` are shadow, midtone, and highlight weights.

The default is monochrome grain. Chroma MAY add a second noise field without changing luminance.

The seed and draw order MUST be deterministic. Grain MUST NOT change LUT or grade parameters.

## Data model

```rust
pub enum RenderMode {
    Film,
    Lut,
}

pub struct LutLookParams {
    pub lut: LutSpec,
    pub effects: LutEffectParams,
    pub grade: Option<FinishingParams>,
}

pub struct LutEffectParams {
    pub halation: Option<LutHalationParams>,
    pub diffusion: Option<DiffusionParams>,
    pub grain: Option<LutGrainParams>,
}
```

`LutHalationParams` and `LutGrainParams` are distinct from `film_render.halation` and `film_render.grain`. Field names MUST NOT alias film-density controls.

A complete runtime parameter document without a LUT look resolves to film mode. A LUT look is active only when the mode is `Lut` and a LUT declaration is present.

A newly imported LUT look starts at strength one with effects and grade disabled.

## Validation

Validation MUST occur before image execution and before output publication.

| Condition | Decision |
| --- | --- |
| `mode = Lut` without a LUT declaration | Reject |
| `mode = Film` with a stored LUT declaration | Ignore the LUT declaration; do not load it |
| A LUT look declaration that carries a film or print profile reference | Reject |
| Missing or invalid domain policy | Reject |
| Unsupported or malformed carrier | Reject |
| Missing input or output space | Reject |
| Unsupported color space | Reject |
| Domain nonfinite, reversed, or empty | Reject |
| Input outside domain without shaper or explicit clamp while the LUT is applied | Reject |
| Missing file or hash mismatch | Reject |
| Strength outside zero to one | Reject |
| Nonfinite or out-of-range effect value | Reject |
| Effect in the wrong placement | Reject |

LUT mode MUST NOT validate or prepare spectral materials while active. Film mode MUST NOT validate or load a stored LUT look while active.

Inactive values remain stored and do not affect the render. Active-mode validation applies only to the active mode and its declaration.

A rejected update MUST preserve the preceding accepted parameters, image, and output.

## Execution

CPU and GPU MUST evaluate the same LUT declaration, interpolation, effect order, and effect model. A backend MUST NOT substitute a different color transform or skip a spatial effect.

The LUT lookup and grain field MUST be deterministic for the same input, declaration, parameters, seed, precision, and backend.

Preview reduction MUST use the same declaration and effect definitions. A reduced preview MUST NOT claim full-resolution equivalence for spatial effects.

LUT mode MUST use existing image input, output gamut, output encoding, photo export, and display-transform boundaries. It MUST NOT add a second image writer or a second output color pipeline.

## Persistence and compatibility

Complete parameter documents, GUI state, and render recipes carry the LUT look as runtime parameters. They MUST retain the path, hash, color spaces, domain, domain policy, shaper, strength, effects, and grade.

A LUT file is not embedded in a document. A document carries a reference and hash; a missing or changed file is a validation failure.

Mode switching preserves inactive values. It MUST NOT reset film controls when entering LUT mode or reset LUT controls when entering film mode.

This capability does not change the preset schema. Film look presets remain film-only. A future LUT look preset MUST define its own preset contract before it can enter the preset library.

## Licensing and provenance

A user-imported LUT retains its own license. The product MUST NOT bundle a third-party LUT without a compatible license.

The LUT hash, declared color spaces, domain, domain policy, shaper, and strength are provenance. They identify the transform but do not establish color accuracy or a film-stock claim.

Adobe XMP and DCP creative profiles are not LUT carriers in this contract. A user MUST supply a `.cube` with an explicit declaration.

## Laws

1. A LUT is pointwise; it carries no spatial effect.
2. LUT mode never loads film, print, or spectral material.
3. Film mode never loads an external LUT look.
4. Effects are independent; disabling one is an exact bypass.
5. No color space, domain, clamp, or fallback is inferred silently.
6. The evaluation order is fixed.
7. Same input, LUT hash, parameters, seed, precision, and backend produce a repeatable result under the compute contract.
