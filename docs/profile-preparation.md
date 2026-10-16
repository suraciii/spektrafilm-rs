# Preparing the Velvia 50 profile

`fujifilm_velvia_50` is a source fit for Fujichrome Velvia 50 Professional
[RVP50]. It supports the existing `input > film > scan` positive-film route.
The displayed name includes `source fit` because the base spectrum and spectral
reconstruction are estimates. No matched film captures establish stock fidelity.

The [film material authoring contract](../specs/film-chain/spec.md#film-material-profiles-and-stock-presets)
defines the material, input-adaptation, effect, and preset boundaries.

## Rebuild

Use Python 3.13 with NumPy 2.5.3, SciPy 1.18.1, and colour-science 0.4.7.
Install these into an environment outside the repository. The generator reads
local data only. It does not execute downloaded Python or read another stock's
profile.

```sh
python -m pip install numpy==2.5.3 scipy==1.18.1 colour-science==0.4.7
python scripts/prepare_velvia50.py \
  --output data/profiles/fujifilm_velvia_50.json \
  --report /tmp/velvia50-preparation.json
```

The report contains source, generator, observer, illuminant, spectral-LUT and
reflectance hashes, package versions, fitted predictions, held-out errors,
base estimates, and adaptation errors. Identical inputs in the same numerical
environment produce the same JSON bytes. Different numerical libraries may
change the final digits; compare the report and physical residuals as well.

## Sources and conditions

The primary reference is Fujifilm's
[FUJICHROME Velvia 50 Professional [RVP50], AF3-0221E2](https://asset.fujifilm.com/master/emea/files/2020-10/a71dda63e2662f012b3b74110794918a/films_velvia-50_datasheet_01.pdf),
revision `KAMIQ-07.3-FP`. Page 8 contains the three input plots:

- Figure 19: daylight characteristic curves, E-6/CR-56, Status A density,
  `log10 H` in lux-seconds.
- Figure 20: spectral sensitivity, E-6/CR-56, Status A density 1.0 above D-min.
  Sensitivity is reciprocal exposure in J/cm², on a logarithmic axis.
- Figure 22: separate-exposure CMY spectral diffuse-density curves, E-6/CR-56.
  The plotted dye peaks are near 1.0. A separate clear-film spectrum is absent.

The older Professional Film Data Guide's RVP page is not the generation
identifier for this profile. The 2007 RVP50 bulletin describes a replacement
production process with nearly identical performance, not an identical emulsion.

[Source coordinates](../data/profile_sources/fujifilm_velvia_50.json) retain
Jan Lohse's digitization at commit
`6307a98f3cf4f2d7b6d09bab82fbf6f64356ccfe` and its source hash. Primary labels,
channel order, curve shapes and clear-end levels were visually cross-checked.
The coordinates were not independently remeasured. The same pinned repository
supplies Status A response tables. No copied Ektachrome 100D grain curve or
`color_masking=0.5` adjustment enters this profile.

## Reconstruction

The generator samples the native 380–780 nm grid at 5 nm. It uses PCHIP within
the digitized spectral intervals. Endpoint slopes extend the tails beyond
those intervals. Density tails are clamped to zero; log sensitivity has a
numerical floor of -12. These tails are extrapolations, not measured values.

Status A weights follow the pinned digitization repository's Colour Science
sequence: linearly extrapolate log weights to the working range, exponentiate,
interpolate the linear weights to the 5 nm grid, then normalize each channel
to unit sum. The tails are not truncated at the tabulated endpoints.

The characteristic curves specify RGB densitometer readings, not CMY dye
amounts. The fit integrates spectral transmittance through normalized Status A
responses and minimizes the difference from those readings. Each dye amount
uses three decreasing normal-CDF components with positive amplitudes and
widths. These are numerical components, not measured emulsion sublayers.
Every fifth characteristic point, starting with index 2, is excluded from the
CDF fit and reported separately. The clear-end base estimate uses the high
exposure points, so the holdout does not independently validate the base model.

The base is a nonnegative smooth spectrum: a constant equal to 75% of the
smallest clear-end RGB density, plus three Gaussian basis functions centered
at 440, 545 and 660 nm, each with a 65 nm width. Nonnegative coefficients match
the median clear-end RGB densities at `log10 H >= 0.45`. Three densitometer
readings do not uniquely determine a spectrum. This inferred base and the
assumption that the dye plots can act as additive basis spectra remain physical
uncertainties.

The exposure origin is the fitted green Status A response at D-min + 1.0.
This selects one material coordinate for the three-channel model; it does not
claim that all channels reach this density at the same exposure. The report
retains the corresponding `log10 H` in lux-seconds. Adding that reference to
the relative exposure grid and model centers recovers the source coordinates.
Print profiles, viewing spectra, and scanner brightness do not enter this choice.

Each sensitivity channel is normalized to a D55 RGB midgray raw response of 1
through the bundled Hanatos LUT. This is the input adapter's coordinate mapping.
D55 is the implementation's daylight convention; the primary plot says daylight
without specifying a CIE illuminant. Neither this mapping nor the stock's name
establishes an absolute ISO calibration.

The Hanatos erf4 window and degree-4 bounded correction surface are fitted
independently for this sensitivity. Training uses the 99 CIE 2017 TCS
reflectances under D55; validation uses the 24 BabelColor Average patches.
The fit uses the runtime's Mitchell interpolation, white normalization and
2-stop bounded polynomial form. The window fit reaches parameter bounds;
its cutoffs are numerical reconstruction controls, not physical film filters.
Its residuals compare film-channel exposure with direct spectral integration.
The report includes surface-enabled and default window-only errors. These
parameters are input-adapter estimates stored in the existing profile format;
they are not measured emulsion properties. The runtime keeps its normal default
of window on and surface off. Final scan color is not a fitting objective.

## Use and limits

```sh
spektrafilm process input.tif -o velvia50.png \
  --film fujifilm_velvia_50 --scan-film --backend cpu --data-dir data
```

Set the input color space and transfer decoding to match the input file.
Use `direct_scan` for this positive profile. `positive_scan` is the separate
negative-film interpretation and is not eligible.

For material acceptance, disable auto exposure, scanner white/black corrections,
grain, DIR couplers, halation, glare, and unsharp masking. Compare characteristic
density and spectral transmittance before print or scan. In the current slide
runtime, scanner white/black correction can feed an exposure adjustment into
filming; leave it disabled when testing this boundary.

Then measure stock-specific effects. A halation comparison needs documented
exposure and highlight geometry, channel response, and spatial scale in film
micrometers. Grain and sharpness comparisons need the source density, aperture,
processing, and imaging-system response. Keep the scan or measurement setup fixed
and characterized so its blur, tone curve, or flare is not assigned to the film.

This profile has no measured stock-specific grain, coupler, or halation fit.
Normal rendering uses existing class defaults; `antihalation=strong` is an
explicit routing assumption, not a measured point-spread function. The source
bulletin's RMS value and MTF graph alone do not determine those effect parameters.
No new built-in Velvia 50 preset claims measured stock fidelity.

The source fit, holdout, spectral-reconstruction check, backend comparison and
real GUI/CLI smoke establish different facts. None alone establishes fidelity
to an actual Velvia exposure and scan. See the source and fit
[attribution notice](../data/license/VELVIA_50.txt) before redistributing assets.
