# Preparing the Velvia 50 profile

`fujifilm_velvia_50` is a source fit for Fujichrome Velvia 50 Professional
[RVP50]. It supports the existing `input > film > scan` positive-film route.
The displayed name includes `source fit` because the base spectrum and spectral
reconstruction are estimates. No matched film captures establish stock fidelity.

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

A D50 scan with luminance 0.184 defines the curve's exposure origin. Each
sensitivity channel is normalized to a D55 midgray raw response of 1 through
the bundled Hanatos LUT. D55 is the implementation's daylight convention; the
primary plot says daylight without specifying a CIE illuminant. Neither this
normalization nor the stock's name establishes an absolute ISO calibration.

The Hanatos erf4 window and degree-4 bounded correction surface are fitted
independently for this sensitivity. Training uses the 99 CIE 2017 TCS
reflectances under D55; validation uses the 24 BabelColor Average patches.
The fit uses the runtime's Mitchell interpolation, white normalization and
2-stop bounded polynomial form. The window fit reaches parameter bounds;
its cutoffs are numerical reconstruction controls, not physical film filters.
The report includes surface-enabled and default window-only errors. The
runtime keeps its normal default of window on and surface off. The review
color-only configuration explicitly enables the surface.

## Use and limits

```sh
spektrafilm process input.tif -o velvia50.png \
  --film fujifilm_velvia_50 --scan-film --backend cpu --data-dir data
```

Set the input color space and transfer decoding to match the input file.
Use `direct_scan` for this positive profile. `positive_scan` is the separate
negative-film interpretation and is not eligible.

For a color-chain comparison, disable auto exposure, grain, DIR couplers,
halation, glare and unsharp masking. Compare neutral exposure steps, color
patches and photographs at identical settings. This profile has no measured
stock-specific grain, coupler or halation calibration. Normal rendering uses
the existing class defaults for those effects; it does not turn them into
Velvia-specific measurements. `antihalation=strong` is an explicit routing
assumption for the class, not a measured point-spread function.

The source fit, holdout, spectral-reconstruction check, backend comparison and
real GUI/CLI smoke establish different facts. None alone establishes fidelity
to an actual Velvia exposure and scan. See the source and fit
[attribution notice](../data/license/VELVIA_50.txt) before redistributing assets.
