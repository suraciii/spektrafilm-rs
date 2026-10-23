# Attribution and licenses

This page records the sources credited by the Rust port and the licenses for the program and its assets. It also records the source revisions for the optional Fotufilm adaptations.

## Credits

Andrea Volpato wrote the original Python implementation: [andreavolpato/spektrafilm](https://github.com/andreavolpato/spektrafilm).
All spectral data, film/paper profiles, and pipeline architecture come from that project.

The spectral-upsampling LUT (`hanatos2025_*`) is named after [Johannes Hanatos](https://github.com/hanatos), author of [vkdt](https://github.com/hanatos/vkdt).
Hanatos provided the upstream Python project with the LUT files and sample code that drive the RGB → spectrum step.

PCHIP 3D LUT, MT19937 binomial sampler, and CIE 1931 observer constants are ported from numpy / scipy / scikit-image / colour-science.

Claude Code assisted the port.

## Program and asset licenses

The Rust program is GPL-3.0; see [LICENSE](../LICENSE).
Andrea Volpato's spectral profiles and their LUT derivatives retain CC BY-SA 4.0; see [the canonical asset license](../data/license/SPEKTRAFILM_LICENSE.txt).

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
