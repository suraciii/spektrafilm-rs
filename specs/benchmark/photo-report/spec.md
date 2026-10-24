# Photo Report

## Authority

This document owns the product behavior for the benchmark's simple offline photo review page. It is separate from the measured comparison report defined by [the instrument machine interface](../../instrument/spec.md) and from the `spektrafilm-lab` instrument interface.

The implementation target is a benchmark-owned command with this form:

```text
spektrafilm-bench report photos --manifest FILE --out DIRECTORY
```

The command name is a product boundary. It MUST NOT be registered as a `spektrafilm-lab` endpoint. The benchmark MAY provide the executable entry point in the implementation work that follows.

## Purpose

Photo Report turns one authored photo manifest into one offline report page. It helps a user or an agent inspect photos and explicitly declared regions of interest (ROIs). It does not measure, score, rank, or select a preferred photo.

The report MUST work when the original photo files and the renderer are unavailable after generation. It MUST retain source identity and the declarations needed to understand each displayed copy.

## Terms

A **photo** is one authored raster image and its display metadata.

A **photo ROI** is a named, rectangular visual crop in the photo's native pixel coordinates. A photo ROI is not a measurement region. It does not establish pixel registration, scene correspondence, or a spectral stimulus.

A **comparison pair** is an explicit left and right photo identity. It controls which two photos the report compares. It is a viewing request, not a baseline, candidate, reference, or quality decision.

A **shared ROI** is an ROI with the same identity declared on both photos in one comparison pair. The shared identity is the only correspondence authority for a photo ROI.

A **photo report** is the generated offline page and its local artifacts. It is a visual inspection artifact, not a benchmark comparison record.

## Input document

The photo manifest MUST be UTF-8 JSON with `schema_version: 1` and this shape:

```json
{
  "schema_version": 1,
  "title": "800T outdoor comparison",
  "description": "Original scan and simulation output.",
  "photos": [
    {
      "id": "scan",
      "path": "scan.jpg",
      "label": "Original scan",
      "note": "No additional sharpening.",
      "color_space": "sRGB",
      "cctf_decoding": true,
      "rois": [
        {
          "id": "face",
          "label": "Face",
          "rect": [100, 120, 300, 400],
          "note": "Face and forehead."
        }
      ]
    },
    {
      "id": "simulation",
      "path": "simulation.png",
      "label": "Simulation output",
      "note": "800T, seed 0.",
      "color_space": "sRGB",
      "cctf_decoding": true,
      "rois": [
        {
          "id": "face",
          "label": "Face",
          "rect": [90, 110, 280, 380]
        }
      ]
    }
  ]
}
```

The manifest MUST contain:

- `schema_version`, equal to `1`;
- a nonempty `title`;
- `photos`, containing at least one photo;
- for every photo, a unique `id`, a relative `path`, and a nonempty `label`.

`description` and `note` are optional plain-text fields. They MUST NOT be interpreted as HTML, CSS, JavaScript, Markdown, or a path.

A photo `path` MUST be relative to the manifest file. It MUST NOT be absolute and MUST NOT escape the manifest directory through `..` components. It MUST identify a regular supported raster file.

The manifest MUST NOT use a file extension to infer color interpretation. `color_space` and `cctf_decoding` are optional explicit declarations. If omitted, they select the fixed display default `sRGB` with `cctf_decoding: true`; the generated report MUST show that default.

A non-sRGB or linear source MUST declare its interpretation explicitly.

The supported explicit `color_space` values are `sRGB`, `Display P3`, `DCI-P3`, `Adobe RGB (1998)`, `ProPhoto RGB`, `ITU-R BT.2020`, `ACEScg`, and `ACES2065-1`. `cctf_decoding` MUST be a boolean. The generator MUST pass only the manifest declaration to image decoding and MUST NOT consume a neighboring sidecar.

A photo ROI is optional. Each ROI MUST contain a unique `id`, a nonempty `label`, and `rect` with four nonnegative integers in `[x, y, width, height]` form. Width and height MUST be positive.

An ROI MUST lie fully within the decoded photo. ROI IDs are scoped to their photo.

The manifest MUST reject unknown authored fields, duplicate JSON keys, duplicate photo IDs, duplicate ROI IDs, unsafe paths, unsupported color declarations, and invalid ROI bounds.


## Selecting comparison pairs

The number of photos selects the report mode:

| Input | Required behavior |
|---|---|
| one photo | Generate a single-photo report. Do not compare it with anything. |
| two photos | Generate one comparison pair in manifest order. Do not require a `comparisons` field. |
| three or more photos | Require a nonempty `comparisons` array and generate only the declared pairs. |

For three or more photos, each comparison entry MUST contain two different photo IDs:

```json
{
  "comparisons": [
    {"left": "scan", "right": "simulation"},
    {"left": "simulation", "right": "final_scan"}
  ]
}
```

A comparison entry MAY contain an optional unique `id`, `label`, and plain-text `note`. If `id` is omitted, the generator MUST derive a stable ID from the ordered pair without changing the pair's order. The generator MUST reject duplicate comparison identities, unknown photo IDs, and self-comparisons.

The generator MUST NOT create undeclared pairs. It MUST NOT generate every combination for three or more photos. A report with ten photos MUST contain only the pairs in `comparisons`, not forty-five inferred pairs.

A manifest with one or two photos MUST NOT provide `comparisons`. This keeps the small input form unambiguous. A later contract revision MAY add explicit pair labels, but it MUST NOT change the pair-selection rule without changing the schema version.

The order of `photos` and `comparisons` is meaningful and MUST be preserved in the report. The first pair is the initial pair for a multi-photo report.

## ROI behavior

A single-photo report MUST show the full photo. If the photo declares ROIs, it MUST also show a selectable ROI list and the selected ROI crop. A ROI is a detail view, not a comparison.

A two-photo or multi-pair report MUST show the full-image comparison for the selected pair. For that pair, the report MUST resolve the union of the two photos' ROI IDs in deterministic input order:

- an ID present on both photos is a shared ROI and MUST have a two-sided ROI view;
- an ID present on only one photo is a one-sided ROI view with an explicit `unavailable` state for the other side;
- an ID present on neither photo cannot appear.

The report MUST NOT match ROIs by label, rectangle, size, position, filename, or visual similarity. Same-named ROI IDs are an explicit author declaration of correspondence for that pair only.

A shared ROI MAY use different rectangles and dimensions on the two photos. The report MUST show each rectangle, its native dimensions, and its photo identity. It MUST NOT resize, crop, pad, translate, warp, register, exposure-match, or color-match either photo to make the rectangles coincide.

If the two rectangles cannot support a justified overlay, the report MUST use side-by-side or A/B alternation. It MUST NOT show a split or aligned overlay by default. The first implementation MUST NOT provide automatic registration.

The report MUST keep the full-image view available when a ROI is selected. Selecting one ROI MUST NOT hide other photos, pairs, or ROIs.

## Report page behavior

The generated page MUST adapt its controls to the selected mode.

### Single-photo mode

The page MUST show:

- the title and overall description;
- the one photo's label and note;
- the full photo;
- fit, native-pixel, zoom, and fullscreen controls;
- the photo's ROI list when one exists;
- the selected ROI crop and its full-photo location;
- source path, SHA-256, decoded dimensions, color declaration, and display declaration.

The page MUST NOT show left/right selectors, swap controls, comparison controls, or A/B language in this mode.

### Two-photo mode

The page MUST show:

- the title and overall description;
- the two photo labels and notes;
- the full-image pair in manifest order;
- side-by-side and A/B alternation controls;
- fit, native-pixel, zoom, and fullscreen controls;
- the shared and one-sided ROI list for the pair;
- the selected ROI view when a ROI exists;
- source and display provenance for both photos.

The page MUST keep the two-photo pair fixed. It MAY provide a swap control, but it MUST NOT provide selectors that can create another pair. The labels MUST NOT call either side `baseline`, `candidate`, or `reference` unless those words are part of the author's plain-text label.

### Multi-photo mode

The page MUST show:

- a selector for the declared comparison pairs;
- the selected pair's two photos and notes;
- the full-image viewer for that pair;
- the selected pair's shared and one-sided ROIs;
- source and display provenance for all photos reachable from the manifest.

Changing the pair MUST replace the complete pair view, including labels, ROI choices, coordinates, notes, and unavailable states. The page MUST NOT retain stale ROI data from the previous pair.

The page MUST NOT create a new pair in the browser. It MUST NOT select an undeclared pair by URL, filename, array position, or UI fallback.

## Visual evidence boundary

Photo Report is visual-only. It MUST NOT calculate or display:

- `delta_e00`, RMSE, FLIP, MTF, grain, halation, or any other metric;
- a quality score, acceptance status, ranking, or recommendation;
- a baseline, candidate, or reference judgment;
- automatic exposure or color matching;
- image registration or inferred coordinate mapping;
- face detection, chart detection, tracking, or automatic ROI discovery.

The report MAY display authored notes and source declarations. It MUST label display copies separately from source photos. A displayed image is evidence for visual inspection, not a measured master.

## Output

Successful generation MUST create a new output directory containing at least:

```text
REPORT/
├── index.html
├── report.json
├── viewer.js
└── images/
    ├── <photo-1>.png
    ├── <photo-2>.png
    └── ...
```

The user opens `REPORT/index.html`. The complete output directory is the shareable artifact. The page MUST work when opened directly from the local filesystem and when served as static files.

It MUST NOT make network requests or require the original source directory.

The generator MUST create one display image per input photo, not one image per comparison pair. ROI views MUST use the retained display image and declared rectangle metadata; the generator MUST NOT duplicate a photo for each pair.

Display copies MUST use the existing display conversion boundary and a lossless 16-bit RGB sRGB PNG encoding. The report MUST retain source path, source SHA-256, decoded dimensions, explicit color declaration, and display conversion metadata. Display resizing for fit-to-container is a browser view operation and MUST NOT change the source or ROI coordinates.

`report.json` MUST be a terminal machine-readable manifest. It MUST identify:

- `schema_version`;
- `command` and `source`;
- report `mode`: `single_photo`, `two_photo`, or `requested_comparisons`;
- title and description;
- photos in authored order;
- generated comparison pairs in authored or derived order;
- resolved ROI availability for each generated pair;
- output artifacts and their SHA-256 values;
- display and source provenance.

An unavailable one-sided ROI MUST remain an object with a reason. It MUST NOT be omitted or serialized as an empty rectangle.

## Offline navigation and accessibility

The page MUST keep its current photo, pair, and ROI in a URL fragment using stable authored identities. It MUST restore that state after reload without a network request. An unknown or malformed location MUST show an accessible error and a recovery action; it MUST NOT silently display an unrelated photo or pair.

The page MUST expose all photo, pair, and ROI selectors to keyboard users. All image controls MUST have accessible labels. Narrow layouts MUST NOT require horizontal page scrolling to access a photo or ROI.

## Machine output and status

The command MUST emit one JSON result on stdout after successful parsing. Progress, diagnostics, and human-readable errors MUST use stderr. It MUST return status `2` for argument syntax errors, status `1` for validation, decoding, conversion, publication, or source-integrity failures, and status `0` for successful publication.

A successful result MUST identify `schema_version`, `command`, `source`, `mode`, `execution_status`, `artifact`, and `manifest`. A failure MUST identify a stable error code and the failed input or publication boundary. An existing output directory MUST be rejected without modifying it.

## Product acceptance scenarios

1. A manifest with one photo and no ROI opens a single-photo page with no comparison controls.
2. A manifest with one photo and two ROIs opens the full photo and each declared ROI detail.
3. A manifest with two photos and no ROI opens one full-image comparison in manifest order.
4. A manifest with two photos and a shared ROI opens both the full-image comparison and the corresponding two-sided ROI comparison.
5. A manifest with two photos and a one-sided ROI shows the available crop and an explicit unavailable counterpart.
6. A manifest with three photos and two declared pairs shows a pair selector with exactly those two pairs.
7. A manifest with three photos and no `comparisons` fails before output publication.
8. A three-photo manifest never creates an undeclared pair, including through a URL fragment.
9. A shared ROI with different coordinates remains a side-by-side or A/B view and retains both native rectangles.
10. An ROI outside decoded bounds fails before report publication.
11. The report opens after the original photo directory is removed, using only bundled display artifacts and `report.json`.
12. A source hash change during generation fails and does not publish a completed report.
13. The report contains no network dependency, metric result, quality judgment, or automatic registration.
14. A hostile title, label, note, or path is displayed as text and cannot execute HTML or JavaScript.
15. The report remains usable from a direct `index.html` opening and from static hosting on a narrow viewport.
