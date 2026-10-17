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

Built-in looks enable their film profile's adaptation surface by default. See the [preset contract](../specs/presets/spec.md). To turn it off for a CLI render, add `--set settings.apply_hanatos2025_adaptation_surface=false`.

## Portable identities and name resolution

Preset names are their portable identities. Core reads schema v1 and v2 documents and exports schema v3. V1 drops its old `id`. Older complete looks acquire the full Simulation blend.

User documents live in `<config_dir>/presets` under the shared `SPEKTRAFILM_CONFIG_DIR` policy. A name resolves in My Presets first, then Built-in. `user:` and `built-in:` select one group.

Choose **American Reminiscence** in the GUI preset browser. You can also pass `--preset 'American Reminiscence'` to `process`. It pairs Kodak Gold 200 with Kodak Portra Endura. Virtual film exposure is +0.5 EV. Auto metering is off. The stored method remains `center_weighted`. The enlarger uses Y shift −3 and M shift 0. Print exposure is 1. Automatic print exposure compensation is enabled. Print density-curve morph is active. Overall and blue gamma factors are 0.95. The other gamma factors are 1. Developer exhaustion is 0. Grain, halation, and print glare are off. The film adaptation surface is on.

Newly saved looks include virtual film exposure and metering. Older look documents without the camera group keep your current values when applied. They do not mark the look modified when those values change. RAW/source exposure correction and input color interpretation remain caller settings. See [look ownership](../specs/presets/spec.md#look-ownership).

Versioned states and recipes permit an absent or null `io.output_gamut_compress.lightness_compression`. A stored triplet must be removed. Current documents and sparse edits reject the retired field even when null. Rejection preserves the active controls and retained output.

## Save, export, and update

A saved look can be exported without an active workflow. Application validates the current scan configuration. CLI composition validates after explicit workflow choices.

Library writes reject names already owned by another document. This includes concurrent writes with different letter case or carriers. Export into My Presets creates a new name. Use Update to replace an existing document.

## See also

- [CLI rendering](cli.md)
- [GUI](gui.md)
- [Grain engines and controls](grain.md)
- [Preset specification](../specs/presets/spec.md)
- [CLI parameter language](../specs/cli/spec.md)
