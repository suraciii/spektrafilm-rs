# CLI processing

This page covers CLI image processing, runtime controls, source interpretation, and image output. It also describes data discovery and diagnostics.

## Process images

```bash
# Explicit GPU export: WGPU f32 shaders. It fails before publication when the
# configuration still needs a host-executed stage, so with the default grain
# distribution this command exits 1 and writes no output. Omit --backend for
# automatic selection, which absorbs those stages as CPU fences.
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
See [source declarations](../specs/cli/spec.md#input-declaration) and the [image input contract](../specs/image-input/spec.md).
RAW examples retain automatic linear ACES2065-1 declaration.
See [RAW input](raw.md) for decoding details and [scanning](scanning.md) for negative conversion.

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
The [CLI parameter contract](../specs/cli/spec.md) defines parameter editing.

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
Omit `--output` when checking parameters without choosing a destination. The report keeps `output.path` as `null`; `output.format` is also `null` unless `--format` is supplied. A normal process invocation still requires a destination.
Without a format, container-specific defaults such as bit depth and compression remain `null`. Supply `--format` to validate codec-specific options such as JPEG quality or compression.

```bash
./target/release/spektrafilm-f64 process input.png \
    --preset 'built-in:Kodak Gold 200' \
    --input-color-space sRGB --input-cctf-decoding true \
    --dry-run --data-dir data
```

It checks the input path and supported file kind.
Image decoding and compute-adapter availability remain unchecked.
Saving color flags control writer conversion independently of scanner `io.output_color_space` and `io.output_cctf_encoding`.

Recipe `seed` values are admitted in the unsigned 32-bit range (`0..=4_294_967_295`).

### Feature profiles

`--feature-profile FAMILY=NAME` applies a built-in feature profile. The option is repeatable, and a later selection for the same family replaces the earlier one. Profile selections apply in appearance order before every `--set` source, so a later `--set` assignment can still override a profile's own leaves. `FAMILY` is `grain.procedural` or `halation`. An unknown family or name fails before rendering. The [feature profile contract](../specs/feature-profiles/spec.md) owns the families, names, and scope values.

```bash
# Select a procedural grain profile
./target/release/spektrafilm-f64 process input.png -o output.tif \
    --input-color-space sRGB --input-cctf-decoding true \
    --film fujifilm_provia_100f --scan-film \
    --feature-profile grain.procedural=35mm250 --data-dir data
```

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
See the [CLI workflow contract](../specs/cli/spec.md#data-directory-selection).
Profile metadata and array dimensions are validated before construction.
Present but malformed neutral-filter JSON is an error.

## Debug and diagnostics

The target `process` interface accepts Boolean `--debug` for additional human-readable
stderr messages. It takes no directory argument, writes no files, captures no numerical
buffers, and must not change parameters, routing, or numerical results. This is a target
contract, not a claim that the current binary implements it. See the authoritative
[debug and diagnostics contract](../specs/cli/spec.md#debug-diagnostics).

`--diagnostics REPORT.json` is separate: `process` and `render` accept it to persist a
structured execution report.
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
    --diagnostics report.json --gpu-timings --data-dir data
```

`--gpu-timings` follows the selected backend. The example omits `--backend` so automatic
selection can absorb the default grain distribution as a CPU fence; `--backend gpu` would
fail this configuration before publication.

See [the telemetry contract](../specs/telemetry/spec.md) and [report metrics](../specs/telemetry/metrics/spec.md) for report semantics, bounded detail, and measurement limits.

## Process film and print states (target)

**Not yet implemented:** the following process routes and `.sfstage` inputs/outputs are
the target interface. They are not instructions for the current executable. The
[stage-artifact contract](../specs/cli/stage-artifacts.md) owns route admission, the
artifact format, frozen controls, and continuation semantics.

The command shape is `process INPUT --route ROUTE -o OUTPUT [--debug] [--diagnostics REPORT]`.
A `.sfstage` directory with `stage.json` is one completed, always-resumable film or print
state, not a directory of optional observations. An image-to-stage route stops after
developing the selected material; a continuation starts from that completed state and
executes only the remaining operations. A stage input always requires an explicit route.

```bash
# Target: develop an image into a retained film state.
./target/release/spektrafilm-f64 process input.tif \
    --route 'input > film' -o negative.sfstage \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --film kodak_portra_400 --backend cpu --data-dir data

# Target: expose and develop a print from that completed film state.
./target/release/spektrafilm-f64 process negative.sfstage \
    --route 'film > print' -o print.sfstage \
    --paper kodak_portra_endura --data-dir data

# Target: scan that completed print; --debug writes messages only to stderr.
./target/release/spektrafilm-f64 process print.sfstage \
    --route 'print > scan' -o print.png --debug \
    --diagnostics print-report.json --data-dir data
```

The added routes are `input > film`, `input > film > print`, `film > scan`,
`film > print`, `film > print > scan`, and `print > scan`. Routes ending in `film` or
`print` write one `.sfstage` state; routes ending in `scan` use the ordinary image writer.
Existing image routes and real-negative `convert-film` behavior remain unchanged.

A continuation inherits the source seed when no seed is supplied. Controls for completed
operations are frozen: conflicting overrides fail rather than re-rendering the source.
Only downstream controls remain editable. Geometry, metering, film development and its
effects, and already completed print development must not run a second time. Loading a
stage requires only the material dependencies used by the requested continuation; a film
scan does not require a paper profile, and a print scan does not rerun film preparation.

There is no `--stage-evidence` process option or replacement capture flag. Stage output is
selected by the route and `-o`, not by `--save-stage`, `--export-stage`, or a directory
argument to `--debug`. Dedicated research observation and measurement workflows own
numerical buffers through the core observer; those observations are not `.sfstage` files.
See the [instrument native-observation contract](../specs/instrument/native-observations.md)
and [measurement](measurement.md). Existing `sample --from-evidence` and `sample --from-run`
remain research readers, not process-stage loaders.

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
- [CLI parameter contract](../specs/cli/spec.md)
- [Data directory selection](../specs/cli/spec.md#data-directory-selection)
- [Image input contract](../specs/image-input/spec.md)
- [Telemetry contract](../specs/telemetry/spec.md)
- [Report metrics](../specs/telemetry/metrics/spec.md)
