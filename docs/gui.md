# GUI workflows

This page explains the desktop GUI controls, rendering actions, viewer, and saved state. It distinguishes retained Preview and Scan output from independent Export work.

The native workspace keeps searched look presets on the left, the photograph in the center, and a continuous stage inspector on the right. The [photo-first product specification](../specs/gui-product-design/spec.md) and [GUI workflow specification](../specs/gui-workflow/spec.md) define the behavior and acceptance requirements; this guide is not a native acceptance report.

## Launch

```bash
./target/release/spektrafilm-gui [optional/path/to/image.orf]
```

**Launch state** loads a deterministic state for repeatable sessions:

```bash
spektrafilm-gui IMAGE --state GUI_STATE.json
```

`SPEKTRAFILM_GUI_STATE` provides the same startup override. See [building and packaging](building.md) for the GUI build.

## Workspace

The header provides **Open**, **Undo**, **Redo**, **Settings**, and **Export**. It identifies the accepted source, film, print material, and workflow route as read-only context, and reports operation status. While an export runs, **Cancel export** replaces **Export**.

The left **Presets** browser is collapsible and resizable. **Search presets** finds looks by name or film/print stock, including stock display names. **Clear preset search** restores the full list. **Built-in** looks are read-only; **My Presets** holds user looks. **Save preset…** saves a look, not a photograph or startup default. Searching, collapsing, or scrolling the browser does not apply a look or change the photograph.

The right inspector contains one continuous list, not mutually exclusive pages. Its sticky **Search controls**, **Only modified**, and stage-jump strip remain visible while controls scroll. The jumps reveal and scroll to **Input and capture**, **Film**, **Print**, **Scan**, or **Output** in that same list; they clear the active search and Only modified filter without changing parameters. Several stages and modules can remain open together. Folding a module changes its presentation only, and ordinary edits, render completion, and route changes retain its disclosure and scroll state.

| Stage | Main modules |
| --- | --- |
| Input and capture | Workflow, Input, Import Raw, Source rotation, Crop and upscale, Camera, Camera diffusion, Input gamut compress, Spectral upsampling, Convert |
| Film | Film material, Film chemistry, Film base, Halation, Couplers, Grain |
| Print | Print material, Enlarger, Preflash, Enlarger diffusion, Print chemistry, Print base |
| Scan | Scanner, Print viewing glare |
| Output | Output, Output gamut compress, CMYK ink style |

The persistent **PREVIEW**, **SCAN**, and **SAVE** action bar stays below the inspector. **Settings** owns application/session preferences, state files and startup defaults, display configuration, and Diagnostics. Independent file-delivery options belong to the **Export options** dialog, not a photographic module.

## Input and controls

**Open** accepts supported RGB images and camera RAW files through one chooser. RAW processing uses LibRaw with as-shot/daylight/tungsten/custom Kelvin+tint white balance and Lensfun correction. RAW enters the runtime as linear ACES2065-1. See [RAW input](raw.md) for white balance details.

The viewer also accepts a dropped image file through the same import path. An unsupported or unreadable import leaves the accepted image and controls unchanged and reports the error in the header.

Choose the base route in **Input and capture → Workflow → workflow**, the film in **Film → Film material → film profile**, and the paper in **Print → Print material → print profile**. Selecting a film keeps the current paper; selecting a paper keeps the current film. Film selection applies its workflow default and scanner normalization, reported in the header. A look preset selects the material pair and its owned controls atomically. Development-time choices follow the selected material's actual family; a material without a selectable family shows its effective development time instead.

### RAW decode draft

**Input and capture → Import Raw** contains **white balance**, **temperature**, **tint**, and **lens correction**. These are decode settings, not simulated camera exposure. Editing them leaves the accepted source pixels unchanged. For a RAW source, the module reports when settings are pending and the accepted source still uses the previous settings; **Reprocess RAW** explicitly decodes the source with the draft. Successful reprocessing replaces the accepted source and starts a new adjustment history. Failure retains the accepted source and preceding history, reports the error, and leaves the pending settings visible. For an RGB source, RAW decoding is inactive and the settings are retained for the next RAW import.

Simulated exposure remains a separate control at **Input and capture → Camera → exposure compensation ev**.

### Editing, search, and reset

**Numeric fields** accept typed exposure, film format, halation, DIR couplers, grain, glare, scanner, enlarger, and output values. Click a field to type with the text cursor. Scrolling over a field changes its value by the control's step. Scrolling elsewhere moves the control panel. Valid edits follow **Settings → Session → auto preview**. Incomplete or invalid text keeps the last valid parameter value and remains visible as an editing draft until Escape or focus leaves the field. Escape cancels the uncommitted edit. The editor range and its relation to the accepted document range follow [numeric editor ranges](../specs/gui-workflow/spec.md#numeric-editor-ranges).

Each numeric row shows the parameter's name on the left and its value on the right of one line, with a slider on the line below. A modified row shows a gold marker and value; the marker uses the same section-reset baseline as the section `modified` chip. Tuple, optional, extra, and boolean rows use the same marker rule. The slider only moves the value; the field stays the row's editor. The slider is logarithmic when the declared range spans more than three decades, or when the declared minimum is zero and the declared maximum reaches one thousand or more. A signed range stays linear. A logarithmic slider anchors its smallest positive value at the parameter's declared step, so its floor matches the field's own granularity. A declared minimum of zero keeps zero as the track's leftmost stop.

A boolean row is a switch that names the concept it enables. A choice row shows the parameter's name on the left and its selector on the right. A row marked experimental carries an `experimental` chip beside its name. A checkbox belongs to list selection only.

**Search controls** searches labels and canonical paths across all stages, including folded or inactive modules, plus Settings, Export, and viewer controls. Results name the real owner and module. Selecting a locator reveals, scrolls to, and focuses the actual control; Settings and Export results open their own surfaces rather than creating duplicate editors. The inspector query does not filter those dialogs or change their selected options. **clear filter** restores ordinary disclosure; an empty result names the unmatched query. Closed modules summarize their contents, and inactive controls explain their applicability while retaining stored values. Opening an inactive module does not enable it.

**Only modified** filters against the factory baseline, independently of the applied preset. It can combine with search. The row marker, module `modified` chip, and this filter use the same factory comparison. The preset browser's applied/modified/missing marker has its own preset baseline.

For Settings and Export, the filter compares accepted preferences with typed factory defaults. Unsubmitted Export drafts do not change these results.

Each editable row offers **Reset**, with help naming `Reset <control> to factory`. It restores only that canonical leaf; a vector leaf resets its components together. A module header's **reset** restores only that module's controls. For example, resetting **Preflash** leaves main **Enlarger** exposure and filtration unchanged; resetting **Print viewing glare** leaves **Print chemistry** unchanged. **Reset film profile to factory** and **Reset print profile to factory** target their respective material selection. Factory reset values are not the current material's suggestions or the last applied preset. Photographic row and module resets are reversible adjustments and never rewrite a preset or startup-default file. Settings, viewer preferences, and an unsubmitted Export draft remain outside adjustment history, including resets on those surfaces. See [section reset](../specs/gui-workflow/spec.md#section-reset).

A module's `Section preset: <name>` menu is a separate operation from its reset. Its serialized scope can cover more than the visible module; the menu states **Full preset scope** before its actions. **Save as…** saves that section. Direct `Apply Built-in / <name>` and `Apply My Presets / <name>` actions apply an entry without a hover-only submenu. Available management actions also name their entry and support keyboard or pointer use. A feature profile is distinct from a look: it changes only its declared feature controls, not materials, route, or unrelated parameters. Direct controls remain available under their applicability rules.

**Scan → Scanner** keeps its controls readable across routes and identifies whether the scanned medium is Paper, Slide, Negative, or Inactive:

- The scan-output selector offers **Preserve medium polarity** and **Negative to positive**. It is editable only for eligible negative-scanning routes; other routes retain the visible, inactive selector.
- Conversion style is editable only while the eligible **Negative to positive** conversion is selected. Otherwise its stored value remains readable with an inactive explanation. `standard` is the built-in automatic style; other names are declared fixed styles in `data/scan-styles/`.
- The readout illuminant is a fixed product value selected by the material category, not an editable scanner lamp.
- **Lens blur (readout optics)** and **Unsharp mask (digital sharpening)** describe readout optics and digital sharpening. On a route without scanning, these controls remain readable but inactive, with stored values retained.

**Scan → Scanner → Operator correction** adjusts the finished positive after this frame's automatic setup, before the fixed tone curve. **Finished exposure** is ±3 EV (positive is brighter); **Warmth** and **Green–magenta tint** are ±1 differential EV (positive is warmer and more magenta respectively, not Kelvin); **Contrast** spans 0.5–2 (1 is unchanged). These are editor ranges, not document limits. The controls are disabled outside eligible negative-to-positive `standard` scans, with their stored values retained.

Each operator row has its own **Reset** action. **Restore automatic** resets only the four operator values to 0, 0, 0, 1. The workspace's source-local **Undo** and **Redo** include these edits, resets and paste actions. **Hold automatic comparison** (primary mouse button, or Space while focused) shows the same prepared frame with default operator values until release; it does not change saved corrections or the retained pixels used by Save. This is separate from the viewer's input/output split comparison.

**Copy operator correction** and **Paste operator correction** use a session-local clipboard containing only the four values. Load another frame and paste to apply them over that frame's own automatic setup; no offsets, film density, capture calibration, or other settings are copied. The clipboard and adjustment history are not saved in presets or GUI state.

Changing the route resets an ineligible scan polarity; leaving negative conversion resets the conversion style to `standard`. This style applies automatic exposure and color setup to the whole frame before the positive tone. `neutral` is accepted only as an input migration alias and is saved as `standard`; it does not preserve the former fixed-curve appearance. See [scanning](scanning.md) for scanner mode and style semantics.

**Input and capture → Convert → scan illuminant** declares the acquisition light of an imported negative RGB image through `film_render.convert.scan_illuminant`. It defaults to `D55` and affects capture inversion and calibration only. Choose from the full shared illuminant catalog or enter a custom `BB<kelvin>` value accepted by the shared validator. It does not override the fixed product readout light. The control remains readable but inactive on routes without convert-film inversion.

**Output → Output → Simulation blend** blends the complete result with the decoded, geometrically aligned source in linear output RGB. Amount `1` keeps the complete result. Amount `0` returns the prepared source before virtual camera exposure. A negative capture stays negative at zero. Preview, Scan, Export, and complete source-to-output LUTs share this operation. Segmented LUTs require amount `1`. See [the output mix specification](../specs/output-mix/spec.md#output-mix) and [the GUI workflow specification](../specs/gui-workflow/spec.md#material-controls-and-output-composition) for the control semantics.

**Print → Enlarger → print exposure mode** selects `auto` or `manual`. `auto` normalizes the print exposure and follows camera exposure compensation, like a color analyzer or minilab. `manual` applies the print exposure time alone, like a test strip. The separate **Print → Preflash** module exposes **preflash exposure**, **preflash y filter shift**, and **preflash m filter shift**. Exposure is a linear multiplier, not EV; zero disables flash without clearing either filter value. On photographic paper, it can change weak highlight differences, but it may lower white, add fog, and shift color. It cannot recover absent detail. Routes without printing retain these values without applying them. See [the output mix specification](../specs/output-mix/spec.md#preflash) and [the GUI workflow specification](../specs/gui-workflow/spec.md#material-controls-and-output-composition) for the control semantics.

The GUI has no standalone Output lightness-compression control. See [presets](presets.md) for the versioned state document rule.

### CMYK ink style
**Output → CMYK ink style → Enable CMYK ink style** applies the fixed `ink` appearance to the selected workflow. Its **strength** control ranges from 0 to 1. Strength 0 is a no-op, and disabling the style retains its strength. Photographic print controls remain separate.

The optional appearance applies to any supported base workflow, including the `input` workflow and a scanned negative. It runs after the base workflow and its scanner-domain effects, before output mix and writing. It is a modeled RGB appearance, not a press proof or physical-print guarantee.

`cmyk.active` and `cmyk.strength` enable the treatment independently of the base workflow. There is no CMYK route. The treatment remains three-channel RGB. It does not generate CMYK files, separations, plates, press-ready output, halftone, or paper texture.

The `input` workflow color-manages an already rendered image to the output space. Film and print profile selections do not affect it. See the [film-chain specification](../specs/film-chain/spec.md) and [CMYK ink style specification](../specs/cmyk-style/spec.md) for the workflow contracts.

### Grain profiles

**Film → Grain** exposes **Procedural Grain Profiles** beside the engine selector. Spektra, Dye, and Fotufilm Film are material-grain engines; Procedural is a post-scan grain appearance. Switching engines retains the other engines' stored controls. Each engine keeps its own parameters and applicability rules rather than sharing a generic strength control. See the [feature profile contract](../specs/feature-profiles/spec.md) for the shared catalog and scope rules.

## Adjustment history

Header **Undo** and **Redo** apply to accepted adjustments for the current source: parameter edits, workflow and material selection with their dependent normalizations, feature/section/look-preset application, source rotation, and row/module resets. Their hover text names the next action; unavailable actions are disabled. One completed slider drag or numeric text edit is one step; each original wheel event is one step. Rejected edits and canceled gestures add no step. A new accepted edit after Undo discards the redo branch.

Undo restores accepted controls, materials, route, explicit edit ownership, and the associated applied-preset state. Redo restores the recorded successor without rereading a possibly changed preset file. Both invalidate obsolete work and follow auto preview; delayed worker results cannot overwrite a newer restored adjustment. The viewer retains its last completed frame while work runs and the header marks it **Retained frame is not current · controls changed** until a current result is accepted.

Use Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z outside text editors and the Settings/Export dialogs. When a text editor has focus, its own text-editing shortcuts do not undo photograph adjustments.

Accepting another source, completing RAW reprocessing, or loading a complete GUI state begins a new history. Failed imports, reprocessing, and state loads preserve the preceding context and history. The undo stack is not saved in GUI state. Viewer presentation, application settings, unsubmitted Export drafts, preset-library changes, startup-default files, diagnostic reports, and completed Save/Export files are outside adjustment history. Undo does not delete or rewrite those files, and it does not change an in-flight Export's captured configuration.

## Preview and Scan

**PREVIEW** renders a bounded image using the configured preview long-edge limit and current runtime controls. Set **preview max size** and **auto preview** in **Settings → Session**. The preview contract disables the spatial and stochastic effects it can safely omit, including lens blur, diffusion blur, physical grain, scanner blur, unsharp masking, and glare blur; Procedural and Fotufilm grain, halation, and diffusion filters remain active. Explicitly update it when auto preview is disabled.

New edits replace queued work. Stale results cannot replace the latest output. The viewer retains its last frame until the current frame is ready. Automatic parameter updates skip the development animation; explicit Preview and Scan retain it.

**SCAN** renders the full-resolution input with spatial and stochastic effects. It prefers GPU execution with CPU fallback. The resulting full-resolution output is retained for Save and inspection in the viewport.

**Scan → Scanner → scan for print** disables print viewing glare for the print scan. Toggle it again to restore the previous value. Loading state or changing profiles clears the transient snapshot.

The current optical model moves scanner lens blur to linear readout light. It runs before interpretation, gamut, sharpening, and transfer encoding. Print viewing glare follows optical blur and does not expose or develop the print material. Nonzero blur can intentionally change historical direct-scan pixels. Recipes and reports identify the current model revision. Corrected scans use faithful per-stage execution when resident shaders cannot preserve that order. Preflash retains both exposure gains and floors. Resident printing applies its outer gain to flash as well as main exposure. Film grain stays in the material domain. Procedural grain stays a post-scan appearance. See [the output mix specification](../specs/output-mix/spec.md) and [the GUI workflow specification](../specs/gui-workflow/spec.md) for the owning semantics.

See the [GUI workflow specification](../specs/gui-workflow/spec.md) for the action contract.

## Viewer

**Input and capture → Source rotation** contains **ccw rotate** and **cw rotate**. These are source adjustments, not viewer transforms: they physically rotate the in-memory input used by Preview, Scan, and Export, clear the retained output, and schedule a new preview. Save writes the retained output of the latest completed render without rerendering, so it cannot render a rotation that has not been previewed or scanned.

`100%`, `200%`, and `400%` map image pixels to exact device-pixel percentages. `fit` returns to the fitted zoom and keeps the pan; `reset view` also recentres it. Full-resolution Scan remains available in the viewport at these zoom levels. The preview long-edge limit controls Preview rendering, not Scan's available detail.

The viewer accepts pointer gestures on its surface. The wheel and pinch zoom at the pointer. A primary-button drag pans. A Shift or secondary-button drag selects a rectangle and its release zooms so the selection fills the viewer. One click zooms one step at the pointer; a double click fits the view. Gestures change the retained view only and never saved or exported pixels.

The `Paper back` layer is a viewer-only virtual rendering generated from paper texture, watermark, and noise. It is not an editable material stage and does not alter retained, saved, or exported pixels.

The viewer footer groups its controls by target: layer selection, view zoom, before/after comparison with its split position, reveal/crossfade presentation, and the current zoom as a read-only readout. Source rotation belongs to the inspector, not this footer.

The footer's layer selector switches between **Input**, **Output**, and **Paper back**. **Settings → Display → output interpolation** selects nearest, linear, cubic, spline16, spline36, Lanczos, or Blackman sampling for Output. Input retains the upstream nearest interpolation. Paper back uses spline36. The viewport samples the retained full-resolution image for visible detail.

The collapsible tonal-distribution readout sits above inspector search and stage navigation and describes the retained output without another render. It is viewer-only. Its empty state and its source follow [viewer presentation](../specs/gui-workflow/spec.md#viewer-presentation).

The canvas uses the pinned 18% gray (`#767676`) or black. It includes normalized white padding and the upstream paper watermark. Hovering reports the original floating RGB pixel.

The window header shows the current source, film profile, print profile, and exact workflow route as read-only text; hover the route for its physical description. The operation status appears only in this header. While Preview or Scan runs, it names the operation and the stage that executes now, for example `Scan · Film development`. With no work running it describes the last accepted operation; Export reports its status with elapsed time and cancellation state there too. Its content and its single-surface rule follow [labels and surfaces](../specs/gui-workflow/spec.md#labels-and-surfaces).

### Display transform and ICC

When the display transform is enabled, every platform converts output-space pixels to encoded sRGB for the disposable viewer raster. Save and Export use their respective saving color and encoding settings. The profile selection rule and the no-profile fallback follow [viewer presentation](../specs/gui-workflow/spec.md#viewer-presentation).

**Settings → Display** lets you choose an ICC profile with **select ICC profile…**, clear it again with **clear**, and shows the selected path or the reason it is unusable. The selection applies to the disposable viewer raster only: Save, Export, and the retained output keep their own pixels.

**use display transform**, **gray 18% canvas**, **white padding**, and **output interpolation** have factory row resets in this section. Each reset updates its live viewer preference only. These preferences and their resets remain outside photograph adjustment history and do not change retained, saved, or exported pixels.

On macOS, the Metal surface is tagged sRGB.

## Independent Export

Header **Export** opens **Export options**. Choose **Backend**, **Format**, **Bit depth**, **Saving color space**, **Apply saving CCTF encoding**, and the applicable codec/compression settings, then **Export file…** to choose the destination. Export renders a snapshot of the full input and simulation parameters independently of retained preview output. **Cancel**, Escape, or closing **Export options** discards its unsubmitted draft without changing accepted adjustments or retained output.

Locating another Export control while the dialog is open preserves its draft. The locator changes focus, not the selected delivery options.

Export uses the in-memory input pixels with the simulation parameters. It runs at the build's precision; see [parity and backends](parity.md#backend-selection). GUI Export does not launch `spektrafilm-f64`; use that CLI separately for CPU f64 reference output. Output metadata uses the current orientation and dimensions.

Export has no second options area in the inspector. Bottom **SAVE** chooses its format from the filename extension independently of Export preferences; it does not submit an Export request.

Export's saving color space and transfer encoding are independent of Save's current simulation saving settings. JPEG exposes quality (1–100) and 4:4:4/4:2:0 chroma sampling. TIFF/EXR expose their supported depth/compression settings. See the [CLI image output](cli.md#image-output) and [photo-export specification](../specs/photo-export/spec.md) for supported output combinations.

The header reports Export status and elapsed time. **Cancel export** discards the result before publication through the active worker's cancellation path. Cancellation may wait for active computation or writing.

### JPEG Export

Explicit JPEG Export uses the chosen quality and 4:4:4 or 4:2:0 chroma subsampling. Save leaves JPEG quality/subsampling to upstream OpenImageIO defaults.

The GUI warns that JPEG remains lossy even at quality 100. 4:2:0 can soften colored grain. Use TIFF or EXR when comparing grain detail without JPEG compression. Quality 100 and 4:4:4 do not make JPEG lossless.

## Save retained output

**SAVE** saves the latest retained floating output without rerendering, even when the header marks that frame as not current. It uses **Output → Output → saving color space** and **saving cctf encoding**. The filename extension selects the format. `.jpg` is the default. Run Preview or Scan first when the file should reflect changed controls.

- JPEG/PNG use 8-bit integer samples.
- TIFF uses 16-bit integer samples with ZIP compression.
- OpenEXR uses 16-bit half samples.

Save uses upstream OpenImageIO JPEG defaults rather than Export's quality/subsampling settings. Neither TIFF nor EXR forces ACES or linear encoding. Viewer borders, watermark, and display ICC transforms stay out of saved pixels. See the [shared writer contract](cli.md) for output format details.

## State and defaults

**Settings → GUI parameters → Save current to file / Load from file** exchange the pinned Python 0.3.4 GUI JSON sections. Partial files merge into factory values. Legacy input aliases and nested sections normalize to the flat upstream format. Invalid JSON, field types, selections, or Rust extension versions appear in the header status without replacing accepted state. A successful load begins a new adjustment history rather than restoring an old undo stack.

In the same settings section, **Save current as default** persists the current controls and reports that it replaced startup controls. **Restore factory default** confirms before it applies, then restores Kodak Gold 200 + Kodak Supra Endura, RMS granularity `[5,5,5]`, and upscale factor `1.0`. The confirmation states that the action replaces the current controls and session settings and removes the saved startup default; cancelling keeps both. Adjustment Undo does not reverse either startup-file change. Saved user values, including zero RMS, survive startup and state loading.

Startup files use the platform configuration directory. `SPEKTRAFILM_CONFIG_DIR` overrides it. Rust settings use `rust.version = 2`. File-dialog directories persist separately.

Legacy states without an explicit export format preserve 8-bit as PNG and 16/32-bit as TIFF before defaults and validation. Recipe values and viewer state remain intact. Explicitly incompatible format/depth combinations remain errors.

### Preset browser state

The GUI remembers the preset browser selection and layout independently of startup controls. An ordinary restart restores that metadata and recomputes the applied, modified, or missing marker. It does not apply the preset again. Loading an explicit GUI state uses that file's browser metadata. Use **Save current as default** to change startup controls.

A failed preset application preserves the current controls and temporary white/black correction state. See [presets](presets.md) for library and composition rules. The [presets specification](../specs/presets/spec.md) owns the preset contract.

Each preset card carries its own `preset actions` control beside the entry name. Opening it reaches the same actions as the card's context menu — update, revert, rename, duplicate, export, show file, and delete — by pointer or by keyboard, so the context menu is an additional route rather than the only one. Update applies only to the applied user preset; revert, rename, and delete apply to user presets only.

## Diagnostics

**Settings → Diagnostics** selects Off, Summary, or GPU timing for subsequent operations. Changing the mode leaves running operations on their captured mode. The session keeps the latest 20 collected operation reports. Select a report to inspect its phases, stages, GPU work, timing availability, and outcome.

**Save report…** writes local JSON without rendering. Save reports identify the retained render that supplied their pixels. This includes renders made with collection off or removed from history.

Launching with `SPEKTRAFILM_GUI_DIAGNOSTICS_DIR` set to an existing directory publishes every completed operation's report there as a new `operation-<id>.json`. It selects summary collection for the session, as a CLI report request does. See the [telemetry specification](../specs/telemetry/spec.md) for report semantics.

## Session configuration

**Settings** contains **Session**, **GUI parameters**, **Display**, and **Diagnostics**. It is separate from photographic adjustments and independent Export options. **Close settings**, Escape, or the window close action returns to the workspace and restores focus to the invoking control. Closing Export options likewise returns focus to its invoking control.

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
