# GUI product language

## Authority and boundary

This specification owns the language that the GUI presents to a person. It owns interface language selection, localized labels, choice labels, help text, workflow descriptions, status wording, and the separation between product language and technical identifiers. [The language design](design.md) owns localization identity, preference, and resource boundaries.

[`CONTEXT.md`](../../CONTEXT.md) owns the domain glossary. [`Film chain`](../film-chain/spec.md) owns scanner operations, scanner stage boundaries, scan-output naming, and conversion-control naming. [`GUI workflow`](../gui-workflow/spec.md) owns control inventory, stage and surface ownership, editing behavior, and persisted-state behavior.

[`GUI presentation`](../gui-presentation/spec.md) owns row and surface presentation.

The GUI MUST use the domain glossary and the owning operation specification as its vocabulary source. This specification MUST NOT redefine physical terms with shorter synonyms.

## Interface language

The GUI MUST support English and Simplified Chinese. English labels in this specification name the English presentation; other supported languages MUST preserve their operation and meaning rather than display those labels literally. This does not permit translation of identifiers protected by [Serialized and diagnostic language](#serialized-and-diagnostic-language).

### Selection and persistence

The language selector MUST reside in **Settings → General**, or **设置 → 通用** in Simplified Chinese. Its label MUST always be **语言 / Language**. Its options MUST remain recognizable in either language: **跟随系统 / System**, **简体中文**, and **English**.

General replaces the visible Session settings group; it retains that group's existing controls and scopes. GUI parameters, Display, Diagnostics, and independent Export keep their distinct owners. This change does not add an appearance setting or move photographic controls into Settings.

The initial preference MUST be System. System MUST resolve the operating system's ordered interface-language preferences to a supported language at startup. Explicit Simplified Chinese script tags, including `zh-Hans`, and Chinese region tags for mainland China or Singapore without a conflicting script MUST resolve to Simplified Chinese.

Traditional Chinese tags MUST NOT be treated as Simplified Chinese. English tags MUST resolve to English. Unqualified `zh` and unsupported or unavailable preferences MUST fall through to the next supported preference, or English when none remains.

The selector MUST distinguish System from an explicit language even when they currently resolve to the same presentation.

An explicit choice MUST override system detection and persist across application restarts. Switching to System MUST resolve the current system preference immediately. Continuous monitoring of system-language changes while the application is running is not required.

Language MUST be an application preference, not a photograph adjustment. Applying a preset, loading GUI state, saving or restoring startup controls, resetting a photographic module, and Undo/Redo MUST NOT change it. Language MUST NOT be carried by presets, photographic GUI-state files, render recipes, CLI parameters, or exported image metadata.

Failure to save the preference MUST leave the selected language usable for the current run and report that persistence failed.

### Live switching

Language selection MUST update all open application-owned surfaces without a restart or a render. It MUST NOT change source pixels, accepted parameters, parameter revisions, edit provenance, history, retained output, or submitted work. Language MUST NOT affect render caching or Save/Export pixels.

Switching language MUST preserve uncommitted numeric and RAW drafts, Export drafts, search text, module disclosure, and editor buffers and selection. It MUST NOT commit, cancel, or restart an edit or operation as a relabeling side effect.

Selecting the language control MAY move focus normally; dialog return focus and keyboard commands MUST continue to address the same controls by identity after relabeling. Existing progress and history descriptions MUST use the current language without replacing their underlying operations.

### Translation coverage

Both languages MUST cover all application-owned workspace, inspector, viewer, preset-browser, Settings, Export, context-menu, and confirmation surfaces. Coverage includes control and option labels, accessible names and descriptions, tooltips, units' explanations, inactive and modified states, empty results, validation guidance, operation progress, success messages, and application-generated error explanations. Technical help MUST NOT remain English merely because it is long.

Brand, material, and product names; film specification codes such as `35mm`, `120`, and `4×5`; standard color-space and illuminant names; file-format names; mathematical symbols; and unit tokens MUST retain their identity. User-authored preset names, descriptions, and paths MUST NOT be translated. Built-in descriptive text MUST be localized without renaming the underlying preset or profile.

Recognized parameter choices MAY have translated display names, but their stored enum values MUST remain exact.

Application errors MUST provide a localized explanation and any known recovery action without hiding the failing path, rejected value, or technical detail. Unmodified third-party and operating-system error text MAY appear as diagnostic detail. The application MUST NOT guess a translated failure category from arbitrary error prose.

Native operating-system dialogs follow the operating system's language; switching the application language need not relabel an already-open native chooser.

The Simplified Chinese presentation MUST use these core terms consistently. The table translates existing concepts; it does not redefine their physical boundaries.

| English presentation | Simplified Chinese presentation |
| --- | --- |
| Settings / General | 设置 / 通用 |
| Open / Undo / Redo | 打开 / 撤销 / 重做 |
| Preview / Scan | 预览 / 扫描 |
| Save / Export | 保存 / 导出 |
| Presets / Search controls | 预设 / 搜索控件 |
| Only modified | 仅显示已修改 |
| Input and capture | 输入与拍摄 |
| Film / Print / Output | 胶片 / 印相 / 输出 |
| Finishing / Range | 成片 / 范围 |
| Brightness / Contrast | 明暗 / 反差 |
| Warmth / Saturation | 冷暖 / 彩度 |
| Film Format / Custom | 胶片规格 / 自定义 |
| Frame Long Edge | 画面长边 |
| reset <module> · help: Restore <module> to the factory state | 重置<模块> · 提示：将<模块>恢复为出厂状态 |
| history: Reset <module> to factory | 历史：将<模块>恢复出厂值 |
| Original / Modified / Paper back | 原图 / 修改图 / 相纸背面 |

### Search and text entry

Control search MUST match Simplified Chinese and English labels, module and destination names, and exact canonical paths in either interface language. Alternate visible labels, including Frame Long Edge, MUST remain searchable through their one canonical owner. Results MUST use the selected presentation language and retain technical paths as secondary detail.

Localization MUST NOT create duplicate controls, parameter aliases, or translated paths. Pinyin and fuzzy transliteration search are outside this contract.

Preset search MUST retain name and stock-name matching, including original English names. Localized built-in descriptions or display labels exposed as searchable names MUST NOT replace their original searchable identity.

Search, preset naming, and editable text MUST support Simplified Chinese input methods, composition, selection, clipboard operations, and backspace. Intermediate IME composition MUST NOT submit a dialog or apply a photographic adjustment. Existing numeric parsing, decimal syntax, unit tokens, and serialized number formatting MUST NOT depend on interface language.

### Typography and accessibility

Installed packages MUST provide licensed fonts that render all shipped Simplified Chinese text and common Simplified Chinese user input without system-font installation or network access. Font licensing and attribution MUST ship with the package. English text, symbols, mixed Chinese/Latin strings, and paths MUST remain legible.

Both languages MUST meet the existing desktop layout, keyboard, focus, and accessibility requirements. Translation MUST NOT make actions unreachable, obscure numeric values, or introduce label/value overlap. Ordinary labels MUST remain readable at supported window sizes and DPI settings; bounded truncation of long user data MUST preserve access to the complete value.

Accessible names and state descriptions MUST follow the visible language. Clipboard content and user data MUST retain their original characters.

### Scope

This capability adds English and Simplified Chinese presentation only. It does not translate the CLI, documentation, reports, or serialized contracts. It does not add external language packs, a translation editor, runtime machine translation, or a new GUI framework.

## Language layers

The GUI presents three language layers.

### Product language

Visible labels MUST name the current product concept or physical operation. A visible label MUST NOT expose a serialized path, snake-case enum, backend name, or implementation history unless that identifier is itself the product concept.

### Technical help

Tooltips and help text MUST preserve the exact technical meaning, domain, units, stage, calibration status, and limitations of the control. Technical help MAY use parameter paths and serialized values when that information helps a person understand the control.

### Serialized and diagnostic language

State keys, enum values, route strings, backend names, precision names, and telemetry identifiers MUST remain exact in state files, reports, diagnostics, and specifications. A presentation label MUST NOT change these identifiers or imply an alias.

A control MAY show a technical identifier in a tooltip or diagnostic detail. It MUST NOT use a second visible label to hide a persisted value's actual meaning.

## Physical vocabulary

The GUI MUST use these terms with the meanings defined by [`CONTEXT.md`](../../CONTEXT.md):

- `record exposure` means exposure received by one film or photographic-paper record before development.
- `record density` means optical density formed in one developed record before an output scan interprets it.
- `scanned medium` means developed film or photographic paper read for digital output.
- `scan sensor` means the colorimetric spectral response used to acquire transmitted or reflected light from a scanned medium.
- `scan capture` means RGB readings acquired by the scan sensor.
- `scanned density` means density derived from one channel of a scan capture relative to that channel's clear reference.
- `clear film base` means the unexposed base of the scanned medium, which reads as the output white reference of a scan.
- `negative conversion` means interpretation of a developed negative as a positive image.
- `exposure reconstruction` means negative conversion that estimates relative exposure through the selected film response.
- `digital output` means the color space, transfer encoding, geometry, and file representation used to deliver an image.

The GUI MUST NOT use the following substitutions when they would change a physical boundary:

- `capture response` for `scan sensor`;
- `interpretation` for `scan output` when the choices describe native polarity;
- `negative interpretation` for `negative conversion`;
- `simulation color space` for `output color space`;

A shorter label is permitted only when it preserves the same operation, domain, stage, and calibration claim.

## Scanner labels

Scanner labels MUST follow the scan-output and neutral-scan-model contracts in [`Film chain`](../film-chain/spec.md).

The GUI MUST display:

| Runtime concept | GUI label or choice |
|---|---|
| `scanner.scan_output = direct_scan` | `Preserve medium polarity` |
| `scanner.scan_output = negative_to_positive` | `Negative to positive` |
| `scanner.lens_blur` | `Lens blur (readout optics)` |
| `scanner.unsharp_mask` | `Unsharp mask (digital sharpening)` |
| `film_render.convert.scan_illuminant` | `Scan illuminant` in Input and capture → Convert |

The GUI MUST NOT offer a scanner light control. The readout illuminant is a product constant selected by the material category.

The Convert scan-illuminant help MUST identify the light used to acquire an imported RGB negative. It MUST distinguish that declaration from the fixed product readout illuminant and the Camera capture illuminant. Conversion-style choices MUST follow the [lab scan style-name contract](../film-chain/lab-scan/spec.md#tone-and-style-names).

The GUI MUST NOT present `neutral` as a fixed built-in style. It MUST expose the experimental frame setup as its own off-by-default control, labelled experimental, rather than folding it into the style choice.

The GUI MUST explain that `negative conversion` interprets a developed negative as a positive image through the film's own density response. It MUST NOT describe negative conversion as a file conversion, a scan finish, or a positive-scan alias.

The GUI MAY add a short explanatory line grouping the scanner controls by acquisition and conversion. Such a line is explanatory only; it MUST NOT rename the runtime concepts or create a second control taxonomy.

## Color and output labels

The GUI MUST distinguish these boundaries:

- `input color space` and input transfer decoding describe the declared source image;
- `output color space` and output transfer encoding describe the runtime digital output;
- `saving color space` and saving transfer encoding describe file conversion;
- `display transform` and display ICC selection describe viewer-only composition.

The GUI MUST keep `CCTF` when it names the transfer-function setting. The first relevant help text MUST expand it as `color component transfer function`. The GUI MUST NOT replace it with a phrase that hides whether the samples are encoded or linear.

The GUI MUST call the terminal modeled appearance `CMYK ink style`. Its help MUST state that the result remains three-channel RGB and is not a press proof or a separation. `Strength` is reserved for `cmyk.strength`; the control MUST NOT expose paper, ink, TAC, GCR, UCR, halftone, texture, or registration parameters.

The GUI MUST keep `Simulation blend` as the name of the full-result/prepared-source mix. It MUST NOT rename it to opacity, intensity, saturation, or strength.

The GUI MUST keep `Print viewing glare`, `Lens blur (readout optics)`, `Unsharp mask (digital sharpening)`, material grain, and Procedural post-scan grain distinct. Similar visual effects MUST NOT share a label that implies the same physical stage.

Finishing labels MUST follow [common finishing controls](../finishing/spec.md#controls). Brightness MUST NOT imply exposure-value units; the specialized negative-conversion operator MUST retain its own exposure-domain meaning and units.

The unified Grain selector MUST retain the names Spektra, Dye, and Procedural. Its choice descriptions MUST identify Spektra and Dye as material grain and Procedural as post-scan grain appearance. Procedural help MUST identify Film Resolution as part of that appearance algorithm, not scanner lens resolution.

Camera diffusion and Enlarger diffusion MUST retain their distinct optical owners. Halation MUST remain a film-domain effect. Preflash exposure and its independent Magenta and Yellow filter shifts MUST remain printing controls.

Print viewing glare help MUST identify readout stray light, not preflash or a change in material density. Visible placement follows [stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership), not serialized section names.

## Viewer language

The viewer uses these presentation labels:

```text
Original
Modified
Paper back
```

Original and Modified follow [photograph inspection](../gui-product-design/photo-work/spec.md#original--modified). They MUST NOT expose capture, adopt, clear, or reference-render actions.

`Paper back` is a viewer-only virtual rendering generated from the retained image. It is not a second editable material stage and it MUST NOT alter retained, saved, or exported pixels. Help MUST state this boundary before any future label change is considered.

The GUI MUST NOT rename `Paper back` to `Paper surface` or another physical term without a domain decision that defines the represented viewing geometry and updates the owning specification.

The viewer MUST distinguish:

- `fit`: return to fitted zoom without changing pan;
- `reset view`: return to fitted zoom and recenter pan;
- exact zoom percentages: source-pixel-to-device-pixel scale;
- `select`: the Shift or secondary-button drag rectangle that zooms to its region on release.

Viewer hints MUST name the gestures in this vocabulary: click zooms at the pointer, primary-button drag pans, Shift or secondary-button drag selects, and double click fits. A hint MUST NOT claim an effect on saved or exported pixels.

## Image-analysis language

The panel MUST use these labels under [image analysis](../gui-product-design/image-analysis/spec.md):

| English presentation | Simplified Chinese presentation |
| --- | --- |
| Image analysis | 图像分析 |
| Histogram | 直方图 |
| Waveform | 波形 |
| RGB parade | RGB 分列 |
| Vectorscope | 矢量示波器 |
| Tone | 明暗 |
| Full image / Selection | 全图 / 选区 |
| Select area | 选择区域 |
| Analyzing | 正在分析 |
| Outside selection | 选区外 |
| Pixel sample | 像素取样 |

Mode help MUST name the photographic judgment from the owning capability rather than require prior knowledge of the instrument. RGB analysis MUST NOT be labeled Spectrum or Spectral analysis.

Select area MUST remain distinct from the normal viewer select-to-zoom gesture. Hints MUST identify the temporary analysis tool while it is armed.

Linked inspection help MUST explain that pointing at the photograph locates a sample without changing statistics. Outside selection and off-scale wording MUST NOT imply missing image data or confirmed clipping.

## Action language

Actions MUST use one stable verb and one explicit target:

| Action | Meaning |
|---|---|
| `Open` | Open image intake choices without changing their RGB and RAW meanings. |
| `Undo` | Restore the preceding accepted edit transaction. |
| `Redo` | Reapply the next undone edit transaction. |
| `Settings` | Open general application, state-file, display, and diagnostics settings. |
| `Export` | Open the independent Export dialog without starting a render. |
| `Select RGB image…` | Choose and load a standard raster image. |
| `Select RAW file…` | Choose and load a camera RAW file through the RAW path. |
| `Preview` or `PREVIEW` | Render the bounded preview using current runtime controls. |
| `Scan` or `SCAN` | Render the original working resolution using current runtime controls. |
| `Save…` or `SAVE` | Write retained output without rerendering. |
| `Export file…` | Start independent export rendering and writing. |
| `Save preset…` | Capture the current look in a preset. |
| `Save current to file` | Write GUI state to a selected state file. |
| `Load from file` | Load GUI state from a selected state file. |
| `Save current as default` | Replace the startup GUI state. |
| `Restore factory default` | Restore factory GUI state and remove the saved startup default. |
| `reset <module>` | Restore one module's editable controls to its [reset baseline](../gui-workflow/spec.md#section-reset). |

Action labels MAY use uppercase casing. Tooltips and status messages MUST preserve the meanings in this table.

`Save` and `Export` MUST never be described as the same operation. Save uses retained output; Export rerenders independently.

Undo and Redo wording MUST follow [edit history](../gui-workflow/spec.md#edit-history). Their help MUST NOT imply that they reverse file writes or preset-library writes. Opening Settings or Export MUST NOT be described as changing pipeline stages.

## Header and status language

The header MUST show the selected film, selected print material, and exact workflow route. Status ownership follows [Labels and surfaces](../gui-workflow/spec.md#labels-and-surfaces). The exact route string remains available because it identifies the physical sequence and is required for diagnostics.

The header MAY add a direct physical description beside the exact route. The description MUST translate the route without inventing a look category. For example, `Film → print → scan` MAY describe `input > film > print > scan`; `Film print look` MUST NOT replace it.

Normal success status MUST describe the accepted operation and its dimensions or target. Backend and precision detail belongs in Diagnostics unless it explains an actionable limitation. A diagnostic label such as `f64 reference` MUST NOT be the only normal success message.

Live viewer rendering uses the short localized message defined by [Viewer presentation](../gui-workflow/spec.md#viewer-presentation). It MUST NOT duplicate a running-stage message in the header. Work-unit names in diagnostic detail MUST use product language, for example `Film development`, `Grain`, `Printing`, and `Scanning`, not serialized identifiers.

A status that reports route or profile normalization MUST identify which stored controls became inactive or were cleared. It MUST NOT claim that a value was applied when the selected route cannot execute it.

## State, reset, and search language

In row and module state, `Modified` MUST mean that the control differs from its exact reset baseline. It MUST NOT mean that the value differs from the selected preset or startup default. This state is distinct from the viewer's Modified image selection.

Reset-baseline comparison follows [presentation design](../gui-presentation/design.md#factory-projection-and-modified-state). Reset behavior follows [Section reset](../gui-workflow/spec.md#section-reset).

A module reset MUST name its module and the reset baseline in its action label, accessible help, and history description under the core-term table. A grouped settings reset MUST name its settings section instead of a photographic module.

The inspector stage names MUST be `Input and capture`, `Film`, `Print`, `Scan`, and `Output`. A stage jump MUST identify the heading it reaches, not a page it replaces. `Settings` and `Export` MUST name dialogs rather than additional stages.

The interface MUST NOT use MAIN, FILM, PRINT, ADVANCED, or CONFIG as page labels.

`Search controls` MUST label the global control locator under [control search](../gui-workflow/spec.md#control-search). Help MUST state that it matches control labels, module names, and canonical parameter paths across editing surfaces. It MUST NOT describe search as a filter limited to the current stage.

A no-match message MUST include the entered text and MUST NOT imply that parameter values changed.

A result MUST name the control, its owning module, and its stage or dialog destination. Canonical paths MAY appear as secondary technical detail. The action MUST describe navigation to the real editor, not application of a value.

A Settings or Export result MUST explicitly name that dialog. A result MUST NOT present a second editable copy.

An inactive reason MUST identify the route, medium, or capability condition that prevents use. It MUST distinguish a retained value from one that was cleared by required normalization. A disabled value MUST NOT be labeled as applied when its stage does not execute.

## Help-text rules

Help text MUST:

1. state the physical operation or data boundary;
2. state the stage that owns the control;
3. state route or medium conditions when the control is conditional;
4. identify approximate, modeled, or uncalibrated behavior;
5. state Save/Export/viewer-only isolation when relevant;
6. state limitations before promising a visual result;
7. use exact units and canonical parameter terminology.

Help text MUST NOT:

- promise recovery of information absent from the source;
- call a modeled convention measured or physical;
- call a viewer decoration a saved material;
- call a scanner sensor a look preset;
- use a former implementation name as a product concept;
- use color as the only state cue.

## Acceptance

1. Every scanner label and choice matches the scanner contracts and preserves the serialized value.
2. Every visible technical term has a tooltip or linked documentation definition when its meaning is not self-evident.
3. Search result labels, accessible labels, state keys, enum values, and diagnostics remain distinct layers.
4. Save, Export, retained output, and viewer-only state remain distinguishable in labels and help.
5. A terminology review against `CONTEXT.md` rejects any rename that changes operation, domain, stage, or calibration meaning.
6. Presentation-only language changes MUST preserve headless behavior, layout accessibility, Save/Export pixels, and persisted-state contracts.
7. Both languages MUST provide the same editing, validation, search, history, and delivery behavior; a language preference MUST NOT participate in photographic state or pixel identity.
8. Release acceptance MUST include native Simplified Chinese rendering and IME use, language-preference restart behavior, live relabeling with pending edits and work, and bilingual control discovery. English-only headless coverage or untranslated fallback labels MUST NOT establish Simplified Chinese support.
