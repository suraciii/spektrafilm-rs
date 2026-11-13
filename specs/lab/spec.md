# SpektraFilm lab

## Purpose

`spektrafilm-lab` is the measurement instrument of the film-simulation laboratory. It quantifies delivered images, compares two quantified images, and publishes the comparison as an offline report.

It answers three questions:

- What is in this image, in a declared domain, unit, and population?
- How do two images differ, and where does the difference exceed the measurement's own uncertainty?
- What retained evidence supports that statement?

It does not decide what an image looks like. Rendering belongs to the renderer. Judgement belongs to a policy the caller declares.

The primary use is an A/B evaluation of two renderer revisions: a script or an agent renders the same inputs with two builds, then hands both image sets to the lab and receives one quantified report. The lab never invokes the renderer for that work.

## Authority

- This document owns the lab product boundary: its scope, its laws, its analysis endpoints, `compare images`, analysis profiles, and the report content contract.
- The [research machine interface](../instrument/spec.md) owns the shared document registry, serialization, publication, and the machine-output contract.
- The [measurement protocol](../benchmark/metrics/spec.md) owns metric meaning, domains, estimators, populations, and availability reasons.
- The [benchmark product](../benchmark/spec.md) owns run orchestration, evaluation policies, and prepared-image review.
- The lab MUST NOT own rendering mathematics, a second parameter language, a second metric implementation, a second record format, or a universal appearance score.

## Laws

1. The lab does not produce the image it measures. Every operation that generates pixels belongs to another binary.
2. One endpoint performs one semantic operation. An endpoint MUST NOT invoke the next endpoint or infer a downstream action from a result shape.
3. A retained record is immutable and self-describing. It names its kind, its schema version, and the identities that produced it.
4. An unavailable value is never zero and never an absent row without a reason.
5. An input is never silently repaired. The lab MUST NOT resize, crop, pad, clamp, extrapolate, or infer geometry from file names, dimensions, labels, or colors.
6. A comparison reports change. It ranks no candidate, selects no winner, and asserts no accuracy.
7. Independence is a build property. The lab binary links no renderer.

## Scope

| The lab owns | The lab consumes | The lab MUST NOT do |
|---|---|---|
| Declared regions and their geometry authority | Delivered raster images with an explicit color and transfer declaration | Decode by extension, infer a color space, or guess a transfer state |
| Sampling, metric execution, and native-stage statistics | Retained stage-evidence bundles | Render, re-render, or reconstruct an absent observation |
| Joins, deltas, condition differences, and findings | Two retained records | Select a candidate, apply an unrequested policy, or invent a score |
| Offline reports over retained comparisons | One completed comparison | Recalculate a metric or read a source image |

## Binaries

Three binaries serve the laboratory. Each owns one question.

| Binary | Owns | Links a renderer |
|---|---|---|
| `spektrafilm-lab` | analysis, comparison, reporting | no |
| `spektrafilm-f64` | f64 reference export; its experimental research commands render a declaration into retained evidence | yes |
| `spektrafilm-bench` | evaluation policy, visual review, photo report | no, it starts the renderer as a process |

The lab MUST NOT start a renderer, and it MUST NOT link the renderer's crates.

A research workflow connects them through files:

```text
declaration → spektrafilm-f64 → images and evidence → spektrafilm-lab → comparison → report
```

### Endpoints moved out of the lab

`spektrafilm-lab` no longer renders a chain, a suite, or an image. These endpoints keep their names and move to `spektrafilm-f64`; only the executable changes:

| Removed from `spektrafilm-lab` | Replacement |
|---|---|
| `run suite`, `run image` | `spektrafilm-f64 run suite`, `spektrafilm-f64 run image` |
| `target`, `fixtures` | `spektrafilm-f64 target`, `spektrafilm-f64 fixtures` |
| `measure run` | `spektrafilm-f64 measure run` |
| `experiment run`, `experiment export` | `spektrafilm-f64 experiment run`, `spektrafilm-f64 experiment export` |
| `spectral evaluate` | `spektrafilm-f64 spectral evaluate` |
| `fit` | `spektrafilm-f64 fit` |

This revision removes the endpoints. The lab MUST NOT accept them as aliases, MUST NOT forward them to another process, and MUST fail an invocation with `argument_syntax` naming the replacement.

The endpoint move and the crate split MUST land together. Moving the endpoints alone would still link the renderer, and splitting the crate alone would leave handlers whose only implementation is the renderer. A silently forwarded command would make the renderer a hidden runtime dependency of the measurement instrument.

A retained `run`, `evidence`, `reading`, or `comparison` record remains readable by the lab after the move. Record compatibility is bound to the kind and its schema version, never to the executable that wrote it.

## Command surface

The canonical grammar is owned by the [research machine interface](../instrument/spec.md#commands). This document owns what each lab endpoint means. The options of `compare images` are listed under [`compare images`](#compare-images).

| Endpoint | Owns | Does not do |
|---|---|---|
| `schema`, `validate` | document shape and locally decidable dependencies | render, measure, or guarantee an execution |
| `regions grid` | expand one finite grid declaration into explicit regions | read an image, render, or score |
| `sample` | read declared windows from one source | render, resize, infer geometry, or score |
| `compare images` | quantify two delivered image sets and join them | invoke a renderer, select a candidate, or report |
| `compare runs` | quantify two completed runs and join them | invoke a renderer, compare readings, or select |
| `compare readings` | join two retained reading records | compare runs, render, or select |
| `compare pairs` | create the explicitly requested pairs of one run | compare two runs, infer a pair, or render |
| `inspect` | project retained comparison evidence | recalculate a metric or evaluate a policy |
| `report comparison` | present one retained comparison | accept a manifest, render, or measure |

The result object, the exit status, and the progress rules are the shared [machine-output contract](../instrument/spec.md#machine-output).

## `compare images`

`compare images` quantifies two delivered image sets and publishes one version-mode comparison. It reads images that already exist. It MUST NOT resolve a suite, a target, a renderer, or a configuration.

```text
IMAGE COMPARISON OPTIONS:
  (--roi ID=X,Y,W,H[:ROLE] ... | --regions FILE)
  [--analysis ID | --protocol FILE]
  [--report]
  [--map-scale N]
  [--case-id ID]
```

`--case-id` applies only to the single-pair form. Every other option applies to both forms.

Both sides declare the same color space and transfer state, because one invocation compares one rendering pipeline. A caller that compares different declarations runs two invocations.

### Pairing

The single-pair form compares one file with one file.

The directory form compares two directories. The two directories MUST contain the same file names and no other regular files.

A name present on one side only MUST fail as `incompatible_image_sets`, naming both lists. A regular file whose extension is not an accepted image extension MUST fail as `unsupported_image_file`. The lab MUST NOT skip a file it cannot classify.

Accepted extensions are `.png`, `.tif`, `.tiff`, and `.exr`.

Pairing is by exact file name, never by order, modification time, or similarity. Equal dimensions MUST NOT establish correspondence; a caller that names two unrelated files declares that comparison itself.

### Case identity

One pair is one case. The case identity is the file stem in the directory form and the baseline file stem in the single-pair form.

The identity MUST match `^[A-Za-z0-9][A-Za-z0-9_.-]*$`. A stem that does not match MUST fail as `invalid_case_identity`, naming the file. The lab MUST NOT sanitize it into a different identity.

Two stems that produce one identity MUST fail as `duplicate_case_identity`.

`--case-id` overrides the identity in the single-pair form and MUST be rejected in the directory form.

### Geometry

`--roi ID=X,Y,W,H[:ROLE]` declares one region directly and repeats.

`X` and `Y` are nonnegative integers; `W` and `H` are positive integers. `ROLE` is a [region role](../instrument/spec.md#regions-document); it defaults to `other`. `full_image` is reserved and MUST be rejected as an authored identity.

`--regions FILE` declares the geometry as an authored regions document. It is mutually exclusive with `--roi`, because one comparison has one geometry authority.

A generated region uses the `render` coordinate space. An authored regions document MUST declare `render` geometry; a `source` or native-stage region MUST be rejected with `unsupported_coordinate_space` before any pixel is read, because a delivered raster has no source declaration.

Omitting both options measures the implicit `full_image` population only.

Every region, generated or authored, becomes the retained `regions.json` of the comparison. It is the only geometry authority for the measurement. A region is a rectangle in `render` coordinates; a mask geometry that a run region can carry is not an input to `compare images` and MUST be rejected rather than re-rooted.

A region that leaves the image bounds MUST fail as `region_out_of_bounds`, naming the region and its case.

### Analysis

`--analysis ID` selects a built-in [analysis profile](#analysis-profiles), and the default is `--analysis lab-appearance-v1`. `--protocol FILE` selects an authored protocol document. The two are mutually exclusive.

Both forms materialize their protocol into `protocol.json` inside the comparison. The comparison MUST record the analysis identity and the protocol digest. A report MUST NOT depend on the caller's original protocol file.

A family that needs a declared probe MUST refuse to measure without it. It MUST retain an unavailable row naming the missing probe, and it MUST NOT fall back to a default probe geometry. The other selected families MUST still measure.

The measurement families of an analysis profile MUST be compatible with the region role they apply to. A `flat_mean` grain residual declared on a region that is not `flat_field` MUST fail as `incompatible_measurement_role`. From a `gaussian_high_pass` declaration the lab MAY measure any region on which it is declared.

The lab MUST NOT substitute one residual method for the other. The substitution would change the measured signal without changing the declaration.

### Decoding

Every raster the lab reads is decoded by one reader that the measurement library owns. `sample --source`, `compare images`, and a retained-master comparison MUST use that reader.

The reader answers one question: what samples does this file contain under the interpretation the caller declared? It MUST NOT answer the renderer's question, which is what the photographer intended the file to be. It therefore MUST NOT apply an embedded ICC profile, an EXIF orientation, a RAW interpretation, camera metadata, or any other evidence the caller did not declare.

A container type whose normalization the reader cannot state MUST be rejected as `unsupported_image_layout`. A silent default is not an acceptable answer for a type the lab has not defined.

| Container | Normalization |
|---|---|
| 8-bit and 16-bit integer | divide by the type's maximum |
| 32-bit float, 64-bit float, half | widen exactly to f64, no scaling |
| single channel | replicate to three equal channels |
| four channels | ignore alpha; never composite |
| `.exr` | half and float channels widen exactly to f64; no tone map, no exposure |

The reader MUST NOT read a sidecar file, a neighbouring metadata document, or a filename convention that changes the interpretation. A declaration comes from the caller or from the retained record that named the file, and from nowhere else.

The lab MUST NOT clip a value before measurement, and it MUST NOT rescale one container to match another.

### Images

Each side MUST decode once and retain one display copy per case as a report artifact. The measurement population keeps the decoded floating samples; the display copy exists only for inspection.

The two sides of one pair MUST have identical dimensions. A mismatch MUST fail as `dimension_mismatch`, naming the case. The lab MUST NOT resize, crop, or pad to reconcile a mismatch.

A nonfinite sample MUST fail as `nonfinite_samples`. The lab MUST NOT replace it.

### Publication

One invocation publishes one comparison directory:

```text
OUT/
  comparison.json
  protocol.json
  regions.json
  cases/<case-id>/...
  report/                 when --report is given
```

The comparison MUST use the version shape: `baseline` and `candidate` source blocks, and one case per pair. It MUST NOT carry a `mode` field. `mode` marks a comparison shape, and this comparison has the version shape.

Each source block MUST declare `kind: images` and carry its own name, the declared color space and transfer state, one file per case, and the digest of that declaration. The [comparison record](../instrument/spec.md#comparison-record) owns the field list.

The source block MUST NOT contain a run record, a target record, a suite record, or an executed command. A delivered image has no command, and a fabricated one would corrupt the evidence chain.

The source name is the file stem in the single-pair form and the directory name in the directory form. It identifies the side to a person; the declaration digest is what verifies it.

Each case MUST retain its measurements, regions, automatic regions, review artifacts, and difference maps. A region readout MUST identify its pair as `baseline/candidate`.

Every case declares seed `0`. A delivered image is one realisation, so the lab MUST NOT claim a seed population and MUST NOT publish a cross-seed statistic.

`condition_differences` MUST report the declared color space, the transfer state, the dimensions per case, and the analysis identity. It MUST report a difference in any of them. The lab MUST NOT present a comparison across a changed declaration as a version difference.

### Difference maps

A difference map is retained evidence for inspection, not a measurement population. The retained maps MAY be reduced by the declared `--map-scale N`, a positive integer that box-averages each map to one sample per `N × N` block.

Measurement populations MUST NOT be sampled, reduced, or aggregated by `--map-scale`. Every scalar measurement MUST be computed from the full-resolution population.

`--map-scale` MUST be recorded in the comparison.

### Report composition

`--report` publishes a report over the comparison this invocation just wrote. It writes `OUT/report/`, inside the comparison directory, so the invocation still has one destination and one publication boundary.

This is a declared composition, not a workflow. It has one fixed sequence and accepts no graph or conditional.

The report MUST fail as a whole. A report failure MUST leave no `OUT/report/` directory, and the comparison it measured MUST remain complete and readable. A comparison failure MUST write nothing.

`report comparison` remains a separate endpoint. A caller MAY invoke it later over a retained comparison with no difference in the resulting page.

## Analysis profiles

An analysis profile is a named, versioned generator of one complete protocol and one region family policy. It is recorded by identity; its materialized protocol and regions are the retained authority.

| Profile | Full-image families | Region families | Intended use |
|---|---|---|---|
| `lab-appearance-v1` | numeric, color, tone, flip | numeric, color, tone | default A/B of two finished renders |
| `lab-color-tone-v1` | numeric, color, tone | numeric, color | deterministic builds with spatial and stochastic effects off |

A profile id MUST equal the `id` of the protocol it materializes. The lab MUST NOT materialize a different protocol under a benchmark protocol id: `appearance-v1` names the standard run protocol, and a second set of constants under that id would make a policy unable to bind to both.

The selected families are part of the protocol the lab materializes, so they contribute to the protocol digest. Two comparisons that measured different families MUST NOT share a digest.

A probe family is added to this list only with the probe that makes it measurable. A `grain` profile without flat fields, or a `halation` profile without a frozen source core, would publish an unavailable row for every region. A caller who has such a probe authors the protocol and declares the probe geometry explicitly.

A profile MUST materialize the same bytes for the same identity on every platform and every invocation. The generated regions document MUST use the profile's region families, except that a `flat_field` region selects `numeric` and `grain`.

A profile is a generator, not a document kind. A caller who needs a declaration the generator cannot express authors a protocol and a regions document and passes them explicitly.

Adding a family to an existing profile identity MUST NOT happen. A changed measurement is a new profile identity.

## Report

`report comparison` presents one retained comparison. It MUST work when the renderer, the original images, and the source directories are unavailable, subject to the redistribution limits the comparison recorded.

A report MUST show:

- the analysis identity, the protocol digest, and the lab version;
- the baseline and candidate source identities, including each file digest;
- the condition differences, before any measurement;
- for each case and region, the two images or region crops, the difference maps, and the measured values with their domain, unit, population, estimator, and sample count;
- every unavailable measurement with its stable reason;
- the automatic detail regions used for navigation, labelled as display aids;
- a navigation that selects exact case, region, and pair identities.

A report MUST NOT:

- recalculate a metric, or backfill a value the comparison did not retain;
- present a version difference as an accuracy or quality claim;
- omit an unavailable reason that the comparison retained;
- substitute another image for one it is not permitted to redistribute; it MUST name the omission instead.

`inspect` projects the same evidence without producing a page. It MUST accept a comparison published by any `compare` endpoint.

## Independence

The lab MUST build and run without a graphics device, a raw decoder, a renderer executable, or the renderer's crates.

The lab's transitive dependency set MUST NOT contain `spektrafilm-core`, `spektrafilm-gpu`, `spektrafilm-backend`, `spektrafilm-raw`, `spektrafilm-model`, or `wgpu`.

The lab requires reference precision unconditionally. It MUST NOT depend on a caller-enabled precision feature to select the precision it measures at.

The evaluation library currently builds a native perceptual-difference evaluator through C++ and dynamic loading. That is a real build dependency of the measurement library, not a lab choice. Removing it for builds that never select `flip` is a follow-up of the library, and this document MUST NOT claim the lab is buildable without a C++ toolchain before that change lands.

The renderer reads its input and writes its output through a native codec bridge in `spektrafilm-core`. The lab MUST NOT use that bridge, for measurement or for any other purpose.

The two codecs serve two contracts. The divergence is deliberate.

| | Renderer input and output | Lab raster reader |
|---|---|---|
| Question it answers | what did the photographer intend this file to be? | what samples does this file contain? |
| ICC profile, EXIF orientation, camera metadata | appropriate | forbidden |
| Container coverage | as wide as the codec library offers | exactly the containers whose decoding the lab can state |
| Owner | `spektrafilm-core` | the measurement library |

A shared codec would force one of two errors. Either the lab would inherit an interpretation it is forbidden to apply, or the renderer would lose an interpretation it needs. Sharing the reader is therefore not a refinement of the boundary; it is the boundary being wrong.

A caller who needs the renderer's reading of a file does not need a shared codec. The run record already retains it: `source_raw` is the floating master that the renderer's own input route produced, and `sample --from-run` reads it.

## Acceptance

1. `cargo tree -p spektrafilm-lab` names none of the forbidden crates.
2. The lab builds and runs on a host with no GPU, no raw decoder, and no renderer installed.
3. `spektrafilm-lab report comparison` succeeds with the renderer removed from the machine.
4. `spektrafilm-lab run suite ...` fails with `argument_syntax` and names `spektrafilm-f64 run suite`.
5. `spektrafilm-lab measure run ...` fails with `argument_syntax` and names `spektrafilm-f64 measure run`.
6. Two identical images produce a comparison whose every available delta is zero and whose every unavailable row carries a reason.
7. A pair whose dimensions differ fails with `dimension_mismatch` and writes no comparison.
8. A directory pair with a name on one side only fails with `incompatible_image_sets` and names both lists.
9. Two file names that reduce to one case identity fail with `duplicate_case_identity`.
10. `--roi skin=820,400,200,300 --roi shadow=100,900,300,200` produces a regions document with those two regions in declaration order and a comparison whose `regions.json` is that document.
11. A `--regions` document that declares `source` geometry fails with `unsupported_coordinate_space` before reading a pixel.
12. A generated region outside the image fails with `region_out_of_bounds` and names the region.
13. The same arguments produce byte-identical `comparison.json`, `protocol.json`, and `regions.json`.
14. `--analysis ID` materializes a protocol whose `id` is that same string, and two runs of the same arguments write byte-identical `protocol.json`.
15. A region that selects `halation` without a declared probe retains an unavailable row with `missing_probe_geometry`, and the region's other families still measure.
16. `--map-scale 4` changes no scalar measurement and records the scale.
17. `--report` publishes `OUT/report/`. A comparison that cannot be reported is refused before the report destination exists, so no report is left behind and no byte of the comparison changes.
18. `inspect` over a `compare images` comparison returns the retained measurements without reading an image.
19. A report opens offline with the original images deleted and shows every source digest it retained.
20. An EXR and the float TIFF written from the same render decode to the same samples.
21. A single-channel PNG decodes to three equal channels, and a four-channel PNG ignores its alpha instead of compositing.
22. A `<path>.json` sidecar beside a measured image does not change the reading.
23. `spektrafilm-f64 compare runs ...` fails with `argument_syntax` and names `spektrafilm-lab compare runs`.
24. A `flat_mean` grain residual on a region that is not a flat field is refused with `missing_probe_geometry`; the same declaration on a `flat_field` region measures.

## Current implementation gap

- The binary split is implemented: `spektrafilm-lab` is its own package, links no renderer, and the render endpoints reject a lab invocation naming the owning binary.
- A region is a rectangle. Mask geometry is rejected; reading mask files on a delivered raster is not implemented.
- The report composition removes a half-written report directory, but every failure the report can be shown to hit is refused before that directory exists, so the cleanup is defensive and unexercised.
- The lab reader decodes PNG, TIFF, JPEG, WebP, and EXR. A container layout with no stated normalization is rejected.
- The evaluation library builds its native perceptual-difference evaluator with a C++ toolchain.
