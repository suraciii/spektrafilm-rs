# Image-analysis boundaries

## Authority

[Image analysis](spec.md) owns photographic questions and observable behavior. This document owns the common measurement interpretation and the boundaries that keep those readings comparable.

[GUI workflow](../../gui-workflow/design.md) owns accepted image publication. [GPU execution](../../gpu-execution/spec.md) owns selected-backend execution and retained-sample residency.

## Accepted subject boundary

Analysis MUST consume an immutable accepted Original or Modified image with its actual color interpretation and source geometry. It MUST NOT infer these properties from live editors or viewer texture formats.

The analysis identity MUST include the following dependencies:

- Photograph and accepted subject generation.
- Color interpretation and accepted geometry.
- Analysis region in source coordinates.
- Measurement definition and plot-specific sampling grid.

The selected layer MUST remain separate from transient viewer composition. The existing layer selector MUST remain the sole subject-selection owner.

Source conversion, statistics, and rendering MUST remain separate from film simulation. Image-analysis failure MUST NOT participate in the acceptance decision for an otherwise completed photographic render.

## Common color interpretation

Analysis MUST decode samples according to their accepted encoding and transform their declared primaries and white point into unbounded linear sRGB under D65. It MUST reuse shared color-space conversion rules rather than interpret every carrier as linear sRGB.

The conversion MUST precede display ICC, viewer quantization, canvas composition, and saving conversion. It MUST NOT introduce tone mapping, gamut compression, per-image exposure normalization, or sample clipping.

Let `E` be the shared sRGB encoding function, extended to signed and above-white values. Analysis signals MUST follow this definition:

| Quantity | Definition |
| --- | --- |
| RGB signals | `E(R)`, `E(G)`, `E(B)` from unbounded linear sRGB. |
| Tone signal | `E(Y)`, where `Y` is the Y row of the shared linear-sRGB-to-XYZ matrix. |
| Color-plane coordinates | Oklab `a` and `b` from the unbounded linear sRGB triple, with signed cube roots. |
| Range observations | Below: any linear sRGB component less than zero; above: any component greater than one. |
| Invalid observation | A source triple or its required converted coordinates contain a nonfinite value. |

Oklab MUST follow the [linear-sRGB definition](https://bottosson.github.io/posts/oklab/#converting-from-linear-srgb-to-oklab). Analysis MUST NOT use HSV saturation or a luminance-normalized chromaticity as a substitute.

Oklab omits the plotted lightness coordinate, not lightness dependence. Its use avoids an absolute-display-luminance assumption; it MUST NOT establish color accuracy, illuminant recovery, or exposure-invariant chroma.

RGB range observations MUST use strict inequalities before plot-window clamping. Exactly black and exactly white MUST remain ordinary endpoint samples.

Range percentages MUST use finite analyzed pixels as their denominator. Below and above percentages MAY overlap when one pixel has components on both sides.

Invalid observations MUST be counted separately against the complete selected population. They MUST NOT contribute to signal bins, spatial-bin denominators, or color density.

## Population and geometry

Statistics MUST visit the native samples of the accepted subject within the declared area. A bounded plot grid MUST limit the reduction size, not replace samples with a downsampled or averaged image.

The population MUST NOT include viewer padding, overlays, animation, or viewport interpolation. Preview and Scan MUST retain their actual sample populations rather than be made artificially identical.

A region MUST be stored in normalized coordinates of the loaded source before presentation rotation. Each accepted subject MUST supply the source-to-image geometry that actually produced it.

Geometry mapping MUST reuse resolved crop rectangles and actual resize extents from that accepted result. Equal dimensions, shared viewer coordinates, or re-evaluated live crop controls MUST NOT establish correspondence.

A valid region maps to an axis-aligned rectangle under the supported rotation, crop, and resize operations. Membership MUST use pixel centers and half-open source-area bounds, including outer image edges by their corresponding half-open pixel extents.

The complete region MUST lie within the accepted subject's source coverage. A partially or completely missing region MUST produce area-unavailable status rather than an intersection population.

Geometry correspondence MUST use the retained subject's snapshot, including when Modified is stale. Pending crop or rotation controls MUST NOT relabel its existing pixels.

Whole-image comparison MUST compare each subject's complete accepted coverage. Different coverage MUST remain explicit under [Analysis subject and comparison](spec.md#analysis-subject-and-comparison).

## Distribution and scale definitions

Counts MUST retain population sizes; normalized peak heights alone are not an analysis result. Counter capacity MUST cover the population without wraparound.

### Histogram

Histogram MUST use 256 equal-width bins on the encoded signal interval from zero to one. Values outside that interval MUST accumulate in the nearest endpoint bin while retaining their separate range observations.

The bin index MUST be `min(255, floor(256 * clamp(v, 0, 1)))`. Histogram MUST retain one Tone distribution and three channel distributions.

Let `p` be a bin count divided by that distribution's finite population. Let `P` be the largest `p` across Tone and RGB for both accepted photographic subjects at the declared region.

The plotted height fraction MUST be `log1p(256 * p) / log1p(256 * P)`. This is the only histogram frequency mapping; there is no linear presentation branch.

The logarithmic mapping exposes low-frequency populated bins without changing their counts. Linear height ratios can express relative frequency directly, but exact inspection already reports that frequency. The fixed mapping removes a presentation choice, not an analysis signal.

The normalizer MUST include endpoint bins and hidden channels. Channel visibility and subject switching MUST NOT recompute it from only visible traces.

A subject that has no matching data MUST remain unavailable rather than contribute an invented distribution. Until a comparable pair is ready, analysis MUST disclose the pending counterpart instead of claiming a stable paired scale.

If only Original exists, its distributions MUST define `P`. New accepted subjects or regions MAY establish a new common normalizer; switching between an established pair MUST NOT change it.

A zero finite population MUST use the specified unavailable presentation instead of dividing by zero. Frequency readouts MUST report `p`, not height fractions.

Filled RGB contributions MUST combine additively with equal-strength channel primaries. Their common display opacity MUST NOT change the histogram heights or reported frequencies.

### Waveform and RGB parade

Spatial reduction MUST use at most 256 horizontal bins and 256 encoded signal bins. A subject or region narrower than 256 pixels MUST NOT acquire invented spatial samples.

Each spatial bin MUST cover equal intervals of the analyzed area's horizontal coordinate. Tone and RGB MUST use the Histogram signal-bin rule.

For each spatial bin, `p` MUST be the cell count divided by that bin's finite population. A bin with no valid samples MUST remain empty.

Density intensity MUST use `log1p(256 * p) / log1p(256)`. RGB parade and Waveform MUST share this mapping without independent per-channel gain.

RGB contributions MUST combine additively without channel-order dependence. Presentation MUST NOT apply an additional nonlinear gain to the defined density intensity.

Resizing the panel MUST scale the plot, not alter the reduction grid or density definition. A correspondence cursor MUST use the same accepted geometry and spatial interval as its indicated band.

### Vectorscope

Vectorscope MUST use a 256 by 256 grid in the Oklab `a,b` plane. Positive `a` MUST point right and positive `b` MUST point up.

The default window MUST span `-0.4` through `0.4` on both axes. Chroma zoom MUST offer 1x, 2x, and 4x by dividing both axis extents by that factor.

Fixed references MUST derive from Oklab transforms of unit linear-sRGB red, yellow, green, cyan, blue, and magenta. Neutral MUST be identified at `a = b = 0`.

References MUST move with explicit plot zoom without becoming clipped-data substitutes. The plot MUST NOT draw an RGB-cube outline as a gamut certificate.

Each in-window coordinate MUST contribute to its containing cell. Window endpoints MUST enter the outer cells; outside coordinates MUST contribute only to an outside-window count.

Let `p` be a cell count divided by the complete finite selected population. Density intensity MUST use `log1p(65536 * p) / log1p(65536)` without image-dependent gain.

Color tint MUST identify hue direction, not average source RGB within a bin. Density MUST remain legible independently of that tint, including at the neutral center.

## Selection and inspection boundary

A selection gesture MUST capture the displayed subject and geometry at its start. A changed subject or geometry MUST invalidate the draft rather than reinterpret its screen coordinates.

Selection mode MUST consume only the explicitly armed primary-drag interaction. Outside that mode, [viewer gestures](../../gui-workflow/spec.md#viewer-interaction) MUST retain their existing meanings.

Keyboard region editing MUST use focusable move and resize handles. Confirm MUST commit one region, while Escape MUST leave the previous committed region unchanged.

Inspection cursors MUST remain separate from committed regions. Their guide bands and readouts MUST NOT create image edits, selection changes, or persistent overlays.

### Linked pixel sampling

Linked inspection MUST reuse the viewer's accepted-subject coordinate mapping and bounded pixel-readout boundary. It MUST sample one native subject pixel rather than a screen RGB8 value, an interpolated display value, or a neighborhood average.

The inspected pixel's analysis values MUST use [Common color interpretation](#common-color-interpretation). Original carrier RGB readouts MUST remain labeled separately from these derived analysis values.

The sample identity MUST capture these dependencies:

- Photograph, accepted subject generation, and color interpretation.
- Accepted geometry and native pixel coordinates.
- Committed analysis region and its membership result.
- Analysis mode, signal choices, visible channels, and plot window.
- Active inspection origin and latest inspection-request sequence.

Coordinates alone MUST NOT authorize a sample response after subject replacement or allocation reuse. A result MUST match the complete active identity before its readout or markers can be published.

Sample movement MUST supersede obsolete requests without building a backlog. Clearing or suspending inspection MUST invalidate pending publication even when the pointer later returns to the same coordinates.

Region membership MUST use the same mapped pixel center and bounds as statistical reduction. An outside-selection sample MUST remain excluded from the selected population and its normalization.

Sample overlays MUST share the plot's immutable subject and population identity. Histogram sample values MUST use the signal-bin rule without adding counts or deriving vertical position from sample magnitude.

Waveform and parade sample markers MUST use the statistical spatial bin and signal coordinate. Vectorscope MUST use the same Oklab coordinates and explicit plot window as its density grid.

Off-scale samples MUST retain their actual finite values and membership status. Their display MUST NOT change endpoint counts, density grids, normalization, or outside-window totals.

Viewer-driven sample inspection and plot-driven band inspection MUST share one active presentation owner. Neither MUST mutate the other's source coordinates to imply a reversible pixel-to-density mapping.

GPU sample conversion MUST execute in the selected device domain. The host MAY receive a bounded sample and its derived coordinates, but MUST NOT perform image arithmetic as an implicit CPU fallback.

Pointer movement, keyboard inspection, and asynchronous readback MUST NOT block viewer navigation or photographic work. Inspection failure MUST invalidate only that readout, not its accepted image or distribution.

## Execution and publication

Analysis MUST run outside the UI event loop on the selected computational backend. A selected GPU MUST reduce resident samples without full-image host materialization or host pixel arithmetic.

The host MAY receive bounded count grids, population totals, and inspection readouts. It MUST NOT receive a second full floating image to construct the panel.

The implementation MUST reuse accepted source carriers and shared color transforms. It MUST NOT retain an image-sized analysis clone or add a film-pipeline stage.

Work MUST be demand-driven for an actually expanded panel. A requested expanded state alone MUST NOT schedule analysis. Collapsing the panel MUST invalidate pending analysis publication without canceling the photographic render.

Subject changes and region changes MUST supersede obsolete analysis. An accepted analysis result MUST match every dependency in its captured identity before publication.

Image and analysis publication MUST remain independent. The panel MAY retain an older plot while a matching result is pending only with the identity required by [Result identity and availability](spec.md#result-identity-and-availability).

Cancellation MUST follow the [GPU cancellation boundary](../../gpu-execution/design.md#bounded-execution). Releasing an analysis result MUST NOT invalidate device work that still holds its samples.

## Layout and preference boundary

The panel MUST reuse native viewer components and semantic tokens. It MUST NOT import a prototype palette, hide essential controls on hover, or create another settings destination.

Presentation MUST distinguish the analysis mode from the Waveform display choice. RGB parade MUST remain a waveform layout, not a fourth mode. This mapping MUST preserve existing counts, coordinates, density, and channel normalization.

The selected mode and waveform display MUST remain sufficient to restore the chart. The mode selector MUST NOT add a persistent advanced-state flag or an extra navigation boundary for Vectorscope.

### Space allocation

The viewer layout MUST be the single owner of total shelf allocation. Shared sizes MUST come from the [native layout baseline](../../gui-presentation/spec.md#native-layout-baseline), not independent constants in the toolbar and plot.

Let `A` be the nonnegative central height remaining after reserving the viewer status and footer. Let `F` be the native photograph reserve. Let `M` be the native minimum shelf height.

An explicit height request MUST clamp to the native shelf resize range. Let `R` be that request and `C = min(shelf_maximum, max(0, A - F))` be the effective maximum.

The shelf MUST render only when expansion is requested and `C >= M`. Its total height MUST then be `min(R, C)`. Otherwise, the viewer MUST omit the shelf, its divider, and its controls, leaving all of `A` to the photograph.

Effective visibility MUST derive from the requested expanded state and available space. It MUST NOT replace the stored request. Temporary collapse and restoration MUST follow [Height and interaction](spec.md#height-and-interaction).

Toolbar, plot, borders, and padding MUST fit inside the allocated shelf. Components MUST NOT append another height allowance after that allocation. Remaining central space MUST belong to the photograph.

Divider input MUST change only the requested shelf height. The plot MUST receive the remaining inner rectangle, not apply a second requested-height preference. Keyboard and pointer resizing MUST use the same allocation rule.

Inspection readouts and conditional statuses MUST remain inside the assigned toolbar or plot rectangle. Their presence MUST NOT change the requested height or allocate an empty information strip.

Layout changes MUST invalidate only affected presentation geometry or rasters. They MUST NOT change reduction identity, sample membership, bin counts, or the common frequency normalizer.

### State and preference cutover

Displayed plot data and its subject/population identity MUST remain one publication. Removing the ordinary scope line MUST NOT disconnect a pending or stale plot from the identity required by [Result identity and availability](spec.md#result-identity-and-availability).

Analysis preferences MUST remain separate from photographic records and runtime state. Full-image mode, transient regions, retained choices, and compatibility behavior follow [Preference ownership](spec.md#preference-ownership).

The loader MUST distinguish a current total-height request from a legacy plot-only height. It MUST NOT reinterpret the old number as a shelf height or recreate the removed toolbar allowance. Legacy scale values MUST be consumed only for migration, not retained as runtime choices.

Reading legacy saved histogram visibility MUST migrate that visibility once without preserving a second histogram implementation. The loader MUST convert the previous top-level parade choice and waveform-RGB boolean into the current waveform display. Current waveform-display values MUST take precedence over a retained obsolete boolean.

Successful migration MUST use the existing independent preference writer. Runtime preferences and newly saved data MUST NOT retain removed fields or duplicate mode representations. Migration MUST NOT change photographic values or create adjustment history.
