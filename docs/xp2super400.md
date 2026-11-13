# Ilford XP2 Super 400

The built-in **Ilford XP2 Super 400** look reconstructs the November 2018
ILFORD XP2 SUPER technical sheet:

- page 2: 2856 K tungsten wedge spectrogram
- page 3: characteristic curve for standard C-41, representative of 35mm and
  roll film

XP2 SUPER is a **chromogenic B&W film**. Its image is made of dyes formed
during C-41 development, not silver. The negative is pink/red-brown; the
datasheet recommends C-41 and says Digital ICE can be used. The profile stores
one aggregate `channel_model=bw` record because the published data provide one
density curve and one relative spectral curve, not three independently measured
emulsion layers.

## Source identity and reconstruction

- Source: ILFORD/Harman, *XP2 SUPER Technical Information*, November 2018,
  5 pages. PDF SHA-256
  `695a2d3d54f11cb8b24f6d040a9723a16819001a38715f2bd6f71f2d05cdc549`.
- Page-2 spectral plot: embedded RGB raster, 515 x 264 px at about 149 ppi.
- Page-3 characteristic plot: embedded RGB raster, 292 x 210 px at about
  96 ppi.
- The spectral ordinate is printed only as `Sensitivity` with 0.5/1.0 ticks.
  This profile treats it as **linear normalized sensitivity with zero at the
  frame bottom**. That is an explicit interpretation, not a Harman statement.
- The page-2 curve is a 2856 K tungsten wedge response. No source spectrum or
  equal-energy correction is supplied, so `relative_sensitivity` is not
  equal-energy film sensitivity.
- The characteristic axis is explicit (`Relative log exposure` / `Density`).
  The first plotted density `0.2685185185185164` is a base-density proxy, not a
  measured D-min.
- The profile stores exposure relative to the base+1.0 reference
  `3.0421916652074086` in the page-3 relative-log-exposure coordinate. This is
  not an absolute lux-seconds origin.
- The curve is resampled to runtime samples; the resampling-vs-source max
  density difference is `1.4233313941458547e-05`. That is reconstruction
  consistency, not a print-digitization error.
- `development_time=[0.0]` is a placeholder: the datasheet does not publish a
  C-41 time. It is not a time calibration.
- The 81-point spectral channel is resampled to 380-780 nm; the 380-355 nm and
  660-780 nm tails are endpoint-slope extrapolations clamped at `1e-12`, not
  measured cutoffs.

## Adapter

The existing engine-matched adapter is refitted from the XP2 curve; no HP5,
T-MAX or Acros coefficients are reused. The XP2 fit is one-channel and the
fitted surface is replicated across RGB; it is not three independently fitted
channels. Training uses 99 CIE 2017 TCS under D55; validation uses 24
BabelColor Average patches under D55.

- Training RMSE/max: `0.02874302110279597/0.0805166100411371` stop.
- Holdout RMSE/max: `0.019553983100897374/0.03889657202057418` stop.
- Window-only holdout RMSE/max: `0.03443306325307617/0.1052454549444259` stop.

These are numerical spectral-reconstruction residuals, not scanner or
physical-film accuracy.

## Built-in look and effects

- One `input > film > print > scan` route; Polymax Fine-Art F; T enlarger;
  `direct_scan`.
- Camera exposure, metering, crop and film format are preserved by the preset
  contract. The source identity is 135/roll representative; the look is not
  separately calibrated for 120/sheet.
- **Dye grain engine**, active. `dye_strength=1.0`, `dye_scale=1.0`; no RMS
  target is authored (`rms_granularity=[0,0,0]` is inert for Dye). Harman
  publishes no RMS or MTF for XP2, so this is a class-level approximation, not
  a measured grain calibration. Fotufilm also has an explicit
  `material=dye_negative` mode for chromogenic monochrome, but it is an
  uncalibrated appearance population model and was not selected; the Dye route
  is the simpler single-record approximation used here.
- `density_min=[0.03;3]`, `uniformity=[0.97;3]`, post-blur `0.2`,
  `blur_dye_clouds_um=2.0`, multiplicative USM `0`.
- Halation/scatter `0.10`; strengths `[0.03,0.01,0]`; first sigma 65
  micrometres; three bounces. B&W reads component 0 only; G/B values are inert.
  Harman publishes no XP2 halation PSF; these are restrained class-level
  approximations.
- `use_cat16=false` selects CAT02, matching the CPU calibration path.
- No creative diffusion, print glare, DIR couplers, chemistry change or
  scanner USM is enabled.
- Preview suppresses active grain; use Scan or save an image.

## Real scans

The scan review is qualitative only. Four XP2 Super 400 scans are in the
research archive under `experiments/film-scans/Ilford/XP2 Super 400/`; they
come from traceable Commons sources and are display-encoded derivatives, not
densitometry. They can show tonal shape, grain appearance and scan variability,
but cannot establish MTF, RMS, scanner-independent color or physical fidelity.

## Limits

- No RMS granularity or MTF is published; the Dye grain preset is an
  approximation and is not a measured RMS target.
- XP2 is a dye image, not a silver image. The profile uses one aggregate BW
  record; it does not measure dye spectra or layer decomposition.
- The spectral ordinate is an explicit linear, zero-based assumption; no
  primary Harman statement confirms it.
- The 2856 K tungsten source is not equal-energy corrected.
- Both selected plots are raster images, not vectors; extraction uncertainty
  remains.
- No base spectrum, halation PSF, scanner response or absolute ISO calibration
  is supplied.
- `densitometer=diffuse_visual` and `antihalation=weak` are conservative
  metadata assumptions; the sheet does not measure either.
- Real scans are not a matched physical validation.
- Rights: Ilford/Harman technical material remains the property of its rights
  holder; see `data/license/XP2_SUPER_400.txt`.

## Rebuild

Run from the product worktree, with the PDF copied or addressed by its
research-root path:

```sh
python3 scripts/extract_xp2.py \
  --pdf /data/research/spektrafilm/experiments/next-film-profile/sources/xp2.pdf \
  --output data/profile_sources/ilford_xp2_super_400.json
python3 scripts/prepare_xp2.py \
  --source data/profile_sources/ilford_xp2_super_400.json \
  --output data/profiles/ilford_xp2_super_400.json \
  --report /data/research/spektrafilm/experiments/xp2-super-400/preparation.json
```

## Source

- https://125px.com/docs/film/ilford/XP2-Super_201811.pdf
- https://www.ilfordphoto.com/xp2-super-35mm
