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
| Vectorscope | How did the look change the image's colors? | Hue direction, chroma, and population density, with fixed references for comparing the same subject before and after simulation. |

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

Select area MUST remain available while an accepted photographic subject supports area inspection. Full image MUST appear only while a committed Selection exists, including an unavailable selection. A selected area MUST remain identifiable on the photograph while the panel is expanded.

Full image is an action that clears the selection, not a second population selector. The active selection tool and committed selection MUST remain distinguishable through their control states and photograph overlays, without a permanent population row.

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
| Vectorscope | Mark the sample's position in the declared color plane, independently of population-density intensity. |

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
| A selection gesture, viewer navigation gesture, or temporary comparison/composite is active | Suspend linked markers and readouts without changing the analysis population or adding a routine suspension message. |

Changing the analysis mode MUST clear incompatible markers and project only a matching sample into the new plot. Hidden RGB channels MUST NOT retain visible sample markers.

Plot-to-image column inspection MUST remain available under [Reading the plots](#reading-the-plots). It identifies a source band rather than a unique pixel and MUST remain distinct from image-to-plot pixel inspection.

Only the active inspection origin MUST drive correspondence markers. Entering plot inspection MUST clear viewer-driven sample markers; returning to viewer inspection MUST clear plot-driven guides.

## Reading the plots

### Histogram

Histogram MUST offer Tone and RGB views. Tone MUST be the default and MUST use a neutral trace rather than imply that any RGB channel represents brightness.

The value axis MUST run from dark to light, left to right. Its bounded range and the meaning of relative pixel frequency MUST remain available through accessible help and inspection readouts, not permanent explanatory labels.

Histogram MUST NOT reserve an axis-label row or show permanent `0`, `0.5`, and `1` value labels. Exact value intervals MUST remain available through cursor inspection.

Histogram MUST use fixed logarithmic frequency presentation under the [scale definition](design.md#histogram). It MUST NOT expose a Linear/Logarithmic selector or a permanent Logarithmic label.

This mapping MUST change only plotted frequency height, not signal values, bin membership, counts, or reported frequencies. Help MUST distinguish logarithmic height ratios from population-frequency ratios.

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

Accessible help MUST identify sRGB analysis independently of the output or monitor profile. The workspace MUST NOT repeat it as a permanent label or scope row. Analysis MUST NOT claim to predict another saving profile, output bit depth, printer, or physical spectrum.

The Tone view MUST describe a display-encoded relative-luminance signal. Help MUST distinguish this signal from physical light measurement and from a raw RGB channel.

Range information MUST distinguish samples below or above the analysis RGB range from samples exactly at its endpoints. It MUST NOT describe these observations as confirmed source or export clipping.

Invalid samples MUST be disclosed and excluded from ordinary distributions. They MUST NOT be replaced with black samples.

Preview measurements MUST retain the Preview detail-effects limitation in accessible help. The workspace MUST NOT repeat the viewer's existing Preview caveat as another permanent row. Analysis MUST NOT render a replacement image or reconstruct details absent from the accepted subject.

## Result identity and availability

Result identity MUST follow the accepted frame, not the latest requested adjustments. The viewer's subject identity, selection state, and accessible interpretation help MUST identify a matching result without a duplicate scope line.

A retained result that differs from the selected subject, population, or mode MUST disclose its actual identity while pending or stale. Removing routine metadata MUST NOT make such a result appear current or matching.

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

### Placement

Image analysis MUST be an optional workspace directly below the photograph and above the viewer footer, not a section in the parameter inspector.

The viewer MUST own its visibility and size. The right inspector MUST contain no analysis entry, plot, or reserved analysis space. A collapsed workspace MUST leave photographic controls and their scroll position untouched.

The workspace MUST have one quiet entry in the viewer presentation controls. It MUST start collapsed without a saved visibility preference.

Expanding it MUST preserve the selected photographic subject and view navigation. It MUST NOT create a dialog, floating photograph overlay, or another application page.

Closing it MUST restore the full photograph workspace and stop demand-driven analysis computation.

The expanded workspace MUST use the available width below the photograph. It MUST show one analysis mode at a time, not a dashboard. Its ordinary state MUST consist of one toolbar row and the plot.

### Toolbar

While the shelf is expanded, its toolbar MUST contain only controls that change the inspection question, signal, population, plot window, or workspace visibility.

| Control | When visible | Choices or action |
| --- | --- | --- |
| Analysis mode | Always | Histogram, Waveform, and Vectorscope as direct choices. |
| Histogram signal | Histogram | Tone or RGB. |
| Waveform display | Waveform | Tone, RGB overlay, or RGB parade. |
| Channel visibility | Histogram RGB or Waveform RGB overlay | Independently show or hide R, G, and B, with textual identity and selected state. |
| Chroma zoom | Vectorscope | Change the plot window while identifying the selected zoom. |
| Select area | An accepted photographic subject supports selection | Arm the temporary area-selection tool. |
| Full image | A committed Selection exists | Clear the selection under [Area inspection](#area-inspection). |
| Close | Always | Close the analysis workspace. |

Histogram MUST be the initial mode without a saved preference. Vectorscope MUST NOT require an Advanced group or a separate enable flag. The closed selector MUST name the selected mode.

RGB parade MUST remain a Waveform display choice, not a separate analysis mode. Its three channel plots MUST retain their labels without showing redundant channel-visibility controls.

Each mode and signal/display choice MUST expose accessible help about the question it answers. Current choices and applicable actions MUST remain visible without hover. Irrelevant controls MUST be absent rather than disabled placeholders.

The toolbar MUST NOT expose Analysis settings, a height editor, or another overflow destination for removed controls. Its controls MUST remain on one row at the supported acceptance sizes in both English and Simplified Chinese.

### Conditional information

The ordinary workspace MUST NOT contain a scope line, explanatory paragraph, empty readout row, or redundant zero-valued metrics. Design instructions about how the panel behaves MUST NOT appear as application content.

Cursor or pixel readouts MUST appear only during active pointer or keyboard inspection. They MUST use compact, aligned numerical values relevant to the selected plot.

Readouts and meaningful exceptions MUST use the toolbar or plot area without reserving a permanent information row. Their appearance and removal MUST NOT resize the photograph or plot.

Readouts MUST NOT intercept plot pointer or keyboard inspection. They MUST leave linked sample markers and required references visible.

Active sample coordinates, values, and membership or off-scale indications MUST remain visible without scrolling or leaving the viewer inspection cursor. Excess notices MUST remain scrollable inside the assigned plot region without resizing it. Focused plot inspection MUST keep arrow keys for sampling and provide Page Up and Page Down for notice scrolling.

Nonzero out-of-range, invalid, unavailable, pending, and stale observations MUST remain disclosed under [Result identity and availability](#result-identity-and-availability). These states MUST NOT be hidden in help merely to make the panel smaller.

### Height and interaction

The height preference MUST describe the entire shelf, including controls, plot, borders, and padding. Default height, resize limits, and the photograph reserve MUST follow the [native layout baseline](../../gui-presentation/spec.md#native-layout-baseline).

Allocation MUST reserve the viewer status, viewer footer, and useful photograph area before assigning analysis height. It MUST NOT limit the plot and then append toolbar or information height outside that limit.

Dragging the photograph/analysis divider MUST resize the shelf. The divider MUST expose a resize cursor and visible keyboard focus. Keyboard operation MUST provide equivalent resizing and bounds.

A drag MUST account for its complete displacement from the press position. Under a height constraint, an upward resize MUST NOT reduce the requested height. A downward resize MUST start from the displayed height for both pointer and keyboard input.

Double-clicking the divider MUST restore the default requested height. Keyboard operation MUST provide an equivalent reset without opening a settings surface.

A smaller window MUST reduce effective shelf height only while the native minimum remains available. If the photograph reserve and minimum shelf cannot both fit, the workspace MUST enter temporary collapse instead of rendering a smaller shelf.

Window constraints MUST NOT overwrite the requested expanded state or height. Temporary collapse MUST preserve display choices and the committed selection. Restoring sufficient space MUST reopen the shelf only if expansion is still requested.

Temporary collapse MUST stop analysis demand and follow the selection-cancellation and inspection-clearing rules for collapse. If it removes the focused control, focus MUST return to the viewer's analysis action.

The viewer-footer action MUST remain operable while space is insufficient. It MUST expose the requested expanded state and an accessible insufficient-space explanation. Turning it off MUST prevent automatic reopening; requesting expansion MUST wait for sufficient space.

The workspace MUST NOT increase inspector width or put the photograph in a scroll area.

The layout MUST preserve the shared text and control sizes rather than shrink them to fit. Vectorscope MUST retain equal horizontal and vertical units. RGB parade MUST retain three readable plots with matching scales.

Expanding, resizing, closing, or restoring analysis, including through temporary collapse, MUST preserve the viewer's zoom mode and pan. These actions MUST NOT change retained pixels or photographic adjustments.

| Viewer zoom mode | Required response to canvas resizing |
| --- | --- |
| Fit | The viewer MUST recompute the fitted scale for the new canvas without resetting its existing zoom multiplier. |
| Explicit percentage | The viewer MUST preserve the actual source-pixel-to-device-pixel percentage, not just its label. Only the visible viewport changes; resizing MUST NOT invoke Fit or reset zoom. |

Cursor readouts and selection outlines MUST use the existing presentation tokens. Focus and channel state MUST remain understandable without color alone.

Plot-driven column inspection MUST use a subdued location band with short boundary ticks rather than a bright line across the distribution. RGB parade MUST mark the same spatial interval in each channel plot. Viewer-linked pixel markers MUST remain visually distinct from this band.

The panel MUST NOT bind dragging or scrolling over a plot to exposure, black level, or another photographic editor.

## Preference ownership

The following choices MUST persist as application presentation preferences:

- Requested expanded state and requested total shelf height.
- Analysis mode, Histogram Tone/RGB choice, and Waveform display choice.
- RGB visibility.
- Vectorscope chroma zoom.

Selections, inspection cursors, computed distributions, and pending work MUST remain transient. These values MUST NOT enter photograph records, look presets, runtime parameters, or export metadata.

Presentation changes MUST NOT create adjustment history or mark a photograph as having unsaved adjustments. Existing saved readout visibility MUST retain its meaning without exposing a second legacy panel.

Existing saved RGB parade preferences MUST restore Waveform with RGB parade selected. Existing waveform Tone/RGB choices MUST retain their meaning.

Saving migrated preferences MUST use the current hierarchy without obsolete top-level parade or waveform-RGB fields. A saved Vectorscope selection MUST remain available without enabling a second advanced-mode preference.

Legacy Linear/Logarithmic preferences MUST NOT restore a linear display branch. Legacy plot-only heights MUST reset to the compact total-height default unless a current total-height preference exists. Saving preferences MUST omit removed scale and plot-only height fields.

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

Acceptance MUST exercise the native GUI with an accepted photograph, not use an HTML layout as implementation evidence.

| Scenario | Required observable result |
| --- | --- |
| Open analysis with no saved preferences | The compact Histogram Tone shelf uses the native default height and fixed logarithmic presentation; no settings, scale, scope, explanation, or empty readout row appears. |
| Inspect at 1100 × 760 and 1460 × 980 in English and Simplified Chinese | Applicable controls fit one toolbar row with normal text sizes; the photograph remains dominant and viewer, Scan, Save, and Export actions remain reachable. |
| Switch analysis modes and Waveform displays | All three modes are direct choices; RGB parade stays within Waveform; returning to a mode preserves its relevant display preferences. |
| Switch Tone, RGB, RGB overlay, RGB parade, and Vectorscope | Only relevant signal, channel, or chroma controls appear; histogram axes are quiet, while waveform numeric guides and vectorscope geometry remain readable. |
| Select an area, clear it, then encounter an unavailable selection | The outline and action state identify the selection; Full image appears only when a selection exists; unavailable coverage is not silently replaced or intersected. |
| Inspect a plot or photograph with pointer and keyboard, then leave | Matching markers and exact readouts appear and clear without changing statistics, plot size, or photograph size. |
| Resize by divider and keyboard; reset; shrink and restore within the expanded budget | Total shelf allocation stays within the native budget, controls remain reachable, and window constraints do not replace requested height or alter photographic edits. |
| Resize while a larger requested height is constrained by the window | Upward input does not reduce the saved request; downward pointer and keyboard input shrink the visible shelf immediately; gradual drags retain their full displacement. |
| Shrink below the minimum expanded budget, then restore to and above its boundary | The shelf, divider, and toolbar disappear rather than compress below the minimum; the footer action explains the constraint; requested expansion and height restore when space permits. |
| Collapse for insufficient space during selection, inspection, or pending work | Unfinished selection and inspection clear, analysis demand stops, and focus moves out of removed controls; the committed selection and display choices remain available on reopening. |
| Turn analysis off or request expansion while space is insufficient | Off prevents automatic reopening; requested expansion waits for sufficient space; saved preferences retain the requested state rather than effective visibility. |
| Open, resize, close, and restore analysis in Fit mode | The fitted scale follows the new canvas while preserving the existing zoom multiplier and pan. |
| Repeat in explicit 100% and 200% modes with a panned photograph | Actual source-to-device pixel scale and pan remain unchanged; no automatic Fit occurs, even during temporary collapse and restoration. |
| Compare Original and Modified at the same region | Counts, exact frequencies, and shared normalization remain comparable; logarithmic peak heights are not presented as frequency ratios. |
| Encounter pending, stale, missing, invalid, out-of-range, or failed analysis | Actual result identity and meaningful exceptions remain visible without claiming that an earlier result matches the new request or blocking image delivery. |
| Hide every RGB channel | The panel requests a channel rather than claiming that the photograph or valid population is absent. |
| Restore legacy linear, plot-height, parade, and waveform preferences | Removed choices do not reappear; valid mode and signal choices retain their meaning; saving emits only the current preference model. |
| Close analysis, including during selection or pending work | Photograph space is restored, analysis demand stops, and normal viewer gestures resume without creating an adjustment-history entry. |

## Implementation status

The native GUI implements the compact toolbar, fixed logarithmic Histogram, conditional plot information, divider resizing, space-constrained collapse and restoration, and preference cutover described in the [GUI guide](../../../docs/gui.md#image-analysis).

Native acceptance MUST exercise the scenarios above. Headless geometry and state regressions MUST NOT substitute for native interaction or cross-platform acceptance.
