# Image input

## Contract

- The application accepts supported raster images and camera RAW files through one normalized image boundary.
- Encoded raster input is decoded with its declared color space and transfer function. RAW input is returned as linear ACES2065-1 after camera orientation, selected white balance, and optional Lensfun correction.
- RAW white balance supports as-shot, daylight, tungsten, and finite custom temperature and tint. Unsupported or invalid camera settings fail clearly.
- Auto-exposure meters the complete source before crop and resize. Crop and resize then produce the working image and preserve the source pixel pitch for spatial stages.
- Crop bounds, dimensions, and resize factors MUST be finite and valid. Empty crops are errors.
- Input loading does not apply film, print, grain, or output encoding; those belong to the film chain and output capabilities.
- The renderer's `input` route MUST apply the requested crop and resize before output color conversion. It MUST preserve decoded input values without auto-exposure or exposure compensation.
- Observed input dimensions MUST identify the original source separately from working and published dimensions.
- Backend selection and the separation of native unpacking from RAW and raster numerical processing follow [GPU computation scope](../gpu-execution/spec.md#computation-scope). Input normalization MUST NOT conceal CPU image arithmetic inside a GPU operation.

## Capture illuminant

The **capture illuminant** is the assumed scene light of the photograph. It is a camera-side declaration and is separate from every loader, film, enlarger, scanner, and output light source.

`camera.capture_illuminant` supplies that independent scene-light declaration. Without an explicit value, the selected film profile's `reference_illuminant` remains the effective scene-light assumption; changing the profile is not required to state a different photograph capture light.

- `camera.capture_illuminant` is an optional runtime leaf. Its unset value — TOML omission, inline `null`, JSON `null`, or an omitted `parameters.camera.capture_illuminant` leaf in a preset — inherits the selected film profile's `reference_illuminant`.
- Unset is meaningful and is the state of every existing preset, stock, and invocation, so an unset leaf changes no pixels.
- An explicit value is a catalog illuminant name or a `BB<kelvin>` token. The shared illuminant vocabulary that [CLI field discovery](../cli/spec.md#discovery-and-dry-run) already reports for `enlarger.illuminant` owns the accepted names.
- An unsupported or malformed value MUST fail validation with the permitted values and MUST NOT fall back to the profile reference.
- The resolved value is the assumed scene light of the camera-side conversion into film exposure. It MUST NOT change the film's sensitivity data, density curves, development, calibration, or the developed negative.
- EXIF and RAW metadata are not a capture-illuminant source in version 1. RAW white balance remains a loader choice reported separately; it MUST NOT select, bias, or silently replace this leaf.
- The leaf is inactive where no film exposure is computed from scene RGB: the `input` route and a real negative-capture `input > convert-film > ...` route, where the incoming image is already a capture rather than a scene. An inactive value remains stored.

### Boundaries

Each control below owns one light source. The capture illuminant is distinct from all of them, and editing it MUST NOT change any of them.

| Control | Owns |
| --- | --- |
| RAW white balance | how the RAW adapter normalizes raw sensor channels |
| Film profile `reference_illuminant` | the illuminant the profile's own data is rated and calibrated for |
| `camera.capture_illuminant` | the assumed scene light of this photograph |
| readout illuminant | the scan readout, selected by the material category |
| `enlarger.illuminant` | the filtered enlarger lamp that exposes photographic paper |
| Output white point (`io.output_color_space`) | the destination color space of the delivered image |

A Capture & Scan Setup document, a universal camera or scanner registry, and automatic scene-light inference from image content are outside this contract. The [capture-illuminant DSL](capture-illuminant.dsl.md) is a worked example of the existing parameter language for this leaf, not a new schema.

### Capture illuminant acceptance

1. An unset leaf resolves to the selected film profile's `reference_illuminant` and produces pixels identical to the same configuration without the leaf.
2. An explicit catalog name and an explicit `BB<kelvin>` token resolve to that illuminant and change the camera-side conversion into film exposure without changing film density curves, development, scan calibration, the scanner receiver, or automatic scan setup.
3. An unsupported name and an out-of-range `BB<kelvin>` fail before output publication with the permitted values, in CLI dry run, parameter-file resolution, preset validation, GUI state import, and rendering alike.
4. A report distinguishes the requested value, the effective value, and the source that supplied it, and reports the profile reference illuminant that an unset leaf inherits.
5. RAW input with any white-balance mode reports its loader choice unchanged and does not populate the capture illuminant.
6. On the `input` route and on `input > convert-film > ...`, an explicit value is reported inactive with a reason and does not change the output.
