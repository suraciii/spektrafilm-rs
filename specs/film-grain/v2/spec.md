# Film grain V2

## Contract

- V2 consumes and returns native display-encoded scanner RGB. It does not apply an internal transfer curve or primaries conversion.
- `Analogue` samples the encoded source as a virtual grain texture and composites it over the Film Resolution result.
- `Noise` is a single-pass image-resolution path. It generates procedural grain from the prepared input and does not execute Film Resolution; `v2_resolution_factor` and `v2_resolution_type` therefore do not alter Noise output.
- Film type is profile metadata and does not select the filter. `v2_resolution_type` (`0` OpticalResolution, `1` FastBlur) selects Film Resolution for Analogue; mode and film type remain independent controls.
- `v2_timer` overrides the seed-derived canonical phase when nonzero. Legacy `0.0` is the seed-derived phase sentinel; the internal model may represent an explicit phase as `Option<f32>`.
- The default profile is `35mm250`. Supported named profiles are `8mm50`, `8mm250`, `8mm500`, `16mm50`, `16mm250`, `16mm500`, `35mm50`, `35mm250`, `35mm500`, `65mm50`, `65mm250`, and `65mm500`.
- `custom` exposes Size, Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution overrides within the shared bounds.
- The seed and resolved controls determine a repeatable realization. CPU and WGPU may differ by normal floating-point device arithmetic.
- The photo V2 API has no overscan or damage-mask input; mask-modulated grain is outside this contract.
