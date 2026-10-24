# CLI

This document is the unified CLI specification. The parameter and workflow sections below form one contract; links between them use local anchors.

The [stage input and output contract](stage-artifacts.md) adds resumable material artifacts and partial routes. The Boolean debug flag below adds human-readable stderr diagnostics. The process interface MUST NOT accept the retired `--stage-evidence` option.

## Parameter contract

The `process` command MUST support [presets](../presets/spec.md), sparse parameter files, and individual parameter assignments through the unified `--set` option. Both `spektrafilm` and `spektrafilm-f64` MUST accept the same language. Backend policy and computation precision follow [GPU execution](../gpu-execution/spec.md#backend-selection).

A **parameter override** is a sparse set of explicit edits to runtime controls for one invocation. It MAY contain look-owned and non-look controls. It MUST NOT change profile references or rewrite its source preset.

A parameter override is not a preset, GUI state, or render recipe. The parameter contract owns the added command syntax. Preset document fields and carriers belong to the [preset specification](../presets/spec.md).

Every leaf carried by a preset MUST remain addressable through canonical `--set` and exposed by `describe --field` or `describe --module`; the CLI description MUST report the same type, validator, units, and effect conditions used by the GUI. Field names, units, bounds, and conditional effect behavior belong to their parameter model and capability specifications.

Help, data-directory selection, command status, and human feedback belong to the [workflow contract](#workflow-contract).

`film_channel_swap` and `print_channel_swap` are profile-data diagnostics. They reorder which density row feeds each RGB channel and exist to diagnose a profile whose channel assignment is wrong. They are not photographic controls.

The GUI exposes them with an `(experimental)` suffix in FILM → Convert; the CLI `--film-channel-swap` and `--print-channel-swap` flags remain the primary surface. A swap value MUST be a permutation of `[0, 1, 2]`; the command MUST reject a non-permutation before rendering.

## Process syntax

The command MUST accept these forms:

```text
spektrafilm process INPUT --output OUTPUT --film STOCK [--paper STOCK] [--params FILE] [OPTIONS]
spektrafilm process INPUT --output OUTPUT --preset SELECTOR [OPTIONS]
spektrafilm process ARTIFACT --route ROUTE --output OUTPUT [OPTIONS]

OPTIONS may include:
  --preset SELECTOR     repeatable; presets apply in appearance order
  --set SOURCE          repeatable file or comma-separated assignments
  --dry-run             resolve and validate; print effective parameters without rendering; --output may be omitted
  --dry-run-format json|text  dry-run presentation; default: json
  --input-color-space NAME   raster source primaries; discover with describe --field io.input_color_space
  --input-cctf-decoding true|false  whether raster samples require transfer decoding
  --seed UINT32         deterministic stochastic seed; image default: 0; artifact default: retained seed
  --feature-profile FAMILY=NAME  repeatable; apply a built-in feature profile before --set sources; see [feature profiles](../feature-profiles/spec.md#cli)
  --debug              emit detailed execution diagnostics to stderr; no argument or implicit files
```

Existing backend, workflow, RAW, output, and data-directory options MUST remain available.

### Preset selector

A selector ending in `.toml` or `.json` MUST designate a file path relative to the current working directory or an absolute path. It MUST NOT fall back to a preset name if the file is absent or invalid. A selector without a supported extension MUST designate an exact preset name.

A name MUST resolve with the group order of the [preset design](../presets/design.md): the user library first, then the built-in presets. The prefixes `user:` and `built-in:` MUST select one group explicitly and MUST otherwise be optional. A name MUST NOT contain a path separator.

A bare name that also exists in another group MUST resolve in the user library, and the command MUST report the resolved group and the shadowed name. An unknown name MUST NOT trigger a filesystem search or a name-similarity match.

`--preset` MUST conflict with `--film`, `--paper`, and `--params` when the resolved preset set supplies the matching profile reference; a preset set that supplies none MUST take required profiles from the existing selectors. For image input without a preset, `--film` remains required. A route ending at film MUST NOT require a print profile.

Artifact input obtains its frozen material from the artifact under the [stage profile rules](stage-artifacts.md#input-declarations-and-profile-requirements). The `input` workflow MUST ignore the resolved film and paper profiles. Without a preset, an image direct-film scan MUST select the film as the runtime print profile.

A photographic print operation MUST obtain its print material from the applicable explicit, retained, or target-print selection. CMYK ink style enablement MUST NOT affect these requirements. Without a preset, a print target MUST accept `support=paper` or `support=film` with `stage=printing` when parameter sources are added.

Preset print references retain the paper-only restriction in the preset specification.

`--preset` MAY be repeated; presets apply in appearance order, and supplied profile references and parameter controls count as explicit. For image input with a look preset, both profile references MUST come from that preset. `--scan-film` MUST request `input > film > scan` without replacing the preset's print reference.

On image input, the preset's workflow default MUST apply unless `--route` or `--scan-film` supplies an explicit choice. A preset set that supplies no film reference MUST NOT select a profile-derived workflow default. Artifact input MUST supply `--route` explicitly.

`--route` and `--scan-film` MUST conflict, including for equivalent routes. The final route MUST determine topology; `io.scan_film` MUST be true for `input > film > scan` and `film > scan`, and false for other supported routes. The shared workflow validator MUST enforce the finite route vocabulary, including the [stage routes](stage-artifacts.md#route-language).

CMYK ink style enablement remains independent of `--route` under its [owning contract](../cmyk-style/spec.md).

`--scan-output direct_scan|negative_to_positive` MUST override `scanner.scan_output` after all parameter sources. The shared resolver MUST enforce the [negative-film scan output contract](../film-chain/spec.md#direct-negative-film-scan-output) before runtime constraints and on effective parameters. An executed scan MAY select `negative_to_positive` for an eligible negative film on `input > film > scan`, `film > scan`, or `input > convert-film > scan`; print-material and scan-minus-base scans remain direct-only.

A route ending at a material state MAY retain scanner controls as downstream defaults without executing a scan. Dry run and rendering MUST reject the same unsupported combinations.

The scan has no light-source control: the readout illuminant is a product constant selected by the material category. The shared resolver MUST reject any legacy scanner light leaf before either dry run or rendering.

### Input declaration

A **source declaration** states the color space and transfer state of raster samples before simulation. It is not a look or a saving conversion. Every public raster `process` invocation MUST supply both `io.input_color_space` and `io.input_cctf_decoding` through dedicated input flags, supplied leaves of legacy `--params`, or `--set`.

Defaults and look presets MUST NOT count as a declaration. A missing component MUST fail with guidance to declare both components. File extension, bit depth, and embedded ICC metadata MUST NOT select a declaration.

This requirement also applies to the `input` route and dry run.

`--input-color-space` and `--input-cctf-decoding` MUST override the matching runtime controls after all `--set` sources, independent of command-line position. Each flag MUST use the type, vocabulary, and validator of its canonical field. A declaration MAY mix sources.

The report MUST identify the winning source for each component. Input flags MUST NOT modify scanner or saving output settings. Declaring these fields in legacy `--params` MUST NOT make other baseline fields protected edits.

RAW input MUST obtain its declaration from the RAW adapter under the [image-input contract](../image-input/spec.md). Omitted raster input controls MUST NOT cause a RAW declaration error. Every explicit input flag or `--set` assignment incompatible with linear ACES2065-1 MUST fail, even if a later source would replace it.

Compatible explicit controls MAY be supplied. Legacy RAW baseline normalization MUST remain unchanged. RAW loading flags MUST retain their existing applicability and validation.

### Capture illuminant

The assumed scene light of the photograph is declared through the canonical path `camera.capture_illuminant`, under the [image-input capture illuminant contract](../image-input/spec.md#capture-illuminant). There is no dedicated `process` flag and no shortened path; the [parameter contract](#parameter-contract) owns its syntax exactly as it does for every other runtime leaf.

- The leaf is an optional string. TOML omission makes no edit.
- Inline `null` and JSON `null` clear it to its unset value, which then inherits the selected film profile's `reference_illuminant`.
- An explicit value is a catalog illuminant name or a `BB<kelvin>` token, validated by the shared illuminant vocabulary. An unsupported or malformed value MUST fail with the permitted values and MUST NOT fall back to the profile reference.
- The leaf belongs to the camera group, so it participates in the same composition layers as the other camera controls: presets apply first, then `--set` sources in appearance order, then workflow, scan-output, and input choices.
- RAW loading flags remain loader controls. `--raw-white-balance`, `--raw-temperature`, and `--raw-tint` MUST NOT write this leaf, and RAW metadata MUST NOT supply a value.
- Editing this leaf MUST NOT change the readout illuminant, `enlarger.illuminant`, the output white point, or the film profile reference. It MUST NOT rewrite a preset document or change profile references.

`describe --field camera.capture_illuminant` and `describe --module camera` MUST report the leaf's type, nullability, the shared illuminant vocabulary, the inheritance rule, and the conditions under which the leaf is inactive, using the same metadata as the GUI. Discovery MUST state that an unset value inherits the selected film profile's reference illuminant and that no metadata source fills it.

The [capture-illuminant DSL](../image-input/capture-illuminant.dsl.md) is a worked example of this existing parameter language. It defines no new schema, parser, or path.

### Seed

`--seed` MUST accept a decimal integer from `0` through `4294967295`. An omitted seed MUST select `0` for image input and inherit the producing seed for artifact input under the [continuation rules](stage-artifacts.md#frozen-state-and-editable-continuation). Both executables MUST accept the same seed domain.

Seed MUST remain outside runtime parameter serialization, presets, and `--set`. It MUST be applied before the first simulation attempt. Every `--iters` attempt MUST use the same selected seed.

The command MUST NOT substitute a time-based or random seed.

The same input pixels, effective parameters, seed, executable precision, and execution route MUST reproduce the same stochastic output under the owning grain contract. This rule MUST NOT promise CPU/GPU or f32/f64 bit identity. Seed validity MUST be checked in dry run and before rendering.


## `--set` sources

Each `--set SOURCE` occurrence MUST be exactly one source. A source beginning with a canonical `PATH=` MUST be an inline assignment source, even when its value ends in `.toml` or `.json`. Otherwise, a source ending in `.toml` or `.json` MUST be a sparse parameter-file path.

An inline source MUST contain one or more comma-separated `PATH=VALUE` assignments. A source MUST NOT mix a file path and assignments. `--set` MAY be repeated; sources apply in appearance order.

A file path MUST be relative to the working directory or absolute. An assignment source MUST be passed as one shell argument.

For a file source, the extension MUST select the parser. Parse failure MUST NOT cause a fallback to another parser. Files MUST NOT support includes or environment interpolation.

A missing or invalid `.toml`/`.json` source MUST fail as a file error; it MUST NOT fall back to a preset name or assignment parsing.

The document root MUST be an object. Nested objects or TOML tables MUST locate runtime parameter groups. Only supplied leaves MUST be edited.

A missing leaf MUST retain its value from the preceding layer. An empty object MUST make no edits. Scalars and arrays MUST replace their whole leaf value.

Arrays MUST NOT merge by index. A group MUST NOT be replaced by a scalar or `null`.

A file MUST NOT contain preset metadata, a `parameters` wrapper, profile references, GUI sections, or render-recipe fields. Unknown fields and duplicate keys MUST fail. A sparse file MUST NOT acquire omitted values by deserialization into a default-filled runtime parameter structure before merging.

The canonical CLI paths for the terminal appearance are `cmyk.active` and `cmyk.strength`. Sparse parameter files and inline `--set` assignments MUST reject `magazine_print_color.*` in the current parameter schema.

A legacy complete `--params` or RenderRecipe carrier MAY translate the old group only when its declared schema version is listed as legacy; the resolver MUST emit the canonical group and preserve unrelated fields. CLI parameter migration MUST NOT mutate the source file.

TOML omission MUST mean no edit. JSON `null` MUST set an optional leaf to its unset value. It MUST NOT mean delete the field.

Non-optional leaves MUST reject `null` in the added parameter-file language, even if a legacy loader accepts it through normalization.

TOML date/time values, non-finite floats, and values that cannot map to the declared parameter type MUST fail. Readers MUST NOT stringify these values. JSON and TOML MUST use the same leaf-type, range, and enum validation after carrier parsing.

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

An assignment MUST contain a canonical runtime path, the first `=`, and a value interpreted by the target field type. A path MUST consist of dot-separated field identifiers. Each identifier MUST match `[a-z][a-z0-9_]*`.

Paths MUST be case-sensitive. Additional `=` characters MUST remain part of the value. Empty paths and empty values MUST fail.

Assignments in one source are separated by commas at the top level. Commas inside a double-quoted string or an array MUST remain part of that value. Whitespace around the separator and assignment is ignored.

Empty items MUST fail. Object values, group replacement, array indexing, aliases, arithmetic, and units are not supported.

String and enum fields MUST accept plain text without JSON quotes. Shell quotes around an argument MUST be sufficient for spaces, such as `--set 'io.output_color_space=ProPhoto RGB'`. A value beginning with a double quote MUST remain a complete JSON string; malformed quoted strings MUST fail.

JSON strings MUST remain available for literal commas, escapes, and empty strings. Bare text such as `true`, `123`, or `null` MUST remain text for non-optional string fields. For an optional field, unquoted `null` MUST clear the value.

Boolean fields MUST accept `true` or `false`. Numeric fields MUST accept finite numbers representable by the declared type. Floating-point fields MUST also accept ordinary decimal forms such as `+0.5` and `.5`.

Array fields MUST use JSON arrays, such as `[4.5,4.5,4.5]`. Integers MAY supply floating-point fields. Fractional numbers MUST NOT supply integer fields.

Quoted strings MUST NOT be coerced into numbers or booleans. Type errors MUST identify the field and expected value form. Enum errors MUST identify the permitted values.

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

The assignment parser MUST split only at top-level commas, then interpret each value using the shared field description. It MUST NOT split commas in double-quoted strings or arrays. The sparse-file carrier rules MUST remain unchanged.

`--set` sources MUST be syntactically valid even if a later source replaces one of their leaves. The existing `--film`, `--paper`, `--params`, and `--route` selectors MUST each occur at most once; `--preset` is repeatable as defined in [Preset selector](#preset-selector). Dedicated input flags, `--seed`, and `--dry-run-format` MUST each occur at most once.

Repetition MUST fail at argument parsing rather than silently select a value.

### Editable paths and conditional controls

Paths MUST use the runtime parameter vocabulary, such as `film_render.grain.v2_amount`. The command MUST NOT add shortened paths such as `grain.amount`. It MUST NOT infer a grain engine or change `v2_profile` to `custom` from the presence of a control.

`workflow.route` MUST use `--route`; `io.scan_film` MUST remain derived from the selected workflow. Neither path MAY appear in a parameter file or inline assignment. Random seed MUST remain outside this language.

Pipeline-derived `film_render.grain.monochrome` and the legacy non-exported `particle_scale_layers` field MUST NOT be editable paths. Other runtime controls MUST use the shared editable parameter description.

Controls for inactive effects or engines MAY be stored without affecting output. [Procedural grain](../film-grain/v2/spec.md) MUST retain its named-profile and Custom behavior. Spektra RMS granularity and Procedural Amount MUST remain distinct controls.

A parameter-file leaf or inline assignment MUST NOT change another control to make itself effective. The [Output mix and material controls contract](../output-mix/spec.md) defines use of existing preflash and chemistry controls and owns `output_mix`. Simulation blend uses the existing canonical assignment `--set output_mix.amount=0.5`, without an alias or dedicated process flag.

`output_mix.amount` is a non-nullable finite f64 in `[0,1]`, with default 1. Current complete parameter documents, sparse sources, recipes, and GUI state MUST share typed value validators. Complete-document compatibility MAY supply an absent output-mix default; sparse omission remains no edit.

Material-control edits MUST retain existing route and profile capability conditions. Preflash affects exposure only on admitted print-development routes, including printing recovered negative density. Film chemistry MUST NOT redevelop real captured negatives or run on the `input` workflow.

Developer exhaustion MUST remain a model-specific development control; unsupported profiles MUST NOT acquire a fitted model or substitute effect. Inactive values remain stored. Edits MUST NOT change a route or silently activate chemistry.

Scanner points, inverse reconstruction, and calibration retain their existing meanings. There are no new generic levels, saturation, color-density, or split-toning controls.

`io.output_gamut_compress.lightness_compression` is retired from the runtime model, editable paths, discovery, GUI, and execution. Current complete documents, sparse files, and inline assignments MUST reject that exact leaf even when null or a later layer would replace it.

A versioned legacy complete-document migration MAY allow absence or null only; every explicit numeric triplet MUST fail even if its old gamut algorithm ignored it. The diagnostic MUST name `io.output_gamut_compress.lightness_compression`, state that no automatic replacement exists, and require explicit removal without suggesting a deleted response group. New writers MUST never emit it.

Existing gamut algorithms otherwise stay unchanged. The unshipped `film_render.response`, `print_render.response`, `enlarger.c_filter_shift`, `reproduction.*`, and `io.finished_rgb_output_processing` paths MUST fail normal unknown-field validation without aliases, defaults, or migration, including in draft version 3 presets. Presets MUST reject the unshipped `output response` section.


## Composition

Options form fixed layers: presets are resolved in their appearance order first, then `--set` sources are applied in their appearance order, regardless of their position among unrelated options. A later preset or source replaces the preceding value at a matching leaf, and a later profile reference replaces an earlier one.

Inline assignments within one source also apply in their written order.

For an invocation with added parameter sources, resolution MUST use this order:

1. Select and validate the film and print profiles. Decode the baseline with `RuntimeParams::default()` or the unchanged legacy `--params` decoder.
2. Apply stock-specific look initialization to this baseline with the same precedence as the legacy batch policy. Defer preview, debug, and database-calibration preparation until the final controls are available.
3. Values supplied only by legacy `--params` MUST retain their legacy relationship to stock initialization and database calibration.
4. Apply every selected preset in appearance order through the shared preset resolver. A look preset replaces the whole look; a section preset edits only the controls of its section, and those values count as explicit from this step on.
5. Retain excluded runtime context and derive the profile workflow default when the preset set supplies a film reference.
6. Apply every `--set` source in appearance order. A file source and an inline assignment source have identical leaf-merge semantics.
7. Apply explicit workflow, scan-output, and input choices. Enforce the [input declaration](#input-declaration) rules.
8. Input flags MUST count as explicit only for their matching leaves.
9. Validate the composed controls. Resolve dependent database defaults only for values that are not explicit.
10. Apply preview, LUT-mode, and debug constraints using the final controls. Do not reapply stock look defaults over explicit edits.
11. Validate the effective typed parameters after constraints. Resolve and validate the output options before decoding input pixels or rendering.
12. Create the runtime using these effective parameters.

The same layers MUST apply with or without a preset. A preset's values, parameter-file leaves, and inline assignment leaves MUST count as explicit. Legacy `--params` MUST NOT introduce this new explicit-value protection.

Each explicitly supplied neutral-filter axis MUST retain its value through preparation. With database calibration enabled, preparation MUST calculate each unprotected axis from the final film, print, and illuminant. Missing database entries MUST retain the preceding value.

Disabling database calibration MUST skip the lookup. Runtime preparation MUST NOT change the user's stored calibration setting to protect an explicit value. Explicit-axis protection MUST remain active during runtime construction and recalibration.

Unprotected axes MUST retain the database's f64 precision during calibration. Protected axes MUST use the supplied runtime control value.

Preview, LUT-mode, and debug constraints MAY disable an explicitly enabled effect. They MUST run after all edits. Disabling one of these modes in a parameter source MUST allow the final configuration to resolve without destructive edits left from an earlier intermediate preparation.

Stock defaults and runtime constraints MUST NOT be treated as the same precedence layer.

The internal numerical debug constraints described in this composition section are not the CLI `--debug` flag. The flag MUST NOT select those constraints, change parameters, or disable effects. Artifact input uses the restored-state preparation boundary in [Frozen state and editable continuation](stage-artifacts.md#frozen-state-and-editable-continuation), not repeated stock/input initialization.

Export options MUST follow [Export and execution options](#export-and-execution-options).

## Export and execution options

The output path, format, sample depth, compression, JPEG controls, saving color conversion, backend, and RAW loading controls MUST use existing dedicated flags. They MUST NOT be accepted as parameter-file paths or inline assignment paths. A parameter file containing an `[export]` table or an inline assignment such as `export.bit_depth=16` MUST fail as an unknown parameter group or path.

This language MUST NOT add a complete job or export-configuration file.

`--output` (`-o`) MUST select the destination. For image terminal routes, `--format jpeg|png|tiff|exr`, `--bit-depth 8|16|32`, `--compression zip|none`, `--jpeg-quality`, and `--jpeg-subsampling 444|420` retain their existing applicability and defaults. The image extension MUST select the format when `--format` is absent; an explicit format MUST agree with that extension.

Image writer constraints belong to [photo export](../photo-export/spec.md). Routes ending at film or print use the [artifact writer](stage-artifacts.md#output-and-publication) and reject image writer options; the output filename MUST NOT select the executed stages.

`--saving-color-space` and `--saving-cctf-encoding true|false` MUST select the final writer-stage color conversion. They MUST NOT be aliases for scanner `io.output_color_space` and `io.output_cctf_encoding`, which remain editable runtime controls. Omitting saving color MUST use ACES2065-1 for EXR and the scanner output space for other formats.

Omitting saving encoding MUST select linear EXR and otherwise inherit scanner output encoding. JPEG and PNG MUST require encoded saving output. EXR MUST require linear saving output.

These rules MUST be validated before rendering, including support for the selected color space and conversion.

`--backend cpu|gpu` MUST retain precedence over `SPEKTRAFILM_BACKEND`. `--backend gpu` MUST be an explicit GPU request under that policy: a configuration whose computation scope cannot run on the device MUST fail before publication instead of executing a stage on the host. Omitting the flag MUST follow the [GPU-first selection policy](../gpu-execution/spec.md#backend-selection) while retaining explicit environment selection.

Output sample depth MUST NOT select computation precision. `spektrafilm-f64 --backend cpu` MUST retain the CPU f64 reference contract; software WGPU and hardware WGPU MUST remain distinguishable execution evidence.

The following POSIX command combines a look with independent TIFF export options:

```bash
spektrafilm-f64 process input.png --output output.tif \
  --preset 'Kodak Portra 400' \
  --input-color-space sRGB --input-cctf-decoding true \
  --backend cpu --bit-depth 16 --compression zip \
  --saving-color-space "ProPhoto RGB" --saving-cctf-encoding true
```

Each file source and inline assignment source MUST pass syntax, field, type, finite-number, leaf-range, and enum validation. Each selected preset MUST pass document validation before `--set` sources apply. Cross-field validation MUST run on the composed controls before runtime constraints so constraints cannot conceal an invalid user configuration.

The runtime MUST also validate parameters after applying its constraints.

Section composition MUST defer scan compatibility until explicit workflow and scan-output choices are applied. Active film and print chemistry MUST use the same control-domain validation in document validation, dry run, and runtime preparation.

Errors MUST identify the offending option or file, field path when available, supplied value when safe to display, and violated rule. Parse errors MUST include the parser location when available. Unknown paths MUST NOT create new fields; paths naming non-settable runtime fields MUST identify the out-of-band field and why it is unavailable.

Unsupported enum values, profile identities, ranges, and combinations MUST NOT fall back to defaults.

A failed configuration MUST return a nonzero status before rendering or writing image output. The command MUST NOT retry with a different preset or baseline.

### Debug diagnostics

`process --debug` MUST be a Boolean flag on both public executables. It MUST take no argument, have no value syntax, and occur at most once. `--debug=true`, `--debug=false`, and a debug destination argument are invalid.

The parser MUST NOT reinterpret an extra positional path as a debug directory. This flag does not become a runtime parameter, preset leaf, recipe field, or persisted GUI control.

The flag MUST add human-readable execution detail to stderr. Detail MAY include parameter resolution sources, actual stage execution and skips, selected and actual backend, computation precision, and error cause chains. Reports MUST distinguish requested configuration from observed execution.

Required warnings, failures, and actionable failure reasons MUST remain visible without the flag. Debug detail MUST NOT disclose credentials or dump complete image arrays.

The flag MUST NOT create a file or directory, save intermediate buffers, retain extra full-image copies, enable GPU timestamp collection, select a different backend, change numerical constraints, consume random draws, or run an additional stage or render. It MUST NOT change exit-status semantics, stdout content, JSON/text dry-run results, image pixels, or required artifact content.

It MUST be valid with dry run without observing unexecuted outcomes. Existing logging infrastructure MAY supply the messages, but it MUST NOT turn the flag into a numerical debug mode.

Normal `process` stdout remains empty. Machine-readable commands MUST keep their declared stdout format free of debug text. Shell redirection owns text-log persistence; no debug path flag or implicit debug bundle is added.

Human debug wording and line order are not a stable machine protocol.

`--diagnostics REPORT.json` remains an independent explicit structured-report request under the telemetry contract. It MUST work without `--debug`, and `--debug` MUST work without it. Both paths MUST reuse execution observations rather than render twice.

The flag MUST NOT change the structured report schema or enable `--gpu-timings`; callers MUST request GPU timings separately. An explicitly requested report retains its existing publication and failure policy.

### Stage input and output

The [stage artifact contract](stage-artifacts.md) owns the single-entry `.sfstage` format, route additions, immutable-state rules, dry-run extensions, and publication. Production state output MUST use ordinary `--output` and MUST NOT require debug. `--stage-evidence` MUST be removed with no alias or automatic translation.

Public all-stage side capture is intentionally removed; neither debug nor separately executed partial renders are equivalent evidence.

Dedicated instrument [native observations](../instrument/native-observations.md) remain research outputs. Their data format and capture contract are not a production continuation format or a debug feature.

## Discovery and dry run

`describe --format json` without a field or module selector MUST retain its existing machine contract. `describe --module PATH --format json` MUST report editable leaf paths under one runtime group prefix, including nested groups. Each field MUST report its type, nullability, static default, unit when applicable, available enum values or numeric bounds, and conditions under which it affects output.

The result MUST distinguish static defaults from values derived from stock profiles. Unknown prefixes MUST fail.

`describe --field PATH --format json` MUST report one editable leaf as a metadata object with the same field keys used in module discovery. `--field` and `--module` MUST conflict. A group supplied to `--field` MUST fail with guidance to use `--module`.

A leaf supplied to `--module` MUST fail with guidance to use `--field`. These selectors MUST use canonical paths without aliases or prefix expansion.

`describe --format text` without a selector MUST list the discoverable top-level parameter groups followed by non-settable runtime fields and their reasons. With `--module`, it MUST show one record per editable leaf, sorted by canonical path. With `--field`, it MUST show the selected leaf.

A record MUST show path, type, nullability, default and its source, unit when applicable, enum values or bounds, distinct array-component domains, and effect conditions. Missing units, bounds, and finite enum inventories MUST NOT be presented as invented values. `describe` MUST retain JSON as its default format.

`preset list --format json` MUST list preset names, the kind and section of a section preset or the film and print references of a look preset, and the group of each entry. `preset list --source built-in|user|all` MUST select the listed groups and MUST default to `built-in`.

`preset list --format text` MUST show those same facts in one readable record per preset. Listing MUST report every unreadable or conflicting library document with its path and MUST NOT hide the remaining entries. Listing MUST NOT search directories other than the library directory.

`preset show SELECTOR --format toml|json` MUST write the selected document to stdout. `--format` MUST default to TOML for `preset show` and JSON for `preset list`. Both commands MUST follow the [CLI data-directory selection policy](#data-directory-selection), and listing library entries MUST NOT require a data directory.

`preset show` MUST validate the selected document and profile references using that data directory. These commands MUST NOT inspect a GUI state or render an image. Export MUST preserve the selected preset name and MUST NOT modify its source file.

Export and update of an older whole-look preset MUST migrate its look-owned controls to version 3 with `output_mix.amount = 1`. Older section presets MUST retain sparse omission and MUST NOT gain output-mix or other leaves. User-library import MUST preserve the older carrier until explicit export or update writes version 3.

Discovery MUST NOT change a stored document merely by listing it. Preset application and export MUST preserve the caller's other `io` settings; no preset owns output gamut processing.

`process --dry-run` MUST accept the same configuration and input paths as a normal process invocation. It MUST resolve parameters, workflow, and writer options using the same decisions as normal processing. `--output` MUST be optional with `--dry-run`.

Normal processing MUST still require `--output`. Without `--output`, `output` MUST remain an object whose `path` is `null`. Its `format` MUST be `null` unless `--format` was supplied.

Writer options independent of the format MUST use their normal defaults or explicit flags. Format-dependent defaults MUST remain `null` when the format is unresolved. Explicit codec options require a resolved format; without a destination they MUST require `--format` and report that corrective action.

It MUST check input existence, readability, and supported file kind without decoding input pixels. It MUST NOT initialize a compute device, render, or create or truncate any output, including `--raw-out`. `--dry-run-format json|text` MUST require `--dry-run` and default to `json`.

Terminal detection MUST NOT change the format. JSON MUST contain one object on stdout with the existing `film_profile`, `print_profile`, `parameters`, `output`, and `backend` keys. `parameters` MUST contain effective runtime controls after constraints.

`output` MUST retain `path`, `format`, `bit_depth`, `color_space`, `cctf_encoding`, `compression`, `jpeg_quality`, and `jpeg_subsampling`; inapplicable options MUST be `null`. `backend` MUST report the requested `cpu` or `gpu` policy, or `auto` when selection remains deferred. Diagnostics MUST go to stderr.

The result MUST be a resolution report, not an implicit RenderRecipe or GUI state.

Artifact input and output extend this report under the [stage dry-run contract](stage-artifacts.md#dry-run). A known artifact terminal output reports `format: "sfstage"` even without a path, with null image-writer fields. The image-only unresolved-format rules below MUST NOT force a fictitious image format on a material state.

When no output format is resolved, `unchecked` MUST include `output_format`. An explicit `--format` MUST resolve and validate writer options without requiring an output path.

The dry-run report MUST also contain `data` and `input` objects. `data` MUST contain the absolute selected `path` and its `source` (`argument`, `environment`, or `automatic`). `input.kind` MUST be `raw`, `raster`, or `stage_artifact`.

For RAW input, `raw_loading` MUST contain the selected `white_balance`, `temperature`, `tint`, and `lens_correction` under the dedicated flag vocabulary; absent custom temperature and tint remain null. Other input kinds MUST report `raw_loading: null`. The [artifact dry-run contract](stage-artifacts.md#dry-run) owns artifact metadata.

Existing keys and meanings MUST remain; loading controls MUST NOT be inserted into runtime `parameters`.

The JSON report MUST add `seed` as the selected integer. Raster and RAW input MUST report `input.declaration` as an object containing `color_space`, `cctf_decoding`, and `sources`. `sources` MUST map each component to a record with `kind`, `option`, `index`, and `path`.

`kind` MUST be `argument`, `params`, `set`, or `raw`. `option` identifies the flag or is null for RAW. `index` is the one-based `--set` occurrence or null.

`path` is an absolute file-source path or null. These paths belong to local resolution, not telemetry. RAW MUST report its adapter as both component sources after compatibility checks.

Artifact input MUST report `input.declaration: null`; its native density interpretation comes from the artifact, not a fabricated RGB declaration.

The report MUST add `edits` as an array of winning `--set` leaves sorted by canonical path. Each entry MUST contain `path`, `requested`, `effective`, `source`, `status`, and `reason`. `source` MUST use the declaration source record shape.

Values MUST be typed JSON values. `status` MUST be `active`, `inactive`, `constrained`, or `overridden`. `active` means the edit remains effective and its known effect conditions hold.

`inactive` means a known effect condition prevents use. `constrained` means runtime preparation changed the requested value. `overridden` means a dedicated flag replaced the winning edit.

Classification precedence MUST be `overridden`, then `constrained`, then `inactive`, then `active`. `reason` MUST be `null` for active edits and identify the responsible condition, constraint, or option otherwise. Later `--set` replacements MUST NOT leave earlier duplicate entries.

A full per-field provenance history MUST NOT be added. Active status MUST NOT claim a measured pixel change.

Text dry run MUST show the same resolved input declaration and sources, profile identities, route, scan-output mode, seed, requested backend, writer options, and edit feedback as JSON. It MUST show data selection and RAW loading choices. Effective exposure, film format, grain engine, and grain controls MUST use their canonical units and conditions.

Text layout MUST NOT be a machine contract.

Both dry-run formats MUST report the capture illuminant separately from every other light source, in a `capture_illuminant` object. The report MUST carry the selected film profile's `reference_illuminant`, the requested `camera.capture_illuminant` value, the effective value in force for the render, and the source that supplied it, where source is one of `unset`, `set`, or `preset`.

An unset requested value MUST report the inherited profile reference as the effective value with source `unset`. An explicit value on a route that computes no film exposure from scene RGB MUST be reported inactive with its reason, and its effective value MUST be the profile reference illuminant.

The report MUST NOT present RAW white balance, the readout illuminant, `enlarger.illuminant`, or the output white point as the capture illuminant.

Both dry-run formats MUST report the resolved readout illuminant separately from the capture and enlarger illuminants, together with the material-category rule that selected it.

Both formats MUST state that pixel decoding, compute-device availability, metered exposure, and final pixel geometry remain unchecked. JSON MUST add `unchecked` with the stable values `pixel_decode`, `compute_device`, `metered_exposure`, and `output_geometry`. Dry run MUST NOT claim these outcomes.

A configuration valid in dry run MAY still fail on decoding, resource availability, rendering, or publication.

Normal processing and dry run MUST warn on stderr for winning `--set` edits with non-active status. Each warning MUST name the canonical path, source, and reason. Known corrective choices MUST be explained without applying them.

Unedited inactive defaults and preset controls MUST NOT produce these warnings. Feedback MUST NOT enable effects, select a grain engine, change a Procedural profile to `custom`, or turn a valid stored inactive value into a configuration error.

Field discovery MUST explain input versus scanner versus saving color, exposure compensation in EV versus print exposure, and film format as the source long-edge dimension in millimeters. Crop discovery MUST state that both `io.crop_size` components are fractions of the uncropped source long edge, not of independent width and height. Procedural Size discovery MUST identify its `custom` profile condition.

Procedural Amount MUST remain applicable to named and custom Procedural profiles. Both controls MUST remain distinct from Spektra RMS granularity. Text and selected-field/module JSON MUST use the same descriptions and effect conditions.

Selector-free JSON MUST retain its existing render contract. Discovery MUST distinguish material density-curve development controls, optical M/Y exposure filtration and calibrated neutral filters, preflash, reference print normalization, existing scanner interpretation points, output gamut processing, and global source mixing. It MUST NOT advertise retired lightness or unshipped response, Cyan-shift, saturation, color-density, or split-toning paths.

Existing Cyan neutral calibration remains a calibration control rather than a new creative Cyan shift. Scanner lens blur discovery MUST identify linear readout optics before interpretation and gamut processing. Unsharp mask MUST identify digital scan sharpening.

`print_render.glare` MUST identify print viewing/readout stray light, not preflash or density formation. Grain descriptions MUST distinguish material grain from [Procedural post-scan appearance](../film-grain/v2/spec.md), including Film Resolution's appearance-only meaning. These descriptions MUST NOT rename canonical paths or introduce aliases.

Dry run MUST report the selected model/version and optical route without claiming that the pixels or kernel were exercised.

Selected-field and module discovery MUST report existing typed defaults, bounds, and units for `enlarger.preflash_exposure`, `enlarger.preflash_y_filter_shift`, and `enlarger.preflash_m_filter_shift`. Preflash exposure is a linear exposure multiplier, not EV. Its zero default disables flash while retaining the independent filter settings.

Both filter shifts use the existing Kodak CC units and zero defaults around the calibrated neutral pack. Their descriptions MUST identify the admitted print-development condition and inactivity when flash exposure is zero or the route has no print development.

Preflash descriptions MUST state the photographic-paper highlight use and possible loss of paper white, midtone changes, and color changes defined in [Preflash](../output-mix/spec.md#preflash). They MUST NOT promise an independent onset, fixed white or midgray, recovered information, or identical behavior for printing film and photographic paper.

Developer-exhaustion discovery MUST report its existing dimensionless `[0,1]` domain, zero default, chemistry enable condition, actual-development route condition, and required supported fitted material model. Discovery without profiles MUST state that model requirement rather than claim a selected profile supports it. Dry run with resolved profiles MUST identify unsupported or inactive chemistry without clearing stored values or fabricating support.

Descriptions MUST NOT promise universal highlight compression or equivalence to a digital compressor. [Output mix](../output-mix/spec.md#output-mix) discovery MUST report f64, non-nullability, default 1, and inclusive `[0,1]` bounds. Amount 1 is the complete result after CMYK ink style; amount 0 is the prepared, same-geometry input before simulated camera exposure and processing.

A captured-negative source remains negative at 0 and MUST NOT be automatically inverted. Mixing occurs in common linear output RGB before save conversion and applies to all workflows. Final output below 1 requires a declared source branch; diagnostic or segmented paths without one MUST reject it.

Amount 1 requires no source copy. Intermediate taps remain unaffected. Dry-run feedback MUST apply these conditions without claiming that pixels were exercised.


The file `adjustments.toml` used below MUST contain:

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

The following command MUST inspect a composed configuration:

```bash
spektrafilm process input.png --output output.tif \
  --film kodak_portra_400 --paper kodak_portra_endura \
  --input-color-space sRGB --input-cctf-decoding true \
  --seed 42 \
  --set adjustments.toml \
  --dry-run
```

## Usability acceptance

Both executables MUST discover a group and an individual field without loading profiles, decoding images, or creating a device. Text and JSON views MUST report the same field facts. Single-field JSON MUST preserve optional values, array-component domains, enum choices, and effect conditions.

Invalid field/group selectors and their conflict MUST fail before image work. Existing selector-free JSON and module JSON output MUST preserve their shape and meaning.

Text preset listing MUST contain the same names, groups, kinds, and profile references as JSON listing. File selectors and preset export validation MUST retain their existing behavior.

Material-control and output-mix discovery acceptance MUST compare text and selected-field/module JSON against their canonical typed defaults, bounds, units, and effect conditions. Dry run MUST retain inactive preflash and chemistry values and explain missing print development, missing Film development, disabled chemistry, and unsupported fitted models when applicable.

Equivalent sparse TOML, JSON, and inline output-mix edits MUST resolve to the same finite f64 amount without changing material activation or routes. Discovery and dry-run descriptions MUST identify both source endpoints, including negative capture, and reject a known missing-source final-output configuration below 1.

Persistence acceptance MUST exercise version 1 and version 2 whole-look migration, sparse Film and Print section migration, and version 3 capture/export/reload. Whole looks gain amount 1; sparse sections gain no leaves; `io` settings outside existing scanner-output normalization remain caller-owned.

Reject unshipped response groups, Cyan-shift paths, and the output-response section through normal schema validation without aliases or migration, including draft version 3 documents. For complete legacy states, recipes, and baselines, exercise absent and null retired lightness leaves and explicit numeric triplets with each old gamut algorithm.

Absence/null MAY migrate; every numeric triplet MUST fail atomically with the exact retired-path diagnostic and no automatic replacement. Current complete documents, sparse sources, and inline nulls MUST fail. Failed migration MUST preserve source, active state, and output sentinels; new writers MUST omit the retired field.

Dry run for custom RAW white balance with temperature `6500` and tint `-5` MUST expose those loading choices. It MUST differ from an as-shot report. Raster input MUST expose inactive RAW loading as `null`.

Both MUST expose the selected data directory and source. None of these scenarios MAY decode input pixels, initialize a compute device, or write image or raw output. Existing runtime and writer fields MUST retain their meanings.

## Compatibility

Legacy `--params` MUST retain its existing decoding and relationship to stock initialization and database calibration, except for the explicit [retired output-lightness migration](#editable-paths-and-conditional-controls). A sparse legacy file MUST NOT be reinterpreted as a sparse parameter override. A legacy baseline MAY be followed by `--set` sources.

The declaration requirement and route normalization are intentional changes for all public `process` invocations. The absence of `--preset` or `--set` MUST NOT bypass them. Seed and report presentation alone MUST NOT select a different numerical preparation policy.

For the same effective parameters, routing, seed, and backend, these interface changes MUST NOT alter numerical computation.

`render --recipe`, `lut --params`, and GUI child-process protocols MUST retain their existing contracts except for the same versioned complete-document output-lightness retirement. They MUST NOT acquire the added process flags implicitly. A rejected migration or configuration MUST preserve the source file, active state, and existing image output.

TOML support for presets and sparse parameter files MUST NOT imply TOML support for complete GUI states, legacy `--params`, or RenderRecipe.

## Image-processing acceptance

1. Run both real executables with encoded sRGB PNG and JPEG input. Omission of either declaration component MUST fail without output.
2. Dedicated flags and equivalent TOML, JSON, and inline declarations MUST resolve to the same input controls and produce equal raw simulation buffers within the same executable/backend. Explicit `false` MUST count as supplied.
3. A look preset MUST NOT satisfy a missing declaration.
4. Run linear prepared TIFF and OpenEXR with explicit declarations. Retained ICC bytes and an integer or float container MUST NOT change those declarations.
5. Run a real RAW fixture through all supported white-balance modes. Compatible declarations MUST preserve RAW pixels; incompatible flags and early incompatible edits MUST fail before decode.
6. Use RAW provenance from existing acceptance fixtures.
7. Exercise fixed layers with reordered unrelated flags, repeated presets, repeated files, duplicate inline edits, mixed declaration sources, and dedicated input overrides. An explicit value equal to its default MUST remain explicit.
8. Legacy baseline leaves MUST NOT acquire new stock/calibration protection. Invalid earlier sources MUST still fail.
9. Reject `--route` with `--scan-film` at argument parsing in dry run and normal processing. Resolve each supported route with and without a no-op `--set` source.
10. Route, `io.scan_film`, and runtime print-profile selection MUST agree. Exercise negative-conversion compatibility through the real CLI.
11. Render stochastic Spektra, Procedural, Dye, and Fotufilm Film with seeds `0` and `4294967295` under each engine's admitted configuration. Repeating a render with the same seed MUST match its raw output in the same executable/backend.
12. Exercise seed `42` through `--iters`. Verify that `42` and another seed change a known stochastic fixture under the owning grain contract.
13. Negative, fractional, malformed, and overflowing seeds MUST fail. This evidence MUST NOT be presented as cross-backend numerical identity.
14. Compare JSON and text dry run for raster, RAW, preview constraints, dedicated overrides, and Procedural custom versus named-profile controls. Verify typed requested/effective values and winning sources.
15. A dry run without `--output` MUST resolve the same parameters, report `output.path` and `output.format` as `null` unless `--format` was supplied, and create no file. Only winning non-active explicit edits MAY warn.
16. Warning output MUST leave JSON parseable. Default dry run MUST remain JSON when redirected or attached to a terminal.
17. Both formats MUST preserve image and raw-output sentinels and report unchecked outcomes.
18. Run field discovery without profiles or a device. Verify the long-edge crop convention, input transfer meaning, and grain conditions.
19. Exercise a crop on a non-square input through normal processing; verify dimensions against the existing geometry contract.
20. Run help and parser scenarios on separately built default-feature `spektrafilm` and `precision-f64` `spektrafilm-f64`. Exercise normal CPU image output with stdout empty.
21. Run explicit WGPU image processing where a usable adapter is available, and distinguish hardware from software execution. Apply [photo-publication acceptance](../photo-export/spec.md#publication-acceptance) to `process`.
22. Exercise [stage artifact acceptance](stage-artifacts.md#acceptance), including frozen-state validation, one-pass versus continued numerical comparison, and publication failures. Run the same image and artifact operations with debug off and on.
23. Compare stdout, structured dry-run output, numerical payloads, exit status, and output file sets. Verify that only stderr detail changes unless an independent report was explicitly requested.
24. Reject retired and abandoned capture options and value-bearing debug syntax. Native research capture acceptance belongs to the instrument contract, not `process --debug`.

Acceptance MUST identify the source commit, both binary identities and build configurations, input provenance, complete invocation, exit status, stdout/stderr, and observed files or pixel comparisons. Specs and parser-only results MUST NOT count as image-processing acceptance. Unavailable RAW or GPU environments MUST be reported as unverified, not passed.

## Workflow contract

Both `spektrafilm` and `spektrafilm-f64` MUST expose the same command syntax and discovery behavior. Their computation precision contracts MUST remain distinct. The [parameter contract](#parameter-contract) owns preset selection, parameter overrides, field discovery, and dry run.

This capability owns help, version identification, data-directory selection, command status, and human feedback. It MUST NOT add shell completion, interactive prompts, a TUI, or a replacement argument parser. LUT publication and quality rules remain owned by [LUT delivery](../lut-delivery/spec.md).

## Help and identity

Every public command and option MUST have a help description. Descriptions MUST identify the purpose, expected value form, and applicability of the option. Dimensional numeric options MUST state their units.

Options with finite choices MUST show the choices from their authoritative definition. A field with an open vocabulary MUST explain how to discover or supply its values.

`process` help MUST group options by input and RAW loading, look and workflow, output, and execution and diagnostics. LUT build help MUST distinguish bundle configuration, delivery, and quality assessment. Long help MUST include a basic preset process command, a parameter override command, and the relevant discovery commands.

It MUST distinguish scanner color controls from saving color conversion. The preset list help MUST state the default source selection, the preset library directory, and the group order that resolves a bare preset name.

Signed numeric options MUST accept a negative number as the following argument. This rule includes `process --raw-tint -5` and `lut build OUT --exposure-ev -1`. Accepting this argument form MUST NOT relax numeric domains or cross-field validation.

`lut build` MUST retain its positional output-directory argument `OUT`. Its help MUST describe that argument as the parent of the generated bundle directory. README and packaged CLI usage examples MUST use the same form.

`--params` help MUST describe only its runtime-parameter snapshot input.

Both executables MUST support `--version`. Version output MUST identify the executable name, application version, and native CPU computation precision. It MUST NOT claim that a GPU was selected or exercised.

Version and help MUST work without a data directory, input file, image decode, or compute device.

## Data-directory selection

An explicit `--data-dir` MUST select the data directory. It MUST take precedence over `SPEKTRAFILM_DATA_DIR`. A nonempty `SPEKTRAFILM_DATA_DIR` MUST select the directory when the flag is absent.

An empty environment value MUST fail with a diagnostic that identifies the variable. Relative paths MUST resolve against the invocation's working directory.

A directory selected by a flag or environment variable MUST exist and be a directory. An invalid explicit selection MUST fail. It MUST NOT fall back to bundled data or the working directory.

Supplying the literal path `data` explicitly MUST still count as an explicit selection.

When neither source is supplied, automatic discovery MUST try `data` beside the executable, `../share/data` beside the executable, `../Resources/data` beside the executable, and `data` in the working directory, in that order. It MUST select the first existing directory. Exhausting these candidates MUST fail with a diagnostic that lists the attempted locations and explains `--data-dir` and `SPEKTRAFILM_DATA_DIR`.

Selection MUST NOT change the working directory. Commands MUST validate the resources they consume within the selected directory. Missing required resources MUST NOT trigger selection of another directory.

A profile-listing command MUST fail if the profile directory cannot be read or a listed profile cannot be loaded. A valid empty profile directory MAY produce an empty list.

Commands that consume data MUST use this selection policy. Built-in preset listing MUST remain available without data when no explicit selection is supplied. Preset library listing MUST read the preset library directory defined in the [preset design](../presets/design.md) and MUST NOT require a data directory.

If it receives a flag or environment selection, it MUST validate that selection without claiming to validate profile contents. Help, version, and field discovery MUST NOT require data.

The process dry-run report MUST expose the selected data location as defined in the [parameter contract](#discovery-and-dry-run). A data-related failure MUST identify the selected or rejected path. Absolute-path reporting here MUST NOT change the separate privacy contract for [telemetry](../telemetry/spec.md).

## Errors and command status

Invalid command syntax MUST return status `2`. A failed command after successful argument parsing MUST return status `1`. A completed command MUST return status `0`.

Existing telemetry-report persistence exceptions MUST follow the [telemetry contract](../telemetry/spec.md#local-surfaces).

An error MUST identify the offending option or input and explain the corrective action when it is known. Unknown parameter paths, preset names, module prefixes, and workflow routes MUST offer up to three nearby canonical candidates when a sufficiently close match exists. Otherwise, they MUST identify the relevant discovery command or help option.

Candidate suggestions MUST NOT change the supplied value, accept an invalid invocation, introduce aliases, or search the filesystem for an unknown preset name.

CLI errors MUST use stderr. Machine results MUST use stdout. Progress, timing, selected-backend messages, and completion notices MUST use stderr.

Human messages MUST NOT be inserted into JSON or TOML results. Normal image processing MUST continue to leave stdout empty.

LUT build MUST announce the start of baking and, when requested, the start of quality assessment on stderr when attached to a terminal. These messages MUST name the current stage. They MUST NOT invent a percentage, ETA, or measured duration.

Completion and quality summaries MUST remain available on stderr when redirected. This change MUST NOT add a JSON LUT-result protocol or alter the meaning of quality assessment.

## Workflow acceptance

The two executables MUST accept negative tint with a custom RAW white balance and valid temperature using separate option and value arguments. LUT exposure MUST accept a separate negative value. The resulting values MUST match the equivalent `--option=-VALUE` invocation.

Invalid numeric domains MUST still fail.

Help MUST explain all public options and the LUT `OUT` argument. Every README and packaged CLI example MUST parse using the executable it names. Version and help MUST succeed outside the repository with no data discovery or device initialization.

An explicit missing directory MUST fail even when working-directory data exists. An invalid environment selection MUST fail even when bundled data exists. A valid flag MUST override an invalid environment selection.

Explicit `--data-dir data` MUST NOT become automatic discovery. A selected directory missing required profiles MUST fail without using another directory. Failure to read or load profiles MUST return a nonzero status.

Automatic discovery MUST exercise relocated executable-relative data, working-directory data, and exhaustion of all candidates. Built-in preset listing MUST succeed without data when no explicit selection exists.

A misspelled known field or preset name MUST fail and suggest its nearby canonical spelling, including a library name. An unrelated unknown value MUST fail without a misleading correction. Existing invalid-option suggestions MUST remain available.

JSON and TOML discovery, dry-run, render, and inspect output MUST remain parseable with stderr captured separately. LUT bake and quality messages MUST use stderr. Normal image rendering MUST preserve its stdout behavior and output pixels.

## Lab scan recipe cutover

The recipe writer MUST emit `spektrafilm-rs-params-6`. Versions 1–5 MAY migrate only at their declared legacy boundary under the [lab scan contract](../film-chain/lab-scan/spec.md#point-wise-consumers-and-migration). Historical scanner provenance MUST match the built-in model before replacement.

Version 5 MUST NOT reopen older retired-field migrations. Current recipe parameters MUST reject `scanner.lamp`; independent imported-capture illumination belongs to `film_render.convert.scan_illuminant`.
