# Kodak Tri-X 400

The film stock `kodak_trix_400` models still-photo **400TX, 35 mm**, developed in
T-MAX for **6 minutes at 20°C**, in a small tank with agitation every 30 seconds.
It is distinct from `kodak_trix`, the existing **7266 cine reversal** stock.
Only one 400TX material and one **Kodak Tri-X 400** built-in look are supplied.

## Source response

The source is [Kodak F-4017, October 2021](https://kodakprofessional.com/sites/default/files/wysiwyg/film/f4017_trix_320400.pdf),
page 8, upper-right characteristic curve. Six minutes also matches the normal
processing starting point on page 3. Standard T-MAX dilution 1:4 is supported by
[Kodak ED-BWF](https://kodakprofessional.com/sites/default/files/wysiwyg/pro/resources/edbwf_0.pdf);
the curve caption itself does not state dilution. This is not a D-76 curve.

The [source record](../data/profile_sources/kodak_trix_400.json) retains the vector
centerlines and axis mappings. Source input hashes and a labelled-geometry
fingerprint prevent silent curve substitution. Cubic segments are sampled before
monotone interpolation; no RGB look is fitted to a photograph.

The profile uses the upper solid sensitivity curve, labelled D=1.0 above gross
fog. The lower dashed curve is labelled D=0.3. Their printed ordering is surprising
for reciprocal exposure; this model preserves the labels rather than silently
correcting them. The [2007 edition](https://125px.com/docs/film/kodak/f4017-400TX-2007.pdf)
attributes sensitivity and MTF to older TRI-X Pan 5063. Later omission of that
qualification does not establish new measurements for current production.

## Model boundaries

- The low-exposure plotted endpoint estimates base plus fog. It is not an
  unexposed-strip or wavelength-resolved base measurement.
- Image density is the total curve minus that floor. A neutral base adds it once;
  a single neutral silver basis supplies image density once.
- One aggregate density record supports existing grain consumers. It is not an
  inferred decomposition into physical emulsion sublayers.
- The sampled curve is authoritative. There is no fitted density model that can
  overwrite it on load. Outside the plotted interval, density clamps; additional
  highlight latitude and a physical shoulder are not established.
- Exposure zero is estimated base plus fog plus one density unit. D55 input
  midgray maps to this relative coordinate; this is not an absolute ISO calibration.
- Sensitivity is sampled on the runtime wavelength grid. Unmeasured tails follow
  endpoint log slopes down to a numerical floor. A log sensitivity of zero at the
  chart edge is not zero physical sensitivity.
- The RGB spectral adapter fits one record and broadcasts its coefficients.
  Training uses CIE 2017 test reflectances under D55; BabelColor patches provide
  a separate numerical check. Neither set is a collection of Tri-X photographs.

This model does not identify spectral silver scattering, the Callier effect,
stock-specific halation, reciprocity, aging, or scanner optics. Publication-curve
reconstruction and backend agreement do not establish real-film fidelity.

## Built-in look

Select **Kodak Tri-X 400** from **Built-in**, or use the qualified CLI name:

```sh
spektrafilm process input.png -o trix.png \
  --preset 'built-in:Kodak Tri-X 400' \
  --route 'input > film > print > scan' \
  --scan-output direct_scan --backend cpu --data-dir data
```

Match the input color space and transfer decoding to the file. The look preserves
per-photograph camera exposure and metering settings; adjust exposure for the input.
It uses [unfiltered Polymax Fine-Art F](polymax.md), the reference T enlarger lamp,
unit film and print gamma, neutral finishing, and no added sharpening.

Grain and halation are enabled as class-level priors, not matched-process
calibration. The Spektra RMS `[17, 17, 17]` uses Kodak's published granularity 17
as an appearance anchor, but that measurement was made after HC-110 B at net
density 1.0 through a 48 µm aperture, not this T-MAX process. The halation values
are the restrained B&W class-level values shared with HP5, T-MAX 100 and Acros,
not a measured Tri-X PSF. DIR inhibition, paper glare, diffusion and sharpening
remain disabled. Users can adjust the effects, but doing so does not establish a
measured Tri-X grain, halation or sharpness match.

Applying a look respects an already selected direct-film route. In that context,
`direct_scan` retains negative polarity and the stored paper does not contribute.
Choose the print route for this optical positive, or explicitly select
`negative_to_positive` for the separate digital inversion interpretation.
The latter inverts the film curve and is not an equivalent paper print.

Camera optics, filters, geometry and input interpretation are photograph context,
not all preset-owned controls. For source-response comparisons, hold these fixed,
disable camera auto exposure, and compare density before display encoding.

## Rebuild

Use an external Python environment with NumPy, SciPy, and colour-science.
The recorded environment uses Python 3.13, NumPy 2.5.4, SciPy 1.18.1,
and colour-science 0.4.7. For example, create an external environment:

```sh
python3.13 -m venv /tmp/trix400-venv
/tmp/trix400-venv/bin/pip install numpy==2.5.4 scipy==1.18.1 colour-science==0.4.7
. /tmp/trix400-venv/bin/activate
export PYTHONDONTWRITEBYTECODE=1 OPENBLAS_NUM_THREADS=1
```
Source extraction also needs Poppler's `pdftocairo`. Download the pinned PDF
outside the repository; it is not redistributed.

```sh
python3 scripts/extract_trix400.py \
  --pdf /tmp/f4017-2021.pdf --output /tmp/kodak_trix_400-source.json
cmp /tmp/kodak_trix_400-source.json data/profile_sources/kodak_trix_400.json
python3 scripts/prepare_trix400.py \
  --output /tmp/kodak_trix_400.json --report /tmp/trix400-report.json
python3 scripts/test_prepare_trix400.py
```

The generator uses checked-in source coordinates. It rejects changed PDF identity,
labelled vector geometry, or coordinates that differ from those vectors.
It reports source, extractor, observer, illuminants, calibration code, LUT and
output hashes, library versions, curve residuals and spectral holdout errors.
Identical bytes require the same numerical environment. The shared calibration
helper is code reuse; no Velvia coefficients or material data are imported.
Read the [attribution notice](../data/license/TRIX_400.txt) before redistribution.
