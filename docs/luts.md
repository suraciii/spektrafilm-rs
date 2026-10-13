# LUT creation and delivery

This page describes LUT baker inputs, bundle contents, and format orders. The [LUT delivery specification](../specs/lut-delivery/spec.md) owns the delivery contract.

## Baker inputs

The calibrated LUT creator mirrors the upstream LUT path in deterministic `lut_mode`.
It bakes against upstream reference commit `28bf883e1672e884307edc75852549376e13644e`.
It includes non-spatial DIR chemistry and neutral-print calibration.
It supports one through four LUTs.
It supports shared film stages across repeated `--print` stocks.
`--combinations` includes every contiguous collapsed sub-chain.

LUT builds accept `--params runtime_params.json`, `--stops-above-midgray <auto|native|null|STOPS>`, and the legacy additive `--exposure-ev EV`.
See [the pipeline guide](pipeline.md) for spectral methods and illuminants.
Schema 3 bundles record the full digested parameter tree per print.
They also record the digest's changed values, a SHA-256 snapshot digest, and resolved input stops and gain.
`Bundle::baked_params` preserves the actual bake configuration for QA.
Preview, Scan, Export, and complete source-to-output LUTs use the same overall effect amount. Segmented LUTs require amount `1`; see the [LUT delivery specification](../specs/lut-delivery/spec.md) for the topology rule.
Grain and coupler TOML presets override controls when stock specifics are requested.
Later edits use `apply_stocks_specifics=false`.

```bash
./target/release/spektrafilm lut list input
./target/release/spektrafilm lut list output
SPEKTRAFILM_BACKEND=cpu ./target/release/spektrafilm-f64 lut build build/lut_bundles \
    --film kodak_portra_400 --print kodak_portra_endura \
    --input vlog --output srgb --topology 4lut --resolution 33 \
    --combinations --data-dir data
./target/release/spektrafilm lut build build/lut_bundles --from bundle.toml \
    --resolution 65 --data-dir data
```

See [the CLI guide](cli.md) for `--data-dir` selection rules.

TOML fields match the typed `spektrafilm_core::lut_baker::BundleSpec`.
Supplied CLI flags override file values.
A minimal spec is:

```toml
film_profile = "kodak_portra_400"
print_profiles = ["kodak_portra_endura"]
input_color_space = "Panasonic V-Log"
output_color_space = "sRGB"
topology = "3lut"
resolution = 33
stops_above_midgray = "auto"
include_combinations = true

[input_gamut_compress]
algorithm = "xy"
knee = [0.0, 1.0, 6.0]

[output_gamut_compress]
algorithm = "cam16ucs"
knee = [0.0, 1.0, 6.0]
```

## Input stops and parameter digestion

Canonical names and registry short tags are accepted.
Disabled scene-linear registry roles fail explicitly.
The default `"auto"` bridge resolves to four stops for encoded SDR inputs.
It resolves to six stops for scene-referred camera-log inputs.
`native`/`null` preserves the registry's native gain.
Explicit stops use `0.18 * 2^stops / decode(1)`.
Legacy `--exposure-ev` adds a deliberate multiplier of `2^EV`.
`bundle.json` records the resolved stops and gain.
PQ and HLG outputs apply the inverse midgray bridge before encoding.
Intermediate wires retain the probed log-exposure margins and below-fog density headroom.

Params-first baking preserves runtime gamut and look controls.
It replaces the passthrough `input` route with `input > film > print > scan`.
It clears taps and preview mode.
LUT digestion disables crop, resize, and internal spectral acceleration LUTs.
It neutralizes per-image exposure, enlarger filter shifts, and preflash.
The snapshot's `digest_changes` records these changes relative to an ordinary render of the same settings.

## Bundle contents and camera target

LUT bundles carry their stock, transport, exposure, topology, wire constants, and parameter snapshots in `bundle.json`.
They also carry consumer instructions, the canonical upstream `SPEKTRAFILM_LICENSE.txt`, and derivative notices.
Directory and ZIP delivery use portable relative artifact paths.

The only active camera target is `lumix_realtime_vlog`.
It uses Panasonic V-Log input, sRGB/Rec.709/Rec.2020 output, and the minimal `#LUMIXPHOTOSTYLE VLOG` header.
Upstream field verification covers the 33³ grid.
Other exporter-accepted resolutions have no camera compatibility claim.

Profiles and their derived LUTs are by Andrea Volpato.
They are licensed under [CC BY-SA 4.0](../data/license/SPEKTRAFILM_LICENSE.txt).
They come from [the canonical spektrafilm source](https://github.com/andreavolpato/spektrafilm), pinned to `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`.
Bundles preserve the source license bytes.
They record migration, bake settings, and quantization modifications.
See [attribution](attribution.md) and [building and packaging](building.md) for license and packaging details.

## Formats and Rust consumers

The core format API reads and writes generic CUBE, strict Lumix CUBE, Autodesk 10-bit 3DL, and Hald RGB8 PNG.
Hald requires a perfect-square cube resolution (`N=L²`).
It packs an `L³`-square image.
It quantizes without a transfer curve.
3DL and Hald match NumPy ties-to-even quantization.
They clamp output codes.
Wire file order is red-fast.
Baker tables remain blue-fast.

For Rust consumers, `BundleBuilder::build` returns typed `Bundle`, `Lut`, and `BundleMeta` values.
`Lut::table` is blue-fast `[r][g][b]`, indexed by `(r * resolution + g) * resolution + b`.
Format writers reorder at serialization.
`export-lut` calls the same baker for a single combined ProPhoto RGB to sRGB transform at the default `auto` input stops.

## OCIO and QA artifacts

OCIO and QA producers can append relative artifacts and finalize the complete bundle before ZIP creation.
`lut build --ocio-config` emits a standalone OCIO 2.4 configuration.
It records the configuration's relative artifact path.
`--qa` runs all sixteen pinned quality scenarios for every print.
`--qa-print-index N` selects one print.
QA writes JSON, Markdown, offline HTML, and PNG figures under `qa/`.
It registers every file before ZIP finalization.
The command prints the measured QA PASS/FAIL result.
A physical quality failure remains visible in the delivered report.

LUT parity checks live in [scripts/parity](../scripts/parity/). See the [LUT delivery specification](../specs/lut-delivery/spec.md) for the delivery requirements.

## See also

- [Documentation index](README.md)
- [CLI guide](cli.md)
- [Pipeline guide](pipeline.md)
- [Preset guide](presets.md)
- [Parity evidence](parity.md)
- [Attribution](attribution.md)
- [Building and packaging](building.md)
- [LUT delivery specification](../specs/lut-delivery/spec.md)
- [Canonical asset license](../data/license/SPEKTRAFILM_LICENSE.txt)
