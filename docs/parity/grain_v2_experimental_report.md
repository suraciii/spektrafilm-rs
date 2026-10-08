# Grain-v2 experimental acceptance report

Date: 2026-10-08

## Binding

- Upstream contract: `andreavolpato/spektrafilm@28bf883e1672e884307edc75852549376e13644e`
- Verified checkout: `/tmp/spektrafilm-upstream-28bf`, `git rev-parse HEAD` = `28bf883e1672e884307edc75852549376e13644e`
- Historical `main/3bb2c2d` GUI/parity evidence was not reused.
- Rust implementation commit: `5389036c6d73f156528ea5c27012a80acec66659` (`Align grain controls with experimental contract`)

## Delivered contract

`FILM` now has one `Grain` section. Its direct controls are `Active` and `RMS granularity (R, G, B)`. The section contains three default-collapsed subsections in order:

1. `pixel statistics`
2. `texture`
3. `micro substructure`

The canonical serialized/runtime fields are:

`active`, `rms_granularity`, `density_min`, `uniformity`, `particle_scale_sublayers`, `blur`, `mult_usm_sigma`, `mult_usm_amount`, `blur_dye_clouds_um`, `micro_structure`.

Legacy/internal fields are excluded from state/preset serialization. Explicit migration accepts `particle_scale_layers`, maps it to `particle_scale_sublayers`, then removes the legacy/internal keys.

## Fixture and state hashes

- Fixture descriptor: `docs/parity/grain_v2_experimental_fixture.json`
  - SHA-256: `7197e6317d8c6bb86348f6867f206d56d75a1cc802097caffdd5fd570d808603`
- Fresh GUI state: `crates/spektrafilm-gui/src/factory_state.json`
  - SHA-256: `913027877ca2f8404a43bd88b3260e3569c4da5aae5fa1778ccfab993bc45994`
- Fixture input used for the runtime probe: `/tmp/grain-v2-runtime/input.tif`
  - SHA-256: `d2c166a5e1c82f95eb94250d6c543941e0a14170b27466c65dce7780902e5a44`
- Fixture parameter JSON: `/tmp/grain-v2-runtime/params.json`
  - SHA-256: `fe4c7e3bfb7828d4b26cb45926c43ee876c371ef644a0e541c82853221d7e477`

## Checks run

```text
PKG_CONFIG_PATH=/tmp/slipstream-pkgconfig:/usr/lib/x86_64-linux-gnu/pkgconfig \
  cargo test -p spektrafilm-core -p spektrafilm-model -p spektrafilm-gui \
             -p spektrafilm-cli --features precision-f64
cargo test: 188 passed (9 suites, 0.00s)
```

Covered behavior:

- canonical defaults and wire keys;
- GUI state migration and canonical state/runtime round-trip;
- RMS, pixel-statistics, texture, and micro-substructure output changes;
- `active=false` density identity;
- runtime-only sublayer flags do not select a second grain model;
- positive-profile and monochrome layered paths;
- deterministic CPU runtime fixture output.

Rust runtime probe:

- raw output: `/tmp/grain-v2-runtime/output.f64`
- SHA-256: `2195e861fdc59ec8dadb0b297b0d150363a44a11c2150d41e62864ef6088bc23`

## Native GUI observation

Command shape:

```bash
xvfb-run -a -s '-screen 0 1600x1100x24' \
  python3 .scratch/grain_v2_gui_probe.py
```

The probe launched the real `target/debug/spektrafilm-gui` under X11/llvmpipe, selected `FILM`, and captured the actual native surface. Observed artifacts are under `/tmp/slipstream-grain-v2-native/`:

- `grain_expanded.png` — one `Grain` section and the three collapsed subsection headings;
- `pixel_statistics.png` — `[6,8,10]`, `[0.03,0.03,0.03]`, `[0.97,0.97,0.97]`, `[1,0.5,0.25]`;
- `texture.png` — `0.89`, `1.5`, `0.7`;
- `micro_substructure.png` — `2.0`, `[0.2,30.0]`.

Screenshot SHA-256 values:

```text
grain_expanded.png       f65dfdf505d62c14b1da79bb73d15aaaf7055690e52521b198824abfa0194bdf
pixel_statistics.png     4b06e9802bc6c2a84dedcb79cd34fa4a45f657406f9c07cefb93799a7d9166d1
texture.png              d6634c09e4898c40fbbfdfc7e3e28c3b50bbb6b8789009a0266c2fda7a00c883
micro_substructure.png   c520869f3cee6e6593d80099e9406725c490d72df0a7b46bb62489109b85ab49
```

## Upstream runtime observation

The same 16×12 linear-RGB fixture and canonical state were executed against the verified upstream checkout with the pinned Python environment. Variant output record:

- record file SHA-256: `9a04ac685c0e13219a0c9a91b3ed9f841417b821add594aa4df94988fda99e22`
- upstream default output SHA-256: `c90633565050cbf26512f044713568c2e45e4ec7a048101380e6bb77e6cf26cc`
- variants recorded: `active_false`, `rms_changed`, `statistics_changed`, `texture_changed`, `micro_changed`

Each canonical parameter group changed the upstream rendered output. The Rust tests independently assert the same group-level effect and the disabled identity.

The historical Rust active-off control comparison changed its fixture by `max_abs=0.0219760838`, `mean_abs=0.0011866505` (`/tmp/grain-v2-runtime/output-off.f64`, SHA-256 `463a404a15a29869ae4d0485ea0b0dc3748f2c6e533c6a02732dee7f2c13c0b9`).

Correction (2026-10-08): the recorded cross-runtime differences (`max_abs=0.765061701` and `0.743085617`) did not compare matching execution contracts. The upstream probe assigned the obsolete `io.scan_film` attribute, while experimental dispatches through `workflow.route`; upstream therefore printed while Rust scanned film. The alleged active-off comparison also used an upstream grain-enabled output. These measurements cannot establish a runtime regression or parity. Replace them with the six explicitly matched routes in `scripts/parity/experimental_runtime.py`; this historical grain report does not establish full-pipeline numerical parity.
