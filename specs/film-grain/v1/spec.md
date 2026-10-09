# Film grain V1

## Contract

- V1 samples grain from developed film density and returns film density for the remaining print or scan stages.
- V1 supports composite-density and layered-density sampling. Layered sampling requires the film profile's layer data; each layer keeps its own density and particle scaling.
- The sampler uses the configured particle area, channel scales, density minimum, uniformity, blur, and micro-structure controls. Values remain in f64 where they affect the stochastic sequence.
- The configured random seed controls the sampling stream. Changing the seed may change the pattern without changing the requested tone model.
- Missing layer data is invalid when layered sampling is enabled; the implementation must not silently substitute composite sampling.
- V1 remains faithful to the reference CPU model. WGPU preview and export route active V1 grain through the CPU stage rather than an unfaithful resident approximation.
