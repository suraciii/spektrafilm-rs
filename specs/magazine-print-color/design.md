# Magazine print color design

## Ownership and entry points

- Base workflow and finishing enablement remain independent in the runtime parameter model. No general effect graph or new workflow syntax is needed.
- `magazine_print_color.active` and `magazine_print_color.strength` retain their roles as the color treatment's enablement and blend weight. Callers must not require a magazine route to activate them.
- The shared runtime owns one finishing implementation for Finished RGB and scanned outputs. The GUI and CLI must not implement separate color transforms.
- Finishing runs after base-output color conversion, optical effects, and existing scanner-domain grain. It runs before image writing and must preserve the declared transfer state.
- Finished RGB must not use film, paper, spectral preparation, or neutral print filters. Profile resolution still happens when the runtime is constructed because the pipeline is profile-typed; the output must not depend on which profiles were resolved. Its source and destination color definitions remain required.
- The finished input must have explicit primaries, white point, and transfer state. A linear ProPhoto image remains finished RGB when the user selects that base workflow; it must not be reinterpreted as scene exposure.
- The input loader remains responsible for supported file color metadata and explicit source overrides. Magazine print must not infer the input color space from pixel values.
- Film stages own exposure, emulsion, development, and film-domain effects. Magazine color owns the final tone and chroma treatment. It must not use photographic paper controls as a CMYK model.
- Spatial print appearance remains outside `magazine_print_color`. A color LUT must not contain halftone or paper-texture behavior.

## Selected color definition

The full-strength definition uses CIELAB with the D50 white point. Source primaries and transfer state must be converted explicitly to this working space. The study's LittleCMS RGB-to-Lab and Lab-to-RGB conversions define the reference conversion, not a required runtime dependency.

For lightness L, chroma C, and hue h in degrees, define `smooth(x)` as `t*t*(3-2*t)` with `t = clamp(x, 0, 1)`. Define `hue(center, width)` as a Gaussian of the shortest signed hue distance with the supplied angular width.

The monotonic tone curve uses input knots `[0, 5, 10, 20, 35, 50, 65, 80, 92, 100]` and output knots `[0, 3.5, 8, 22, 40, 56, 70, 83, 93, 99]`. It uses cubic Hermite interpolation, endpoint secant slopes, and harmonic-mean interior slopes. Evaluation outside the knot domain uses the nearest endpoint.

The shared color weights are:

- `chroma_weight = smooth((C - 20) / 28)`.
- `mid_weight = smooth((L - 8) / 15) * (1 - smooth((L - 78) / 18))`.
- `red = hue(35, 25) * chroma_weight * mid_weight`.
- `blue = hue(-65, 29) * chroma_weight * mid_weight`.
- `green = hue(140, 30) * chroma_weight * mid_weight`.

Mapped lightness is `tone(L) - 4*red - 3.5*blue`. Both Lab chromatic coordinates use the scale `1 + 0.22*red + 0.10*blue - 0.04*green`. Near-neutral highlight weight is `smooth((L - 75) / 25) * (1 - smooth(C / 18))`; it adds `0.12*weight` to a and `0.7*weight` to b.

Full-strength output uses radial chroma reduction at fixed mapped lightness and hue to fit the selected destination RGB gamut. It must not hard-clip chromatic channels as its gamut-mapping method. The study used 13 bisection steps; a production optimization must meet a measured reference budget.

The study compared bounded display samples. Production headroom handling must be explicit at the appearance boundary and covered by finite and out-of-range probes; the study's display clipping is not an input-loader rule. Strength 0 must bypass all working-space conversion and gamut mapping.

## State and output

- Switching base workflows must retain Magazine settings. Enabling Magazine must not switch negative scan polarity or choose photographic paper.
- Finished RGB must not require changing the film, paper, exposure, scanner, or grain controls, and their saved values must remain intact. The UI must state that the resolved film and print profiles are unused by Finished RGB.
- The UI must distinguish a native negative-polarity scan from a positive interpretation. Magazine print must not implicitly invert a negative image.
- Preview and Export use one finishing definition. Unsupported WGPU execution must use faithful CPU processing, not a disabled effect or an old appearance.
- LUT metadata must record the base route and active finishing definition independently. Color Strength is bakeable; spatial print structure is not.

## Alternatives

A dedicated magazine workflow combines base routing and finishing intent. It excludes photographic-print outputs and needs another route for Finished RGB. Independent finishing enablement avoids those route combinations and preserves the existing base workflows.

A separate image-conversion application duplicates color management and export behavior. Finished RGB uses the existing `input` path and the shared finishing implementation instead.
