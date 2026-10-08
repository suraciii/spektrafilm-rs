#![cfg(feature = "wgpu-backend")]

/// Validate the standalone shader with a WGPU device when one is available.
#[test]
fn grain_v2_shader_compiles() {
    let source = include_str!("../src/wgpu_backend/grain/grain_v2.wgsl");
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()));
    let Some(adapter) = adapter else {
        eprintln!(
            "Grain V2 GPU smoke skipped: no WGPU adapter; CPU behavior tests remain available"
        );
        return;
    };
    let (device, _) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .expect("open Grain V2 smoke device");
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("grain_v2_smoke"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let _pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("grain_v2_smoke"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let error = pollster::block_on(device.pop_error_scope());
    assert!(
        error.is_none(),
        "Grain V2 WGSL validation failed: {error:?}"
    );
}
