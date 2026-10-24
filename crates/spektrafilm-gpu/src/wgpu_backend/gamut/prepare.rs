use super::super::*;
use spektrafilm_backend::{DeviceImagePayload, GamutTableParams, PreparedTable};
use std::{any::Any, sync::Arc};

/// The gamut consumer needs only the narrowed boundary, unlike spectral
/// preparation whose calibration consumer also uses binary64 table samples.
#[derive(Debug)]
pub(super) struct GamutPreparationTable {
    pub backend_id: u64,
    pub buffer: wgpu::Buffer,
}

impl DeviceImagePayload for GamutPreparationTable {
    fn backend_id(&self) -> u64 {
        self.backend_id
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    mode: u32,
    n_l: u32,
    n_h: u32,
    n_bisect: u32,
    first: u32,
    count: u32,
    pad: [u32; 2],
    geometry: [f64; 4],
    xyz_to_rgb: [[f64; 4]; 3],
    cam16: [f64; 4],
}

const PREPARE_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("jzazbz.wgsl"),
    include_str!("prepare.wgsl"),
);

impl WgpuBackend {
    pub(in crate::wgpu_backend) fn prepare_gamut_table_device(
        &self,
        params: &GamutTableParams,
    ) -> Result<PreparedTable, String> {
        if !self.device.features().contains(wgpu::Features::SHADER_F64) {
            return Err(
                "output gamut boundary preparation requires device binary64 shader arithmetic"
                    .into(),
            );
        }
        if !(1..=4).contains(&params.mode) || params.n_l < 2 || params.n_h == 0 {
            return Err(
                "output gamut boundary has invalid perceptual mode or grid dimensions".into(),
            );
        }
        let count = params
            .n_l
            .checked_mul(params.n_h)
            .ok_or_else(|| "output gamut boundary exceeds device indexing".to_owned())?;
        let bytes = u64::from(count) * 4;
        // The compression pass requires random access to all four bilinear
        // corners. Its table is one immutable binding, independent of image
        // dimensions, so this is the actual minimum resident working set.
        let limit = bounded::storage_binding_limit(self)?;
        if bytes > limit {
            return Err(format!(
                "output gamut boundary minimum coefficient binding needs {bytes} bytes; device binding limit is {limit}"
            ));
        }
        let pipeline = self.cached_pipeline_at(
            PREPARE_SHADER,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            "main",
        );
        let xyz_to_rgb = params.xyz_to_rgb.map(|row| [row[0], row[1], row[2], 0.0]);
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gamut_prepare_params"),
                contents: bytemuck::bytes_of(&Params {
                    mode: params.mode,
                    n_l: params.n_l,
                    n_h: params.n_h,
                    n_bisect: params.n_bisect,
                    first: 0,
                    count,
                    pad: [0; 2],
                    geometry: [params.l_min, params.l_max, params.chroma_upper, 0.0],
                    xyz_to_rgb,
                    cam16: params.cam16,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gamut_prepared_cmax"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gamut_prepare"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gamut_prepare"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            dispatch_linear(&mut pass, count);
        }
        self.queue.submit(Some(encoder.finish()));
        self.device.context.record_executor(Executor::Gpu, None);
        Ok(PreparedTable {
            values: Arc::new(GamutPreparationTable {
                backend_id: self.backend_id,
                buffer,
            }),
            neutral: [0.0; 3],
        })
    }

    /// Explicit diagnostic export for cold-table parity probes. Production
    /// compression binds this resource directly and never calls this method.
    pub fn diagnostic_gamut_table(&self, table: &PreparedTable) -> Result<Vec<f64>, String> {
        let payload = table
            .values
            .as_any()
            .downcast_ref::<GamutPreparationTable>()
            .ok_or_else(|| "diagnostic gamut table payload is not a GPU gamut table".to_owned())?;
        if payload.backend_id != self.backend_id {
            return Err("diagnostic gamut table belongs to a different GPU backend".into());
        }
        Ok(self
            .pipe
            .readback_buffer(&payload.buffer, payload.buffer.size())
            .into_iter()
            .map(f64::from)
            .collect())
    }
}
