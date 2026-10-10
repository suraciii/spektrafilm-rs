# CLI parameter language

## Contract

The `process` command must support [look presets](../presets/spec.md), sparse parameter files, and individual parameter assignments through the unified `--set` option. Both `spektrafilm` and `spektrafilm-f64` must accept the same language. The executable and backend must retain their existing computation precision contracts.

A **parameter override** is a sparse set of explicit edits to runtime controls for one invocation. It may contain look-owned and non-look controls. It must not change profile references or rewrite its source preset. A parameter override is not a complete preset, GUI state, or render recipe.

This document owns the added command syntax. Preset document fields and carriers belong to the [preset specification](../presets/spec.md). Field names, units, bounds, and conditional effect behavior belong to their parameter model and capability specifications. Help, data-directory selection, command status, and human feedback belong to the [CLI workflow specification](../cli-workflow/spec.md).

## Process syntax

The command must accept these forms:

```text
spektrafilm process INPUT --output OUTPUT --film STOCK [--paper STOCK] [--params FILE] [OPTIONS]
spektrafilm process INPUT --output OUTPUT --preset SELECTOR [OPTIONS]

OPTIONS may include:
  --set SOURCE          repeatable file or comma-separated assignments
  --dry-run             resolve and validate; print effective parameters without rendering
```

Existing backend, workflow, RAW, output, and data-directory options must remain available.

### Preset selector

A selector ending in `.toml` or `.json` must designate a file path relative to the current working directory or an absolute path. It must not fall back to a built-in ID if the file is absent or invalid. A selector without a supported extension must designate an exact built-in preset ID. Such an ID must not contain a path separator. Names shown in the GUI must not act as IDs. The command must not search user libraries or directories for an unknown ID.

`--preset` must conflict with `--film`, `--paper`, and `--params`. Without `--preset`, `--film` must remain required to resolve the runtime profiles. The `input` workflow must ignore the resolved film and paper profiles. Without a preset, `--scan-film` or the direct film scan route must select the film as the runtime print profile. Otherwise, a photographic print route must obtain its print profile from `--paper` or the film's `target_print`. Magazine enablement must not affect these profile requirements.
Without a preset, a print target must accept `support=paper` or `support=film` with `stage=printing` when parameter sources are added. Preset print references retain the paper-only restriction in the preset specification.

With a preset, both profile references must come from the preset. `--scan-film` must request `input > film > scan` without replacing the preset's print reference. The preset's workflow default must apply unless `--route` or `--scan-film` supplies an explicit workflow choice. `--route` must retain precedence over `--scan-film`. For invocations with added parameter sources, the final base route must determine topology: `io.scan_film` must be true exactly for `input > film > scan`, and false for the other supported routes. The existing workflow validator must determine which routes are supported. Magazine enablement must use the existing `magazine_print_color` parameter fields, independently of `--route`, as specified by the [Magazine print contract](../magazine-print-color/spec.md).

`--scan-output direct_scan|positive_scan` must override `scanner.scan_output` after all parameter sources. The shared resolver must enforce the [direct negative-film scan contract](../film-chain/spec.md#direct-negative-film-scan-output) before runtime constraints and for the effective parameters. Dry run and rendering must reject the same unsupported scan combinations.

## `--set` sources

Each `--set SOURCE` occurrence must be exactly one source. A source beginning with a canonical `PATH=` must be an inline assignment source, even when its value ends in `.toml` or `.json`. Otherwise, a source ending in `.toml` or `.json` must be a sparse parameter-file path. An inline source must contain one or more comma-separated `PATH=VALUE` assignments. A source must not mix a file path and assignments. `--set` may be repeated; sources apply in appearance order. A file path must be relative to the working directory or absolute. An assignment source must be passed as one shell argument.

For a file source, the extension must select the parser. Parse failure must not cause a fallback to another parser. Files must not support includes or environment interpolation. A missing or invalid `.toml`/`.json` source must fail as a file error; it must not fall back to a built-in preset ID or assignment parsing.

The document root must be an object. Nested objects or TOML tables must locate runtime parameter groups. Only supplied leaves must be edited. A missing leaf must retain its value from the preceding layer. An empty object must make no edits. Scalars and arrays must replace their whole leaf value. Arrays must not merge by index. A group must not be replaced by a scalar or `null`.

A file must not contain preset metadata, a `parameters` wrapper, profile references, GUI sections, or render-recipe fields. Unknown fields and duplicate keys must fail. A sparse file must not acquire omitted values by deserialization into a default-filled runtime parameter structure before merging.

TOML omission must mean no edit. JSON `null` must set an optional leaf to its unset value. It must not mean delete the field. Non-optional leaves must reject `null` in the added parameter-file language, even if a legacy loader accepts it through normalization.

TOML date/time values, non-finite floats, and values that cannot map to the declared parameter type must fail. Readers must not stringify these values. JSON and TOML must use the same leaf-type, range, and enum validation after carrier parsing.

The following TOML document is a sparse parameter file:

```toml
[film_render.grain]
engine = "v2"
v2_profile = "35mm250"
v2_amount = 25.0

[film_render.halation]
halation_amount = 0.6

[camera]
exposure_compensation_ev = 0.5
```

The following JSON document clears one optional parameter:

```json
{"film_render":{"grain":{"v2_amount":null}}}
```

### Inline assignments

An assignment must contain a canonical runtime path, the first `=`, and a value interpreted by the target field type. A path must consist of dot-separated field identifiers. Each identifier must match `[a-z][a-z0-9_]*`. Paths must be case-sensitive. Additional `=` characters must remain part of the value. Empty paths and empty values must fail.

Assignments in one source are separated by commas at the top level. Commas inside a double-quoted string or an array must remain part of that value. Whitespace around the separator and assignment is ignored. Empty items must fail. Object values, group replacement, array indexing, aliases, arithmetic, and units are not supported.

String and enum fields must accept plain text without JSON quotes. Shell quotes around an argument must be sufficient for spaces, such as `--set 'io.output_color_space=ProPhoto RGB'`. A value beginning with a double quote must remain a complete JSON string; malformed quoted strings must fail. JSON strings must remain available for literal commas, escapes, and empty strings. Bare text such as `true`, `123`, or `null` must remain text for non-optional string fields. For an optional field, unquoted `null` must clear the value.

Boolean fields must accept `true` or `false`. Numeric fields must accept finite numbers representable by the declared type. Floating-point fields must also accept ordinary decimal forms such as `+0.5` and `.5`. Array fields must use JSON arrays, such as `[4.5,4.5,4.5]`. Integers may supply floating-point fields. Fractional numbers must not supply integer fields. Quoted strings must not be coerced into numbers or booleans. Type errors must identify the field and expected value form. Enum errors must identify the permitted values.

The following source is valid:

```text
film_render.grain.engine=v2,film_render.grain.v2_profile=custom,film_render.grain.v2_amount=25,camera.auto_exposure=false
```

For a POSIX shell, quote a source containing brackets, spaces, or shell-significant characters:

```bash
spektrafilm process input.png --output output.tif \
  --preset classic-kodak-portra-400 \
  --set film_render.grain.engine=v2 \
  --set 'io.output_color_space=ProPhoto RGB' \
  --set 'camera.exposure_compensation_ev=0.5,io.input_cctf_decoding=true'
```

The assignment parser must split only at top-level commas, then interpret each value using the shared field description. It must not split commas in double-quoted strings or arrays. The sparse-file carrier rules must remain unchanged.

`--set` sources must be syntactically valid even if a later source replaces one of their leaves. The existing `--preset`, `--film`, `--paper`, `--params`, and `--route` selectors must each occur at most once.

### Editable paths and conditional controls

Paths must use the runtime parameter vocabulary, such as `film_render.grain.v2_amount`. The command must not add shortened paths such as `grain.amount`. It must not infer a grain engine or change `v2_profile` to `custom` from the presence of a control.

`workflow.route` must use `--route`; `io.scan_film` must remain derived from the selected workflow. Neither path may appear in a parameter file or inline assignment. Random seed must remain outside this language. Pipeline-derived `film_render.grain.monochrome` and the legacy non-exported `particle_scale_layers` field must not be editable paths. Other runtime controls must use the shared editable parameter description.

Controls for inactive effects or engines may be stored without affecting output. [Grain V2](../film-grain/v2/spec.md) must retain its named-profile and Custom behavior. V1 RMS granularity and V2 Amount must remain distinct controls. A parameter-file leaf or inline assignment must not change another control to make itself effective.

## Composition

Options form fixed layers: the preset is resolved first, then `--set` sources are applied in their appearance order, regardless of their position among unrelated options. A later source replaces the preceding value at a matching leaf. Inline assignments within one source also apply in their written order.

For an invocation with added parameter sources, resolution must use this order:

1. Select and validate the film and print profiles. Decode the baseline with `RuntimeParams::default()` or the unchanged legacy `--params` decoder. Apply stock-specific look initialization to this baseline with the same precedence as the legacy batch policy. Defer preview, debug, and database-calibration preparation until the final controls are available. Values supplied only by legacy `--params` must retain their legacy relationship to stock initialization and database calibration.
2. If a preset is selected, resolve and apply its complete look through the shared preset resolver. Retain excluded runtime context and derive the profile workflow default.
3. Apply every `--set` source in appearance order. A file source and an inline assignment source have identical leaf-merge semantics.
4. Apply explicit workflow choices and enforce input constraints. RAW must use linear ACES2065-1. An explicit parameter-file value or inline assignment that requests an incompatible RAW input color space or decoding mode must fail instead of being silently ignored.
5. Validate the composed controls. Resolve dependent database defaults only for values that are not explicit. Apply preview, LUT-mode, and debug constraints using the final controls. Do not reapply stock look defaults over explicit edits.
6. Validate the effective typed parameters after constraints. Resolve and validate the output options before decoding input pixels or rendering. Create the runtime using these effective parameters.

The same layers must apply with or without a preset. A preset's values, parameter-file leaves, and inline assignment leaves must count as explicit. Legacy `--params` must not introduce this new explicit-value protection. Each explicitly supplied neutral-filter axis must retain its value through preparation. With database calibration enabled, preparation must calculate each unprotected axis from the final film, print, and illuminant. Missing database entries must retain the preceding value. Disabling database calibration must skip the lookup. Runtime preparation must not change the user's stored calibration setting to protect an explicit value.
Explicit-axis protection must remain active during runtime construction and recalibration. Unprotected axes must retain the database's f64 precision during calibration. Protected axes must use the supplied runtime control value.

Preview, LUT-mode, and debug constraints may disable an explicitly enabled effect. They must run after all edits. Disabling one of these modes in a parameter source must allow the final configuration to resolve without destructive edits left from an earlier intermediate preparation. Stock defaults and runtime constraints must not be treated as the same precedence layer.
Export options must follow [Export and execution options](#export-and-execution-options).

## Export and execution options

The output path, format, sample depth, compression, JPEG controls, saving color conversion, backend, and RAW loading controls must use existing dedicated flags. They must not be accepted as parameter-file paths or inline assignment paths. A parameter file containing an `[export]` table or an inline assignment such as `export.bit_depth=16` must fail as an unknown parameter group or path. This language must not add a complete job or export-configuration file.

`--output` (`-o`) must select the destination. `--format jpeg|png|tiff|exr`, `--bit-depth 8|16|32`, `--compression zip|none`, `--jpeg-quality`, and `--jpeg-subsampling 444|420` must retain their existing applicability and defaults. The output extension must select the format when `--format` is absent. An explicit format must agree with that extension. Format and writer constraints belong to [photo export](../photo-export/spec.md).

`--saving-color-space` and `--saving-cctf-encoding true|false` must select the final writer-stage color conversion. They must not be aliases for scanner `io.output_color_space` and `io.output_cctf_encoding`, which remain editable runtime controls. Omitting saving color must use ACES2065-1 for EXR and the scanner output space for other formats. Omitting saving encoding must select linear EXR and otherwise inherit scanner output encoding. JPEG and PNG must require encoded saving output. EXR must require linear saving output. These rules must be validated before rendering, including support for the selected color space and conversion.

`--backend cpu|gpu` must retain precedence over `SPEKTRAFILM_BACKEND`. Omitting the flag must retain existing environment/default selection. Output sample depth must not select computation precision. `spektrafilm-f64 --backend cpu` must retain the CPU f64 reference contract; software WGPU and hardware WGPU must remain distinguishable execution evidence.

The following POSIX command combines a look with independent TIFF export options:

```bash
spektrafilm-f64 process input.png --output output.tif \
  --preset classic-kodak-portra-400 \
  --backend cpu --format tiff --bit-depth 16 --compression zip \
  --saving-color-space "ProPhoto RGB" --saving-cctf-encoding true
```

Each file source and inline assignment source must pass syntax, field, type, finite-number, leaf-range, and enum validation. The selected preset must pass complete preset validation before `--set` sources apply. Cross-field validation must run on the composed controls before runtime constraints so constraints cannot conceal an invalid user configuration. The runtime must also validate parameters after applying its constraints.

Errors must identify the offending option or file, field path when available, supplied value when safe to display, and violated rule. Parse errors must include the parser location when available. Unknown paths must not create new fields. Unsupported enum values, profile identities, ranges, and combinations must not fall back to defaults.

A failed configuration must return a nonzero status before rendering or writing image output. The command must not retry with a different preset or baseline.

## Discovery and dry run

`describe --format json` without a field or module selector must retain its existing machine contract. `describe --module PATH --format json` must report editable leaf paths under one runtime group prefix, including nested groups. Each field must report its type, nullability, static default, unit when applicable, available enum values or numeric bounds, and conditions under which it affects output. The result must distinguish static defaults from values derived from stock profiles. Unknown prefixes must fail.

`describe --field PATH --format json` must report one editable leaf as a metadata object with the same field keys used in module discovery. `--field` and `--module` must conflict. A group supplied to `--field` must fail with guidance to use `--module`. A leaf supplied to `--module` must fail with guidance to use `--field`. These selectors must use canonical paths without aliases or prefix expansion.

`describe --format text` without a selector must list the discoverable top-level parameter groups. With `--module`, it must show one record per editable leaf, sorted by canonical path. With `--field`, it must show the selected leaf. A record must show path, type, nullability, default and its source, unit when applicable, enum values or bounds, distinct array-component domains, and effect conditions. Missing units, bounds, and finite enum inventories must not be presented as invented values. `describe` must retain JSON as its default format.

`preset list --format json` must list built-in IDs, names, and film and print references. `preset list --format text` must show those same facts in one readable record per built-in preset. It must not search user libraries. `preset show SELECTOR --format toml|json` must write a complete preset to stdout. `--format` must default to TOML for `preset show` and JSON for `preset list`. Both commands must follow the [CLI data-directory selection policy](../cli-workflow/spec.md#data-directory-selection). `preset show` must validate the selected document and profile references using that data directory. These commands must not inspect a GUI state or render an image. Export must preserve the selected preset ID and must not modify its source file.

`process --dry-run` must accept the same configuration and required input/output paths as a normal process invocation. It must resolve parameters, workflow, and writer options using the same decisions as normal processing. It must check input existence, readability, and supported file kind without decoding input pixels. It must not initialize a compute device, render, or create or truncate any output, including `--raw-out`. It must print one JSON object to stdout with `film_profile`, `print_profile`, `parameters`, `output`, and `backend`. `parameters` must contain the effective runtime controls after constraints. `output` must contain `path`, `format`, `bit_depth`, `color_space`, `cctf_encoding`, `compression`, `jpeg_quality`, and `jpeg_subsampling`; inapplicable options must be `null`. `backend` must report the requested `cpu` or `gpu` policy, or `auto` when normal selection remains deferred. It must not claim compute-adapter availability. Diagnostics must go to stderr. The result must be a resolution report rather than an implicit RenderRecipe or GUI state. It must not claim pixel-dependent values such as metered auto-exposure or successful image decoding. A configuration valid in dry run may still fail on input decoding, resource availability, or rendering.

The dry-run report must also contain `data` and `input` objects. `data` must contain the absolute `path` selected for the invocation and its `source`, which must be `argument`, `environment`, or `automatic`. `input` must contain `kind` as `raw` or `raster` and `raw_loading`. For RAW input, `raw_loading` must contain the selected `white_balance`, `temperature`, `tint`, and `lens_correction`. White-balance values must use the dedicated flag's vocabulary. Absent custom temperature and tint must remain `null`. For raster input, `raw_loading` must be `null` because these controls do not apply. The report must retain all existing keys and meanings. It must not include loading controls in the runtime `parameters` object.

The following command must inspect a composed configuration:

```bash
spektrafilm process input.png --output output.tif \
  --film kodak_portra_400 --paper kodak_portra_endura \
  --set adjustments.toml \
  --dry-run
```

## Usability acceptance

Both executables must discover a group and an individual field without loading profiles, decoding images, or creating a device. Text and JSON views must report the same field facts. Single-field JSON must preserve optional values, array-component domains, enum choices, and effect conditions. Invalid field/group selectors and their conflict must fail before image work. Existing selector-free JSON and module JSON output must preserve their shape and meaning.

Text preset listing must contain the same IDs, names, and profile references as JSON listing. File selectors and preset export validation must retain their existing behavior.

Dry run for custom RAW white balance with temperature `6500` and tint `-5` must expose those loading choices. It must differ from an as-shot report. Raster input must expose inactive RAW loading as `null`. Both must expose the selected data directory and source. None of these scenarios may decode input pixels, initialize a compute device, or write image or raw output. Existing runtime and writer fields must retain their meanings.

## Implementation gap

Discovery currently supports module JSON only, with no single-field selector or text format. Preset listing currently supports JSON only. Dry run omits data-selection source and RAW loading choices. The added syntax and report keys in this document are target behavior awaiting implementation.

## Compatibility

A `process` invocation without `--preset` or `--set` must retain its existing parameter decoding, profile selection, workflow precedence, runtime preparation, and output behavior. Adding `--dry-run` alone must report the parameters that this legacy path would use. Sparse legacy `--params` files must retain their existing field-deserialization defaults. The command must not reinterpret them as sparse parameter files. A legacy `--params` baseline may be followed by one or more `--set` sources.

`render --recipe`, `lut --params`, and GUI child-process protocols must retain their existing contracts. They must not acquire the added process flags implicitly. TOML support for presets and sparse parameter files must not imply TOML support for complete GUI states, legacy `--params`, or RenderRecipe.
