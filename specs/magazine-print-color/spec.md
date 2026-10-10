# Magazine print color

## Contract

- The feature applies a bundled magazine-style color appearance to a film-scan image.
- The film chain runs the feature after film scanning when the magazine print appearance route is selected.
- The feature accepts output-referred RGB with an explicit color space and transfer state. It must not interpret the film-scan image as scene-linear input.
- The feature preserves the input RGB primaries, dimensions, and film characteristics.
- The feature exposes an appearance strength control. The bundled definition provides the paper white, black response, print tone, gamut compression, and paper tint that form the magazine style.
- The feature outputs RGB. It does not generate CMYK output.
- The feature changes color appearance only. It does not simulate halftone grids, plate registration, total ink limits, spot colors, paper texture, or press-ready files.
- Disabling the feature leaves the film-scan result unchanged.
