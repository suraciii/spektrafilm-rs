# Ilford HP5 Plus 400

The built-in **Ilford HP5 Plus 400** look reconstructs the November 2018
ILFORD HP5 PLUS technical sheet: the page-5 characteristic curve for
**ILFOTEC HC 1+31, 6.5 minutes at 20 C/68 F, intermittent agitation**, and the
page-1 wedge spectrogram.

## Source identity and reconstruction

- Official source: ILFORD/Harman, *HP5 PLUS Technical Information*, November
  2018, 6 pages. Japanese official PDF SHA-256
  `b2006ecb9f96b1645726a90a7f15e798bdb7948b4ae1d0e6e9fa9719c93925f2`.
- English-language mirror used only to read labels: SHA-256
  `f06f383d714bc9d13033f9e30a6aedf29d9b9f4d25c4a353d69e6526cb2f037f`; it is
  byte-identical to the older English sheet.
- The characteristic plot is an embedded raster image, 488 x 344 px at about
  170 ppi. It is not a vector centerline. The 2018 English caption says the
  curve is representative of roll and sheet film; the 2018 Japanese caption
  says 35mm and sheet, and the 2010 English sheet says 35mm, rollfilm and
  sheet. The 135 identity is taken from the shared 35mm coverage, not from one
  caption alone.
- The spectral plot is an embedded raster image, 515 x 264 px at about
  149 ppi, measured as a wedge spectrogram under 2850 K tungsten light.
- The characteristic axis is explicit (`Relative log exposure` / `Density`).
  The spectral ordinate is printed only as `Sensitivity` with 0.5/1.0 ticks.
  **This profile treats it as linear normalized sensitivity with zero at the
  frame bottom.** Both linear and log readings can be anchored to the printed
  0.5/1.0 ticks; the sheet does not decide. This is an explicit interpretation
  accepted for the approximation, not a statement printed by Harman. It must
  not be presented as a manufacturer-confirmed axis (linear or log).
- The p1 ordinate is a 2850 K tungsten wedge response. No source spectrum or
  equal-energy correction is supplied, so `relative_sensitivity` is not
  equal-energy film sensitivity.
- The first plotted density is `0.1779041772307175`; it is used as a
  base-density proxy, not as a measured D-min. The stored reference is
  `2.5983131443114766` in the source's relative log-exposure coordinate; the
  profile stores exposure relative to that reference and density above the
  base proxy. It is not an absolute lux-seconds origin.
- The curve is resampled to 1066 runtime samples; the resampling-vs-source max
  density difference is `1.0698724707769447e-05`. This is reconstruction
  consistency, not a print-digitization error.
- The profile is one aggregate `channel_model=bw` silver record with
  `development_time=[6.5]`. It is not a measured emulsion-layer
  decomposition. `log_sensitivity_density_over_min=1.0` marks the base+1.0
  curve reference; it is not a separately measured threshold.
- The 81-point channel is resampled to 380-780 nm; the 380-355 nm and
  660-780 nm tails are endpoint-slope extrapolations clamped at `1e-12`, not
  measured cutoffs.

## Adapter

The existing engine-matched adapter is refitted from the HP5 curve; no T-MAX
or Acros coefficients are reused. The HP5 fit is one-channel and the fitted
surface is replicated across RGB; it is not three independently fitted
channels. Training uses 99 CIE 2017 TCS under D55; validation uses 24
BabelColor Average patches under D55.

- Training RMSE/max: `0.028734663707670246/0.08036248591736748` stop.
- Holdout RMSE/max: `0.01954518644814914/0.038910535219804976` stop.
- Window-only holdout RMSE/max: `0.034448034911689525/0.10526623591669931` stop.

These are numerical spectral-reconstruction residuals, not scanner or
physical-film accuracy.

## Built-in look and effects

- One `input > film > print > scan` route; Polymax Fine-Art F; T enlarger;
  `direct_scan`.
- Camera exposure, metering, crop and film format are preserved by the preset
  contract. The source identity is 135; the look is not separately calibrated
  for 120/sheet.
- Spektra grain is enabled with `rms_granularity=[18,18,18]`. This is a
  **model calibration**: the 48 micrometre grain-stage check realizes about
  `15.91/1000`, matching the HP5 motion-picture sheet's diffuse RMS 16 in
  D-96. The still HP5 sheet publishes no RMS/MTF; the motion-picture value is
  context, not a still-film measurement. A 16.0 parameter is estimated to
  realize about `14.1-14.3/1000`; it was not separately run. Final print/scan
  RGB sigma is not 16.
- `density_min=[0.03;3]`, `uniformity=[0.97;3]`, post-blur `0.2`,
  multiplicative USM `0`.
- Halation/scatter `0.15`; strengths `[0.05,0.015,0]`; first sigma 65 micrometres;
  three bounces. B&W reads component 0 only; G/B values are inert. Harman
  publishes no HP5 halation PSF; these are restrained class-level
  approximations.
- `use_cat16=false` selects CAT02, matching the CPU calibration path.
- No creative diffusion, print glare, DIR couplers, chemistry change or
  scanner USM is enabled.
- Preview suppresses Spektra grain; use Scan or save an image.

## Real scans

The scan review is qualitative only. Ten or more HP5 Plus 135 scans from
traceable sources are collected under `experiments/hp5-plus-profile-preset/scans/`;
they are display-encoded derivatives, not densitometry. They can show tonal
shape, grain appearance and process/scan variability, but cannot establish
MTF, RMS, scanner-independent color or physical fidelity.

## Limits

- The spectral ordinate is an explicit linear, zero-based assumption; no primary
  Harman statement confirms it. Both linear and log readings fit the printed
  ticks. The 2850 K tungsten source is not equal-energy corrected.
- Both selected curves are raster plots, not vectors; extraction uncertainty
  remains.
- The still sheet publishes no RMS/MTF/resolving power. The grain value is an
  approximation informed by the HP5 motion-picture sheet.
- No measured base spectrum, silver halide layer decomposition, halation PSF,
  scanner response or absolute ISO calibration is supplied.
- `densitometer=diffuse_visual` and `antihalation=weak` are conservative
  metadata assumptions; the sheet does not measure either.
- The spectral tails outside 355-657 nm are endpoint-slope assumptions, not
  measured cutoffs. The 81-point profile channel therefore includes
  extrapolated 380-355 nm and 660-780 nm regions.
- Real scans are not a matched physical validation.
- Rights: Ilford/Harman technical material remains the property of its rights
  holder; see `data/license/HP5_PLUS.txt`.

## Rebuild

Run from the product worktree, with the PDF copied or addressed by its
research-root path:

```sh
python3 scripts/extract_hp5.py \
  --pdf /data/research/spektrafilm/experiments/next-film-after-tmax/hp5-ilford-jp.pdf \
  --output data/profile_sources/ilford_hp5_plus.json
python3 scripts/prepare_hp5.py \
  --source data/profile_sources/ilford_hp5_plus.json \
  --output data/profiles/ilford_hp5_plus.json \
  --report /data/research/spektrafilm/experiments/hp5-plus-profile-preset/preparation.json
```

## Source

- https://ilford.co.jp/photo/wp-content/uploads/2023/02/TDS_HP5PLUS.pdf
- https://radlabstudio.com/wp-content/uploads/2024/06/HP5-technical-data-sheet.pdf
- https://bioskoplab.wordpress.com/wp-content/uploads/2011/10/hp5mpic.pdf (motion-picture RMS 16 context)
