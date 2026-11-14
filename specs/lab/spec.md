# SpektraFilm lab

## Purpose

`spektrafilm-lab` measures the color science of delivered images and publishes
structured quantitative records. It analyzes one source, compares two, and
samples declared points. It publishes machine-readable records and nothing
else.

It answers three questions:

- What are the color-science properties of this image, in a declared domain,
  unit, and population?
- How do two images differ in color error, neutrality, tone response, and
  color noise?
- What retained evidence supports those numbers?

It does not decide what an image looks like. Rendering belongs to the
renderer. Judgement belongs to a policy the caller declares.

It does not present an image either. The record is the product; a page, a
plot, or a document is a reading of the record and is owned by whoever reads
it. The record MUST nevertheless be complete enough that such a reading needs
no source the lab did not retain.

The primary use is an A/B evaluation of two renderer revisions: a script or an
agent renders the same inputs with two builds, then hands both image sets to
the lab and receives one structured comparison.

## Authority

- This document owns the lab product boundary: its scope, its laws, its
  concepts, its command surface, and the record each command publishes.
- The [measurement protocol](../benchmark/metrics/spec.md) owns metric
  meaning, domains, estimators, populations, and availability reasons. This
  document names the families it selects; it does not define a second
  implementation of one.
- The [research machine interface](../instrument/spec.md) owns the shared
  document registry, serialization, and the machine-output contract.
- The lab MUST NOT own rendering mathematics, a second parameter language, a
  second metric implementation, a second record format, or an appearance
  score.

## Laws

1. The lab does not produce the image it measures. Every operation that
   generates pixels belongs to another binary.
2. One endpoint performs one semantic operation. An endpoint MUST NOT invoke
   the next endpoint or infer a downstream action from a result shape.
3. A retained record is immutable and self-describing. It names its kind, its
   schema version, and the identities that produced it.
4. An unavailable value is never zero and never an absent row without a
   reason. A quantity that does not exist for a source is recorded as
   unavailable with the reason it does not exist.
5. An input is never silently repaired. The lab MUST NOT resize, crop, pad,
   clamp, extrapolate, or infer geometry from file names, dimensions, labels,
   or colors.
6. A comparison reports change. It ranks no candidate, selects no winner, and
   asserts no accuracy.
7. Independence is a build property. The lab binary links no renderer.

## Scope

| The lab owns | The lab consumes | The lab MUST NOT do |
|---|---|---|
| Color-science measurement and its structured record | Delivered raster images with an explicit color and transfer declaration | Render, re-render, or present an image |
| Declared regions and their geometry authority | Retained stage-evidence bundles | Decode by extension, infer a color space, or guess a transfer state |
| Joins, deltas, and distribution statistics | Two retained records | Rank a candidate, apply an unrequested policy, or invent a score |
| | | Manage a document repository, orchestrate a workflow, or describe a graph of invocations |

GUI automation, evaluation policy, benchmark review, and profile authoring are
other capabilities. A question that needs one of them is not a lab question.

## Concepts

Six nouns. A caller holds all six at once, so a seventh displaces one.

| Concept | What it is | Why it is irreducible |
|---|---|---|
| **Source** | One delivered image set, read under one declaration | The thing being measured |
| **Declaration** | Color space and transfer state of the samples | Law 5 forbids inferring it, so the caller states it |
| **Region** | A named rectangle in delivered-pixel space, with a role | Color science is measured on populations, never on a whole image by accident |
| **Family** | One of the measurement families below | Decides what is computed |
| **Record** | The structured result of one invocation | The product |
| **Reference** | A second source, or declared reference values | Color error is relative; absolute analysis does not need one |

### Region roles

A region carries one role. The roles are the closed set the regions document
already declares; this product introduces no second vocabulary.

A family whose meaning depends on the population's structure is measured only
on a role that has that structure. A family that measures a declared quantity
is meaningful on any population and is not restricted.

| Role | Population | Structure required by |
|---|---|---|
| `skin` | a skin patch | — |
| `neutral` | a neutral patch or gray step | `neutrality` |
| `shadow` | a shadow step | — |
| `highlight` | a highlight step | — |
| `color` | a color patch | — |
| `flat_field` | a uniform field | `color_noise` |
| `texture` | a textured field | `color_noise` |
| `edge` | a slanted or straight edge | — |
| `other` | anything | — |
| `full_image` | the whole raster | — |

Only `neutrality` and `color_noise` require a role. A whole image is not a flat
field, so `color_noise` on `full_image` is refused rather than measured over
whatever content the image happens to contain.

The other families need no cell. `tone`, `edge`, `grain`, and `halation`
declare a probe, and the code that reads the probe is the code that refuses a
missing one; restating that rule as a role table would give it two owners.
`numeric`, `color`, `flip`, and `distribution` measure a declared quantity and
are meaningful anywhere.

A family declared on a role it does not apply to MUST be refused with
`incompatible_measurement_role`, naming the family and the role. The refusal
retains an unavailable row, and the region's other families still measure. The
lab MUST NOT substitute a family that does apply: the substitution would
measure a signal the caller never declared.

## Command surface

| Endpoint | Owns | Does not do |
|---|---|---|
| `analyze` | quantify one delivered image set | compare, evaluate a policy, or present |
| `compare` | quantify how two delivered image sets differ | compare two runs, join reading records, select a candidate, or present |
| `regions` | expand one finite grid declaration into explicit regions | read an image, render, or measure |
| `schema` | print the authoritative schema of one document kind | validate a document or measure |
| `validate` | document shape and locally decidable dependencies | measure or guarantee an execution |

`analyze` and `compare` are the product. The other three produce and check the
declarations they consume, and a caller who authors those documents another way
never needs them.

The evaluation workflow and the reading family belong to `spektrafilm-bench`.
That surface compares two completed runs, joins two reading records, selects
evidence pairs from one run, projects a retained comparison, and reads declared
sample points from an image or a retained stage capture.

Those endpoints read a renderer's evidence or evaluate a policy. Neither is a
color-science measurement of a delivered image set.

A lab invocation of one of them MUST fail with `argument_syntax` naming
`spektrafilm-bench`. The lab MUST NOT forward it.

### compare

```text
spektrafilm-lab compare --baseline <FILE_OR_DIR> --candidate <FILE_OR_DIR>
  --declaration <SPACE:encoded|decoded>
  (--roi ID=X,Y,W,H[:ROLE] ... | --regions FILE)
  [--analysis ID | --protocol FILE]
  [--map-scale N]
  --out <DIR>
```

`--declaration` states the color space and the transfer state of both sides.
`--roi` and `--regions` are alternative geometry authorities.

`--analysis` selects a built-in analysis profile and `--protocol` an authored
protocol document; the two are mutually exclusive. `--map-scale` reduces
retained difference maps only.

### analyze

```text
spektrafilm-lab analyze --source <FILE_OR_DIR>
  --declaration <SPACE:encoded|decoded>
  (--roi ID=X,Y,W,H[:ROLE] ... | --regions FILE)
  [--analysis ID | --protocol FILE]
  --out <DIR>
```

`analyze` reads one source and publishes `analysis.json`. Its measurement rows
name no side: one source is one measurement, and there is no difference to
state.

A family that measures a difference between two sources cannot be measured
here. It is recorded as unavailable with `requires_second_source` rather than
computed against the source itself, because subtracting an image from itself
would report a zero that no measurement produced. `color`, `flip`, and the two
`numeric` difference statistics are the families this applies to.

The default profile of `analyze` is `lab-analysis-v1`, whose whole-image
families are the ones a whole image supports. The structural families appear on
the regions a caller declares, for the same reason a whole image is not a flat
field.

`compare` takes the same source, declaration, and geometry options in the
two-source form as `--baseline FILE --candidate FILE`.

Both endpoints measure the same quantity for the same source. An analysis and
the baseline side of a comparison of that source MUST report equal values. `regions` and `validate`
are authoring tools: they produce and check the declarations the product
consumes, and a caller who authors those documents another way never needs
them.

The result object and the exit status follow the shared
[machine-output contract](../instrument/spec.md#machine-output).

### Source

A source is one directory of delivered rasters, or one file, plus a
declaration. Both sides of one `compare` declare the same color space and
transfer state, because one invocation measures one pipeline. A caller that
measures different declarations runs two invocations.

Pairing is by exact file name, never by order, modification time, or
similarity. Equal dimensions MUST NOT establish correspondence.

A name present on one side only MUST fail as `incompatible_image_sets`, naming
both lists. A regular file whose extension is not an accepted image extension
MUST fail as `unsupported_image_file`; the lab MUST NOT skip a file it cannot
classify.

Accepted extensions are `.png`, `.tif`, `.tiff`, and `.exr`.

### Case identity

One pair is one case. The case identity is the file stem in the directory form
and the baseline file stem in the single-pair form.

The identity MUST match `^[A-Za-z0-9][A-Za-z0-9_.-]*$`. A stem that does not
match MUST fail as `invalid_case_identity`, naming the file. The lab MUST NOT
sanitize it into a different identity.

Two stems that produce one identity MUST fail as `duplicate_case_identity`.

### Geometry

A region is a rectangle in the delivered raster's own pixel space. That is the
only coordinate space this product has. The regions document names that space
`render`, which is a name from the renderer's vocabulary that predates this
product: a delivered raster has no render, and the name means "the pixels as
delivered" here and nothing more.

A regions document MUST declare `render` geometry. Every other declared space
MUST be rejected with `unsupported_coordinate_space` before any pixel is read:
a delivered raster carries neither a source declaration nor a working stage.

`--roi ID=X,Y,W,H[:ROLE]` declares one region directly and repeats.
`--regions FILE` declares the geometry as an authored regions document. The two
are mutually exclusive, because one measurement has one geometry authority.

Omitting both measures the implicit `full_image` population only.

`X` and `Y` are nonnegative integers; `W` and `H` are positive integers.
`ROLE` defaults to `other`. `full_image` is reserved and MUST be rejected as an
authored identity.

A region that leaves the image bounds MUST fail as `region_out_of_bounds`,
naming the region and its case. A mask geometry is not an input to this product
and MUST be rejected rather than re-rooted.

### Measurement families

The [measurement protocol](../benchmark/metrics/spec.md) owns each family's
domain, estimator, and availability rules. This document names the families
and what each measures; it does not define a second implementation of one. The
family names are the ones the measurement library already emits, so a retained
record stays readable.

| Family | Measures |
|---|---|
| `numeric` | native-channel statistics: maximum absolute difference, RMSE, out-of-range fractions |
| `color` | ΔE00 and ΔL\*, Δa\*, Δb\*, ΔC\* against a reference |
| `neutrality` | the gray-axis cast: mean a\* and b\*, the chroma radius of the population, and its lightness spread |
| `tone` | density or log-exposure response over a declared ramp: slope, shoulder, toe, Dmax |
| `color_noise` | luminance deviation, chroma RMS, the ratio between them, channel correlation, and the radial centroid of the residual |
| `edge` | spatial frequency response and MTF50 |
| `flip` | perceptual difference over the whole raster |
| `grain` | grain residual under a declared probe |
| `halation` | halation residual under a declared probe |

`neutrality` reports the cast itself, not a distance from it: the mean `a*` and
`b*` are the direction of the cast, and the chroma radius is the same quantity
expressed as a magnitude.

The lightness spread belongs to this family because a neutral field that is not
flat is not a neutral field, and one number for its lightness would hide that.

`color_noise` is the family this product is most often asked for: a film grain
that is not neutral is a color error, and the ratio between the chroma RMS and
the luminance deviation is the number that says so. Its residual is the sample
minus its own low pass, so the quantity is a grain amplitude and not a
gradient.

`color` requires a reference. Without one it is unavailable with
`missing_reference`, and the remaining families still measure.

A probe family is added only with the probe that makes it measurable. A family
that needs a declared probe MUST refuse without one, retain an unavailable row
naming the missing probe, and MUST NOT fall back to a default probe geometry.

### Decoding

Every raster the lab reads is decoded by one reader that the measurement
library owns. `analyze`, `compare`, `sample`, and any retained-source reading
MUST use that reader.

The reader answers one question: what samples does this file contain under the
interpretation the caller declared? It MUST NOT answer the renderer's question,
which is what the photographer intended the file to be. It therefore MUST NOT
apply an embedded ICC profile, an EXIF orientation, a RAW interpretation,
camera metadata, or any other evidence the caller did not declare.

A container type whose normalization the reader cannot state MUST be rejected
as `unsupported_image_layout`. A silent default is not an acceptable answer for
a type the lab has not defined.

| Container | Normalization |
|---|---|
| 8-bit and 16-bit integer | divide by the type's maximum |
| 32-bit float, 64-bit float, half | widen exactly to f64, no scaling |
| single channel | replicate to three equal channels |
| four channels | ignore alpha; never composite |
| `.exr` | half and float channels widen exactly to f64; no tone map, no exposure |

The reader MUST NOT read a sidecar file, a neighbouring metadata document, or
a filename convention that changes the interpretation. A declaration comes from
the caller or from the retained record that named the file, and from nowhere
else.

The lab MUST NOT clip a value before measurement, and it MUST NOT rescale one
container to match another.

The declared transfer is decoded exactly once, before measurement, so a family
measures the signal rather than its encoding. A family that names an encoded
domain explicitly is the exception and records the encoding it measured.

### Images

Each side MUST decode once and retain one display copy per case as an
artifact. The measurement population keeps the decoded floating samples; the
display copy exists only for inspection, and a reader of the record MUST NOT
be given it as a measurement input.

The two sides of one pair MUST have identical dimensions. A mismatch MUST fail
as `dimension_mismatch`, naming the case. The lab MUST NOT resize, crop, or
pad to reconcile a mismatch.

A nonfinite sample MUST fail as `nonfinite_samples`. The lab MUST NOT replace
it.

## Structured output

This product publishes records and no page. The record is therefore complete
on its own. A later reading of it needs no source the lab did not retain, and
is never left guessing what a number meant.

One invocation publishes one directory:

```text
OUT/
  analysis.json | comparison.json
  protocol.json
  regions.json
  cases/<case-id>/...
```

Every record MUST carry:

- `kind`, `schema_version`, and the command that produced it;
- the binary and the endpoint, so a reader can tell a lab record from a render
  record without inspecting the payload;
- each source's name, declared color space and transfer state, dimensions per
  case, and the sha256 of every file it read;
- the materialized protocol and its digest, so the numbers are reproducible
  from the record alone;
- the regions document and its role per region;
- for every measurement: its domain, unit, population, estimator, and sample
  count, beside the value;
- every unavailable value with its stable reason;
- the identity of the comparison as a digest over its inputs and its protocol.

The record MUST NOT carry a value that the measurement did not compute. A
quantity that does not exist for a source is unavailable with the reason it
does not exist. In particular, a delivered image is one realisation and has no
seed population: the record MUST carry no seed field, and MUST NOT publish a
cross-realisation statistic.

A record MUST be byte-identical for identical inputs and protocol on every
platform and every invocation. Identity is content-addressed; it MUST NOT
contain a timestamp, a path outside the recorded inputs, or an execution
duration.

A retained comparison MAY be reduced for inspection: difference maps MAY be
box-averaged by `--map-scale N` to one sample per `N × N` block. Measurement
populations MUST NOT be sampled, reduced, or aggregated by it, every scalar
MUST be computed from the full-resolution population, and the scale MUST be
recorded.

## Use cases

The product exists for these and for nothing else. A case that needs a concept
this document does not have is out of scope.

| # | Case | Command |
|---|---|---|
| U1 | Quantify one image set: noise statistics, neutrality, distribution | `analyze` |
| U2 | Quantify how two builds differ: per-patch color error, family deltas, regressions | `compare` |
| U4 | Confine a measurement to the regions a change touched | `compare --regions` |
| U5 | Publish evidence a third party can recompute to the same identity | any, from the record |
| U6 | Establish a noise floor by comparing a source with itself | `compare` |
| U7 | Verify a delivery approximation: a LUT against full precision | `compare` |

U6 needs no command of its own: the same invocation over two identical paths
reports every available delta as zero and every unavailable value with its
reason, which is the noise floor of the pipeline that produced the files.

## Acceptance

1. `cargo tree -p spektrafilm-lab` names none of `spektrafilm-core`,
   `spektrafilm-gpu`, `spektrafilm-backend`, `spektrafilm-raw`,
   `spektrafilm-model`, `wgpu`.
2. The lab builds and runs on a host with no GPU, no raw decoder, and no
   renderer installed.
3. Every record names its binary and its endpoint.
4. `spektrafilm-lab run suite ...` fails with `argument_syntax` and names
   `spektrafilm-f64 run suite`.
5. Two identical sources produce a record whose every available delta is zero
   and whose every unavailable row carries a reason.
6. A record that would need a seed carries none, and no value is written as
   zero to stand for a quantity that does not exist.
7. A pair whose dimensions differ fails with `dimension_mismatch` and writes
   no record.
8. A directory pair with a name on one side only fails with
   `incompatible_image_sets` and names both lists.
9. Two file names that reduce to one case identity fail with
   `duplicate_case_identity`.
10. A regions document that declares `source` or `render` geometry fails with
    `unsupported_coordinate_space` before reading a pixel.
11. A generated region outside the image fails with `region_out_of_bounds` and
    names the region.
12. The same arguments produce byte-identical `analysis.json`,
    `comparison.json`, `protocol.json`, and `regions.json`.
13. A family declared on a role it does not apply to fails with
    `incompatible_measurement_role`, naming the family and the role, and the
    region's other families still measure.
14. `color_error` without a reference retains an unavailable row with
    `missing_reference`, and `color_noise` and `distribution` still measure on
    the same region.
15. `--map-scale 4` changes no scalar measurement and records the scale.
16. An EXR and the float TIFF written from the same render decode to the same
    samples.
17. A single-channel PNG decodes to three equal channels, and a four-channel
    PNG ignores its alpha instead of compositing.
18. A `<path>.json` sidecar beside a measured image does not change the
    reading.
19. A record carries the sha256 of every file it read and the digest of the
    protocol it materialized, so a third party recomputes the same identity.
20. `spektrafilm-f64 compare runs ...` fails with `argument_syntax` and names
    the owning binary.

## Independence

The lab MUST build and run without a graphics device, a raw decoder, a
renderer executable, or the renderer's crates.

The lab's transitive dependency set MUST NOT contain `spektrafilm-core`,
`spektrafilm-gpu`, `spektrafilm-backend`, `spektrafilm-raw`,
`spektrafilm-model`, or `wgpu`.

The lab requires reference precision unconditionally. It MUST NOT depend on a
caller-enabled precision feature to select the precision it measures at.

The evaluation library currently builds a native perceptual-difference
evaluator through C++ and dynamic loading. That is a real build dependency of
the measurement library, not a lab choice. Removing it for builds that never
select `flip` is a follow-up of the library, and this document MUST NOT claim
the lab is buildable without a C++ toolchain before that change lands.

The renderer reads its input and writes its output through a native codec
bridge in `spektrafilm-core`. The lab MUST NOT use that bridge, for
measurement or for any other purpose.

The two codecs serve two contracts. The divergence is deliberate.

| | Renderer input and output | Lab raster reader |
|---|---|---|
| Question it answers | what did the photographer intend this file to be? | what samples does this file contain? |
| ICC profile, EXIF orientation, camera metadata | appropriate | forbidden |
| Container coverage | as wide as the codec library offers | exactly the containers whose decoding the lab can state |
| Owner | `spektrafilm-core` | the measurement library |

A shared codec would force one of two errors. Either the lab would inherit an
interpretation it is forbidden to apply, or the renderer would lose an
interpretation it needs. Sharing the reader is therefore not a refinement of
the boundary; it is the boundary being wrong.

A caller who needs the renderer's reading of a file does not need a shared
codec. The run record already retains it: `source_raw` is the floating master
that the renderer's own input route produced, and `sample` reads it.

## Current implementation gap

- The command surface is `analyze`, `compare`, `regions grid`, `schema`, and
  `validate`.
  `sample`, `inspect`, `report comparison`, and the run, reading, and pair
  comparisons belong to `spektrafilm-bench` and are refused here.
- `regions grid` is still spelled with its subcommand; this document names the
  endpoint `regions` and leaves the leaf form to the machine interface.
- `color_noise` and `neutrality` are implemented. The other families in the
  table above are unchanged.
- The comparison record no longer carries `seed`, `review`,
  `evaluation_status`, or `findings` for an image source, and a measurement of
  a delivered image carries no `seed_population`. The `comparison` schema
  still states those fields as optional because a run comparison evaluated by
  a policy writes them.
- The one coordinate space of a delivered raster is called `render`, a name
  that belongs to the renderer's vocabulary. Renaming it is a wire change and
  does not happen here; the name is carried with its meaning stated.
- A region is a rectangle. Mask geometry is rejected; reading mask files on a
  delivered raster is not implemented.
- The lab reader decodes PNG, TIFF, JPEG, WebP, and EXR. A container layout
  with no stated normalization is rejected.
- The evaluation library builds its native perceptual-difference evaluator
  with a C++ toolchain.
