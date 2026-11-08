# Fujifilm Neopan Acros 100 II

`fujifilm_acros_100_ii` approximates NEOPAN 100 ACROS II (135) from the
manufacturer's AF3-0258E publication. It is a black-and-white negative using
one effective silver record. It is not the original Acros 100, a digital
Acros simulation, or a measured scanner/film match.

The built-in **Fujifilm Acros 100 II** look uses the Microfine 20 C
small-tank 10-minute condition, Polymax Fine-Art paper, neutral finishing and a
print route. It preserves camera exposure, metering and film format. Its
stored Polymax reference is used by the default `input > film > print > scan`
route, not treated as a measured paper match.

## Source and reconstruction

The primary reference is Fujifilm's
[AF3-0258E, 20.02-BX](https://asset.fujifilm.com/www/us/files/2020-08/73080443d0289b1a7b116add14a9bdfc/films_neopan100acros2_135_01_0.pdf),
pages 2-3. The PDF SHA-256 is
`afb5e24db0d9b7a219b6682b640237a525b7927e8cf1b95ba2299c0be6385cdd`.

Page 2 gives the normal EI 100 Microfine table: 20 C, 10 minutes. Page 3
contains four 300 ppi raster plots. The selected Microfine characteristic plot
is identified by its title and the printed 5/10/15/20-minute family; the
10-minute curve is the second-lowest-density curve. The spectral plot is a
`Spectrogram to Daylight (5400K)` with an explicit `Relative Sensitivity (log)`
axis. The sheet does not state the density threshold for that spectral curve. The
profile's numeric `log_sensitivity_density_over_min` field is `0.0` because the
schema has no null sentinel and the runtime does not consume the value; this is
not a measured D-min+1.0 threshold. The datasource text records the limitation.

The extractor pins the two raster image hashes:
`ba0e8db7dec9bd4a2d5f6bb33f5815791b4edf61faf1d635941dff382cf400de` for the
characteristic plot and
`a7705ce8bf7465aac546181ff5fab3b51329483c0bd034b4ba34b55fe7470029` for the
spectral plot. It traces the printed line after removing the frame and grid,
uses continuity for the spectral curve, and selects the 10-minute curve by
its vertical order and label. The source JSON retains the pixel/axis mapping,
13 characteristic anchors, the traced coordinates and the image hashes.

The characteristic exposure domain is approximately -2.725 to 0.614 log10
lux-seconds. The reconstructed base density is 0.13487. The stored reference
is -0.37131 log10 lux-seconds, the point at base+1.0. It is not an absolute ISO
calibration or a guarantee of output brightness. Density is clamped outside the
plotted domain; no shoulder or extra latitude is invented.

The spectral curve is only plotted from about 401 to 663 nm. Values outside
that domain use endpoint-slope tails and a -12 log floor. Those tails are
assumptions. The curve is normalized by the existing engine-matched RGB
adapter, fitted on 99 CIE 2017 reflectances with a 24-patch BabelColor holdout.
The sheet does not publish a halation point-spread function. The profile therefore uses the conservative `weak` antihalation metadata default; the built-in look owns its explicit restrained halation values.

The fit does not make the scanner or the real film match. In particular, the
profile is a single silver record, not a measured layer decomposition.

## Built-in look and effects

The look enables Spektra grain at RMS `[7, 7, 7]`. Fujifilm reports visual RMS 7
at density 1.0 above minimum, through a 48 micrometer aperture, with Microfine
processing. Spektra uses the same aperture scale but sizes particles from peak
modeled variance. Equal channel targets are a practical approximation, not a
measured per-channel granularity.

Grain post-blur is 0.2 pixels and multiplicative grain USM is disabled. These
avoid the strong smoothing/edge enhancement of the generic grain defaults seen
in photo trials. Dye-cloud blur remains 2 micrometres. The global halation and
scatter amounts are both 0.15; halation channel strengths are `[0.05, 0.015, 0]`
with 65 micrometre first sigma and three bounces. This is a restrained
approximation for a fine-grain film with a gray base, not a measured Acros PSF.
No creative diffusion, print glare, DIR, chemistry change or scanner sharpening
is added. `use_cat16=false` selects CAT02 for the input adaptation path,
matching the CPU calibration path; it does not disable chromatic adaptation.

Grain is disabled in Preview by the existing Spektra product policy. Use Scan
or a saved render to inspect texture. Grain scale follows the caller's film
format and image sampling.

## Rebuild

Use Python 3.13, NumPy 2.5.4, SciPy 1.18.1 and colour-science 0.4.7 in an
external environment, plus `pdfimages` from poppler-utils.

```sh
python3 scripts/extract_acros100ii.py \
  --pdf /path/to/acros135.pdf \
  --output /tmp/acros-source.json
cmp /tmp/acros-source.json data/profile_sources/fujifilm_acros_100_ii.json
OPENBLAS_NUM_THREADS=1 PYTHONDONTWRITEBYTECODE=1 \
  python3 scripts/prepare_acros100ii.py \
  --source data/profile_sources/fujifilm_acros_100_ii.json \
  --output /tmp/acros-profile.json --report /tmp/acros-report.json
cmp /tmp/acros-profile.json data/profiles/fujifilm_acros_100_ii.json
```

Set input color space and transfer decoding to match the file. Fixed-exposure
comparisons must explicitly disable camera auto exposure. The material uses
the product's D55 readout convention and chromatic adaptation; it is not an
uncorrected scanner measurement.

## Limits

- Publication plots are 300 ppi rasters, not vector measurements; line width
  and axis reading bound the extracted coordinates.
- Microfine 10 minutes is a starting point; tank, agitation, age and scanner
  change the result.
- The spectral curve's density threshold is not stated.
- Base, spectral tails and one-record silver approximation are assumptions.
- Manufacturer RMS 7 and resolving power do not calibrate runtime grain,
  halation or MTF.
- Real-lab scans reviewed for this profile are qualitative evidence only; they
  do not provide matched exposure, processing, clear/base and scanner data.
- Numerical reconstruction and software consistency do not establish physical
  fidelity.
