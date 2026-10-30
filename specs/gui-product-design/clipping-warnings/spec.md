# Black and white endpoint warnings

## Purpose and authority

Black and white endpoint warnings help locate photograph samples that reach the native RGB black or white boundary. They MUST remain viewer-only inspection tools.

[GUI workflow](../../gui-workflow/spec.md#viewer-presentation) owns accepted photograph identity and presentation. [GUI presentation](../../gui-presentation/spec.md#viewer-footer) owns footer layout. [The warning design](design.md) owns sample interpretation and execution boundaries.

The warnings MUST NOT claim that a photograph has an exposure error, unrecoverable capture detail, or confirmed export clipping. An intentional black background or white highlight MAY satisfy the warning condition.

The warnings MUST NOT change photographic adjustments, retained samples, pixel inspection values, adjustment history, Save pixels, or Export pixels.

## Subject and classification

The viewer's explicit Original or Modified selection MUST select the warning subject. Original MUST use the accepted oriented input under its accepted color interpretation. Modified MUST use the latest accepted Preview or Scan under that result's interpretation and geometry.

Classification MUST use native linear RGB before viewer display conversion, display ICC, interpolation, canvas composition, or quantization. It MUST NOT convert native primaries to a fixed analysis or monitor space. Encoded samples MUST use the equivalent native transfer-function endpoints.

- A finite pixel MUST receive a black warning when all three native linear components are less than or equal to zero.
- A finite pixel MUST receive a white warning when any native linear component is greater than or equal to one.
- A nonfinite source triple MUST NOT receive either warning.

These conditions include exact endpoints. A zero or negative channel alone MUST NOT establish a black warning. A white warning MUST NOT imply that all channels are white.

The existing [image-analysis range observations](../image-analysis/spec.md#measurement-meaning-and-limitations) MUST retain their independent strict below/above rules and common sRGB interpretation. Warning switches MUST NOT change analysis populations, regions, bins, counts, or scheduling.

## Coverage and presentation

Warnings MUST cover only the actual photograph. White padding, canvas, watermark, Paper back, text, and linked inspection decorations MUST NOT become warning samples.

Classification MUST precede display filtering. Downscaled warning coverage MUST preserve a qualifying native pixel instead of classifying its averaged display color.

Magnified coverage MUST use the native cell containing each viewport-cell center. At exact source-pixel zoom, warning coverage MUST align with the corresponding native pixel cells.

Pan, zoom, DPI, orientation, and accepted crop geometry MUST preserve this correspondence.

Black warnings MUST use a blue overlay. White warnings MUST use a red overlay.

Nonqualifying photograph pixels MUST retain their normal appearance. Both overlays MUST be independently selectable.

If conservative black and white coverage share a viewport cell, an enabled white warning MUST take precedence over the black overlay.

Temporary black inspection MUST show a white photograph area with qualifying pixels black. Temporary white inspection MUST show a black photograph area with qualifying pixels white. These inspection backgrounds MUST remain inside the photograph area.

Paper back and an empty or unavailable subject MUST show no warning. Their controls MUST retain the requested choices while disabled.

Temporary reveal, development animation, crossfade, and held comparison composites MUST suspend warning presentation without changing those requested choices. Warnings MUST return for the matching accepted subject when the temporary presentation ends.

A pending warning computation MUST NOT put a mask from a different subject, interpretation, or geometry on the accepted photograph. Failure MUST retain the last accepted viewer frame and use the existing display-error surface.

## Controls

The viewer footer MUST contain one compact pair of independent icon switches immediately left of Image analysis. The black switch MUST precede the white switch. The pair MUST remain in the existing footer row and MUST NOT require the analysis workspace to be expanded.

The switches MUST NOT be duplicated inside the histogram, photographic inspector, or application settings. They MUST NOT add a permanent information row or reduce the photograph area.

Primary activation and focused Enter MUST toggle the corresponding overlay. A secondary-pointer hold on a switch and a focused Space hold MUST select its temporary isolated view. Releasing the hold MUST restore the previous overlay choices without toggling them.

Temporary isolation MUST end on release, control focus loss, window focus loss, modal entry, or loss of the photographic subject. Pointer cancellation MUST NOT leave a latched inspection mode.

Each switch MUST expose a localized accessible name, requested selected state, visible keyboard focus, and a selected cue that does not rely on color. Tooltip help MUST identify activation, temporary inspection, and the endpoint limitation. Control search MUST locate the real switches through their localized names and canonical paths `viewer.clipping.shadows` and `viewer.clipping.highlights`.

Viewer warning input MUST NOT intercept editing in search, text, numeric fields, or an active modal.

## Preferences

Both warning choices MUST default to off without saved application preferences. Requested black and white choices MUST persist independently of photographic records, look presets, startup photographic controls, and adjustment history.

Loading a photograph, state, or preset and using adjustment Undo or Redo MUST NOT replace the requested warning choices.

Temporary inspection, computed masks, pending requests, and source identities MUST remain transient. They MUST NOT be serialized as photographic or application preferences.

Preference load or save failures MUST be reported through the existing application message surface. A failed write MUST preserve the previous complete preference file.

## Excluded behavior

This capability MUST NOT introduce automatic exposure correction, raw sensor saturation provenance, export soft proofing, adjustable warning thresholds, another analysis color model, or a photographic editing mask.

## Acceptance

Acceptance MUST exercise the real native viewer and production CPU and GPU paths.

| Scenario | Required result |
| --- | --- |
| Open without saved warning preferences | Both switches are off; normal viewer pixels and allocation remain unchanged. |
| Activate black, white, and both switches with analysis collapsed | Applicable overlays appear independently without analysis work, new rows, or photographic edits. |
| Inspect exact zero and one, one negative channel, one high channel, and invalid triples | Inclusive native endpoint rules hold; partial low channels and invalid values do not become black warnings. |
| Inspect wide-gamut and transfer-encoded samples, including ACEScct | Native interpretation determines warnings; fixed-sRGB conversion and encoded code zero/one do not substitute for the accepted interpretation. |
| Fit a photograph containing one qualifying pixel, then inspect it at 100% with pan and DPI changes | Fit retains its indication; source-pixel coverage aligns at exact zoom without padding or canvas warnings. |
| Hold and release temporary inspection using pointer and keyboard; lose focus or open a modal | Isolation is temporary and restores the requested flags without an accidental toggle. |
| Switch subjects, accept a replacement frame, and supersede pending work | Only matching masks publish; obsolete work cannot replace the accepted frame. |
| Inspect Paper back and temporary photographic composites | No unrelated mask appears; requested flags return on the matching photograph. |
| Restart, load photographic state or a preset, and use Undo/Redo | Only independent warning preferences restore; temporary modes do not persist or enter adjustment history. |
| Repeat at 1100 by 760 and 1460 by 980 in English and Simplified Chinese | Footer actions remain nonoverlapping, focusable, and named without another row. |
| Save and independently Export with warnings off, on, and in an isolated view | Decoded output dimensions and samples remain identical for unchanged render and delivery settings. |
