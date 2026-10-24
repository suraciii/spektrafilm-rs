# CLI design

This document is the unified CLI design. Parameter composition and workflow behavior remain distinct sections of one implementation boundary.

## Design stance
Use Unix-style design; dumb pipes, smart endpoints.

## Parameter boundary
- The [parameter contract](spec.md#parameter-contract) owns syntax, composition, validation, and inspection behavior. The CLI adapter MUST parse options and choose the execution contract.
- Core MUST own preset resolution and effective parameter preparation. The adapter MUST NOT implement another film pipeline or duplicate stock calibration rules.

A preset document, a sparse parameter file, and inline assignments MUST remain different source types. Sparse files and assignments MUST retain supplied leaf paths until resolution. Deserializing a file into default-filled controls before merge would discard the distinction between omission and explicit values.

- TOML and JSON MUST map into the same parameter types. Carrier parsing MUST NOT select a different resolver.
- Syntax adapters MUST NOT change units or coerce strings. The semantic validator MUST remain authoritative for field support, bounds, and combinations.
- Inline assignments MUST use the shared field types to distinguish plain strings from boolean, numeric, and array values. JSON quoting MUST remain available for strings that need escaping.
- Invalid numeric or boolean input MUST fail with a type-specific message instead of falling back to text. The carrier parser MUST return the same typed values to the existing leaf validator.

## Parameter preparation
- Stock look initialization, explicit controls, and runtime constraints MUST be separate preparation decisions. Preparation MUST retain each explicit neutral-filter axis through database calibration.
- It MUST use supplied-field information rather than comparing numbers with defaults. A value equal to a stock default is still an explicit edit.
- The capture illuminant is resolved in core, not in the CLI adapter. The CLI MUST NOT add a dedicated flag, a shortened path, or a second illuminant vocabulary for it.
- It is an optional camera leaf under the [capture illuminant contract](spec.md#capture-illuminant). It is resolved through the same field description, sparse-file and inline carriers, and shared illuminant validator as every other light-source leaf.
- An unset leaf resolves to the selected film profile's `reference_illuminant` after profile selection. The resolution report MUST record the requested value, the effective value, and the source that supplied it rather than inferring one from the other.

Explicit-axis ownership MUST survive runtime construction and calibration rebuilds. It MUST NOT change the user's database-calibration setting. Calibration MUST retain database f64 values for axes that remain database-owned.

A prepared configuration MUST NOT receive another stock initialization step when the runtime is constructed. Applying preview, LUT-mode, or debug constraints MUST NOT corrupt the editable controls used to resolve a subsequent configuration. Legacy baseline decoding and numerical preparation MUST remain unchanged except for the explicit declaration and route changes in the [compatibility contract](spec.md#compatibility).

- Core MUST return the resolved profiles and effective parameters as one validated result. The CLI MUST resolve and validate writer options separately from scanner output parameters.
- Writer resolution MUST NOT require an output artifact. A dry run without `--output` MUST resolve the writer defaults without inventing a destination path.
- Normal processing and dry run MUST consume these same resolution results. Dry run MUST stop before image decoding and compute-device creation.
- It MUST NOT approximate a render or independently reconstruct the parameter composition.

The resolution report MUST include deterministic topology and monochrome channel normalization performed by runtime construction. These decisions MUST use the selected profiles and workflow without decoding image pixels.

## Field description
- The editable field description MUST serve parameter-file and inline-assignment validation and parameter discovery. It MUST identify leaves, types, optional values, units, domains, and effect conditions.
- These facts MUST come from the owning parameter definitions and validators. The CLI MUST NOT maintain a second handwritten list of bounds or enum values.
- The carrier adapter MAY map field names to this description but MUST NOT invent shorter aliases.

Array controls with distinct component domains MUST describe each component. Source validation MUST apply the same component rules as runtime construction before a later source can replace the array.

- Single-field and module discovery MUST use the same metadata records. Text output MUST format these records without changing validation or static defaults.
- Selector-free JSON MUST remain the render adapter contract; selector-free text MUST list the editable groups instead. This explicit format choice MUST NOT depend on terminal detection.
- The dry-run report MUST combine the shared runtime result with the CLI's selected data location and RAW loading options. These adapter choices MUST remain outside runtime parameters.
- Raster input MUST NOT report RAW controls as active. Reporting the selected directory MUST NOT reselect it or load image pixels.

The [workflow design](#workflow-boundary) owns source-aware data selection, help, signed numeric argument handling, and corrective suggestions.

## Existing contracts
The original `--params` decoder MUST remain the legacy baseline decoder. A recursive merge into Rust defaults is not an equivalent replacement: a supplied group can have field-level deserialization defaults that differ from the group's Rust defaults. New sparse parameter files MUST operate on the resolved baseline rather than changing this decoder.

- For a legacy baseline followed by new parameter sources, baseline decoding MUST remain unchanged while preparation follows the added language's composition rules. Only preset, sparse-file, inline-assignment, and dedicated input-flag leaves MUST protect explicit edits from recalibration.
- Source tracking MUST NOT promote every leaf produced by legacy deserialization into a protected edit. This distinction MUST NOT require a second implementation of stock defaults or runtime constraints.

RenderRecipe MUST remain the structured render contract. The resolution report MUST NOT change recipe normalization, seed ownership, or parameter-digest computation. GUI persistence MUST follow the [preset state compatibility rules](../presets/spec.md#gui-state-compatibility).

## Source declarations and execution context
- The CLI MUST retain supplied input leaves separately from decoded baseline values. This distinction permits legacy files to declare source color without changing their calibration ownership.
- Dedicated input flags MUST use the same typed field validation as `--set`; they MUST NOT create a second color-space registry or an input-conversion algorithm. The RAW adapter MUST remain the authority for RAW sample color.
- Core owns typed completed material states and the artifact reader, writer, and validator. CLI selects an input state, finite route, and terminal writer; it MUST NOT reconstruct material state from diagnostic arrays or reimplement pipeline mathematics.
- Dedicated instrument observations share the executed pipeline but remain research data under their [own contract](../instrument/native-observations.md). Removing a public side-capture option MUST NOT remove an observer still required by research commands.
- Route selection MUST finish before runtime print-profile selection. Public process paths MUST derive topology from the selected route through the same rule, with or without parameter sources.
- Parser conflicts MUST eliminate redundant route choices. Core MUST still reject an inconsistent invocation from a non-parser caller.
- Internal GUI child contracts MUST retain their own admitted behavior rather than inherit new public argument requirements.
- Seed MUST be an invocation value, not a sparse runtime leaf. The CLI MUST reuse the admitted seed domain from structured render.
- Both adapters MUST apply it to the runtime before simulation. Seed MUST NOT select a digest policy or modify parameters other than the existing out-of-band stochastic seed.
- Repeated benchmark attempts MUST restart from the same input and seed.

## Resolution feedback
- The shared resolver MUST retain winning explicit edits before constraints and compare them with the effective typed result. It MUST retain source identity only where the [report contract](spec.md#discovery-and-dry-run) requires it.
- Tracking MUST NOT serialize a complete history or copy image buffers. One feedback evaluation MUST serve normal processing and both dry-run views.
- The authoritative effect conditions MUST be evaluable against effective controls and selected workflow. Validators, discovery, and feedback MUST share these conditions.
- Parsing human condition strings is not an effect evaluator. The CLI MUST NOT add a handwritten switch over every control.
- Controls with no known disabling condition MAY be classified as active; this classification MUST NOT claim measured influence on output pixels.
- When a source is invalid, validation MUST fail before feedback is emitted for a resolved configuration. When a valid edit is inactive, feedback MUST preserve it rather than activate another control.
- Text rendering MUST format the same result as JSON. Diagnostics remain on stderr and telemetry remains independent.

## Design choice and implementation dependencies
- Canonical runtime paths and the existing sparse TOML/JSON language are retained. Short aliases would duplicate vocabulary and obscure distinct exposure and grain controls.
- A complete job file would duplicate profile, render-recipe, and export boundaries. Neither alternative is part of this design.
- Dedicated source declarations and seed flags solve invocation context without expanding the look language.
- Safe publication MUST land before new success-facing image examples are accepted. The publication transaction belongs to [photo export](../photo-export/design.md#transaction-boundary).
- Declaration, seed, and route changes MUST then migrate every public process caller, README example, packaged example, acceptance script, and affected behavior test. New required declarations MUST NOT be bypassed by legacy-only calls or a compatibility flag.
- Feedback and text presentation MUST build on the same prepared result after source tracking is available. Selected-field descriptions MUST be updated with the authoritative conditions.
- Existing RenderRecipe, selector-free discovery, GUI child-process, LUT, and preset document schemas MUST stay unchanged. GUI internal calls that use `process` MUST be exercised without silently treating them as public batch invocations.
- Implementation MUST run the existing formatting, relevant checks, and relevant tests defined in `Justfile`. It MUST additionally exercise the [real-entry-point scenarios](spec.md#image-processing-acceptance) and [publication failures](../photo-export/spec.md#publication-acceptance).
- The current f64-feature workspace tests alone do not establish default-f32 executable behavior. Baseline incidental-behavior tests for ambiguous route precedence MUST be removed or replaced by consumer-visible route invariants, not re-pinned to a new incidental order.

## Workflow boundary
- The [workflow contract](spec.md#workflow-contract) is an adapter over the existing parameter, image-input, LUT-delivery, and telemetry capabilities. It MUST retain clap derive and existing command handlers.
- Parser replacement, shell completion, terminal prompts, and a replacement machine protocol do not belong to this change. Additive dry-run feedback and explicit text presentation belong to the [parameter contract](spec.md#discovery-and-dry-run).
- Clap MUST own external argument forms, finite CLI choices, argument relations, help, and syntax errors. Core MUST own parameter types, supported field paths, runtime choices, and semantic validation.
- Human field descriptions and suggestions MUST consume the existing field description defined by the [parameter design](#field-description). They MUST NOT maintain another inventory of fields or bounds.

## Explicit selection and automatic discovery
- Data selection MUST retain whether a path came from the command line, environment, or automatic discovery. A defaulted `PathBuf` alone loses this distinction.
- The adapter MUST obtain source information from clap before choosing a path. Comparing a path with the string `data` cannot determine whether the user supplied it.
- One adapter-owned selection function MUST implement source precedence, explicit-path validation, and automatic candidate order for all data-consuming commands. It MUST return the selected path and source together.
- Resource loading MUST remain owned by the consuming capability. The selected location MUST NOT change after a missing profile or calibration asset is found.

The source distinction protects two workflows: a relocated package can find bundled data without a flag, and a user selecting another installation receives an error instead of an unrelated stock database. An executable-relative default MUST NOT override an explicit path. The CLI MUST NOT depend on a runtime Cargo manifest environment variable to find release data.

## Descriptions and corrective guidance
Help groups and examples MUST be defined with existing clap attributes. Signed numeric options MUST allow negative numeric values without allowing arbitrary hyphen-prefixed values. Existing numeric and semantic validators MUST still run.

- Parameter candidates MUST come from editable schema leaves. Module candidates MUST come from group prefixes in that same schema.
- Preset candidates MUST come from built-in IDs. Workflow candidates MUST come from the workflow validator's supported routes.
- Matching MUST be deterministic and bounded. It MUST run only after validation fails.
- It MUST NOT select a candidate on the user's behalf or require loading image data. If no close candidate exists, the error MUST offer a discovery command instead.
- Suggestions MUST use Levenshtein edit distance over the supplied identifier and canonical candidates. Inputs of at most four characters MUST use a maximum distance of one.
- Longer inputs MUST use a maximum distance of two. Results MUST sort by distance, then canonical spelling, and stop after three candidates.
- When no candidate meets the threshold, the diagnostic MUST provide discovery guidance instead.

Descriptions of fields, units, array component domains, optional values, defaults, and effect conditions MUST come from the existing metadata surface. Human formatting MUST NOT become a second validator. JSON defaults and the structured render adapter contract MUST remain intact.

- TOML syntax errors already retain parser locations and source fragments. This implementation MUST preserve those diagnostics.
- It MUST NOT add miette or migrate anyhow solely to restyle errors. Source-span tracking for semantic file errors is outside this implementation.

## Inspection and execution
- Dry run MUST share profile, runtime, writer, RAW-option, and data-selection decisions with normal processing. It MUST report resolved choices before decode and device creation.
- It MUST distinguish inactive RAW controls on raster input from controls used for RAW loading. It MUST NOT claim that a header-only RAW probe proves a file can decode.

Native CPU precision in version output MUST follow the compiled executable's actual precision. A filename or output sample depth MUST NOT decide that value. GPU selection and actual execution remain runtime facts owned by the existing backend and telemetry contracts.

- Terminal LUT stage notices MUST be emitted at existing bake and QA boundaries. They do not require a progress framework, telemetry collection, or percentage estimation.
- Completion and error paths MUST still produce useful redirected stderr. A later progress bar would require real progress events and is outside this implementation.

## Compatibility and verification
Existing `--set` source ordering, sparse-file rules, legacy `--params` decoding, preset profile ownership, and numerical preparation MUST remain authoritative in the [parameter contract](spec.md#parameter-contract). This work MUST preserve GUI child-process, RenderRecipe, and inspect contracts. Telemetry remains a separate report surface.

Changing explicit invalid data paths from fallback to failure and moving LUT notices from stdout to stderr are intentional observable changes. Their help and documentation MUST describe the resulting behavior. Existing JSON defaults MUST NOT be changed according to terminal detection.

- Verification MUST run both real CLI entry points for syntax, diagnostics, discovery, dry run, and image output. Directory-selection scenarios MUST isolate executable location, current directory, environment, and explicit arguments.
- Version and help MUST be exercised without valid data. A deterministic CPU render MUST compare raw output across the usability change with the same effective parameters.
- Tests MUST cover plausible behavior failures rather than exact help layout or source text.

## Stage state and diagnostic separation
- The [stage input and output contract](stage-artifacts.md) owns single-entry resumable artifacts. A validated input state MUST restore completed operations and initialize only the remaining operations.
- Profile content and model-required state MUST belong to the artifact; mutable external names MUST NOT reinterpret a saved material. A full parameter snapshot is provenance, not instructions to rerun already completed work.
- `--debug` is a Boolean logging request owned by the [CLI diagnostic contract](spec.md#debug-diagnostics). It MUST reuse stderr logging and existing observation events without enabling numerical debug constraints, GPU timing, stage capture, or persistence.
- Shell redirection owns text-log files. Explicit `--diagnostics` owns structured report persistence independently.
- The rejected design combined observations and resumable state in one multi-entry `.sfstage`, with `--save-stage` or `--export-stage` bindings. It made a production input's validity depend on a capture capability.
- The selected design uses one normal terminal output, one guaranteed continuation entry, and a separate logging flag. It intentionally removes public simultaneous all-stage side capture rather than hiding it behind debug.
- Dedicated research commands retain their own explicit observation purpose.

The stage and debug contracts are specification targets. Runtime implementation and acceptance are not established by this documentation change. The cutover MUST update all public callers, instrument adapters, validation, and examples together, with no retired-option alias.
