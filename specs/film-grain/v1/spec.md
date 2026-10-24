# Spektra grain
## Contract
- Spektra samples grain from developed film density and returns film density for the remaining print or scan stages.
- Spektra supports composite-density and layered-density sampling. Layered sampling requires the film profile's layer data; each layer keeps its own density and particle scaling.
- The sampler uses the configured particle area, channel scales, density minimum, uniformity, blur, and micro-structure controls. Values remain in f64 where they affect the stochastic sequence.
- The configured random seed controls the sampling stream. Changing the seed MAY change the pattern without changing the requested tone model.
- Missing layer data is invalid when layered sampling is enabled; the implementation MUST NOT silently substitute composite sampling.
- Spektra remains faithful to the reference CPU model. CPU and GPU MUST run the same composite/layered algorithm under [GPU algorithm alignment](../../gpu-execution/spec.md#algorithm-alignment); active Spektra grain MUST run on the selected executor and MUST NOT be replaced by an unfaithful resident approximation or another engine.
