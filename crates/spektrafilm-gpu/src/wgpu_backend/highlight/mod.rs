use super::*;
use super::blur::DispatchJob;

#[cfg(feature = "wgpu-backend")]
pub(super) struct HighlightBoostState {
    _buffers: Vec<wgpu::Buffer>,
    reductions: Vec<(DispatchJob, u32)>,
    boost: DispatchJob,
    n_values: u32,
}

#[cfg(feature = "wgpu-backend")]
pub(super) fn build_highlight_boost_state(
    device: &wgpu::Device,
    hp: &crate::HighlightBoostGpuParams,
    n_pixels: u32,
    img: &wgpu::Buffer,
    backend: &WgpuBackend,
) -> HighlightBoostState {
    use wgpu::util::DeviceExt;
    let n_values = n_pixels * 3;
    let mut counts = Vec::new();
    let mut values = n_values;
    loop {
        let blocks = values.div_ceil(2048).max(1);
        counts.push(blocks);
        if blocks == 1 {
            break;
        }
        values = blocks;
    }

    let reduce_pipe = backend.cached_pipeline(
        include_str!("max_reduce.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let boost_pipe = backend.cached_pipeline(
        include_str!("highlight_boost.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );

    let buffers: Vec<wgpu::Buffer> = counts
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("highlight_reduce_{i}")),
                size: (c as u64) * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        })
        .collect();

    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct ReduceParams {
        n_values: u32,
        _pad: [u32; 7],
    }

    let mut reductions = Vec::with_capacity(counts.len());
    for (i, &blocks) in counts.iter().enumerate() {
        let pass_values = if i == 0 { n_values } else { counts[i - 1] };
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("highlight_reduce_params_{i}")),
            contents: bytemuck::bytes_of(&ReduceParams {
                n_values: pass_values,
                _pad: [0; 7],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let input = if i == 0 { img } else { &buffers[i - 1] };
        let output = &buffers[i];
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("highlight_reduce_bg_{i}")),
            layout: &reduce_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        reductions.push((
            DispatchJob {
                _params_buf: params_buf,
                pipeline: reduce_pipe.clone(),
                bg,
            },
            blocks,
        ));
    }

    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct BoostParams {
        n_values: u32,
        boost_ev: f32,
        boost_range: f32,
        protect_ev: f32,
    }
    let boost_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("highlight_boost_params"),
        contents: bytemuck::bytes_of(&BoostParams {
            n_values,
            boost_ev: hp.boost_ev,
            boost_range: hp.boost_range,
            protect_ev: hp.protect_ev,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let final_max = buffers.last().expect("highlight reduction has output");
    let boost_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("highlight_boost_bg"),
        layout: &boost_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: boost_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: final_max.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: img.as_entire_binding(),
            },
        ],
    });

    HighlightBoostState {
        _buffers: buffers,
        reductions,
        boost: DispatchJob {
            _params_buf: boost_params,
            pipeline: boost_pipe,
            bg: boost_bg,
        },
        n_values,
    }
}

#[cfg(feature = "wgpu-backend")]
impl HighlightBoostState {
    pub(super) fn encode_passes(&self, encoder: &mut wgpu::CommandEncoder) {
        for (job, blocks) in &self.reductions {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("highlight_reduce"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&job.pipeline.pipeline);
            pass.set_bind_group(0, &job.bg, &[]);
            let (x, y) = dispatch_grid(*blocks);
            pass.dispatch_workgroups(x, y, 1);
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("highlight_boost"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.boost.pipeline.pipeline);
        pass.set_bind_group(0, &self.boost.bg, &[]);
        dispatch_linear(&mut pass, self.n_values);
    }
}


