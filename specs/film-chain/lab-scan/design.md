# Lab scan design

The [readout contract](spec.md) owns behavior and numerical definitions.

## Point-wise readout

The readout is point-wise. Each record reconstructs its exposure from its own density, and every parameter depends only on the material, the readout setting, and the operator values. No stage measures the complete frame.

The product retired an automatic frame setup. A frame measurement made the readout depend on picture content, could not be baked into a point-wise LUT, and had no paired minilab evidence. The [readout contract](spec.md#retired-frame-setup) keeps that boundary; manual exposure, cast, and contrast judgment belongs to operator correction.

## Exposure and tone

The tone family and its constants remain unchanged. Operator values are the only user control over the readout coordinate.

Every style retains the historical camera compensation on simulated film-scan routes. Real captures receive no virtual camera compensation.

## Operator coordinates and source primaries

The [operator equations](spec.md#operator-correction) separate common exposure from zero-sum channel differences. Independent per-channel contrast would amplify channel imbalance and move a corrected gray off neutral. Common-exposure contrast avoids that failure without adding a second tone family.

The neutral operator path bypasses coordinate arithmetic to preserve the sRGB baseline exactly. Host-side controls and prepared exposure data retain f64 values; a selected lower rendering precision does not change the saved parameter representation.

The tone defines a synthetic fixed linear-sRGB response. Treating its unchanged numbers as each selected output space changes their colorimetric meaning. A single source-to-output transform before the shared output tail corrects that boundary for CPU and resident execution.

An exact sRGB identity avoids a rounding-only default regression. Saving-space conversion and display ICC processing MUST NOT substitute for this transform.

## Prepared baseline and previews

The readout is frame-independent, so a preview MAY reduce the working grid to the preview size and reduce only terminal RGB. Diagnostic taps keep that working grid.

The core frame cache shares preparation between preview and scan requests when the upstream route exposes it. Optical Dye/Fotufilm grain MAY use exact full-route recomputation to retain optical context. Its identity combines the live input image identity, source material and backend identity, and all effective parameters except operator and preview settings.

The cache MUST NOT retain an input through a strong identity reference. Crop, upscale, material, random seed, upstream controls, backend, and precision distinguish incompatible preparation. Pan, zoom, and operator edits do not.

Replacing the input invalidates reuse even when dimensions match. Publish preparation only after successful completion; request epochs and revisions still guard output acceptance.

This cache is exact reuse, not a photometry lock. No GUI readout implementation, separate approximate engine, or hidden f32 exposure cache is permitted.

Full-grid buffers can be large; allocation failure MUST NOT silently change the grid or arithmetic policy.

## Saved and effective correction

Core runtime parameters own correction values and route/style validation. Presets and recipes carry explicit intent and MUST reject unsupported nondefault combinations. GUI state separately retains inactive editing intent; rendering receives neutral correction on an inapplicable route or style.

Both carriers use the same typed operator values and reject unknown keys. No compatibility shim maps retired fields onto the new controls. Existing schema versions admit this additive neutral-defaulted group; the model version records the changed non-sRGB interpretation.

## Point-wise consumers

The readout is point-wise, so LUT baking includes it exactly once under [LUT delivery](../../lut-delivery/spec.md). A LUT lattice is not a photograph. The readout MUST NOT depend on the lattice size or sampling distribution.

A fixed user-authored style remains the explicit point-wise alternative to the built-in tone.

## Names and provenance

Runtime scanner parameters own canonical style spelling. Deserialization maps `neutral` to `standard`; serialization emits the canonical spelling even for a programmatically assigned alias. The GUI catalog uses the same core catalog owner and hides reserved names.

Reports carry the canonical style, the model revision, and the material-category readout selection.

The model revision records pixel-changing readout and source-primary definitions. GUI state migration removes retired lamp keys only for explicitly versioned legacy documents. Current-schema unknown-field validation stays strict.

Complete legacy documents preserve acquisition light under the Convert owner. Scanner-only presets discard the removed readout leaf without adding another section. Recipe, preset, and GUI adapters share the core cutover and provenance validators.

## Evidence limits

The exposure pivot uses the normalized 0.184 camera reference and Kodak's 16% LAD scene target. These choices do not calibrate the renderer to a real minilab.

Synthetic tests establish record independence, tone ordering, operator determinism, and backend integration. They do not establish perceptual quality or algorithmic optimality. The earlier research bake-off contained broadcasting and tone-parameter errors; it is not acceptance evidence for this implementation.

## Sources

- Kodak, *LAD for KODAK Color Negative Film*, H-61A: https://www.kodak.com/content/products-brochures/Film/LAD-for-KODAK-Color-Negative-Film-H-61a.pdf
