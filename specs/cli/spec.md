# CLI

This document is the unified CLI specification. The parameter and workflow sections below form one contract; links between them use local anchors.

## Parameter contract

The `process` command must support [presets](../presets/spec.md), sparse parameter files, and individual parameter assignments through the unified `--set` option. Both `spektrafilm` and `spektrafilm-f64` must accept the same language. Backend policy and computation precision follow [GPU execution](../gpu-execution/spec.md#backend-selection).

A **parameter override** is a sparse set of explicit edits to runtime controls for one invocation. It may contain look-owned and non-look controls. It must not change profile references or rewrite its source preset. A parameter override is not a preset, GUI state, or render recipe.

The parameter contract owns the added command syntax. Preset document fields and carriers belong to the [preset specification](../presets/spec.md). Field names, units, bounds, and conditional effect behavior belong to their parameter model and capability specifications. Help, data-directory selection, command status, and human feedback belong to the [workflow contract](#workflow-contract).

`film_channel_swap` and `print_channel_swap` are profile-data diagnostics. They reorder which density row feeds each RGB channel and exist to diagnose a profile whose channel assignment is wrong. They are not photographic controls and must not appear in the GUI. The CLI `--film-channel-swap` and `--print-channel-swap` flags are the only public surface. A swap value must be a permutation of `[0, 1, 2]`; the command must reject a non-permutation before rendering.

## Process syntax

The command must accept these forms:

```text
spektrafilm process INPUT --output OUTPUT --film STOCK [--paper STOCK] [--params FILE] [OPTIONS]
spektrafilm process INPUT --output OUTPUT --preset SELECTOR [OPTIONS]

OPTIONS may include:
  --preset SELECTOR     repeatable; presets apply in appearance order
  --set SOURCE          repeatable file or comma-separated assignments
  --dry-run             resolve and validate; print effective parameters without rendering; --output may be omitted
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

`--preset` may be repeated; presets apply in appearance order, and their supplied profile references and parameter controls count as explicit. With a look preset, both profile references must come from that preset. `--scan-film` must request `input > film > scan` without replacing the preset's print reference. The preset's workflow default must apply unless `--route` or `--scan-film` supplies an explicit workflow choice. A preset set that supplies no film reference must not select a profile-derived workflow default. `--route` and `--scan-film` must conflict, including when the route is `input > film > scan`. For every public process invocation, the final base route must determine topology: `io.scan_film` must be true exactly for `input > film > scan`, and false for the other supported routes. The existing workflow validator must determine which routes are supported. Magazine enablement must use the existing `magazine_print_color` parameter fields, independently of `--route`, as specified by the [Magazine finishing contract](../magazine-finishing/spec.md).

`--scan-output direct_scan|negative_to_positive` must override `scanner.scan_output` after all parameter sources. The shared resolver must enforce the [negative-film scan output contract](../film-chain/spec.md#direct-negative-film-scan-output) before runtime constraints and for the effective parameters. `negative_to_positive` accepts direct `input > film > scan` and real negative-capture `input > convert-film > scan`; paper, print-paper, and scan-minus-base routes remain direct-only. `scanner.receiver_profile`, `scanner.negative_conversion_method`, `scanner.auto_setup`, `scanner.finish`, and `scanner.finish_strength` must use the same parameter source, preset, dry-run, and rendering validation as GUI state. Dry run and rendering must reject the same unsupported scan combinations.

Digital Reference selection and manual reference controls use `--set` under the [Digital Reference contract](../film-chain/digital-reference/spec.md). The shared resolver must reject incompatible receiver, reconstruction controls, automatic setup, point windows, profiles, and routes before either dry run or rendering.

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
engine = "procedural"
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
film_render.grain.engine=procedural,film_render.grain.v2_profile=custom,film_render.grain.v2_amount=25,camera.auto_exposure=false
```

For a POSIX shell, quote a source containing brackets, spaces, or shell-significant characters:

```bash
spektrafilm process input.png --output output.tif \
  --preset 'Kodak Portra 400' \
  --input-color-space sRGB --input-cctf-decoding true \
  --set film_render.grain.engine=procedural \
  --set 'io.output_color_space=ProPhoto RGB' \
  --set camera.exposure_compensation_ev=0.5
```

The assignment parser must split only at top-level commas, then interpret each value using the shared field description. It must not split commas in double-quoted strings or arrays. The sparse-file carrier rules must remain unchanged.

`--set` sources must be syntactically valid even if a later source replaces one of their leaves. The existing `--film`, `--paper`, `--params`, and `--route` selectors must each occur at most once; `--preset` is repeatable as defined in [Preset selector](#preset-selector).
Dedicated input flags, `--seed`, and `--dry-run-format` must each occur at most once. Repetition must fail at argument parsing rather than silently select a value.

### Editable paths and conditional controls

Paths must use the runtime parameter vocabulary, such as `film_render.grain.v2_amount`. The command must not add shortened paths such as `grain.amount`. It must not infer a grain engine or change `v2_profile` to `custom` from the presence of a control.

`workflow.route` must use `--route`; `io.scan_film` must remain derived from the selected workflow. Neither path may appear in a parameter file or inline assignment. Random seed must remain outside this language. Pipeline-derived `film_render.grain.monochrome` and the legacy non-exported `particle_scale_layers` field must not be editable paths. Other runtime controls must use the shared editable parameter description.

Controls for inactive effects or engines may be stored without affecting output. [Procedural grain](../film-grain/v2/spec.md) must retain its named-profile and Custom behavior. Spektra RMS granularity and Procedural Amount must remain distinct controls. A parameter-file leaf or inline assignment must not change another control to make itself effective.
The [Output mix and material controls contract](../output-mix/spec.md) defines use of existing preflash and chemistry controls and owns `output_mix`. Overall effect amount uses the existing canonical assignment `--set output_mix.amount=0.5`, without an alias or dedicated process flag. `output_mix.amount` is a non-nullable finite f64 in `[0,1]`, with default 1. Current complete parameter documents, sparse sources, recipes, and GUI state must share typed value validators. Complete-document compatibility may supply an absent output-mix default; sparse omission remains no edit.

Material-control edits must retain existing route and profile capability conditions. Preflash affects exposure only on admitted print-development routes, including printing recovered negative density. Film chemistry must not redevelop real captured negatives or run on Finished RGB routes. Developer exhaustion must remain a model-specific development control; unsupported profiles must not acquire a fitted model or substitute effect. Inactive values remain stored. Edits must not change a route or silently activate chemistry. Scanner points, inverse reconstruction, and calibration retain their existing meanings. There are no new generic levels, saturation, color-density, or split-toning controls.

`io.output_gamut_compress.lightness_compression` is retired from the runtime model, editable paths, discovery, GUI, and execution. Current complete documents, sparse files, and inline assignments must reject that exact leaf even when null or a later layer would replace it. A versioned legacy complete-document migration may allow absence or null only; every explicit numeric triplet must fail even if its old gamut algorithm ignored it. The diagnostic must name `io.output_gamut_compress.lightness_compression`, state that no automatic replacement exists, and require explicit removal without suggesting a deleted response group. New writers must never emit it. Existing gamut algorithms otherwise stay unchanged. The unshipped `film_render.response`, `print_render.response`, `enlarger.c_filter_shift`, `reproduction.*`, and `io.finished_rgb_output_processing` paths must fail normal unknown-field validation without aliases, defaults, or migration, including in draft version 3 presets. Presets must reject the unshipped `output response` section.


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

`--backend cpu|gpu` must retain precedence over `SPEKTRAFILM_BACKEND`. Omitting the flag must follow the [GPU-first selection policy](../gpu-execution/spec.md#backend-selection) while retaining explicit environment selection. Output sample depth must not select computation precision. `spektrafilm-f64 --backend cpu` must retain the CPU f64 reference contract; software WGPU and hardware WGPU must remain distinguishable execution evidence.

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

`preset list --format json` must list preset names, the kind and section of a section preset or the film and print references of a look preset, and the group of each entry. `preset list --source built-in|user|all` must select the listed groups and must default to `built-in`. `preset list --format text` must show those same facts in one readable record per preset. Listing must report every unreadable or conflicting library document with its path and must not hide the remaining entries. Listing must not search directories other than the library directory. `preset show SELECTOR --format toml|json` must write the selected document to stdout. `--format` must default to TOML for `preset show` and JSON for `preset list`. Both commands must follow the [CLI data-directory selection policy](#data-directory-selection), and listing library entries must not require a data directory. `preset show` must validate the selected document and profile references using that data directory. These commands must not inspect a GUI state or render an image. Export must preserve the selected preset name and must not modify its source file.

Export and update of an older whole-look preset must migrate its look-owned controls to version 3 with `output_mix.amount = 1`. Older section presets must retain sparse omission and must not gain output-mix or other leaves. User-library import must preserve the older carrier until explicit export or update writes version 3. Discovery must not change a stored document merely by listing it. Preset application and export must preserve the caller's other `io` settings; no preset owns output gamut processing.

`process --dry-run` must accept the same configuration and input paths as a normal process invocation. It must resolve parameters, workflow, and writer options using the same decisions as normal processing. `--output` must be optional with `--dry-run`. Normal processing must still require `--output`. Without `--output`, `output` must remain an object whose `path` is `null`. Its `format` must be `null` unless `--format` was supplied. Writer options independent of the format must use their normal defaults or explicit flags. Format-dependent defaults must remain `null` when the format is unresolved. Explicit codec options require a resolved format; without a destination they must require `--format` and report that corrective action. It must check input existence, readability, and supported file kind without decoding input pixels. It must not initialize a compute device, render, or create or truncate any output, including `--raw-out`. `--dry-run-format json|text` must require `--dry-run` and default to `json`. Terminal detection must not change the format. JSON must contain one object on stdout with the existing `film_profile`, `print_profile`, `parameters`, `output`, and `backend` keys. `parameters` must contain effective runtime controls after constraints. `output` must retain `path`, `format`, `bit_depth`, `color_space`, `cctf_encoding`, `compression`, `jpeg_quality`, and `jpeg_subsampling`; inapplicable options must be `null`. `backend` must report the requested `cpu` or `gpu` policy, or `auto` when selection remains deferred. Diagnostics must go to stderr. The result must be a resolution report, not an implicit RenderRecipe or GUI state.

When no output format is resolved, `unchecked` must include `output_format`. An explicit `--format` must resolve and validate writer options without requiring an output path.

The dry-run report must also contain `data` and `input` objects. `data` must contain the absolute `path` selected for the invocation and its `source`, which must be `argument`, `environment`, or `automatic`. `input` must contain `kind` as `raw` or `raster` and `raw_loading`. For RAW input, `raw_loading` must contain the selected `white_balance`, `temperature`, `tint`, and `lens_correction`. White-balance values must use the dedicated flag's vocabulary. Absent custom temperature and tint must remain `null`. For raster input, `raw_loading` must be `null` because these controls do not apply. The report must retain all existing keys and meanings. It must not include loading controls in the runtime `parameters` object.

The JSON report must add `seed` as the selected integer and `input.declaration` as an object. The declaration must contain `color_space`, `cctf_decoding`, and `sources`. `sources` must map each of those two component names to a record with `kind`, `option`, `index`, and `path`. `kind` must be `argument`, `params`, `set`, or `raw`. `option` must name the originating flag, or be `null` for the RAW adapter. `index` must be the one-based `--set` occurrence, or `null` for other sources. `path` must identify a file source as an absolute path, or be `null` for non-file sources. These paths belong to the local resolution report, not telemetry. RAW must report its adapter as the source of both components after compatibility checks.

The report must add `edits` as an array of winning `--set` leaves sorted by canonical path. Each entry must contain `path`, `requested`, `effective`, `source`, `status`, and `reason`. `source` must use the declaration source record shape. Values must be typed JSON values. `status` must be `active`, `inactive`, `constrained`, or `overridden`. `active` means the edit remains effective and its known effect conditions hold. `inactive` means a known effect condition prevents use. `constrained` means runtime preparation changed the requested value. `overridden` means a dedicated flag replaced the winning edit. Classification precedence must be `overridden`, then `constrained`, then `inactive`, then `active`. `reason` must be `null` for active edits and identify the responsible condition, constraint, or option otherwise. Later `--set` replacements must not leave earlier duplicate entries. A full per-field provenance history must not be added. Active status must not claim a measured pixel change.

Text dry run must show the same resolved input declaration and sources, profile identities, route, scan-output mode, seed, requested backend, writer options, and edit feedback as JSON. It must show data selection and RAW loading choices. Effective exposure, film format, grain engine, and grain controls must use their canonical units and conditions. Text layout must not be a machine contract.

Both dry-run formats must report scanner receiver provenance and calibration status. Approximate receiver profiles must be labeled approximate. Scanner finish strength must be reported as inactive when the selected finish is neutral.

Both formats must state that pixel decoding, compute-device availability, metered exposure, and final pixel geometry remain unchecked. JSON must add `unchecked` with the stable values `pixel_decode`, `compute_device`, `metered_exposure`, and `output_geometry`. Dry run must not claim these outcomes. A configuration valid in dry run may still fail on decoding, resource availability, rendering, or publication.

Normal processing and dry run must warn on stderr for winning `--set` edits with non-active status. Each warning must name the canonical path, source, and reason. Known corrective choices must be explained without applying them. Unedited inactive defaults and preset controls must not produce these warnings. Feedback must not enable effects, select a grain engine, change a Procedural profile to `custom`, or turn a valid stored inactive value into a configuration error.

Field discovery must explain input versus scanner versus saving color, exposure compensation in EV versus print exposure, and film format as the source long-edge dimension in millimeters. Crop discovery must state that both `io.crop_size` components are fractions of the uncropped source long edge, not of independent width and height. Procedural Size discovery must identify its `custom` profile condition. Procedural Amount must remain applicable to named and custom Procedural profiles. Both controls must remain distinct from Spektra RMS granularity. Text and selected-field/module JSON must use the same descriptions and effect conditions. Selector-free JSON must retain its existing render contract.
Discovery must distinguish material density-curve development controls, optical M/Y exposure filtration and calibrated neutral filters, preflash, reference print normalization, existing scanner interpretation points, output gamut processing, and global source mixing. It must not advertise retired lightness or unshipped response, Cyan-shift, saturation, color-density, or split-toning paths. Existing Cyan neutral calibration remains a calibration control rather than a new creative Cyan shift.
Scanner lens blur discovery must identify linear readout optics before interpretation and gamut processing. Unsharp mask must identify digital scan sharpening. `print_render.glare` must identify print viewing/readout stray light, not preflash or density formation. Grain descriptions must distinguish material grain from [Procedural post-scan appearance](../film-grain/v2/spec.md), including Film Resolution's appearance-only meaning. These descriptions must not rename canonical paths or introduce aliases. Dry run must report the selected model/version and optical route without claiming that the pixels or kernel were exercised.

Selected-field and module discovery must report existing typed defaults, bounds, and units for `enlarger.preflash_exposure`, `enlarger.preflash_y_filter_shift`, and `enlarger.preflash_m_filter_shift`. Preflash exposure is a linear exposure multiplier, not EV. Its zero default disables flash while retaining the independent filter settings. Both filter shifts use the existing Kodak CC units and zero defaults around the calibrated neutral pack. Their descriptions must identify the admitted print-development condition and inactivity when flash exposure is zero or the route has no print development. Preflash descriptions must state the photographic-paper highlight use and possible loss of paper white, midtone changes, and color changes defined in [Preflash](../output-mix/spec.md#preflash). They must not promise an independent onset, fixed white or midgray, recovered information, or identical behavior for printing film and photographic paper.

Developer-exhaustion discovery must report its existing dimensionless `[0,1]` domain, zero default, chemistry enable condition, actual-development route condition, and required supported fitted material model. Discovery without profiles must state that model requirement rather than claim a selected profile supports it. Dry run with resolved profiles must identify unsupported or inactive chemistry without clearing stored values or fabricating support. Descriptions must not promise universal highlight compression or equivalence to a digital compressor. [Output mix](../output-mix/spec.md#output-mix) discovery must report f64, non-nullability, default 1, and inclusive `[0,1]` bounds. Amount 1 is the complete result after Magazine; amount 0 is the prepared, same-geometry input before simulated camera exposure and processing. A captured-negative source remains negative at 0 and must not be automatically inverted. Mixing occurs in common linear output RGB before save conversion and applies to all workflows. Final output below 1 requires a declared source branch; diagnostic or segmented paths without one must reject it. Amount 1 requires no source copy. Intermediate taps remain unaffected. Dry-run feedback must apply these conditions without claiming that pixels were exercised.


The file `adjustments.toml` used below must contain:

```toml
[camera]
auto_exposure = false
exposure_compensation_ev = 0.5
film_format_mm = 36.0

[film_render.grain]
active = true
engine = "procedural"
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

Material-control and output-mix discovery acceptance must compare text and selected-field/module JSON against their canonical typed defaults, bounds, units, and effect conditions. Dry run must retain inactive preflash and chemistry values and explain missing print development, missing Film development, disabled chemistry, and unsupported fitted models when applicable. Equivalent sparse TOML, JSON, and inline output-mix edits must resolve to the same finite f64 amount without changing material activation or routes. Discovery and dry-run descriptions must identify both source endpoints, including negative capture, and reject a known missing-source final-output configuration below 1.

Persistence acceptance must exercise version 1 and version 2 whole-look migration, sparse Film and Print section migration, and version 3 capture/export/reload. Whole looks gain amount 1; sparse sections gain no leaves; `io` settings outside existing scanner-output normalization remain caller-owned. Reject unshipped response groups, Cyan-shift paths, and the output-response section through normal schema validation without aliases or migration, including draft version 3 documents. For complete legacy states, recipes, and baselines, exercise absent and null retired lightness leaves and explicit numeric triplets with each old gamut algorithm. Absence/null may migrate; every numeric triplet must fail atomically with the exact retired-path diagnostic and no automatic replacement. Current complete documents, sparse sources, and inline nulls must fail. Failed migration must preserve source, active state, and output sentinels; new writers must omit the retired field.

Dry run for custom RAW white balance with temperature `6500` and tint `-5` must expose those loading choices. It must differ from an as-shot report. Raster input must expose inactive RAW loading as `null`. Both must expose the selected data directory and source. None of these scenarios may decode input pixels, initialize a compute device, or write image or raw output. Existing runtime and writer fields must retain their meanings.

## Compatibility

Legacy `--params` must retain its existing decoding and relationship to stock initialization and database calibration, except for the explicit [retired output-lightness migration](#editable-paths-and-conditional-controls). A sparse legacy file must not be reinterpreted as a sparse parameter override. A legacy baseline may be followed by `--set` sources. The declaration requirement and route normalization are intentional changes for all public `process` invocations. The absence of `--preset` or `--set` must not bypass them. Seed and report presentation alone must not select a different numerical preparation policy. For the same effective parameters, routing, seed, and backend, these interface changes must not alter numerical computation.

`render --recipe`, `lut --params`, and GUI child-process protocols must retain their existing contracts except for the same versioned complete-document output-lightness retirement. They must not acquire the added process flags implicitly. A rejected migration or configuration must preserve the source file, active state, and existing image output. TOML support for presets and sparse parameter files must not imply TOML support for complete GUI states, legacy `--params`, or RenderRecipe.

## Image-processing acceptance

1. Run both real executables with encoded sRGB PNG and JPEG input. Omission of either declaration component must fail without output. Dedicated flags and equivalent TOML, JSON, and inline declarations must resolve to the same input controls and produce equal raw simulation buffers within the same executable/backend. Explicit `false` must count as supplied. A look preset must not satisfy a missing declaration.
2. Run linear prepared TIFF and OpenEXR with explicit declarations. Retained ICC bytes and an integer or float container must not change those declarations. Run a real RAW fixture through all supported white-balance modes. Compatible declarations must preserve RAW pixels; incompatible flags and early incompatible edits must fail before decode. Use RAW provenance from existing acceptance fixtures.
3. Exercise fixed layers with reordered unrelated flags, repeated presets, repeated files, duplicate inline edits, mixed declaration sources, and dedicated input overrides. An explicit value equal to its default must remain explicit. Legacy baseline leaves must not acquire new stock/calibration protection. Invalid earlier sources must still fail.
4. Reject `--route` with `--scan-film` at argument parsing in dry run and normal processing. Resolve each supported route with and without a no-op `--set` source. Route, `io.scan_film`, and runtime print-profile selection must agree. Exercise negative-conversion compatibility through the real CLI.
5. Render stochastic Spektra, Procedural, Dye, and Fotufilm Film with seeds `0` and `4294967295` under each engine's admitted configuration. Repeating a render with the same seed must match its raw output in the same executable/backend. Exercise seed `42` through `--iters`. Verify that `42` and another seed change a known stochastic fixture under the owning grain contract. Negative, fractional, malformed, and overflowing seeds must fail. This evidence must not be presented as cross-backend numerical identity.
6. Compare JSON and text dry run for raster, RAW, preview constraints, dedicated overrides, and Procedural custom versus named-profile controls. Verify typed requested/effective values and winning sources. A dry run without `--output` must resolve the same parameters, report `output.path` and `output.format` as `null` unless `--format` was supplied, and create no file. Only winning non-active explicit edits may warn. Warning output must leave JSON parseable. Default dry run must remain JSON when redirected or attached to a terminal. Both formats must preserve image and raw-output sentinels and report unchecked outcomes.
7. Run field discovery without profiles or a device. Verify the long-edge crop convention, input transfer meaning, and grain conditions. Exercise a crop on a non-square input through normal processing; verify dimensions against the existing geometry contract.
8. Run help and parser scenarios on separately built default-feature `spektrafilm` and `precision-f64` `spektrafilm-f64`. Exercise normal CPU image output with stdout empty. Run explicit WGPU image processing where a usable adapter is available, and distinguish hardware from software execution. Apply [photo-publication acceptance](../photo-export/spec.md#publication-acceptance) to `process`.

Acceptance must identify the source commit, both binary identities and build configurations, input provenance, complete invocation, exit status, stdout/stderr, and observed files or pixel comparisons. Specs and parser-only results must not count as image-processing acceptance. Unavailable RAW or GPU environments must be reported as unverified, not passed.

## Implementation status
The current `process` implementation provides the dedicated raster input declarations, deterministic seed selection, text and JSON dry-run reports, input provenance, winning-edit feedback, shared route resolution, and staged image publication described above. The acceptance section remains the source of required behavioral evidence; this specification alone does not establish that every scenario has been exercised in every supported environment.

Native writer behavior, including platform-specific failure and cancellation scenarios, remains owned by [photo export](../photo-export/spec.md#publication). RAW and GPU scenarios still require environment-specific evidence when those dependencies or adapters are unavailable.

## Workflow contract

Both `spektrafilm` and `spektrafilm-f64` must expose the same command syntax and discovery behavior. Their computation precision contracts must remain distinct. The [parameter contract](#parameter-contract) owns preset selection, parameter overrides, field discovery, and dry run.

This capability owns help, version identification, data-directory selection, command status, and human feedback. It must not add shell completion, interactive prompts, a TUI, or a replacement argument parser. LUT publication and quality rules remain owned by [LUT delivery](../lut-delivery/spec.md).

## Help and identity

Every public command and option must have a help description. Descriptions must identify the purpose, expected value form, and applicability of the option. Dimensional numeric options must state their units. Options with finite choices must show the choices from their authoritative definition. A field with an open vocabulary must explain how to discover or supply its values.

`process` help must group options by input and RAW loading, look and workflow, output, and execution and diagnostics. LUT build help must distinguish bundle configuration, delivery, and quality assessment. Long help must include a basic preset process command, a parameter override command, and the relevant discovery commands. It must distinguish scanner color controls from saving color conversion. The preset list help must state the default source selection, the preset library directory, and the group order that resolves a bare preset name.

Signed numeric options must accept a negative number as the following argument. This rule includes `process --raw-tint -5` and `lut build OUT --exposure-ev -1`. Accepting this argument form must not relax numeric domains or cross-field validation.

`lut build` must retain its positional output-directory argument `OUT`. Its help must describe that argument as the parent of the generated bundle directory. README and packaged CLI usage examples must use the same form. `--params` help must describe only its runtime-parameter snapshot input.

Both executables must support `--version`. Version output must identify the executable name, application version, and native CPU computation precision. It must not claim that a GPU was selected or exercised. Version and help must work without a data directory, input file, image decode, or compute device.

## Data-directory selection

An explicit `--data-dir` must select the data directory. It must take precedence over `SPEKTRAFILM_DATA_DIR`. A nonempty `SPEKTRAFILM_DATA_DIR` must select the directory when the flag is absent. An empty environment value must fail with a diagnostic that identifies the variable. Relative paths must resolve against the invocation's working directory.

A directory selected by a flag or environment variable must exist and be a directory. An invalid explicit selection must fail. It must not fall back to bundled data or the working directory. Supplying the literal path `data` explicitly must still count as an explicit selection.

When neither source is supplied, automatic discovery must try `data` beside the executable, `../share/data` beside the executable, `../Resources/data` beside the executable, and `data` in the working directory, in that order. It must select the first existing directory. Exhausting these candidates must fail with a diagnostic that lists the attempted locations and explains `--data-dir` and `SPEKTRAFILM_DATA_DIR`.

Selection must not change the working directory. Commands must validate the resources they consume within the selected directory. Missing required resources must not trigger selection of another directory. A profile-listing command must fail if the profile directory cannot be read or a listed profile cannot be loaded. A valid empty profile directory may produce an empty list.

Commands that consume data must use this selection policy. Built-in preset listing must remain available without data when no explicit selection is supplied. Preset library listing must read the preset library directory defined in the [preset design](../presets/design.md) and must not require a data directory. If it receives a flag or environment selection, it must validate that selection without claiming to validate profile contents. Help, version, and field discovery must not require data.

The process dry-run report must expose the selected data location as defined in the [parameter contract](#discovery-and-dry-run). A data-related failure must identify the selected or rejected path. Absolute-path reporting here must not change the separate privacy contract for [telemetry](../telemetry/spec.md).

## Errors and command status

Invalid command syntax must return status `2`. A failed command after successful argument parsing must return status `1`. A completed command must return status `0`. Existing telemetry-report persistence exceptions must follow the [telemetry contract](../telemetry/spec.md#local-surfaces).

An error must identify the offending option or input and explain the corrective action when it is known. Unknown parameter paths, preset names, module prefixes, and workflow routes must offer up to three nearby canonical candidates when a sufficiently close match exists. Otherwise, they must identify the relevant discovery command or help option. Candidate suggestions must not change the supplied value, accept an invalid invocation, introduce aliases, or search the filesystem for an unknown preset name.

CLI errors must use stderr. Machine results must use stdout. Progress, timing, selected-backend messages, and completion notices must use stderr. Human messages must not be inserted into JSON or TOML results. Normal image processing must continue to leave stdout empty.

LUT build must announce the start of baking and, when requested, the start of quality assessment on stderr when attached to a terminal. These messages must name the current stage. They must not invent a percentage, ETA, or measured duration. Completion and quality summaries must remain available on stderr when redirected. This change must not add a JSON LUT-result protocol or alter the meaning of quality assessment.

## Workflow acceptance

The two executables must accept negative tint with a custom RAW white balance and valid temperature using separate option and value arguments. LUT exposure must accept a separate negative value. The resulting values must match the equivalent `--option=-VALUE` invocation. Invalid numeric domains must still fail.

Help must explain all public options and the LUT `OUT` argument. Every README and packaged CLI example must parse using the executable it names. Version and help must succeed outside the repository with no data discovery or device initialization.

An explicit missing directory must fail even when working-directory data exists. An invalid environment selection must fail even when bundled data exists. A valid flag must override an invalid environment selection. Explicit `--data-dir data` must not become automatic discovery. A selected directory missing required profiles must fail without using another directory. Failure to read or load profiles must return a nonzero status.

Automatic discovery must exercise relocated executable-relative data, working-directory data, and exhaustion of all candidates. Built-in preset listing must succeed without data when no explicit selection exists.

A misspelled known field or preset name must fail and suggest its nearby canonical spelling, including a library name. An unrelated unknown value must fail without a misleading correction. Existing invalid-option suggestions must remain available.

JSON and TOML discovery, dry-run, render, and inspect output must remain parseable with stderr captured separately. LUT bake and quality messages must use stderr. Normal image rendering must preserve its stdout behavior and output pixels.

## Workflow implementation status
The current executable implements the help, version, signed-number, data-selection, discovery, dry-run, and stderr/stdout separation described above. Native image publication requirements and failure scenarios remain owned by [photo export](../photo-export/spec.md#publication). LUT QA exit semantics, bundle replacement protection, and multi-print naming collisions remain separate behavioral investigations; they are not inferred from this CLI workflow contract.

LUT QA exit semantics, bundle replacement protection, and multi-print naming collisions require a separate behavioral investigation before a publication or quality contract is changed. They are not acceptance criteria for this implementation.
