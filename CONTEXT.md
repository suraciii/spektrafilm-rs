# Film simulation

SpektraFilm models how a digital photograph forms a film image and how that image becomes a digital output or a photographic print.

## Language

Real operations of exposure, development, photographic printing, and film digitization own the unqualified vocabulary of this language. A modeled convention must carry a distinguishing qualifier and must not occupy an unqualified term of a real operation. The product models the real operational workflow first; a modeled convention extends it.

**Record exposure**: The exposure received by one film or photographic-paper record before development. In a chromogenic material, cyan, magenta, and yellow identify the dye-forming records, not RGB image colors or developed record densities.
_Avoid_: Receiver capture, record density

**Record density**: The optical density formed in one developed record of a film or a photographic paper. It describes the developed material, before a scan interprets it.
_Avoid_: Scanned density, scan capture

**Simulated material state**: The completed state of a modeled film or print material, with the material interpretation and physical context needed for later operations. Continuing from this state does not repeat the operations that formed it.
_Avoid_: Debug dump, scan capture

**Scanned medium**: The developed film or photographic paper read for digital output. Its native polarity is independent of the delivered image's polarity.

**Scan workflow**: Reading a developed film or photographic print for a digital image, with interpretation and image adjustments appropriate to the delivered polarity. It includes optical readout and digital operations without treating them as the same physical process.
_Avoid_: Scan capture alone

**Scanner lamp**: The single physical light source of a scan. Its accepted values are the illuminant vocabulary the enlarger lamp uses: `D50`, `D55`, `D65`, `T`, `TH-KG3`, `TH-KG3-L`, `K75P`, and a `BB<kelvin>` blackbody token. `D55` is the default.
_Avoid_: Scanner rendering, output color space, scan illuminant

**Scan sensor**: The colorimetric spectral response that acquires transmitted or reflected light from a scanned medium. The scanner has one sensor, modeled on the CIE 1931 color-matching functions; it is not selectable.
_Avoid_: Scanner rendering, output color space, receiver profile

**Scan capture**: The RGB readings the scan sensor acquires from a scanned medium. A negative conversion reads record densities instead of a scan capture.
_Avoid_: Scan output, scanned medium

**Clear film base**: The unexposed base of the scanned medium. It is the default white reference of a scan: a clear area reads as the output color space's white. The explicit readout-illuminant white-reference mode opts out of that calibration.
_Avoid_: Scanner white and black correction, output levels

**Scanned density**: The density derived from one channel of a scan capture, relative to that channel's clear reference.
_Avoid_: Record density, scan capture

**Monochrome**: A film material represented by one image-forming record. Monochrome does not identify whether the image consists of metallic silver or chromogenic dyes.
_Avoid_: Black and white correction, scanner white and black correction

**Negative conversion**: The interpretation of a developed negative as a positive image. It reads each dye record's density through the selected film's response and applies a positive tone response from one logistic curve family. It has one interpretation and no selectable method.
_Avoid_: Positive scan, scanner finish

**Exposure reconstruction**: The negative conversion that uses the selected film's response to estimate relative exposure from record densities.
_Avoid_: Reference conversion

**Conversion style**: The named positive rendering of a developed negative. The built-in `standard` combines automatic frame setup with a declared positive tone; a fixed style uses a declared tone without automatic frame setup.
_Avoid_: Scanner finish, receiver profile, chart fit, calibration

**Automatic frame setup**: The global exposure and color judgment derived from one complete developed frame. It is separate from exposure reconstruction and operator correction, not a calibrated response of a particular scanner.
_Avoid_: Capture exposure, operator correction, scanner calibration

**Operator correction**: The user's global finished-positive exposure, warmth, green–magenta balance, and common-exposure contrast judgment over a frame's automatic result. It does not change the developed negative or the automatic judgment.
_Avoid_: Capture white balance, material gamma, scanner calibration

**Tone and color adjustments**: The user's brightness, contrast, warmth, saturation, and input-range adjustments to a completed positive image, independent of the material and readout route that formed it. They do not describe material exposure, development, scanner calibration, or negative-conversion operator correction.
_Avoid_: Common finishing, scanner finish, material response, color density

**Digital output**: The color space, transfer encoding, geometry, and file representation used to deliver an image.
_Avoid_: Scanned medium, negative conversion

**Original**: The loaded photograph under its accepted input interpretation and orientation, before film simulation. It is not the scanner's automatic result.

**Modified**: The photograph produced by the latest accepted Preview or Scan. It can remain visible after later adjustments make that result stale.

**CIELAB**: A three-coordinate color representation under a declared reference white and observer. L* describes lightness; a* and b* describe opponent color directions. These coordinates describe color appearance, not film or photographic-paper record exposures or densities.
_Avoid_: Lab Scan, film record channels

**Color center**: A representative color of a declared sample population, calculated with a named estimator. A median of Lab channels and Lab converted from mean linear RGB are different color centers. Neither establishes that a reference is suitable for fitting or acceptance.

**Prepared image**: A raster image explicitly prepared outside the benchmark and declared in a review manifest. Its color interpretation is declared rather than inferred; preparation does not establish measured color values, exposure equivalence, or original framing.
_Avoid_: Prepared chart image

**Set**: A declared group of prepared images inside one review manifest, with either a shared patch grid or explicit per-image regions. A shared set does not establish the same spectral stimulus or pixel-level registration.
_Avoid_: Chart group

**Patch window**: The declared rectangular area of one grid patch used for visual inspection. It may cover the patch or a smaller central area, but it does not include an inferred correspondence to another set.

**Render mode**: The exclusive processing path selected from Film, LUT, or Off. It selects which stages execute; inactive stages retain stored values.
_Avoid_: Look, color source, page

**Film route**: The spectral stage sequence inside Film mode, such as `input > film > print > scan`. It is serialized as `workflow.route` and is inactive outside Film mode.
_Avoid_: Workflow mode, render workflow

**Film preset**: A named, reusable choice of film and print materials with their preset-owned adjustments. It is a starting point for a photograph, not a saved photograph or a complete application setup.
_Avoid_: Photograph record, startup state, LUT

**LUT transform**: A non-spectral RGB color transform and its independent Effects layers, applied without film or print material simulation. It is not a film profile and not a preset by itself.
_Avoid_: Film profile, film preset, LUT delivery

**LUT preset**: A named, user-owned LUT transform with an external LUT file path and content hash, declaration, Effects, and optional LUT Grade. It selects LUT mode and carries no film or print profile references.
_Avoid_: Film preset, film profile, embedded LUT

**Effects preset**: A named, user-owned Off-mode preset carrying Diffusion, Halation, and Grain. It carries no LUT declaration, LUT Grade, or film or print profile reference.
_Avoid_: Film preset, LUT preset, Effects mode

**Exported LUT**: A `.cube` file produced from a spectral film rendering. It is a file, not an active mode, an imported LUT, or a preset.
_Avoid_: Film profile, film preset, measured stock

**LUT workflow**: The product workflow that exports a LUT file, imports a LUT file for use, saves and recalls LUT presets, and renders a LUT transform. It is separate from the Film render mode.
_Avoid_: Film workflow, spectral simulation

**LUT mode**: The render mode that applies a LUT transform. It carries no material or illuminant simulation.
_Avoid_: Film workflow, spectral simulation

**Off mode**: The render mode that applies neither Film nor LUT. It runs the Effects stage and output color management.
_Avoid_: Effects mode, input workflow

**LUT Grade**: The optional Tone & Color response applied after an external LUT. It uses independent LUT-stage values and is not Scan Tone & Color or a second grading response.
_Avoid_: Film grade, baked LUT grade

**LUT stage**: The inspector stage that owns the LUT declaration, strength, import actions, and LUT Grade. It is not a physical film-chain stage.
_Avoid_: Film density, camera profile

**Effects stage**: The inspector stage that owns Diffusion, Halation, and Grain for LUT and Off. It is not a film material stage or a render mode.
_Avoid_: Global effects, film grain, LUT mode

**Preset thumbnail**: A small rendering of the current photograph with a candidate preset, used to compare presets without changing accepted adjustments. It is not a reference sample or the retained Preview result.
_Avoid_: Reference thumbnail, saved preset image

**Feature profile**: A named, reusable parameter set for one film-chain feature. It resolves only that feature's editable controls and does not select film or print materials, change mode, or represent a complete preset.

**Feature profile family**: The named alternatives for one feature, including the feature's `custom` state when the feature exposes editable overrides. A family owns one parameter schema and one resolver; its entries do not become film profiles or presets.

**Feature profile application**: The operation that resolves one feature profile and replaces exactly that feature's controls. It preserves every control outside the feature and fails transactionally when the resolved values are invalid or incompatible with the active route.

## Grain engines

**Spektra**: The upstream density-domain particle grain model.
_Avoid_: Grain V1

**Procedural**: The Dehancer-derived encoded-RGB grain model with Analogue and Noise modes.
_Avoid_: Grain V2

**Exposure Grain**: The reconstructed post-scan encoded-RGB value-noise grain appearance model.

**Dye**: The self-developed film-coordinate dye-field grain model.
_Avoid_: Grain V3

**Fotufilm Film**: The independent appearance population model fitted to resolved film curves.
