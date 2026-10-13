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

`process` accepts an exact preset name or a TOML/JSON preset file through `--preset`. Repeat `--preset` to apply documents in appearance order. A look selects profiles and replaces look controls. A section edits only its supplied controls.

Repeat `--set` to apply sparse TOML/JSON files or comma-separated `PATH=VALUE` assignments after the presets. String and enum values need no JSON quotes. Use `--set film_render.grain.engine=procedural` or `--set 'io.output_color_space=ProPhoto RGB'`.

Use these value forms:

- `true`/`false` for booleans.
- Numbers for numeric controls.
- `[1,2,3]` for arrays.
- `null` to clear optional controls.

The [CLI parameter language](../specs/cli-parameters/spec.md) defines editable paths and composition.

Built-in looks enable their film profile's adaptation surface by default. See the [preset contract](../specs/presets/spec.md). To turn it off for a CLI render, add `--set settings.apply_hanatos2025_adaptation_surface=false`.

## Portable identities and name resolution

Preset names are their portable identities. Schema v1 documents remain readable; their old `id` is ignored. Export uses schema v2.

User documents live in `<config_dir>/presets` under the shared `SPEKTRAFILM_CONFIG_DIR` policy. A name resolves in My Presets first, then Built-in. `user:` and `built-in:` select one group.

Section documents contain sparse editable leaves from exactly one look section. Application retains omitted controls. Explicit JSON null edits require JSON export. TOML cannot represent those edits.

## Save, export, and update

Section files can be saved and exported without an active workflow. Their application validates the current scan configuration. CLI composition validates after explicit workflow choices.

Library writes reject names already owned by another document. This includes concurrent writes with different letter case or carriers. Export into My Presets creates a new name. Use Update to replace an existing document.

## See also

- [CLI rendering](cli.md)
- [GUI](gui.md)
- [Grain engines and controls](grain.md)
- [Preset specification](../specs/presets/spec.md)
- [CLI parameter language](../specs/cli-parameters/spec.md)
