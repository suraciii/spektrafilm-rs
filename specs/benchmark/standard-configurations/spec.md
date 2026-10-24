# Standard benchmark configurations

## Purpose

An agent MUST be able to generate reusable rendering configurations and run a local photograph without authoring simulation parameters first. The same generated settings MUST support controlled version comparisons and human review.

A standard configuration is a rendering setup, not a new film profile or preset kind. It MUST use the existing preset and case-configuration contracts. A baseline remains an explicitly selected completed run.

The [machine interface](../../instrument/spec.md#standard-configuration-generation) owns command syntax, output filenames, and generation results. The [system design](../../instrument/design.md#standard-configuration-ownership) owns construction and reuse boundaries.

## User workflow

1. Generate fixtures and standard configurations into a new directory outside the repository.
2. Choose the isolated color-and-tone configuration or the full-appearance configuration.
3. Use a generated suite, or provide a local image with its source declaration and selected seeds.
4. Render the baseline and candidate with the same generated configuration files.
5. Compare retained runs and generate an offline review with the existing commands.

Generation MUST NOT render images, run comparisons, choose a baseline, or approve a candidate. It MAY export a preset through the renderer's public preset command. Natural-image acquisition MUST remain opt-in.

## Two rendering purposes

### Isolated color and tone

The isolated configuration MUST disable spatial and stochastic effects explicitly. It MUST disable grain explicitly. It MUST disable automatic camera exposure and automatic scan setup.

It MUST use fixed manual exposure.

This configuration MUST make color and tone changes inspectable without a changed grain realization or spatial spread. It MUST NOT claim to represent the preset's complete appearance.

Controlled color and exposure-response cases MUST use this configuration. Their seed population MUST be `[0]`.

### Full appearance

The full-appearance configuration MUST preserve the exported preset's selected grain, halation, diffusion, and other look controls. It MUST enable spatial and stochastic processing. It MUST use the same manual exposure and output context as the isolated configuration.

A local photograph MUST render at its supplied resolution without preview reduction, automatic crop, or automatic resize. The configuration MUST declare a 35 mm film-format scale. A caller that changes framing, resolution, or film scale MUST retain that change in the case configuration.

The generated full-appearance cases MUST use seeds `[0,42,99]`. A caller MUST select that population explicitly in single-input execution. A single seed remains useful for image review but MUST NOT establish statistical grain acceptance.

Full-image color, tone, and FLIP diagnostics MUST remain separate from dedicated flat-field grain and bright-source halation measurements. A textured photograph MUST NOT silently become a flat-field grain probe.

## Default look and output

The initial standard look MUST use the renderer's explicit built-in `Kodak Gold 200` look preset. The preset MUST be exported as a materialized portable file. User-library settings MUST NOT affect its selection.

Both configurations MUST use `input > film > print > scan`. Output MUST be encoded sRGB. The independent CMYK ink style treatment MUST be disabled.

The scanner MUST use its default lamp and direct scan output. Preview and optional film-chain LUT shortcuts MUST be disabled.

The generator MUST NOT construct a synthetic benchmark film profile. The generated look MUST NOT enter the user's preset library or alter GUI startup settings.

The standard look is an end-to-end rendering convention. It MUST NOT claim physical film fidelity or serve as the only coverage for all film stocks, grain engines, and output routes.

Changing the selected stock or route MUST create an explicitly authored configuration. Generating arbitrary named looks, selecting a custom preset during generation, and adding a GUI benchmark mode are outside this specification.

## Controlled suites and review coverage

The quick suite MUST retain the controlled color and exposure-response cases. If the caller acquires the pinned natural scene, the quick suite MUST retain its isolated color-and-tone scene case.

The complete suite MUST retain the dedicated grain, halation, and edge cases. Their effect-specific settings MUST remain explicit rather than inherit the full-appearance preset without adjustment. The existing Procedural custom Noise grain probe MUST remain an engine-specific diagnostic.

The complete suite MUST add a distinct `final_appearance` case using the generated chart and full-appearance configuration. With the pinned natural scene, it MUST also add `natural_appearance` using that source and the full-appearance configuration. These cases MUST select numeric, color, tone, and FLIP measurements.

Existing natural-scene source declarations and regions MUST remain intact.

The generated small probes and optional natural scene MUST identify their limited image and scene coverage. A suite filename containing `complete` MUST NOT imply comprehensive stock coverage, calibrated acceptance, or full-resolution photographic review. A caller MUST use representative local photographs for native-resolution spatial review.

## Frozen comparisons

An algorithm comparison MUST use the same exported preset and configuration bytes on both sides. It MUST NOT resolve each renderer version's own same-named built-in preset during the two runs.

Generating a new bundle after an upgrade MUST remain an explicit setup action. It MUST NOT overwrite an existing bundle or completed evidence. The renderer, data directory, exported preset, source images, and resolved settings MUST retain the existing dependency identities.

A profile, preset, or data change MUST remain a declared comparison condition. A generated preset MUST NOT require the baseline's profile content hash when the caller intentionally compares profile revisions. The selected runtime data MUST still be hashed by the runner.

Comparing different film materials MAY require a separately authored common output configuration. That configuration is authored per comparison and MUST NOT be enabled silently.

No generated configuration MAY supply universal acceptance thresholds. The existing exploratory policy MUST remain unqualified. Generation MUST NOT create a calibrated policy or update an approved baseline.

## Acceptance

1. Generate the standard files with the real renderer. Validate the exported preset and both configuration documents.
2. Retain the selected preset identity and successful generator output.
3. Place a conflicting `Kodak Gold 200` preset in a user library. Generate again into a new directory.
4. Verify that the exported look still comes from Built-in and that the user library remains unchanged.
5. Move the generated directory. Run both configurations with a local raster through the real renderer.
6. Verify that relative dependencies resolve and that full-appearance rendering preserves input dimensions and the declared film scale.
7. Execute an isolated case twice with the same renderer, data, and configuration. Compare the native output, color, and FLIP measurements.
8. Record repeatability without asserting universal cross-platform or cross-backend identity.
9. Render the full-appearance case with the declared seed population. Verify the resolved effect settings and per-seed evidence.
10. Change only a grain seed and keep pixel differences separate from statistical conclusions.
11. Run the generated quick and complete suites. Verify the specialized probe settings, distinct appearance cases, and case selection.
12. Exercise the optional natural-scene branch with its existing pinned provenance.
13. Compare two renderer targets using one frozen bundle. Verify that both commands consume the same preset bytes.
14. Preserve data and effective-parameter differences rather than hiding them behind the standard look name.
15. Modify or remove a referenced preset file before execution. Reject missing dependencies and preserve changed dependency identity under the existing run rules.
16. Reject an existing generation destination without altering its contents.
17. Fail preset export or configuration validation. Return a structured failure and publish no successful generation manifest.
18. Do not leave a partial bundle identified as ready to run.
19. Generate a comparison and offline report from the new appearance cases and a local photograph. Inspect full-image and native crop evidence.
20. Verify that uncalibrated or absent policy does not produce qualified acceptance.
