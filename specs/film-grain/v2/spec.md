# Film grain V2

## Contract

- V2 consumes and returns native display-encoded scanner RGB. It does not apply an internal transfer curve or primaries conversion.
- `Analogue` samples the encoded source as a virtual grain texture and composites it over the Film Resolution result.
- `Noise` generates procedural grain with the configured resolution-scaled sampling and effective Amount.
- Film type selects the Negative or Positive optical-resolution behavior. Mode and film type are independent controls.
- The default profile is `35mm250`. Supported named profiles are `8mm50`, `8mm250`, `8mm500`, `16mm50`, `16mm250`, `16mm500`, `35mm50`, `35mm250`, `35mm500`, `65mm50`, `65mm250`, and `65mm500`.
- `custom` exposes Size, Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution overrides within the shared bounds.
- The seed and resolved controls determine a repeatable realization. CPU and WGPU may differ by normal floating-point device arithmetic.
