#![cfg(feature = "wgpu-backend")]

/// Validate the assembled gamut shader with a WGPU device when one is
/// available.
///
/// The device lock is held for the whole test: the software Vulkan driver
/// (llvmpipe) is not safe under concurrent device use in one process, so a
/// raw `wgpu::Instance` here would race every other device test in this
/// binary and any sibling test binary the runner starts in parallel.
///
/// The shader is the same concatenation the pass builds
/// (`GAMUT_SHADER` in `wgpu_backend/gamut/mod.rs`): the shared binary64
/// helpers, the JzAzBz f64 transforms, then the f32 kernel. Compiling it
/// catches a WGSL error in the JzAzBz series or in the f64 literals that
/// only the device compiler reports.
/// The gamut pass carries the JzAzBz perceptual transform in binary64, so
/// the device must be opened with `SHADER_F64` — the same feature the
/// production backend requests (see `WgpuBackend::new`). Opening a default
/// device here would report a validation failure for a shader that is
/// correct on any device the production path accepts.
#[test]
fn gamut_shader_compiles() {
    let _guard = spektrafilm_gpu::test_support::device_lock();
    let source = concat!(
        include_str!("../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
        include_str!("../src/wgpu_backend/gamut/jzazbz.wgsl"),
        include_str!("../src/wgpu_backend/gamut/gamut_compress.wgsl"),
    );
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()));
    let adapter = adapter.expect("WGPU adapter required for GPU tests");
    let (device, _) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("gamut_smoke"),
            required_features: wgpu::Features::SHADER_F64,
            ..Default::default()
        },
        None,
    ))
    .expect("open gamut smoke device with binary64 shader support");
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gamut_smoke"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let _pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("gamut_smoke"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let error = pollster::block_on(device.pop_error_scope());
    assert!(
        error.is_none(),
        "gamut compression WGSL validation failed: {error:?}"
    );
}
