# CLI parameter language

## Contract

The `process` command must support [presets](../presets/spec.md), sparse parameter files, and individual parameter assignments through the unified `--set` option. Both `spektrafilm` and `spektrafilm-f64` must accept the same language. The executable and backend must retain their existing computation precision contracts.

A **parameter override** is a sparse set of explicit edits to runtime controls for one invocation. It may contain look-owned and non-look controls. It must not change profile references or rewrite its source preset. A parameter override is not a preset, GUI state, or render recipe.

This document owns the added command syntax. Preset document fields and carriers belong to the [preset specification](../presets/spec.md). Field names, units, bounds, and conditional effect behavior belong to their parameter model and capability specifications. Help, data-directory selection, command status, and human feedback belong to the [CLI workflow specification](../cli-workflow/spec.md).

## Process syntax

The command must accept these forms:

```text
spektrafilm process INPUT --output OUTPUT --film STOCK [--paper STOCK] [--params FILE] [OPTIONS]
spektrafilm process INPUT --output OUTPUT --preset SELECTOR [OPTIONS]

OPTIONS may include:
  --preset SELECTOR     repeatable; presets apply in appearance order
  --set SOURCE          repeatable file or comma-separated assignments
  --dry-run             resolve and validate; print effective parameters without rendering
  --dry-run-format json|text  dry-run presentation; default: json
  --input-color-space NAME   raster source primaries; discover with describe --field io.input_color_space
  --input-cctf-decoding true|false  whether raster samples require transfer decoding
  --seed UINT32         deterministic stochastic seed; default: 0
```

Existing backend, workflow, RAW, output, and data-directory options must remain available.

### Preset selector

A selector ending in `.toml` or `.json` must designate a file path relative to the current working directory or an absolute path. It must not fall back to a preset name if the file is absent or invalid. A selector without a supported extension must designate an exact preset name. A name must resolve with the group order of the [preset design](../presets/design.md): the user library first, then the built-in presets. The prefixes `user:` and `built-in:` must select one group explicitly and must otherwise be optional. A name must not contain a path separator. A bare name that also exists in another group must resolve in the user library, and the command must report the resolved group and the shadowed name. An unknown name must not trigger a filesystem search or a name-similarity match.

`--preset` must conflict with `--film`, `--paper`, and `--params` when the resolved preset set supplies the matching profile reference; a preset set that supplies none must take the profiles from the existing rules (`--film`, `--paper`, or the film's `target_print`). Without `--preset`, `--film` must remain required to resolve the runtime profiles. The `input` workflow must ignore the resolved film and paper profiles. Without a preset, `--scan-film` or the direct film scan route must select the film as the runtime print profile. Otherwise, a photographic print route must obtain its print profile from `--paper` or the film's `target_print`. Magazine enablement must not affect these profile requirements.
Without a preset, a print target must accept `support=paper` or `support=film` with `stage=printing` when parameter sources are added. Preset print references retain the paper-only restriction in the preset specification.

`--preset` may be repeated; presets apply in appearance order, and their supplied profile references and parameter controls count as explicit. With a look preset, both profile references must come from that preset. `--scan-film` must request `input > film > scan` without replacing the preset's print reference. The preset's workflow default must apply unless `--route` or `--scan-film` supplies an explicit workflow choice. A preset set that supplies no film reference must not select a profile-derived workflow default. `--route` and `--scan-film` must conflict, including when the route is `input > film > scan`. For every public process invocation, the final base route must determine topology: `io.scan_film` must be true exactly for `input > film > scan`, and false for the other supported routes. The existing workflow validator must determine which routes are supported. Magazine enablement must use the existing `magazine_print_color` parameter fields, independently of `--route`, as specified by the [Magazine print contract](../magazine-print-color/spec.md).

`--scan-output direct_scan|positive_scan` must override `scanner.scan_output` after all parameter sources. The shared resolver must enforce the [negative-film scan output contract](../film-chain/spec.md#direct-negative-film-scan-output) before runtime constraints and for the effective parameters. `positive_scan` accepts direct `input > film > scan` and real negative-capture `input > convert-film > scan`; paper, print-paper, and scan-minus-base routes remain direct-only. `scanner.receiver_profile`, `scanner.positive_interpretation`, `scanner.auto_setup`, `scanner.finish`, and `scanner.finish_strength` must use the same parameter source, preset, dry-run, and rendering validation as GUI state. Dry run and rendering must reject the same unsupported scan combinations.

### Input declaration

A **source declaration** states the color space and transfer state of raster samples before simulation. It is not a look or a saving conversion. Every public raster `process` invocation must supply both `io.input_color_space` and `io.input_cctf_decoding` through dedicated input flags, supplied leaves of legacy `--params`, or `--set`. Defaults and look presets must not count as a declaration. A missing component must fail with guidance to declare both components. File extension, bit depth, and embedded ICC metadata must not select a declaration. This requirement also applies to the `input` route and dry run.

`--input-color-space` and `--input-cctf-decoding` must override the matching runtime controls after all `--set` sources, independent of command-line position. Each flag must use the type, vocabulary, and validator of its canonical field. A declaration may mix sources. The report must identify the winning source for each component. Input flags must not modify scanner or saving output settings. Declaring these fields in legacy `--params` must not make other baseline fields protected edits.

RAW input must obtain its declaration from the RAW adapter under the [image-input contract](../image-input/spec.md). Omitted raster input controls must not cause a RAW declaration error. Every explicit input flag or `--set` assignment incompatible with linear ACES2065-1 must fail, even if a later source would replace it. Compatible explicit controls may be supplied. Legacy RAW baseline normalization must remain unchanged. RAW loading flags must retain their existing applicability and validation.

### Seed

`--seed` must accept a decimal integer from `0` through `4294967295`. An omitted seed must select `0`. Both executables must use the same seed domain. Seed must remain outside runtime parameter serialization, presets, and `--set`. It must be applied before the first simulation attempt. Every `--iters` attempt must use the same selected seed. The command must not substitute a time-based or random seed.

The same input pixels, effective parameters, seed, executable precision, and execution route must reproduce the same stochastic output under the owning grain contract. This rule must not promise CPU/GPU or f32/f64 bit identity. Seed validity must be checked in dry run and before rendering.


## `--set` sources

Each `--set SOURCE` occurrence must be exactly one source. A source beginning with a canonical `PATH=` must be an inline assignment source, even when its value ends in `.toml` or `.json`. Otherwise, a source ending in `.toml` or `.json` must be a sparse parameter-file path. An inline source must contain one or more comma-separated `PATH=VALUE` assignments. A source must not mix a file path and assignments. `--set` may be repeated; sources apply in appearance order. A file path must be relative to the working directory or absolute. An assignment source must be passed as one shell argument.

For a file source, the extension must select the parser. Parse failure must not cause a fallback to another parser. Files must not support includes or environment interpolation. A missing or invalid `.toml`/`.json` source must fail as a file error; it must not fall back to a preset name or assignment parsing.

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
  --preset 'Kodak Portra 400' \
  --input-color-space sRGB --input-cctf-decoding true \
  --set film_render.grain.engine=v2 \
  --set 'io.output_color_space=ProPhoto RGB' \
  --set camera.exposure_compensation_ev=0.5
```

The assignment parser must split only at top-level commas, then interpret each value using the shared field description. It must not split commas in double-quoted strings or arrays. The sparse-file carrier rules must remain unchanged.

`--set` sources must be syntactically valid even if a later source replaces one of their leaves. The existing `--film`, `--paper`, `--params`, and `--route` selectors must each occur at most once; `--preset` is repeatable as defined in [Preset selector](#preset-selector).
Dedicated input flags, `--seed`, and `--dry-run-format` must each occur at most once. Repetition must fail at argument parsing rather than silently select a value.

### Editable paths and conditional controls

Paths must use the runtime parameter vocabulary, such as `film_render.grain.v2_amount`. The command must not add shortened paths such as `grain.amount`. It must not infer a grain engine or change `v2_profile` to `custom` from the presence of a control.

`workflow.route` must use `--route`; `io.scan_film` must remain derived from the selected workflow. Neither path may appear in a parameter file or inline assignment. Random seed must remain outside this language. Pipeline-derived `film_render.grain.monochrome` and the legacy non-exported `particle_scale_layers` field must not be editable paths. Other runtime controls must use the shared editable parameter description.

Controls for inactive effects or engines may be stored without affecting output. [Grain V2](../film-grain/v2/spec.md) must retain its named-profile and Custom behavior. V1 RMS granularity and V2 Amount must remain distinct controls. A parameter-file leaf or inline assignment must not change another control to make itself effective.

## Composition

Options form fixed layers: presets are resolved in their appearance order first, then `--set` sources are applied in their appearance order, regardless of their position among unrelated options. A later preset or source replaces the preceding value at a matching leaf, and a later profile reference replaces an earlier one. Inline assignments within one source also apply in their written order.

For an invocation with added parameter sources, resolution must use this order:

1. Select and validate the film and print profiles. Decode the baseline with `RuntimeParams::default()` or the unchanged legacy `--params` decoder. Apply stock-specific look initialization to this baseline with the same precedence as the legacy batch policy. Defer preview, debug, and database-calibration preparation until the final controls are available. Values supplied only by legacy `--params` must retain their legacy relationship to stock initialization and database calibration.
2. Apply every selected preset in appearance order through the shared preset resolver. A look preset replaces the whole look; a section preset edits only the controls of its section, and those values count as explicit from this step on. Retain excluded runtime context and derive the profile workflow default when the preset set supplies a film reference.
3. Apply every `--set` source in appearance order. A file source and an inline assignment source have identical leaf-merge semantics.
4. Apply explicit workflow, scan-output, and input choices. Enforce the [input declaration](#input-declaration) rules. Input flags must count as explicit only for their matching leaves.
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
  --preset 'Kodak Portra 400' \
  --input-color-space sRGB --input-cctf-decoding true \
  --backend cpu --bit-depth 16 --compression zip \
  --saving-color-space "ProPhoto RGB" --saving-cctf-encoding true
```

Each file source and inline assignment source must pass syntax, field, type, finite-number, leaf-range, and enum validation. Each selected preset must pass document validation before `--set` sources apply. Cross-field validation must run on the composed controls before runtime constraints so constraints cannot conceal an invalid user configuration. The runtime must also validate parameters after applying its constraints.

Section composition must defer scan compatibility until explicit workflow and scan-output choices are applied. Active film and print chemistry must use the same control-domain validation in document validation, dry run, and runtime preparation.

Errors must identify the offending option or file, field path when available, supplied value when safe to display, and violated rule. Parse errors must include the parser location when available. Unknown paths must not create new fields; paths naming non-settable runtime fields must identify the out-of-band field and why it is unavailable. Unsupported enum values, profile identities, ranges, and combinations must not fall back to defaults.

A failed configuration must return a nonzero status before rendering or writing image output. The command must not retry with a different preset or baseline.

## Discovery and dry run

`describe --format json` without a field or module selector must retain its existing machine contract. `describe --module PATH --format json` must report editable leaf paths under one runtime group prefix, including nested groups. Each field must report its type, nullability, static default, unit when applicable, available enum values or numeric bounds, and conditions under which it affects output. The result must distinguish static defaults from values derived from stock profiles. Unknown prefixes must fail.

`describe --field PATH --format json` must report one editable leaf as a metadata object with the same field keys used in module discovery. `--field` and `--module` must conflict. A group supplied to `--field` must fail with guidance to use `--module`. A leaf supplied to `--module` must fail with guidance to use `--field`. These selectors must use canonical paths without aliases or prefix expansion.

`describe --format text` without a selector must list the discoverable top-level parameter groups followed by non-settable runtime fields and their reasons. With `--module`, it must show one record per editable leaf, sorted by canonical path. With `--field`, it must show the selected leaf. A record must show path, type, nullability, default and its source, unit when applicable, enum values or bounds, distinct array-component domains, and effect conditions. Missing units, bounds, and finite enum inventories must not be presented as invented values. `describe` must retain JSON as its default format.

`preset list --format json` must list preset names, the kind and section of a section preset or the film and print references of a look preset, and the group of each entry. `preset list --source built-in|user|all` must select the listed groups and must default to `built-in`. `preset list --format text` must show those same facts in one readable record per preset. Listing must report every unreadable or conflicting library document with its path and must not hide the remaining entries. Listing must not search directories other than the library directory. `preset show SELECTOR --format toml|json` must write the selected document to stdout. `--format` must default to TOML for `preset show` and JSON for `preset list`. Both commands must follow the [CLI data-directory selection policy](../cli-workflow/spec.md#data-directory-selection), and listing library entries must not require a data directory. `preset show` must validate the selected document and profile references using that data directory. These commands must not inspect a GUI state or render an image. Export must preserve the selected preset name and must not modify its source file.

`process --dry-run` must accept the same configuration and required input/output paths as a normal process invocation. It must resolve parameters, workflow, and writer options using the same decisions as normal processing. It must check input existence, readability, and supported file kind without decoding input pixels. It must not initialize a compute device, render, or create or truncate any output, including `--raw-out`. `--dry-run-format json|text` must require `--dry-run` and default to `json`. Terminal detection must not change the format. JSON must contain one object on stdout with the existing `film_profile`, `print_profile`, `parameters`, `output`, and `backend` keys. `parameters` must contain effective runtime controls after constraints. `output` must retain `path`, `format`, `bit_depth`, `color_space`, `cctf_encoding`, `compression`, `jpeg_quality`, and `jpeg_subsampling`; inapplicable options must be `null`. `backend` must report the requested `cpu` or `gpu` policy, or `auto` when selection remains deferred. Diagnostics must go to stderr. The result must be a resolution report, not an implicit RenderRecipe or GUI state.

The dry-run report must also contain `data` and `input` objects. `data` must contain the absolute `path` selected for the invocation and its `source`, which must be `argument`, `environment`, or `automatic`. `input` must contain `kind` as `raw` or `raster` and `raw_loading`. For RAW input, `raw_loading` must contain the selected `white_balance`, `temperature`, `tint`, and `lens_correction`. White-balance values must use the dedicated flag's vocabulary. Absent custom temperature and tint must remain `null`. For raster input, `raw_loading` must be `null` because these controls do not apply. The report must retain all existing keys and meanings. It must not include loading controls in the runtime `parameters` object.

The JSON report must add `seed` as the selected integer and `input.declaration` as an object. The declaration must contain `color_space`, `cctf_decoding`, and `sources`. `sources` must map each of those two component names to a record with `kind`, `option`, `index`, and `path`. `kind` must be `argument`, `params`, `set`, or `raw`. `option` must name the originating flag, or be `null` for the RAW adapter. `index` must be the one-based `--set` occurrence, or `null` for other sources. `path` must identify a file source as an absolute path, or be `null` for non-file sources. These paths belong to the local resolution report, not telemetry. RAW must report its adapter as the source of both components after compatibility checks.

The report must add `edits` as an array of winning `--set` leaves sorted by canonical path. Each entry must contain `path`, `requested`, `effective`, `source`, `status`, and `reason`. `source` must use the declaration source record shape. Values must be typed JSON values. `status` must be `active`, `inactive`, `constrained`, or `overridden`. `active` means the edit remains effective and its known effect conditions hold. `inactive` means a known effect condition prevents use. `constrained` means runtime preparation changed the requested value. `overridden` means a dedicated flag replaced the winning edit. Classification precedence must be `overridden`, then `constrained`, then `inactive`, then `active`. `reason` must be `null` for active edits and identify the responsible condition, constraint, or option otherwise. Later `--set` replacements must not leave earlier duplicate entries. A full per-field provenance history must not be added. Active status must not claim a measured pixel change.

Text dry run must show the same resolved input declaration and sources, profile identities, route, scan-output mode, seed, requested backend, writer options, and edit feedback as JSON. It must show data selection and RAW loading choices. Effective exposure, film format, grain engine, and grain controls must use their canonical units and conditions. Text layout must not be a machine contract.

Both dry-run formats must report scanner receiver provenance and calibration status. Approximate receiver profiles must be labeled approximate. Scanner finish strength must be reported as inactive when the selected finish is neutral.

Both formats must state that pixel decoding, compute-device availability, metered exposure, and final pixel geometry remain unchecked. JSON must add `unchecked` with the stable values `pixel_decode`, `compute_device`, `metered_exposure`, and `output_geometry`. Dry run must not claim these outcomes. A configuration valid in dry run may still fail on decoding, resource availability, rendering, or publication.

Normal processing and dry run must warn on stderr for winning `--set` edits with non-active status. Each warning must name the canonical path, source, and reason. Known corrective choices must be explained without applying them. Unedited inactive defaults and preset controls must not produce these warnings. Feedback must not enable effects, select a grain engine, change a V2 profile to `custom`, or turn a valid stored inactive value into a configuration error.

Field discovery must explain input versus scanner versus saving color, exposure compensation in EV versus print exposure, and film format as the source long-edge dimension in millimeters. Crop discovery must state that both `io.crop_size` components are fractions of the uncropped source long edge, not of independent width and height. V2 Size discovery must identify its `custom` profile condition. V2 Amount must remain applicable to named and custom V2 profiles. Both controls must remain distinct from V1 RMS granularity. Text and selected-field/module JSON must use the same descriptions and effect conditions. Selector-free JSON must retain its existing render contract.

The file `adjustments.toml` used below must contain:

```toml
[camera]
auto_exposure = false
exposure_compensation_ev = 0.5
film_format_mm = 36.0

[film_render.grain]
active = true
engine = "v2"
v2_profile = "custom"
v2_size = 12.0
v2_amount = 25.0
```

The following command must inspect a composed configuration:

```bash
spektrafilm process input.png --output output.tif \
  --film kodak_portra_400 --paper kodak_portra_endura \
  --input-color-space sRGB --input-cctf-decoding true \
  --seed 42 \
  --set adjustments.toml \
  --dry-run
```

## Usability acceptance

Both executables must discover a group and an individual field without loading profiles, decoding images, or creating a device. Text and JSON views must report the same field facts. Single-field JSON must preserve optional values, array-component domains, enum choices, and effect conditions. Invalid field/group selectors and their conflict must fail before image work. Existing selector-free JSON and module JSON output must preserve their shape and meaning.

Text preset listing must contain the same names, groups, kinds, and profile references as JSON listing. File selectors and preset export validation must retain their existing behavior.

Dry run for custom RAW white balance with temperature `6500` and tint `-5` must expose those loading choices. It must differ from an as-shot report. Raster input must expose inactive RAW loading as `null`. Both must expose the selected data directory and source. None of these scenarios may decode input pixels, initialize a compute device, or write image or raw output. Existing runtime and writer fields must retain their meanings.

## Compatibility

Legacy `--params` must retain its existing decoding and relationship to stock initialization and database calibration. A sparse legacy file must not be reinterpreted as a sparse parameter override. A legacy baseline may be followed by `--set` sources. The declaration requirement and route normalization are intentional changes for all public `process` invocations. The absence of `--preset` or `--set` must not bypass them. Seed and report presentation alone must not select a different numerical preparation policy. For the same effective parameters, routing, seed, and backend, these interface changes must not alter numerical computation.

`render --recipe`, `lut --params`, and GUI child-process protocols must retain their existing contracts. They must not acquire the added process flags implicitly. TOML support for presets and sparse parameter files must not imply TOML support for complete GUI states, legacy `--params`, or RenderRecipe.

## Image-processing acceptance

1. Run both real executables with encoded sRGB PNG and JPEG input. Omission of either declaration component must fail without output. Dedicated flags and equivalent TOML, JSON, and inline declarations must resolve to the same input controls and produce equal raw simulation buffers within the same executable/backend. Explicit `false` must count as supplied. A look preset must not satisfy a missing declaration.
2. Run linear prepared TIFF and OpenEXR with explicit declarations. Retained ICC bytes and an integer or float container must not change those declarations. Run a real RAW fixture through all supported white-balance modes. Compatible declarations must preserve RAW pixels; incompatible flags and early incompatible edits must fail before decode. Use RAW provenance from existing acceptance fixtures.
3. Exercise fixed layers with reordered unrelated flags, repeated presets, repeated files, duplicate inline edits, mixed declaration sources, and dedicated input overrides. An explicit value equal to its default must remain explicit. Legacy baseline leaves must not acquire new stock/calibration protection. Invalid earlier sources must still fail.
4. Reject `--route` with `--scan-film` at argument parsing in dry run and normal processing. Resolve each supported route with and without a no-op `--set` source. Route, `io.scan_film`, and runtime print-profile selection must agree. Exercise positive-scan compatibility through the real CLI.
5. Render stochastic V1, V2, and V3 with seeds `0` and `4294967295` under each engine's admitted configuration. Repeating a render with the same seed must match its raw output in the same executable/backend. Exercise seed `42` through `--iters`. Verify that `42` and another seed change a known stochastic fixture under the owning grain contract. Negative, fractional, malformed, and overflowing seeds must fail. This evidence must not be presented as cross-backend numerical identity.
6. Compare JSON and text dry run for raster, RAW, preview constraints, dedicated overrides, and V2 custom versus named-profile controls. Verify typed requested/effective values and winning sources. Only winning non-active explicit edits may warn. Warning output must leave JSON parseable. Default dry run must remain JSON when redirected or attached to a terminal. Both formats must preserve image and raw-output sentinels and report unchecked outcomes.
7. Run field discovery without profiles or a device. Verify the long-edge crop convention, input transfer meaning, and grain conditions. Exercise a crop on a non-square input through normal processing; verify dimensions against the existing geometry contract.
8. Run help and parser scenarios on separately built default-feature `spektrafilm` and `precision-f64` `spektrafilm-f64`. Exercise normal CPU image output with stdout empty. Run explicit WGPU image processing where a usable adapter is available, and distinguish hardware from software execution. Apply [photo-publication acceptance](../photo-export/spec.md#publication-acceptance) to `process`.

Acceptance must identify the source commit, both binary identities and build configurations, input provenance, complete invocation, exit status, stdout/stderr, and observed files or pixel comparisons. Specs and parser-only results must not count as image-processing acceptance. Unavailable RAW or GPU environments must be reported as unverified, not passed.

## Implementation status
The current `process` implementation provides the dedicated raster input declarations, deterministic seed selection, text and JSON dry-run reports, input provenance, winning-edit feedback, shared route resolution, and staged image publication described above. The acceptance section remains the source of required behavioral evidence; this specification alone does not establish that every scenario has been exercised in every supported environment.

Native writer behavior, including platform-specific failure and cancellation scenarios, remains owned by [photo export](../photo-export/spec.md#publication). RAW and GPU scenarios still require environment-specific evidence when those dependencies or adapters are unavailable.

