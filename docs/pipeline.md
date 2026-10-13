# Spectral pipeline

This page explains the spectral film, print, and scanner chain. It lists the selectable spectral methods, illuminants, camera taking filters, and bundled material profiles.

## From RGB input to scanner output

The print-and-scan chain follows this order: RGB → film dye density → enlarger illuminant → print paper → scanner RGB. The [film-chain specification](../specs/film-chain/spec.md) owns the processing contract. The [film-chain design](../specs/film-chain/design.md) defines its boundaries and invariants, and the [domain glossary](../CONTEXT.md) defines the scan and conversion terms.

1. **Prepare film exposure.** Hanatos2025 provides RGB→raw spectral upsampling with its full sensitivity adaptation. That adaptation includes camera UV/IR band-pass filters with reference-illuminant normalization and spectral Gaussian blur.
2. **Develop film density.** The chain uses density-curve interpolation to obtain developed film dye density. Film processing includes halation, DIR couplers, and grain. See [grain](grain.md) for the grain engines.
3. **Expose and develop print paper.** The enlarger illuminant carries the film dye-density result into the print-paper stage. The chain performs full 81-wavelength film/print/scanner spectral integration.
4. **Scan and encode output.** The scanner produces RGB output from the developed medium. The pipeline includes glare and output CCTF encoding.

See [scanning](scanning.md) for scan polarity and negative conversion.

## Spectral methods and illuminants

The experimental spectral registry ships these methods:

- `hanatos2025`
- `mallett2019`
- `arctic2026alpha02`
- `arctic2026beta04`
- `gauss-lasers`
- `jakob2019`
- `otsu2018`

Runtime parameters can select a method and an illuminant. The supported illuminants are `A`, `D50`, `D55`, `D60`, `D65`, `D75`, `E`, `T`, `TH-KG3`, `TH-KG3-L`, and `K75P`. `BB<temperature>` selects a temperature in the supported 1667–25000 K range.

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
