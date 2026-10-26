use super::grain::raster::RasterStorage;
#[cfg(test)]
use super::paged::ImagePage;
use super::*;

pub(super) use super::page_views::Access as PointwiseAccess;
use super::page_views::ViewPlan;

/// Retain only the image plan and immutable kernel parameters, never a
/// full-image list of per-chunk uniforms and bind groups.
pub(super) struct PointwiseJob {
    plan: ViewPlan,
    pipeline: CachedPipelineRef,
    params: Vec<u8>,
    global_offset_word: Option<usize>,
    label: String,
}

pub(super) fn image_transport_bytes(
    _backend: &WgpuBackend,
    width: u32,
    height: u32,
    label: &str,
) -> Result<u64, String> {
    let pixels = u64::from(width) * u64::from(height);
    let bytes = pixels
        .checked_mul(12)
        .ok_or_else(|| format!("{label}: image byte count overflow"))?;
    if pixels == 0 || pixels > u64::from(u32::MAX) {
        return Err(format!(
            "{label}: image needs {pixels} pixels, exceeding logical u32 pixel indexing"
        ));
    }
    Ok(bytes)
}

/// `params` starts with the local item count. Address residuals are scalar,
/// independent for every binding, including scalar (4-byte) kernels.
#[allow(clippy::too_many_arguments)]
pub(super) fn pointwise_job<P: bytemuck::Pod>(
    backend: &WgpuBackend,
    n: u64,
    bytes_per_item: u64,
    shader: &'static str,
    params: &P,
    buffers: &[(&dyn RasterStorage, PointwiseAccess)],
    global_offset_word: Option<usize>,
    label: &str,
) -> Result<PointwiseJob, String> {
    let params = bytemuck::bytes_of(params);
    if params.len() < 4
        || buffers.len() >= 16
        || global_offset_word.is_some_and(|word| word >= params.len() / 4)
    {
        return Err(format!("{label}: invalid pointwise parameter layout"));
    }
    let plan = ViewPlan::new(backend, n, bytes_per_item, buffers, label)?;
    let mut layout = vec![wgpu::BufferBindingType::Uniform];
    layout.extend(
        buffers
            .iter()
            .map(|(_, access)| wgpu::BufferBindingType::Storage {
                read_only: matches!(access, PointwiseAccess::Read | PointwiseAccess::Fixed),
            }),
    );
    let mut names = Vec::new();
    for (index, (_, access)) in buffers.iter().enumerate() {
        if *access == PointwiseAccess::Fixed {
            continue;
        }
        let binding = index + 1;
        let marker = format!("@binding({binding})");
        let name = shader
            .lines()
            .find(|line| line.contains(&marker))
            .and_then(|line| line.split_once("var<"))
            .and_then(|(_, declaration)| declaration.split_once('>'))
            .and_then(|(_, declaration)| declaration.split_once(':'))
            .map(|(name, _)| name.trim())
            .ok_or_else(|| format!("{label}: missing storage declaration for binding {binding}"))?;
        names.push((binding, name));
    }
    let pipeline = backend.cached_pointwise_pipeline(shader, &layout, &names);
    Ok(PointwiseJob {
        plan,
        pipeline,
        params: params.to_vec(),
        global_offset_word,
        label: label.into(),
    })
}

impl PointwiseJob {
    pub(super) fn execute(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.plan.for_each(backend, |encoder, chunk| {
            let mut params = self.params.clone();
            params[..4].copy_from_slice(&chunk.count.to_ne_bytes());
            if let Some(word) = self.global_offset_word {
                params[word * 4..word * 4 + 4].copy_from_slice(&(chunk.first as u32).to_ne_bytes());
            }
            let params = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&self.label),
                    contents: &params,
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let mut bases = [0u32; 16];
            for (index, view) in chunk.views.iter().enumerate() {
                bases[index + 1] = view.base;
            }
            let bases = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("pointwise_bases"),
                    contents: bytemuck::cast_slice(&bases),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }];
            entries.extend(chunk.views.iter().enumerate().map(|(index, view)| {
                wgpu::BindGroupEntry {
                    binding: index as u32 + 1,
                    resource: view.binding(),
                }
            }));
            entries.push(wgpu::BindGroupEntry {
                binding: chunk.views.len() as u32 + 1,
                resource: bases.as_entire_binding(),
            });
            let group = backend
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&self.label),
                    layout: &self.pipeline.layout,
                    entries: &entries,
                });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(&self.label),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            dispatch_linear(&mut pass, chunk.count);
            Ok(())
        })
    }
}

pub(super) struct HighlightBoostState {
    reduction: ViewPlan,
    buffers: Vec<wgpu::Buffer>,
    maximum: wgpu::Buffer,
    reduce_pipe: CachedPipelineRef,
    merge_pipe: CachedPipelineRef,
    boost: PointwiseJob,
}

pub(super) fn build_highlight_boost_state(
    device: &ObservedDevice,
    hp: &spektrafilm_backend::HighlightBoostGpuParams,
    n_pixels: u32,
    img: &(impl RasterStorage + ?Sized),
    backend: &WgpuBackend,
) -> Result<HighlightBoostState, String> {
    let n_values = u64::from(n_pixels) * 3;
    let limit = super::bounded::storage_binding_limit(backend)?;
    let capacity = (limit / 4).min(u64::from(u32::MAX)) as u32;
    if capacity == 0 || n_values == 0 {
        return Err(format!(
            "highlight: minimum maximum-reduction binding needs 4 bytes; limit is {limit}"
        ));
    }
    let image = img.pages();
    let reduction = ViewPlan::new(
        backend,
        n_values,
        4,
        &[(&image, PointwiseAccess::Read)],
        "highlight_reduce",
    )?;
    let mk_buffer = |size, label| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    let maximum = mk_buffer(4, "highlight_global_maximum");
    let mut buffers = Vec::new();
    let mut values = u64::from(capacity).min(n_values) as u32;
    loop {
        values = values.div_ceil(2048).max(1);
        buffers.push(mk_buffer(u64::from(values) * 4, "highlight_reduce"));
        if values == 1 {
            break;
        }
    }
    let reduce_pipe = backend.cached_pointwise_pipeline(
        include_str!("max_reduce.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
        &[(1, "input")],
    );
    let merge_pipe = backend.cached_pipeline(
        include_str!("max_merge.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct BoostParams {
        n_values: u32,
        boost_ev: f32,
        boost_range: f32,
        protect_ev: f32,
    }
    let boost = pointwise_job(
        backend,
        n_values,
        4,
        include_str!("highlight_boost.wgsl"),
        &BoostParams {
            n_values: 0,
            boost_ev: hp.boost_ev,
            boost_range: hp.boost_range,
            protect_ev: hp.protect_ev,
        },
        &[
            (&maximum, PointwiseAccess::Fixed),
            (&img.pages(), PointwiseAccess::ReadWrite),
        ],
        None,
        "highlight_boost",
    )?;
    Ok(HighlightBoostState {
        reduction,
        buffers,
        maximum,
        reduce_pipe,
        merge_pipe,
        boost,
    })
}

impl HighlightBoostState {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.reduction.for_each(backend, |encoder, chunk| {
            let mut values = chunk.count;
            for (index, output) in self.buffers.iter().enumerate() {
                let blocks = values.div_ceil(2048).max(1);
                let params = backend
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("highlight_reduce_params"),
                        contents: bytemuck::cast_slice(&[values, 0, 0, 0, 0, 0, 0, 0]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let input = if index == 0 {
                    chunk.views[0].clone()
                } else {
                    super::page_views::StorageView::entire(&self.buffers[index - 1])
                };
                let mut bases = [0u32; 16];
                bases[1] = input.base;
                let bases = backend
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("highlight_reduce_bases"),
                        contents: bytemuck::cast_slice(&bases),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let group = backend
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("highlight_reduce"),
                        layout: &self.reduce_pipe.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: params.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: input.binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: output.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: bases.as_entire_binding(),
                            },
                        ],
                    });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("highlight_reduce"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.reduce_pipe.pipeline);
                pass.set_bind_group(0, &group, &[]);
                let (x, y) = dispatch_grid(blocks);
                pass.dispatch_workgroups(x, y, 1);
                values = blocks;
            }
            let params = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("highlight_merge_params"),
                    contents: bytemuck::cast_slice(&[u32::from(chunk.first == 0), 0, 0, 0]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let group = backend
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("highlight_merge"),
                    layout: &self.merge_pipe.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.buffers.last().unwrap().as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.maximum.as_entire_binding(),
                        },
                    ],
                });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("highlight_merge"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.merge_pipe.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            Ok(())
        })?;
        self.boost.execute(backend)
    }
}

#[cfg(test)]
mod tests;
