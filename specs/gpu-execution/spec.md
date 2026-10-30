# GPU execution

## Contract

GPU computation is the primary production execution path. CPU execution remains an explicit reference and a compatibility path when no usable hardware GPU is available. Backend selection MUST NOT change the selected photographic model.

This capability owns computation coverage, backend selection, device residency, CPU/GPU algorithm alignment, and their acceptance. The owning image, film, grain, scanner, output, and GUI capabilities retain their processing semantics, controls, validation, and publication rules.

A supported image-processing configuration MUST have a complete GPU implementation. Missing GPU kernels MUST NOT make a valid configuration a permanently CPU-only feature. Implementation acceptance requires the complete scope below; a subset MUST NOT be described as full-GPU support.

## Computation scope

**Full-GPU computation** means that every image-dependent numerical stage executes on the selected compute device. This includes reductions, iterative solves, stochastic formation, resampling, color transforms, and output sample conversion. Host orchestration and file I/O do not count as image computation.

**GPU-resident processing** means that image samples and generated fields remain on the device between numerical stages. It does not require one command buffer or one submission. A GPU kernel followed by CPU image arithmetic is not full-GPU computation.

The scope MUST cover these production operations:

- RAW demosaicing, sensor corrections required by the selected decoder, camera orientation, selected white balance, lens correction, and transformation to normalized linear input.
- Raster input transfer decoding and color interpretation, full-source exposure metering, crop, rotation, resize, antialiasing, and working pixel-pitch propagation.
- All supported base workflows, including the `input` workflow, film scan, film-print-scan, and every supported convert-film route.
- Both RGB-to-raw methods, selected input and output gamut operations, spectral integration, selected spectral-LUT evaluation, material exposure and development, independently filtered preflash, chemistry, and channel mapping.
- Camera and enlarger diffusion, highlight boost, per-channel halation, DIR including its exponential tails and non-default coefficients, scanner optics, print viewing glare, and unsharp masking.
- All four grain engines in their declared domains, including Spektra's resolved layer model and single-curve case, both Procedural modes and resolution models, Dye field readout, and Fotufilm population formation and readout.
- Every readout-illuminant category, direct scan, negative conversion, and scanner optics.
- Eligible [common finishing](../finishing/spec.md#scope-and-applicability), including its fixed color interpretation and RGB-only policy.
- CMYK ink style, complete-result output mixing, output transfer encoding, and independent saving color and sample conversion.
- GUI viewer resampling, display color and ICC transforms, and disposable viewer composition. These operations MUST NOT change retained or saved image samples.
- Pointwise LUT lattice evaluation through the calibrated runtime, with the existing bake neutralization and delivery rules.

Existing route restrictions and named preview, diagnostic, spatial, stochastic, or LUT bypass rules remain authoritative. GPU selection MUST NOT add a bypass. Save MUST convert retained output without rerendering simulation or regenerating grain.

Material-stage export and continuation currently remain CPU-only. Selected GPU execution MUST reject those boundaries before numerical processing or publication. This implementation gap does not narrow the complete-workflow target above.

Native file reading, container parsing, compressed-sample unpacking, file encoding and compression, metadata handling, hashing, and atomic publication MAY run on the host. A native RAW call that also performs demosaicing, white balance, lens correction, or color conversion MUST NOT be classified entirely as file I/O. Those numerical operations remain in scope.

File encoding here means container, entropy, and format-required codec work. Saving primaries and transfer conversion, output clipping, and requested bit-depth quantization remain GPU sample conversion. Necessary packing of already converted samples MUST NOT reapply those numerical operations on CPU.

The host MAY validate profiles and resolve bounded, image-independent coefficients in reference precision. CPU preparation of display-transform data follows the [GUI display color-management boundary](../gui-workflow/design.md#display-color-management). It MUST NOT precompute image-sized random arrays, grain populations, correction rasters, or per-pixel results and upload them as a substitute for GPU computation.

Bulk spectral-table generation, calibration sampling, and grain preparation that execute numerical workloads MUST also have GPU execution. The [design](design.md#preparation-boundary) defines this boundary.

Reference analysis and profile-optimizer execution are not image-render execution. The benchmark's reference metrics do not have to use GPU arithmetic to judge a GPU render.

## Backend selection

Backend selection follows this order for the production GUI and CLI:

```text
select_backend(request, executable):
    if request == cpu:
        return CPU
    if request == default and executable == spektrafilm-f64:
        return CPU
    if request == gpu:
        return WGPU or fail before output publication
    if a usable hardware WGPU adapter exists:
        return WGPU
    return CPU and report the selection and its reason
```

GPU image computation takes priority over retaining a CPU-only optimization or a preview-only GPU approximation. It does not take priority over the selected algorithm or valid output.

A selected GPU operation MUST run the complete computation scope on WGPU or fail before output publication. It MUST NOT silently fall back for a missing kernel, unsupported parameter, precision limitation, resource limit, or device failure.

After GPU selection it MUST NOT restart the operation on CPU. GUI failure MUST preserve the last accepted output.

A user MAY explicitly retry with CPU.

Hardware usability means successful compute-device initialization within the supported platform and device requirements. A missing implementation for a supported effect MUST NOT be labeled an unavailable adapter to select CPU. Resource exhaustion and model-specific precision failure MUST retain their own execution errors.

Explicit GPU selection in an f64 build MUST still identify actual GPU precision; f64 host storage MUST NOT imply f64 shader arithmetic.

Existing backend flags, environment precedence, GUI selection, and recipe values MUST retain their syntax. This capability adds no backend-specific photographic parameters. Output depth MUST NOT choose computation precision.

Selecting a grain engine or negative conversion MUST NOT select CPU implicitly.

Hardware and software WGPU adapters MUST remain distinguishable. An explicit WGPU request MAY use a software adapter for portability checks. Software execution MUST NOT satisfy hardware-GPU availability, performance, or full-platform acceptance claims.

## Algorithm alignment

CPU and GPU MUST implement the same photographic algorithm. They MUST use the same resolved parameters, data identities, stage order, color domains, spectral grids, interpolation rules, spatial kernels, boundary extension, endpoint continuation, random laws, and validation policy.

CPU and GPU MUST follow the [common finishing response](../finishing/design.md#response-definition) and its [preparation and execution](../finishing/design.md#preparation-and-execution) contract.

GPU execution MUST NOT average channel-specific optical parameters, replace exponential DIR diffusion with Gaussian diffusion, truncate a kernel to a device radius cap, or replace a sampler with Gaussian noise. It MUST NOT substitute a grain engine or use a different inverse or appearance model. The same restrictions apply to a new CPU implementation introduced for alignment.

CPU f64 remains the numerical reference for the accepted model. A GPU port MUST preserve that reference behavior under measured, domain-specific budgets. A change to the reference algorithm requires a separate approved model change; it MUST NOT be hidden inside GPU acceleration.

Universal floating-point bit identity is not required. Parameter choices, discrete decisions, seed interpretation, and random-count decisions MUST agree exactly where the owning algorithm defines them. Floating-point differences MUST satisfy [numerical alignment](design.md#numerical-alignment).

Agreement of grain mean and variance alone MUST NOT establish algorithm alignment.

Failure and cancellation outcomes MUST preserve the owning publication and stale-result contracts. CPU and GPU MUST reject the same invalid model configurations. A GPU device limit MAY cause an explicitly identified execution failure, not a change of image parameters.

## Residency and resources

A full-GPU render MUST NOT materialize an intermediate image on the host to execute another numerical stage. Small reduction results and error flags MAY cross the boundary for orchestration. The host MUST NOT use them to perform image-dependent numerical interpretation that belongs on the device.

An image already resident on the compute device MUST be reused without an image download and reupload solely to cross runtime, stage, saving-conversion, or viewer boundaries. A host-decoded input MAY be uploaded once per required region. A host writer MAY receive the final converted image.

An explicitly requested diagnostic tap or raw dump MAY read its requested samples without changing normal execution.

Resource planning MUST support bounded chunking or tiling when a complete image exceeds a single GPU binding. Tiling MUST preserve optical context, full-source reductions, absolute film coordinates, stochastic identity, and output footprints. It MUST NOT reduce resolution, shorten support, alter grain statistics, or switch algorithms to fit memory.

Unrepresentable dimensions, precision requirements, or minimum working sets MUST fail before publication. Each accepted GPU operation MUST have bounded temporary storage and cancellation boundaries between submitted chunks. Cancellation MUST discard incomplete output; it need not interrupt a submitted GPU pass.

A failed readback mapping or a missing mapping completion MUST return an operation error instead of panicking. A readback failure MUST NOT be reported as a successful dispatch decline, publish partial samples, or replace accepted output.

## Acceptance

Acceptance MUST freeze the CPU reference, input and data identities, effective controls, seeds, stage domains, and comparison budgets before judging GPU candidates. Each budget MUST state its metric, units, population, absolute and relative bounds where applicable, and source. Bounds MUST follow reference calculations, repeatability, or numerical sensitivity.

A missing justified budget means that numerical acceptance is incomplete, not passed.

1. Exercise real GUI Preview, Scan, Export, Save, and viewer operations, CLI process and render, RAW ingestion, and pointwise LUT evaluation. Use the existing application entry points and readable published artifacts.
2. Exercise every supported route, conversion style, grain engine and mode, color-space and gamut family, chemistry branch, spectral-LUT setting, and optical-effect family. Cover relevant combinations, not only isolated kernels.
3. Spektra, active diffusion, standard full-frame negative-to-positive setup and operator correction, declared fixed per-channel styles, non-default DIR, scanner lens blur, and unsharp masking MUST complete without CPU image arithmetic.
4. Compare paired CPU native-f32, CPU f64, and hardware-GPU outputs from identical declared inputs. Compare stage-domain samples and complete outputs.
   Include neutral and chromatic wedges, toe and shoulder, extreme headroom, signed inputs where supported, fractional geometry, edges, and photographic scenes.
   CPU native-f32 is a separate comparison, not the authority for reference-sensitive discrete decisions. A precision comparison MUST identify the exact conditioning values supplied to each sampler.
5. Prove stochastic alignment under the [stochastic contract](design.md#stochastic-alignment). Include zero and nonzero seeds, layered sampling, count-law transition boundaries, field overlap, cropped and tiled reads, and repeated execution.
6. Statistical appearance evidence MUST supplement exact discrete checks and bounded sample comparisons.
7. Prove full-source and tiled execution equivalence. Exercise images larger than a single device binding, wide optical support beyond the historical FIR cap, non-square and portrait geometry, and bounded working sets.
8. Retain seams, boundary, full-source metering, and global grain-coordinate evidence.
9. Exercise explicit CPU, explicit GPU, automatic hardware selection, absent hardware, software WGPU, missing capabilities, device failures, cancellation, and stale GUI results; no failed operation MAY replace published output. An adapter without binary64 MUST run the [declared f32 stages](design.md#devices-without-native-f64) and fail the engines that require binary64 or 64-bit integers explicitly.
10. Retain actual executors and transfers for the complete computation scope, including input numerics, preparation, simulation, saving conversion, and viewer work. A backend name, shader source, dispatch count, fused-path label, or truncated report MUST NOT alone establish full-GPU coverage.
11. Follow the [telemetry contract](../telemetry/spec.md) and its historical path semantics.
12. Run hardware acceptance on the supported Metal, Vulkan, and Windows WGPU APIs. Identify each adapter, driver, executable hash, build precision, reference, invocation, and source data.
13. A missing platform or hardware environment MUST remain unverified. Software Vulkan smoke is a separate portability result.
14. Measure end-to-end and compute-only time, cold preparation, repeated execution, memory and transfer behavior against matched CPU runs on declared image workloads. Hold controls, seeds, output, and reuse conditions constant.
15. Report observations without promising an unmeasured speedup. A resource optimization MUST pass the same alignment checks before cutover.

Specs, compilation, software-adapter runs, and isolated shader tests MUST NOT be reported as implementation acceptance of this contract.
