# CLI design

This document is the unified CLI design. Parameter composition and workflow behavior remain distinct sections of one implementation boundary.

## Design stance

Use Unix-style design; dumb pipes, smart endpoints.

## Parameter boundary

The [parameter contract](spec.md#parameter-contract) owns syntax, composition, validation, and inspection behavior. The CLI adapter must parse options and choose the execution contract. Core must own preset resolution and effective parameter preparation. The adapter must not implement another film pipeline or duplicate stock calibration rules.

A preset document, a sparse parameter file, and inline assignments must remain different source types. Sparse files and assignments must retain supplied leaf paths until resolution. Deserializing a file into default-filled controls before merge would discard the distinction between omission and explicit values.

TOML and JSON must map into the same parameter types. Carrier parsing must not select a different resolver. Syntax adapters must not change units or coerce strings. The semantic validator must remain authoritative for field support, bounds, and combinations.

Inline assignments must use the shared field types to distinguish plain strings from boolean, numeric, and array values. JSON quoting must remain available for strings that need escaping. Invalid numeric or boolean input must fail with a type-specific message instead of falling back to text. The carrier parser must return the same typed values to the existing leaf validator.

## Parameter preparation

Stock look initialization, explicit controls, and runtime constraints must be separate preparation decisions. Preparation must retain each explicit neutral-filter axis through database calibration. It must use supplied-field information rather than comparing numbers with defaults. A value equal to a stock default is still an explicit edit.

Explicit-axis ownership must survive runtime construction and calibration rebuilds. It must not change the user's database-calibration setting. Calibration must retain database f64 values for axes that remain database-owned.

A prepared configuration must not receive another stock initialization step when the runtime is constructed. Applying preview, LUT-mode, or debug constraints must not corrupt the editable controls used to resolve a subsequent configuration. Legacy baseline decoding and numerical preparation must remain unchanged except for the explicit declaration and route changes in the [compatibility contract](spec.md#compatibility).

Core must return the resolved profiles and effective parameters as one validated result. The CLI must resolve and validate writer options separately from scanner output parameters. Writer resolution must not require an output artifact. A dry run without `--output` must resolve the writer defaults without inventing a destination path. Normal processing and dry run must consume these same resolution results. Dry run must stop before image decoding and compute-device creation. It must not approximate a render or independently reconstruct the parameter composition.

The resolution report must include deterministic topology and monochrome channel normalization performed by runtime construction. These decisions must use the selected profiles and workflow without decoding image pixels.

## Field description

The editable field description must serve parameter-file and inline-assignment validation and parameter discovery. It must identify leaves, types, optional values, units, domains, and effect conditions. These facts must come from the owning parameter definitions and validators. The CLI must not maintain a second handwritten list of bounds or enum values. The carrier adapter may map field names to this description but must not invent shorter aliases.

Array controls with distinct component domains must describe each component. Source validation must apply the same component rules as runtime construction before a later source can replace the array.

Single-field and module discovery must use the same metadata records. Text output must format these records without changing validation or static defaults. Selector-free JSON must remain the render adapter contract; selector-free text must list the editable groups instead. This explicit format choice must not depend on terminal detection.

The dry-run report must combine the shared runtime result with the CLI's selected data location and RAW loading options. These adapter choices must remain outside runtime parameters. Raster input must not report RAW controls as active. Reporting the selected directory must not reselect it or load image pixels.

The [workflow design](#workflow-boundary) owns source-aware data selection, help, signed numeric argument handling, and corrective suggestions.

## Existing contracts

The original `--params` decoder must remain the legacy baseline decoder. A recursive merge into Rust defaults is not an equivalent replacement: a supplied group can have field-level deserialization defaults that differ from the group's Rust defaults. New sparse parameter files must operate on the resolved baseline rather than changing this decoder.

For a legacy baseline followed by new parameter sources, baseline decoding must remain unchanged while preparation follows the added language's composition rules. Only preset, sparse-file, inline-assignment, and dedicated input-flag leaves must protect explicit edits from recalibration. Source tracking must not promote every leaf produced by legacy deserialization into a protected edit. This distinction must not require a second implementation of stock defaults or runtime constraints.

RenderRecipe must remain the structured render contract. The resolution report must not change recipe normalization, seed ownership, or parameter-digest computation. GUI persistence must follow the [preset state compatibility rules](../presets/spec.md#gui-state-compatibility).

## Source declarations and execution context

The CLI must retain supplied input leaves separately from decoded baseline values. This distinction permits legacy files to declare source color without changing their calibration ownership. Dedicated input flags must use the same typed field validation as `--set`; they must not create a second color-space registry or an input-conversion algorithm. The RAW adapter must remain the authority for RAW sample color.

Native stage evidence is a core-owned capture and bundle format, not a CLI reconstruction: core observes the metering actually applied, the prepared print normalization and the tap boundaries of the executed route, and publishes the manifest last. The CLI only selects the destination, rejects execution the capture route cannot describe (GPU backends, preview mode), and treats an incomplete or unwritable bundle as a command failure distinct from optional telemetry persistence. The capture must share the rendering path it observes; a parallel re-execution or an independent stage walk would report different stochastic output and must not exist.

Route selection must finish before runtime print-profile selection. Public process paths must derive topology from the selected route through the same rule, with or without parameter sources. Parser conflicts must eliminate redundant route choices. Core must still reject an inconsistent invocation from a non-parser caller. Internal GUI child contracts must retain their own admitted behavior rather than inherit new public argument requirements.

Seed must be an invocation value, not a sparse runtime leaf. The CLI must reuse the admitted seed domain from structured render. Both adapters must apply it to the runtime before simulation. Seed must not select a digest policy or modify parameters other than the existing out-of-band stochastic seed. Repeated benchmark attempts must restart from the same input and seed.

## Resolution feedback

The shared resolver must retain winning explicit edits before constraints and compare them with the effective typed result. It must retain source identity only where the [report contract](spec.md#discovery-and-dry-run) requires it. Tracking must not serialize a complete history or copy image buffers. One feedback evaluation must serve normal processing and both dry-run views.

The authoritative effect conditions must be evaluable against effective controls and selected workflow. Validators, discovery, and feedback must share these conditions. Parsing human condition strings is not an effect evaluator. The CLI must not add a handwritten switch over every control. Controls with no known disabling condition may be classified as active; this classification must not claim measured influence on output pixels.

When a source is invalid, validation must fail before feedback is emitted for a resolved configuration. When a valid edit is inactive, feedback must preserve it rather than activate another control. Text rendering must format the same result as JSON. Diagnostics remain on stderr and telemetry remains independent.

## Design choice and implementation dependencies

Canonical runtime paths and the existing sparse TOML/JSON language are retained. Short aliases would duplicate vocabulary and obscure distinct exposure and grain controls. A complete job file would duplicate profile, render-recipe, and export boundaries. Neither alternative is part of this design. Dedicated source declarations and seed flags solve invocation context without expanding the look language.

Safe publication must land before new success-facing image examples are accepted. The publication transaction belongs to [photo export](../photo-export/design.md#transaction-boundary). Declaration, seed, and route changes must then migrate every public process caller, README example, packaged example, acceptance script, and affected behavior test. New required declarations must not be bypassed by legacy-only calls or a compatibility flag.

Feedback and text presentation must build on the same prepared result after source tracking is available. Selected-field descriptions must be updated with the authoritative conditions. Existing RenderRecipe, selector-free discovery, GUI child-process, LUT, and preset document schemas must stay unchanged. GUI internal calls that use `process` must be exercised without silently treating them as public batch invocations.

Implementation must run the existing formatting, relevant checks, and relevant tests defined in `Justfile`. It must additionally exercise the [real-entry-point scenarios](spec.md#image-processing-acceptance) and [publication failures](../photo-export/spec.md#publication-acceptance). The current f64-feature workspace tests alone do not establish default-f32 executable behavior. Baseline incidental-behavior tests for ambiguous route precedence must be removed or replaced by consumer-visible route invariants, not re-pinned to a new incidental order.

## Workflow boundary

The [workflow contract](spec.md#workflow-contract) is an adapter over the existing parameter, image-input, LUT-delivery, and telemetry capabilities. It must retain clap derive and existing command handlers. Parser replacement, shell completion, terminal prompts, and a replacement machine protocol do not belong to this change. Additive dry-run feedback and explicit text presentation belong to the [parameter contract](spec.md#discovery-and-dry-run).

Clap must own external argument forms, finite CLI choices, argument relations, help, and syntax errors. Core must own parameter types, supported field paths, runtime choices, and semantic validation. Human field descriptions and suggestions must consume the existing field description defined by the [parameter design](#field-description). They must not maintain another inventory of fields or bounds.

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

Existing `--set` source ordering, sparse-file rules, legacy `--params` decoding, preset profile ownership, and numerical preparation must remain authoritative in the [parameter contract](spec.md#parameter-contract). This work must preserve GUI child-process, RenderRecipe, and inspect contracts. Telemetry remains a separate report surface.

Changing explicit invalid data paths from fallback to failure and moving LUT notices from stdout to stderr are intentional observable changes. Their help and documentation must describe the resulting behavior. Existing JSON defaults must not be changed according to terminal detection.

Verification must run both real CLI entry points for syntax, diagnostics, discovery, dry run, and image output. Directory-selection scenarios must isolate executable location, current directory, environment, and explicit arguments. Version and help must be exercised without valid data. A deterministic CPU render must compare raw output across the usability change with the same effective parameters. Tests must cover plausible behavior failures rather than exact help layout or source text.
