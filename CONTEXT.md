# Film simulation

SpektraFilm models how a digital photograph forms a film image and how that image becomes a digital output or a photographic print.

## Language

Real operations of exposure, development, photographic printing, and film digitization own the unqualified vocabulary of this language. A modeled convention must carry a distinguishing qualifier and must not occupy an unqualified term of a real operation. The product models the real operational workflow first; a modeled convention extends it.

**Record exposure**: The exposure received by one film or photographic-paper record before development. In a chromogenic material, cyan, magenta, and yellow identify the dye-forming records, not RGB image colors or developed record densities.
_Avoid_: Receiver capture, record density

**Record density**: The optical density formed in one developed record of a film or a photographic paper. It describes the developed material, before an output receiver interprets it.
_Avoid_: Scanned density, receiver capture

**Scanned medium**: The developed film or photographic paper read for digital output. Its native polarity is independent of the delivered image's polarity.

**Scan receiver**: The spectral response and reference readings used to acquire transmitted or reflected light from a scanned medium.
_Avoid_: Scanner look, output color space

**Receiver capture**: The RGB readings a scan receiver acquires from a scanned medium. A negative conversion may interpret a receiver capture instead of record densities.
_Avoid_: Scan output, scanned medium

**Scanned density**: The density derived from one channel of a scan capture, relative to that channel's clear reference. A slide scan reads its point range on scanned densities.
_Avoid_: Record density, receiver capture

**Monochrome**: A film stock that forms one silver image. It uses the silver material law whether its profile is negative or positive. A chromogenic monochrome stock forms dye records and requires an explicit dye material law.
_Avoid_: Black and white correction, scanner white and black correction

**Negative conversion**: The interpretation of a developed negative as a positive image. It includes a color interpretation and a base positive tone response.
_Avoid_: Positive scan, scanner finish

**Exposure reconstruction**: A negative conversion that uses the selected film's response to estimate relative exposure from record densities or receiver measurements.
_Avoid_: Reference conversion

**Reference conversion**: A negative conversion that uses a common receiver and positive response instead of reconstructing exposure through the selected film's characteristic curves.
_Avoid_: Unprocessed film color, physical ground truth

**Digital Reference**: The Scan output treatment that uses reference conversion to present simulated film under a common digital output model. The model is a reference convention, not a measured scanner.

**Reference Exposure**: A fixed placement of the reference conversion's tone response. Its exposure and tone placement do not depend on the photograph's content.

**Image correction**: A brightness, color balance, or contrast adjustment to the delivered image. It may be manual or derived from the photograph.

**Automatic scan setup**: Image correction derived from a photograph's measurements for its digital delivery.
_Avoid_: Receiver calibration, film development

**Scanner finishing**: An optional color and tone appearance applied after negative conversion.

_Avoid_: Negative conversion, scan receiver

**CMYK ink style**: A modeled RGB appearance that suggests a fixed ink-on-paper grade. It runs after scanner interpretation, scanner finishing, output gamut processing, unsharp masking, transfer encoding, and Procedural grain. It remains three-channel RGB and does not produce CMYK separations, press-ready output, or an ICC press workflow.

_Avoid_: Magazine finish, Printing, photographic paper, press proof, physical print

**Lab Scan**: A modeled minilab-style digital delivery of a simulated negative. Receiver acquisition, conversion, automatic scan setup, and scanner finishing are separate parts of that delivery.
_Avoid_: Measured Frontier profile, measured Noritsu profile

**Scanner white and black correction**: The endpoint correction of a positive scan. A luminance remap maps the scanned medium's white and black reference luminances onto target white and black levels, with the matched film or print exposure correction. It applies to a photographic-paper or slide film scan, not a negative film scan.
_Avoid_: Black and white correction, monochrome

**Film-scan black and white points**: The per-channel window positions on a scanned film's point range: the relative log-exposure range of the derived inverse for a negative conversion, and the shared scanned-density range for a slide film scan. Points remap linearly without clipping.
_Avoid_: Scanner white and black correction, output levels

**Digital output**: The color space, transfer encoding, geometry, and file representation used to deliver the image.
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
