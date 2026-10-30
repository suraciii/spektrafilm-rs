# Endpoint-warning design

## Ownership

[The product contract](spec.md) owns visible behavior and endpoint meaning. Viewer presentation owns requested warning flags and temporary isolation.

The display worker owns disposable warning computation and publication. The backend owns classification, coverage reduction, and CPU/GPU agreement.

The film pipeline, photographic parameter model, look presets, and file writers MUST NOT consume warning state. Existing image analysis MUST retain its own sample interpretation and demand boundary.

## Sample boundary

Warnings MUST read the immutable accepted native carrier, not a fitted, display-transformed, ICC-converted, interpolated, or quantized raster.

For a linear carrier, the native endpoints MUST be zero and one. For an encoded carrier, classification MUST use the native transfer function's encoded zero and encoded one at the carrier precision.

This monotone comparison preserves native linear endpoint meaning without per-pixel decoding or a primaries conversion. ACEScct code zero and code one MUST NOT substitute for its encoded endpoints.

The implementation MUST reject a non-RGB sample domain rather than interpret density or log exposure as image RGB. Nonfinite triples MUST be excluded before endpoint comparisons.

The warning API MUST carry the accepted source independently of the disposable display image. Display ICC and output saving conversion MUST NOT select the warning endpoints.

No endpoint observation contains capture or processing provenance. The interface MUST NOT infer which earlier operation produced a qualifying value.

## Coverage boundary

A bounded viewport mask MUST accumulate native source-pixel classifications. A reduced axis MUST include each native cell with positive-area overlap.

A magnified or source-pixel-scale axis MUST select the native cell containing the viewport-cell center. Only viewport-cell centers inside the half-open photograph rectangle MAY receive warning presentation.

Classification MUST precede the display filter. Conservative coverage prevents a qualifying pixel from disappearing during Fit and prevents interpolation overshoot from creating a warning. Exact source-pixel zoom MUST map to native cells without an adjacent-cell halo.

The source-pixel crossover MUST allow relative geometry roundoff of `8 * f32::EPSILON`. This allowance MUST NOT alter RGB endpoint comparisons.

Traversal MUST exclude source regions that cannot contribute to the visible photograph. Mask storage MUST be bounded by the current presentation grid instead of retaining a second full floating photograph.

CPU and GPU coverage MUST reuse one per-axis geometry rule. Host geometry metadata MUST scale with viewport axes rather than require a per-pixel host cell table.

## Execution and publication

Warning work MUST run in the existing background display composition. CPU composition MUST use the shared host reference.

Selected GPU composition MUST classify resident source pages and apply the warning on the selected device before packed RGBA presentation readback. It MUST NOT materialize the complete photograph on the host or add a film-render stage.

Native work inside each GPU invocation MUST be bounded. Dispatch batches MUST observe cancellation under the [bounded-execution contract](../../gpu-execution/design.md#bounded-execution).

Warnings off MUST preserve the existing composition path without warning scratch allocation or dispatch. Native source uploads MUST remain reusable across view-only changes.

Warning mode MUST participate in viewer request identity. Source ownership, accepted color interpretation, geometry, presentation grid, backend, and cancellation MUST follow the existing retained-display publication boundary. A superseded or canceled request MUST NOT publish a warning mask.

Failure MUST leave the accepted photograph available. Temporary composites MUST suspend effective warnings rather than attach one accepted subject's mask to blended pixels.

## Preference boundary

Requested black and white flags MUST live in independent application presentation preferences. Temporary hold mode MUST remain viewer-local and transient. Changing warning state MUST NOT use photographic parameter synchronization or create an adjustment-history entry.

Preference publication MUST use an isolated sibling temporary file and atomic replacement under the existing application configuration conventions. Malformed or unreadable preferences MUST be reported without silently replacing the file during loading.

## Alternatives

Classifying final display bytes would include ICC clipping, quantization endpoints, and display-filter artifacts. It would also lose original super-white and negative sample values. The accepted-source boundary avoids those changes in meaning.

Reusing analysis below/above counts would substitute strict any-channel sRGB range observations for native endpoint warnings and would couple the feature to an expanded analysis workspace. The separate viewer boundary preserves both contracts.

Storing warning switches in photographic GUI state would let photograph reopening or preset application alter a viewing preference. Independent preference ownership avoids that coupling.
