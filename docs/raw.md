# RAW input

This page describes RAW decoding, white balance, and optional Lensfun correction.

## Decoding and white balance

`spektrafilm-raw` returns linear ACES2065-1 pixels after LibRaw camera orientation, optional Lensfun correction, and white balance. See the [image input contract](../specs/image-input/spec.md) for the normalized input boundary.

- Camera WB (`as-shot`) is the default.
- `daylight` uses LibRaw's daylight base.
- `tungsten` and `custom` adapt that base using the reference CIE D/Kang 2002 whitepoints and Von Kries/CAT02 transform.
- Custom temperature must be finite and in the range 1667–25000 K.
- Custom tint must be finite.
- An absent custom temperature is an error.

Lensfun applies vignetting before combined channel geometry resampling. Resampling uses bilinear interpolation and nearest boundary extension. When camera or lens matching fails, Lensfun preserves unchanged pixels and an empty lens summary.

See [building](building.md) for LibRaw, Lensfun, Exiv2, and GLib dependencies, `pkg-config` setup, and the Windows LibRaw build script. See the [GUI guide](gui.md) for opening camera RAW files and selecting RAW white balance and Lensfun correction.

## Shared decoder helper

The shared helper accepts this command:

```bash
decode_raw_gui input.CR2 output.tif --raw-white-balance custom --raw-temperature 5000 --raw-tint 1.05 --lens-correction
```

It writes **linear ACES2065-1 float TIFF**. Configure pipeline input as ACES2065-1 with input CCTF decoding disabled.

RAW CLI examples retain automatic linear ACES2065-1 declaration. See the [CLI guide](cli.md) and [source declarations](../specs/cli/spec.md#input-declaration).

## Decoder parity

Decoder parity is verified by [`scripts/parity/raw_reference_compare.py`](../scripts/parity/raw_reference_compare.py) and [`scripts/parity/lens_reference_compare.py`](../scripts/parity/lens_reference_compare.py). Fixture provenance lives in [`scripts/parity/raw_fixtures.json`](../scripts/parity/raw_fixtures.json). Downloaded images are not redistributed.

The ignored [`real_input` integration test](../crates/spektrafilm-raw/tests/real_input.rs)
compares every normalized ACES RGB sample from a frozen external
`SPEKTRAFILM_RAW_PARITY_FIXTURE`. It reports the eight largest positive errors
with coordinates and both RGB triplets. It preserves the mean 0.005 / maximum
0.05 acceptance bounds and checks output dimensions, retained device storage,
and the native/device lens summaries. By default it covers all four white
balances with Lensfun off and on. For a focused diagnostic, set
`SPEKTRAFILM_RAW_PARITY_WB` to `as_shot`, `daylight`, `tungsten`, or `custom`,
and/or `SPEKTRAFILM_RAW_PARITY_LENS` to `false` or `true`; invalid selections
fail rather than silently skipping all comparisons.

The sensor log records the decoder version, logical colors, packed filters,
storage channels and layout, Foveon flag, floating-point maximum, black metadata,
CFA extent, Fuji width, pixel aspect, and native-default demosaic selection.
The selection comes from the production [`plan` resolver](../crates/spektrafilm-raw/src/lib.rs),
not a test-specific classifier. A filename or camera label is not proof that a
particular interpolation or storage path ran. Lensfun-on acceptance requires a
nonempty matched lens summary; enabling the option alone can exercise the
unchanged-pixel path when calibration is unavailable.

Before running, freeze each fixture outside the repository and retain a SHA-256
manifest. Record provenance and license, expected sensor metadata and demosaic
selection, output dimensions, and required camera/lens match with that manifest.
Check the hash before each acceptance run and compare the logged mode with the
recorded expectation. Keep fixture bytes, manifests, and run output external;
do not substitute a different sensor family under the same case name.

With `SPEKTRAFILM_RAW_PARITY_FIXTURE` and `RAW_FIXTURE_SHA256_MANIFEST` set to
the frozen input and its existing SHA-256 manifest, run all eight combinations:

```bash
sha256sum --check "${RAW_FIXTURE_SHA256_MANIFEST:?set frozen SHA-256 manifest}" &&
cargo test -p spektrafilm-raw --test real_input \
  native_and_retained_device_outputs_report_comparable_samples \
  -- --ignored --exact --nocapture --test-threads=1
```

The manifest must name the selected fixture. Its paths must be absolute or
relative to the repository root, where this command runs. Leave both optional
selection variables unset for the full eight-combination acceptance run.

Native X-Trans decoding uses LibRaw's unchanged three-pass arithmetic with
ordered tile rows, matching device execution. LibRaw 0.22.2's OpenMP top-row
loop otherwise races candidate green-bound reads against the previous row's
RGB writeback. For each X-Trans native processing call only, the bridge scopes
the calling OpenMP task's thread count to one and restores its prior setting
on success or failure. Other RAW algorithms and concurrent callers keep their
own thread settings; unpack remains unchanged. This removes schedule-dependent
boundary pixels and can reduce native X-Trans processing throughput. Builds
without an OpenMP-enabled LibRaw already execute that ordered algorithm.

## Device execution

An active GPU backend uses LibRaw for file parsing and sensor unpacking, then
executes numerical postprocessing on the device. The selected interpolation
follows LibRaw 0.22.2's native default: Bayer AHD, Fuji Bayer PPG, irregular or
four-colour VNG, three-pass Markesteijn X-Trans, or no interpolation for
already-coloured samples. VNG includes native linear interpolation; linear is
not substituted for a sensor whose native default is another algorithm.

The device stages include black subtraction, the measured saturation-level
reduction and scale-factor resolution, missing-zero correction when requested
by the decoder, second-green handling, interpolation, Fuji diamond placement
and rotation, camera-to-ACES conversion, pixel-aspect stretch, orientation,
optional Lensfun correction, CAT02 adaptation, and custom tint. Fuji rotation
precedes camera conversion; stretch follows it. The stretch implementation
preserves LibRaw's integer conversion of its fractional coordinate rather than
silently replacing the native operation with a different resampler.

Lensfun database lookup, calibration interpolation, and automatic-scale
preparation remain shared native metadata work. Only a fixed-size coefficient
record crosses to the GPU, not host-generated correction coordinates or gains.
Vignetting, projection, distortion, TCA, coordinate recurrence, and nearest-edge
bilinear resampling run on the device before CAT02 and tint.
A fixed four-pair synthetic calibration classifies the linked Lensfun Poly3 and
PTLens callbacks once, using only its public API. It exports a scalar/SSE
semantics bit, not probe coordinates: the SSE callback associates four radius
scales with interleaved coordinate lanes, which combined RGB geometry must
preserve. Unknown callback behavior is rejected rather than inferred from CPU
architecture. Odd-width unaligned rows and scalar callback tails retain their
native behavior.

The retained RAW loader returns the final device buffer to the image pipeline
without a download/reupload. The materializing loader exists for explicit host
boundaries such as decoder parity probes. Once a retained GPU route is selected,
a missing implementation, invalid native CFA, or unrepresentable working set is
an error, not permission to run host image arithmetic.

Large RAW planes use bounded four-channel integer pages. Global maximum and
automatic-WB reductions complete before any scaled demosaic window is committed.
Bayer windows preserve their full dependency halo and global CFA phase;
X-Trans retains native tile/refinement order. Output geometry and Lensfun use
device-generated sparse gather requests against retained pages. Only bounded
footprint/cursor metadata is read by the host, never per-pixel correction maps.
`SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES` can force smaller image windows for seam
regressions; the fixed native X-Trans tile scratch remains a separately checked
minimum working set.

Floating-point DNG unpack explicitly disables LibRaw's optional
`CONVERTFLOAT_TO_INT` operation. The GPU reproduces its float-to-ushort
normalization and black metadata scaling **before** black-level folding; scaling
an already folded sum would move a native truncation boundary. Format-required
floating delta reconstruction and its decoder maximum remain part of unpack.
The CPU reference retains LibRaw's original unpack and arithmetic defaults;
X-Trans native processing uses the ordered execution contract described above.

`cargo test -p spektrafilm-raw --features precision-f64 --lib` selects the
binary64 host contracts in the RAW crate, its backend, and its device parity
dependency together; it does not change the retained GPU sample format.

Synthetic regressions cover CFA interpolation, native geometry boundaries,
orientation, Lensfun stages, and coordinate rounding. Frozen real-input
comparisons also cover examples of ordinary Bayer, four-color, X-Trans,
Fuji-rotated, nonidentity-aspect, floating-point DNG, already-colored X3F,
and calibrated Lensfun processing. This coverage applies to those fixtures and
the exercised native-library and adapter configurations, not all RAW files,
cameras, decoders, or hardware. X3F acceptance requires a LibRaw build that
enables its decoder; it does not change a distribution's decoder security policy.

Other GPU hardware, drivers, and platform-native library configurations still
require real-input acceptance and performance measurements. Lensfun's native
x86 SSE reciprocal-square-root/reciprocal approximations, SIMD lane ordering,
and double transcendental functions can differ from scalar callbacks and WGSL
arithmetic. Acceptance must record the platform, adapter/driver, native library
versions, and Lensfun database used. Neither bitwise Lensfun parity nor
cross-platform acceptance follows from passing the normalized mean/max bounds.
X-Trans's ordered native processing can also change reference throughput.

## See also

- [Building](building.md)
- [GUI guide](gui.md)
- [CLI guide](cli.md)
- [Benchmark guide](benchmark.md)
- [Parity evidence](parity.md)
- [RAW fixture provenance](../scripts/parity/raw_fixtures.json)
- [Parity scripts guide](../scripts/parity/README.md)
- [Image input specification](../specs/image-input/spec.md)
- [CLI input declarations](../specs/cli/spec.md#input-declaration)
