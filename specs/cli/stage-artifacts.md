# Stage input and output

## Status and scope

This is the approved target contract for stage input and output on `process`. The [CLI specification](spec.md) owns common command syntax, parameter composition, and Boolean debug diagnostics. This document owns the added routes and the stage artifact contract. It replaces the earlier stage-export DSL proposal.

The implementation does not yet provide this contract. It still exposes the retired stage-evidence option. Specification examples describe the target language, not current executable support. The implementation must deliver the validation and real-entry-point acceptance below before this capability is presented as available.

## Product contract

A [simulated material state](../../CONTEXT.md#language) is a completed film or print result with the information needed to interpret it and continue processing. A stage artifact must contain exactly one such state and exactly one continuation entry. Every successfully published stage artifact must be resumable. An observation-only array or a multi-stage research bundle must not be published as a stage artifact.

A stage artifact is a normal input or output. It must not depend on debug logging or diagnostics collection. Changing `--debug` must not change its required content, numerical payload, continuation eligibility, or selected processing model.

`--route` must select the execution path. `--output` must publish its terminal result. The command must not add `--save-stage`, `--export-stage`, `--from`, `--until`, or `--resume` options. It must not introduce a job DSL, a graph language, or an output-selector expression inside a filename.

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

`film` includes film exposure, development, and effects that belong before the completed film boundary. `print` includes print exposure and development. It retains the existing admitted printing-material types; it does not mean paper only. `scan` includes the existing eligible scan interpretation, post-scan appearance, and final output processing. It must not stop before those operations or run them twice.

The numerical film boundary is `cmy_film`. The numerical print boundary is `cmy_print`. These names describe the record-density domains; they do not imply RGB color data or establish that every model can be represented by three density arrays alone.

Node names are case-sensitive. ASCII whitespace around `>` is insignificant. The route must be one shell argument. Unknown nodes, empty nodes, repetitions, cycles, and arbitrary node permutations must fail. Existing image and convert-film routes retain their behavior. No shortened convert-film routes are added.

Image inputs retain existing route defaults. An artifact input must supply `--route` explicitly. Its first node must match the artifact's single state. The reader must reject a mismatch instead of converting it, selecting another entry, or restarting from an embedded source. A preset must not choose an artifact continuation route implicitly.

`--scan-film` remains an image-input shorthand and conflicts with `--route`. Artifact input must use the explicit continuation routes. Public artifact input or artifact output must reject explicit `taps.inject` and `taps.collect`, including those supplied through a parameter carrier. These diagnostic controls must not become a second authority for the artifact boundary.

## Input declarations and profile requirements

Raster and RAW declarations retain their existing rules. Artifact input must obtain domain, units, material identity, geometry, and precision from the artifact. It must not require raster color declarations or RAW loading controls. Explicit raster declarations and RAW loading controls on artifact input must fail as inapplicable, including compatible-looking values.

An image-to-film route must resolve its film profile but must not require a print profile only to prepare a stage it will not execute. An image-to-print route must resolve both materials. A film artifact must supply the frozen film profile content. A print artifact must supply the frozen print material and any upstream context still needed by its admitted continuation.

A new print operation must resolve its material from the explicit selection, a retained downstream selection, or the retained film's target-print selection under the existing profile rules. A missing required downstream material must fail before execution. An unused future profile name must not make an otherwise self-contained film state unloadable.

Explicit profile references on artifact input may restate the frozen material only when the resolved content identity is identical. The same name with different content must fail. Downstream material replacement remains allowed. It must use that material's existing initialization and calibration rules instead of keeping prepared values from the replaced material.

Film-scan eligibility must derive from the selected medium and its origin. `film > scan` on a simulated negative must permit the existing negative-to-positive interpretation and retain the frozen camera compensation needed by that interpretation. A print artifact must not acquire film-negative scan eligibility. Existing direct-image and convert-film polarity rules remain unchanged.

## Artifact carrier and required content

An artifact must be a self-contained directory with a UTF-8 JSON root manifest named `stage.json`. `.sfstage` is the recommended directory suffix. The suffix alone must not establish validity or select a stage. This contract adds no archive encoding, stdin stream, or stdout stream for artifacts. `-` must not be interpreted as a supported artifact stream.

The manifest must identify `schema_version: 1`, `kind: stage_artifact`, a single `state` (`film` or `print`), its matching numerical boundary, and a versioned state representation. It must not contain a list of selectable continuation entries or a capability that downgrades the artifact to observation-only data.

The manifest and its referenced members must carry the following information:

| Information | Required semantics |
| --- | --- |
| State and representation | One completed material boundary and the versioned numerical representation that the reader validates. |
| Native payload | The actual boundary values and every model-owned state component required to continue. |
| Layout | Width, height, channel count and order, scalar type, byte order, native domain, units, and exact member byte lengths. |
| Material interpretation | Embedded profile contents and all referenced material data needed to interpret the frozen state, with content identities. Names or local file paths alone are insufficient. |
| Geometry | Current raster grid, physical scale, original frame relationship, completed geometry, and any pending geometry required by the continuation. |
| Interpretation context | Actual upstream exposure and prepared state that downstream operations still consume, including negative-conversion context and applicable print normalization. |
| Continuation configuration | Frozen-state values separated from editable downstream baseline values using the existing canonical parameter schema. |
| Randomness | Producing seed and any additional model-owned state required for deterministic continuation. A seed alone must not be assumed sufficient. |
| Provenance | Producer version, numerical-model version, actual backend and computation precision, producing route, effective parameters, and input identity. Provenance must distinguish newly executed from inherited operations. |
| Integrity | All required member paths, byte lengths, and content hashes. |

Native numeric members must preserve their scalar values without clipping, transfer encoding, color conversion, or channel permutation. Density data must declare record density and optical-density units, not an RGB color space. If a density representation uses three chromogenic records, its axis order must be CMY. An admitted monochrome representation must retain its owning material's channel semantics rather than invent a color interpretation.

Scalar members must declare `f32` or `f64` storage and little-endian byte order. A raster array must declare its row-major, channel-interleaved layout. Storage precision must not be presented as computation precision. Widening a stored f32 value must not turn it into an f64-reference computation claim.

The producer must preserve the state required by the admitted model, not export only an observed array that loses a latent field, pending optical integration, stochastic realization, or geometry. If the model cannot represent a complete state at the requested boundary, the command must fail with the model and boundary reason. It must not disable an effect or substitute another model to make export possible.

The artifact must contain the dependency closure needed to interpret the saved state. Moving it away from the original photograph and mutable profile library must not change that interpretation. It need not contain the original photograph, every intermediate array, unrelated profiles, compiled shaders, GPU memory, recomputable caches, execution logs, or timing reports.

A prepared-source branch is a conditional dependency. If retained final-output mixing requires that branch, the producer must retain the correctly aligned branch and its color declaration, or reject the artifact request. An artifact that does not retain it must reject a later request for `output_mix.amount < 1`. A density array must never be used as a substitute source image.

Core must own a deterministic validator for the envelope and each supported state representation. The validator must check the complete required state, not only manifest fields or a producer's claim of support. Unknown fields, duplicate keys, unknown schema or representation versions, non-finite required state, missing dependencies, inconsistent shapes, truncated data, and digest mismatches must fail. Checked arithmetic must validate dimensions and lengths before allocation.

Member paths must be relative to the artifact root. Absolute paths, parent traversal, and symlink escapes must fail. A reader must not resolve a frozen dependency against an external same-named profile after a missing member or hash failure. A changed numerical model must either explicitly support the stored representation or reject it; it must not silently reinterpret the state.

This is a generated persistence format, not a new user-authored configuration language. The implementation must supply its typed schema and representation validators with the reader and writer. No runtime may accept a placeholder or opaque unvalidated context object as a substitute for required model state.

## Frozen state and editable continuation

The artifact must provide the baseline for continuation. Existing preset and `--set` source ordering must apply to downstream controls. The resolver must validate all explicitly supplied source syntax and values before composition, as for image input.

A completed-state control or interpretation dependency must be immutable. An explicit semantically equal restatement is permitted. A different value must fail with the canonical path, the source of the conflicting edit, and guidance to regenerate from the earlier input. This rule also applies to profile changes, legacy parameter files, and upstream edits carried by complete look presets. A later source must not make an illegal frozen-state mutation acceptable by overriding it again.

The resolver must not replay a complete saved parameter snapshot through input preparation and stock initialization. It must restore validated completed state and prepare only remaining operations. Frozen interpretation data, recorded provenance, and editable downstream defaults must remain distinct even when they refer to the same parameter vocabulary.

Applicability must follow the executed operation and the owning model, not a parameter prefix or GUI section. Procedural grain remains post-scan appearance even though it is configured under `film_render.grain`. Already formed material grain must not be generated again. Camera context needed by negative conversion must remain available without applying camera exposure again.

Omitted `--seed` must inherit the artifact's seed. An explicit seed must affect only the remaining random operations. It must not replace an already stored realization. Image input keeps the default seed of 0. Existing repeated-iteration rules apply to the selected seed.

Already completed orientation, crop, resize, and physical scaling must not be applied again. Pending operations must execute exactly once under the saved geometry contract. Requests that would reinterpret frozen geometry must fail. An independently supported downstream geometry operation must remain downstream rather than modifying the stored state.

A consumer must report any admitted storage-to-computation precision conversion. It must reject a conversion that the owning model does not support. Cross-precision, cross-backend, and cross-model identity must not be promised. Equality acceptance must use the same supported precision, backend, model, parameters, and random realization.

## Output and publication

A route ending at `film` or `print` must select the artifact writer. A route ending at `scan` must select the existing image writer. The output path must not determine which operations run. Normal execution must require one `--output`; dry run may omit it.

Artifact output must reject image writer flags such as `--format`, `--bit-depth`, compression, JPEG options, and saving color or transfer conversion. Retained scanner and output parameters may remain editable downstream defaults; they must not cause the density payload to undergo an image-saving conversion.

Artifact output must not require an extra image destination. It must not create a preview image, debug directory, or sibling raw dump unless an independent existing output option explicitly requested that data. `--raw-out` keeps its numerical-dump role and does not become an artifact-input format. It must not advertise continuation capability.

Artifact destinations must not already exist. No artifact overwrite option is introduced. The destination and its staging paths must not alias inputs, profile/configuration sources, raw dumps, or diagnostics. Input artifacts must be read-only. A conflicting or invalid destination must fail before rendering when it can be determined during validation.

The writer must prepare all members in a sibling staging directory and finalize the manifest after members and hashes are complete. It must publish the completed directory without replacing an existing destination. A partial, failed, or cancelled write must not leave a valid artifact at the requested path. Failure must return nonzero and preserve existing user files.

A stale or incomplete staging directory must not be recognized as a published artifact. The reader must require the completed manifest and validate its members before use. `--iters` must publish only the final successful iteration supplying the result. Neither publication nor diagnostics may trigger a second render.

Normal `process` stdout remains empty. Progress and errors belong on stderr. Multiple explicitly requested destinations, such as a raw dump and the primary artifact, are not a single filesystem transaction. Failure must report any already published output honestly without claiming cross-destination atomicity.

## Dry run

Dry run must resolve the same input class, route, frozen-state constraints, model capability, profile dependencies, and output writer as execution. It must not decode numerical payloads, render, initialize a compute device, or create any output.

For artifact input, it may read the manifest and small embedded configuration/profile records. It must identify unchecked large-payload values and hashes. Normal execution must verify every required payload before using it. Dry-run validity must not be presented as proof of data integrity or device availability.

The existing JSON report must retain its current fields. `input.kind` must additionally admit `stage_artifact`. For that kind, `input.declaration` and `input.raw_loading` must be null. An `input.artifact` record must report the state, boundary, schema and representation versions, producer precision, and embedded material identities. Raster and RAW reports must not acquire a fabricated artifact record.

The report must show `execution` with the input state, already completed operations, and planned operations. It must show seed value and whether it comes from the artifact, explicit argument, or image-input default. Frozen-state conflicts are validation errors, not inactive-edit warnings.

For artifact output, `output.kind` must be `stage_artifact` and `output.format` must be `sfstage`, even if the path is omitted. `output.path` must then be null. Image-only writer fields must be null. For image output, `output.kind` must be `image` and existing format-resolution behavior remains unchanged. A route with no required print material must report `print_profile: null`, not resolve an unused stock.

Artifact dry runs must add `artifact_payload` and `artifact_integrity` to unchecked work when those checks were not performed. Compute-device and execution-dependent outcomes remain unchecked under the common dry-run contract. JSON and text must describe the same decisions. `--debug` may add stderr detail but must not change either report's stdout or decisions.

## Removed interfaces and research boundary

All active parsers and help must remove `--stage-evidence`. There must be no deprecated forwarding option, hidden alias, automatic rewrite, or debug-driven replacement. The abandoned `--save-stage`, `--export-stage`, `--debug DIRECTORY`, and multi-entry `.sfstage` designs must not be implemented.

Public `process` no longer provides simultaneous all-stage side capture. This is an intentional capability removal, not a claim that debug logs or separate partial renders provide equivalent evidence. Ordinary instrument suite and single-image runs must not gain implicit full capture to compensate.

Dedicated instrument measurement and experiment operations retain their explicit numerical-observation purpose under the [native observation contract](../instrument/native-observations.md). Research observation data is not a stage artifact and must not be accepted as one. Its existing readers remain research readers, not aliases for artifact continuation. The experiment export input uses a positional `INPUT`, not the removed option.

Removing the process option must not delete the core observation machinery still required by those research operations. They must use the shared execution path with an explicit instrument-owned observation request, not shell out through a retired option. No research caller may relabel several executions as one captured run.

Existing image-only RenderRecipe, GUI child-process, preset, LUT, and telemetry contracts remain unchanged unless their owning specification explicitly adopts stage artifacts. Those callers must not acquire new mandatory flags or stage persistence implicitly.

## Syntax examples

These POSIX examples define target syntax. Fixtures must be supplied by the implementation acceptance runner. They are not demonstrations that the current executable supports artifact processing.

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
3. Compare one-pass output with film-split and print-split output under the same model, seed, precision, backend, and effective controls. Establish exact identity for the deterministic same-path CPU reference fixture. For model/backend cases without bit identity, use the owning predeclared numerical budget and report measured errors; do not select a tolerance after observing the result.
4. Exercise active material grain, post-scan Procedural grain, non-square geometry, negative conversion's camera context, print material replacement, and prepared-source mixing. Verify each owning model's state completeness. Unsupported configurations must reject rather than disable effects or publish observation-only states.
5. Move artifacts away from original images and profile libraries. Modify a same-named external profile and verify that stored interpretation is unchanged. Verify immutable profile conflicts, downstream overrides, seed inheritance, and the separation between provenance and executable defaults.
6. Reject unknown versions, missing state, malformed layouts, non-finite required values, unsafe paths, hash mismatches, and truncated payloads before simulation. Exercise both f32 and f64 storage and admitted precision conversions without mislabeling computation precision.
7. Force cancellation, write failure, destination races, and path aliasing. Verify no valid-looking partial primary artifact, preservation of existing files, no second render, and nonzero status on failure.
8. Compare debug-on/off runs: primary payloads and numerical results must agree, stdout and structured dry-run results must remain unchanged, and no additional files may appear without an explicit file-output request or shell redirection. Exercise debug with diagnostics both absent and present.
9. Verify dry-run input classes, output kinds, unused print profile handling, frozen-conflict errors, and unchecked integrity/device outcomes. It must not modify sentinels or create staging directories.
10. Exercise dedicated measurement and experiment consumers after the retired flag is removed. Preserve their native numeric evidence and reference-protocol checks. Ordinary suite/image runs must report absent observation data without reconstructing it.

Acceptance must report executable identity, model/version, input provenance, complete invocations, status, stdout/stderr, artifact checks, and measured numerical comparisons. Documentation checks, shell parsing, and compilation alone are not processing acceptance.
