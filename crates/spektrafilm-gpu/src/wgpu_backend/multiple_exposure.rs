//! Device-side multiple-exposure logE composition.
//!
//! The film boundary is `log10` exposure. A real multiple exposure adds light
//! before development, so the device accumulator stores linear exposure:
//! `S = sum_i 10^L_i`, then `log_e_finalize` converts back to `log10(S)`.
//! This is the same formula as the CPU accumulator; f32 device arithmetic is
//! the current device limitation.

use super::page_views::{Access, ViewPlan};
use super::paged::ImagePage;
use super::*;
use spektrafilm_backend::{DeviceImage, SampleDomain};

const INIT_SHADER: &str = r#"
struct Params {
    n_pixels: u32,
    shot_base: u32,
    acc_base: u32,
    _pad: u32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> shot: array<f32>;
@group(0) @binding(2) var<storage, read_write> acc: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;
    for (var c = 0u; c < 3u; c += 1u) {
        let i = base + c;
        acc[params.acc_base + i] = exp2(shot[params.shot_base + i] * 3.321928094887362);
    }
}
"#;

const ADD_SHADER: &str = r#"
struct Params {
    n_pixels: u32,
    shot_base: u32,
    acc_base: u32,
    _pad: u32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> shot: array<f32>;
@group(0) @binding(2) var<storage, read_write> acc: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;
    for (var c = 0u; c < 3u; c += 1u) {
        let i = base + c;
        acc[params.acc_base + i] = acc[params.acc_base + i]
            + exp2(shot[params.shot_base + i] * 3.321928094887362);
    }
}
"#;

const FINALIZE_SHADER: &str = r#"
struct Params {
    n_pixels: u32,
    _shot_base: u32,
    acc_base: u32,
    _pad: u32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> acc: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;
    for (var c = 0u; c < 3u; c += 1u) {
        let i = base + c;
        let v = max(acc[params.acc_base + i], 1e-30);
        acc[params.acc_base + i] = log2(v) * 0.3010299956639812;
    }
}
"#;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ComposeParams {
    n_pixels: u32,
    shot_base: u32,
    acc_base: u32,
    _pad: u32,
}

impl WgpuBackend {
    fn compose_pass(
        &self,
        shader: &'static str,
        bindings: &[wgpu::BufferBindingType],
        shot_pages: Option<&Vec<ImagePage>>,
        acc_pages: &Vec<ImagePage>,
        acc_access: Access,
        label: &str,
    ) -> Result<(), String> {
        let mut buffers: Vec<(&dyn super::grain::raster::RasterStorage, Access)> = Vec::new();
        if let Some(pages) = shot_pages {
            buffers.push((pages, Access::Read));
        }
        buffers.push((acc_pages, acc_access));
        let plan = ViewPlan::new(
            self,
            u64::from(acc_pages.iter().map(|p| p.n_pixels).sum::<u32>()),
            12,
            &buffers,
            label,
        )?;
        let pipeline = self.cached_pipeline(shader, bindings);
        plan.for_each(self, |encoder, chunk| {
            let (shot_base, acc_base) = if shot_pages.is_some() {
                (chunk.views[0].base, chunk.views[1].base)
            } else {
                (0, chunk.views[0].base)
            };
            let uniform = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::bytes_of(&ComposeParams {
                        n_pixels: chunk.count,
                        shot_base,
                        acc_base,
                        _pad: 0,
                    }),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }];
            entries.extend(chunk.views.iter().enumerate().map(|(index, view)| {
                wgpu::BindGroupEntry {
                    binding: index as u32 + 1,
                    resource: view.binding(),
                }
            }));
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &pipeline.layout,
                entries: &entries,
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            super::dispatch_linear(&mut pass, chunk.count);
            Ok(())
        })
    }

    pub(in crate::wgpu_backend) fn accumulate_log_e(
        &self,
        accumulator: Option<&DeviceImage>,
        shot: &DeviceImage,
    ) -> Result<Option<DeviceImage>, String> {
        if shot.width() == 0 || shot.height() == 0 {
            return Err("multiple exposure shot is empty".into());
        }
        let shot_payload = self.buffer_payload(shot)?;
        if let Some(acc) = accumulator {
            if acc.width() != shot.width() || acc.height() != shot.height() {
                return Err("multiple exposure accumulator dimensions do not match shot".into());
            }
            if acc.domain() != SampleDomain::LogExposure {
                return Err("multiple exposure accumulator has the wrong sample domain".into());
            }
            let acc_payload = self.buffer_payload(acc)?;
            self.compose_pass(
                ADD_SHADER,
                &[
                    wgpu::BufferBindingType::Uniform,
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: false },
                ],
                Some(&shot_payload.pages),
                &acc_payload.pages,
                Access::ReadWrite,
                "multiple_exposure_add",
            )?;
            return Ok(Some(acc.clone()));
        }

        let pages =
            self.allocate_image_pages(shot.width(), shot.height(), "multiple_exposure_acc")?;
        self.compose_pass(
            INIT_SHADER,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            Some(&shot_payload.pages),
            &pages,
            Access::Write,
            "multiple_exposure_init",
        )?;
        Ok(Some(self.retain_image_pages(
            pages,
            shot.width(),
            shot.height(),
            SampleDomain::LogExposure,
            shot.space(),
            shot.pixel_pitch_um(),
        )))
    }

    /// Finalize in place. The returned carrier aliases the input; callers must
    /// not accumulate into it after this call.
    pub(in crate::wgpu_backend) fn finalize_log_e(
        &self,
        accumulator: &DeviceImage,
    ) -> Result<Option<DeviceImage>, String> {
        if accumulator.domain() != SampleDomain::LogExposure {
            return Err("multiple exposure accumulator has the wrong sample domain".into());
        }
        let payload = self.buffer_payload(accumulator)?;
        self.compose_pass(
            FINALIZE_SHADER,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            None,
            &payload.pages,
            Access::ReadWrite,
            "multiple_exposure_finalize",
        )?;
        Ok(Some(accumulator.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::adapter;
    use spektrafilm_backend::{ComputeBackend, Precision, RetainedImageMeta};
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::from_f64;

    fn log_image(width: u32, height: u32, phase: f32) -> ImageBuf {
        let mut image = ImageBuf::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let u = x as f32 / width.max(1) as f32;
                let v = y as f32 / height.max(1) as f32;
                image.set(
                    x,
                    y,
                    [
                        from_f64((-2.0 + 2.5 * u + phase) as f64),
                        from_f64((-1.8 + 2.0 * v + 0.3 * phase) as f64),
                        from_f64((-2.2 + 1.7 * (u + v) * 0.5 + phase) as f64),
                    ],
                );
            }
        }
        image
    }

    fn upload(gpu: &WgpuBackend, image: &ImageBuf) -> DeviceImage {
        gpu.retain_image(
            image,
            &RetainedImageMeta {
                width: image.width,
                height: image.height,
                domain: SampleDomain::LogExposure,
                precision: Precision::F32,
                space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
                pixel_pitch_um: Some(3.8),
            },
        )
        .unwrap()
    }

    #[test]
    fn device_log_e_composition_matches_f64_reference() {
        let gpu = adapter();
        let width = 64u32;
        let height = 32u32;
        let images: Vec<ImageBuf> = (0..4)
            .map(|i| log_image(width, height, i as f32 * 0.15))
            .collect();
        let mut reference = vec![0.0f64; (width * height * 3) as usize];
        for image in &images {
            for (slot, &value) in reference.iter_mut().zip(&image.data) {
                *slot += 10.0_f64.powf(value as f64);
            }
        }
        let reference: Vec<f32> = reference.iter().map(|value| value.log10() as f32).collect();

        let mut accumulator = None;
        for image in &images {
            let retained = upload(&gpu, image);
            accumulator = Some(
                gpu.log_e_accumulate(accumulator.as_ref(), &retained)
                    .unwrap()
                    .unwrap(),
            );
        }
        let composed = gpu
            .log_e_finalize(accumulator.as_ref().unwrap())
            .unwrap()
            .unwrap()
            .to_host(gpu.backend())
            .unwrap();
        let mut worst = 0.0f32;
        for (&actual, &expected) in composed.data.iter().zip(&reference) {
            worst = worst.max((actual - expected).abs());
        }
        assert!(
            worst < 5e-6,
            "device logE composition differs from f64 by {worst:.3e}"
        );
    }
}
