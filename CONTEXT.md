# Film simulation

SpektraFilm models how a digital photograph forms a film image and how that image becomes a digital output or a photographic print.

## Language

Real operations of exposure, development, photographic printing, and film digitization own the unqualified vocabulary of this language. A modeled convention must carry a distinguishing qualifier and must not occupy an unqualified term of a real operation. The product models the real operational workflow first; a modeled convention extends it.

**Record exposure**: The exposure received by one film or photographic-paper record before development. In a chromogenic material, cyan, magenta, and yellow identify the dye-forming records, not RGB image colors or developed record densities.
_Avoid_: Receiver capture, record density

**Record density**: The optical density formed in one developed record of a film or a photographic paper. It describes the developed material, before a scan interprets it.
_Avoid_: Scanned density, scan capture

**Scanned medium**: The developed film or photographic paper read for digital output. Its native polarity is independent of the delivered image's polarity.

**Scanner lamp**: The single physical light source of a scan. Its accepted values are the illuminant vocabulary the enlarger lamp uses: `D50`, `D55`, `D65`, `T`, `TH-KG3`, `TH-KG3-L`, `K75P`, and a `BB<kelvin>` blackbody token. `D55` is the default.
_Avoid_: Scanner look, output color space, scan illuminant

**Scan sensor**: The colorimetric spectral response that acquires transmitted or reflected light from a scanned medium. The scanner has one sensor, modeled on the CIE 1931 color-matching functions; it is not selectable.
_Avoid_: Scanner look, output color space, receiver profile

**Scan capture**: The RGB readings the scan sensor acquires from a scanned medium. A negative conversion reads record densities instead of a scan capture.
_Avoid_: Scan output, scanned medium

**Clear film base**: The unexposed base of the scanned medium. It is the white reference of a scan: a clear area reads as the output color space's white.
_Avoid_: Scanner white and black correction, output levels

**Scanned density**: The density derived from one channel of a scan capture, relative to that channel's clear reference.
_Avoid_: Record density, scan capture

**Monochrome**: A film stock that forms one silver image. It uses the silver material law whether its profile is negative or positive. A chromogenic monochrome stock forms dye records and requires an explicit dye material law.
_Avoid_: Black and white correction, scanner white and black correction

**Negative conversion**: The interpretation of a developed negative as a positive image. It reads each dye record's density through the selected film's response and applies a positive tone response from one logistic curve family. It has one interpretation and no selectable method.
_Avoid_: Positive scan, scanner finish

**Exposure reconstruction**: The negative conversion that uses the selected film's response to estimate relative exposure from record densities.
_Avoid_: Reference conversion

**Conversion style**: The named stylization of the negative-to-positive tone. `neutral` is the frozen reference constants of that tone curve and the only built-in style; any other name is user-authored declared data in `data/scan-styles/<name>.json` carrying one brightness, contrast, and maximum-density triple per channel. A style parameterizes the constants of the same curve family. It is never fitted from image or chart data, and it is inert on a direct scan.
_Avoid_: Scanner finish, receiver profile, chart fit, calibration

**Digital output**: The color space, transfer encoding, geometry, and file representation used to deliver an image.
_Avoid_: Scanned medium, negative conversion

**CIELAB**: A three-coordinate color representation under a declared reference white and observer. L* describes lightness; a* and b* describe opponent color directions. These coordinates describe color appearance, not film or photographic-paper record exposures or densities.
_Avoid_: Lab Scan, film record channels

**Color center**: A representative color of a declared sample population, calculated with a named estimator. A median of Lab channels and Lab converted from mean linear RGB are different color centers. Neither establishes that a reference is suitable for fitting or acceptance.

**Prepared image**: A raster image explicitly prepared outside the benchmark and declared in a review manifest. Its color interpretation is declared rather than inferred; preparation does not establish measured color values, exposure equivalence, or original framing.
_Avoid_: Prepared chart image

**Set**: A declared group of prepared images inside one review manifest, with either a shared patch grid or explicit per-image regions. A shared set does not establish the same spectral stimulus or pixel-level registration.
_Avoid_: Chart group

**Patch window**: The declared rectangular area of one grid patch used for visual inspection. It may cover the patch or a smaller central area, but it does not include an inferred correspondence to another set.

**Feature profile**: A named, reusable parameter set for one film-chain feature. It resolves only that feature's editable controls and does not select film or print materials, change workflow, or represent a complete look.

**Feature profile family**: The named alternatives for one feature, including the feature's `custom` state when the feature exposes editable overrides. A family owns one parameter schema and one resolver; its entries do not become film profiles or look presets.

**Feature profile application**: The operation that resolves one feature profile and replaces exactly that feature's controls. It preserves every control outside the feature and fails transactionally when the resolved values are invalid or incompatible with the active route.
