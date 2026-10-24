# Spectral pipeline

This page explains the spectral film, print, and scanner chain. It lists the selectable spectral methods, illuminants, camera taking filters, and bundled material profiles.

## From RGB input to scanner output

The print-and-scan chain follows this order: RGB → film dye density → enlarger illuminant → print paper → scanner RGB. The [film-chain specification](../specs/film-chain/spec.md) owns the processing contract. The [film-chain design](../specs/film-chain/design.md) defines its boundaries and invariants, and the [domain glossary](../CONTEXT.md) defines the scan and conversion terms.

1. **Prepare film exposure.** Hanatos2025 provides RGB→raw spectral upsampling with its full sensitivity adaptation. That adaptation includes camera UV/IR band-pass filters with reference-illuminant normalization and spectral Gaussian blur.
2. **Develop film density.** The chain uses density-curve interpolation to obtain developed film dye density. Film processing includes halation, DIR couplers, and grain. See [grain](grain.md) for the grain engines.
3. **Expose and develop print paper.** The enlarger illuminant carries the film dye-density result into the print-paper stage. The chain performs full 81-wavelength film/print/scanner spectral integration.
4. **Scan and encode output.** The scanner produces one colorimetric RGB capture of the developed medium under the product readout illuminant, referenced to the clear film base. The pipeline includes glare and output CCTF encoding.

See [scanning](scanning.md) for scan polarity and negative conversion.

## Film and print continuation

The CLI can stop after film or print development and resume from that completed material
state. The [stage-artifact contract](../specs/cli/stage-artifacts.md) owns the format and
admission rules. A `.sfstage` directory containing `stage.json`
represents exactly one always-resumable film or print state, not an image or a collection
of research observations.

The added routes are `input > film`, `input > film > print`, `film > scan`, `film > print`,
`film > print > scan`, and `print > scan`. A stage input requires an explicit route.
The route and `-o` select the resulting material artifact or ordinary scan image;
there is no optional process capture flag. Existing image and real-negative
`convert-film` routes retain their behavior.

A film source has already completed source interpretation, metering, geometry, film
exposure, film development, and film-domain effects. A print source has also completed
print exposure and development. A continuation must not repeat those operations or
their stochastic effects. Conflicting overrides of completed controls fail; downstream
controls remain editable, and an omitted seed inherits the source seed. It loads only
the material dependencies consumed by its remaining operations: a film scan needs no
paper, and a print scan does not require film preparation. A negative `film > scan`
conversion retains the original frozen camera exposure compensation; it does not meter
the material state as a new photograph or synthesize a negative capture.

See [CLI stage processing](cli.md#process-film-and-print-states) for executable examples.
Boolean `--debug` adds stderr messages only; `--diagnostics REPORT` writes a structured
execution report. Neither selects material output nor captures numerical observations.
Dedicated research workflows own those buffers through the core observer under the
[instrument native-observation contract](../specs/instrument/native-observations.md),
not the `.sfstage` contract.

### Selected-GPU preparation

Library callers must select the backend before spectral preparation: use `Runtime::new_with_backend` or `Pipeline::new_with_spectral_backend`. `Runtime::new`, `Runtime::from_stocks`, and `Pipeline::new_with_spectral` prepare the CPU reference. To move an existing instance to WGPU, call `with_params_with_backend` with the selected backend before rendering. Immutable processing entry points reject a CPU-prepared TC table or a table owned by another device, before uploading the source; they never upload host-generated TC values as a compatibility path. Changing devices requires rebuilding preparation on the new device through the same backend-aware API. CPU processing retains its reference preparation and algorithms.

### Bounded GPU optical operations

Highlight boost, halation, viewing glare, and unsharp masking partition individual device operations when an image exceeds the storage-buffer binding limit. They do not process independent image tiles: highlight boost first finds one maximum across the full source, glare retains each original pixel's noise index, and the reference Gaussian carries its full-image boundary conditions through every partition. Halation preserves per-channel widths and the scatter/source-response/bounce/renormalization order; unsharp combines only after the full-source blur completes.

These optical stages keep full-source rasters physically paged on the GPU, so neither storage bindings nor copy transports require one allocation as large as the image. In addition to the caller's image, halation retains three RGB rasters (blur destination, accumulator, and a shared filter intermediate), plus one when source compression is active. Glare retains one RGB noise field, plus a blur destination and filter intermediate when blurred. Unsharp retains a blur destination and filter intermediate. Each raster's pages and the separate working buffers obey the effective storage limit; repeated halation blurs share the intermediate raster. `SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES` can lower that limit to exercise real device-copy partitions without changing the source extent. A limit too small for the operation's minimum working binding returns a named error rather than selecting CPU image processing.

The selected GPU chain submits optical pointwise work in bounded chunks and checks cancellation before encoding and submitting each chunk. Gaussian stages use the same cancellation-aware paged submission path. Partitioning changes scheduling and storage ownership, not the optical source extent or global statistics.

## Spectral methods and illuminants

The experimental spectral registry ships these methods:

- `hanatos2025`
- `mallett2019`
- `arctic2026alpha02`
- `arctic2026beta04`
- `gauss-lasers`
- `jakob2019`
- `otsu2018`

Runtime parameters can select a method and an illuminant. Selectable illuminants come from [`data/illuminants/catalog.json`](../data/illuminants/catalog.json): the CIE standard illuminants plus its fluorescent, LED, sodium, mercury, metal-halide, incandescent, and ISO 7589 studio and photoflood sources. The bundled `T`, `TH-KG3`, `TH-KG3-L`, and `K75P` selections are always available. `BB<temperature>` selects a temperature in the supported 1667–25000 K range.

See [LUTs](luts.md) for the corresponding LUT-build controls.

## Camera taking filters

The measured Hoya X0, X1, Y2, YA3, and R1 transmission curves are selectable in runtime parameters and the GUI.

## Bundled film and paper profiles

The repository bundles 30+ film and paper profiles in [`data/profiles/`](../data/profiles/). The profiles include Kodak Gold/Portra/Ektar, Fuji Velvia/Provia, Kodak Endura papers, and Fuji Crystal Archive papers.

## See also

- [GUI controls](gui.md)
- [CLI workflows](cli.md)
- [Scanning](scanning.md)
- [Grain](grain.md)
- [LUTs](luts.md)
- [Profile fitting](profile-fitting.md)
- [Film-chain specification](../specs/film-chain/spec.md)
- [Film-chain design](../specs/film-chain/design.md)
- [Domain glossary](../CONTEXT.md)
