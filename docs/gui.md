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

The header provides a **Photographs** menu (**照片**) containing **Save adjustments…**, **Reopen adjustments…**, and **Photographs…**, plus **Open**, **Undo**, **Redo**, **Settings**, and primary **Export**. It identifies the accepted source, film, print material, and workflow route as read-only context, and reports import, delivery, completion, and error messages. While an export runs, **Cancel export** replaces **Export**. The single viewer status strip above the canvas owns result identity and live rendering status.

The left **Presets** browser is collapsible and resizable. **Search presets** finds looks by name or film/print stock, including stock display names. **Clear preset search** restores the full list. **Built-in** looks are read-only; **My Presets** holds user looks. **Save preset…** saves a look, not a photograph or startup default. Searching, collapsing, or scrolling the browser does not apply a look or change the photograph.
At narrow window widths, the browser automatically becomes a reopenable rail under the [responsive preset layout](../specs/presets/design.md#boundaries). Reopening it while narrow is session-only; it does not change the saved expanded preference, which returns in a wider window.
Built-in reference thumbnails sit beside the compact card controls; the fixed-reference warning, attribution, and license appear once above the group. Each card's visible action menu supports pointer and keyboard activation, with focused controls scrolled into view; library issues precede the cards, and the applied or missing look remains visible above the list.

**Favorites** and **Recently used** filter the same library together with search. Favoriting or browsing does not apply a look. Recently used retains ten distinct successful whole-look applications. These discovery preferences persist separately from photographic controls and preset files. Reference thumbnails use one licensed photograph, not the active source; missing or outdated assets show an unavailable reference instead of a misleading image.

The right inspector contains one continuous list, not mutually exclusive pages. Its sticky **Search controls**, **Only modified**, and stage-jump strip remain visible while controls scroll. The jumps reveal and scroll to **Input and capture**, **Film**, **Print**, **Scan**, or **Output** in that same list while preserving the active search and Only modified filter without changing parameters. Several stages and modules can remain open together. Folding a module changes its presentation only, and ordinary edits, render completion, and route changes retain its disclosure and scroll state.

| Stage | Main modules |
| --- | --- |
| Input and capture | Workflow, Input, Import Raw, Source rotation, Crop and upscale, Camera, Camera diffusion, Input gamut compress, Spectral upsampling, Convert |
| Film | Film material, Film chemistry, Film base, Halation, Couplers, Grain |
| Print | Print material, Enlarger, Preflash, Enlarger diffusion, Print chemistry, Print base |
| Scan | Scanner, Print viewing glare |
| Output | Output, Output gamut compress, CMYK ink style |

The persistent **SCAN** and **SAVE** action bar stays below the inspector. SCAN shows `full-resolution effects`; SAVE shows `retained result · no rerender`. Preview updates automatically; there is no Preview button or enablement preference. **Settings** owns general application preferences, state files and startup defaults, display configuration, and Diagnostics. Independent file-delivery options belong to the **Export options** dialog, not a photographic module.

## Interface language

Open **Settings → General → 语言 / Language** (**设置 → 通用** in Simplified Chinese). Choose **跟随系统 / System**, **简体中文**, or **English**. System follows supported operating-system language preferences and otherwise uses English. The selector keeps its bilingual label so it remains recognizable after switching.

Language changes apply immediately and persist independently of photographic GUI state. Loading a state, applying a preset, restoring startup controls, and adjustment Undo/Redo do not change the language. Switching language does not rerender the photograph, alter numeric parsing, or clear existing editing and Export drafts. A preference write failure reports that the choice could not be saved while keeping it active for the current run.

Control search accepts Chinese labels, English labels, and canonical paths in either language. For example, **胶片规格**, **Film Format**, **画面长边**, and `camera.film_format_mm` locate the same control. Brand and film-stock names, `35mm`/`120`/`4×5`, standard color-space names, paths, and user-authored preset text keep their original identities. Native file choosers follow the operating system's language.

Chinese fonts are bundled for offline use; no system font installation is required. The interface supports native input-method composition in editable text. The CLI, serialized state, recipes, and diagnostic reports retain their existing machine vocabulary. The product contract is [GUI language](../specs/gui-language/spec.md#interface-language).

## Input and controls

**Open** accepts supported RGB images and camera RAW files through one chooser. RAW processing uses LibRaw with as-shot/daylight/tungsten/custom Kelvin+tint white balance and Lensfun correction. RAW enters the runtime as linear ACES2065-1. See [RAW input](raw.md) for white balance details.

The viewer also accepts a dropped image file through the same import path. An unsupported or unreadable import leaves the accepted image and controls unchanged and reports the error in the header.

Source loading, RAW reprocessing, rotation, and state loading prepare the replacement input view in the background. The header reports input preparation. Photographic editors and Undo/Redo pause during an input transaction; viewer navigation remains available. The previous accepted source, output, and history remain intact until preparation succeeds.

Choose the base route in **Input and capture → Workflow → workflow**, the film in **Film → Film material → film profile**, and the paper in **Print → Print material → print profile**. Selecting a film keeps the current paper; selecting a paper keeps the current film. Film selection applies its workflow default and scanner normalization, reported in the header. A look preset selects the material pair and its owned controls atomically. Development-time choices follow the selected material's actual family; a material without a selectable family shows its effective development time instead.

RGB imports retain their original host samples; GPU RAW imports retain the decoded device image and its color metadata. Preview, Scan, independent GPU Export, and input viewing consume that source without rewriting it. With a selected GPU, viewing a rotated RGB import uploads the original samples and turns them on the device; both RGB and RAW rotation produce a separate retained presentation image. Import and rotation prepare the replacement view before publishing the new source or orientation; failure preserves the previous accepted source and output. An explicit CPU reference export or source-analysis operation materializes device samples only at that requested host boundary.

GPU Preview and Scan reuse the current RGB input upload while its source, device, and color interpretation stay unchanged. Full-resolution Original navigation also reuses its source upload across pans and zoom changes. These caches retain only the current input for their viewing or rendering scope; they do not preload the photograph list.

Input color-space and CCTF controls reinterpret the accepted samples; they do not convert or rewrite the stored RAW carrier. Preview, Scan, input viewing, and Export use that accepted declaration, and queued exports keep the declaration captured at submission even if the active photograph changes.

### Film Format

**Input and capture → Camera → Film Format** offers **35mm**, **120**, **4×5**, and **Custom**. These choices set the physical film scale for grain and other film effects. They do not crop, resize, or change the image's aspect ratio.

The choices use exposed-image long edges of 36 mm, 56 mm, and 127 mm respectively. **120** uses one representative size; it does not select a square crop. **Custom** reveals **Frame Long Edge** for precise millimeter input. Opening Custom alone does not change the photograph.

Existing states and presets retain their exact `camera.film_format_mm` value. A value without a matching specification, including the existing 35 mm factory value, displays as Custom. Selecting a specification is one reversible adjustment; the module reset keeps the existing factory default.

### RAW decode draft

**Input and capture → Import Raw** contains **white balance**, **temperature**, **tint**, and **lens correction**. These are decode settings, not simulated camera exposure. Editing them leaves the accepted source pixels unchanged. For a RAW source, the module reports when settings are pending and the accepted source still uses the previous settings; **Reprocess RAW** explicitly decodes the source with the draft. Successful reprocessing replaces the accepted source and starts a new adjustment history. Failure retains the accepted source and preceding history, reports the error, and leaves the pending settings visible. For an RGB source, RAW decoding is inactive and the settings are retained for the next RAW import.

Simulated exposure remains a separate control at **Input and capture → Camera → exposure compensation ev**.

### Editing, search, and reset

**Numeric fields** accept typed exposure, film format, halation, DIR couplers, grain, glare, scanner, enlarger, and output values. Click a field to type with the text cursor. Scrolling over a field changes its value by the control's step. Scrolling elsewhere moves the control panel. Valid edits automatically refresh Preview after a short debounce. Incomplete or invalid text keeps the last valid parameter value and remains visible as an editing draft until Escape or ordinary focus loss. Opening language settings suspends the draft instead of finishing it; return to the field to continue or cancel it. Escape cancels the uncommitted edit. The editor range and its relation to the accepted document range follow [numeric editor ranges](../specs/gui-workflow/spec.md#numeric-editor-ranges).

Each numeric row shows the parameter's name on the left and its value on the right of one line, with a slider on the line below. A modified row shows a marker beside its label and marks its value; the marker uses the same factory baseline as the module's `modified` chip. Tuple, optional, extra, and boolean rows use the same marker rule. The slider only moves the value; the field stays the row's editor. The slider is logarithmic when the declared range spans more than three decades, or when the declared minimum is zero and the declared maximum reaches one thousand or more. A signed range stays linear. A logarithmic slider anchors its smallest positive value at the parameter's declared step, so its floor matches the field's own granularity. A declared minimum of zero keeps zero as the track's leftmost stop.

A boolean row is a switch that names the concept it enables. A choice row shows the parameter's name on the left and its selector on the right. A row marked experimental carries an `experimental` chip beside its name. A checkbox belongs to list selection only.

**Search controls** searches labels and canonical paths across all stages, including folded or inactive modules, plus Settings, Export, and viewer controls. Results name the real owner and module. Selecting a locator reveals, scrolls to, and focuses the actual control; Settings and Export results open their own surfaces rather than creating duplicate editors. The inspector query does not filter those dialogs or change their selected options. **clear filter** restores ordinary disclosure; an empty result names the unmatched query. Closed modules summarize their contents, and inactive controls explain their applicability while retaining stored values. Opening an inactive module does not enable it.

**Only modified** filters against the factory baseline, independently of the applied preset. It can combine with search. The row marker, module `modified` chip, and this filter use the same factory comparison. The preset browser's applied/modified/missing marker has its own preset baseline.

For Settings and Export, the filter compares accepted preferences with typed factory defaults. Unsubmitted Export drafts do not change these results.

A module header's **reset** restores only that module's controls. Parameter rows, sliders, and Export options do not have separate reset buttons. Use **Undo** to reverse an adjustment. For example, resetting **Preflash** leaves main **Enlarger** exposure and filtration unchanged; resetting **Print viewing glare** leaves **Print chemistry** unchanged. Material-module reset restores its material selection and development time. Factory reset values are not the current material's suggestions or the last applied preset. Photographic module resets are reversible adjustments and never rewrite a preset or startup-default file. Settings, viewer preferences, and an unsubmitted Export draft remain outside adjustment history. See [section reset](../specs/gui-workflow/spec.md#section-reset).

The left browser owns whole-look presets. Modules do not have section-preset menus or separate preset management. Existing section-preset files are left on disk but cannot be imported or applied through the GUI. A feature profile is distinct from a look: it changes only its declared feature controls, not materials, route, or unrelated parameters. Direct controls remain available under their applicability rules.

**Scan → Scanner** keeps its controls readable across routes and identifies whether the scanned medium is Paper, Slide, Negative, or Inactive:

- The scan-output selector offers **Preserve medium polarity** and **Negative to positive**. It is editable only for eligible negative-scanning routes; other routes retain the visible, inactive selector.
- Conversion style is editable only while the eligible **Negative to positive** conversion is selected. Otherwise its stored value remains readable with an inactive explanation. `standard` is the built-in automatic style; other names are declared fixed styles in `data/scan-styles/`.
- The readout illuminant is a fixed product value selected by the material category, not an editable scanner lamp.
- **Lens blur (readout optics)** and **Unsharp mask (digital sharpening)** describe readout optics and digital sharpening. On a route without scanning, these controls remain readable but inactive, with stored values retained.

**Scan → Scanner → Operator correction** adjusts the finished positive after this frame's automatic setup, before the fixed tone curve. **Finished exposure** is ±3 EV (positive is brighter); **Warmth** and **Green–magenta tint** are ±1 differential EV (positive is warmer and more magenta respectively, not Kelvin); **Contrast** spans 0.5–2 (1 is unchanged). These are editor ranges, not document limits. The controls are disabled outside eligible negative-to-positive `standard` scans, with their stored values retained.

**Restore automatic** resets only the four operator values to 0, 0, 0, 1. The workspace's source-local **Undo** and **Redo** include these edits, restore and paste actions. **Hold automatic comparison** (primary mouse button, or Space while focused) shows the same prepared frame with default operator values until release; it does not change saved corrections or the retained pixels used by Save. This is separate from the viewer's Original/Modified selection.

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

The `input` workflow color-manages an already rendered image to the output space. Film and print profile selections do not affect it. It skips film-exposure and print-calibration tables while retaining configuration validation and output color preparation. Switching to a film workflow prepares the selected materials. See the [film-chain specification](../specs/film-chain/spec.md) and [CMYK ink style specification](../specs/cmyk-style/spec.md) for the workflow contracts.

### Grain profiles

**Film → Grain** exposes **Procedural Grain Profiles** beside the engine selector. Spektra, Dye, and Fotufilm Film are material-grain engines; Procedural is a post-scan grain appearance. Switching engines retains the other engines' stored controls. Each engine keeps its own parameters and applicability rules rather than sharing a generic strength control. See the [feature profile contract](../specs/feature-profiles/spec.md) for the shared catalog and scope rules.

## Adjustment history

Header **Undo** and **Redo** apply to accepted adjustments for the current source: parameter edits, workflow and material selection with their dependent normalizations, feature and look-preset application, source rotation, and module resets. Their hover text names the next action; unavailable actions are disabled. One completed slider drag or numeric text edit is one step; each original wheel event is one step. Rejected edits and canceled gestures add no step. A new accepted edit after Undo discards the redo branch.

Undo restores accepted controls, materials, route, explicit edit ownership, and the associated applied-preset state. Redo restores the recorded successor without rereading a possibly changed preset file. Both invalidate obsolete work and automatically refresh Preview; delayed worker results cannot overwrite a newer restored adjustment. The viewer retains its last completed frame while work runs and the retained-result label marks it stale until a current result is accepted.

Rotation Undo/Redo refreshes input presentation without discarding the completed output. Modified remains viewable and Save continues to write the same retained samples until the automatic replacement render is accepted.

Use Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z outside text editors and the Settings/Export dialogs. When a text editor has focus, its own text-editing shortcuts do not undo photograph adjustments.

Accepting another source, completing RAW reprocessing, or loading a complete GUI state begins a new history. Failed imports, reprocessing, and state loads preserve the preceding context and history. The undo stack is not saved in GUI state. Viewer presentation, application settings, unsubmitted Export drafts, preset-library changes, startup-default files, diagnostic reports, and completed Save/Export files are outside adjustment history. Undo does not delete or rewrite those files, and it does not change an in-flight Export's captured configuration.

## Preview and Scan

Preview automatically renders a bounded image using the configured preview long-edge limit and current runtime controls. Set **preview max size** in **Settings → General**; accepting a new limit automatically updates Preview. The General section's **reset** restores that limit to the factory value. The preview contract disables the spatial and stochastic effects it can safely omit, including lens blur, diffusion blur, physical grain, scanner blur, unsharp masking, and glare blur; Procedural and Fotufilm grain, halation, and diffusion filters remain active.

Opening an image, applying a preset, moving a slider, or accepting numeric text refreshes **Modified** automatically. No Preview button, Update action, or auto-preview preference is required. Rendering waits while calibration or the existing modal guards block submission.

New edits replace queued work. Stale results cannot replace the latest output. The viewer retains its last frame until the current frame is ready. Automatic parameter updates after the first output skip the development animation; the first output and explicit Scan retain it.

The viewer status strip identifies the retained **Preview** or **Scan**, pixel dimensions, and whether it matches the current adjustments. This identity stays visible while another operation runs. Preview has an explicit detail-effects caveat; independent Export is the file-delivery action.

**SCAN** renders the full-resolution input with spatial and stochastic effects. Automatic backend selection chooses CPU only when a usable GPU is unavailable. A selected GPU computation failure fails the operation without substituting host arithmetic. The resulting full-resolution output is retained for Save and inspection in the viewport.

**Scan → Scanner → scan for print** disables print viewing glare for the print scan. Toggle it again to restore the previous value. Loading state or changing profiles clears the transient snapshot.

The current optical model moves scanner lens blur to linear readout light. It runs before interpretation, gamut, sharpening, and transfer encoding. Print viewing glare follows optical blur and does not expose or develop the print material. Nonzero blur can intentionally change historical direct-scan pixels. Recipes and reports identify the current model revision. Corrected scans use faithful per-stage execution when resident shaders cannot preserve that order. Preflash retains both exposure gains and floors. Resident printing applies its outer gain to flash as well as main exposure. Film grain stays in the material domain. Procedural grain stays a post-scan appearance. See [the output mix specification](../specs/output-mix/spec.md) and [the GUI workflow specification](../specs/gui-workflow/spec.md) for the owning semantics.

See the [GUI workflow specification](../specs/gui-workflow/spec.md) for the action contract.

## Viewer

**Input and capture → Source rotation** contains **ccw rotate** and **cw rotate**. These actions record quarter-turn orientation for Preview, Scan, and Export without changing the imported source samples. A successful rotation prepares the oriented input view, clears the retained output, and schedules a new preview. Save writes the retained output of the latest completed render without rerendering, so it cannot render a rotation that has not been previewed or scanned.

`100%`, `200%`, and `400%` map image pixels to exact device-pixel percentages. The viewer samples at device-pixel centers and treats image bounds as half-open, so fractional viewport origins and pans do not add or drop edge pixels. `fit` returns to the fitted zoom and keeps the pan; `reset view` also recentres it. Full-resolution Scan remains available in the viewport at these zoom levels. The preview long-edge limit controls Preview rendering, not Scan's available detail.

The viewer accepts pointer gestures on its surface. The wheel and pinch zoom at the pointer. A primary-button drag pans. A Shift or secondary-button drag selects a rectangle and its release zooms so the selection fills the viewer. One click zooms one step at the pointer; a double click fits the view. Gestures change the retained view only and never saved or exported pixels.

The `Paper back` layer is a viewer-only virtual rendering generated from paper texture, watermark, and noise. It is not an editable material stage and does not alter retained, saved, or exported pixels.

The viewer footer groups its controls by target: Original/Modified/Paper back selection, reveal/crossfade presentation, view zoom, and the current zoom as a read-only readout. Source rotation belongs to the inspector, not this footer.

The footer switches between **Original** (原图), the retained input, and **Modified** (修改图), the latest accepted Preview or Scan. Modified is disabled until output exists. The first accepted Preview or Scan selects Modified. Switching does not render or edit adjustments; both views share zoom and pan. **Paper back** remains a separate presentation. **Settings → Display → output interpolation** selects nearest, linear, cubic, spline16, spline36, Lanczos, or Blackman sampling for Modified. Original uses nearest interpolation. Paper back uses spline36. The viewport samples the retained full-resolution image for visible detail.

The collapsible tonal-distribution readout sits above inspector search and stage navigation and describes the retained output without another render. It is viewer-only. Its empty state and its source follow [viewer presentation](../specs/gui-workflow/spec.md#viewer-presentation).

The canvas uses the pinned 18% gray (`#767676`) or black. It includes normalized white padding and the upstream paper watermark. Hovering reports the original floating RGB pixel. GPU hover readings arrive asynchronously without blocking viewer navigation. Moving to another source pixel or changing the source or layer clears the old reading until the current result is ready.

The single status strip above the canvas identifies **Original**, **Paper back**, or the retained Preview/Scan result and its detail-effects caveat where applicable. It shows **Rendering…**, or **渲染中……** in Simplified Chinese, during debounce, pipeline work, and the latest viewer composition. The running message disappears on completion or failure and is absent without a source. The last completed image and pointer navigation remain available while it runs. There is no top-right canvas overlay or duplicate result identity in the header. The header retains import, Save, Export, completion, and error evidence, including Export elapsed time and cancellation. Status ownership follows [labels and surfaces](../specs/gui-workflow/spec.md#labels-and-surfaces).

The strip reserves two text lines so changing status does not resize the canvas. A long identity is shortened on screen; hover shows its full text.

With a selected GPU, the accepted render owns device samples through viewing and Save. Fitted resampling, display conversion, ICC evaluation, full-source interpolation, crossfade, development animation, and virtual-paper numerical synthesis use disposable device buffers; none rewrites the accepted render. The native egui frontend receives only packed RGBA8 presentation pixels. Histogram data and hovered source pixels are bounded readouts, not a full-image download followed by re-upload. Inactive camera and scanner lens blur do not allocate their image-sized blur scratch buffers. A failed viewer computation keeps the last accepted texture and reports the display error.

### Display transform and ICC

When the display transform is enabled, every platform converts output-space pixels to encoded sRGB for the disposable viewer raster. Save and Export use their respective saving color and encoding settings. The profile selection rule and the no-profile fallback follow [viewer presentation](../specs/gui-workflow/spec.md#viewer-presentation).

**Settings → Display** lets you choose an ICC profile with **select ICC profile…**, clear it again with **clear**, and shows the selected path or the reason it is unusable. The selection applies to the disposable viewer raster only: Save, Export, and the retained output keep their own pixels.

Display ICC preparation and GPU compilation run in the background. With a selected GPU, the CPU prepares image-independent LittleCMS conversion data and fixed GPU kernels apply it to viewer pixels. Complex profiles retain the LittleCMS RGB8 conversion exactly rather than using an approximate coarse LUT or CPU processing of the displayed image.

Compatible prepared ICC data is reused across images and application restarts. The bounded cache lives under `cache/display-icc-v1` in the application configuration directory. Missing, corrupt, or unwritable cache files cause background preparation, not a different conversion. Device buffers and compiled pipelines are reused within the process; this cache does not store driver-compiled binaries. Save and Export do not use the display ICC conversion.

One **reset** action in Display restores **use display transform**, **gray 18% canvas**, **white padding**, and **output interpolation** together. It keeps the selected ICC profile. These viewer preferences and their reset remain outside photograph adjustment history and do not change retained, saved, or exported pixels.

On macOS, the Metal surface is tagged sRGB.

## Independent Export

Header **Export** opens the modal **Export options** dialog. Choose **Format**, **Bit depth**, **Saving color space**, **Backend**, **Apply saving CCTF encoding**, and the applicable codec/compression settings, then **Export file…** to choose the destination. Export renders a snapshot of the full input and simulation parameters independently of retained preview output. **Cancel**, an explicit close action, or Escape when Export is the topmost modal discards its unsubmitted draft without changing accepted adjustments or retained output. A backdrop click does not dismiss it.

Locating another Export control while the dialog is open preserves its draft. The locator changes focus, not the selected delivery options.

Export uses the in-memory input pixels with the simulation parameters. It runs at the build's precision; see [parity and backends](parity.md#backend-selection). GUI Export does not launch `spektrafilm-f64`; use that CLI separately for CPU f64 reference output. Output metadata uses the current orientation and dimensions.

Export has no second options area in the inspector. Bottom **SAVE** chooses its format from the filename extension independently of Export preferences; it does not submit an Export request.

Export's saving color space and transfer encoding are independent of Save's current simulation saving settings. Format capabilities follow the [shared writer](../specs/photo-export/spec.md) and [saving conversion rules](../specs/cli/spec.md#export-and-execution-options):

| Format | Depth | Saving encoding | Compression or codec options |
| --- | --- | --- | --- |
| JPEG | 8-bit | Encoded | Quality 1–100; 4:4:4 or 4:2:0 chroma sampling |
| PNG | 8-bit | Encoded | Fixed by the writer |
| TIFF | 8-bit, 16-bit, or 32-bit float | Optional CCTF | ZIP or none |
| OpenEXR | 16-bit half or 32-bit float | Linear | ZIP |

Fixed rows show their effective values and reasons instead of one-choice selectors. JPEG quality and subsampling remain readable with their stored preferences in other formats, marked not applicable. Every row remains locatable by its canonical control identity; locating it does not alter the draft.

The header reports Export status and elapsed time. **Cancel export** discards the result before publication through the active worker's cancellation path. Cancellation may wait for active computation or writing.

### JPEG Export

Explicit JPEG Export uses the chosen quality and 4:4:4 or 4:2:0 chroma subsampling. Save leaves JPEG quality/subsampling to upstream OpenImageIO defaults.

The GUI warns that JPEG remains lossy even at quality 100. 4:2:0 can soften colored grain. Use TIFF or EXR when comparing grain detail without JPEG compression. Quality 100 and 4:4:4 do not make JPEG lossless.

## Save retained output

**SAVE** saves the latest retained floating output without rerendering. If it no longer matches the accepted adjustments, an explicit confirmation offers to continue saving that frame or cancel before opening the file chooser. It uses **Output → Output → saving color space** and **saving cctf encoding**. The filename extension selects the format. `.jpg` is the default. Wait for automatic Preview or run Scan first when the file should reflect changed controls.

- JPEG/PNG use 8-bit integer samples.
- TIFF uses 16-bit integer samples with ZIP compression.
- OpenEXR uses 16-bit half samples.

Save uses upstream OpenImageIO JPEG defaults rather than Export's quality/subsampling settings. Neither TIFF nor EXR forces ACES or linear encoding. Viewer borders, watermark, and display ICC transforms stay out of saved pixels. See the [shared writer contract](cli.md) for output format details.

## Photograph adjustment records

**Save adjustments…** writes a versioned photograph record containing source identity, accepted input interpretation, RAW decode settings, geometry, adjustments, seed, material identities, and render-preparation ownership. It does not embed source images or runtime assets. Keep the referenced external files unchanged. Pending text and RAW drafts, application preferences, undo stacks, jobs, and Export drafts are excluded.

**Save adjustments…** is unavailable while an input transaction prepares a replacement source or view. Wait for that transaction to finish before saving a record. A rejected save during preparation leaves the existing record, baseline, and record path unchanged.

**Reopen adjustments…** validates the source and dependencies before replacing the photograph. Missing or changed assets leave the current photograph intact. Explicit source relocation accepts only identical contents. A record is not a reusable preset or a startup-default file. Failed writes preserve an existing valid record and never overwrite the source.

Closing or replacing unsaved accepted work offers **Save record**, **Discard changes**, and **Cancel**. Canceling the chooser or a failed save stops the transition. Closing also protects unsaved inactive work-list entries, one identified source at a time.

## Photograph work list and queue

**Photographs…** opens the explicit work list. Add selected files, select an entry, or use previous/next. Each entry retains its accepted adjustments, history, record-save state, and separate pending drafts. Removing an entry never deletes its source, record, or exported files. Single-photograph editing does not require a work list.

Select destinations and an explicit copy scope before **Copy adjustments**. The dialog states which controls will be replaced. Look and section scopes use the existing preset boundaries. Procedural-grain and Halation feature-profile scopes copy only their canonical owned leaves, including custom overrides, without changing feature enable or engine selection. Operator correction copies only its four correction values. Copy preserves each destination's source interpretation, RAW settings, geometry, source calibration, and delivery settings. Each destination validates independently and gets one reversible history transaction; a failed destination stays unchanged.

Choose an output directory and explicitly permit replacement when needed, then submit selected entries. Duplicate destinations must be resolved before submission. The sequential queue captures source pixels, accepted adjustments, dependency identities, delivery options, and destinations independently of later edits or removal. Changed dependencies fail that submission rather than silently rendering another result.

The queue shows waiting, running, completed, failed, and canceled outcomes. Cancel one job or all remaining jobs without removing completed files. A failure does not stop unrelated entries. **Retry submitted snapshot** repeats the failed submission, not current adjustments. Queue work and interactive Export share the existing staged publication path; the batch is not one atomic transaction. Restart recovery is not supported.

## State and defaults

**Settings → GUI parameters → Save current to file / Load from file** exchange the pinned Python 0.3.4 GUI JSON sections. Partial files merge into factory values. Legacy input aliases and nested sections normalize to the flat upstream format. Invalid JSON, field types, selections, or Rust extension versions appear in the header status without replacing accepted state. A successful load begins a new adjustment history rather than restoring an old undo stack.

In the same settings section, **Save current as default** persists the current controls and reports that it replaced startup controls. **Restore factory default** confirms before it applies, then restores Kodak Gold 200 + Kodak Supra Endura, RMS granularity `[5,5,5]`, and upscale factor `1.0`. The confirmation states that the action replaces the current controls and session settings and removes the saved startup default; cancelling keeps both. Adjustment Undo does not reverse either startup-file change. Saved user values, including zero RMS, survive startup and state loading.

Startup files use the platform configuration directory. `SPEKTRAFILM_CONFIG_DIR` overrides it. Rust settings use `rust.version = 2`. File-dialog directories persist separately.

Legacy states without an explicit export format preserve 8-bit as PNG and 16/32-bit as TIFF before defaults and validation. Recipe values and viewer state remain intact. Explicitly incompatible format/depth combinations remain errors.

Older states may contain `simulation.auto_preview`. Loading discards that retired preference; saved states omit it. Preview always updates automatically.

### Preset browser state

The GUI remembers the preset browser selection and layout independently of startup controls. An ordinary restart restores that metadata and recomputes the applied, modified, or missing marker. It does not apply the preset again. Loading an explicit GUI state uses that file's browser metadata. Use **Save current as default** to change startup controls.

A failed preset application preserves the current controls and temporary white/black correction state. See [presets](presets.md) for library and composition rules. The [presets specification](../specs/presets/spec.md) owns the preset contract.

Each preset card carries its own `preset actions` control beside the entry name and a star toggle for favorites. The star changes favorites only; it does not apply the look. Opening `preset actions` reaches the same actions as the card's context menu — update, revert, rename, duplicate, export, show file, and delete — by pointer or by keyboard, so the context menu is an additional route rather than the only one. Update applies only to the applied user preset; revert, rename, and delete apply to user presets only.

## Diagnostics

Performance reports are recorded automatically from startup. Reproduce the problem, then choose **Settings → Diagnostics → Open log folder** to find the files. No folder or collection-mode setup is required.

Completed operations write JSON reports in the background. Each application launch uses its own persistent session folder under the application's `logs/diagnostics` directory. Restarting does not overwrite earlier sessions. A report-write failure does not fail the image operation.

### Advanced details

The collapsed **Advanced details** panel contains collection modes, the latest 20 in-memory reports, timing and work counters, and **Save report…** for individual export. Changing the mode affects only subsequent operations and does not render the image. Summary is the GUI default; GPU pass timing remains an explicit advanced choice.

Search for `diagnostics.collection_mode` to reveal **Collection mode** inside Advanced details, even before the first report exists.

Reports distinguish measured values from unsupported or uncollected measurements. Host waits are not shader execution time, and requested buffer bytes are not process RSS or committed VRAM. See the [telemetry specification](../specs/telemetry/spec.md) for report semantics.

For controlled launches, `SPEKTRAFILM_GUI_DIAGNOSTICS_DIR` can override the automatic session destination. Ordinary GUI use does not need it.

## Application configuration

**Settings** contains **General**, **GUI parameters**, **Display**, and **Diagnostics**. General and Display start open. Settings and Export block background pointer and keyboard input while open; a backdrop click does not dismiss them. **Close settings**, an explicit window close action, or Escape when Settings is the topmost modal returns to the workspace and restores focus to the invoking control. Closing Export options likewise restores its opener's focus. Factory-default confirmation can stack above Settings; Escape dismisses only the topmost modal.

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
