# CLI parameter design

## Boundary

The [CLI parameter language](spec.md) owns syntax, composition, validation, and inspection behavior. The CLI adapter must parse options and choose the execution contract. Core must own preset resolution and effective parameter preparation. The adapter must not implement another film pipeline or duplicate stock calibration rules.

A complete preset, a sparse parameter file, and inline assignments must remain different source types. Sparse files and assignments must retain supplied leaf paths until resolution. Deserializing a file into default-filled controls before merge would discard the distinction between omission and explicit values.

TOML and JSON must map into the same parameter types. Carrier parsing must not select a different resolver. Syntax adapters must not change units or coerce strings. The semantic validator must remain authoritative for field support, bounds, and combinations.

Inline assignments must use the shared field types to distinguish plain strings from boolean, numeric, and array values. JSON quoting must remain available for strings that need escaping. Invalid numeric or boolean input must fail with a type-specific message instead of falling back to text. The carrier parser must return the same typed values to the existing leaf validator.

## Parameter preparation

Stock look initialization, explicit controls, and runtime constraints must be separate preparation decisions. Preparation must retain each explicit neutral-filter axis through database calibration. It must use supplied-field information rather than comparing numbers with defaults. A value equal to a stock default is still an explicit edit.

Explicit-axis ownership must survive runtime construction and calibration rebuilds. It must not change the user's database-calibration setting. Calibration must retain database f64 values for axes that remain database-owned.

A prepared configuration must not receive another stock initialization step when the runtime is constructed. Applying preview, LUT-mode, or debug constraints must not corrupt the editable controls used to resolve a subsequent configuration. Legacy invocations must retain their established preparation order and numerical results.

Core must return the resolved profiles and effective parameters as one validated result. The CLI must resolve and validate writer options separately from scanner output parameters. Normal processing and dry run must consume these same resolution results. Dry run must stop before image decoding and compute-device creation. It must not approximate a render or independently reconstruct the parameter composition.

The resolution report must include deterministic topology and monochrome channel normalization performed by runtime construction. These decisions must use the selected profiles and workflow without decoding image pixels.

## Field description

The editable field description must serve parameter-file and inline-assignment validation and parameter discovery. It must identify leaves, types, optional values, units, domains, and effect conditions. These facts must come from the owning parameter definitions and validators. The CLI must not maintain a second handwritten list of bounds or enum values. The carrier adapter may map field names to this description but must not invent shorter aliases.

Array controls with distinct component domains must describe each component. Source validation must apply the same component rules as runtime construction before a later source can replace the array.

Single-field and module discovery must use the same metadata records. Text output must format these records without changing validation or static defaults. Selector-free JSON must remain the render adapter contract; selector-free text must list the editable groups instead. This explicit format choice must not depend on terminal detection.

The dry-run report must combine the shared runtime result with the CLI's selected data location and RAW loading options. These adapter choices must remain outside runtime parameters. Raster input must not report RAW controls as active. Reporting the selected directory must not reselect it or load image pixels.

The [CLI workflow design](../cli-workflow/design.md) owns source-aware data selection, help, signed numeric argument handling, and corrective suggestions.

## Existing contracts

The original `--params` decoder must remain the legacy baseline decoder. A recursive merge into Rust defaults is not an equivalent replacement: a supplied group can have field-level deserialization defaults that differ from the group's Rust defaults. New sparse parameter files must operate on the resolved baseline rather than changing this decoder.

For a legacy baseline followed by new parameter sources, baseline decoding must remain unchanged while preparation follows the added language's composition rules. Only preset, sparse-file, and inline-assignment sources must protect explicit values from recalibration. Source tracking must not promote every leaf produced by legacy deserialization into an explicit edit. This distinction must not require a second implementation of stock defaults or runtime constraints.

RenderRecipe must remain the structured render contract. The resolution report must not change recipe normalization, seed ownership, or parameter-digest computation. GUI persistence must follow the [preset state compatibility rules](../presets/spec.md#gui-state-compatibility).
