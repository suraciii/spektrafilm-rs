# Kodak T-MAX 100 / 100TMX

`kodak_tmax_100` approximates KODAK PROFESSIONAL T-MAX 100 Film / 100TMX
135 from the manufacturer's F-4016 publication. It is a black-and-white
negative using one effective silver record. It is not a measured scanner/film
match and does not model a real emulsion sublayer decomposition.

The built-in **Kodak T-MAX 100** look uses the D-76 20 C 7.5-minute curve,
Polymax Fine-Art paper, and the print route. It preserves camera exposure,
metering and film format. Its stored Polymax reference is used by the default
`input > film > print > scan` route, not treated as a measured paper match.

## Source and reconstruction

The primary reference is Kodak's
[F-4016, June 2018](https://www.kodakprofessional.com/sites/default/files/wysiwyg/pro/resources/f4016_TMax_100.pdf),
page 8. The PDF SHA-256 is
`00518d7d1e296d6065f6e41ea3db999e5d8714679fb3d2b76d9a49eb207113be`.

Page 8 gives D-76 small-tank 20 C characteristic curves at 6, 7.5 and 10
minutes, a `LOG EXPOSURE (lux-seconds)` axis, `LOG SENSITIVITY` curves at 0.3
and 1.0 above D-min, diffuse RMS granularity 8, and resolving power 63/200
lines/mm. The page-8 plots are vector paths: `pdfimages -list` reports no
raster image objects.

The extractor pins the SVG geometry for the 7.5-minute characteristic curve,
the 0.3-above-D-min spectral curve and the 1.0-above-D-min comparison curve.
The selected path is identified by its page geometry and line style; a
geometry hash rejects a changed source. The source JSON retains the axes,
converter version, vector paths and extracted coordinates.

The characteristic exposure domain is approximately -3.150 to 0.650 log10
lux-seconds. The reconstructed first-plotted density is 0.21307, used as a
base-density proxy; it is not a measured D-min. The stored reference is
-0.84659 log10 lux-seconds, the point at first-plotted density + 1.0. This is not an absolute
ISO calibration or a guarantee of output brightness. Density is clamped outside
the plotted domain; no shoulder or extra latitude is invented.

The p3 table lists D-76 at 68 F as 6.5 minutes for small-tank roll film, while
the p8 plot labels the family 6/7.5/10 minutes. This profile deliberately uses
the **printed 7.5-minute curve** and records the table discrepancy; it does not
silently substitute a table time for a curve. A future development variant
would need its own curve and identity.

The spectral baseline is the 0.3-above-D-min curve. The 1.0 curve is retained
for an ablation, not treated as a second measured layer. Values outside the
plotted domain use endpoint-slope tails and a -12 log floor. Those tails are
assumptions. The curve is normalized by the existing engine-matched RGB
adapter, fitted on 99 CIE 2017 reflectances with a 24-patch BabelColor holdout.

The fit does not make the scanner or the real film match. The profile is a
single silver record, not a measured layer decomposition. The sheet does not
publish a halation point-spread function, so the metadata uses the conservative
`weak` antihalation default; the built-in look owns its explicit restrained
halation values. Spektra has no tabular-crystal morphology or per-layer
sensitivity: T-GRAIN is represented only by the source curve, scalar RMS and
MTF context, not by a crystal-shape model. The source identity is 135; the
built-in look preserves the caller format and is not separately calibrated for
120/sheet. No 120/sheet material or 2026 Ektapan 100 alias is delivered, and
Ektapan identity is unverified.

## Built-in look and effects

The look enables Spektra grain with `rms_granularity=[9,9,9]`. Kodak reports
diffuse RMS granularity 8 at density 1.0 above minimum through a 48 micrometer
aperture with D-76 processing. Spektra uses the same aperture scale but sizes
particles from peak modeled variance. The 9.0 parameter is a model calibration:
the 48 micrometer grain-stage check on this profile realizes about 8.01/1000 at
net density 1.0, whereas the nominal 8.0 realizes about 7.09/1000. Equal
channel targets are a practical approximation, not a measured per-channel
granularity. The preset also pins `density_min=[0.03;3]` and
`uniformity=[0.97;3]`; these are engine shape defaults that place the modeled
variance peak near net density 1.0, not T-MAX measurements. The realized value
is a grain-stage target, not a final RGB RMS after post-blur, print, scan and
encoding.

Grain post-blur is 0.2 pixels and multiplicative grain USM is disabled. These
avoid the strong smoothing/edge enhancement of the generic grain defaults seen
in photo trials. The legacy `blur_dye_clouds_um = 2.0` control remains enabled; the engine applies it as a pixel-space per-particle blur, not as a measured 2 micrometre silver PSF. The global halation and
scatter amounts are both 0.15; halation channel strengths are `[0.05, 0.015, 0]`
with 65 micrometre first sigma and three bounces. This is a restrained
approximation for a fine-grain film, not a measured T-MAX PSF. No creative
diffusion, print glare, DIR, chemistry change or scanner sharpening is added.
`use_cat16=false` selects CAT02 for the input adaptation path, matching the CPU
calibration path; it does not disable chromatic adaptation.

Grain is disabled in Preview by the existing Spektra product policy. Use Scan
or a saved render to inspect texture. Grain scale follows the caller's film
format and image sampling.

## Real-scan review

Ten T-MAX 100 135 scans were accepted for qualitative review: six
Photofolly/Nimmfilm 16-bit TIFFs with lab-level but unpublished developer, and
four Alex Luyckx scans with HC-110 B 6:00 at 20 C. The sets use different
scanners, bit depths, ICC profiles and editing; no pixel fit, MTF, RMS, ΔE or
scanner-independent color claim is made. See
`experiments/tmax100-profile-preset/scans/` in the research workspace.

## Rebuild

Use Python 3.13, NumPy 2.5.4, SciPy 1.18.1 and colour-science 0.4.7 in an
external environment, plus `pdftocairo` from poppler-utils.

```sh
python3 scripts/extract_tmax100.py \
  --pdf /path/to/f4016_TMax_100.pdf \
  --output /tmp/tmax-source.json
cmp /tmp/tmax-source.json data/profile_sources/kodak_tmax_100.json
OPENBLAS_NUM_THREADS=1 PYTHONDONTWRITEBYTECODE=1 \
  python3 scripts/prepare_tmax100.py \
  --source data/profile_sources/kodak_tmax_100.json \
  --output /tmp/tmax-profile.json --report /tmp/tmax-report.json
cmp /tmp/tmax-profile.json data/profiles/kodak_tmax_100.json
```

Set input color space and transfer decoding to match the file. Fixed-exposure
comparisons must explicitly disable camera auto exposure. The material uses
the product's D55 readout convention and chromatic adaptation; it is not an
uncorrected scanner measurement.

## Limits

- F-4016 is a manufacturer publication drawing, not a batch-specific
  measurement or a scanner target.
- The 7.5-minute curve and the p3 6.5-minute table value disagree; the profile
  uses the former and records the latter rather than mixing conditions.
- The 0.3/1.0 spectral curves are two density-threshold views, not separate
  emulsion layers.
- Base, spectral tails and one-record silver approximation are assumptions.
- Kodak RMS 8 is a target, not a per-channel measurement. The `rms_granularity=9.0`
  parameter is a model calibration to the 48 micrometre grain stage; it does not
  calibrate halation or MTF, and `density_min`/`uniformity` are engine shape defaults.
- The sheet does not publish a T-GRAIN crystal morphology model; Spektra's
  scalar particle model is not a tabular-crystal simulation.
- The source identity is 135. The built-in look preserves the caller format
  (the preset test uses 60 mm and asserts it is not overwritten), so it is not
  separately calibrated for 120/sheet; no 120/sheet material or Ektapan 100
  alias is delivered. Ektapan 100 is a reported 2026 rename, not a confirmed
  identical emulsion in this work.
- Real scans reviewed for this profile are qualitative evidence only; they do
  not provide matched exposure, processing, clear/base and scanner data.
- Numerical reconstruction and software consistency do not establish physical
  fidelity.
