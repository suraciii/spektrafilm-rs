# CLI processing

This page covers CLI image processing, runtime controls, source interpretation, and image output. It also describes data discovery and diagnostics.

## Process images

```bash
# GPU export: WGPU f32 shaders, CPU stages where required
./target/release/spektrafilm-f64 process input.ORF -o out.png \
    --backend gpu --film kodak_gold_200 --paper kodak_portra_endura --data-dir data

# CPU f64 reference export
./target/release/spektrafilm-f64 process input.ORF -o out.png \
    --backend cpu --film kodak_gold_200 --paper kodak_portra_endura --data-dir data

# Explicit JPEG quality and chroma sampling
./target/release/spektrafilm-f64 process input.tif -o out.jpg \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --format jpeg --bit-depth 8 --jpeg-quality 95 --jpeg-subsampling 444 \
    --backend cpu --film kodak_gold_200 --scan-film --data-dir data

# Override any params via JSON (matches RuntimeParams struct)
./target/release/spektrafilm-f64 process input.ORF -o out.png \
    --backend cpu --film kodak_gold_200 --paper kodak_portra_endura \
    --params my_params.json --data-dir data

# List available film + paper profiles
./target/release/spektrafilm list-profiles --data-dir data
```

The TIFF examples above use prepared linear ProPhoto RGB samples.
The PNG example below uses encoded sRGB samples.
Declare the actual source interpretation for your own files.
See [source declarations](../specs/cli-parameters/spec.md#input-declaration) and the [image input contract](../specs/image-input/spec.md).
RAW examples retain automatic linear ACES2065-1 declaration.
See [RAW input](raw.md) for decoding details and [scanning](scanning.md) for negative conversion.

## Original Fotufilm simulation

`engine=fotufilm` selects the complete pinned original model independently of the adapted Fotufilm grain appearance inside SpektraFilm. Original stock, grain, optics, layered transport, print or scan medium, and source controls run through original preparation and the Halide CPU graph. It retains CPU f32 arithmetic even in `spektrafilm-f64`. See the [source-engine contract](../specs/film-chain/spec.md#original-fotufilm-engine).

```bash
# Inspect original controls, stock applicability, native options, and defaults.
./target/release/spektrafilm describe --engine fotufilm --data-dir data

# Complete original simulation with an explicit raster input interpretation.
./target/release/spektrafilm process input.png -o output.png \
    --engine fotufilm --fotufilm-stock portra400 \
    --input-color-space sRGB --input-cctf-decoding true \
    --backend cpu --seed 1179208781 --data-dir data

# Select the original crystal-population grain model.
./target/release/spektrafilm process input.png -o film.png \
    --engine fotufilm --fotufilm-stock portra400 \
    --fotufilm-native 'grainModel="film"' \
    --input-color-space sRGB --input-cctf-decoding true \
    --backend cpu --seed 1179208781 --data-dir data
```

Native `stage="negative"` and the `cmy_film` collection tap return gross C/M/Y optical record densities. Save and Export require unencoded 32-bit TIFF or EXR. Density output preserves source geometry even with preview settings enabled; crop and resize belong after print or RGB readout. Collecting `rgb_in` retains the declared input color space and transfer state for display and saving.

`--fotufilm-native` accepts inline JSON objects and arrays, including nested objects. Unknown fields in structured options are rejected before rendering. Input declarations, parameter-source occurrence numbers, and RAW declaration rejection follow the shared CLI parameter contract. Original simulation seeds support the full UInt64 range. See [building](building.md) for Halide dependencies and preparation-module rebuilds, and [the GUI guide](gui.md#original-fotufilm-engine) for interactive operation.

## Backend and precision

`process --backend cpu|gpu` overrides `SPEKTRAFILM_BACKEND`.
Omitting it preserves the environment/default selection.
CPU precision follows the executable build.
Use `spektrafilm-f64` for reference exports.
Output bit depth is independent of computation precision.
GPU selection does not disable grain, optical effects, or requested spectral LUTs to force acceleration.
Software Vulkan adapters can also execute the WGPU path.
Speed depends on the adapter and active effects.

## Runtime edits and dry runs

See [presets](presets.md) for preset composition, identity, and the library.
The [CLI parameter contract](../specs/cli-parameters/spec.md) defines parameter editing.

```bash
# Apply a sparse file, then an inline edit, with independent writer settings
./target/release/spektrafilm-f64 process input.png -o output.tif \
    --input-color-space sRGB --input-cctf-decoding true \
    --preset 'Kodak Portra 400' --set adjustments.toml \
    --set 'enlarger.print_exposure=1.2' --backend cpu \
    --format tiff --bit-depth 16 --compression zip \
    --saving-color-space 'ProPhoto RGB' --saving-cctf-encoding true --data-dir data
```

`--dry-run` prints effective runtime controls, writer options, profile names, and the requested backend policy.
It checks the input path and supported file kind.
Image decoding and compute-adapter availability remain unchecked.
Saving color flags control writer conversion independently of scanner `io.output_color_space` and `io.output_cctf_encoding`.

Recipe `seed` values are admitted in the unsigned 32-bit range (`0..=4_294_967_295`).

## Geometry and auto-exposure

In JSON, set `io.crop`, `io.crop_center: [x, y]`, `io.crop_size: [width, height]`, and `io.upscale_factor`.
Center coordinates are normalized to the source axes.
Both size components are fractions of the source's long edge.
Bounds and rounding follow the upstream NumPy slice convention, including negative-index slicing when a crop exceeds the short edge.
Empty crops and nonpositive/nonfinite resize factors return errors before output is written.

Auto-exposure meters an antialiased nearest-neighbor preview of the complete source before crop and resize.
Encoded preview samples are decoded for f64 luminance measurement.
The resulting exposure scales source samples before geometry and filming decode, matching Python's order.
Cropping preserves the source film pixel pitch.
Resizing divides it by the requested factor.
Lens blur, halation, and DIR consume that retained pitch on CPU and supported resident GPU paths.
Spektra grain and faithful optical diffusion remain on the CPU per-stage path.
Resize uses cubic B-spline interpolation with half-pixel coordinates, mirror boundaries, Gaussian antialiasing for downscale, ties-to-even dimensions, and source-range clipping.
Interpolation computes in f64 before the configured image precision boundary.

## Color management

Color management follows the pinned Python 0.3.4 registry for sRGB, ProPhoto RGB, ITU-R BT.2020, ACES2065-1, Adobe RGB (1998), Display P3, and DCI-P3.
`io.input_cctf_decoding` decodes encoded input for metering after preview sampling.
It decodes input for film exposure after source exposure scaling and geometry.
RAW loaders supply linear values.
`io.output_cctf_encoding` selects the destination's own transfer function.
Floating scan output retains negative and super-white values.
Encoding preserves them only through sign-preserving transfer functions; Adobe RGB (1998) and gamma 2.6 destinations do not preserve negative values.
Bounded image writers and the display preview perform their own clipping.

Input gamut compression uses `active`, `algorithm` (`xy` or `oklch`), and `knee`.
Output `algorithm` accepts `off`, `aces_rgc`, `oklch`, `oklrab`, `jzazbz`, and `cam16ucs`.
Perceptual tables are built for the selected destination.
Unknown spaces and algorithms fail before rendering.
The GUI converts native output to sRGB for display while preserving native values for saving.

## Image output

CLI `process --format jpeg|png|tiff|exr` selects the container.
When supplied, `--format` must match the output extension.
Otherwise, the extension selects JPEG/PNG/TIFF/EXR.

`--bit-depth 8|16|32` selects the output depth:

- JPEG and PNG require 8-bit uint8 output.
- TIFF supports 8-bit uint8, 16-bit uint16, and 32-bit float32 output.
- EXR supports 16-bit half and 32-bit float32 output. EXR rejects 8-bit output.

Bit depth defaults to 8 for JPEG/PNG and 16 for TIFF/EXR.
JPEG defaults to quality 95 and 4:4:4 chroma sampling.
Explicit JPEG quality is 1–100.
Chroma sampling is 4:4:4 or 4:2:0.
`--compression zip|none` applies to TIFF/EXR.
JPEG/PNG reject compression options.
TIFF compression accepts ZIP/none.
EXR uses ZIP.

Integer output clips to [0,1], scales, and truncates.
Float TIFF/EXR preserves negative and super-white samples.
The writer's color space/CCTF describes the pixels it receives and controls ICC/color tags.
It applies no extra transfer function.
CLI `process` requires linear EXR output and limits the EXR saving space to `sRGB` or `ACES2065-1`.
Omitting `--saving-color-space` selects `ACES2065-1` for EXR and the scanner output space for other formats.
Omitting `--saving-cctf-encoding` selects linear for EXR and inherits the scanner output encoding otherwise.
GUI Save converts retained output using the current simulation saving settings.
GUI Export uses its independent saving settings.
Matching profiles in `data/icc` include encoded/linear variants.
Linear P3 has no bundled upstream profile.
Source EXIF/IPTC/XMP is copied for non-EXR output.
Orientation, dimensions, software, date, and color tags are refreshed.

Writer options describe the resulting pixels and never change the simulation gamut mapping.

## Data discovery

Data-consuming commands select `--data-dir`, then `SPEKTRAFILM_DATA_DIR`, then automatic executable-relative or working-directory data.
Explicit selections must exist and never fall back.
Help, version, field discovery, and preset listing without an explicit selection need no data.
See the [CLI workflow contract](../specs/cli-workflow/spec.md#data-directory-selection).
Profile metadata and array dimensions are validated before construction.
Present but malformed neutral-filter JSON is an error.

## Diagnostics

`process` and `render` accept `--diagnostics REPORT.json`.
Add `--gpu-timings` to collect compute-pass timestamps when the adapter supports them.
The report includes operation outcomes, repeated simulation attempts, saving attribution, actual GPU work, and explicit availability reasons.
Diagnostics do not change image parameters or machine-readable render output.
Report-write errors are reported separately from the image result.
CLI reports never replace an existing file.
Report paths must not alias input, output, or configuration files.

```bash
./target/release/spektrafilm process input.tif -o out.png \
    --film kodak_portra_400 --paper kodak_portra_endura \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend gpu --diagnostics report.json --gpu-timings --data-dir data
```

See [the telemetry contract](../specs/telemetry/spec.md) and [report metrics](../specs/telemetry/metrics.md) for report semantics, bounded detail, and measurement limits.

## See also

- [Documentation index](README.md)
- [Building](building.md)
- [Pipeline](pipeline.md)
- [RAW input](raw.md)
- [GUI](gui.md)
- [Scanning](scanning.md)
- [Presets](presets.md)
- [Grain](grain.md)
- [LUTs](luts.md)
- [CLI parameter contract](../specs/cli-parameters/spec.md)
- [Data directory selection](../specs/cli-workflow/spec.md#data-directory-selection)
- [Image input contract](../specs/image-input/spec.md)
- [Telemetry contract](../specs/telemetry/spec.md)
- [Report metrics](../specs/telemetry/metrics.md)
