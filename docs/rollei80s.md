# Rollei Retro 80S 35 mm / Neofin Blau

The built-in **Rollei Retro 80S 35mm (Neofin Blau)** look uses a
**third-party 10-zone densitometry set**, not a manufacturer characteristic
curve. It is deliberately scoped to:

- 35 mm Rollei Retro 80S, exposed at **EI 80/20**;
- **Tetenal Neofin Blau**, one flask to 500 ml, **6 min at 20 C**;
- a 10-zone APHOG measurement made with a **Wallner MC 505** enlarging meter,
  relative to base+fog;
- the 2021 Rollei Retro 80S page-3 vector spectral plot.

It is not a 120 profile, not an ID-11/D-76 profile, and not a
manufacturer-published curve.

## Source identity

- Third-party measurement: APHOG forum thread “Rollei Retro 80s”, posts
  #40–41, 2011-06-05:
  <https://forum.aphog.com/index.php?thread/13483-rollei-retro-80s/&pageNo=3>
  Archived page SHA-256:
  `8da29c06a10b430d4c70534550fb96a6ac242b3c8f90259e46f9e07fa68137c9`.
- The measurement is 35 mm, Leica M2, gray card on a photo wall, EI 80/20.
  The author describes the instrument as a Wallner MC 505 enlarging meter and
  the values as relative logD on the enlarger baseboard.
- Product/spectral sheet: Rollei Retro 80S data sheet, 2021-01-15, 6 pp.;
  page 3 spectral sensitivity:
  <https://www.rolleianalog.com/wp-content/uploads/2021/02/RETRO80S_Datenblatt_EN_R012101.pdf>
  PDF SHA-256:
  `f37754ce4548ceab6e6902143118cea712d5bf4f1dee32488bfa37dfb02e0f45`.
- Product identity: 35 mm, ISO 80/20, superpanchromatic to 750 nm, RMS 14,
  180 lp/mm, AHU, clear PET. The profile does not claim that the 2010–2011
  measured batch is identical to the current 2026 batch.

## Characteristic curve

The source values are logD above base+fog, Zones I–X, one stop per zone:

| Zone | I | II | III | IV | V | VI | VII | VIII | IX | X |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| logD | 0.05 | 0.15 | 0.28 | 0.36 | 0.66 | 0.90 | 1.13 | 1.24 | 1.38 | 1.45 |

- The first measured value `0.05` is used as the **base-density proxy**,
  matching the established B&W pattern. It is not a measured D-min.
- The profile stores density **above** that proxy, from `0.0` to `1.4`.
- The curve reference is where density is `base + 1.0 = 1.05`; the source
  coordinate is `1.6797995136750812`. This is not an absolute lux-seconds
  origin.
- The 10 points are interpolated with PCHIP in a 1034-sample runtime grid.
  The runtime-vs-source maximum density error is
  `7.023955089446865e-06`: reconstruction consistency, not digitization
  accuracy.
- Zone IV (`0.36`) is lower than a smooth S-curve would suggest. It is kept
  as measured, not silently corrected.
- There is no measured toe below Zone I or shoulder above Zone X. The curve
  clamps at the measured endpoints; no extra latitude is inferred.

The mid-tone slope from Zone V to Zone VIII is about `0.644`, close to the
`0.65` reference in Rollei's development table. This is a descriptive slope,
not a standard ISO average-gradient measurement.

## Spectral curve

The Rollei page-3 chart is a vector plot with 15 filled circles from 350 to
800 nm. The ordinate is printed as `0.0–3.0` and is read as **relative log
sensitivity with an arbitrary zero**; it is not an absolute sensitivity and
is not equal-energy corrected.

The 15 points are extracted from the PDF vector geometry by
`scripts/extract_rollei80s.py`. The profile resamples them to the 380–780 nm
runtime grid. The spectral adaptation is fitted with the shared D55
calibrator:

- training RMSE/max: `0.3499354454871084 / 1.8592126960588002` stop;
- holdout RMSE/max: `0.26020564549231734 / 0.958740882786194` stop;
- window-only holdout RMSE/max: `0.21429961167223635 / 0.6173843536616308`
  stop.

These residuals are **high** compared with the HP5/Acros profiles. They are
reported as-is: they reflect the red-extended superpanchromatic shape and the
coarse 15-point source, not a measured scanner match. The profile is a
single-channel B&W silver record; the residual is not a color-print accuracy
claim.

## Built-in look and effects

- One `input > film > print > scan` route; Polymax Fine-Art F; T enlarger;
  `direct_scan`.
- Camera exposure, metering, crop and film format are preserved by the preset
  contract.
- Spektra grain with `rms_granularity=[14,14,14]`. The 14 comes from the
  Rollei data sheet (RMS ×1000 = 14); the effect strength is a class-level
  model setting, not a measured grain match.
- `density_min=[0.03;3]`, `uniformity=[0.97;3]`, post-blur `0.2`,
  multiplicative USM `0`.
- Halation/scatter `0.10`; strengths `[0.03,0.01,0]`; first sigma 65
  micrometres; three bounces. Rollei publishes an AHU layer but no PSF; these
  are restrained class-level approximations.
- `use_cat16=false` selects CAT02, matching the CPU calibration path.
- No creative diffusion, print glare, DIR couplers, chemistry change or
  scanner USM is enabled.
- Preview suppresses Spektra grain; use Scan or save an image.

## Limits

- **Third-party, not manufacturer:** the curve is a forum measurement, not a
  Rollei datasheet. The instrument is a relative enlarging meter, not a
  calibrated diffuse densitometer.
- **35 mm only:** no 120-specific curve. The 120 product uses the same
  emulsion family but was not measured here; the profile must not be
  presented as a 120 measurement.
- **Neofin Blau only:** no ID-11/D-76 curve. Neofin Blau is a low-contrast,
  high-acutance developer; its curve is not the curve of a standard ID-11
  negative.
- **10 zones:** no toe below Zone I, no shoulder above Zone X; the endpoints
  are clamped model assumptions.
- **Base proxy:** `0.05` is the first measured zone, not a measured D-min.
- **Batch:** data are from 2010–2011; current-batch equivalence is unverified.
- **Absolute ISO:** no absolute ISO calibration. The spectral scale is
  relative and the profile's exposure reference is model-relative.
- **Spectral adaptation residual is high:** see the numbers above; this is a
  real limitation for strongly colored subjects.
- **No physical validation:** no matched film/scanner/densitometer chain, no
  measured base spectrum, RMS/MTF instrument, halation PSF, or scanner
  response.

**Verification: BLOCKED** for physical film/scanner fidelity, absolute ISO,
120 behavior, ID-11 behavior, current-batch equivalence, and a measured
grain/halation match.

## Rebuild

From the product worktree:

```sh
python3 scripts/extract_rollei80s.py \
  --pdf /data/research/spektrafilm/experiments/next-film-profile/sources/rollei-retro80s.pdf \
  --source data/profile_sources/rollei_retro_80s_35mm_neofin_blau.json

python3 scripts/prepare_rollei80s.py \
  --source data/profile_sources/rollei_retro_80s_35mm_neofin_blau.json \
  --output data/profiles/rollei_retro_80s_35mm_neofin_blau.json \
  --report /data/research/spektrafilm/experiments/rollei-retro80s-neofin/preparation.json
```

The Python checks are in `scripts/test_prepare_rollei80s.py`.

## Sources

- <https://forum.aphog.com/index.php?thread/13483-rollei-retro-80s/&pageNo=3>
- <https://www.rolleianalog.com/wp-content/uploads/2021/02/RETRO80S_Datenblatt_EN_R012101.pdf>
- <https://www.freestylephoto.com/static/pdf/product_pdf/rollei/Rollei80S.pdf>
  (Agfa Aviphot Pan 80 parent sheet; context only, not the curve source)
- <https://www.fotoimpex.com/chemistry/tetenal-neofin-blau-6-one-shot-bottles-50-ml-conc.html>
