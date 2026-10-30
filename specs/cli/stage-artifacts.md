# Stage input and output

## Status and scope

The [CLI specification](spec.md) owns common command syntax, parameter composition, and Boolean debug diagnostics. This document owns the implemented stage routes and the stage artifact contract. It replaces the earlier stage-export DSL proposal.

## Product contract

- A [simulated material state](../../CONTEXT.md#language) is a completed film or print result with the information needed to interpret it and continue processing. A stage artifact MUST contain exactly one such state and exactly one continuation entry.
- Every successfully published stage artifact MUST be resumable. An observation-only array or a multi-stage research bundle MUST NOT be published as a stage artifact.

A stage artifact is a normal input or output. It MUST NOT depend on debug logging or diagnostics collection. Changing `--debug` MUST NOT change its required content, numerical payload, continuation eligibility, or selected processing model.

- `--route` MUST select the execution path; `--output` MUST publish its terminal result.
- The command MUST NOT add `--save-stage`, `--export-stage`, `--from`, `--until`, or `--resume` options. It MUST NOT introduce a job DSL, a graph language, or an output-selector expression inside a filename.

## Route language

The first node names the input state. Each following node names an operation to execute. The existing single-node `input` route keeps its current image-processing meaning.

The following routes extend the existing finite route vocabulary:

| Route | Input | Output |
| --- | --- | --- |
| `input > film` | Declared raster or camera RAW | Film artifact |
| `input > film > print` | Declared raster or camera RAW | Print artifact |
| `film > scan` | Film artifact | Image |
| `film > print` | Film artifact | Print artifact |
| `film > print > scan` | Film artifact | Image |
| `print > scan` | Print artifact | Image |

- `film` includes film exposure, development, and effects that belong before the completed film boundary. `print` includes print exposure and development, retaining the existing admitted printing-material types rather than meaning paper only.
- `scan` includes the existing eligible scan interpretation, post-scan appearance, and final output processing. It MUST NOT stop before those operations or run them twice.

The numerical film boundary is `cmy_film`. The numerical print boundary is `cmy_print`. These names describe the record-density domains; they do not imply RGB color data or establish that every model can be represented by three density arrays alone.

- Node names are case-sensitive; ASCII whitespace around `>` is insignificant. The route MUST be one shell argument; unknown nodes, empty nodes, repetitions, cycles, and arbitrary node permutations MUST fail.
- Existing image and convert-film routes retain their behavior. No shortened convert-film routes are added.
- Image inputs retain existing route defaults. An artifact input MUST supply `--route` explicitly, with its first node matching the artifact's single state.
- The reader MUST reject a mismatch instead of converting it, selecting another entry, or restarting from an embedded source. A preset MUST NOT choose an artifact continuation route implicitly.
- `--scan-film` remains an image-input shorthand and conflicts with `--route`. Artifact input MUST use the explicit continuation routes.
- Public artifact input or artifact output MUST reject explicit `taps.inject` and `taps.collect`, including those supplied through a parameter carrier. These diagnostic controls MUST NOT become a second authority for the artifact boundary.

## Input declarations and profile requirements

- Raster and RAW declarations retain their existing rules. Artifact input MUST obtain domain, units, material identity, geometry, and precision from the artifact.
- Artifact input MUST NOT require raster color declarations or RAW loading controls. Explicit raster declarations and RAW loading controls on artifact input MUST fail as inapplicable, including compatible-looking values.
- An image-to-film route MUST resolve its film profile but MUST NOT require a print profile only to prepare a stage it will not execute. An image-to-print route MUST resolve both materials.
- A film artifact MUST supply the frozen film profile content. A print artifact MUST supply the frozen print material and any upstream context still needed by its admitted continuation.

A new print operation MUST resolve its material from the explicit selection, a retained downstream selection, or the retained film's target-print selection under the existing profile rules. A missing required downstream material MUST fail before execution. An unused future profile name MUST NOT make an otherwise self-contained film state unloadable.

- Explicit profile references on artifact input MAY restate the frozen material only when the resolved content identity is identical. The same name with different content MUST fail.
- Downstream material replacement remains allowed. It MUST use that material's existing initialization and calibration rules instead of keeping prepared values from the replaced material.
- Film-scan eligibility MUST derive from the selected medium and its origin. `film > scan` on a simulated negative MUST permit the existing negative-to-positive interpretation and retain the frozen camera compensation needed by that interpretation.
- A print artifact MUST NOT acquire film-negative scan eligibility. Existing direct-image and convert-film polarity rules remain unchanged.

## Artifact carrier and required content

- An artifact MUST be a self-contained directory with a UTF-8 JSON root manifest named `stage.json`. `.sfstage` is the recommended directory suffix, but the suffix alone MUST NOT establish validity or select a stage.
- This contract adds no archive encoding, stdin stream, or stdout stream for artifacts. `-` MUST NOT be interpreted as a supported artifact stream.

The manifest MUST identify `schema_version: 1`, `kind: stage_artifact`, a single `state` (`film` or `print`), its matching numerical boundary, and a versioned state representation. It MUST NOT contain a list of selectable continuation entries or a capability that downgrades the artifact to observation-only data.

The manifest and its referenced members MUST carry the following information:

| Information | Required semantics |
| --- | --- |
| State and representation | One completed material boundary and the versioned numerical representation that the reader validates. |
| Native payload | The actual boundary values and every model-owned state component required to continue. |
| Layout | Width, height, channel count and order, scalar type, byte order, native domain, units, and exact member byte lengths. |
| Material interpretation | Embedded profile contents and all referenced material data needed to interpret the frozen state, with content identities. Names or local file paths alone are insufficient. |
| Geometry | Current raster grid, physical scale, original frame relationship, completed geometry, and any pending geometry required by the continuation. |
| Interpretation context | Actual upstream exposure and prepared state that downstream operations still consume, including negative-conversion context and applicable print normalization. |
| Continuation configuration | Frozen-state values separated from editable downstream baseline values using the existing canonical parameter schema. |
| Randomness | Producing seed and any additional model-owned state required for deterministic continuation. A seed alone MUST NOT be assumed sufficient. |
| Provenance | Producer version, numerical-model version, actual backend and computation precision, producing route, effective parameters, and input identity. Provenance MUST distinguish newly executed from inherited operations. |
| Integrity | All required member paths, byte lengths, and content hashes. |

- Native numeric members MUST preserve their scalar values without clipping, transfer encoding, color conversion, or channel permutation. Density data MUST declare record density and optical-density units, not an RGB color space.
- If a density representation uses three chromogenic records, its axis order MUST be CMY. An admitted monochrome representation MUST retain its owning material's channel semantics rather than invent a color interpretation.
- Scalar members MUST declare `f32` or `f64` storage and little-endian byte order. A raster array MUST declare its row-major, channel-interleaved layout.
- Storage precision MUST NOT be presented as computation precision. Widening a stored f32 value MUST NOT turn it into an f64-reference computation claim.
- The producer MUST preserve the state required by the admitted model, not export only an observed array that loses a latent field, pending optical integration, stochastic realization, or geometry.
- If the model cannot represent a complete state at the requested boundary, the command MUST fail with the model and boundary reason. It MUST NOT disable an effect or substitute another model to make export possible.

The artifact MUST contain the dependency closure needed to interpret the saved state. Moving it away from the original photograph and mutable profile library MUST NOT change that interpretation. It need not contain the original photograph, every intermediate array, unrelated profiles, compiled shaders, GPU memory, recomputable caches, execution logs, or timing reports.

- A prepared-source branch is a conditional dependency. If retained final-output mixing requires that branch, the producer MUST retain the correctly aligned branch and its color declaration, or reject the artifact request.
- An artifact that does not retain it MUST reject a later request for `output_mix.amount < 1`. A density array MUST never be used as a substitute source image.
- Core MUST own a deterministic validator for the envelope and each supported state representation. The validator MUST check the complete required state, not only manifest fields or a producer's claim of support.
- Unknown fields, duplicate keys, unknown schema or representation versions, non-finite required state, missing dependencies, inconsistent shapes, truncated data, and digest mismatches MUST fail. Checked arithmetic MUST validate dimensions and lengths before allocation.
- Member paths MUST be relative to the artifact root. Absolute paths, parent traversal, and symlink escapes MUST fail.
- A reader MUST NOT resolve a frozen dependency against an external same-named profile after a missing member or hash failure. A changed numerical model MUST either explicitly support the stored representation or reject it; it MUST NOT silently reinterpret the state.
- `common_finishing_v1` MUST admit the unchanged `lab_scan_operator_v6` material representation. Missing finishing groups in each legacy baseline or lineage record MUST become neutral without changing material values or original provenance.
- Unsupported runtime-model identities MUST fail.

This is a generated persistence format, not a new user-authored configuration language. The implementation MUST supply its typed schema and representation validators with the reader and writer. No runtime MAY accept a placeholder or opaque unvalidated context object as a substitute for required model state.

## Frozen state and editable continuation

The artifact MUST provide the baseline for continuation. Existing preset and `--set` source ordering MUST apply to downstream controls. The resolver MUST validate all explicitly supplied source syntax and values before composition, as for image input.

- A completed-state control or interpretation dependency MUST be immutable; an explicit semantically equal restatement is permitted. A different value MUST fail with the canonical path, the source of the conflicting edit, and guidance to regenerate from the earlier input.
- This rule also applies to profile changes, legacy parameter files, and upstream edits carried by complete look presets. A later source MUST NOT make an illegal frozen-state mutation acceptable by overriding it again.

The resolver MUST NOT replay a complete saved parameter snapshot through input preparation and stock initialization. It MUST restore validated completed state and prepare only remaining operations. Frozen interpretation data, recorded provenance, and editable downstream defaults MUST remain distinct even when they refer to the same parameter vocabulary.

- Applicability MUST follow the executed operation and the owning model, not a parameter prefix or GUI section. Procedural grain remains post-scan appearance even though it is configured under `film_render.grain`.
- [Common finishing](../finishing/spec.md#scope-and-applicability) remains editable downstream of a saved material. It MUST execute only on an eligible positive continuation and MUST NOT alter the frozen material or its interpretation dependencies.
- Already formed material grain MUST NOT be generated again. Camera context needed by negative conversion MUST remain available without applying camera exposure again.
- Omitted `--seed` MUST inherit the artifact's seed. An explicit seed MUST affect only the remaining random operations and MUST NOT replace an already stored realization.
- Image input keeps the default seed of 0. Existing repeated-iteration rules apply to the selected seed.
- Already completed orientation, crop, resize, and physical scaling MUST NOT be applied again. Pending operations MUST execute exactly once under the saved geometry contract.
- Requests that would reinterpret frozen geometry MUST fail. An independently supported downstream geometry operation MUST remain downstream rather than modifying the stored state.
- A consumer MUST report any admitted storage-to-computation precision conversion. It MUST reject a conversion that the owning model does not support.
- Cross-precision, cross-backend, and cross-model identity MUST NOT be promised. Equality acceptance MUST use the same supported precision, backend, model, parameters, and random realization.

## Output and publication

- A route ending at `film` or `print` MUST select the artifact writer. A route ending at `scan` MUST select the existing image writer.
- The output path MUST NOT determine which operations run. Normal execution MUST require one `--output`; dry run MAY omit it.

Artifact output MUST reject image writer flags such as `--format`, `--bit-depth`, compression, JPEG options, and saving color or transfer conversion. Retained scanner and output parameters MAY remain editable downstream defaults; they MUST NOT cause the density payload to undergo an image-saving conversion.

- Artifact output MUST NOT require an extra image destination. It MUST NOT create a preview image, debug directory, or sibling raw dump unless an independent existing output option explicitly requested that data.
- `--raw-out` keeps its numerical-dump role and does not become an artifact-input format. It MUST NOT advertise continuation capability.
- Artifact destinations MUST NOT already exist; no artifact overwrite option is introduced. The destination and its staging paths MUST NOT alias inputs, profile/configuration sources, raw dumps, or diagnostics.
- Input artifacts MUST be read-only. A conflicting or invalid destination MUST fail before rendering when it can be determined during validation.
- The writer MUST prepare all members in a sibling staging directory and finalize the manifest after members and hashes are complete. It MUST publish the completed directory without replacing an existing destination.
- A partial, failed, or cancelled write MUST NOT leave a valid artifact at the requested path. Failure MUST return nonzero and preserve existing user files.
- A stale or incomplete staging directory MUST NOT be recognized as a published artifact. The reader MUST require the completed manifest and validate its members before use.
- `--iters` MUST publish only the final successful iteration supplying the result. Neither publication nor diagnostics MAY trigger a second render.
- Normal `process` stdout remains empty. Progress and errors belong on stderr.
- Multiple explicitly requested destinations, such as a raw dump and the primary artifact, are not a single filesystem transaction. Failure MUST report any already published output honestly without claiming cross-destination atomicity.

## Dry run

Dry run MUST resolve the same input class, route, frozen-state constraints, model capability, profile dependencies, and output writer as execution. It MUST NOT decode numerical payloads, render, initialize a compute device, or create any output.

- For artifact input, dry run MAY read the manifest and small embedded configuration/profile records. It MUST identify unchecked large-payload values and hashes.
- Normal execution MUST verify every required payload before using it. Dry-run validity MUST NOT be presented as proof of data integrity or device availability.
- The existing JSON report MUST retain its current fields. `input.kind` MUST additionally admit `stage_artifact`; for that kind, `input.declaration` and `input.raw_loading` MUST be null.
- An `input.artifact` record MUST report the state, boundary, schema and representation versions, producer precision, and embedded material identities. Raster and RAW reports MUST NOT acquire a fabricated artifact record.

The report MUST show `execution` with the input state, already completed operations, and planned operations. It MUST show seed value and whether it comes from the artifact, explicit argument, or image-input default. Frozen-state conflicts are validation errors, not inactive-edit warnings.

- For artifact output, `output.kind` MUST be `stage_artifact` and `output.format` MUST be `sfstage`, even if the path is omitted. `output.path` MUST then be null; image-only writer fields MUST be null.
- For image output, `output.kind` MUST be `image` and existing format-resolution behavior remains unchanged. A route with no required print material MUST report `print_profile: null`, not resolve an unused stock.
- Artifact dry runs MUST add `artifact_payload` and `artifact_integrity` to unchecked work when those checks were not performed. Compute-device and execution-dependent outcomes remain unchecked under the common dry-run contract.
- JSON and text MUST describe the same decisions. `--debug` MAY add stderr detail but MUST NOT change either report's stdout or decisions.

## Removed interfaces and research boundary

All active parsers and help MUST remove `--stage-evidence`. There MUST be no deprecated forwarding option, hidden alias, automatic rewrite, or debug-driven replacement. The abandoned `--save-stage`, `--export-stage`, `--debug DIRECTORY`, and multi-entry `.sfstage` designs MUST NOT be implemented.

Public `process` no longer provides simultaneous all-stage side capture. This is an intentional capability removal, not a claim that debug logs or separate partial renders provide equivalent evidence. Ordinary instrument suite and single-image runs MUST NOT gain implicit full capture to compensate.

- Dedicated instrument measurement and experiment operations retain their explicit numerical-observation purpose under the [native observation contract](../instrument/native-observations.md). Research observation data is not a stage artifact and MUST NOT be accepted as one.
- Its existing readers remain research readers, not aliases for artifact continuation. The experiment export input uses a positional `INPUT`, not the removed option.

Removing the process option MUST NOT delete the core observation machinery still required by those research operations. They MUST use the shared execution path with an explicit instrument-owned observation request, not shell out through a retired option. No research caller MAY relabel several executions as one captured run.

Existing image-only RenderRecipe, GUI child-process, preset, LUT, and telemetry contracts remain unchanged unless their owning specification explicitly adopts stage artifacts. Those callers MUST NOT acquire new mandatory flags or stage persistence implicitly.

## Syntax examples

These POSIX examples define target syntax. Fixtures MUST be supplied by the implementation acceptance runner. They are not demonstrations that the current executable supports artifact processing.

Generate one film state without a print profile:

```bash
spektrafilm process input.png --backend cpu \
  --film kodak_portra_400 \
  --input-color-space sRGB --input-cctf-decoding true \
  --set film_render.grain.active=false \
  --route 'input > film' -o negative.sfstage
```

Continue through print and scan:

```bash
spektrafilm process negative.sfstage --backend cpu \
  --route 'film > print > scan' --paper kodak_portra_endura \
  -o final.tif
```

Scan the same negative directly and redirect detailed diagnostics with the shell:

```bash
spektrafilm process negative.sfstage --backend cpu \
  --route 'film > scan' --scan-output negative_to_positive \
  -o direct.tif --debug 2>debug.log
```

Save print separately, then scan it:

```bash
spektrafilm process negative.sfstage --backend cpu \
  --route 'film > print' --paper kodak_portra_endura \
  -o print.sfstage

spektrafilm process print.sfstage --backend cpu \
  --route 'print > scan' --set scanner.lens_blur=0.5 \
  -o print-scan.tif --diagnostics report.json
```

Inspect continuation without rendering or creating output:

```bash
spektrafilm process negative.sfstage --backend cpu \
  --route 'film > scan' --dry-run --debug
```

## Acceptance

1. Parse every example through both real CLI executables. Reject route/type mismatches, missing artifact routes, invalid node paths, image writer options on artifact output, explicit source declarations on artifact input, diagnostic tap conflicts, and the retired option without publishing output.
2. Exercise all six added routes through the real CLI. Prove that film-only processing does not require or execute printing and that continuation does not execute upstream exposure, development, material grain, or geometry again.
3. Compare one-pass output with film-split and print-split output under the same model, seed, precision, backend, and effective controls; establish exact identity for the deterministic same-path CPU reference fixture. For model/backend cases without bit identity, use the owning predeclared numerical budget and report measured errors; do not select a tolerance after observing the result.
4. Exercise active material grain, post-scan Procedural grain, non-square geometry, negative conversion's camera context, print material replacement, and prepared-source mixing; verify each owning model's state completeness. Unsupported configurations MUST reject rather than disable effects or publish observation-only states.
5. Move artifacts away from original images and profile libraries; modify a same-named external profile and verify that stored interpretation is unchanged. Verify immutable profile conflicts, downstream overrides, seed inheritance, and the separation between provenance and executable defaults.
6. Reject unknown versions, missing state, malformed layouts, non-finite required values, unsafe paths, hash mismatches, and truncated payloads before simulation. Exercise both f32 and f64 storage and admitted precision conversions without mislabeling computation precision.
7. Force cancellation, write failure, destination races, and path aliasing. Verify no valid-looking partial primary artifact, preservation of existing files, no second render, and nonzero status on failure.
8. Compare debug-on/off runs: primary payloads and numerical results MUST agree, stdout and structured dry-run results MUST remain unchanged, and no additional files MAY appear without an explicit file-output request or shell redirection. Exercise debug with diagnostics both absent and present.
9. Verify dry-run input classes, output kinds, unused print profile handling, frozen-conflict errors, and unchecked integrity/device outcomes. It MUST NOT modify sentinels or create staging directories.
10. Exercise dedicated measurement and experiment consumers after the retired flag is removed; preserve their native numeric evidence and reference-protocol checks. Ordinary suite/image runs MUST report absent observation data without reconstructing it.

Acceptance MUST report executable identity, model/version, input provenance, complete invocations, status, stdout/stderr, artifact checks, and measured numerical comparisons. Documentation checks, shell parsing, and compilation alone are not processing acceptance.
