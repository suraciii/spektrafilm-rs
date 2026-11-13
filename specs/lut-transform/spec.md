# LUT transform

## Authority

| Capability | Owns |
| --- | --- |
| This capability | The non-spectral LUT transform, external LUT color management, and independent effect layers |
| [Render modes](../render-mode/spec.md) | Film, LUT, and Off selection; stage activation; switching and inactive-value retention |
| [Film chain](../film-chain/spec.md) | Spectral film and print simulation |
| [LUT delivery](../lut-delivery/spec.md) | Exporting the film chain |
| [Tone and color](../finishing/spec.md) | Grading |
| [Film grain](../film-grain/spec.md) | Film grain engines |
| [Presets](../presets/spec.md) | Film presets; LUT preset library behavior is owned by the [LUT workflow](../lut-workflow/spec.md#lut-presets) |

## Scope

A LUT transform uses an external 3D color transform plus independent effect layers. It is non-spectral and operates on RGB. It does not model film sensitivity, record density, development, printing, scanning, or illumination.

A **LUT mode** renders a LUT transform. A **Film mode** renders the spectral film chain. **Off mode** renders the effects without a LUT or grade.

[Render modes](../render-mode/spec.md) owns mode selection and activation.

The product-level LUT export/import handoff and LUT preset lifecycle are owned by the [LUT workflow](../lut-workflow/spec.md).

| Property | Film mode | LUT mode | Off mode |
| --- | --- | --- | --- |
| Color source | Spectral film and print simulation | External 3D LUT | Input image |
| Material profiles | Required by the route | Not consumed | Not consumed |
| Illuminant adaptation | Spectral | None | None |
| Grain | Film-density or post-scan engine | Independent appearance layer | Independent appearance layer |
| Halation | Film exposure and density | Independent highlight layer | Independent highlight layer |
| Diffuse optics | Camera or enlarger stage | Pre-LUT effect | Pre-LUT effect |
| Grade | Scan Tone & Color | LUT Grade | None |
| Spectral claims | Governed by film chain | None | None |

LUT mode MUST NOT be described as a film stock, a measured film response, or a spectral simulation. A LUT MAY encode a rendering derived from film, but the LUT transform itself is a colorimetric transform.

LUT and Off modes MUST NOT add scene atmosphere, fog, motion integration, volumetric scene glow, local masks, or subject-aware adjustments. Those are outside this contract.

## LUT input

A LUT transform MUST use a 3D LUT and MAY declare a 1D shaper before it. The supported carrier is `.cube`.

The LUT declaration MUST include:

- the LUT file;
- its content hash;
- its input color space;
- its output color space;
- its domain minimum and maximum;
- its domain policy;
- its optional shaper;
- its strength.

The supported input and output encodings are linear sRGB, display-encoded sRGB, and display-encoded Display P3. Camera-RGB LUTs are not admitted in this contract.

The domain policy is either fail on out-of-domain input or an explicit clamp. The import UI MUST preselect clamp, and the declaration MUST carry the selected policy. The import UI MUST expose fail as a strict option and explain both policies at the decision point.

A missing, ambiguous, or unsupported color-space declaration MUST fail before output. The product MUST NOT infer a color space from a filename, a film profile, or image content.

A LUT whose domain does not cover the input MUST use a declared shaper, select an explicit clamp, or fail before output while the LUT is applied. Silent clipping is forbidden. An explicit clamp MUST report that clipping occurred.

LUT strength is a finite value from zero to one. Zero MUST bypass the LUT color transform exactly; it MUST NOT bypass active effects. One MUST apply the declared transform without mixing.

The LUT file is external to the product. The product MUST NOT silently substitute another file when the declared file is missing or its hash differs.

## Color order

LUT mode MUST use one fixed evaluation order:

```text
input
  → linear working RGB
  → diffusion
  → halation
  → shaper / LUT input transform
  → 3D LUT
  → LUT Grade
  → output gamut
  → output encoding
  → grain
```

Off mode MUST use the same order with the shaper, LUT, and LUT Grade as exact bypasses:

```text
input
  → linear working RGB
  → diffusion
  → halation
  → output gamut
  → output encoding
  → grain
```

- Diffusion MUST evaluate in linear light before halation.
- Halation MUST evaluate in linear light before the LUT.
- The LUT MUST remain pointwise.
- LUT Grade MUST follow the LUT.
- Output gamut MUST follow the LUT or LUT Grade.
- Grain MUST follow output encoding.

The order MUST NOT be user-reorderable. An inactive effect MUST be an exact bypass.

## Effects

The runtime root is `effects`. Its `diffusion`, `halation`, and `grain` groups are optional and each carries `active` plus its declared controls.

The canonical effect switch paths are `effects.<effect>.active`. Enabling or disabling one effect MUST NOT change another effect's stored values or the LUT declaration.

The Effects stage is active in LUT and Off. It uses the same effect definitions and values in both modes. Film mode MUST NOT execute these effects; film material effects remain in Film and Print.

An effect MUST NOT be encoded in the LUT. A 3D LUT is pointwise and cannot carry a spatial radius, a grain field, or a highlight mask.

### Halation

Halation MUST use a linear-light highlight mask and a declared spatial response. It MUST NOT read film density, film exposure, or a film profile.

The effect MUST expose a luminance threshold, a spatial radius, an intensity, a tint, and a highlight-protection range. Its tint is an appearance choice, not a measured stock response.

### Diffusion

Diffusion MUST provide the existing optical diffusion responses where their declared response is applicable. It MUST run in linear light before the LUT.

A LUT transform that already contains lens character MUST allow diffusion to remain off. The product MUST NOT infer that a LUT contains diffusion.

### LUT Grade

LUT Grade MUST use the existing [Tone and color](../finishing/spec.md) response in the root `grade` group. It MUST remain optional and MUST NOT become part of the LUT.

`grade.active` switches the response. Disabling it MUST retain the response values and bypass it exactly; re-enabling it MUST restore the same accepted response. LUT Grade is active only in LUT mode, and Off mode MUST bypass it.

### Grain

Grain MUST be an output-referred appearance layer. It MUST NOT use a film grain engine, film RMS granularity, or a film profile.

The effect MUST expose an amount, a size, separate shadow, midtone, and highlight weights, an optional chroma component, and a seed. The default grain is monochrome.

A seed MUST make a realization repeatable for the same input, parameters, and implementation. Grain MUST NOT alter the LUT color transform or the grade definition.

## Comparison and preview

Preview, Scan, and Export MUST use the same LUT declaration, color order, and effect definitions. A reduced preview MUST NOT claim full-resolution equivalence for halation, diffusion, or grain.

The product MUST provide a neutral gray ramp and a color-checker preview for an imported LUT. The preview MUST NOT auto-fit exposure, white balance, or contrast to make the LUT appear correct.

## UI and CLI

[Render modes](../render-mode/spec.md) owns the global Film / LUT / Off selector. This capability owns the LUT stage and the Effects stage content.

The LUT stage MUST expose LUT import, the declared input and output spaces, the domain, the domain policy, the shaper, LUT strength, and LUT Grade.

The Effects stage MUST expose independent Diffusion, Halation, and Grain controls. It MUST be active in LUT and Off and inactive in Film.

The LUT stage MUST show all three domain axes. When a clamp render reports clipped pixels, the LUT stage MUST show a persistent warning with the clipped percentage. The warning MUST NOT depend on hovering a truncated header status.

A new LUT import MUST default to strength one with every effect inactive. A replacement cube preserves the existing strength, Effects, and LUT Grade values under the [LUT workflow](../lut-workflow/spec.md#import-lut).

The UI MUST label the feature as a non-spectral LUT. It MUST NOT present a LUT as a film profile or a measured stock.

The CLI MUST expose the mode, LUT declaration, Effects, and LUT Grade through its canonical parameter language. Exact flags and option names belong to the [CLI contract](../cli/spec.md).

## Persistence

LUT transform and Effects settings are runtime parameters. Complete parameter documents, GUI state, and render recipes MUST retain the LUT reference, content hash, color spaces, domain, domain policy, shaper, strength, Effects, and LUT Grade.

A LUT transform is not a film preset. It MAY be captured as a LUT preset under the [LUT workflow](../lut-workflow/spec.md#lut-presets). Effects without a LUT MAY be captured as an [Effects preset](../presets/spec.md#effects-presets).

A saved LUT reference MUST fail validation while LUT mode is active when the file is missing or its content hash differs. The product MUST NOT replace the reference with another LUT or fall back to Film mode. A LUT preset references an external LUT file by path and hash; the preset document does not embed the LUT.

A mode switch MUST preserve the stored LUT transform and Effects. It MUST NOT reinterpret film controls.

## Failure

A LUT transform MUST fail before publishing output when any of the following is true:

- the carrier is unsupported or malformed;
- a required color-space, domain, domain policy, shaper, or strength declaration is missing or invalid;
- the LUT file is missing or its hash differs;
- the input or output space is unsupported;
- the input exceeds the domain without a shaper or explicit clamp while the LUT is applied;
- an effect value is nonfinite or outside its declared domain.

A failed LUT import or render MUST preserve the preceding accepted image and parameters.

## Non-goals

- Physical film simulation, spectral sensitivity, metamerism, dye density, or illuminant adaptation.
- Scene atmosphere, fog, motion blur, temporal integration, or volumetric scene glow.
- Local masks, subject detection, or content-aware adjustments.
- Adobe XMP or DCP import.
- Converting a film profile into a LUT transform.
- Baking grain, halation, diffusion, or grade into a LUT.
- Treating a LUT as a film stock or a measured material.

## Acceptance

1. LUT mode applies a declared external LUT without loading film or print profiles; Off mode applies the same effects without a LUT or LUT Grade; mode switching preserves inactive values.
2. Diffusion, Halation, LUT Grade, and Grain remain independently toggleable in the fixed order; LUT strength zero bypasses the LUT color transform and no spatial effect is encoded in the LUT.
3. Missing or ambiguous color-space, domain, domain policy, shaper, file, or hash declarations fail before output without silent substitution, clipping, or fallback.
4. A clamp render outside the declared domain clips only at the domain edge and reports the clipped fraction; fail stops the render instead. The LUT stage shows the full clipped warning and every domain axis.
