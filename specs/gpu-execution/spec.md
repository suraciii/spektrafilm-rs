# GPU execution

## Contract

GPU computation is the primary production execution path. CPU execution remains an explicit reference and a compatibility path when no usable hardware GPU is available. Backend selection must not change the selected photographic model.

This capability owns computation coverage, backend selection, device residency, CPU/GPU algorithm alignment, and their acceptance. The owning image, film, grain, scanner, output, and GUI capabilities retain their processing semantics, controls, validation, and publication rules.

A supported image-processing configuration must have a complete GPU implementation. Missing GPU kernels must not make a valid configuration a permanently CPU-only feature. Implementation acceptance requires the complete scope below; a subset must not be described as full-GPU support.

## Computation scope

**Full-GPU computation** means that every image-dependent numerical stage executes on the selected compute device. This includes reductions, iterative solves, stochastic formation, resampling, color transforms, and output sample conversion. Host orchestration and file I/O do not count as image computation.

**GPU-resident processing** means that image samples and generated fields remain on the device between numerical stages. It does not require one command buffer or one submission. A GPU kernel followed by CPU image arithmetic is not full-GPU computation.

The scope must cover these production operations:

- RAW demosaicing, sensor corrections required by the selected decoder, camera orientation, selected white balance, lens correction, and transformation to normalized linear input.
- Raster input transfer decoding and color interpretation, full-source exposure and scanner-setup measurements, crop, rotation, resize, antialiasing, and working pixel-pitch propagation.
- All supported base workflows, including the `input` workflow, film scan, film-print-scan, and every supported convert-film route.
- Both RGB-to-raw methods, selected input and output gamut operations, spectral integration, selected spectral-LUT evaluation, material exposure and development, independently filtered preflash, chemistry, and channel mapping.
- Camera and enlarger diffusion, highlight boost, per-channel halation, DIR including its exponential tails and non-default coefficients, scanner optics, print viewing glare, and unsharp masking.
- All four grain engines in their declared domains, including Spektra composite and layered sampling, both Procedural modes and resolution models, Dye field readout, and Fotufilm population formation and readout.
- Every supported receiver, direct scan, negative conversion method including Digital Reference, film points, scanner correction, per-frame setup, and scanner finishing.
- CMYK ink style, complete-result output mixing, output transfer encoding, and independent saving color and sample conversion.
- GUI viewer resampling, display color and ICC transforms, and disposable viewer composition. These operations must not change retained or saved image samples.
- Pointwise LUT lattice evaluation through the calibrated runtime, with the existing bake neutralization and delivery rules.

Existing route restrictions and named preview, diagnostic, spatial, stochastic, or LUT bypass rules remain authoritative. GPU selection must not add a bypass. Save must convert retained output without rerendering simulation or regenerating grain.

Native file reading, container parsing, compressed-sample unpacking, file encoding and compression, metadata handling, hashing, and atomic publication may run on the host. A native RAW call that also performs demosaicing, white balance, lens correction, or color conversion must not be classified entirely as file I/O. Those numerical operations remain in scope.

File encoding here means container, entropy, and format-required codec work. Saving primaries and transfer conversion, output clipping, and requested bit-depth quantization remain GPU sample conversion. Necessary packing of already converted samples must not reapply those numerical operations on CPU.

The host may validate profiles and resolve bounded, image-independent coefficients in reference precision. It must not precompute image-sized random arrays, grain populations, correction rasters, or per-pixel results and upload them as a substitute for GPU computation. Bulk spectral-table generation, calibration sampling, and grain preparation that execute numerical workloads must also have GPU execution. The [design](design.md#preparation-boundary) defines this boundary.

Reference analysis and profile-optimizer execution are not image-render execution. The benchmark's reference metrics do not have to use GPU arithmetic to judge a GPU render.

## Backend selection

The default production GUI and native-f32 CLI must prefer a usable hardware WGPU adapter for Preview, Scan, Export, process, render, and runtime LUT evaluation. GPU image computation takes priority over retaining a CPU-only optimization or a preview-only GPU approximation. It does not take priority over the selected algorithm or valid output.

An explicit GPU request must run the complete computation scope on WGPU or fail before output publication. It must not silently fall back for a missing kernel, unsupported parameter, precision limitation, resource limit, or device failure. GUI failure must preserve the last accepted output.

Automatic selection may choose the complete CPU path when no usable hardware GPU is available. It must report that selection and its reason. After a GPU operation is selected, an unsupported stage or device failure must fail the operation rather than restart it on CPU. A user may explicitly retry with CPU.

Hardware usability means successful compute-device initialization within the supported platform and device requirements. A missing implementation for a supported effect must not be labeled an unavailable adapter to select CPU. Resource exhaustion and model-specific precision failure must retain their own execution errors.

An explicit CPU request must remain supported. The `spektrafilm-f64` reference executable must retain its CPU default. Explicit GPU selection in an f64 build must still identify actual GPU precision; f64 host storage must not imply f64 shader arithmetic.

Existing backend flags, environment precedence, GUI selection, and recipe values must retain their syntax. This capability adds no backend-specific photographic parameters. Output depth must not choose computation precision. Selecting a grain engine or negative conversion must not select CPU implicitly.

Hardware and software WGPU adapters must remain distinguishable. An explicit WGPU request may use a software adapter for portability checks. Software execution must not satisfy hardware-GPU availability, performance, or full-platform acceptance claims.

## Algorithm alignment

CPU and GPU must implement the same photographic algorithm. They must use the same resolved parameters, data identities, stage order, color domains, spectral grids, interpolation rules, spatial kernels, boundary extension, endpoint continuation, random laws, and validation policy.

GPU execution must not average channel-specific optical parameters, replace exponential DIR diffusion with Gaussian diffusion, truncate a kernel to a device radius cap, replace a sampler with Gaussian noise, substitute a grain engine, or use a different inverse or appearance model. The same restrictions apply to a new CPU implementation introduced for alignment.

CPU f64 remains the numerical reference for the accepted model. A GPU port must preserve that reference behavior under measured, domain-specific budgets. A change to the reference algorithm requires a separate approved model change; it must not be hidden inside GPU acceleration.

Universal floating-point bit identity is not required. Parameter choices, discrete decisions, seed interpretation, and random-count decisions must agree exactly where the owning algorithm defines them. Floating-point differences must satisfy [numerical alignment](design.md#numerical-alignment). Agreement of grain mean and variance alone must not establish algorithm alignment.

Failure and cancellation outcomes must preserve the owning publication and stale-result contracts. CPU and GPU must reject the same invalid model configurations. A GPU device limit may cause an explicitly identified execution failure, not a change of image parameters.

## Residency and resources

A full-GPU render must not materialize an intermediate image on the host to execute another numerical stage. Small reduction results and error flags may cross the boundary for orchestration. The host must not use them to perform image-dependent numerical interpretation that belongs on the device.

An image already resident on the compute device must be reused without an image download and reupload solely to cross runtime, stage, saving-conversion, or viewer boundaries. A host-decoded input may be uploaded once per required region. A host writer may receive the final converted image. An explicitly requested diagnostic tap or raw dump may read its requested samples without changing normal execution.

Resource planning must support bounded chunking or tiling when a complete image exceeds a single GPU binding. Tiling must preserve optical context, full-source reductions, absolute film coordinates, stochastic identity, and output footprints. It must not reduce resolution, shorten support, alter grain statistics, or switch algorithms to fit memory.

Unrepresentable dimensions, precision requirements, or minimum working sets must fail before publication. Each accepted GPU operation must have bounded temporary storage and cancellation boundaries between submitted chunks. Cancellation must discard incomplete output; it need not interrupt a submitted GPU pass.

## Acceptance

Acceptance must freeze the CPU reference, input and data identities, effective controls, seeds, stage domains, and comparison budgets before judging GPU candidates. Each budget must state its metric, units, population, absolute and relative bounds where applicable, and source. Bounds must follow reference calculations, repeatability, or numerical sensitivity. A missing justified budget means that numerical acceptance is incomplete, not passed.

1. Exercise real GUI Preview, Scan, Export, Save, and viewer operations, CLI process and render, RAW ingestion, and pointwise LUT evaluation. Use the existing application entry points and readable published artifacts.
2. Exercise every supported route, receiver, conversion method, grain engine and mode, color-space and gamut family, chemistry branch, spectral-LUT setting, and optical-effect family. Cover relevant combinations, not only isolated kernels. Default Spektra, active diffusion, negative conversion, and non-default DIR and slide points must complete without CPU image arithmetic.
3. Compare paired CPU native-f32, CPU f64, and hardware-GPU outputs from identical declared inputs. Compare stage-domain samples and complete outputs. Include neutral and chromatic wedges, toe and shoulder, extreme headroom, signed inputs where supported, fractional geometry, edges, and photographic scenes.
   CPU native-f32 is a separate comparison, not the authority for reference-sensitive discrete decisions. A precision comparison must identify the exact conditioning values supplied to each sampler.
4. Prove stochastic alignment under the [stochastic contract](design.md#stochastic-alignment). Include zero and nonzero seeds, layered sampling, count-law transition boundaries, field overlap, cropped and tiled reads, and repeated execution. Statistical appearance evidence must supplement exact discrete checks and bounded sample comparisons.
5. Prove full-source and tiled execution equivalence. Exercise images larger than a single device binding, wide optical support beyond the historical FIR cap, non-square and portrait geometry, and bounded working sets. Retain seams, boundary, full-source metering, and global grain-coordinate evidence.
6. Exercise explicit CPU, explicit GPU, automatic hardware selection, absent hardware, software WGPU, missing capabilities, device/resource failure, cancellation, and stale GUI results. Verify that no requested effect changes and no failed operation replaces or publishes output.
7. Retain actual executors and transfers for the complete computation scope, including input numerics, preparation, simulation, saving conversion, and viewer work. A backend name, shader source, dispatch count, fused-path label, or truncated report must not alone establish full-GPU coverage. Follow the [telemetry contract](../telemetry/spec.md) and its historical path semantics.
8. Run hardware acceptance on the supported Metal, Vulkan, and Windows WGPU APIs. Identify each adapter, driver, executable hash, build precision, reference, invocation, and source data. A missing platform or hardware environment must remain unverified. Software Vulkan smoke is a separate portability result.
9. Measure end-to-end and compute-only time, cold preparation, repeated execution, memory and transfer behavior against matched CPU runs on declared image workloads. Hold controls, seeds, output, and reuse conditions constant. Report observations without promising an unmeasured speedup. A resource optimization must pass the same alignment checks before cutover.

Specs, compilation, software-adapter runs, and isolated shader tests must not be reported as implementation acceptance of this contract.
