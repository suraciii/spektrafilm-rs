//! Exact whole-frame setup without raster readback or host numerical work.
use super::{WgpuBackend, grain::{RasterStorage, raster::RasterCommands}};

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
        for _ in 0..64 {
            self.count.submit(backend)?;
            self.select.submit(backend)?;
        }
        self.finish.submit(backend)
    }
}

pub(super) fn build(
    backend: &WgpuBackend, source: &impl RasterStorage, uniform: &wgpu::Buffer,
    density: &wgpu::Buffer, exposure: &wgpu::Buffer, status: &wgpu::Buffer,
    state: &wgpu::Buffer, pixels: u32,
) -> Result<FrameSetupState, String> {
    use wgpu::BufferBindingType::{Uniform as U, Storage as S};
    let limit = super::bounded::storage_binding_limit(backend)?;
    if state.size() > limit || density.size() > limit || exposure.size() > limit {
        return Err(format!("standard setup coefficients/state exceed {limit} byte binding limit"));
    }
    // Bound serial shader loops independently of a driver's physical binding
    // limit, so cancellation is checked between short device submissions.
    let capacity = (super::bounded::chunk_elements(limit, 12)? as u32).min(4096);
    let window = backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("standard_setup_density_window"), size: u64::from(capacity.min(pixels))*12,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pipeline = backend.cached_pipeline(concat!(
        include_str!("../../../spektrafilm-shaders/wgsl/scan/density_lib.wgsl"),
        include_str!("../../../spektrafilm-shaders/wgsl/scan/density_setup.wgsl")),
        &[U,S{read_only:true},S{read_only:true},S{read_only:true},S{read_only:false},S{read_only:false}]);
    let params = |count: u32, operation: u32, first: u32| -> Result<wgpu::Buffer, String> {
        backend.check_cancelled()?;
        let result = backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("standard_setup_window_params"), size: uniform.size(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = backend.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(uniform,0,&result,0,uniform.size());
        backend.queue.submit(Some(encoder.finish()));
        backend.queue.write_buffer(&result,0,bytemuck::cast_slice(&[count,operation,first,pixels]));
        Ok(result)
    };
    let command = |commands: &mut RasterCommands, count, operation, first| -> Result<(), String> {
        let p = params(count,operation,first)?;
        // workgroup_size(1): exactly one invocation updates ordered state.
        commands.dispatch(&pipeline,&[&p,&window,density,exposure,state,status],1,true,backend);
        Ok(())
    };
    let mut measure = RasterCommands::default();
    let mut count = RasterCommands::default();
    let mut first = 0;
    while first < pixels {
        let n = capacity.min(pixels-first);
        measure.copy(source,u64::from(first)*12,&window,0,u64::from(n)*12);
        command(&mut measure,n,0,first)?;
        count.copy(source,u64::from(first)*12,&window,0,u64::from(n)*12);
        command(&mut count,n,1,first)?;
        first += n;
    }
    let mut initialize = RasterCommands::default();
    let mut select = RasterCommands::default();
    let mut finish = RasterCommands::default();
    command(&mut initialize,0,2,0)?;
    command(&mut select,0,3,0)?;
    command(&mut finish,0,4,0)?;
    Ok(FrameSetupState { measure, initialize, count, select, finish })
}
