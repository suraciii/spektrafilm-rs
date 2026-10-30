# Common finishing design

## Ownership

The [product contract](spec.md) owns control behavior, applicability, and delivery. The design preserves one finishing definition without adding a second image pipeline.

| Boundary | Owner |
| --- | --- |
| Typed values, neutral defaults, validation, and canonical metadata | Core runtime parameters |
| Route, medium, and delivered-polarity applicability | Core film-chain resolution |
| Fixed response definition and prepared color transforms | Core finishing |
| Image resources and device dispatch | The selected compute backend |
| Look capture, section scope, and document migrations | Presets and each versioned carrier |
| Control presentation, transient comparison, and edit transactions | GUI workflow |
| Prepared-source composition | Output mix |
| Delivery conversion and bounded output clipping | Photo export |

CLI, GUI, LUT baking, and diagnostic collection MUST consume the same effective parameter model. They MUST NOT own private scales, route predicates, or response implementations.

## Color and order

The finishing boundary MUST receive the completed positive after eligible print viewing glare. It MUST precede existing output gamut processing.

The remaining tail MUST retain the [film-chain order](../film-chain/spec.md#contract). Finishing MUST NOT move a nonlinear response across scanner optics, readout glare, output gamut, or encoded-domain appearance processing.

An active finishing evaluation MUST follow this order:

1. Interpret the supplied linear output RGB using its declared primaries and white point.
2. Transform into fixed linear sRGB with D65 adaptation.
3. Apply optional Expand in fixed sRGB encoding, then decode to linear RGB.
4. Convert to Oklab and apply the joint tone response.
5. Apply the fixed warm–cool response.
6. Apply selective saturation and invert Oklab.
7. Transform back into declared linear output RGB for existing output gamut processing.

The [public linear-sRGB Oklab definition](https://bottosson.github.io/posts/oklab/#converting-from-linear-srgb-to-oklab) owns the coordinate interpretation. Signed cube roots MUST extend the transform to negative LMS coordinates without a zero floor.

Changing output primaries MUST change only the boundary transforms, not the finishing definition. Saving primaries and viewer transforms MUST NOT enter this evaluation.

Linear sRGB coordinates MUST NOT imply clipping into the sRGB gamut or reconstruction of scene radiance. Working conversion MUST preserve finite signed samples and super-white headroom admitted by the image pipeline.

Default finishing MUST bypass before either color transform. It MUST NOT allocate another image, round-trip through Oklab, or clamp samples to establish neutrality.

With daily controls neutral and Expand disabled, nondefault stored Expand references MUST NOT defeat that bypass. Enabled Expand MUST defeat it even at default references.

Finishing MUST NOT add RGB clipping outside Expand. Existing output gamut and writer boundaries retain their own laws; the complete downstream tail does not promise a bounded final RGB cube.

The carrier is the existing RGB image boundary. Finishing MUST NOT invent a premultiplication convention or imply that discarded input alpha survives the pipeline.

### Expand law

For each fixed working channel, let `E` denote the signed extension of the sRGB encoding function. Let `B` and `W` denote encoded input references.

The enabled mapping MUST be:

\[
x_{out}=E^{-1}\left(\operatorname{clamp}\left(\frac{E(x)-B}{W-B},0,1\right)\right).
\]

The signed transfer extension MUST evaluate negative values through the corresponding positive transfer and restore their sign. It MUST NOT clip before the range mapping.

The numerical domain MUST declare a positive minimum span that remains representable on every admitted execution precision. Validation MUST reject a smaller span before render publication, including when Expand is stored inactive.

Disabled Expand MUST skip encoding, mapping, and decoding. It MUST NOT replace invalid stored parameters with default references.

## Response definition

One versioned definition MUST own all response choices below.

| Choice | Required declaration |
| --- | --- |
| Working conversion | Oklab matrix pairing, inverse, coefficient precision, and signed extension |
| Joint tone | Brightness-after-contrast composition, fixed reference center, windows, and parameter maps |
| Warmth | One normalized opponent-color direction and tone/chroma weights |
| Saturation | Selective chroma law and its positive and negative parameter maps |
| Valid domain | Finite parameter limits and Expand minimum span |
| Public scale | Neutral values, units, editor ranges, and mapping to the numerical response |

The definition MUST use Oklab lightness for joint tone and opponent coordinates for color adjustment. It MUST NOT interpret its reference center as measured image gray or scene-linear middle gray.

### Fixed response model

The model identity MUST be `common_finishing_v1`. Let the normalized Brightness, Contrast, Warmth, and Saturation values be `u_b`, `u_c`, `u_w`, and `u_s`.

For Oklab lightness `L`, the shared window and joint tone MUST be:

\[
w(L)=\begin{cases}16L^2(1-L)^2,&0<L<1,\\0,&\text{otherwise},\end{cases}
\quad H=L+0.10u_cw(L)(2L-1),\quad T=H+0.08u_bw(H).
\]

Let `A=(a,b)`, `C=|A|`, and `v=(0.35,1)/sqrt(1+0.35²)`. The color responses MUST be:

\[
A'=A+\frac{0.012u_ww(T)}{1+(C/0.15)^2}v,
\qquad A''=A'\exp\left(\frac{0.40u_s}{1+(|A'|/0.15)^2}\right).
\]

The inverse Oklab transform MUST use lightness `T` and opponent coordinates `A''`. Its inverse matrices MUST pair with the exact published forward coefficients rather than a separately rounded inverse.


Joint tone MUST preserve its scalar black and white references. Its endpoint influence MUST diminish continuously, and its extension outside the reference tone interval MUST be identity.

The tone curve MUST remain strictly increasing throughout the accepted joint parameter domain. Its interval boundaries MUST join the identity extension with continuous first derivatives.

The warmth weight MUST diminish toward the reference tone endpoints and with increasing existing chroma. Zero warmth MUST add no opponent-color displacement.

Saturation MUST preserve Oklab lightness and opponent-color direction before downstream gamut processing. Its positive response MUST restrain amplification of strong colors without reversing chroma ordering.

An achromatic input MUST remain achromatic under saturation alone. A nonzero warmth result becomes the input color for saturation, not a separate protected neutral class.

The definition MUST NOT add chroma-dependent darkening under a physical-density claim. It MUST NOT promise complete desaturation at the negative endpoint unless that endpoint actually removes every admitted input chroma.

Window weights MUST NOT be described as skin detection, highlight recovery, or gamut protection. Scalar endpoint and ordering laws MUST NOT be generalized into per-channel RGB bounds or perceptual independence.

### Qualification and publication

A public parameter contract MUST bind one complete response definition before implementation exposes its controls. Numerical validity alone MUST NOT establish photographic qualification.

Qualification MUST cover the completed color transform and output tail, not only scalar curves. It MUST evaluate each control's primary effect and interactions without changing the declared input exposure or white balance.

The released definition MUST identify its coefficients and accepted domain through existing model provenance. A changed response or public-scale mapping MUST receive an explicit model revision and replay policy.

The typed schema and shared validator MUST enforce the normalized scale and encoded Expand domain. Expand's accepted minimum span MUST be `1/1024` in fixed sRGB encoding.

## Preparation and execution

Finishing preparation MUST depend only on its accepted response definition, parameters, and required color transforms. It MUST NOT trigger material calibration or automatic frame statistics.

Finishing-only edits MUST preserve compatible material, spectral, reconstruction, and frame-setup preparation. They MUST invalidate any downstream result that contains finishing.

The base positive MUST remain upstream of all finishing edits. A cached finished result MUST NOT serve as an input baseline for later adjustments.

All consumers MUST use the [applicability contract](spec.md#scope-and-applicability), including saved-material continuations without a live film profile. Inactive controls MUST remain validated stored values, not fabricated effective support.

The implementation SHOULD evaluate the pointwise response in place with local pixel values. It MUST NOT retain full-frame Oklab planes or duplicate the prepared-source branch solely for finishing.

The resident executor MUST preserve the same operation order and constants. Resource preflight and device execution MUST follow [GPU execution](../gpu-execution/design.md), without hidden CPU image arithmetic.

Native-f32, CPU-f64, and GPU arithmetic MUST implement the same mathematical definition. Precision-specific execution MUST preserve the numerical laws within measured admission budgets rather than substitute another tone family.

Nonlinear finishing and geometry reduction MUST remain distinct operations. A reduced preview MUST use the same response and color interpretation without a new measured reference.

Transient comparison MUST neutralize only effective finishing for its render snapshot. It MUST preserve stored values, seeds, input epochs, and normal stale-result rejection.

LUT evaluation MUST apply finishing in the existing terminal RGB portion of the scan transform. Segmented and collapsed delivery MUST record its inclusion so consumers cannot apply it twice.

Material-density and reconstructed-exposure taps MUST remain upstream and retain their meanings. Final RGB observations MUST identify the applied finishing definition through existing effective-configuration and model provenance.

## Wire and compatibility

The canonical look-owned runtime group MUST be `finishing`, with an independent Finishing section scope. Exhaustive leaf names and public bounds belong to the schema and discovery metadata, not a second inventory in this document.

Publishing that group MUST update its parameter language, validator, discovery, and consumer documentation together. New syntax or changed omission behavior MUST have an explicit compatibility decision before publication.

A new required group in complete look documents MUST use the [preset version policy](../presets/spec.md#file-formats). Versioned older complete documents MUST gain neutral finishing; older sparse sections MUST preserve omission.

GUI state, recipes, parameter documents, LUT bundles, and model provenance MUST follow their own version policies. A preset version change MUST NOT imply identical version increments for unrelated carriers.

Preset schema 7 and parameter document `spektrafilm-rs-params-7` MUST require the complete finishing group. Their complete legacy versions MUST insert only neutral finishing and reject pre-cutover authored finishing fields.

GUI state version 7 MUST store finishing under `rust.runtime.finishing`. Legacy omitted finishing MUST restore neutral values without changing the GUI state's existing sparse-document semantics.

Material representation version 1 MUST accept `lab_scan_operator_v6` and `common_finishing_v1` provenance because common finishing does not change the stored material. Reading MUST retain the original producing model identity.

Unknown fields and explicit nulls MUST retain each carrier's declared validation semantics. A migration MUST NOT bypass the shared finite-value and response-domain validators.

Existing negative-conversion operator values MUST remain in their original coordinate system. No legacy control MUST be converted into a common finishing value by assumed numerical equivalence.

Restoration and history replay MUST preserve accepted finishing without applying stock defaults. Invalid candidates MUST fail before active-state mutation, cache publication, or destination replacement.

## Implementation integrity

The product MUST use public color definitions and its own response implementation. It MUST NOT import private extraction harnesses, proprietary kernels, binaries, lookup assets, or research-script side effects.

A shared helper extraction MUST preserve existing consumers' color interpretations and numerical contracts. Finishing MUST NOT silently replace the output gamut owner's XYZ/Oklab pairing with a different matrix definition.
