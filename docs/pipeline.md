# Spectral pipeline

This page explains the spectral film, print, and scanner chain. It lists the selectable spectral methods, illuminants, camera taking filters, and bundled material profiles.

## From RGB input to scanner output

The print-and-scan chain follows this order: RGB → film dye density → enlarger illuminant → print paper → scanner RGB. The [film-chain specification](../specs/film-chain/spec.md) owns the processing contract. The [film-chain design](../specs/film-chain/design.md) defines its boundaries and invariants, and the [domain glossary](../CONTEXT.md) defines the scan and conversion terms.

1. **Prepare film exposure.** Hanatos2025 provides RGB→raw spectral upsampling with its full sensitivity adaptation. That adaptation includes camera UV/IR band-pass filters with reference-illuminant normalization and spectral Gaussian blur.
2. **Develop film density.** The chain uses density-curve interpolation to obtain developed film dye density. Film processing includes halation, DIR couplers, and grain. See [grain](grain.md) for the grain engines.
3. **Expose and develop print paper.** The enlarger illuminant carries the film dye-density result into the print-paper stage. The chain performs full 81-wavelength film/print/scanner spectral integration.
4. **Scan and encode output.** The scanner produces one colorimetric RGB capture of the developed medium under the scanner lamp, referenced to the clear film base. The pipeline includes glare and output CCTF encoding.

See [scanning](scanning.md) for scan polarity and negative conversion.

## Film and print continuation (target)

**Not yet implemented:** the target CLI can stop after film or print development and
resume from that completed material state. The [stage-artifact contract](../specs/cli/stage-artifacts.md)
owns the format and admission rules. A `.sfstage` directory containing `stage.json`
represents exactly one always-resumable film or print state, not an image or a collection
of research observations.

The added routes are `input > film`, `input > film > print`, `film > scan`, `film > print`,
`film > print > scan`, and `print > scan`. A stage input requires an explicit route.
The route and `-o` select the resulting material artifact or ordinary scan image;
there is no optional process capture flag. Existing image and real-negative
`convert-film` routes retain their behavior.

A film source has already completed source interpretation, metering, geometry, film
exposure, film development, and film-domain effects. A print source has also completed
print exposure and development. A continuation MUST NOT repeat those operations or
their stochastic effects. Conflicting overrides of completed controls fail; downstream
controls remain editable, and an omitted seed inherits the source seed. It loads only
the material dependencies consumed by its remaining operations: a film scan needs no
paper, and a print scan does not require film preparation. A negative `film > scan`
conversion retains the original frozen camera exposure compensation; it does not meter
the material state as a new photograph or synthesize a negative capture.

See [CLI stage processing](cli.md#process-film-and-print-states-target) for target examples.
Boolean `--debug` adds stderr messages only; `--diagnostics REPORT` writes a structured
execution report. Neither selects material output nor captures numerical observations.
Dedicated research workflows own those buffers through the core observer under the
[instrument native-observation contract](../specs/instrument/native-observations.md),
not the `.sfstage` contract.

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
