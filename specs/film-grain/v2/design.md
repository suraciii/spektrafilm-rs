# Film grain V2 design

- V2 runs after scanner encoding and before the final output handoff. Linear export decodes the selected transfer after V2; it does not rerun V2 in linear RGB.
- Profile lookup resolves defaults into one immutable V2 parameter set before dispatch. The CPU model and WGSL implementation consume the same resolved semantic inputs.
- The WGPU resident path owns texture generation, sampling, and composition in its command sequence. If resident eligibility fails, the pipeline uses the CPU V2 stage without changing the requested controls.
- CPU and WGPU share portable math and texture boundaries, but the contract allows f32/device rounding differences.
- V2 does not depend on V1 density layers, Poisson parameters, or film-profile grain chemistry.
