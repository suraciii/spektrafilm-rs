# Native observations

## Status and authority

This document owns instrument-owned native numerical observations. Dedicated research endpoints MUST observe the shared core execution and publish their own retained evidence.

The instrument owns research observation configuration, retention, and readers. The shared core owns the observed numerical execution. [Experiment runs](experiment/spec.md) and [`measure run`](spec.md#measure-run) attach the common core observer to their actual execution; they MUST NOT invoke a process capture option, implement a second stage walk, or reconstruct pipeline mathematics.

Ordinary `process`, `run suite`, and `run image` do not offer optional native capture. There is no implicit replacement capture path or rerender when observations are absent.

A native observation bundle is research evidence, not a resumable rendering state. Its terminal manifest remains `stage-evidence.json`; retaining that wire filename does not retain a command option. It MUST NOT be renamed or represented as `.sfstage`, and it does not satisfy the [stage artifact](../cli/stage-artifacts.md) contract.

A `.sfstage` directory with `stage.json` is one always-resumable film or print state, not an observation dump or a directory of all named taps. Human debug output is separately governed by [debug diagnostics](../cli/spec.md#debug-diagnostics) and cannot request observations.

## Observed execution

Capture MUST observe the values that actually flowed through the executed route: full-source exposure application, profile-prepared print normalization, and the named tap boundaries. It MUST NOT change parameters, RNG, exposure, execution route, backend, numerical model, or output pixels. It MUST add no capture work to observation-off rendering.

Pixel evidence MUST stay out of ordinary telemetry; explicit observation persistence failure is distinct from optional telemetry failure.

Dedicated research observation uses the CPU reference execution route; f64 reference producers use the instrument's f64 build. A GPU request MUST fail with actionable guidance before execution, never silently switch backend. Preview mode MUST fail before execution: preview bounding resizes the decoded source before metering, which the version-1 manifest cannot describe without a false full-original metering claim.

Ordinary pipeline crop and resize remain supported through stage geometry. A permitted dry run resolves the owning request but observes no values and writes no bundle; `experiment run` separately rejects dry run. A producer with repeated execution MUST identify the execution actually observed rather than combine unrelated runs.

## Publication

A bundle contains one f64 buffer per available stage and the terminal `stage-evidence.json` manifest. The manifest is the completeness token. A producer MUST remove any stale manifest in a replaceable destination before execution and write the new manifest last and atomically, using a temporary file plus rename after all required buffers, owning image output, and any requested raw output succeed.

The owning endpoint's destination replacement policy still applies. A run that fails before publication MUST leave no completeness manifest; an output or path failure MUST never appear as complete evidence. An incomplete or unwritable required bundle forces a nonzero command status.

Bundle files and publication temporaries MUST NOT alias invocation inputs, image/raw outputs, parameter sources, or diagnostics. Reject conflicts before execution or overwriting protected paths. An invalid destination rejected before capture MUST NOT publish a success manifest.

A failed manifest, when retained as structured failure evidence, is not a completeness token and MUST never be consumed as a completed observation.

## Manifest schema

The version-1 root MUST carry:

- `schema_version`: `1`.
- `execution_status`: `completed` or `failed`.
- `host_precision`: `f32` or `f64`.
- `workflow_route`, `scan_output`, and `normalization_mode`: `"profile_prepared"` for the latter.
- `exposure`, `print_normalization`, `stages`, and `errors`.

`stages` is an array of six records in fixed order: `rgb_in`, `log_e_film`, `cmy_film`, `log_e_print`, `cmy_print`, `rgb_out`. These are the existing named taps. `rgb_pre` is outside this contract.

Every record carries `id` and `availability` (`available`, `unavailable`, or `not_applicable`). An available record carries:

- `path`: bundle-relative `<id>.f64` filename.
- `sha256`.
- `layout`: `width`, `height`, `channels: 3`, `dtype: "f64"`, `endianness: "little"`, and `channel_order`.
- `domain`, `units`, and `geometry`.
- `color` for RGB-order records only.

A non-available record carries only `id`, `availability`, and a stable snake_case `reason`. It MUST contain no path, hash, layout, domain, units, geometry, or color keys. `errors` is an array of precise message strings naming the failing stage.

## Buffer and channel semantics

Buffers are row-major HxWx3, channel-interleaved f64 little-endian: the raw-output byte layout with host values widened to f64. `host_precision: "f32"` declares f32 host storage; f64 persistence is not an f64 computation claim. The wire format retains both host-precision declarations even though dedicated reference producers are f64-only.

The film and print boundaries share one record axis: the cyan-/magenta-/yellow-forming records. `filming::develop` maps `log_raw[c]` to `density_cmy[c]` without permutation. Consumers MUST NOT permute channels across stages.

| Stage | `channel_order` | `domain` | `units` |
|---|---|---|---|
| `rgb_in` | `RGB` | `input_rgb` | `relative` |
| `log_e_film` | `CMY` | `record_exposure` | `log10` |
| `cmy_film` | `CMY` | `record_density` | `optical_density` |
| `log_e_print` | `CMY` | `record_exposure` | `log10` |
| `cmy_print` | `CMY` | `record_density` | `optical_density` |
| `rgb_out` | `RGB` | `output_rgb` | `relative` |

Log-exposure records are log10 record exposure before development, not developed density and not image color. CMY-order records have no `color` key. RGB-order records carry `color: {color_space, transfer}`; `transfer` is `"encoded"` exactly when the matching `io.input_cctf_decoding` / `io.output_cctf_encoding` is true.

## Geometry

`geometry` is `{coordinate_space}`. The allowed values identify the actual producer grid:

| Value | Meaning |
|---|---|
| `source_pixels` | decoded source raster entering the pipeline |
| `working_pixels` | prepared grid before any grain-owned crop and rescale |
| `film_frame_pixels` | full-film frame grid while a delivered-region crop is pending |
| `grain_aperture_pixels` | supersampled grain-aperture grid before exposure collapse |
| `output_pixels` | delivered pre-saving grid |

Grain readout MUST label each intermediate by the grid it actually produces. With no pending spatial-effect crop, cropped or resized intermediates use `output_pixels`, not the preceding full-source working grid. Equal dimensions never establish a coordinate mapping for regional readers.

## Exposure and print normalization

`exposure` describes the camera exposure application of the executed run. When available it carries `auto_enabled`, `method`, `source_dimensions` (the full-source metering grid before crop/resize), `metered_ev`, `metered_ev_source` (`"measured"` or `"disabled_zero"`), `compensation_ev`, `applied_ev`, and `gain`.

Disabled automatic exposure MUST report `metered_ev: 0.0` and `metered_ev_source: "disabled_zero"`: an explicitly identified zero metering contribution, not measured physical exposure. `compensation_ev` is the compensation actually multiplied into film exposure by this execution. `applied_ev = metered_ev + compensation_ev`; `gain = 2^applied_ev`.

Routes without a camera exposure application, including `input`, convert-film routes, and mid-chain tap runs, report `not_applicable` with a stable reason and no numbers. `print_normalization` reports `value` (the actual prepared print exposure factor) with `mode: "profile_prepared"` on every route except `input`, which reports `not_applicable` with a reason and no value.

These are observed execution facts, not values reconstructed from authored controls.

## Availability and readers

A stage the executed route does not produce is `not_applicable` with a stable reason: route-skipped film or print stages, conversion without a film exposure boundary, fused grain readout, or a boundary outside the executed tap range. This is not an execution failure.

A required executed stage that fails to capture or write is `unavailable`, adds `errors` entries, and forces `execution_status: "failed"`; such a bundle is never complete evidence. `rgb_out` is the pre-saving pipeline output, equivalent to raw output and including selected post-processing.

Research readers retain `sample --from-evidence` and `sample --from-run`. They validate the manifest and indexed identities and never render, resume a `.sfstage`, reconstruct a missing stage, or add capture to a historical or ordinary run. An absent observation remains explicitly unavailable.

Historical supported native bundles remain readable under this unchanged numerical wire contract; dedicated experiment export additionally requires f64 host precision and finite samples as specified by its owner.

## Acceptance

1. Dedicated experiment and measurement executions observe the shared core path, with no process capture argument or duplicate mathematics, and retain their owning replayable declaration plus observer configuration.
2. Enabling observation leaves the same execution's numerical image output, parameter values, RNG behavior, and route unchanged. Observation-off execution performs no capture work.
3. Manifest order, stage availability, byte layout, channel axes, domains, units, geometry, exposure, and print normalization obey this contract, including f32 host-storage declarations in retained reader fixtures.
4. Publication rejects aliasing, failed output, failed buffer writes, and invalid destinations without a completed manifest; a failed required capture never qualifies as evidence.
5. GPU and preview requests fail before execution without backend substitution. Dry resolution observes no values.
6. Ordinary suite and single-image runs expose no optional native capture and leave native observations explicitly absent. Sampling, inspection, and reporting do not rerender or backfill them.
7. Existing supported research bundles remain consumable by the explicit readers; `.sfstage` and debug text are not accepted as observation bundles.
