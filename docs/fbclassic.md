# Ilford MULTIGRADE FB CLASSIC Glossy, grade 2

The print-material selector includes **Ilford MULTIGRADE FB CLASSIC Glossy**
as a fixed **filter 2 / grade 2** source-curve profile. It is one grade of one
surface, not a model of the MULTIGRADE 00–5 family. It works with the existing
photographic-print route and needs no look preset.

## Source and scope

The source is HARMAN/ILFORD's *MULTIGRADE FB CLASSIC Technical Information*,
October 2013, page 2. The sheet describes a 255 g/m² baryta-coated fibre base,
glossy 1K and matt 5K surfaces, neutral image colour, and seven full grades.
The shipped curve is the **glossy, filter 2** characteristic curve; the sheet's
curve conditions are **MULTIGRADE 1+9, 2 minutes at 20 °C**. The same sheet
gives filter-2 paper speed **ISO P230** and ISO range **R95**.

The manufacturer PDF is not redistributed. The source record is
[`data/profile_sources/ilford_multigrade_fb_classic_glossy_g2.json`](../data/profile_sources/ilford_multigrade_fb_classic_glossy_g2.json);
the source PDF SHA-256 is
`b4fe0ea11aad13ef4210c59bb42f49bc121ea213cf1a9ee96faf35832473dd43`.

## What the curve is

The generator reads the page-2 vector centerline for the cyan **filter 2**
curve. It preserves 68 source vertices and linearly samples between them for
the renderer. The extracted source domain is approximately
`-0.014080397 … 2.507444801` log-exposure units and the plotted density ends at
`2.081434961`. Those are **plotting coordinates**, not measurements of a
particular box.

The profile's exposure origin is the paper-white-relative 18% reflectance
convention: reflection density `-log10(0.184) = 0.735182…`. The plotted
x-axis is **not** used as an absolute lux-second scale. In particular, the
plotted filter-2 curve reaches density 0.6 at source x≈0.735, whereas ISO P230
implies `log10(1000/230)=0.638`; the panel and the speed table do not establish
a common absolute origin. ISO P230 is therefore metadata, not a timer
calibration.

The curve is monotone after extraction. The generator records the source
vertex error, the runtime linear-versus-PCHIP error, and the profile hash in
its preparation report.

## Spectral and base approximations

The page-1 spectral chart has only a **Relative Sensitivity** ordinate; no
numeric y-axis is printed, and the chart is shared rather than grade-specific.
The shipped `log_sensitivity` is a **flat 0.0 placeholder**. It is not a
measured paper spectral sensitivity and must not be used to predict coloured
enlarger light or variable-contrast filtration.

`base_density` is the plotted D-min proxy, treated as `0.0`; it is not a
measured paper base or reflectance spectrum. `midscale_neutral_density` is the
18% convention value. No paper fluorescence, surface BRDF, batch variation,
toning, texture or gloss is modelled.

## What the model reproduces

- The published glossy filter-2 characteristic curve up to the plotted endpoint.
- The source's plotted D-max endpoint (`≈2.0814`) as a graph endpoint, not a measured Dmax.
- A fixed-spectrum exposure response for a neutral negative under the reference-light convention.

It does **not** reproduce:

- grades 00, 0, 1, 3, 4 or 5;
- half-grade filtration;
- variable-contrast prediction from yellow/magenta filtration;
- the spectral response of the paper;
- absolute paper speed or enlarger timer values;
- base white, Dmax, toning, surface or scanner behaviour.

## Use

The stock is `ilford_multigrade_fb_classic_glossy_g2`. No preset is required;
select it with `--paper` and use an existing film preset or explicit film
stock:

```sh
spektrafilm process input.png -o fbclassic.png \
  --film kodak_doublex \
  --paper ilford_multigrade_fb_classic_glossy_g2 \
  --route 'input > film > print > scan' --scan-output direct_scan \
  --backend cpu --data-dir data \
  --input-color-space sRGB --input-cctf-decoding true \
  --set 'enlarger.illuminant=T,enlarger.c_filter_neutral=0,enlarger.m_filter_neutral=0,enlarger.y_filter_neutral=0,enlarger.m_filter_shift=0,enlarger.y_filter_shift=0'
```

`T` is the runtime's reference-light convention, not a reconstruction of the
ILFORD test enlarger. For source-response checks, disable grain, halation,
scatter, glare, automatic exposure, preflash and artistic tone changes; hold
lamp and filtration fixed; use print gamma 1; compare paper densities before
paper-white readout normalisation. The profile is fixed at the source
development condition and offers no development-time selector.

## Rebuild

Use an external Python environment with NumPy and SciPy (the research build
used NumPy 2.5.4 and SciPy 1.18.1):

```sh
python3 scripts/prepare_fbclassic.py \
  --output-dir /tmp/fbclassic-profiles \
  --report /tmp/fbclassic-preparation.json
```

The generator reads only the checked-in source record and writes
`ilford_multigrade_fb_classic_glossy_g2.json`. It checks the source PDF hash,
monotonicity and source-vertex round-trip. It does not download anything.

## Limits and verification

**Verification: BLOCKED** for absolute spectral sensitivity, measured base
spectrum, physical Dmax/paper white, batch variation, and a matched
print/scanner chain. The profile is a fixed-condition source-curve
approximation with a flat-spectrum placeholder; it is not a physical
MULTIGRADE model. The public scan audit found no scan with the exposure,
filtration, development, densitometry and scanner target required to validate
the curve.

See the [source notice](../data/license/ILFORD_MULTIGRADE_FB_CLASSIC.txt) for
attribution and redistribution boundaries.
