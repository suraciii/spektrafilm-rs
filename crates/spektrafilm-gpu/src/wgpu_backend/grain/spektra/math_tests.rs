#[test]
fn explicit_binary64_math_preserves_transcendental_values() {
    let gpu = crate::test_support::adapter();
    assert!(gpu.device.features().contains(wgpu::Features::SHADER_F64));
    const SHADER: &str = concat!(
        include_str!("../../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
        r#"
@group(0) @binding(0) var<storage, read> inputs: array<f64>;
@group(0) @binding(1) var<storage, read_write> outputs: array<f64>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if gid.x >= arrayLength(&inputs) { return; }
  let x = inputs[gid.x];
  outputs[gid.x * 4u] = sf64_log(abs(x));
  outputs[gid.x * 4u + 1u] = sf64_exp(x);
  outputs[gid.x * 4u + 2u] = sf64_powi(x, 5u);
  outputs[gid.x * 4u + 3u] = sf64_round(x);
}
"#
    );
    let input = [
        -700.0f64,
        -30.0,
        -2.0,
        -0.5,
        -0.49999999999999994,
        -1e-8,
        1e-8,
        0.1,
        0.49999999999999994,
        0.5,
        1.0,
        2.0,
        30.0,
        700.0,
    ];
    let bytes = (input.len() * 4 * 8) as u64;
    let source = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("binary64_math_input"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let destination = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("binary64_math_output"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("binary64_math_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pipe = gpu.cached_pipeline(
        SHADER,
        &[
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("binary64_math_bindings"),
        layout: &pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: source.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: destination.as_entire_binding(),
            },
        ],
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("binary64_math"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipe.pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&destination, 0, &readback, 0, bytes);
    gpu.queue.submit(Some(encoder.finish()));
    let actual: Vec<f64> = gpu.pipe.read_mapped(&readback, bytes).unwrap();
    for (i, x) in input.into_iter().enumerate() {
        let reference = [x.abs().ln(), x.exp(), x.powi(5), x.round()];
        for (operation, expected) in reference.into_iter().enumerate() {
            let actual = actual[i * 4 + operation];
            // The reduced Taylor/atanh remainder is below binary64 epsilon;
            // 32 eps covers operation rounding and the range-reduction sum.
            let bound = expected.abs().max(f64::MIN_POSITIVE) * 32.0 * f64::EPSILON;
            assert!(
                actual.is_finite() && (actual - expected).abs() <= bound,
                "operation {operation} at {x}: device {actual:e}, reference {expected:e}, bound {bound:e}"
            );
        }
    }
}
