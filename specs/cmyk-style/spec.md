# CMYK ink style

## Product model

`CMYK ink style` is an optional modeled appearance for an RGB output. It is not a film stock, a photographic paper, a photographic printing operation, or a base workflow. The `input` workflow treats its source as already rendered RGB and applies input-to-output color management.

The first style is the fixed `ink` style. It is a self-authored RGB appearance model informed by the CMYK research. It is not an ICC soft proof, a press simulation, a press certification, or a reproduction guarantee for a historical publication.

The product MUST describe this feature as a style simulation. It MUST NOT claim that the output is colorimetrically accurate for a named press condition.

## Enablement

- The GUI MUST expose a `CMYK ink style` checkbox next to the base workflow controls. It MUST be off in the factory state.
- Enabling or disabling CMYK ink style MUST NOT change the base workflow, film stock, photographic paper, scan polarity, or source exposure.
- The existing `input` workflow MUST perform source-to-output color management without film simulation, photographic printing, metering, or new film grain.
- CMYK ink style MUST accept the completed RGB output of every supported base workflow. It MUST NOT require a photographic print stage.
- The GUI MUST show the resulting base workflow and enabled finishing treatment together before rendering.
- A single `Strength` control MUST use the range 0 to 1. Its enabled default MUST be 1.
- Disabling the treatment MUST retain the selected Strength.
- Presets and saved GUI state MUST retain the base workflow, enablement, and Strength independently. Preview, Scan, and Export MUST use the same effective finishing settings.
- The retired route `input > film > scan > magazine` MUST NOT remain a selectable base workflow. A versioned document migration MAY replace it with `input > film > scan` and enable `cmyk`.
- A current-schema document or direct current-schema route using the retired route MUST fail with an actionable migration error.

## Color contract

- The feature MUST accept output-referred RGB with explicit primaries, white point, and transfer state. Linear encoding MUST NOT imply scene-referred exposure.
- The feature MUST run once after the base workflow and its scanner-domain effects, before output publication:

  `Scanning → output gamut processing → unsharp masking → transfer encoding → Procedural grain → CMYK ink style → output mix → writer`

- Existing material controls run at their eligible exposure and development stages before scanning. They MUST NOT alter CMYK ink style's definition, Strength, or enablement.
- Overall output mixing follows CMYK ink style in common linear output RGB. Material processing and CMYK ink style MUST each run once.
- The `input` workflow runs CMYK ink style without film or print development.
- The feature MUST preserve image dimensions and spatial detail positions. It MUST NOT re-run exposure, film development, or grain generation.
- All pixels MUST use the same color rule. The feature MUST NOT detect faces or use manually selected image regions.
- The low-chroma behavior MUST NOT be described as guaranteed skin protection.
- Strength MUST blend the base image and the full-strength `ink` appearance in one explicitly defined common linear RGB space. Disabled processing and Strength 0 MUST return the base result exactly.
- The feature MUST preserve the caller's output primaries and transfer state. It MUST NOT apply a second transfer encoding.
- Output MUST remain three-channel RGB. The feature MUST NOT produce CMYK files, CMYK separations, plates, press-ready files, physical proofs, or ICC press-workflow output.
- CMYK ink style MUST NOT add halftone grids, plate registration, dot gain, paper texture, gloss, GCR, UCR, TAC controls, or stochastic noise.

## `ink` style definition

The `ink` style is a deterministic point transform. Its public runtime controls are only enablement and Strength. The following constants are part of the `cmyk_ink_v1` model and are not editable runtime fields:

| Constant | Value | Meaning |
|---|---:|---|
| `grade.k_l` | 0.28 | Chroma-dependent lightness reduction |
| `grade.k_c` | 1.40 | Chroma scale |
| `grade.contrast` | 1.03 | Lightness contrast |
| `grade.smoothstep` | `(8, 45)` | Shadow-protection interval in L* |
| `grade.toe` | none | No toe lift or black-point compensation |

For each pixel, the implementation MUST:

1. Decode the declared output transfer function into linear output RGB.
2. Convert linear output RGB into CIELAB D50 through the declared output color space and a declared chromatic-adaptation transform.
3. Let `L` be L*, `a` and `b` be the Lab opponent coordinates, and `C = hypot(a, b)`.
4. Compute `s = smoothstep((L - 8) / (45 - 8))`, where `smoothstep(x) = t*t*(3 - 2*t)` and `t = clamp(x, 0, 1)`.
5. Compute `L' = max(4, 50 + (L - 50) * 1.03 - 0.28 * C * s)`.
6. Compute `a' = 1.40 * a` and `b' = 1.40 * b`.
7. Convert `[L', a', b']` from CIELAB D50 back to the declared output color space's linear RGB.
8. Blend the full-strength result with the base linear RGB using Strength.
9. Restore the caller's original transfer state exactly once.

The implementation MUST define finite handling and floating headroom at the appearance boundary. Values outside the display interval MUST NOT be silently treated as an input-loader clipping rule. The implementation MUST preserve the existing writer policy for final clipping and integer encoding.

The research paper/ink model is not part of `cmyk_ink_v1` runtime behavior. `paper_gain`, `warm`, `ink_gamma`, `ink_density`, `tac`, `chroma_compress`, YNSN parameters, FOGRA characterization data, and related CMYK coverage values MAY remain offline research inputs. They MUST NOT appear as runtime fields or product claims until a separate asset, licensing, and model specification is approved.

## Print structure boundary

Halftone, GCR, UCR, dot gain, registration, paper texture, and other spatial print effects are separate future features. They MUST NOT be implemented as part of `cmyk_ink_v1` and MUST NOT be baked into a pointwise LUT. Any future spatial print effect requires its own scale, resolution, seed, CPU/GPU, Preview/Scan/Export, and LUT-delivery contract.

## State, CLI, and migration

The implementation MUST bump every complete-document schema that carries runtime parameters at its existing owner boundary before emitting `cmyk`. The parameter-document and RenderRecipe schema MUST advance from `spektra-rs-params-4`. GUI Rust-state extension MUST advance from its current version, and preset schema MUST advance from version 4.

LUT bundle metadata MUST advance from its current schema when its role vocabulary or provenance shape changes. The migration tables MUST name the accepted legacy versions and MUST never treat a current-schema document as a legacy carrier.

Telemetry and stage-evidence schema versions remain unchanged unless their serialized field shape changes; their reports MUST carry the canonical `cmyk` configuration name after implementation.

- The canonical runtime group is `cmyk` with `active` and `strength` leaves. New runtime documents, current-schema sparse sources, discovery, diagnostics, telemetry, GUI state, and LUT metadata MUST use `cmyk` terminology.
- The retired `magazine_print_color.active` and `magazine_print_color.strength` fields are legacy fields. They MUST NOT be runtime aliases or current-schema editable paths.
- Versioned complete parameter documents, versioned CLI recipes, and versioned complete GUI state MAY translate the legacy fields to `cmyk.active` and `cmyk.strength`.
- A migration MUST reject a document that supplies both a legacy field and its new counterpart. It MUST leave the source document unchanged on failure.
- A migration MUST be idempotent. A migrated document MUST serialize only the new fields and MUST pass current-schema strict validation.
- Existing documents using the retired route `input > film > scan > magazine` MAY migrate to `input > film > scan` with `cmyk.active = true`. A current-schema document, sparse source, inline assignment, or direct route selector naming the retired route MUST fail and MUST identify the replacement.
- CLI `--set` and discovery MUST expose only `cmyk.active` and `cmyk.strength`. There MUST be no retired route or field alias.
- GUI labels and help MUST use `CMYK ink style`. Technical help MUST state that the output remains RGB and is not a press proof or separation.
- Preset and complete-state migration MUST preserve the base workflow, profile references, unrelated controls, Strength, and enablement. Migration MUST NOT silently alter scan polarity, exposure, film, paper, or output settings.
- A legacy field or route MUST NOT be migrated merely because its spelling is recognized; the carrier MUST first prove that its schema version is an approved legacy version.

## GPU and execution contract

- CPU and GPU MUST implement the same `cmyk_ink_v1` transform, constants, transfer handling, color-space adaptation, stage order, and Strength blend.
- The feature MUST run on the selected backend without a CPU image-processing fallback when the caller explicitly requests GPU.
- An explicit GPU request MUST fail before output publication if the complete selected operation, including CMYK ink style, cannot execute on the device. GUI failure MUST retain the last accepted output.
- Automatic backend selection MAY choose the complete CPU path when no usable GPU is available. It MUST report that selection and reason.
- It MUST NOT silently change the feature or execute only part of the operation on CPU after GPU selection.
- The feature MUST NOT add a fourth spectral stage, a fourth `ImageBuf` channel, a new workflow route, or a second implementation in the CLI or GUI.

## LUT delivery and provenance

- `cmyk_ink_v1` is pointwise and MAY be included in a deterministic RGB→RGB LUT.
- A LUT containing the feature MUST record the base route and the CMYK ink style definition independently.
- LUT provenance MUST identify the style identifier, model revision, Strength, output color space, transfer state, parameter digest, and whether the feature is included in the final RGB stage.
- A LUT MUST NOT claim that it contains halftone, paper texture, press certification, ICC proofing, or physical-print equivalence.
- A segmented LUT consumer or a collapsed complete transform MUST NOT apply CMYK ink style more than once.
- LUT baking MUST NOT silently change an `input` route into a film/print/scan route. The selected base route MUST remain the route represented by the baked transform.

## Telemetry and diagnostics

- Telemetry MUST identify the feature with the canonical `cmyk` stage/configuration name.
- Reports MUST distinguish requested backend, selected backend, actual execution path, and precision. A backend label alone MUST NOT prove that CMYK ink style ran on the GPU.
- An explicit GPU failure, automatic CPU selection, or mixed execution MUST retain its existing failure/reason and publication semantics.
- The feature MUST NOT add image pixels or external asset contents to default telemetry.

## Acceptance

- The default-disabled configuration is an exact identity.
- Active Strength 0 is an exact identity.
- Active Strength 1 matches the frozen `cmyk_ink_v1` reference probes for encoded and linear output states.
- The transform is finite on neutral, chromatic, boundary, negative-headroom, and super-white probes accepted by the existing image contract.
- Neutral input remains neutral within the declared numerical budget, and the declared tone response remains monotone over the accepted display interval.
- The `input` workflow and every supported scan route apply the same terminal RGB transform; the input route has no film or photographic-paper dependence.
- Preview, Scan, Export, CLI process, resident execution, per-stage execution, and LUT evaluation use one definition and apply it at most once.
- CPU f64, native f32, and GPU outputs satisfy budgets owned by the GPU execution contract. An explicit GPU request cannot silently use CPU image arithmetic.
- Legacy field migration is transactional and idempotent. Legacy/new conflicts reject.
- Current-schema legacy paths reject. The retired route is not selectable.
- GUI state, CLI recipes, presets, discovery, diagnostics, telemetry, benchmarks, specifications, and user documentation emit only the canonical new terminology after migration.
- The implementation MUST include visual review on the fixed research image and additional neutral/chromatic photographic probes. Research images are visual evidence, not a claim of physical-print accuracy.

***
