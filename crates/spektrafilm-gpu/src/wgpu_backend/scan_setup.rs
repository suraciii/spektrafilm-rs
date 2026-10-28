//! Exact whole-frame setup without raster readback or host numerical work.
use crate::wgpu_backend::{
    WgpuBackend, bounded,
    grain::{RasterStorage, raster::RasterCommands},
};

pub(super) struct FrameSetupState {
    measure: RasterCommands,
    initialize: RasterCommands,
    count: RasterCommands,
    select: RasterCommands,
    finish: RasterCommands,
}
impl FrameSetupState {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.measure.submit(backend)?;
        self.initialize.submit(backend)?;
        // Four 16-bit digits cover the sixty-four bits of the order key.
        for _ in 0..4 {
            self.count.submit(backend)?;
            self.select.submit(backend)?;
        }
        self.finish.submit(backend)
    }
}

// Borrow the existing device resources; grouping them would only hide bindings.
#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    backend: &WgpuBackend,
    source: &impl RasterStorage,
    uniform: &wgpu::Buffer,
    density: &wgpu::Buffer,
    exposure: &wgpu::Buffer,
    status: &wgpu::Buffer,
    state: &wgpu::Buffer,
    pixels: u32,
) -> Result<FrameSetupState, String> {
    use wgpu::BufferBindingType::{Storage as S, Uniform as U};
    let limit = bounded::storage_binding_limit(backend)?;
    if state.size() > limit || density.size() > limit || exposure.size() > limit {
        return Err(format!(
            "standard setup coefficients/state exceed {limit} byte binding limit"
        ));
    }
    // Bound serial loops independently of physical binding size so cancellation
    // is checked between short device submissions.
    let capacity = (bounded::chunk_elements(limit, 12)? as u32).min(4096);
    let window = backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("standard_setup_density_window"),
        size: u64::from(capacity.min(pixels)) * 12,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pipeline = backend.cached_pipeline(
        concat!(
            include_str!("../../../spektrafilm-shaders/wgsl/scan/density_lib.wgsl"),
            include_str!("../../../spektrafilm-shaders/wgsl/scan/density_setup.wgsl")
        ),
        &[
            U,
            S { read_only: true },
            S { read_only: true },
            S { read_only: true },
            S { read_only: false },
            S { read_only: false },
            S { read_only: false },
        ],
    );
    // 256 bins per rank; integers, so window order cannot change the result.
    let histogram = backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("standard_setup_digit_histogram"),
        size: 131072 * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    backend
        .queue
        .write_buffer(&histogram, 0, &[0u8; 524288]);
    let params = |count: u32, operation: u32, first: u32| -> Result<wgpu::Buffer, String> {
        backend.check_cancelled()?;
        let result = backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("standard_setup_window_params"),
            size: uniform.size(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = backend.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(uniform, 0, &result, 0, uniform.size());
        backend.submit_commands(Some(encoder.finish()))?;
        backend.queue.write_buffer(
            &result,
            0,
            bytemuck::cast_slice(&[count, operation, first, pixels]),
        );
        Ok(result)
    };
    let command = |commands: &mut RasterCommands, count, operation, first| -> Result<(), String> {
        let p = params(count, operation, first)?;
        // workgroup_size(1): exactly one invocation updates ordered state.
        commands.dispatch(
            &pipeline,
            &[&p, &window, density, exposure, state, status, &histogram],
            1,
            true,
            backend,
        );
        Ok(())
    };
    let mut measure = RasterCommands::default();
    let mut count = RasterCommands::default();
    let mut first = 0;
    while first < pixels {
        let n = capacity.min(pixels - first);
        measure.copy(source, u64::from(first) * 12, &window, 0, u64::from(n) * 12);
        command(&mut measure, n, 0, first)?;
        count.copy(source, u64::from(first) * 12, &window, 0, u64::from(n) * 12);
        command(&mut count, n, 1, first)?;
        first += n;
    }
    let mut initialize = RasterCommands::default();
    let mut select = RasterCommands::default();
    let mut finish = RasterCommands::default();
    command(&mut initialize, 0, 2, 0)?;
    command(&mut select, 0, 3, 0)?;
    command(&mut finish, 0, 4, 0)?;
    Ok(FrameSetupState {
        measure,
        initialize,
        count,
        select,
        finish,
    })
}
