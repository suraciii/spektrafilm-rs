//! Bounded RAW output. Device-produced gather indices address retained pages;
//! the host reads only two-word footprint bounds, never sensor/image samples.
use super::super::paged::ImagePage;
use super::paged::{RawPage, RawPlane};
use super::*;

const RO: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: true };
const RW: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: false };
const U: wgpu::BufferBindingType = wgpu::BufferBindingType::Uniform;
const GEOMETRY: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_geometry_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_output_geometry.wgsl")
);
const AXES: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_geometry_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_output_axes.wgsl")
);
const LENS: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lens_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lens_mapping.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_output_lens.wgsl")
);
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Geometry {
    dimensions: [u32; 4],
    tile: [u32; 4],
    controls: [u32; 4],
    step: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Axis {
    range: [u32; 4],
    step: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Adaptation {
    matrix: [[f32; 4]; 3],
    count: u32,
    active: u32,
    tint: f32,
    pad: u32,
}

struct Scratch {
    indices: RawBuffer,
    values: RawBuffer,
    output: RawBuffer,
    coords: RawBuffer,
    bounds: RawBuffer,
    reset: RawBuffer,
}
impl Scratch {
    fn new(gpu: &WgpuBackend, count: u32, requests: u32) -> Self {
        Self {
            indices: buffer(
                gpu,
                "raw_output_indices",
                u64::from(count) * u64::from(requests) * 4,
            ),
            values: buffer(
                gpu,
                "raw_output_values",
                u64::from(count) * u64::from(requests) * 16,
            ),
            output: buffer(gpu, "raw_output_tile", u64::from(count) * 16),
            coords: buffer(gpu, "raw_output_coordinates", u64::from(count) * 48),
            bounds: buffer(gpu, "raw_output_bounds", 8),
            reset: RawBuffer::upload(
                &gpu.device,
                "raw_output_bounds_reset",
                bytemuck::cast_slice(&[u32::MAX, 0u32]),
                wgpu::BufferUsages::COPY_SRC,
            ),
        }
    }
    fn reset(&self, encoder: &mut ObservedEncoder) {
        encoder.copy_buffer_to_buffer(&self.reset.buffer, 0, &self.bounds.buffer, 0, 8);
    }
}
fn buffer(gpu: &WgpuBackend, label: &str, size: u64) -> RawBuffer {
    RawBuffer {
        buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size.max(16),
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        }),
    }
}
fn uniform<T: bytemuck::Pod>(gpu: &WgpuBackend, value: &T) -> RawBuffer {
    RawBuffer::upload(
        &gpu.device,
        "raw_output_params",
        bytemuck::bytes_of(value),
        wgpu::BufferUsages::UNIFORM,
    )
}
fn scatter(
    encoder: &mut ObservedEncoder,
    source: &wgpu::Buffer,
    first: u32,
    count: u32,
    pages: &[ImagePage],
) {
    for page in pages {
        let start = first.max(page.first_pixel);
        let end = (first + count).min(page.first_pixel + page.n_pixels);
        if start < end {
            encoder.copy_buffer_to_buffer(
                source,
                u64::from(start - first) * 12,
                &page.buffer,
                u64::from(start - page.first_pixel) * 12,
                u64::from(end - start) * 12,
            );
        }
    }
}
fn scatter_raw(
    encoder: &mut ObservedEncoder,
    source: &wgpu::Buffer,
    first: u32,
    count: u32,
    pages: &[RawPage],
) {
    for page in pages {
        let start = first.max(page.first_pixel);
        let end = (first + count).min(page.first_pixel + page.n_pixels);
        if start < end {
            encoder.copy_buffer_to_buffer(
                source,
                u64::from(start - first) * 16,
                &page.buffer,
                u64::from(start - page.first_pixel) * 16,
                u64::from(end - start) * 16,
            );
        }
    }
}

fn output_capacity(
    limit: u64,
    lens: bool,
    fuji: bool,
    rotated: (u32, u32),
    output: (u32, u32),
) -> Result<u32, String> {
    let bytes = if lens {
        192
    } else if fuji {
        64
    } else {
        48
    };
    Ok(
        (super::super::bounded::chunk_elements(limit, bytes)? as u32)
            .min(65536)
            .min((output.0 * output.1).max(rotated.0 * rotated.1)),
    )
}

impl WgpuBackend {
    /// Gather a device-selected footprint. Readback is fixed-size control data,
    /// not a correction raster; arbitrarily disconnected TCA footprints work.
    fn raw_output_gather<'a>(
        &self,
        scratch: &Scratch,
        count: u32,
        channels: u32,
        pages: impl Iterator<Item = (u32, u32, &'a wgpu::Buffer)>,
    ) -> Result<(), String> {
        self.raw_gather_requests_gpu(
            &scratch.indices,
            &scratch.bounds,
            &scratch.values,
            count,
            channels,
            pages,
        )
    }
    pub(super) fn raw_gather_requests_gpu<'a>(
        &self,
        indices: &RawBuffer,
        bounds: &RawBuffer,
        values: &RawBuffer,
        count: u32,
        channels: u32,
        pages: impl Iterator<Item = (u32, u32, &'a wgpu::Buffer)>,
    ) -> Result<(), String> {
        let extent = self
            .pipe
            .readback_buffer_cancellable::<u32>(&bounds.buffer, 8)?;
        if extent.len() != 2 {
            return Err("RAW output footprint metadata is unavailable".into());
        }
        if extent[0] == u32::MAX {
            return Ok(());
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for (first, n, source) in pages {
            self.check_cancelled()?;
            if first > extent[1] || first + n <= extent[0] {
                continue;
            }
            let p = uniform(self, &[first, n, channels, count]);
            let source = RawBuffer {
                buffer: source.clone(),
            };
            self.raw_dispatch(
                include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_output_gather.wgsl"),
                &[&p, indices, &source, values],
                &[U, RO, RO, RW],
                count,
                "raw_output_gather",
            )
            .encode(&mut encoder, "raw_output_gather");
        }
        self.raw_submit(encoder)?;
        Ok(())
    }
    fn raw_output_geometry_dispatch(
        &self,
        p: &Geometry,
        scratch: &Scratch,
        encoder: &mut ObservedEncoder,
    ) {
        let u = uniform(self, p);
        self.raw_dispatch(
            GEOMETRY,
            &[
                &u,
                &scratch.indices,
                &scratch.values,
                &scratch.output,
                &scratch.bounds,
            ],
            &[U, RW, RO, RW, RW],
            p.tile[1],
            "raw_tiled_geometry",
        )
        .encode(encoder, "raw_tiled_geometry");
    }
    fn raw_output_axes(
        &self,
        count: u32,
        split: u32,
        step: [u32; 4],
        initial: [u32; 2],
        mode: u32,
    ) -> Result<RawPlane, String> {
        let plane = self.raw_allocate_plane(count, 1, "raw_output_axes")?;
        let state = RawBuffer::upload(
            &self.device,
            "raw_output_axis_state",
            bytemuck::cast_slice(&initial),
            wgpu::BufferUsages::STORAGE,
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for page in &plane.pages {
            self.check_cancelled()?;
            let p = uniform(
                self,
                &Axis {
                    range: [page.first_pixel, page.n_pixels, mode, split],
                    step,
                },
            );
            let output = RawBuffer {
                buffer: page.buffer.clone(),
            };
            self.raw_dispatch(
                AXES,
                &[&p, &state, &output],
                &[U, RW, RW],
                1,
                "raw_output_axes",
            )
            .encode(&mut encoder, "raw_output_axes");
        }
        self.raw_submit(encoder)?;
        Ok(plane)
    }
    pub(super) fn raw_output_paged_gpu(
        &self,
        sensor: &RawSensor,
        params: &RawGpuParams,
        input: &RawPlane,
    ) -> Result<Vec<ImagePage>, String> {
        let limit = super::super::bounded::storage_binding_limit(self)?;
        self.raw_output_paged_with_limit(sensor, params, input, limit)
    }
    fn raw_output_paged_with_limit(
        &self,
        sensor: &RawSensor,
        params: &RawGpuParams,
        input: &RawPlane,
        limit: u64,
    ) -> Result<Vec<ImagePage>, String> {
        self.check_cancelled()?;
        let (rw, rh) = sensor.rotated_dimensions()?;
        let (sw, sh) = sensor.stretched_dimensions()?;
        let (ow, oh) = sensor.output_dimensions()?;
        if input.width != sensor.width || input.height != sensor.height {
            return Err("RAW output input dimensions disagree with sensor".into());
        }
        let capacity = output_capacity(
            limit,
            params.lens.is_some(),
            sensor.fuji_width != 0,
            (rw, rh),
            (ow, oh),
        )?;
        let scratch = Scratch::new(
            self,
            capacity,
            if params.lens.is_some() {
                12
            } else if sensor.fuji_width != 0 {
                4
            } else {
                1
            },
        );
        // Native IO.mix_green runs before Fuji rotation. Preserve the caller's
        // demosaic pages, and average on a GPU-owned copy rather than mutating it.
        let mix_green = sensor.mix_green && !params.green_merge;
        let merged = if mix_green {
            let plane =
                self.raw_allocate_plane(sensor.width, sensor.height, "raw_merged_green_pages")?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            for page in &plane.pages {
                self.check_cancelled()?;
                input.copy_range(
                    &mut encoder,
                    u64::from(page.first_pixel),
                    page.n_pixels,
                    &page.buffer,
                    0,
                    false,
                );
                let p = uniform(self, &[page.n_pixels, 0, 0, 0]);
                let image = RawBuffer {
                    buffer: page.buffer.clone(),
                };
                self.raw_dispatch(
                    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_mix_green.wgsl"),
                    &[&p, &image],
                    &[U, RW],
                    page.n_pixels,
                    "raw_mix_green_pages",
                )
                .encode(&mut encoder, "raw_mix_green_pages");
            }
            self.raw_submit(encoder)?;
            Some(plane)
        } else {
            None
        };
        let input = merged.as_ref().unwrap_or(input);
        let rotated = if sensor.fuji_width != 0 {
            let plane = self.raw_allocate_plane(rw, rh, "raw_rotated_pages")?;
            let step = 0.5f64.sqrt();
            let high = step as f32;
            for first in (0..rw * rh).step_by(capacity as usize) {
                self.check_cancelled()?;
                let n = capacity.min(rw * rh - first);
                let mut p = Geometry {
                    dimensions: [sensor.width, sensor.height, rw, rh],
                    tile: [first, n, 0, 0],
                    controls: [sensor.fuji_width - 1, sensor.colors, 0, 0],
                    step: [high, (step - f64::from(high)) as f32, 0.0, 0.0],
                };
                let mut encoder = self.device.create_command_encoder(&Default::default());
                scratch.reset(&mut encoder);
                encoder.clear_buffer(&scratch.output.buffer, 0, None);
                self.raw_output_geometry_dispatch(&p, &scratch, &mut encoder);
                self.raw_submit(encoder)?;
                self.raw_output_gather(
                    &scratch,
                    n * 4,
                    4,
                    input
                        .pages
                        .iter()
                        .map(|p| (p.first_pixel, p.n_pixels, &p.buffer)),
                )?;
                p.tile[2] = 1;
                let mut encoder = self.device.create_command_encoder(&Default::default());
                self.raw_output_geometry_dispatch(&p, &scratch, &mut encoder);
                scatter_raw(&mut encoder, &scratch.output.buffer, first, n, &plane.pages);
                self.raw_submit(encoder)?;
            }
            Some(plane)
        } else {
            None
        };
        let source = rotated.as_ref().unwrap_or(input);
        let camera = self.raw_allocate_plane(rw, rh, "raw_camera_pages")?;
        // Camera conversion is pointwise and independent of page origins.
        for first in (0..rw * rh).step_by(capacity as usize) {
            self.check_cancelled()?;
            let n = capacity.min(rw * rh - first);
            let p = uniform(
                self,
                &ColorPass {
                    out_width: n,
                    out_height: 1,
                    iwidth: n,
                    iheight: 1,
                    flip: 0,
                    colors: if mix_green { 3 } else { sensor.colors },
                    adaptation_active: 0,
                    _pad0: 0,
                    tint: 1.0,
                    _pad1: 0.0,
                    _pad2: 0.0,
                    _pad3: 0.0,
                    camera_to_aces: params.camera_to_aces,
                    adaptation: [[0.0; 4]; 3],
                    aces_to_xyz: [[0.0; 4]; 3],
                    xyz_to_aces: [[0.0; 4]; 3],
                },
            );
            let mut encoder = self.device.create_command_encoder(&Default::default());
            source.copy_range(
                &mut encoder,
                u64::from(first),
                n,
                &scratch.values.buffer,
                0,
                false,
            );
            self.raw_dispatch(
                concat!(
                    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
                    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_camera.wgsl")
                ),
                &[&p, &scratch.values, &scratch.output],
                &[U, RO, RW],
                n,
                "raw_camera_pages",
            )
            .encode(&mut encoder, "raw_camera_pages");
            scatter_raw(
                &mut encoder,
                &scratch.output.buffer,
                first,
                n,
                &camera.pages,
            );
            self.raw_submit(encoder)?;
        }
        let vertical = sensor.pixel_aspect < 1.0;
        let stretch = if sensor.pixel_aspect != 1.0 {
            let step = if vertical {
                sensor.pixel_aspect
            } else {
                1.0 / sensor.pixel_aspect
            };
            let bits = step.to_bits();
            Some(self.raw_output_axes(
                if vertical { sh } else { sw },
                if vertical { rh } else { rw },
                [bits as u32, (bits >> 32) as u32, 0, 0],
                [0, 0],
                0,
            )?)
        } else {
            None
        };
        let oriented = self.allocate_image_pages(ow, oh, "raw_oriented_pages")?;
        for first in (0..ow * oh).step_by(capacity as usize) {
            self.check_cancelled()?;
            let n = capacity.min(ow * oh - first);
            let mut p = Geometry {
                dimensions: [sw, sh, ow, oh],
                tile: [first, n, if stretch.is_some() { 2 } else { 4 }, 0],
                controls: [sensor.flip, u32::from(vertical), rw, 0],
                step: [0.0; 4],
            };
            let mut encoder = self.device.create_command_encoder(&Default::default());
            scratch.reset(&mut encoder);
            self.raw_output_geometry_dispatch(&p, &scratch, &mut encoder);
            self.raw_submit(encoder)?;
            if let Some(axis) = &stretch {
                self.raw_output_gather(
                    &scratch,
                    n,
                    4,
                    axis.pages
                        .iter()
                        .map(|p| (p.first_pixel, p.n_pixels, &p.buffer)),
                )?;
                p.tile[2] = 3;
                let mut encoder = self.device.create_command_encoder(&Default::default());
                scratch.reset(&mut encoder);
                self.raw_output_geometry_dispatch(&p, &scratch, &mut encoder);
                self.raw_submit(encoder)?;
            }
            self.raw_output_gather(
                &scratch,
                n,
                4,
                camera
                    .pages
                    .iter()
                    .map(|p| (p.first_pixel, p.n_pixels, &p.buffer)),
            )?;
            p.tile[2] = 5;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.raw_output_geometry_dispatch(&p, &scratch, &mut encoder);
            scatter(&mut encoder, &scratch.output.buffer, first, n, &oriented);
            self.raw_submit(encoder)?;
        }
        let result = if let Some(lens) = &params.lens {
            if lens.dimensions[..2] != [ow, oh] {
                return Err("RAW Lensfun dimensions disagree with oriented image".into());
            }
            self.raw_output_lens_pages(lens, &oriented, capacity, &scratch)?
        } else {
            oriented
        };
        for page in &result {
            self.check_cancelled()?;
            let p = uniform(
                self,
                &Adaptation {
                    matrix: pack_rows(&params.adaptation),
                    count: page.n_pixels,
                    active: u32::from(params.adaptation_active),
                    tint: params.tint,
                    pad: 0,
                },
            );
            let pixels = RawBuffer {
                buffer: page.buffer.clone(),
            };
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.raw_dispatch(
                include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_adaptation.wgsl"),
                &[&p, &pixels],
                &[U, RW],
                page.n_pixels,
                "raw_adaptation_pages",
            )
            .encode(&mut encoder, "raw_adaptation_pages");
            self.raw_submit(encoder)?;
        }
        self.check_cancelled()?;
        Ok(result)
    }
    // Keep dispatch range/mode and independent lens recurrence bindings explicit.
    #[allow(clippy::too_many_arguments)]
    fn raw_output_lens_dispatch(
        &self,
        lens: &RawBuffer,
        first: u32,
        count: u32,
        mode: u32,
        scratch: &Scratch,
        state: &RawBuffer,
        encoder: &mut ObservedEncoder,
    ) {
        let tile = uniform(self, &[first, count, mode, 0]);
        self.raw_dispatch(
            LENS,
            &[
                lens,
                &tile,
                &scratch.indices,
                &scratch.values,
                &scratch.coords,
                &scratch.output,
                &scratch.bounds,
                state,
            ],
            &[U, U, RW, RO, RW, RW, RW, RW],
            count,
            "raw_lens_pages",
        )
        .encode(encoder, "raw_lens_pages");
    }
    fn raw_output_lens_pages(
        &self,
        lens: &spektrafilm_backend::RawLensParams,
        input: &[ImagePage],
        capacity: u32,
        scratch: &Scratch,
    ) -> Result<Vec<ImagePage>, String> {
        let width = lens.dimensions[0];
        let height = lens.dimensions[1];
        let count = width * height;
        let round_negative = |hi: f32, lo: f32| (-hi) + (-lo);
        let axes = self.raw_output_axes(
            width + height,
            width,
            [
                lens.normalization[0].to_bits(),
                lens.normalization[1].to_bits(),
                round_negative(lens.center[2], lens.center[3]).to_bits(),
                0,
            ],
            [round_negative(lens.center[0], lens.center[1]).to_bits(), 0],
            1,
        )?;
        let u = uniform(self, &super::lens::LensUniform::from(lens));
        let axis_requests = if lens.flags[3] != 0 && matches!(lens.dimensions[2], 1 | 3) {
            4
        } else {
            2
        };
        let vignette = self.allocate_image_pages(width, height, "raw_vignette_pages")?;
        let state = buffer(self, "raw_lens_vignette_recurrence", 16);
        for first in (0..count).step_by(capacity as usize) {
            self.check_cancelled()?;
            let n = capacity.min(count - first);
            let mut encoder = self.device.create_command_encoder(&Default::default());
            scratch.reset(&mut encoder);
            self.raw_output_lens_dispatch(&u, first, n, 3, scratch, &state, &mut encoder);
            self.raw_submit(encoder)?;
            self.raw_output_gather(
                scratch,
                n * 2,
                4,
                axes.pages
                    .iter()
                    .map(|p| (p.first_pixel, p.n_pixels, &p.buffer)),
            )?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            scratch.reset(&mut encoder);
            self.raw_output_lens_dispatch(&u, first, n, 5, scratch, &state, &mut encoder);
            self.raw_submit(encoder)?;
            self.raw_output_gather(
                scratch,
                n,
                3,
                input.iter().map(|p| (p.first_pixel, p.n_pixels, &p.buffer)),
            )?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.raw_output_lens_dispatch(&u, first, n, 4, scratch, &state, &mut encoder);
            scatter(&mut encoder, &scratch.output.buffer, first, n, &vignette);
            self.raw_submit(encoder)?;
        }
        let output = self.allocate_image_pages(width, height, "raw_lens_output_pages")?;
        for first in (0..count).step_by(capacity as usize) {
            self.check_cancelled()?;
            let n = capacity.min(count - first);
            let mut encoder = self.device.create_command_encoder(&Default::default());
            scratch.reset(&mut encoder);
            self.raw_output_lens_dispatch(&u, first, n, 0, scratch, &state, &mut encoder);
            self.raw_submit(encoder)?;
            self.raw_output_gather(
                scratch,
                n * axis_requests,
                4,
                axes.pages
                    .iter()
                    .map(|p| (p.first_pixel, p.n_pixels, &p.buffer)),
            )?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            scratch.reset(&mut encoder);
            self.raw_output_lens_dispatch(&u, first, n, 1, scratch, &state, &mut encoder);
            self.raw_submit(encoder)?;
            self.raw_output_gather(
                scratch,
                n * 12,
                3,
                vignette
                    .iter()
                    .map(|p| (p.first_pixel, p.n_pixels, &p.buffer)),
            )?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.raw_output_lens_dispatch(&u, first, n, 2, scratch, &state, &mut encoder);
            scatter(&mut encoder, &scratch.output.buffer, first, n, &output);
            self.raw_submit(encoder)?;
        }
        Ok(output)
    }
}

#[cfg(test)]
mod logic_tests {
    use super::*;

    #[test]
    fn output_capacity_requires_mode_specific_minimum_binding() {
        for (lens, fuji, bytes) in [
            (false, false, 48),
            (false, true, 64),
            (true, false, 192),
            (true, true, 192),
        ] {
            assert!(output_capacity(bytes - 1, lens, fuji, (10, 10), (10, 10)).is_err());
            assert_eq!(
                output_capacity(bytes, lens, fuji, (10, 10), (10, 10)).unwrap(),
                1
            );
            assert_eq!(
                output_capacity(bytes * 3 - 1, lens, fuji, (10, 10), (10, 10)).unwrap(),
                2
            );
        }
    }

    #[test]
    fn output_capacity_is_capped_by_larger_geometry_and_dispatch_limit() {
        assert_eq!(
            output_capacity(48 * 1000, false, false, (7, 9), (3, 4)).unwrap(),
            63
        );
        assert_eq!(
            output_capacity(48 * 1000, false, false, (3, 4), (7, 9)).unwrap(),
            63
        );
        assert_eq!(
            output_capacity(48 * 100_000, false, false, (500, 500), (500, 500)).unwrap(),
            65_536
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct RestoreLimit(Option<std::ffi::OsString>);
    impl Drop for RestoreLimit {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(v) => std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", v),
                    None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
                }
            }
        }
    }
    fn read_pages(gpu: &WgpuBackend, pages: &[ImagePage]) -> Vec<f32> {
        pages
            .iter()
            .flat_map(|p| {
                gpu.pipe
                    .readback_buffer_as::<f32>(&p.buffer, u64::from(p.n_pixels) * 12)
            })
            .collect()
    }
    fn input(gpu: &WgpuBackend, sensor: &RawSensor, values: &[u32]) -> RawPlane {
        let plane = gpu
            .raw_allocate_plane(sensor.width, sensor.height, "output_test_input")
            .unwrap();
        let source = RawBuffer::upload(
            &gpu.device,
            "output_test_upload",
            bytemuck::cast_slice(values),
            wgpu::BufferUsages::COPY_SRC,
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        scatter_raw(
            &mut encoder,
            &source.buffer,
            0,
            sensor.width * sensor.height,
            &plane.pages,
        );
        gpu.queue.submit(Some(encoder.finish()));
        plane
    }
    fn reference(
        gpu: &WgpuBackend,
        sensor: &RawSensor,
        params: &RawGpuParams,
        values: &[u32],
    ) -> Vec<f32> {
        let mut values = values.to_vec();
        if sensor.mix_green && !params.green_merge {
            for p in values.as_chunks_mut::<4>().0 {
                p[1] = (p[1] + p[3]) >> 1;
            }
        }
        let source = RawBuffer::upload(
            &gpu.device,
            "output_reference",
            bytemuck::cast_slice(&values),
            wgpu::BufferUsages::STORAGE,
        );
        let rotated = gpu.raw_geometry_pass(sensor, &source, true).unwrap();
        let (camera, conversion) = gpu
            .raw_camera_conversion(sensor, params, rotated.as_ref().map_or(&source, |p| &p.0))
            .unwrap();
        let stretched = gpu.raw_geometry_pass(sensor, &camera, false).unwrap();
        let (iw, ih) = sensor.stretched_dimensions().unwrap();
        let (ow, oh) = sensor.output_dimensions().unwrap();
        let p = uniform(
            gpu,
            &ColorPass {
                out_width: ow,
                out_height: oh,
                iwidth: iw,
                iheight: ih,
                flip: sensor.flip,
                colors: 3,
                adaptation_active: 0,
                _pad0: 0,
                tint: 1.0,
                _pad1: 0.0,
                _pad2: 0.0,
                _pad3: 0.0,
                camera_to_aces: params.camera_to_aces,
                adaptation: pack_rows(&params.adaptation),
                aces_to_xyz: pack_rows(&ACES_TO_XYZ),
                xyz_to_aces: pack_rows(&XYZ_TO_ACES),
            },
        );
        let output = buffer(gpu, "output_reference_rgb", u64::from(ow * oh) * 12);
        let lens_output = params
            .lens
            .as_ref()
            .map(|_| buffer(gpu, "output_reference_lens", u64::from(ow * oh) * 12));
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        if let Some((_, p)) = &rotated {
            p.encode(&mut encoder, "reference_fuji");
        }
        conversion.encode(&mut encoder, "reference_camera");
        if let Some((_, p)) = &stretched {
            p.encode(&mut encoder, "reference_stretch");
        }
        gpu.raw_dispatch(
            include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_convert_to_rgb.wgsl"),
            &[&p, stretched.as_ref().map_or(&camera, |p| &p.0), &output],
            &[U, RO, RW],
            ow * oh,
            "reference_orientation",
        )
        .encode(&mut encoder, "reference_orientation");
        if let (Some(lens), Some(target)) = (&params.lens, &lens_output) {
            for pass in gpu.prepare_lens_passes(lens, &output, target) {
                pass.encode(&mut encoder, "reference_lens");
            }
        }
        let final_output = lens_output.as_ref().unwrap_or(&output);
        let p = uniform(
            gpu,
            &Adaptation {
                matrix: pack_rows(&params.adaptation),
                count: ow * oh,
                active: u32::from(params.adaptation_active),
                tint: params.tint,
                pad: 0,
            },
        );
        gpu.raw_dispatch(
            include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_adaptation.wgsl"),
            &[&p, final_output],
            &[U, RW],
            ow * oh,
            "reference_adaptation",
        )
        .encode(&mut encoder, "reference_adaptation");
        gpu.queue.submit(Some(encoder.finish()));
        gpu.pipe
            .readback_buffer_as::<f32>(&final_output.buffer, u64::from(ow * oh) * 12)
    }

    #[test]
    fn raw_output_cancellation_stops_between_tiles_and_preserves_healthy_reuse() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let gpu = crate::test_support::adapter();
        let sensor = super::super::tests::synthetic_sensor(17);
        let params = super::super::tests::synthetic_params(&sensor);
        let values: Vec<u32> = sensor.samples.iter().flat_map(|&v| [v as u32; 4]).collect();
        let plane = input(&gpu, &sensor, &values);
        let expected = reference(&gpu, &sensor, &params, &values);
        let cancellation = Arc::new(AtomicBool::new(false));
        let readbacks = Arc::new(AtomicUsize::new(0));
        gpu.pipe.test_on_readback({
            let cancellation = cancellation.clone();
            let readbacks = readbacks.clone();
            move || {
                readbacks.fetch_add(1, Ordering::Relaxed);
                cancellation.store(true, Ordering::Relaxed);
            }
        });
        let mut operation = gpu.derived_explicit();
        operation.pipe = operation.pipe.with_cancellation(Some(cancellation.clone()));
        operation.cancellation = Some(cancellation);
        // 16-pixel output tiles: camera conversion and the first geometry
        // request have run before its real footprint readback cancels the call.
        assert_eq!(
            operation
                .raw_output_paged_with_limit(&sensor, &params, &plane, 768)
                .err()
                .unwrap(),
            "operation cancelled"
        );
        assert_eq!(
            readbacks.load(Ordering::Relaxed),
            1,
            "cancelled RAW must not start the next tile's footprint readback"
        );
        gpu.pipe.test_on_readback(|| {});
        let actual = gpu
            .raw_output_paged_with_limit(&sensor, &params, &plane, 768)
            .unwrap();
        assert_eq!(read_pages(&gpu, &actual), expected);
        let healthy = operation.with_cancellation(None).unwrap();
        let actual = healthy
            .raw_process_sensor_retained(&sensor, &params)
            .unwrap()
            .unwrap();
        assert!(actual.is_retained());
    }
    fn parity(fuji: bool, aspect: f64, flip: u32, lens_enabled: bool) {
        let gpu = crate::test_support::adapter();
        let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
        unsafe {
            std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES");
        }
        let mut sensor = super::super::tests::synthetic_sensor(17);
        sensor.flip = flip;
        sensor.pixel_aspect = aspect;
        if fuji {
            sensor.fuji_width = 8;
        }
        sensor.colors = 4;
        sensor.mix_green = true;
        let mut params = super::super::tests::synthetic_params(&sensor);
        params.green_merge = false;
        params.adaptation_active = true;
        params.tint = 0.83;
        params.adaptation = [[1.01, 0.01, -0.02], [0.02, 0.98, 0.01], [-0.01, 0.03, 1.02]];
        params.camera_to_aces = [
            [0.9, 0.1, 0.05, 0.01],
            [0.02, 0.95, 0.04, 0.03],
            [0.04, 0.02, 0.93, 0.02],
        ];
        let (ow, oh) = sensor.output_dimensions().unwrap();
        if lens_enabled {
            let step = 0.000_491_314_271_9_f64;
            params.lens = Some(spektrafilm_backend::RawLensParams {
                dimensions: [ow, oh, 3, 2],
                flags: [7, 1, 1, 1],
                normalization: [
                    step as f32,
                    (step - f64::from(step as f32)) as f32,
                    1.0 / step as f32,
                    0.0,
                ],
                center: [0.002, 1e-11, 0.003, -1e-11],
                distortion: [0.002, 0.00003, 0.0004, 0.0],
                tca: [0.97, 1.02, 0.01, 0.02],
                tca_tail: [0.0001, 0.0002, 0.0, 0.0],
                vignette: [0.03, -0.002, 0.0001, 0.03125],
                mapping: [1.0, 1.0, 1.0, 0.93],
            });
        }
        let values: Vec<u32> = (0..sensor.width * sensor.height)
            .flat_map(|i| {
                [
                    i * 97 % 65536,
                    (i * 251 + 17) % 65536,
                    (i * 431 + 13) % 65536,
                    (i * 71 + 3) % 65536,
                ]
            })
            .collect();
        let expected = reference(&gpu, &sensor, &params, &values);
        let full = input(&gpu, &sensor, &values);
        let full = read_pages(
            &gpu,
            &gpu.raw_output_paged_gpu(&sensor, &params, &full).unwrap(),
        );
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "384");
        }
        let tiny = input(&gpu, &sensor, &values);
        assert!(tiny.pages.len() > 1);
        let tiny_pages = gpu.raw_output_paged_gpu(&sensor, &params, &tiny).unwrap();
        assert!(tiny_pages.len() > 1);
        let tiny = read_pages(&gpu, &tiny_pages);
        assert_eq!(
            full, tiny,
            "tile/page boundaries must not change numerical output"
        );
        assert_eq!(
            tiny, expected,
            "bounded transport must preserve the existing full-frame shader arithmetic"
        );
    }
    #[test]
    fn tiny_binding_fuji_camera_horizontal_stretch_flipped_matches_full_frame() {
        parity(true, 1.37, 7, false);
    }
    #[test]
    fn tiny_binding_vertical_stretch_transpose_lens_tca_vignette_cat_tint_matches_full_frame() {
        parity(false, 0.73, 6, true);
    }
    #[test]
    fn tiny_binding_identity_geometry_preserves_camera_ushort_quantization() {
        parity(false, 1.0, 0, false);
    }
    #[test]
    fn tiny_binding_orientation_without_stretch_matches_full_frame() {
        parity(false, 1.0, 5, false);
    }
    #[test]
    fn tiny_binding_fuji_and_lens_preserve_both_quantization_boundaries() {
        parity(true, 0.81, 3, true);
    }
}
