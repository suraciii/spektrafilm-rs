# Magazine print color design

- The feature owns one bundled RGB-to-RGB appearance definition for the first product version.
- The feature is separate from photographic print exposure and development. It does not reuse photographic print parameters as a CMYK model.
- The film chain supplies output-referred film-scan RGB with explicit primaries, white point, and transfer state.
- The implementation may use output-referred linear RGB, XYZ, or a generated LUT internally. Every conversion at the feature boundary must be explicit.
- The runtime image remains three-channel RGB. Offline ICC or CMYK processing may inform an appearance asset, but it must not create a four-channel runtime contract.
- The feature owns paper white, black response, print tone, gamut compression, and paper tint. Film stages own exposure, emulsion, development, and film-domain effects.
- The feature applies color operations only. Spatial print effects belong to a later, separate stage.
- The feature must not apply an implicit or second transfer encoding. The caller owns the selected output encoding.
- The first product version does not expose a general print-condition, ICC-profile, or press-output API.
