use std::{borrow::Cow, collections::HashMap};

use parking_lot::Mutex;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum BufferBindingKey {
    Uniform,
    StorageReadOnly,
    StorageReadWrite,
}

impl From<wgpu::BufferBindingType> for BufferBindingKey {
    fn from(value: wgpu::BufferBindingType) -> Self {
        match value {
            wgpu::BufferBindingType::Uniform => Self::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true } => Self::StorageReadOnly,
            wgpu::BufferBindingType::Storage { read_only: false } => Self::StorageReadWrite,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PipelineKey {
    shader_source: usize,
    binding_types: Vec<BufferBindingKey>,
}

impl PipelineKey {
    fn new(shader_source: &'static str, binding_types: &[wgpu::BufferBindingType]) -> Self {
        Self {
            shader_source: shader_source.as_ptr() as usize,
            binding_types: binding_types.iter().copied().map(Into::into).collect(),
        }
    }
}

struct CachedPipeline {
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

#[derive(Clone)]
pub(super) struct CachedPipelineRef {
    pub(super) pipeline: wgpu::ComputePipeline,
    pub(super) layout: wgpu::BindGroupLayout,
}

#[derive(Default)]
pub(super) struct PipelineCache {
    pipelines: Mutex<HashMap<PipelineKey, CachedPipeline>>,
}

impl PipelineCache {
    pub(super) fn get_or_compile(
        &self,
        device: &wgpu::Device,
        shader_source: &'static str,
        binding_types: &[wgpu::BufferBindingType],
    ) -> (CachedPipelineRef, bool) {
        let key = PipelineKey::new(shader_source, binding_types);
        let mut cache = self.pipelines.lock();
        let hit = cache.contains_key(&key);
        if !hit {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("compute_shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(shader_source)),
            });
            let entries: Vec<wgpu::BindGroupLayoutEntry> = binding_types
                .iter()
                .enumerate()
                .map(|(i, &ty)| wgpu::BindGroupLayoutEntry {
                    binding: i as u32,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect();
            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("compute_layout"),
                    entries: &entries,
                });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("compute_pipeline_layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("compute_pipeline"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            cache.insert(
                key.clone(),
                CachedPipeline {
                    bind_group_layout,
                    pipeline,
                },
            );
        }
        let cached = cache.get(&key).expect("pipeline was inserted or cached");
        (
            CachedPipelineRef {
                pipeline: cached.pipeline.clone(),
                layout: cached.bind_group_layout.clone(),
            },
            hit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_includes_binding_layout() {
        let shader = "@compute @workgroup_size(1) fn main() {}";
        let uniform = [wgpu::BufferBindingType::Uniform];
        let readonly = [wgpu::BufferBindingType::Storage { read_only: true }];
        let readwrite = [wgpu::BufferBindingType::Storage { read_only: false }];

        assert_ne!(
            PipelineKey::new(shader, &uniform),
            PipelineKey::new(shader, &readonly)
        );
        assert_ne!(
            PipelineKey::new(shader, &readonly),
            PipelineKey::new(shader, &readwrite)
        );
    }
}
