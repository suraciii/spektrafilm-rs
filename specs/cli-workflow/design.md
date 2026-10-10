# CLI workflow design

## Boundaries

The [CLI workflow](spec.md) is an adapter over the existing parameter, image-input, LUT-delivery, and telemetry capabilities. It must retain clap derive and existing command handlers. Parser replacement, shell completion, terminal prompts, and a new machine protocol do not belong to this change.

Clap must own external argument forms, finite CLI choices, argument relations, help, and syntax errors. Core must own parameter types, supported field paths, runtime choices, and semantic validation. Human field descriptions and suggestions must consume the existing field description defined by the [CLI parameter design](../cli-parameters/design.md#field-description). They must not maintain another inventory of fields or bounds.

## Explicit selection and automatic discovery

Data selection must retain whether a path came from the command line, environment, or automatic discovery. A defaulted `PathBuf` alone loses this distinction. The adapter must obtain source information from clap before choosing a path. Comparing a path with the string `data` cannot determine whether the user supplied it.

One adapter-owned selection function must implement source precedence, explicit-path validation, and automatic candidate order for all data-consuming commands. It must return the selected path and source together. Resource loading must remain owned by the consuming capability. The selected location must not change after a missing profile or calibration asset is found.

The source distinction protects two workflows: a relocated package can find bundled data without a flag, and a user selecting another installation receives an error instead of an unrelated stock database. An executable-relative default must not override an explicit path. The CLI must not depend on a runtime Cargo manifest environment variable to find release data.

## Descriptions and corrective guidance

Help groups and examples must be defined with existing clap attributes. Signed numeric options must allow negative numeric values without allowing arbitrary hyphen-prefixed values. Existing numeric and semantic validators must still run.

Parameter candidates must come from editable schema leaves. Module candidates must come from group prefixes in that same schema. Preset candidates must come from built-in IDs. Workflow candidates must come from the workflow validator's supported routes. Matching must be deterministic and bounded. It must run only after validation fails. It must not select a candidate on the user's behalf or require loading image data. If no close candidate exists, the error must offer a discovery command instead.

Suggestions must use Levenshtein edit distance over the supplied identifier and canonical candidates. Inputs of at most four characters must use a maximum distance of one. Longer inputs must use a maximum distance of two. Results must sort by distance, then canonical spelling, and stop after three candidates. When no candidate meets the threshold, the diagnostic must provide discovery guidance instead.

Descriptions of fields, units, array component domains, optional values, defaults, and effect conditions must come from the existing metadata surface. Human formatting must not become a second validator. JSON defaults and the structured render adapter contract must remain intact.

TOML syntax errors already retain parser locations and source fragments. This implementation must preserve those diagnostics. It must not add miette or migrate anyhow solely to restyle errors. Source-span tracking for semantic file errors is outside this implementation.

## Inspection and execution

Dry run must share profile, runtime, writer, RAW-option, and data-selection decisions with normal processing. It must report resolved choices before decode and device creation. It must distinguish inactive RAW controls on raster input from controls used for RAW loading. It must not claim that a header-only RAW probe proves a file can decode.

Native CPU precision in version output must follow the compiled executable's actual precision. A filename or output sample depth must not decide that value. GPU selection and actual execution remain runtime facts owned by the existing backend and telemetry contracts.

Terminal LUT stage notices must be emitted at existing bake and QA boundaries. They do not require a progress framework, telemetry collection, or percentage estimation. Completion and error paths must still produce useful redirected stderr. A later progress bar would require real progress events and is outside this implementation.

## Compatibility and verification

Existing `--set` source ordering, sparse-file rules, legacy `--params` decoding, preset profile ownership, and numerical preparation must remain authoritative in the [CLI parameter specification](../cli-parameters/spec.md). This work must preserve GUI child-process, RenderRecipe, inspect, and parity contracts. Telemetry remains a separate report surface.

Changing explicit invalid data paths from fallback to failure and moving LUT notices from stdout to stderr are intentional observable changes. Their help and documentation must describe the resulting behavior. Existing JSON defaults must not be changed according to terminal detection.

Verification must run both real CLI entry points for syntax, diagnostics, discovery, dry run, and image output. Directory-selection scenarios must isolate executable location, current directory, environment, and explicit arguments. Version and help must be exercised without valid data. A deterministic CPU render must compare raw output across the usability change with the same effective parameters. Tests must cover plausible behavior failures rather than exact help layout or source text.
