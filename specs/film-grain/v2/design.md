# Procedural grain design
- Procedural runs after scanner encoding and before the final output handoff. Linear export decodes the selected transfer after Procedural; it does not rerun Procedural in linear RGB.
- Profile lookup resolves defaults into one immutable Procedural parameter set before dispatch. The CPU model and WGSL implementation consume the same resolved semantic inputs.
- The WGPU resident path owns texture generation, sampling, and composition in its command sequence. If resident eligibility fails, the pipeline uses the CPU Procedural stage without changing the requested controls.
- CPU and WGPU share portable math and texture boundaries, but the contract allows f32/device rounding differences.
- Procedural does not depend on Spektra density layers, Poisson parameters, or film-profile grain chemistry.
