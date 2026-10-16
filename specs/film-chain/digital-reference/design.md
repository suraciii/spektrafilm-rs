# Digital Reference design

## Ownership

The [product contract](spec.md) owns behavior and acceptance. The [domain glossary](../../../CONTEXT.md) owns terms. The Film chain owns this addition; it does not create another workflow or subsystem.

Scan preparation must resolve a reference receiver, its common color interpretation, its reference tone placement, and fixed normalization as one compatible conversion configuration. This coupling is specific to reference conversion. Existing receiver-independent `density` behavior must remain intact.

The implementation must reuse the existing Scan entry point, prepared configuration identity, output processing, and state validation boundaries. A prepared reference conversion must not mutate film preparation or read image statistics. Image pixels must reuse prepared constants or bounded tables; they must not rebuild the model or run an iterative color solver per pixel.

The selected film's base and single reference reading may supply fixed normalization. They must not supply an exposure-reconstruction inverse or a stock-specific dye-separation matrix. The reference model's version, data sources, illumination, bands, separation basis, and tone convention must identify its prepared configuration and provenance.

Manual corrections must apply in a declared reference-output coordinate system. Existing reconstruction controls must not be silently reused where their exposure-axis meaning differs. Reference conversion must own its endpoint policy; shared output processing must retain its clipping and encoding boundaries.

## Alternatives

A preset over `density`, disabled setup, and neutral finish would reuse all existing controls. It would still reconstruct exposure through the selected film and would not implement reference conversion. Such a preset must not be labeled Digital Reference.

A new finish would receive already converted RGB. It would not own spectral reception or dye separation. It would not implement the required behavior.

A matched reference receiver and conversion inside Scan preserves the existing stage boundary and implements the required common output model. The existing conversion-method and receiver selectors carry that choice.

## Implementation decisions

### Public selection

The `scanner.negative_conversion_method` enum carries `digital_reference`. The existing `scanner.receiver_profile` enum adds the matching `digital_reference`. Both values are required together. This reuses the existing parameter source, preset, state, recipe, and discovery boundaries without adding a Scan treatment field. Defaults remain unchanged.

Manual controls are `scanner.reference_exposure_ev` and `scanner.reference_balance_ev`. They default to zero. Their output-gain semantics belong to the [product contract](spec.md#reference-exposure). Existing reconstruction controls must remain at their defaults in this mode.

### Numerical reference model

The project-authored model is `spektrafilm_digital_reference_v1`. It uses equal-energy illumination on the production 380–780 nm grid at 5 nm spacing. The receiver bands are `10^log_sensitivity` from the runtime data directory's `kodak_portra_endura` profile, normalized to unit sums. The common dye basis comes from the same directory's `kodak_portra_400` profile. The model must retain the source profiles' attribution and CC BY-SA 4.0 metadata. It does not reuse Fotufilm source or claim its pixel response.

Missing spectral samples represented by JSON `null` follow the existing profile conventions: sensitivity contributes zero, and a wavelength with a missing base or dye reading contributes zero transmission. Infinite source values fail preparation. Receiver, basis, and selected-film grids must match the production grid exactly. The common dye matrix must be invertible with Frobenius condition number at most 1000; the reference exposure axis must be finite, strictly increasing, and cover zero.

For color, the fixed small-signal matrix is `J[c,k] = sum_i(weights[i,c] * reference_dyes[i,k])`. Preparation validates and inverts this common matrix. The selected film supplies its dye spectra and clear-base capture. Receiver density is `-log10(T / Tclear)`. The common inverse maps that density into reference coordinates. One selected-film reading at relative log10 exposure zero supplies a positive anchor per axis. No other selected-film curve samples enter the inverse or tone response.

The normalized coordinate is `x = reference_coordinate / anchor`. The common signed response is `sign(x) * x^2 / (x^2 + 41/9)`. Clear base maps to zero; the declared reference reading maps to 0.18. Finite out-of-reference coordinates follow the same continuous response, approaching negative or positive unit values. Evaluation must avoid squaring overflow. The monochrome path uses the mean receiver optical density and one monochrome reference anchor, then emits equal channels.

The common assignment is linear sRGB. Reference exposure and balance gains apply there before transformation into the requested output color space. A color-space change must not change receiver weights, the common matrix, or anchors. Scanner lens blur operates on receiver capture before reference conversion.

Unblurred capture uses base-10 logarithmic spectral integration to retain finite extreme-density measurements without a floor or multiplication by `ln(10)`. Lens blur requires positive transmissions representable in the build's native precision; an unrepresentable capture fails rather than clipping the measurement. Prepared provenance records the selected film's clear base-10 log readings and reference coordinates.

Each prepared reference configuration must retain SHA-256 identities of both runtime-resolved source files and the model version. Missing or invalid source data, a singular matrix, a nonpositive anchor, or nonfinite input must fail explicitly. The diagnostic exact-cell capture path must reject reference conversion before rendering because it uses a different capture representation.

Native-f32 and GPU comparisons must use measured budgets recorded with acceptance evidence under [GPU numerical alignment](../../gpu-execution/design.md#numerical-alignment). Reference conversion must execute on the requested backend without a conversion-specific CPU override.

### Additional placements and controls

Reference Exposure is the supported placement. Graded Print, Auto Levels, custom receiver-band editing, paper grade, and reference-axis black/white points are outside this addition. Existing nonidentity point windows fail validation. Their exposure-reconstruction coordinates must not be silently reused for reference conversion.

### Source reuse

If Fotufilm source or spectral assets are reused, record the license and provenance of each selected asset. Code licensing and data licensing are separate inputs to that decision. Record source revisions and content identities with the implemented model.

## Research basis

The researched source is Fotufilm revision `59d43471dbef45c85ae442e507a291fdebfb2ec5`:

- [Receiver definition](https://github.com/DhaliwalX/fotufilm-engine/blob/59d43471dbef45c85ae442e507a291fdebfb2ec5/Sources/FotufilmCore/DigitalReferenceReceiver.swift#L168) distinguishes the common reference model from a selected-film inverse.
- [Common curve and base placement](https://github.com/DhaliwalX/fotufilm-engine/blob/59d43471dbef45c85ae442e507a291fdebfb2ec5/Sources/FotufilmCore/DigitalReferenceReceiver.swift#L197) show that fixed placement still includes a tone convention and stock-specific base normalization.
- [Dye separation](https://github.com/DhaliwalX/fotufilm-engine/blob/59d43471dbef45c85ae442e507a291fdebfb2ec5/Sources/FotufilmCore/DigitalReferenceReceiver.swift#L502) derives a common small-signal response from a shared dye basis.

The existing `density`, `separable`, and `coupled` methods retain exposure reconstruction, optional automatic setup, and independent neutral or Lab Scan finishing. Digital Reference supplies the common reference alternative. The `legacy_colorimetric` receiver and `density` with neutral finish retain their prior meanings.
