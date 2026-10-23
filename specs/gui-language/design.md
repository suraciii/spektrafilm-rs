# GUI language design

## Authority

[GUI product language](spec.md#interface-language) owns supported languages, presentation, switching, search, and coverage. [GUI workflow design](../gui-workflow/design.md) owns accepted parameters, worker identities, and persistence of photographic state. [GUI presentation design](../gui-presentation/design.md) owns reusable components and layout. Localization must remain a GUI adapter over those boundaries.

## Technical choices

| Concern | Implementation |
| --- | --- |
| Translation and interpolation | `rust-i18n`, with explicit locale selection per lookup |
| Catalogs | GUI-owned `locales/en.json` and `locales/zh-CN.json`, embedded at compile time |
| System-language discovery | `sys-locale::get_locales()`, followed by the supported-language resolver |
| Chinese font | Bundled, licensed Noto Sans SC, registered through the existing egui font API |
| Preference persistence | Serde JSON in the existing per-user configuration directory, separate from photographic state |

The GUI must use [rust-i18n](https://docs.rs/rust-i18n/latest/rust_i18n/) for catalog embedding, named-argument interpolation, and English fallback. Runtime catalog-file loading must remain disabled. Each lookup must receive the application's resolved locale explicitly; production code and tests must not depend on the library's mutable global locale. Supported-language detection and preference ownership remain application responsibilities.

The platform adapter must use [sys-locale](https://docs.rs/sys-locale/latest/sys_locale/) rather than implement separate native language-discovery bindings. The shared resolver maps its ordered results to the product's two supported languages. `zh-CN` is the Simplified Chinese catalog identifier; it does not restrict Simplified Chinese selection to mainland-China system preferences.

Fluent with `i18n-embed` is not selected: the current English/Simplified Chinese scope does not require its richer grammar system or additional resource configuration. A hand-written translation engine is also rejected. The implementation must not add a template parser, translation backend abstraction, language-pack registry, or general translation-cache framework. Adding another language later may add a catalog, but does not justify those abstractions now.

## Identity before presentation

Controls, actions, sections, choices, and messages must have stable identities independent of rendered text. Existing canonical parameter paths, component identifiers, action identifiers, and typed choice values remain authoritative. Ordinary catalog keys must be stable semantic keys, not English sentences. Each message does not require a custom Rust enum or wrapper type.

Widget IDs, disclosure state, dialog focus restoration, numeric buffers, history ownership, and action dispatch must not depend on translated labels. English identifiers currently used for internal section or action lookup must be separated from their displayed text during the cutover. Renaming the visible Session group to General must migrate its callers and locators without introducing a second settings owner or changing parameter keys. Language receives a GUI-preference locator, not a runtime parameter path.

Visible-label substitution at the paint boundary is insufficient: text also drives existing search, accessibility, edit signals, and cached messages. A second translated control tree is rejected because it would duplicate editing, validation, and reset behavior. Both locales must render the same controls and invoke the same actions through stable identities.

## Message resources

One GUI-owned catalog must provide the English and Simplified Chinese presentations for each message identity. The same identity must supply visible and accessible wording where they express the same meaning. Context-specific messages may differ when their operations differ; a translated word must not collapse Save, Export, preset capture, and startup-state writes into one action.

Messages with values must use the library's named arguments so translations can reorder complete phrases. Application call sites must supply validated domain values and preserve their intended types until formatting. Product sentences must not be assembled from translated fragments or interpolated by replacing English substrings. Formatting must preserve paths, enum tokens, profile identities, units, and numeric precision rather than translate or reparse them. This capability does not introduce locale-dependent numeric parsing or number formatting.

Shipping catalogs must have matching message and placeholder coverage. A missing translation or mismatched argument is a release defect, not an accepted partially English interface. English fallback is defensive behavior for missing resources, not an accepted partially translated release; fallback use must be visible in diagnostics. Catalogs and fonts must be available locally; normal painting must not perform file IO, network access, catalog parsing, or machine translation. Static text must use the library's borrowed lookup result where available. Dynamic text must reuse existing state-change and presentation-cache boundaries; localization alone must not introduce per-message caches or require caching every formatted string.

## Preference ownership and resolution

A typed application preference distinguishes System, English, and Simplified Chinese. The application owns the preference and its derived resolved locale; the library's global locale is not the preference store. The resolved locale must not be persisted as a replacement for System. Discovery and matching follow [Technical choices](#technical-choices) and the [product resolution rules](spec.md#selection-and-persistence). Language normalization and matching must have one shared resolver.

The preference belongs in the existing per-user configuration directory, outside photograph/startup state. It must have one validated reader and writer. Publication must not truncate the last valid preference on a failed write; reuse an existing safe writer where available rather than duplicate it. A missing preference selects System. Invalid or unreadable data must not prevent startup or overwrite the invalid file merely by reading it; the GUI must remain usable under System and expose the preference problem in localized guidance or diagnostics. An explicit user selection may replace that preference through the writer.

Language selection must update presentation independently of preference publication. A write failure must report non-persistence without reverting the current run's display. Neither core RuntimeParams nor GUI photographic-state projection, parameter digests, look presets, render requests, or worker cache keys may acquire a language field. No CLI parameter or legacy GUI-state extension is introduced by this design.

## Live surfaces and dynamic state

A resolved-language change invalidates localized presentation and cached search-result text only. It must not invoke the photographic edit boundary, request a preview, cancel work, clear drafts, or reset normal disclosure. Selecting a language is itself a focus action; returning from Settings must still restore the invoking control by identity, with any preceding draft intact.

Progress, history labels, and application-generated statuses that remain on screen must retain only the semantic identity and arguments needed to render in the current language. Existing typed actions and work-unit identifiers should be reused; a small message representation is needed only where a cached final English string would prevent relabeling. This must not become a new generic event system or require a type for every label. Workers report semantic events and technical error details, not locale-dependent render identities. Changing language while a job runs must relabel its current operation without restarting it or changing result-acceptance guards. Existing raw third-party errors remain verbatim detail behind localized application context.

Enum presentation maps from validated values to localized labels. A selected choice must still dispatch and serialize the original typed value. Material/profile/preset identities and user-authored names must not be replaced with translated display names in selection, file lookup, or history restoration.

## Bilingual search

The existing control inventory remains the sole owner/path index. Its searchable text must include both catalogs' labels and visible alternate labels, plus original canonical paths. Search must reuse the existing case-insensitive English and substring matching semantics; Chinese text must match without spaces between words. General/通用 and both language-selector names must locate their actual Settings owner.

Search entries and result presentation must be separate. Changing locale may refresh displayed destinations and labels, but it must preserve the query and result identities. Only modified must continue to compare canonical values with the same typed factory projection. Language preference modified-state, if shown, must use its application-preference baseline and remain outside photographic history.

## Fonts and packaging

The GUI must register bundled Noto Sans SC as a CJK fallback for every text family that can display Chinese, including diagnostic and mixed-script text. Existing Latin and monospace fonts remain primary where appropriate. The application must not rely on the operating system's installed CJK fonts. Font assets and their licenses must be included by every supported platform package. Any subsetting must retain shipped translations, required punctuation and symbols, and common Simplified Chinese user input; a subset covering only fixed labels is insufficient.

Text measurement must use the actual selected fonts and locale. The existing bounded layout must accommodate both catalogs without widening the inspector into the viewer. IME composition and caret placement must use the native toolkit's text-input path rather than custom key-to-character conversion.

## Delivery boundary

The production GUI must cut over every application-owned text surface, stable control identity, and affected search/accessibility caller together. A partial translated workspace with English-only dialogs, help, or application errors does not satisfy [translation coverage](spec.md#translation-coverage). Existing English tests may retain fixed English fixtures, but locale-specific behavior must enter through the same production controls and workers. Tests must address stable identities when text is not the behavior under test; visible wording and accessibility checks must use the selected catalog.

The implementation must retain the current toolkit, numerical models, accepted document schemas, and validator rules. No general-purpose language-pack framework or parallel preference system is required. Native rendering and IME evidence, catalog completeness, and language-independent edit/delivery behavior are distinct acceptance obligations; a catalog check alone cannot establish a localized application.
