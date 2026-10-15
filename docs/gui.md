# GUI workflows

This page explains the desktop GUI controls, rendering actions, viewer, and saved state. It distinguishes retained Preview and Scan output from independent Export work.

## Launch

```bash
./target/release/spektrafilm-gui [optional/path/to/image.orf]
```

**Launch state** loads a deterministic state for repeatable sessions:

```bash
spektrafilm-gui IMAGE --state GUI_STATE.json
```

`SPEKTRAFILM_GUI_STATE` provides the same startup override. See [building and packaging](building.md) for the GUI build.

## Input and controls

**Open…** loads standard images or camera RAW. RAW processing uses LibRaw. It exposes as-shot/daylight/tungsten/custom Kelvin+tint white balance and Lensfun correction. RAW enters the runtime as linear ACES2065-1. See [RAW input](raw.md) for white balance details.

**Input image / Profiles** selects the input/output color workflow, film stock, and print paper. Selecting a film keeps the current paper; applying a look preset selects the paper that the look resolves.

**Numeric fields** accept typed exposure, film format, halation, DIR couplers, grain, glare, scanner, enlarger, and output values. Click a field to type with the text cursor. Scrolling over a field changes its value by the control's step. Scrolling elsewhere moves the sidebar. Valid edits follow Auto preview. Incomplete or invalid text keeps the last valid parameter value. The text resets when focus leaves the field. The editor range and its relation to the accepted document range follow [numeric editor ranges](../specs/gui-workflow/spec.md#numeric-editor-ranges).

Pages and their section assignment follow [page ownership](../specs/gui-workflow/spec.md#page-ownership).

The GUI shows scanner controls for the scanned medium:

- Paper and slides expose white/black correction.
- Slides and negatives expose the receiver profile.
- Negatives expose scan polarity; negative conversion controls appear on `input > film > scan` and `input > convert-film > scan`.
- Slides and negative conversion outside Digital Reference expose film-scan points.

Selecting the negative conversion method clears incompatible capture-correction flags. State files and scanner section presets retain point values. See [scanning](scanning.md) for scanner mode semantics.

**MAIN → Output → Simulation blend** blends the complete result with the decoded, geometrically aligned source in linear output RGB. Amount `1` keeps the complete result. Amount `0` returns the prepared source before virtual camera exposure. A negative capture stays negative at zero. Preview, Scan, Export, and complete source-to-output LUTs share this operation. Segmented LUTs require amount `1`. See [the output mix specification](../specs/output-mix/spec.md#output-mix) and [the GUI workflow specification](../specs/gui-workflow/spec.md#material-controls-and-output-composition) for the control semantics.

**MAIN → Enlarger → Print exposure mode** selects `auto` or `manual`. `auto` normalizes the print exposure and follows camera exposure compensation, like a color analyzer or minilab. `manual` applies the print exposure time alone, like a test strip. **MAIN → Enlarger → Preflash** retains independent Yellow/Magenta flash filtration. Exposure is a linear multiplier, not EV. On photographic paper, it can change weak highlight differences, but it may lower white, add fog, and shift color. It cannot recover absent detail. Routes without printing retain these values without applying them. See [the output mix specification](../specs/output-mix/spec.md#preflash) and [the GUI workflow specification](../specs/gui-workflow/spec.md#material-controls-and-output-composition) for the control semantics.

The GUI has no standalone Output lightness-compression control. See [presets](presets.md) for the versioned state document rule.

<<<<<<< HEAD

The **Magazine finish** checkbox next to the workflow selector applies the bundled appearance to the selected workflow. **ADVANCED → Magazine finish color** sets Strength from 0 to 1. Strength 0 is a no-op. Photographic print controls remain separate.
=======
### Magazine finish
The **Magazine finish** checkbox next to the workflow selector applies the bundled appearance to the selected workflow. **PRINT → Magazine finish color** sets Strength from 0 to 1. Strength 0 is a no-op. Photographic print controls remain separate.

The optional finishing treatment applies a bundled RGB appearance to any supported base workflow, including a scanned negative or a finished RGB input. It runs after the base workflow and its scanner-domain effects. It adds warm paper/highlights, compressed print contrast, cyan-biased shadows, and smooth chroma compression.
>>>>>>> origin/main-0.3.4

`magazine_print_color.active` and one appearance Strength from 0 to 1 enable the treatment independently of the base workflow. There is no magazine route. The treatment remains RGB-only. It does not generate CMYK or halftone output.

The `Finished RGB` workflow color-manages an already rendered image to the output space. Film and print profile selections do not affect it. See the [film-chain specification](../specs/film-chain/spec.md) for the workflow contract.

## Preview and Scan

**Preview** renders a reduced image using the configured preview long-edge limit and preview digestion. Explicitly update it when auto-preview is disabled. Crop, spectral adaptation/blur, UV/IR, layered grain, and both diffusion filters apply to the runtime.

New edits replace queued work. Stale results cannot replace the latest output. The viewer retains its last frame until the current frame is ready. Automatic parameter updates skip the development animation; explicit Preview and Scan retain it.

**Scan** renders the full-resolution input with spatial and stochastic effects. It prefers GPU execution with CPU fallback. The resulting full-resolution output is retained for Save and inspection in the viewport.

**Scan-for-print** temporarily enables scanner white/black corrections and disables print glare. Toggle it again to restore all three previous values. Loading state or changing profiles clears the transient snapshot.

The current optical model moves scanner lens blur to linear readout light. It runs before interpretation, points, correction, gamut, sharpening, and transfer encoding. Print viewing glare follows optical blur and does not expose or develop the print material. Nonzero blur can intentionally change historical direct-scan pixels. Recipes and reports identify the current model revision. Corrected scans use faithful per-stage execution when resident shaders cannot preserve that order. Preflash retains both exposure gains and floors. Resident printing applies its outer gain to flash as well as main exposure. Film grain stays in the material domain. Procedural grain stays a post-scan appearance. See [the output mix specification](../specs/output-mix/spec.md) and [the GUI workflow specification](../specs/gui-workflow/spec.md) for the owning semantics.

See the [GUI workflow specification](../specs/gui-workflow/spec.md) for the action contract.

## Viewer

**Viewer controls** include `ccw rotate` and `cw rotate`. These actions physically rotate the in-memory input used by Preview, Scan, and Export. Quarter-turn actions preserve source metadata. Save writes the retained output of the latest render.

`100%`, `200%`, and `400%` map image pixels to exact device-pixel percentages. Reset view returns to fit. Full-resolution Scan remains available in the viewport at these zoom levels. The preview long-edge limit controls Preview rendering, not Scan's available detail.

**Viewer** switches between Input / Output / Paper back from one selector in the viewer footer. Output supports nearest, linear, cubic, spline16, spline36, Lanczos, or Blackman sampling. Input retains the upstream nearest interpolation. Paper back uses spline36. The viewport samples the retained full-resolution image for visible detail.

The viewer shows a tonal-distribution readout of the retained output. It is viewer-only. Its empty state and its source follow [viewer presentation](../specs/gui-workflow/spec.md#viewer-presentation).

The canvas uses the pinned 18% gray (`#767676`) or black. It includes normalized white padding and the upstream paper watermark. Hovering reports the original floating RGB pixel.

The operation status appears in the controls footer. Its content and its single-surface rule follow [labels and surfaces](../specs/gui-workflow/spec.md#labels-and-surfaces).

### Display transform and ICC

When the display transform is enabled, every platform converts output-space pixels to encoded sRGB for the disposable viewer raster. Save and Export use their respective saving color and encoding settings. The profile selection rule and the no-profile fallback follow [viewer presentation](../specs/gui-workflow/spec.md#viewer-presentation).

On macOS, the Metal surface is tagged sRGB.

## Independent Export

**Export…** opens its own independent output settings dialog, then a file chooser for the destination. It renders a snapshot of the full input and simulation parameters independently of retained preview output.

Export uses the in-memory input pixels with the simulation parameters. It runs at the build's precision; see [parity and backends](parity.md#backend-selection). GUI Export does not launch `spektrafilm-f64`; use that CLI separately for CPU f64 reference output. Output metadata uses the current orientation and dimensions.

The single default-collapsed **MAIN → Output → Export options** area opens the independent settings dialog or cancels its active worker. Import RGB/Raw contain only import controls. Bottom SAVE uses the chosen filename extension independently of Export preferences.

Export's saving color space and transfer encoding are independent of Save's current simulation saving settings. JPEG exposes quality (1–100) and 4:4:4/4:2:0 chroma sampling. TIFF/EXR expose their supported depth/compression settings. See the [CLI image output](cli.md#image-output) and [photo-export specification](../specs/photo-export/spec.md) for supported output combinations.

The status bar shows elapsed time. **Cancel** discards the result before publication. Cancellation may wait for active computation or writing.

### JPEG Export

Explicit JPEG Export uses the chosen quality and 4:4:4 or 4:2:0 chroma subsampling. Save leaves JPEG quality/subsampling to upstream OpenImageIO defaults.

The GUI warns that JPEG remains lossy even at quality 100. 4:2:0 can soften colored grain. Use TIFF or EXR when comparing grain detail without JPEG compression. Quality 100 and 4:4:4 do not make JPEG lossless.

## Save retained output

**Save…** saves the latest retained floating output without rerendering. It uses the current simulation saving color space and transfer encoding. The filename extension selects the format. `.jpg` is the default.

- JPEG/PNG use 8-bit integer samples.
- TIFF uses 16-bit integer samples with ZIP compression.
- OpenEXR uses 16-bit half samples.

Save uses upstream OpenImageIO JPEG defaults rather than Export's quality/subsampling settings. Neither TIFF nor EXR forces ACES or linear encoding. Viewer borders, watermark, and display ICC transforms stay out of saved pixels. See the [shared writer contract](cli.md) for output format details.

## State and defaults

**Save state… / Load state…** exchange the pinned Python 0.3.4 GUI JSON sections. Partial files merge into factory values. Legacy input aliases and nested sections normalize to the flat upstream format. Invalid JSON, field types, selections, or Rust extension versions appear in the status bar.

**Save current as default** persists the current controls. **Restore factory default** restores Kodak Gold 200 + Kodak Supra Endura, RMS granularity `[5,5,5]`, and upscale factor `1.0`. Saved user values, including zero RMS, survive startup and state loading.

Startup files use the platform configuration directory. `SPEKTRAFILM_CONFIG_DIR` overrides it. Rust settings use `rust.version = 2`. File-dialog directories persist separately.

Legacy states without an explicit export format preserve 8-bit as PNG and 16/32-bit as TIFF before defaults and validation. Recipe values and viewer state remain intact. Explicitly incompatible format/depth combinations remain errors.

### Preset browser state

The GUI remembers the preset browser selection and layout independently of startup controls. An ordinary restart restores that metadata and recomputes the applied, modified, or missing marker. It does not apply the preset again. Loading an explicit GUI state uses that file's browser metadata. Use **Save current as default** to change startup controls.

A failed preset application preserves the current controls and temporary white/black correction state. See [presets](presets.md) for library and composition rules. The [presets specification](../specs/presets/spec.md) owns the preset contract.

## Diagnostics

**Diagnostics** in CONFIG selects Off, Summary, or GPU timing for subsequent operations. Changing the mode leaves running operations on their captured mode. The session keeps the latest 20 collected operation reports. Select a report to inspect its phases, stages, GPU work, timing availability, and outcome.

**Save report** writes local JSON without rendering. Save reports identify the retained render that supplied their pixels. This includes renders made with collection off or removed from history.

Launching with `SPEKTRAFILM_GUI_DIAGNOSTICS_DIR` set to an existing directory publishes every completed operation's report there as a new `operation-<id>.json`. It selects summary collection for the session, as a CLI report request does. See the [telemetry specification](../specs/telemetry/spec.md) for report semantics.

## Session configuration

The CONFIG page holds the Diagnostics section, startup state management, and the viewer presentation settings. See [page ownership](../specs/gui-workflow/spec.md#page-ownership).

The viewer interpolation mode, canvas, white padding, and display transform affect the disposable viewer raster. See [Viewer](#viewer).

## See also

- [Documentation index](README.md)
- [Building and packaging](building.md)
- [RAW input](raw.md)
- [CLI and shared output writer](cli.md)
- [Scanning](scanning.md)
- [Presets](presets.md)
- [GUI workflow specification](../specs/gui-workflow/spec.md)
- [Photo-export specification](../specs/photo-export/spec.md)
- [Presets specification](../specs/presets/spec.md)
- [Film-chain specification](../specs/film-chain/spec.md)
- [Telemetry specification](../specs/telemetry/spec.md)
