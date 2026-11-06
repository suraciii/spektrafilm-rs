# Presets

Use this page to discover portable presets, compose them for a render, and manage the user library. The [preset contract](../specs/presets/spec.md) defines preset behavior.

## Discover presets and controls

```bash
# Discover portable presets and editable grain controls
./target/release/spektrafilm preset list --source all --format text
./target/release/spektrafilm preset show 'Kodak Portra 400' --data-dir data
./target/release/spektrafilm describe --module film_render.grain --format json
```

## Compose a render

`process` accepts an exact preset name or a TOML/JSON preset file through `--preset`. Repeat `--preset` to apply documents in appearance order. A look selects profiles and replaces look controls.

Repeat `--set` to apply sparse TOML/JSON files or comma-separated `PATH=VALUE` assignments after the presets. String and enum values need no JSON quotes. Use `--set film_render.grain.engine=procedural` or `--set 'io.output_color_space=ProPhoto RGB'`.

Use these value forms:

- `true`/`false` for booleans.
- Numbers for numeric controls.
- `[1,2,3]` for arrays.
- `null` to clear optional controls.

The [CLI parameter language](../specs/cli/spec.md) defines editable paths and composition.

Built-in looks enable their film profile's adaptation surface by default, except Classic Neg Inspired, which preserves its authored look with the surface off. See the [preset contract](../specs/presets/spec.md). To turn it off for a CLI render, add `--set settings.apply_hanatos2025_adaptation_surface=false`.

## Portable identities and name resolution

Core migrates versioned legacy documents and exports schema v7. Older complete looks gain neutral tone and color adjustments, while sparse sections keep omitted controls unchanged. Current complete looks include every `finishing` control. Retired spellings retain their own migration boundaries; changing a preset version does not reopen them.

User documents live in `<config_dir>/presets` under the shared `SPEKTRAFILM_CONFIG_DIR` policy. A name resolves in My Presets first, then Built-in. `user:` and `built-in:` select one group.

Choose **American Reminiscence** in the GUI preset browser. You can also pass `--preset 'American Reminiscence'` to `process`. It pairs Kodak Gold 200 with Kodak Portra Endura. Virtual film exposure is +0.5 EV. Auto metering is off. The stored method remains `center_weighted`. The enlarger uses Y shift −3 and M shift 0. Print exposure is 1. Automatic print exposure compensation is enabled. Print density-curve morph is active. Overall and blue gamma factors are 0.95. The other gamma factors are 1. Developer exhaustion is 0. Grain, halation, and print glare are off. The film adaptation surface is on.

Choose **Classic Neg Inspired** in the GUI preset browser or pass `--preset 'built-in:Classic Neg Inspired'` to `process`. This independently authored look pairs Fujifilm X-Tra 400 with Crystal Archive Type II; it is not an official Fujifilm simulation. It uses original-strength Spektra grain with RMS granularity 4 in each record. Print channel gamma factors are red 1.08, green 0.94, and blue 1.02. Tone & Color uses brightness +0.5, contrast +0.1, neutral warmth, and saturation −0.2; Expand is off.

The look sets virtual film exposure to 0 EV with auto metering off. Automatic print calibration remains on with a print-exposure multiplier of 0.6 and zero M/Y shifts. Treat 0.6 as a starting point for each photograph: lower print exposure makes this reflection print lighter, and higher exposure makes it darker. Applying the look preserves your source white balance, input settings, image size, and output settings.

Newly saved looks include virtual film exposure and metering. Older look documents without the camera group keep your current values when applied. They do not mark the look modified when those values change. RAW/source exposure correction and input color interpretation remain caller settings. See [look ownership](../specs/presets/spec.md#look-ownership).

Tone & Color belongs to the look independently of virtual exposure and scanner operator correction. Applying a complete look replaces it; changing film, paper, or route keeps the stored values. The independent `finishing` section remains available to CLI composition and scoped photograph copying, not as another GUI preset browser.

Versioned states and recipes permit an absent or null `io.output_gamut_compress.lightness_compression`. A stored triplet must be removed. Current documents and sparse edits reject the retired field even when null. Rejection preserves the active controls and retained output.

## Save, export, and update

A saved look can be exported without an active workflow. Application validates the current scan configuration. CLI composition validates after explicit workflow choices.

Library writes reject names already owned by another document. This includes concurrent writes with different letter case or carriers. Export into My Presets creates a new name. Saving the current look under an existing My Presets name offers an explicit **Replace "name"** action instead of overwriting silently.

## See also

- [CLI rendering](cli.md)
- [GUI](gui.md)
- [Grain engines and controls](grain.md)
- [Preset specification](../specs/presets/spec.md)
- [CLI parameter language](../specs/cli/spec.md)
