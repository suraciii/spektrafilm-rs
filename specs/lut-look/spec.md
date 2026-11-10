# LUT look

## Authority

| Capability | Owns |
| --- | --- |
| This capability | The non-spectral LUT look, its render-mode boundary, external LUT color management, and independent effect layers |
| [Film chain](../film-chain/spec.md) | Spectral film and print simulation |
| [LUT delivery](../lut-delivery/spec.md) | Exporting the film chain |
| [Tone and color](../finishing/spec.md) | Grading |
| [Film grain](../film-grain/spec.md) | Film grain engines |
| [Presets](../presets/spec.md) | Film look presets; this contract does not add a LUT look preset |

## Scope

A **LUT look** is an external 3D color transform plus independent effect layers. It is non-spectral and operates on RGB. It does not model film sensitivity, record density, development, printing, scanning, or illumination.

A **LUT mode** renders a LUT look. A **film mode** renders the existing spectral film chain. Mode selection is explicit and exclusive.

| Property | Film mode | LUT mode |
| --- | --- | --- |
| Color source | Spectral film and print simulation | External 3D LUT |
| Material profiles | Required by the route | Not consumed |
| Illuminant adaptation | Spectral | None |
| Grain | Film-density or post-scan engine | Independent appearance layer |
| Halation | Film exposure and density | Independent highlight layer |
| Diffuse optics | Camera or enlarger stage | Pre-LUT effect |
| Spectral claims | Governed by film chain | None |

LUT mode MUST NOT be described as a film stock, a measured film response, or a spectral simulation. A LUT MAY encode a look derived from film, but the LUT look itself is a colorimetric transform.

LUT mode MUST NOT add scene atmosphere, fog, motion integration, volumetric scene glow, local masks, or subject-aware adjustments. Those are outside this contract.

## LUT input

A LUT look MUST use a 3D LUT and MAY declare a 1D shaper before it. The supported carrier is `.cube`.

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

The domain policy is either fail on out-of-domain input or an explicit clamp. The import UI MUST preselect fail, and the declaration MUST carry the selected policy.

A missing, ambiguous, or unsupported color-space declaration MUST fail before output. The product MUST NOT infer a color space from a filename, a film profile, or image content.

A LUT whose domain does not cover the input MUST use a declared shaper, select an explicit clamp, or fail before output while the LUT is applied. Silent clipping is forbidden. An explicit clamp MUST report that clipping occurred.

LUT strength is a finite value from zero to one. Zero MUST bypass the LUT color transform exactly; it MUST NOT bypass active effects. One MUST apply the declared transform without mixing.

The LUT file is an external asset. The product MUST NOT silently substitute another file when the declared file is missing or its hash differs.

## Color order

LUT mode MUST use one fixed evaluation order:

```text
input
  → linear working RGB
  → diffusion
  → halation
  → shaper / LUT input transform
  → 3D LUT
  → optional grade
  → output gamut
  → output encoding
  → grain
```

- Diffusion MUST evaluate in linear light before halation.
- Halation MUST evaluate in linear light before the LUT.
- The LUT MUST remain pointwise.
- Grade MUST follow the LUT.
- Output gamut MUST follow the LUT or grade.
- Grain MUST follow output encoding.

The order MUST NOT be user-reorderable. An inactive effect MUST be an exact bypass.

## Effects

Every effect is optional and independent. Enabling or disabling one effect MUST NOT change another effect's stored values or the LUT declaration.

An effect MUST NOT be encoded in the LUT. A 3D LUT is pointwise and cannot carry a spatial radius, a grain field, or a highlight mask.

### Halation

Halation MUST use a linear-light highlight mask and a declared spatial response. It MUST NOT read film density, film exposure, or a film profile.

The effect MUST expose a luminance threshold, a spatial radius, an intensity, a tint, and a highlight-protection range. Its tint is an appearance choice, not a measured stock response.

### Diffusion

Diffusion MUST provide the existing optical diffusion responses where their declared response is applicable. It MUST run in linear light before the LUT.

A LUT look that already contains lens character MUST allow diffusion to remain off. The product MUST NOT infer that a LUT contains diffusion.

### Grade

Grade MUST use the existing [Tone and color](../finishing/spec.md) response. It MUST remain optional and MUST NOT become part of the LUT.

The LUT look MUST preserve grade values when grade is disabled. Re-enabling grade MUST restore the same accepted response.

### Grain

Grain MUST be an output-referred appearance layer. It MUST NOT use a film grain engine, film RMS granularity, or a film profile.

The effect MUST expose an amount, a size, separate shadow, midtone, and highlight weights, an optional chroma component, and a seed. The default grain is monochrome.

A seed MUST make a realization repeatable for the same input, parameters, and implementation. Grain MUST NOT alter the LUT color transform or the grade definition.

## Comparison and preview

Preview, Scan, and Export MUST use the same LUT declaration, color order, and effect definitions. A reduced preview MUST NOT claim full-resolution equivalence for halation, diffusion, or grain.

The product MUST provide a neutral gray ramp and a color-checker preview for an imported LUT. The preview MUST NOT auto-fit exposure, white balance, or contrast to make the LUT appear correct.

## UI and CLI

The product MUST expose an explicit Film / LUT mode selection. Switching mode MUST preserve stored values in the inactive mode without executing them.

LUT mode MUST expose LUT import, the declared input and output spaces, the domain, the domain policy, the shaper, LUT strength, and independent halation, diffusion, grade, and grain controls.

A newly imported LUT MUST default to strength one with every effect disabled.

The UI MUST label the feature as a non-spectral LUT look. It MUST NOT present a LUT as a film profile or a measured stock.

The CLI MUST expose the same mode, LUT declaration, and effect controls through its canonical parameter language. Exact flags and option names belong to the [CLI contract](../cli/spec.md).

## Persistence

LUT look settings are runtime parameters. Complete parameter documents, GUI state, and render recipes MUST retain the LUT reference, content hash, color spaces, domain, domain policy, shaper, strength, effects, and grade.

A LUT look is not a film look preset. Preset-library support for LUT looks is outside this contract.

A saved LUT reference MUST fail validation while LUT mode is active when the file is missing or its content hash differs. The product MUST NOT replace the reference with another LUT or fall back to film mode.

Switching to film mode MUST preserve the stored LUT look. Switching back MUST restore it without reinterpreting the film controls.

## Failure

A LUT look MUST fail before publishing output when any of the following is true:

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
- Converting a film profile into a LUT look.
- Baking grain, halation, diffusion, or grade into a LUT.
- Treating a LUT as a film stock or a measured material.

## Acceptance

1. LUT mode applies a declared external LUT without loading film or print profiles; film mode remains unchanged and mode switching preserves inactive values.
2. Diffusion, halation, grade, and grain remain independently toggleable in the fixed order; LUT strength zero bypasses the LUT color transform and no spatial effect is encoded in the LUT.
3. Missing or ambiguous color-space, domain, domain policy, shaper, file, or hash declarations fail before output without silent substitution, clipping, or fallback.
