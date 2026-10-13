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

RAW CLI examples retain automatic linear ACES2065-1 declaration. See the [CLI guide](cli.md) and [source declarations](../specs/cli-parameters/spec.md#input-declaration).

## Decoder parity

Decoder parity is verified by [`scripts/parity/raw_reference_compare.py`](../scripts/parity/raw_reference_compare.py) and [`scripts/parity/lens_reference_compare.py`](../scripts/parity/lens_reference_compare.py). Fixture provenance lives in [`scripts/parity/raw_fixtures.json`](../scripts/parity/raw_fixtures.json). Downloaded images are not redistributed.

## See also

- [Building](building.md)
- [GUI guide](gui.md)
- [CLI guide](cli.md)
- [Benchmark guide](benchmark.md)
- [Parity evidence](parity.md)
- [RAW fixture provenance](../scripts/parity/raw_fixtures.json)
- [Parity scripts guide](../scripts/parity/README.md)
- [Image input specification](../specs/image-input/spec.md)
- [CLI input declarations](../specs/cli-parameters/spec.md#input-declaration)
