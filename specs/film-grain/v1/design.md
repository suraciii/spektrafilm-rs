# Film grain V1 design

- V1 is called from the filming stage after film density and DIR chemistry are prepared.
- The composite path samples one density field. The layered path derives sublayer densities from the fitted film model and samples each sublayer independently before recomposition.
- The sampler preserves the reference f64 parameter values and random-number operation order. f32 conversion is not allowed before Poisson or binomial decisions.
- The CPU implementation is the only V1 grain execution implementation. GPU execution resumes at a later compatible stage after the CPU grain result is produced.
- Film-profile validation belongs at pipeline construction so a bad layered profile fails before processing.
