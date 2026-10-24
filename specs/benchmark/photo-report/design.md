# Photo Report design

## Boundary

Photo Report is a benchmark-owned visual report builder. It consumes one photo manifest and publishes one offline page. It does not consume a run, comparison, policy, review manifest, or `spektrafilm-lab` document.

The existing comparison report remains the owner of retained metrics, findings, difference maps, and comparison evidence. The existing prepared-image review remains a separate product for sets, shared grids, and explicit per-image geometry. Photo Report is intentionally smaller: it owns a flat photo inventory, rectangular photo ROIs, and explicit comparison pairs.

```text
photo-manifest.json
  → strict validation and source snapshot
  → one display copy per photo
  → one generated report page
  → report.json + local assets
```

The pipe carries files. The report endpoint owns mode selection, pair validation, ROI resolution, display packaging, and page state.

## Why this is a separate input contract

The prepared-image review manifest supports chart-like sets, shared grid arithmetic, and measurement-region derivation. Those concepts are useful for controlled benchmark material but are unnecessary for an agent that has ordinary photographs and a few manually declared ROIs.

Photo Report MUST NOT add `sets`, `grid`, `measurement`, or `default_pair` to its input. It MUST NOT overload the prepared-image review manifest with single-photo mode or arbitrary multi-photo comparison requests. One small contract is easier for a user and an agent to author, validate, and repair.

The command MUST NOT be added to `spektrafilm-lab`. `spektrafilm-lab` remains the instrument for rendering, measurement, retained comparison, inspection, and comparison reporting. If the benchmark executable is consolidated later, the Photo Report subcommand MUST remain a benchmark product command with its own document kind and handler.

## Domain model

Use typed internal records rather than passing unvalidated JSON through page generation:

```text
PhotoManifest {
  schema_version: 1
  title: String
  description: String?
  photos: Vec<Photo>
  comparisons: Vec<ComparisonRequest>
}

Photo {
  id: PhotoId
  source: AuthoredSource
  label: String
  note: String?
  color: ColorDeclaration
  rois: Vec<PhotoRoi>
}

PhotoRoi {
  id: RoiId
  label: String
  rect: Rect
  note: String?
}

ComparisonRequest {
  id: ComparisonId
  left: PhotoId
  right: PhotoId
  label: String?
  note: String?
}

ResolvedComparison {
  request: ComparisonRequest
  rois: Vec<ResolvedRoi>
}

ResolvedRoi {
  id: RoiId
  left: RoiSide
  right: RoiSide
}

RoiSide = Available { rect, dimensions, display_artifact }
        | Unavailable { reason }
```

`PhotoId`, `RoiId`, and `ComparisonId` are stable semantic identities. DOM order, vector index, generated filename, and display label MUST NOT become identity.

The model MUST distinguish authored fields from observed fields:

- authored: paths, labels, notes, color declarations, ROI rectangles, and pair requests;
- observed: source hashes, decoded dimensions, display artifact hashes, and display conversion records;
- generated: report mode, derived two-photo pair, resolved ROI sides, and output artifacts.

## Validation and source integrity

Validation MUST complete before final publication. It MUST use strict JSON loading with duplicate-key and unknown-field rejection. It MUST resolve authored paths relative to the manifest, reject absolute and parent-traversing paths, and verify that each source is a supported regular raster file.

The validator MUST accept only the closed color-space vocabulary in the product specification and MUST apply the fixed default declaration when fields are omitted.

The generator MUST pass a source snapshot and an explicit declaration object to image decoding. It MUST NOT read a neighboring sidecar or infer source interpretation from a file extension.

The generator MUST snapshot each source before decoding, using an isolated temporary file or equivalent immutable read boundary. It MUST hash the snapshot and compare it with a hash taken from the authored path before publication. A source change during generation MUST fail with `hash_mismatch`.

A neighboring sidecar MUST NOT override the explicit manifest color declaration.

The validator MUST decode every photo once to observe dimensions and validate all ROI rectangles. An error MUST identify the photo ID and ROI ID when applicable. Validation MUST reject the output destination before reading or publishing any generated artifact if the destination already exists.

The generator MUST NOT mutate the manifest or source files. A failed generation MUST NOT leave a directory that looks like a completed report. If cleanup cannot remove an incomplete destination, the command result MUST identify it as incomplete and unusable.

## Mode and pair resolution

Mode resolution is deterministic:

```text
photos.len() == 1
  → single_photo

photos.len() == 2 and comparisons absent
  → two_photo, pair = (photos[0], photos[1])

photos.len() >= 3 and comparisons nonempty
  → requested_comparisons, pairs = comparisons in authored order
```

All other combinations fail validation. In particular:

- zero photos fails;
- one or two photos with `comparisons` fails;
- three or more photos without `comparisons` fails;
- an empty `comparisons` array fails;
- self-pairs, unknown IDs, and duplicate pair IDs fail.

For a comparison, build the ROI union by first listing the left photo's ROI order, then the right photo's ROI IDs not already listed. This gives deterministic display order while preserving author order. The resolver does not inspect coordinates or labels.

## Display artifact strategy

Create exactly one lossless display image per source photo. Use the existing `image_data::display_rgb` conversion boundary and `write_png16` encoder. Store each generated image under an output-local path derived from a validated stable identity, not from the source filename.

A collision-resistant generated filename MAY include a deterministic ordinal, but `report.json` MUST retain the photo ID as the semantic identity.

The browser MUST use the generated display image for full-image and ROI views. It MAY apply CSS sizing and viewport cropping. It MUST NOT create canvas-derived replacements, perform a second color conversion, or request the source file.

Retain these records for every photo:

```json
{
  "id": "scan",
  "source": {
    "path": "scan.jpg",
    "sha256": "..."
  },
  "dimensions": [4032, 3024],
  "color": {
    "color_space": "sRGB",
    "cctf_decoding": true
  },
  "display": {
    "path": "images/0000.png",
    "sha256": "...",
    "carrier": "png",
    "bit_depth": 16,
    "color_space": "sRGB",
    "transfer": "srgb",
    "resampling": "none",
    "quantization": "16-bit rounded; no dither"
  }
}
```

`report.json` is the machine-readable terminal record. HTML MUST embed a sanitized view payload so direct file opening does not depend on loading JSON or JavaScript from another origin. `viewer.js` remains a local asset.

The page MUST make no network request.

## Page state and interaction

The page has one viewer module with three modes:

```text
single_photo
  full photo → selected ROI

two_photo
  one pair → full-image pair → selected ROI pair

requested_comparisons
  pair selector → selected pair → selected ROI pair
```

The state is small and serializable:

```text
PhotoReportViewState {
  photo_id?
  comparison_id?
  roi_id?
  layout: side | blink
  zoom: fit | native | x2 | x4
}
```

`split` is not a first-release layout because independently authored ROI rectangles do not establish pixel registration. A later contract MAY enable an explicit aligned display mode, but it MUST add a distinct declaration and MUST NOT infer it from equal dimensions.

The URL fragment MAY contain only identities and view settings accepted by the generated payload. A malformed or unknown value MUST show an error and a recovery link. It MUST NOT fall back silently to another pair or ROI.

The page MUST preserve the selected pair while changing layout or zoom. When the pair changes, it MUST replace all pair-scoped labels, ROI entries, notes, coordinates, and unavailable states atomically.

## Artifact and security boundary

The report page is a static presentation. The generator MUST escape all authored text in HTML and JavaScript data. It MUST encode embedded JSON so `<`, `</script`, and equivalent payloads cannot terminate the data element.

It MUST use relative artifact paths generated by the builder and reject paths that escape the report directory.

The output CSP SHOULD follow the existing offline report policy:

```text
default-src 'none';
img-src 'self';
style-src 'unsafe-inline';
script-src 'self';
connect-src 'none';
base-uri 'none';
form-action 'none'
```

The page MUST NOT execute notes, labels, source paths, or metadata. It MUST NOT load adjacent source sidecars. Redistribution restrictions do not apply to an authored local photo report unless a future input contract adds an explicit permission field; the generated report still records source identity and MUST fail if the source cannot be read.

## Result and publication boundary

The public result MUST identify the generated HTML and terminal manifest:

```json
{
  "schema_version": 1,
  "command": "report photos",
  "source": "photo_manifest",
  "mode": "requested_comparisons",
  "execution_status": "completed",
  "artifact": "index.html",
  "manifest": "report.json"
}
```

Publication order MUST be:

1. validate the manifest and source dependencies;
2. create a private temporary output;
3. write display assets;
4. write `report.json` and embedded HTML;
5. hash and validate every generated artifact;
6. publish the completed directory without replacing an existing destination.

The command MUST NOT claim success before `index.html`, `report.json`, every display image, and the artifact hashes are present and valid.

## Rejected alternatives

### Reuse the existing review manifest

Rejected for this product. It requires set membership, grid or explicit-region forms, default-pair semantics, and at least two images. It does not naturally express one-photo reports or an explicit list of arbitrary pairs.

Reusing it would make the smallest Agent input carry concepts that the page does not need.

### Generate every pair automatically

Rejected. Pair count grows quadratically and creates evidence the user did not request. Explicit pair requests preserve user intent and keep a multi-photo report navigable.

### Create one HTML file per pair

Rejected. It duplicates assets, complicates sharing, and makes a multi-photo report harder to browse. One page owns one manifest and switches only among declared pairs.

### Use HTML as the input

Rejected. JSON gives the validator, Agent, and generator one strict machine-readable contract. HTML remains a generated artifact.

### Add metrics or automatic alignment

Rejected. These would turn a visual report into a second evaluation pipeline and introduce unsupported judgments.

## Implementation order

1. Add the photo manifest schema and strict validator owned by the benchmark.
2. Add typed manifest loading and mode/pair/ROI resolution.
3. Add source snapshot, display conversion, and artifact publication.
4. Add the single-photo page.
5. Add the two-photo viewer and ROI states.
6. Add explicit multi-photo pair selection.
7. Add offline fragment navigation, accessibility, and narrow-layout behavior.
8. Add contract, security, publication-integrity, and real-browser smoke coverage.

Each step MUST preserve the existing comparison report and prepared-image review contracts. No step MAY make a lab endpoint consume the new manifest.
