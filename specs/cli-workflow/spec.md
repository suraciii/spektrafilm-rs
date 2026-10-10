# CLI workflow

## Contract

Both `spektrafilm` and `spektrafilm-f64` must expose the same command syntax and discovery behavior. Their computation precision contracts must remain distinct. The [CLI parameter language](../cli-parameters/spec.md) owns preset selection, parameter overrides, field discovery, and dry run.

This capability owns help, version identification, data-directory selection, command status, and human feedback. It must not add shell completion, interactive prompts, a TUI, or a replacement argument parser. LUT publication and quality rules remain owned by [LUT delivery](../lut-delivery/spec.md).

## Help and identity

Every public command and option must have a help description. Descriptions must identify the purpose, expected value form, and applicability of the option. Dimensional numeric options must state their units. Options with finite choices must show the choices from their authoritative definition. A field with an open vocabulary must explain how to discover or supply its values.

`process` help must group options by input and RAW loading, look and workflow, output, and execution and diagnostics. LUT build help must distinguish bundle configuration, delivery, and quality assessment. Long help must include a basic preset process command, a parameter override command, and the relevant discovery commands. It must distinguish scanner color controls from saving color conversion. The preset list help must state that it lists built-in presets only.

Signed numeric options must accept a negative number as the following argument. This rule includes `process --raw-tint -5` and `lut build OUT --exposure-ev -1`. Accepting this argument form must not relax numeric domains or cross-field validation.

`lut build` must retain its positional output-directory argument `OUT`. Its help must describe that argument as the parent of the generated bundle directory. README and packaged CLI usage examples must use the same form. `--params` help must describe only its runtime-parameter snapshot input.

Both executables must support `--version`. Version output must identify the executable name, application version, and native CPU computation precision. It must not claim that a GPU was selected or exercised. Version and help must work without a data directory, input file, image decode, or compute device.

## Data-directory selection

An explicit `--data-dir` must select the data directory. It must take precedence over `SPEKTRAFILM_DATA_DIR`. A nonempty `SPEKTRAFILM_DATA_DIR` must select the directory when the flag is absent. An empty environment value must fail with a diagnostic that identifies the variable. Relative paths must resolve against the invocation's working directory.

A directory selected by a flag or environment variable must exist and be a directory. An invalid explicit selection must fail. It must not fall back to bundled data or the working directory. Supplying the literal path `data` explicitly must still count as an explicit selection.

When neither source is supplied, automatic discovery must try `data` beside the executable, `../share/data` beside the executable, `../Resources/data` beside the executable, and `data` in the working directory, in that order. It must select the first existing directory. Exhausting these candidates must fail with a diagnostic that lists the attempted locations and explains `--data-dir` and `SPEKTRAFILM_DATA_DIR`.

Selection must not change the working directory. Commands must validate the resources they consume within the selected directory. Missing required resources must not trigger selection of another directory. A profile-listing command must fail if the profile directory cannot be read or a listed profile cannot be loaded. A valid empty profile directory may produce an empty list.

Commands that consume data must use this selection policy. Built-in preset listing must remain available without data when no explicit selection is supplied. If it receives a flag or environment selection, it must validate that selection without claiming to validate profile contents. Help, version, and field discovery must not require data.

The process dry-run report must expose the selected data location as defined in the [CLI parameter specification](../cli-parameters/spec.md#discovery-and-dry-run). A data-related failure must identify the selected or rejected path. Absolute-path reporting here must not change the separate privacy contract for [telemetry](../telemetry/spec.md).

## Errors and command status

Invalid command syntax must return status `2`. A failed command after successful argument parsing must return status `1`. A completed command must return status `0`. Existing telemetry-report persistence exceptions must follow the [telemetry contract](../telemetry/spec.md#local-surfaces).

An error must identify the offending option or input and explain the corrective action when it is known. Unknown parameter paths, preset IDs, module prefixes, and workflow routes must offer up to three nearby canonical candidates when a sufficiently close match exists. Otherwise, they must identify the relevant discovery command or help option. Candidate suggestions must not change the supplied value, accept an invalid invocation, introduce aliases, or search for an unknown preset ID in a user directory.

CLI errors must use stderr. Machine results must use stdout. Progress, timing, selected-backend messages, and completion notices must use stderr. Human messages must not be inserted into JSON or TOML results. Normal image processing must continue to leave stdout empty.

LUT build must announce the start of baking and, when requested, the start of quality assessment on stderr when attached to a terminal. These messages must name the current stage. They must not invent a percentage, ETA, or measured duration. Completion and quality summaries must remain available on stderr when redirected. This change must not add a JSON LUT-result protocol or alter the meaning of quality assessment.

## Acceptance

The two executables must accept negative tint with a custom RAW white balance and valid temperature using separate option and value arguments. LUT exposure must accept a separate negative value. The resulting values must match the equivalent `--option=-VALUE` invocation. Invalid numeric domains must still fail.

Help must explain all public options and the LUT `OUT` argument. Every README and packaged CLI example must parse using the executable it names. Version and help must succeed outside the repository with no data discovery or device initialization.

An explicit missing directory must fail even when working-directory data exists. An invalid environment selection must fail even when bundled data exists. A valid flag must override an invalid environment selection. Explicit `--data-dir data` must not become automatic discovery. A selected directory missing required profiles must fail without using another directory. Failure to read or load profiles must return a nonzero status.

Automatic discovery must exercise relocated executable-relative data, working-directory data, and exhaustion of all candidates. Built-in preset listing must succeed without data when no explicit selection exists.

A misspelled known field or preset ID must fail and suggest its nearby canonical spelling. An unrelated unknown value must fail without a misleading correction. Existing invalid-option suggestions must remain available.

JSON and TOML discovery, dry-run, render, and inspect output must remain parseable with stderr captured separately. LUT bake and quality messages must use stderr. Normal image rendering must preserve its stdout behavior and output pixels.

## Implementation gap

Current CLI arguments reject separate negative tint and exposure values. Data selection falls back after an explicit invalid path. Profile listing can report a missing directory and return success. Help lacks several descriptions, groups, and examples. Version flags are absent. Domain errors lack candidate or discovery guidance. LUT README examples use unsupported `--out`, and LUT stage notices use stdout.

LUT QA exit semantics, bundle replacement protection, and multi-print naming collisions require a separate behavioral investigation before a publication or quality contract is changed. They are not acceptance criteria for this implementation.
