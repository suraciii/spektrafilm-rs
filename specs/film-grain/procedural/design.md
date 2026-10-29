# Procedural grain design
- The [Procedural contract](spec.md) owns its post-scan placement. Linear export decodes the selected transfer after Procedural; it does not rerun Procedural in linear RGB.
- A future material-domain algorithm would need its own numerical definition and acceptance; relabeling or moving the existing RGB stage is not that adaptation.
- Profile lookup resolves defaults into one immutable Procedural parameter set before dispatch. The CPU reference and GPU implementation consume the same resolved semantic inputs; backend-specific packing MUST NOT change them.
- WGPU owns texture generation, sampling, and composition in its device command sequence. Backend selection follows [GPU backend selection](../../gpu-execution/spec.md#backend-selection); a fused resident decline MAY use another complete GPU sequence but MUST NOT silently substitute the CPU Procedural stage.
- An explicit GPU request fails before publication when no complete GPU sequence is eligible, while automatic CPU selection is allowed only when no usable hardware GPU exists.
- CPU and WGPU share portable math, texture boundaries, phase/seed addressing, and operation order. Device arithmetic MAY differ only within [GPU numerical alignment](../../gpu-execution/design.md#numerical-alignment), and model-defined random values MUST satisfy [stochastic alignment](../../gpu-execution/design.md#stochastic-alignment).
- Procedural does not depend on Spektra density layers, Poisson parameters, or film-profile grain chemistry.

## Vulkan arithmetic

The Procedural module MUST use a dedicated Vulkan marker.
The marker MUST NOT change Spektra or MT19937 compilation.
Both Analogue and Digital share the Procedural module.

Ordinary float add, subtract, multiply, and divide results, including vector-times-scalar results, MUST retain separate rounding through SPIR-V `NoContraction`.
Explicit fused operations MUST remain unchanged.
This constraint does not define correctly rounded general division or subnormal behavior.

Analogue canvas coordinates MUST use correctly rounded binary32 division before float-bit hashing.
The private divider accepts integer cell coordinates and a positive normal scaled canvas size.

Positive image dimensions with `width * height <= u32::MAX / 3` bound virtual sizes below `2^43`.
A finite raw scale clamped to `[0.5, 1.4]` makes every nonzero coordinate quotient normal and finite.

The host MUST reject non-finite Analogue scales before device allocation.

The host MUST resolve the same binary32 virtual canvas dimensions for the generator, canvas sampling, and mean reduction.

Image-independent Analogue generator coefficients MUST be prepared with the CPU reference's stepwise binary32 expressions.
These coefficients are the two multipliers, the aspect ratio, and the texel size.
Digital MUST retain its image-dependent computation on the device.

The rotation quotient MUST use the integer divider when finite normal operands satisfy the conservative exponent-interior guard.
The guard requires `1 < exponent_a - exponent_b + 127 < 254`.

The device's existing division handles exceptional, subnormal, and exponent-boundary cases.
This repair does not certify those cases as bit-identical to CPU.
