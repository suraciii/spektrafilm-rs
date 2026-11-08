# Attribution and licenses

This page records the sources credited by the Rust port and the licenses for the program and its assets. It also records the source revisions for the optional Fotufilm adaptations.

## Credits

Andrea Volpato wrote the original Python implementation: [andreavolpato/spektrafilm](https://github.com/andreavolpato/spektrafilm).
The original spectral data, film/paper profiles, and pipeline architecture come from that project.
Later source reconstructions carry their own attribution notices.
The [Polymax fixed-condition profiles](polymax.md) use Kodak G-24 vector curves,
with a separate [source and reconstruction notice](../data/license/POLYMAX.txt).

The spectral-upsampling LUT (`hanatos2025_*`) is named after [Johannes Hanatos](https://github.com/hanatos), author of [vkdt](https://github.com/hanatos/vkdt).
Hanatos provided the upstream Python project with the LUT files and sample code that drive the RGB → spectrum step.
The bundled `kodak_kodachrome_64` profile carries density curves rebuilt from the
Kodak E-88 KODACHROME 64 publication (digitized characteristic curves, anchored at
the profile's own midscale density, per-channel minima at zero per the bundled-profile
convention; the pre-E-88 upstream sublayer decomposition is retained under the E-88
composite (the research-validated pairing — no E-88 sublayer data exists), and
the upstream fitted curve model was removed so the sampled curves are authoritative).
The modification is noted in the profile's `datasource` field per the asset license.

PCHIP 3D LUT, MT19937 binomial sampler, and CIE 1931 observer constants are ported from numpy / scipy / scikit-image / colour-science.

Claude Code assisted the port.

## Program and asset licenses

The Rust program is GPL-3.0; see [LICENSE](../LICENSE).
Andrea Volpato's spectral profiles and their LUT derivatives retain CC BY-SA 4.0; see [the canonical asset license](../data/license/SPEKTRAFILM_LICENSE.txt).

## GUI Chinese font

The GUI embeds unmodified Noto Sans SC Regular (Version 2.004) from [Noto CJK](https://github.com/notofonts/noto-cjk). The font is © 2014–2021 Adobe and distributed under the SIL Open Font License 1.1. The [source notice](../crates/spektrafilm-gui/assets/NotoSansSC-NOTICE.txt) records its upstream blob; the [license](../crates/spektrafilm-gui/assets/NotoSansSC-OFL.txt) ships with native packages. It supplies offline Chinese and mixed-script text without requiring system font installation.

## Fotufilm Film grain engine

The optional Fotufilm Film grain engine adapts the crystal-population and periodic-aperture mechanism of Fotufilm project-authored `Sources/FotufilmCore/FilmGrain.swift` under Apache-2.0.
The source is pinned at the same revision `59d43471dbef45c85ae442e507a291fdebfb2ec5`.
It is an authored appearance model.
The resolved spectral film profile remains the authority for developed color and tone.
Its grain parameters are explicitly authored effective values rather than measured stock data.

## See also

- [Building and packaging](building.md) for third-party library license facts.
- [Scanning](scanning.md).
- [Grain](grain.md).
- [LUTs](luts.md) for LUT bundle attribution details.
- [Fotufilm grain specification](../specs/film-grain/fotufilm/spec.md).

## Kodak Tri-X 400 / 400TX

The still-negative [Tri-X 400 profile and built-in look](trix400.md) use a direct
reconstruction of Kodak F-4017's 35 mm T-MAX six-minute curve. The
[attribution notice](../data/license/TRIX_400.txt) separates new contributions
from Kodak's original technical material and describes the cross-check sources.

## Fujifilm Astia 100F

The [Astia 100F profile and built-in look](astia100f.md) reconstruct the
AF3-149E RAP100F publication curves with an inferred clear-film spectrum.
The [source notice](../data/license/ASTIA_100F.txt) distinguishes new contributions,
the MIT-licensed Status A table transcription and Fujifilm's original material.

## Fujifilm Neopan Acros 100 II

The [Acros 100 II profile and built-in look](acros100ii.md) reconstruct the
AF3-0258E Microfine 10-minute characteristic and relative log spectral curves
from 300 dpi publication plots. The [source notice](../data/license/ACROS_100_II.txt)
distinguishes new coordinate-extraction contributions from Fujifilm's original
technical material. The plots are raster images, not vector centerlines.

## Kodak T-MAX 100 / 100TMX

The [T-MAX 100 profile and built-in look](tmax100.md) reconstruct the F-4016
D-76 20 C 7.5-minute characteristic curve and the 0.3-above-D-min log
sensitivity curve from page-8 vector paths. The
[source notice](../data/license/TMAX_100.txt) distinguishes new coordinate
extraction and reconstruction contributions from Kodak's original technical
material. The 1.0-above-D-min curve is retained for an ablation, not as a
second layer.
