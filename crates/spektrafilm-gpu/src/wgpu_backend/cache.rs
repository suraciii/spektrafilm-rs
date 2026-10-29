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
    entry_point: &'static str,
    pointwise_images: [Option<&'static str>; 16],
}

impl PipelineKey {
    fn new(
        shader_source: &'static str,
        binding_types: &[wgpu::BufferBindingType],
        entry_point: &'static str,
    ) -> Self {
        Self {
            shader_source: shader_source.as_ptr() as usize,
            binding_types: binding_types.iter().copied().map(Into::into).collect(),
            entry_point,
            pointwise_images: [None; 16],
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
        self.get_or_compile_at(device, shader_source, binding_types, "main")
    }

    /// Compile (or fetch) the pipeline of one entry point. A shader module may
    /// declare several entry points — the Fotufilm modules do — and each one
    /// is its own pipeline with its own binding layout, so the entry point
    /// names the cache entry alongside the source and the binding types.
    pub(super) fn get_or_compile_at(
        &self,
        device: &wgpu::Device,
        shader_source: &'static str,
        binding_types: &[wgpu::BufferBindingType],
        entry_point: &'static str,
    ) -> (CachedPipelineRef, bool) {
        self.get_or_compile_key(
            device,
            PipelineKey::new(shader_source, binding_types, entry_point),
            binding_types,
            || Cow::Borrowed(shader_source),
        )
    }

    pub(super) fn get_or_compile_pointwise(
        &self,
        device: &wgpu::Device,
        original: &'static str,
        binding_types: &[wgpu::BufferBindingType],
        images: &[(usize, &'static str)],
        source: impl FnOnce() -> String,
    ) -> (CachedPipelineRef, bool) {
        let mut key = PipelineKey::new(original, binding_types, "main");
        for &(binding, name) in images {
            key.pointwise_images[binding] = Some(name);
        }
        self.get_or_compile_key(device, key, binding_types, || Cow::Owned(source()))
    }

    fn get_or_compile_key(
        &self,
        device: &wgpu::Device,
        key: PipelineKey,
        binding_types: &[wgpu::BufferBindingType],
        source: impl FnOnce() -> Cow<'static, str>,
    ) -> (CachedPipelineRef, bool) {
        let mut cache = self.pipelines.lock();
        let hit = cache.contains_key(&key);
        if !hit {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("compute_shader"),
                source: wgpu::ShaderSource::Wgsl(source()),
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
                entry_point: Some(key.entry_point),
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
mod logic_tests {
    use super::*;

    #[test]
    fn cache_key_includes_binding_layout() {
        let shader = "@compute @workgroup_size(1) fn main() {}";
        let uniform = [wgpu::BufferBindingType::Uniform];
        let readonly = [wgpu::BufferBindingType::Storage { read_only: true }];
        let readwrite = [wgpu::BufferBindingType::Storage { read_only: false }];

        assert_ne!(
            PipelineKey::new(shader, &uniform, "main"),
            PipelineKey::new(shader, &readonly, "main")
        );
        assert_ne!(
            PipelineKey::new(shader, &readonly, "main"),
            PipelineKey::new(shader, &readwrite, "main")
        );
        assert_ne!(
            PipelineKey::new(shader, &readonly, "main"),
            PipelineKey::new(shader, &readonly, "other")
        );
    }
}
