# Standard benchmark configurations

## Purpose

An agent must be able to generate reusable rendering configurations and run a local photograph without authoring simulation parameters first. The same generated settings must support controlled version comparisons and human review.

A standard configuration is a rendering setup, not a new film profile or preset kind. It must use the existing preset and case-configuration contracts. A baseline remains an explicitly selected completed run.

The [machine interface](../../instrument/spec.md#standard-configuration-generation) owns command syntax, output filenames, and generation results. The [system design](../../instrument/design.md#standard-configuration-ownership) owns construction and reuse boundaries.

## User workflow

1. Generate fixtures and standard configurations into a new directory outside the repository.
2. Choose the isolated color-and-tone configuration or the full-appearance configuration.
3. Use a generated suite, or provide a local image with its source declaration and selected seeds.
4. Render the baseline and candidate with the same generated configuration files.
5. Compare retained runs and generate an offline review with the existing commands.

Generation must not render images, run comparisons, choose a baseline, or approve a candidate. It may export a preset through the renderer's public preset command. Natural-image acquisition must remain opt-in.

## Two rendering purposes

### Isolated color and tone

The isolated configuration must disable spatial and stochastic effects explicitly. It must disable grain explicitly. It must disable automatic camera exposure and automatic scan setup. It must use fixed manual exposure.

This configuration must make color and tone changes inspectable without a changed grain realization or spatial spread. It must not claim to represent the preset's complete appearance.

Controlled color and exposure-response cases must use this configuration. Their seed population must be `[0]`.

### Full appearance

The full-appearance configuration must preserve the exported preset's selected grain, halation, diffusion, and other look controls. It must enable spatial and stochastic processing. It must use the same manual exposure and output context as the isolated configuration.

A local photograph must render at its supplied resolution without preview reduction, automatic crop, or automatic resize. The configuration must declare a 35 mm film-format scale. A caller that changes framing, resolution, or film scale must retain that change in the case configuration.

The generated full-appearance cases must use seeds `[0,42,99]`. A caller must select that population explicitly in single-input execution. A single seed remains useful for image review but must not establish statistical grain acceptance.

Full-image color, tone, and FLIP diagnostics must remain separate from dedicated flat-field grain and bright-source halation measurements. A textured photograph must not silently become a flat-field grain probe.

## Default look and output

The initial standard look must use the renderer's explicit built-in `Kodak Gold 200` look preset. The preset must be exported as a materialized portable file. User-library settings must not affect its selection.

Both configurations must use `input > film > print > scan`. Output must be encoded sRGB. The independent CMYK ink style treatment must be disabled. The scanner must use neutral finishing with automatic setup off. Preview and optional film-chain LUT shortcuts must be disabled.

The generator must not construct a synthetic benchmark film profile. The generated look must not enter the user's preset library or alter GUI startup settings.

The standard look is an end-to-end rendering convention. It must not claim physical film fidelity or serve as the only coverage for all film stocks, grain engines, and output routes.

Changing the selected stock or route must create an explicitly authored configuration. Generating arbitrary named looks, selecting a custom preset during generation, and adding a GUI benchmark mode are outside this specification.

## Controlled suites and review coverage

The quick suite must retain the controlled color and exposure-response cases. If the caller acquires the pinned natural scene, the quick suite must retain its isolated color-and-tone scene case.

The complete suite must retain the dedicated grain, halation, and edge cases. Their effect-specific settings must remain explicit rather than inherit the full-appearance preset without adjustment. The existing Procedural custom Noise grain probe must remain an engine-specific diagnostic.

The complete suite must add a distinct `final_appearance` case using the generated chart and full-appearance configuration. With the pinned natural scene, it must also add `natural_appearance` using that source and the full-appearance configuration. These cases must select numeric, color, tone, and FLIP measurements. Existing natural-scene source declarations and regions must remain intact.

The generated small probes and optional natural scene must identify their limited image and scene coverage. A suite filename containing `complete` must not imply comprehensive stock coverage, calibrated acceptance, or full-resolution photographic review. A caller must use representative local photographs for native-resolution spatial review.

## Frozen comparisons

An algorithm comparison must use the same exported preset and configuration bytes on both sides. It must not resolve each renderer version's own same-named built-in preset during the two runs.

Generating a new bundle after an upgrade must remain an explicit setup action. It must not overwrite an existing bundle or completed evidence. The renderer, data directory, exported preset, source images, and resolved settings must retain the existing dependency identities.

A profile, preset, or data change must remain a declared comparison condition. A generated preset must not require the baseline's profile content hash when the caller intentionally compares profile revisions. The selected runtime data must still be hashed by the runner.

Comparing different film materials may require a separately authored common output configuration. [Digital Reference](../../film-chain/digital-reference/spec.md) is one modeled convention for supported negative-film scans. It is not the standard print-scan setup and must not be enabled silently.

No generated configuration may supply universal acceptance thresholds. The existing exploratory policy must remain unqualified. Generation must not create a calibrated policy or update an approved baseline.

## Acceptance

1. Generate the standard files with the real renderer. Validate the exported preset and both configuration documents. Retain the selected preset identity and successful generator output.
2. Place a conflicting `Kodak Gold 200` preset in a user library. Generate again into a new directory. Verify that the exported look still comes from Built-in and that the user library remains unchanged.
3. Move the generated directory. Run both configurations with a local raster through the real renderer. Verify that relative dependencies resolve and that full-appearance rendering preserves input dimensions and the declared film scale.
4. Execute an isolated case twice with the same renderer, data, and configuration. Compare the native output, color, and FLIP measurements. Record repeatability without asserting universal cross-platform or cross-backend identity.
5. Render the full-appearance case with the declared seed population. Verify the resolved effect settings and per-seed evidence. Change only a grain seed and keep pixel differences separate from statistical conclusions.
6. Run the generated quick and complete suites. Verify the specialized probe settings, distinct appearance cases, and case selection. Exercise the optional natural-scene branch with its existing pinned provenance.
7. Compare two renderer targets using one frozen bundle. Verify that both commands consume the same preset bytes. Preserve data and effective-parameter differences rather than hiding them behind the standard look name.
8. Modify or remove a referenced preset file before execution. Reject missing dependencies and preserve changed dependency identity under the existing run rules. Reject an existing generation destination without altering its contents.
9. Fail preset export or configuration validation. Return a structured failure and publish no successful generation manifest. Do not leave a partial bundle identified as ready to run.
10. Generate a comparison and offline report from the new appearance cases and a local photograph. Inspect full-image and native crop evidence. Verify that uncalibrated or absent policy does not produce qualified acceptance.
