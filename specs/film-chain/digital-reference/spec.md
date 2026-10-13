# Digital Reference scan

## Scope

Digital Reference must be an opt-in output treatment within the existing Scan capability. This contract covers a digital photograph processed through simulated negative film on `input > film > scan`, with `scanner.scan_output=negative_to_positive`.

Reference Exposure is the supported placement. Graded Print and Auto Levels are outside this implementation.

Real negative import, `convert-film`, Telecine, photographic-paper scanning, and reversal-film normalization are outside this addition. Their existing behavior must remain available.

Terms follow the [Film simulation language](../../../CONTEXT.md). Existing scanner behavior follows the [Film chain contract](../spec.md).

## Reference conversion

Reference conversion must read the developed negative through a common output model. It must not use the selected film's characteristic curves to reconstruct its original exposure.

For color negatives, the output model must define its illumination, spectral receiver bands, dye-separation basis, color mapping, and positive tone response. These definitions must be shared across selected films. The selected film still supplies its own developed densities, base contribution, and dye spectra.

A fixed reference normalization may use the selected film's clear base and one declared reference exposure. It must not fit a separate color transform or tone curve to each selected film. It must not force every density on a neutral exposure wedge to the same common wedge.

Differences in film-record slopes, toe and shoulder behavior, and spectral dye coupling must remain observable when they are present in the chosen model. Reference conversion must not promise that every pair of films produces different pixels.

The output must be linear RGB in the requested output color space before the shared output stages. A display-space change must not rebuild the receiver's physical response or alter its reference convention.

For monochrome negatives, conversion must produce a neutral image through the declared monochrome reference response. It must not introduce color through the color-negative separation model.

Digital Reference must be labeled a modeled reference. It must not claim a measured scanner response, recovered scene color, or an unprocessed view of the film.

## Reference Exposure

Reference Exposure must use a fixed positive tone response and fixed exposure placement. The response must declare its middle-gray anchor, film-base black convention, highlight behavior, and endpoint continuation.

With identical developed film densities, selected film, prepared reference configuration, output color space, and manual Scan controls, it must produce identical conversion results regardless of image histograms, neighboring pixels, crop, or preview size. This comparison excludes explicitly enabled spatial effects and the shared output resampling stages.

A fixed response must retain continuous, finite behavior at the film base, in the mid-range, and in the highlight shoulder. Out-of-reference inputs must use a declared finite continuation. It must not infer black or white points from image percentiles or clip input measurements before evaluating the response.

Reference Exposure must require automatic scan setup to be off. An explicitly supplied combination with automatic setup must fail validation before output publication. The GUI may clear the incompatible automatic setup when the user selects Reference Exposure.

`scanner.reference_exposure_ev` applies a common output gain of `2^EV`. `scanner.reference_balance_ev` applies independent red, green, and blue gains of `2^EV` in linear sRGB before the output-space transform. Both default to zero. They adjust the reference output without changing its tone response or calibration. Saved state and effective parameter reports must retain these controls.

## Integration and control ownership

Output polarity must remain `negative_to_positive`. Digital Reference must not add a workflow route, a photographic print stage, or a new scan output polarity.

A reference conversion must use its matching reference receiver and response. An incompatible receiver or exposure-reconstruction method must fail validation. The GUI must not offer independent selections that produce such a combination.

`scanner.positive_interpretation=digital_reference` selects reference conversion. It requires `scanner.receiver_profile=digital_reference`. The GUI labels the conversion `Digital Reference (modeled)` and displays its fixed `Reference Exposure` placement. Choosing it pairs the receiver and conversion, disables automatic setup, restores identity point windows, and resets reconstruction controls to their existing defaults. Explicit incompatible combinations must fail validation.

Reconstruction brightness, balance, contrast, and maximum-density controls do not parameterize this model. Nondefault reconstruction controls must fail validation with reference conversion. Reference manual controls must remain neutral with other conversion methods.

Selection must not change film exposure, development, grain, halation, camera exposure compensation, or the developed negative. Lens blur remains a readout operation. Scanner finishing remains an optional operation after conversion under the [scanner finishing contract](../spec.md#scanner-receiver-and-interpretation).

Neutral finishing must expose the reference conversion before shared output processing. An explicitly selected Lab Scan finish may style that result, but it must not replace its receiver or conversion. Reports must distinguish the reference conversion from the enabled finish.

Existing film-scan black and white points use an exposure-reconstruction axis. Until reference-axis semantics are specified, Digital Reference must accept only their identity values. A nonidentity point window must fail validation rather than being ignored or reinterpreted.

The conversion must integrate with Preview, Scan, Save, and Export. Complete state and recipes must retain the accepted treatment, reference placement, manual controls, and model declaration. Render diagnostics must retain the prepared spectral source identities. Static state must not claim prepared asset identities. Preset content remains governed by the [preset contract](../../presets/spec.md#parameter-set).

GUI, CLI dry run, parameter-file resolution, and rendering must apply the same capability rules. Invalid imports must retain accepted GUI parameters and retained images. Existing settings and defaults must keep their previous meaning.

## Acceptance

1. With the addition disabled, existing direct-negative, density, separable, coupled, reversal-film, and paper scenarios must retain their outputs within their established numerical budgets.
2. Selecting Digital Reference must leave developed film-density outputs unchanged for a fixed input, film, development, spatial-effect configuration, and seed.
3. A fixed reference exposure wedge and a set of chromatic patches must demonstrate the common receiver and response. Film variants with deliberately different record slopes or dye spectra must retain the predicted differences. An identity-like reconstruction of the original exposure must not satisfy this check.
4. Two images with identical test patches but different surrounding histograms must produce identical Reference Exposure patch values with spatial effects disabled and identical manual controls.
5. CPU f64 evaluation must cover the film base, mid-gray, toe, shoulder, and out-of-reference inputs. Output must remain finite and continuous under the declared endpoint policy. Monochrome patches must remain neutral.
6. Preview and full-resolution rendering must agree on conversion values for identical density samples before resampling and output encoding, within the declared precision budget. Save must retain the accepted rendered image. Export must reproduce its configured conversion through the real entry point.
7. State and recipe round trips must preserve the selected treatment and model declaration. Render reports must identify the actual prepared source assets. Incompatible setup, receiver, conversion, point windows, profiles, and routes must fail before replacing or publishing output.
8. A photographic comparison must include a neutral wedge, mixed-color subjects, a low-contrast frame, and a high-dynamic-range frame. Claims must identify the source model, numerical comparison scope, and visual limits. A visual resemblance must not substitute for numerical checks.

## Delivery status

Digital Reference is available through the paired `digital_reference` receiver and conversion selectors. The implemented Reference Exposure model and its source conventions are defined in the [design](design.md#numerical-reference-model). Graded Print and Auto Levels remain outside this addition.
