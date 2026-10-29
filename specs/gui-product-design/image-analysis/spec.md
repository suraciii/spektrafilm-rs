# Image analysis

## Authority and purpose

This specification owns the read-only image-analysis workspace within [GUI product design](../spec.md). [Original and Modified](../../../CONTEXT.md#language) retain their shared meanings.

[GUI workflow](../../gui-workflow/spec.md) owns accepted images, photographic edits, and delivery. [The analysis design](design.md) owns measurement, geometry, and execution boundaries.

The panel MUST support these photographic judgments without declaring an image correct or incorrect:

| View | User question | Information the view MUST expose |
| --- | --- | --- |
| Histogram | Where are the image's tones concentrated? | Shadow, midtone, and highlight distribution; endpoint accumulation; optional channel differences. |
| Waveform | Which part of the picture contains those tones? | Horizontal picture position against tone, with density showing how frequently values occur at that position. |
| Waveform / RGB parade | Where and at which tone levels do channels differ? | Three spatial distributions with matching coordinates and scales, so the same subject can be compared across channels. |
| Advanced / Vectorscope | How did the look change the image's colors? | Hue direction, chroma, and population density, with fixed references for comparing the same subject before and after simulation. |

Image analysis is an information layer, not an editing surface. It MUST remain secondary to the photograph and parameter work.

Image analysis MUST NOT modify photographic adjustments, accepted image samples, or Save and Export behavior. Its visibility MUST NOT imply that photographic work has changed or requires correction. It MUST NOT present RGB or colorimetric measurements as wavelength spectra or material record densities.

## Analysis subject and comparison

The viewer's explicit layer selection MUST choose the analysis subject. The panel MUST NOT introduce another Original/Modified selector.

| Viewer presentation | Analysis subject |
| --- | --- |
| Original | The accepted photograph under its input interpretation and orientation, before film simulation. |
| Modified | The latest accepted Preview or Scan, with that result's interpretation and geometry. |
| Paper back | No photographic analysis; an explicit explanation replaces the plot. |
| Reveal, crossfade, development animation, or held automatic comparison | The explicitly selected Original or Modified subject, not the temporary composite or comparison image. |

Pan and zoom MUST NOT change the analysis population. Display ICC and other viewer decorations MUST NOT change measurements.

Switching Original/Modified MUST preserve the selected analysis mode and display settings. It MUST preserve corresponding subject regions under [Area inspection](#area-inspection).

Plot coordinates and density interpretation MUST remain comparable when switching subjects. The plots MUST NOT independently stretch each channel or subject to fill the available area.

Full-image analysis MUST identify different framing when Original and Modified cover different source areas. It MUST NOT imply that changes in population then result only from color or tone processing.

Analysis MUST distinguish Original from Preview and Scan. Independent Export MUST NOT replace the analysis subject or establish that the displayed measurements describe the exported file.

## Area inspection

Full image MUST be the initial population for each accepted photograph. The person MUST be able to inspect one rectangular Selection instead.

The panel MUST expose Select area and Full image actions beside the population indicator. A selected area MUST remain identifiable on the photograph while the panel is expanded.

Select area MUST explicitly arm a temporary viewer tool. A primary drag inside the image MUST define the rectangle without changing crop, zoom, or photographic adjustments.

The tool MUST provide keyboard-operable positioning and resizing. Completing a selection MUST restore normal viewer gestures.

Escape MUST cancel an unfinished selection and preserve the previous population. Collapsing the panel or changing the analysis subject during selection MUST cancel the unfinished gesture.

Full image MUST clear the selection and its overlay. Selecting an empty area or viewer padding MUST NOT produce a committed selection.

A selection MUST identify a source area, not a rectangle in screen pixels. Rotation, crop, and scaling MUST use the selected subject's accepted geometry to locate that area.

Switching subjects MUST preserve the selection only when its complete area exists in the destination subject. Otherwise the panel MUST explain that the selection is outside that subject's frame and offer Full image or Select area.

The panel MUST NOT silently intersect a selection with a crop, stretch it to new bounds, or substitute unrelated pixels. A new photograph MUST clear the previous photograph's selection.

An accepted geometry change during an unfinished selection MUST cancel that gesture. A committed selection MUST keep its source-area identity across later accepted results.

## Linked pixel inspection

The expanded panel MUST link viewer pixel inspection to the selected plot without requiring Select area, a qualifier tool, or a click. Hover MUST locate a sample within the existing distribution rather than replace that distribution.

The sample MUST come from the explicitly selected photographic subject under the pointer. Its readout MUST identify the subject pixel coordinates and the analysis values represented by the selected plot.

| Selected plot | Required sample marker |
| --- | --- |
| Histogram | Mark the Tone value interval or each visible RGB channel's value interval; do not treat the marker as a frequency measurement. |
| Waveform | Mark the sample's horizontal position and Tone or visible RGB values; a column guide alone MUST NOT substitute for the sample values. |
| Waveform / RGB parade | Mark the same sample's position and value in each channel plot, with explicit channel identity. |
| Advanced / Vectorscope | Mark the sample's position in the declared color plane, independently of population-density intensity. |

Markers MUST remain distinguishable from distribution traces and reference guides. They MUST NOT imply that the indicated bin contains only the inspected pixel.

Linked inspection MUST NOT change the population, selection, plot scale, or photographic adjustments. It MUST NOT consume clicks, drags, scrolling, or keyboard commands owned by other viewer operations.

A focusable keyboard pixel-inspection cursor MUST provide the same sample values and plot markers. Moving that cursor MUST NOT commit a selection or move the system pointer.

| Inspection condition | Required response |
| --- | --- |
| Pointer or keyboard cursor is inside the analyzed population | Show the matching sample readout and markers without changing statistics. |
| Sample is outside the committed selection | Keep its numeric readout labeled Outside selection; suppress plot markers and preserve the selected distribution. |
| Selection is unavailable in the subject | Keep the area-unavailable explanation; do not mark a pixel as part of that population. |
| Sample exceeds the plot's value or chroma window | Show its numeric value and an off-scale indication instead of clamping it to an apparently in-range marker. |
| Sample has invalid analysis values | Show an invalid-sample explanation without placing a valid-looking marker. |
| Sample or matching plot is pending | Clear the previous sample marker; do not overlay a sample on a plot from another subject, population, or generation. |
| Pointer leaves the photograph or enters padding or another control | Clear pointer-driven markers and readouts. |
| Keyboard inspection loses focus | Clear keyboard-driven markers and readouts. |
| Analysis subject, generation, or pixel changes | Clear the previous sample before accepting another sample result. |
| Panel collapses or Paper back is selected | Clear linked inspection. |
| A selection gesture, viewer navigation gesture, or temporary comparison/composite is active | Suspend linked markers and explain the suspension without changing the analysis population. |

Changing the analysis mode MUST clear incompatible markers and project only a matching sample into the new plot. Hidden RGB channels MUST NOT retain visible sample markers.

Plot-to-image column inspection MUST remain available under [Reading the plots](#reading-the-plots). It identifies a source band rather than a unique pixel and MUST remain distinct from image-to-plot pixel inspection.

Only the active inspection origin MUST drive correspondence markers. Entering plot inspection MUST clear viewer-driven sample markers; returning to viewer inspection MUST clear plot-driven guides.

## Reading the plots

### Histogram

Histogram MUST offer Tone and RGB views. Tone MUST be the default and MUST use a neutral trace rather than imply that any RGB channel represents brightness.

The value axis MUST identify the dark-to-light direction and its bounded analysis range. The frequency axis MUST identify relative pixel frequency rather than exposure or absolute luminance.

Linear and Logarithmic MUST change only the frequency-axis presentation. Logarithmic MUST be the default.

RGB MUST allow each channel to be shown or hidden. Channel overlap MUST remain readable, with a persistent textual channel identity and selected state.

Histogram MUST fill the distributions. Tone MUST use a neutral fill. RGB overlap MUST preserve secondary-color combinations and a neutral overlap of all three channels without hiding the distribution contours.

Endpoint accumulation MUST remain represented. The graph MUST NOT label it as proven clipping, missing detail, or an exposure error.

A cursor MUST expose the selected value interval and its share of the analyzed population. Keyboard inspection MUST provide the same readout as pointer inspection.

### Waveform

Waveform MUST offer Tone, RGB overlay, and RGB parade as display choices. Tone MUST be the default. Leaving Waveform and returning MUST retain its display choice.

The horizontal axis MUST span the analyzed image or selection from left to right. The vertical axis MUST show the same tone or channel-value definition used by Histogram.

Visible reference lines MUST identify the range endpoints and midpoint with numeric labels. Plot intensity MUST represent frequency, not a second image-brightness value.

Quarter-range guides MUST remain subordinate to the labeled endpoints and midpoint. RGB density overlap MUST NOT depend on channel draw order.

Pointer or keyboard inspection of a column MUST identify its corresponding vertical band on the photograph. It MUST NOT imply that one waveform point identifies one source pixel.

RGB overlay MUST retain the Histogram channel visibility choices. RGB parade MUST show all three channels without changing those saved visibility choices. Switching the waveform display MUST NOT change the population.

#### RGB parade

RGB parade MUST show R, G, and B in three named, adjacent plots. Each plot MUST use the Waveform position, value, and density definitions.

The three plots MUST use the same dimensions and scales. Cursor inspection MUST identify corresponding columns in all three plots and the photograph.

The view's help MUST distinguish channel comparison on a known neutral subject from the natural differences of colored subjects. It MUST NOT prescribe channel alignment as a universal target.

### Vectorscope

Vectorscope MUST show hue direction and chroma distance from a labeled neutral center. Plot intensity MUST show the relative frequency of colors.

The view MUST use one declared perceptual color plane. Its identity MUST be available without requiring the person to select a color model.

Fixed hue references MUST identify color directions. They MUST NOT be presented as a gamut boundary, a required skin-tone line, or preferred color harmonies.

The chroma scale MUST remain fixed across subject switches and accepted updates. Explicit chroma zoom MUST change only the plot window and MUST show its scale.

The panel MUST report colors outside that window instead of silently dropping them or suggesting that they lie inside the boundary.

The view's help MUST explain that color position does not identify an object's location. It MUST also explain that exposure can change measured chroma even though lightness is not plotted.

## Measurement meaning and limitations

All modes MUST use the same declared sRGB-based analysis interpretation for both subjects. Changing an image's declared encoding MUST NOT cause that encoding to be applied twice.

The panel MUST identify sRGB analysis independently of the output or monitor profile. It MUST NOT claim to predict another saving profile, output bit depth, printer, or physical spectrum.

The Tone view MUST describe a display-encoded relative-luminance signal. Help MUST distinguish this signal from physical light measurement and from a raw RGB channel.

Range information MUST distinguish samples below or above the analysis RGB range from samples exactly at its endpoints. It MUST NOT describe these observations as confirmed source or export clipping.

Invalid samples MUST be disclosed and excluded from ordinary distributions. They MUST NOT be replaced with black samples.

Preview measurements MUST retain the Preview detail-effects limitation. Analysis MUST NOT render a replacement image or reconstruct details absent from the accepted subject.

## Result identity and availability

The panel MUST identify the subject, population, and analysis interpretation beside the plot. Result identity MUST follow the accepted frame, not the latest requested adjustments.

| Condition | Required presentation |
| --- | --- |
| No accepted photographic subject | Explain the absence; show no previous photograph's plot. |
| Photographic edits make Modified stale | Keep its available measurements labeled as an earlier result. |
| Analysis for the selected subject, region, or mode is pending | Show Analyzing; a retained plot MUST remain identified by its actual subject and population. |
| The matching analysis completes | Publish the plot and its identity together. |
| An obsolete task completes | Do not replace the selected analysis. |
| The selected area is absent from the subject | Explain the area mismatch; do not substitute Full image. |
| No finite samples exist in the population | Explain that no valid samples are available and disclose the invalid population. |
| All RGB channels are hidden | Ask the person to select a channel; do not show No image or a fabricated distribution. |
| Analysis fails | Preserve the photographic result and explain that its analysis is unavailable. |

Image readiness MUST remain independent of analysis readiness. Failure of the analysis panel MUST NOT reject an otherwise accepted Preview or Scan or prevent Save and Export.

## Workspace and controls

Image analysis MUST be an optional workspace directly below the photograph and above the viewer footer, not a section in the parameter inspector.

The viewer MUST own its visibility and size. The right inspector MUST contain no analysis entry, plot, or reserved analysis space. A collapsed workspace MUST leave photographic controls and their scroll position untouched.

The workspace MUST have one quiet entry in the viewer presentation controls. It MUST start collapsed without a saved visibility preference.

Expanding it MUST preserve the selected photographic subject and view navigation. It MUST NOT create a dialog or another application page.

Closing it MUST restore the full photograph workspace and stop demand-driven analysis computation. Its visibility and requested height MUST remain application presentation preferences.

The expanded workspace MUST use the available width below the photograph. It MUST show one analysis mode at a time, not a dashboard.

Its compact toolbar MUST contain the mode, relevant signal choices, population actions, and an explicit close action. Information and available exceptions MUST remain legible in this viewer shelf. It MUST NOT move the photograph into a temporary inspector scroll area.

The expanded panel MUST present one **Analysis mode** selector. Histogram and Waveform MUST be its direct choices. Histogram MUST be the initial mode without a saved preference.

Vectorscope MUST be available through an initially collapsed **Advanced** group within that selector. A selected Vectorscope MUST remain identified by name in the closed selector.

RGB parade MUST belong to the Waveform display selector, not the analysis-mode selector. Each mode and waveform display choice MUST expose accessible help about the question it answers. The toolbar MUST NOT require a separate explanatory paragraph.

Signal and histogram-scale controls MUST show the current choice compactly and expose alternatives on demand. Irrelevant controls MUST be absent rather than occupy disabled toolbar space.

Essential mode and population controls MUST remain visible while expanded rather than appear only on hover. The panel MUST identify the subject, Full image or Selection, and sRGB interpretation in one compact scope line.

**Select area** and **Full image** MUST be actions in that scope, not repeated as a second population selector. Technical help MUST belong to accessible help rather than repeat the same meaning in multiple permanent rows.

The plot and its labeled coordinate guides MUST be the dominant content. Persistent height controls, explanatory paragraphs, and redundant zero-valued metrics MUST NOT squeeze it to a narrow trace.

A cursor or pixel readout MUST use a compact aligned numerical presentation relevant to the selected plot. Detailed definitions and ordinary zero range observations belong in help. Nonzero out-of-range or invalid observations MUST remain disclosed.

**Analysis settings** MAY own requested plot height. Increasing it MUST preserve a useful photograph area and all viewer controls.

A smaller window MUST reduce the available analysis height rather than hide photographic or delivery work. The workspace MUST NOT increase the inspector width.

Vectorscope MUST preserve equal horizontal and vertical units. RGB parade MUST preserve three readable plots rather than compress labels or alter scales to fit.

Changing the workspace height MUST retain photograph framing, zoom, and pan as view state. The viewer MAY refit the photograph to its remaining area while retaining the selected explicit zoom percentage. It MUST NOT change retained pixels or photographic adjustments.

Cursor readouts and selection outlines MUST use the existing presentation tokens. Focus and channel state MUST remain understandable without color alone.

Plot-driven column inspection MUST use a subdued location band with short boundary ticks rather than a bright line across the distribution. RGB parade MUST mark the same spatial interval in each channel plot. Viewer-linked pixel markers MUST remain visually distinct from this band.

The panel MUST NOT bind dragging or scrolling over a plot to exposure, black level, or another photographic editor.

## Preference ownership

The following choices MUST persist as application presentation preferences:

- Expanded state and requested height.
- Analysis mode, Histogram Tone/RGB choice, and Waveform display choice.
- Histogram scale and RGB visibility.
- Vectorscope chroma zoom.

Selections, inspection cursors, computed distributions, and pending work MUST remain transient. These values MUST NOT enter photograph records, look presets, runtime parameters, or export metadata.

Presentation changes MUST NOT create adjustment history or mark a photograph as having unsaved adjustments. Existing saved readout visibility MUST retain its meaning without exposing a second legacy panel.

Existing saved RGB parade preferences MUST restore Waveform with RGB parade selected. Existing waveform Tone/RGB choices MUST retain their meaning.

Saving migrated preferences MUST use the current hierarchy without obsolete top-level parade or waveform-RGB fields. A saved Vectorscope selection MUST remain available without enabling a second advanced-mode preference.

## Excluded capabilities

This capability MUST NOT introduce the following surfaces:

- Wavelength spectra or reconstructed per-pixel spectral measurements.
- Arbitrary analysis profiles or selectable perceptual color models.
- CIE chromaticity plots or output-gamut certification.
- Automatic exposure, neutrality, skin-color, or aesthetic judgments.
- Simultaneous plot dashboards or historical-result overlays.
- Analysis of transient reveal, animation, or automatic-comparison composites.
- Freeform masks, multiple selections, or selected-area photographic edits.

## Acceptance

1. A person can distinguish overall tone distribution, spatial tone structure, channel differences, and hue/chroma changes without treating a reference as a correctness target.
2. A person can compare Original and Modified for the same declared population and locate an inspected photograph pixel in each plot without changing the statistics or confusing sample identity.
3. Analysis remains readable and operable without changing photographic work, blocking delivery, or confusing invalid, pending, stale, and unavailable data.
4. The collapsed workspace occupies no inspector space; expanding and closing it preserves photographic edits and view state, with no analysis work while hidden.
5. The mode selector exposes Histogram and Waveform directly, with Vectorscope under Advanced. RGB parade remains available within Waveform and survives mode changes and application restart without changing the photograph.
