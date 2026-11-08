# Agfachrome RSX II 100

The built-in **Agfachrome RSX II 100** look reconstructs the Agfa Professional Films
(F-PF-E4, fourth edition, August 2004) page-8 RSX II 100 data: ISO 100/21°,
AP44/E-6, daylight 1/100 s, Status A/M densitometry.

## Source and reconstruction

- Primary source: Agfa, *Professional Films*, F-PF-E4, fourth edition, August
  2004, pp. 5 and 8. PDF SHA-256
  `1a80cea6b7afca268ace723f969a5027197c8c6118483372000b98a63b35298e`.
- Page 8 gives independent Blue/Green/Red `lg Sensitivity`, independent
  Yellow/Magenta/Cyan spectral density plus `Visual grey`, RGB colour-density
  curves against `lg exposure (Lx · s)`, MTF, RMS 10.0, and 130/50 lines/mm.
- Page 5 defines equal-energy sensitivity, density 1.0 above minimum, daylight
  1/100 s, AP44/E-6, Status A/M, and the 48 µm RMS aperture.
- The plots are vector paths in the PDF; `pdftocairo` page-8 SVG is the pinned
  geometry. The extraction uses the RSX II 100 frame endpoints and `pdftotext -bbox`
  labels, Bézier centerline sampling, and an affine axis map; the
  independently transcribed anchors are compared in source audit.
- The profile resamples the 380–780 nm grid. The RGB sensitivity curves are
  `lg Sensitivity`; the characteristic curves are Green/Red/Blue density; the
  CMY dye curves are additive effective bases. The profile stores dye columns
  in runtime **C/M/Y** order (source plots are labelled Y/M/C); an early
  Y/M/C storage order produced blue skin and is now covered by a regression
  test.
- The clear-film/base spectrum is not published. The reconstruction uses a
  neutral clear-end prior and moves residual dye minima into the inferred base;
  this is an assumption, not a measured base spectrum.
- `log_sensitivity_density_over_min=1.0` marks the base+1.0 curve reference.
- `antihalation="weak"` and the halation/scatter strengths are conservative
  class-level assumptions; Agfa does not publish an RSX II 100 halation PSF in
  this guide.

## Adapter

The existing positive-reversal adapter is refitted from the Agfa curves; no
Astia/Velvia/Provia coefficients are reused. Training uses 99 CIE 2017 TCS
under D55; validation uses 24 BabelColor Average patches under D55.

- Training RMSE/max: `[0.17158396689499464, 0.05066366035225925, 0.09491177969094454]` / `[0.5444636096914242, 0.15866408910688198, 0.3049932091979734]` stop.
- Holdout RMSE/max: `[0.1495297877675561, 0.06123205279683387, 0.05915598642233006]` / `[0.5330222561400536, 0.1460814062759326, 0.12606112633678007]` stop.
- Window-only holdout: `[0.1700017233814392, 0.07326772375401831, 0.07115111746639136]` / `[0.37273709574508296, 0.1775422619272791, 0.19550647342237362]` stop.

These are numerical spectral-reconstruction residuals, not scanner or
physical-film accuracy. The large red-channel tail maximum is a known
limitation of the publication curves and the neutral-base prior.

## Built-in look and effects

- One positive direct-scan route; `input > film > print > scan` with Crystal
  Archive Type II as the nominal print profile, which the direct positive
  readout does not use in the same way as a negative print.
- Camera exposure, metering, crop and format are preserved by the preset
  contract. The source identity is 135; 120/sheet is not separately calibrated.
- Spektra grain `rms_granularity=[11.78,11.78,11.78]`; the 48 µm grain-stage check
  realizes about `10.0/1000` (nominal model parameter, not a measured per-channel
  RMS). The source RMS 10.0 is at diffuse density 1.0 and 48 µm.
- `density_min=[0.03;3]`, `uniformity=[0.97;3]`, post-blur `0.2`,
  multiplicative USM `0`.
- Halation/scatter `0.15/0.2`; strengths `[0.05,0.015,0]`, 65 µm, 3 bounces.
  This is a restrained class-level approximation; Agfa does not publish a
  halation PSF in this guide.
- `use_cat16=false` selects CAT02; no creative diffusion, glare, DIR, chemistry,
  or scanner USM.
- Preview suppresses Spektra grain; use Scan or save an image.

## Real scans

The scan review is qualitative only. Agfachrome RSX II 100 is a discontinued
2004 material; matching E-6/AP44 scans are scarce. Any collected images are
display-encoded derivatives, not densitometry, and cannot establish MTF, RMS,
ΔE, scanner-independent colour, or physical fidelity. All Rights Reserved
images remain internal and are not published.

## Limits

- 2004 publication, not a batch-specific measurement or a current product claim.
- No independently measured base spectrum; the neutral clear-end prior is
  inferred.
- CMY curves are treated as additive effective bases; nonneutral layer coupling
  and the fourth Agfa layer are not identified.
- The 2850 K spectrogram has an explicit `lg Sensitivity` axis but no
  equal-energy correction or source spectrum in the guide.
- RMS 10 is a published condition, not a runtime grain calibration; final
  print/scan RGB sigma is not 10.
- No matched physical film/scanner validation or absolute ISO calibration.
- Rights: Agfa material remains the property of its rights holder; see
  `data/license/AGFA_RSX_100.txt`.

## Rebuild

Run from the product worktree, with the PDF and Status A source path supplied:

```sh
python3 scripts/extract_agfa100.py \
  --pdf /data/research/spektrafilm/experiments/next-color-film/sources/agfa-professional-films-2004.pdf \
  --status-source /data/research/spektrafilm/experiments/astia-profile-preset/lead/densiometry.py.txt \
  --output data/profile_sources/agfachrome_rsx_ii_100.json
python3 scripts/prepare_agfa100.py \
  --output data/profiles/agfachrome_rsx_ii_100.json \
  --report /data/research/spektrafilm/experiments/agfa-rsx100/preparation.json
```

## Source

- https://analoguephotolab.com/wp-content/uploads/2019/04/AGFA-Films.pdf
- Jan Lohse, spectral_film_lut Status A transcription (MIT): see
  `data/license/JAN_LOHSE_MIT.txt`.
