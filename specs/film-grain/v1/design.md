# Spektra grain design
- Spektra is called from the filming stage after film density and DIR chemistry are prepared.
- The composite path samples one density field. The layered path derives sublayer densities from the fitted film model and samples each sublayer independently before recomposition.
- The sampler preserves the reference f64 parameter values and random-number operation order under [stochastic alignment](../../gpu-execution/design.md#stochastic-alignment). f32 conversion is not allowed before Poisson or binomial decisions; a device without native f64 must use exact integer or extended-precision conditioning or fail under [numerical alignment](../../gpu-execution/design.md#numerical-alignment).
- GPU must execute the same composite/layered algorithm with the same layer and channel identities under [numerical alignment](../../gpu-execution/design.md#numerical-alignment). GPU execution must not satisfy full-GPU scope with CPU image-stage work.
- Film-profile validation belongs at pipeline construction so a bad layered profile fails before processing.
