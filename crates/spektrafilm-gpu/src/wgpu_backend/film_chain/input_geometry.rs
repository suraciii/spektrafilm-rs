//! Working geometry on retained samples: rotation, exact pixel crop, and
//! resize on the compute device.
//!
//! The host resolves every number this module uses — quarter turns, the crop
//! rectangle (`crate::resizing::requested_crop_rect`), the destination grid,
//! and the working pixel pitch — so the device cannot disagree with the
//! runtime about the raster. What runs here is exactly the reference's own
//! kernel arithmetic on the samples that are already resident:
//!
//! * rotation and crop are verbatim pixel moves (`rotated_quarter_turns`
//!   followed by `crop_image`);
//! * `Spline3` reproduces `rescale_spline3` in binary64 — the Gaussian
//!   antialias prefilter with mirror extension and a normalized kernel, the
//!   cubic-spline prefilter recurrence, the half-pixel 4×4 tap gather, and the
//!   clamp to the input interval measured on the device;
//! * `PreviewTriangle` reproduces the preview resampler's triangle kernel, row
//!   pass then column pass, with the reference's own tap normalization.
//!
//! No host image arithmetic is involved and no intermediate is materialized:
//! the samples arrive as a retained carrier and leave as one, so the route
//! never downloads and reuploads the source to cross this stage.
//!
//! Retained carriers are paged. Storage windows obey the configured binding
//! cap; global coordinates and carried recurrence state survive every split.
//! Binary64 intermediates and Gaussian coefficients are independently paged
//! COPY-only transports; no allocation scales with the complete image size.

use super::super::bounded::storage_binding_limit;
use super::super::paged::ImagePage;
use super::super::{ObservedEncoder, WgpuBackend, dispatch_grid, dispatch_linear};
use super::device_image::BufferPayload;
use super::device_image::stage_output_buffer;
use spektrafilm_backend::telemetry::{Executor, ObservationKind};
use spektrafilm_backend::{DeviceImage, InputGeometryGpuParams, InputResizeMethod};

const WIDEN_SHADER: &str =
    include_str!("../../../../spektrafilm-shaders/wgsl/input_geometry_widen.wgsl");
const STATS_SHADER: &str =
    include_str!("../../../../spektrafilm-shaders/wgsl/input_geometry_stats.wgsl");
const TRIANGLE_SHADER: &str =
    include_str!("../../../../spektrafilm-shaders/wgsl/input_geometry_triangle.wgsl");
const GAUSSIAN_WEIGHTS_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/input_geometry_gaussian_weights.wgsl")
);
const GAUSSIAN_NORMALIZE_SHADER: &str =
    include_str!("../../../../spektrafilm-shaders/wgsl/input_geometry_gaussian_normalize.wgsl");
const GAUSSIAN_LINE_SHADER: &str =
    include_str!("../../../../spektrafilm-shaders/wgsl/input_geometry_gaussian_line.wgsl");
const SPLINE_LINE_SHADER: &str =
    include_str!("../../../../spektrafilm-shaders/wgsl/input_geometry_spline_line.wgsl");
const SPLINE_SAMPLE_SHADER: &str =
    include_str!("../../../../spektrafilm-shaders/wgsl/input_geometry_spline_sample.wgsl");
const UNIFORM: wgpu::BufferBindingType = wgpu::BufferBindingType::Uniform;
const READ: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: true };
const WRITE: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: false };

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Words {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GaussianWeightsParams {
    c: f64,
    radius: u32,
    count: u32,
    first: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SplineLineParams {
    len: u32,
    count: u32,
    mode: u32,
    first: u32,
    zn: f64,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SplineSampleParams {
    src_width: u32,
    src_height: u32,
    out_width: u32,
    out_height: u32,
    first_pixel: u32,
    count: u32,
    first_tap: u32,
    taps: u32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct TriangleParams {
    src_extent: u32,
    dst_extent: u32,
    position: u32,
    vertical: u32,
    count: u32,
    first_tap: u32,
    taps: u32,
    left: u32,
    right: u32,
    _pad: u32,
}

fn mirror(index: i64, len: u32) -> u32 {
    if len <= 1 {
        return 0;
    }
    let period = 2 * i64::from(len) - 2;
    let folded = index.rem_euclid(period);
    if folded >= i64::from(len) {
        (period - folded) as u32
    } else {
        folded as u32
    }
}

// Only coordinates are resolved here; weights and image arithmetic stay on device.
fn spline_taps(position: u32, len: u32, out: u32) -> [u32; 4] {
    let mut x = (f64::from(position) + 0.5) * (f64::from(len) / f64::from(out)) - 0.5;
    if len <= 1 {
        x = 0.0;
    } else {
        let period = f64::from(len) * 2.0 - 2.0;
        x = x.rem_euclid(period);
        if x > f64::from(len - 1) {
            x = period - x;
        }
    }
    let start = x.floor() as i64 - 1;
    std::array::from_fn(|i| mirror(start + i as i64, len))
}

/// Raw binary64 sample transport. Fixed-size pages allow direct page lookup;
/// a gathered RGB pixel may straddle pages without changing its sample order.
struct GeometryTransport {
    pages: Vec<wgpu::Buffer>,
    page_bytes: u64,
}

impl GeometryTransport {
    fn copy_to(
        &self,
        encoder: &mut ObservedEncoder,
        first: u64,
        destination: &wgpu::Buffer,
        offset: u64,
        bytes: u64,
    ) {
        self.copy(encoder, first, destination, offset, bytes, false);
    }

    fn copy_from(
        &self,
        encoder: &mut ObservedEncoder,
        source: &wgpu::Buffer,
        offset: u64,
        first: u64,
        bytes: u64,
    ) {
        self.copy(encoder, first, source, offset, bytes, true);
    }

    fn copy(
        &self,
        encoder: &mut ObservedEncoder,
        first: u64,
        buffer: &wgpu::Buffer,
        offset: u64,
        bytes: u64,
        scatter: bool,
    ) {
        let mut copied = 0;
        while copied < bytes {
            let absolute = first + copied;
            let page = &self.pages[(absolute / self.page_bytes) as usize];
            let local = absolute % self.page_bytes;
            let count = (page.size() - local).min(bytes - copied);
            if scatter {
                encoder.copy_buffer_to_buffer(buffer, offset + copied, page, local, count);
            } else {
                encoder.copy_buffer_to_buffer(page, local, buffer, offset + copied, count);
            }
            copied += count;
        }
    }
}

impl WgpuBackend {
    /// Rotation, exact pixel crop, and resize of a retained carrier, all on
    /// this device. See the module documentation for the reference each
    /// method reproduces.
    pub(in crate::wgpu_backend) fn encode_input_geometry(
        &self,
        image: &DeviceImage,
        params: &InputGeometryGpuParams,
    ) -> Result<Option<DeviceImage>, String> {
        self.check_cancelled()?;
        let source = self.buffer_payload(image)?;
        let (src_width, src_height) = (image.width(), image.height());
        if src_width == 0 || src_height == 0 || params.pixel_pitch_um <= 0.0 {
            return Err("input geometry: empty source or non-positive working pitch".into());
        }
        if !params.pixel_pitch_um.is_finite() {
            return Err("input geometry: working pixel pitch must be finite".into());
        }
        if params.rotation_quarter_turns > 3 {
            return Err(format!(
                "input geometry: rotation {} is not a whole quarter turn",
                params.rotation_quarter_turns
            ));
        }
        let turns = params.rotation_quarter_turns;
        let (rotated_width, rotated_height) = if turns.is_multiple_of(2) {
            (src_width, src_height)
        } else {
            (src_height, src_width)
        };
        let crop = match params.crop {
            Some([x, y, width, height]) => {
                if width == 0 || height == 0 {
                    return Err("input geometry: crop rectangle is empty".into());
                }
                let fits = x.checked_add(width).is_some_and(|end| end <= rotated_width)
                    && y.checked_add(height)
                        .is_some_and(|end| end <= rotated_height);
                if !fits {
                    return Err(format!(
                        "input geometry: crop [{x}, {y}, {width}, {height}] exceeds the \
                         {rotated_width}x{rotated_height} rotated source"
                    ));
                }
                Some([x, y, width, height])
            }
            None => None,
        };
        if turns == 0 && crop.is_none() && params.resize.is_none() {
            // The reference borrows the same samples when the request asks for
            // no geometry at all, so the carrier is reused, not copied.
            return Ok(Some(image.clone()));
        }
        let (cropped_width, cropped_height) =
            crop.map_or((rotated_width, rotated_height), |r| (r[2], r[3]));
        let (out_width, out_height) = match params.resize {
            Some(resize) => {
                if resize.width == 0 || resize.height == 0 {
                    return Err("input geometry: resize dimensions must both be nonzero".into());
                }
                (resize.width, resize.height)
            }
            None => (cropped_width, cropped_height),
        };
        let spline = params
            .resize
            .is_some_and(|r| r.method == InputResizeMethod::Spline3)
            && (out_width != cropped_width || out_height != cropped_height);
        if spline {
            // The reference resize is binary64; a device without native f64
            // fails this operation rather than approximating it.
            if !self.device.features().contains(wgpu::Features::SHADER_F64) {
                return Err(
                    "input geometry: the spline resize requires device binary64 arithmetic".into(),
                );
            }
        }
        let limit = storage_binding_limit(self)?;
        let minimum = if spline { 24 } else { 12 };
        if limit < minimum {
            return Err(format!(
                "input geometry: minimum working set needs {minimum} bytes per binding; limit is {limit}"
            ));
        }

        let scope = self.device.context.enabled().then(|| {
            self.device.context.scope(
                "geometry",
                ObservationKind::Stage,
                self.device.context.purpose(),
            )
        });
        if let Some(scope) = &scope {
            scope.context().record_executor(Executor::Gpu, None);
        }

        let mut encoder = self.device.create_command_encoder(&Default::default());
        // Pure copies preserve quarter-turn mapping and cross source pages.
        let cropped_pages = if turns == 0 && crop.is_none() {
            source.pages.clone()
        } else {
            let pages =
                self.allocate_image_pages(cropped_width, cropped_height, "geometry_crop")?;
            let [x0, y0, _, _] = crop.unwrap_or([0, 0, cropped_width, cropped_height]);
            for page in &pages {
                self.check_cancelled()?;
                let mut local = 0;
                while local < page.n_pixels {
                    self.check_cancelled()?;
                    let pixel = page.first_pixel + local;
                    let x = pixel % cropped_width + x0;
                    let y = pixel / cropped_width + y0;
                    let (sx, sy) = match turns {
                        0 => (x, y),
                        1 => (src_width - 1 - y, x),
                        2 => (src_width - 1 - x, src_height - 1 - y),
                        _ => (y, src_height - 1 - x),
                    };
                    let count = if turns == 0 {
                        (cropped_width - pixel % cropped_width).min(page.n_pixels - local)
                    } else {
                        1
                    };
                    self.copy_samples_to(
                        &mut encoder,
                        source,
                        (u64::from(sy) * u64::from(src_width) + u64::from(sx)) * 3,
                        u64::from(count) * 3,
                        &page.buffer,
                        u64::from(local) * 3,
                    )?;
                    local += count;
                }
                self.geometry_submit_pending(&mut encoder)?;
            }
            pages
        };
        let cropped = BufferPayload::from_pages(
            self.backend_id(),
            cropped_pages,
            cropped_width,
            cropped_height,
        );
        let pages = match params.resize {
            None => cropped.pages.clone(),
            Some(_) if out_width == cropped_width && out_height == cropped_height => {
                cropped.pages.clone()
            }
            Some(resize) if resize.method == InputResizeMethod::PreviewTriangle => {
                let rows = self.geometry_triangle_axis(
                    &mut encoder,
                    &cropped,
                    cropped_width,
                    out_height,
                    true,
                    limit,
                )?;
                let rows =
                    BufferPayload::from_pages(self.backend_id(), rows, cropped_width, out_height);
                self.geometry_triangle_axis(
                    &mut encoder,
                    &rows,
                    out_width,
                    out_height,
                    false,
                    limit,
                )?
            }
            Some(_) => {
                self.encode_spline_resize(&mut encoder, &cropped, out_width, out_height, limit)?
            }
        };
        self.geometry_submit_pending(&mut encoder)?;
        Ok(Some(self.retain_image_pages(
            pages,
            out_width,
            out_height,
            image.domain(),
            image.space(),
            Some(params.pixel_pitch_um),
        )))
    }

    #[allow(clippy::too_many_arguments)]
    fn geometry_triangle_axis(
        &self,
        encoder: &mut ObservedEncoder,
        source: &BufferPayload,
        width: u32,
        height: u32,
        vertical: bool,
        limit: u64,
    ) -> Result<Vec<ImagePage>, String> {
        let pages = self.allocate_image_pages(width, height, "geometry_preview")?;
        let extent = if vertical {
            source.height
        } else {
            source.width
        };
        let output_extent = if vertical { height } else { width };
        let others = if vertical { width } else { height };
        let capacity = (limit / 12).min(u64::from(u32::MAX)) as u32;
        for position in 0..output_extent {
            self.check_cancelled()?;
            let ratio = extent as f32 / output_extent as f32;
            let support = ratio.max(1.0);
            let centre = (position as f32 + 0.5) * ratio;
            let left = ((centre - support).floor() as i64).clamp(0, i64::from(extent) - 1) as u32;
            let right = ((centre + support).ceil() as i64)
                .clamp(i64::from(left) + 1, i64::from(extent)) as u32;
            let mut other = 0;
            while other < others {
                self.check_cancelled()?;
                let count = (capacity / (right - left)).max(1).min(others - other);
                let acc = stage_output_buffer(
                    &self.device,
                    u64::from(count) * 12,
                    "geometry_triangle_acc",
                );
                let taps_per_chunk = (capacity / count).max(1);
                let input = stage_output_buffer(
                    &self.device,
                    u64::from(count) * u64::from(taps_per_chunk.min(right - left)) * 12,
                    "geometry_triangle_window",
                );
                let mut tap = left;
                while tap < right {
                    self.check_cancelled()?;
                    let taps = taps_per_chunk.min(right - tap);
                    for t in 0..taps {
                        for l in 0..count {
                            let (x, y) = if vertical {
                                (other + l, tap + t)
                            } else {
                                (tap + t, other + l)
                            };
                            self.copy_samples_to(
                                encoder,
                                source,
                                (u64::from(y) * u64::from(source.width) + u64::from(x)) * 3,
                                3,
                                &input,
                                u64::from(t * count + l) * 3,
                            )?;
                        }
                    }
                    let uniform = self.geometry_uniform(
                        "geometry_triangle_params",
                        &TriangleParams {
                            src_extent: extent,
                            dst_extent: output_extent,
                            position,
                            vertical: u32::from(vertical),
                            count,
                            first_tap: tap,
                            taps,
                            left,
                            right,
                            _pad: 0,
                        },
                    );
                    self.geometry_pass_linear(
                        encoder,
                        TRIANGLE_SHADER,
                        &[UNIFORM, READ, WRITE],
                        &[&uniform, &input, &acc],
                        count,
                        "geometry_triangle",
                    );
                    self.geometry_submit_pending(encoder)?;
                    tap += taps;
                }
                for l in 0..count {
                    let pixel = if vertical {
                        position * width + other + l
                    } else {
                        (other + l) * width + position
                    };
                    self.copy_samples_from(
                        encoder,
                        &acc,
                        u64::from(l) * 3,
                        &pages,
                        u64::from(pixel) * 3,
                        3,
                    )?;
                }
                self.geometry_submit_pending(encoder)?;
                other += count;
            }
        }
        Ok(pages)
    }

    /// Allocate sample-aligned COPY pages, independent of image footprint.
    fn geometry_transport(
        &self,
        bytes: u64,
        label: &'static str,
        limit: u64,
    ) -> Result<GeometryTransport, String> {
        let page_bytes = (limit / 8) * 8;
        let mut pages = Vec::new();
        for page in 0..bytes.div_ceil(page_bytes) {
            self.check_cancelled()?;
            pages.push(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: page_bytes.min(bytes - page * page_bytes),
                mapped_at_creation: false,
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            }));
        }
        Ok(GeometryTransport { pages, page_bytes })
    }

    // Queue order preserves recurrence state between bounded submissions.
    // The result carrier stays local until every submission accepts cancellation.
    fn geometry_submit_pending(&self, encoder: &mut ObservedEncoder) -> Result<(), String> {
        self.check_cancelled()?;
        let pending = std::mem::replace(
            encoder,
            self.device.create_command_encoder(&Default::default()),
        );
        self.queue.submit(Some(pending.finish()));
        self.check_cancelled()
    }

    fn encode_spline_resize(
        &self,
        encoder: &mut ObservedEncoder,
        cropped: &BufferPayload,
        out_width: u32,
        out_height: u32,
        limit: u64,
    ) -> Result<Vec<ImagePage>, String> {
        let (width, height) = (cropped.width, cropped.height);
        let pixels = u64::from(width) * u64::from(height);
        let capacity = (limit / 24).min(pixels).min(u64::from(u32::MAX) / 3) as u32;
        let mut current = self.geometry_transport(pixels * 24, "geometry_spline_a", limit)?;
        let mut scratch = self.geometry_transport(pixels * 24, "geometry_spline_b", limit)?;
        let input = stage_output_buffer(
            &self.device,
            u64::from(capacity) * 12,
            "geometry_widen_input",
        );
        let output = stage_output_buffer(
            &self.device,
            u64::from(capacity) * 24,
            "geometry_widen_output",
        );
        let minimum = stage_output_buffer(&self.device, 4, "geometry_minimum");
        let maximum = stage_output_buffer(&self.device, 4, "geometry_maximum");
        let mut first = 0;
        while first < pixels {
            self.check_cancelled()?;
            let count = u64::from(capacity).min(pixels - first) as u32;
            self.copy_samples_to(encoder, cropped, first * 3, u64::from(count) * 3, &input, 0)?;
            let uniform = self.geometry_uniform(
                "geometry_widen_params",
                &Words {
                    a: count * 3,
                    b: 0,
                    c: 0,
                    d: 0,
                },
            );
            self.geometry_pass_linear(
                encoder,
                WIDEN_SHADER,
                &[UNIFORM, READ, WRITE],
                &[&uniform, &input, &output],
                count * 3,
                "geometry_widen",
            );
            current.copy_from(encoder, &output, 0, first * 24, u64::from(count) * 24);
            let uniform = self.geometry_uniform(
                "geometry_range_params",
                &Words {
                    a: count * 3,
                    b: u32::from(first == 0),
                    c: 0,
                    d: 0,
                },
            );
            self.geometry_pass_groups(
                encoder,
                STATS_SHADER,
                &[UNIFORM, READ, WRITE, WRITE],
                &[&uniform, &input, &minimum, &maximum],
                1,
                "geometry_range",
            );
            self.geometry_submit_pending(encoder)?;
            first += u64::from(count);
        }
        for (axis, sigma) in [
            (
                0,
                ((height as f64 / out_height as f64 - 1.0) / 2.0).max(0.0),
            ),
            (1, ((width as f64 / out_width as f64 - 1.0) / 2.0).max(0.0)),
        ] {
            if sigma <= 1e-15 {
                continue;
            }
            self.geometry_gaussian_axis(
                encoder, &current, &scratch, width, height, axis, sigma, limit,
            )?;
            std::mem::swap(&mut current, &mut scratch);
        }
        for axis in [0, 1] {
            self.geometry_spline_axis(encoder, &current, &scratch, width, height, axis, limit)?;
            std::mem::swap(&mut current, &mut scratch);
        }
        let pages = self.allocate_image_pages(out_width, out_height, "geometry_spline_result")?;
        for page in &pages {
            self.check_cancelled()?;
            let mut local = 0;
            while local < page.n_pixels {
                self.check_cancelled()?;
                let count = (capacity / 16).max(1).min(page.n_pixels - local);
                let taps_per_chunk = (capacity / count).clamp(1, 16);
                let gathered = stage_output_buffer(
                    &self.device,
                    u64::from(count) * u64::from(taps_per_chunk) * 24,
                    "geometry_spline_taps",
                );
                let acc =
                    stage_output_buffer(&self.device, u64::from(count) * 24, "geometry_spline_acc");
                let out = stage_output_buffer(
                    &self.device,
                    u64::from(count) * 12,
                    "geometry_spline_readout",
                );
                let mut tap = 0;
                while tap < 16 {
                    self.check_cancelled()?;
                    let taps = taps_per_chunk.min(16 - tap);
                    for l in 0..count {
                        let pixel = page.first_pixel + local + l;
                        let xs = spline_taps(pixel % out_width, width, out_width);
                        let ys = spline_taps(pixel / out_width, height, out_height);
                        for t in 0..taps {
                            let index = tap + t;
                            let source = u64::from(ys[(index / 4) as usize]) * u64::from(width)
                                + u64::from(xs[(index % 4) as usize]);
                            current.copy_to(
                                encoder,
                                source * 24,
                                &gathered,
                                u64::from(t * count + l) * 24,
                                24,
                            );
                        }
                    }
                    let uniform = self.geometry_uniform(
                        "geometry_spline_sample_params",
                        &SplineSampleParams {
                            src_width: width,
                            src_height: height,
                            out_width,
                            out_height,
                            first_pixel: page.first_pixel + local,
                            count,
                            first_tap: tap,
                            taps,
                        },
                    );
                    self.geometry_pass_linear(
                        encoder,
                        SPLINE_SAMPLE_SHADER,
                        &[UNIFORM, READ, READ, READ, WRITE, WRITE],
                        &[&uniform, &gathered, &minimum, &maximum, &acc, &out],
                        count,
                        "geometry_spline_sample",
                    );
                    self.geometry_submit_pending(encoder)?;
                    tap += taps;
                }
                encoder.copy_buffer_to_buffer(
                    &out,
                    0,
                    &page.buffer,
                    u64::from(local) * 12,
                    u64::from(count) * 12,
                );
                self.geometry_submit_pending(encoder)?;
                local += count;
            }
        }
        Ok(pages)
    }

    #[allow(clippy::too_many_arguments)]
    fn geometry_gaussian_axis(
        &self,
        encoder: &mut ObservedEncoder,
        source: &GeometryTransport,
        destination: &GeometryTransport,
        width: u32,
        height: u32,
        axis: u32,
        sigma: f64,
        limit: u64,
    ) -> Result<(), String> {
        let radius = (4.0 * sigma + 0.5) as u32;
        let count = radius
            .checked_mul(2)
            .and_then(|v| v.checked_add(1))
            .ok_or("input geometry: Gaussian support overflows")?;
        let weights =
            self.geometry_transport(u64::from(count) * 8, "geometry_gaussian_weights", limit)?;
        let weight_capacity = (limit / 8).min(u64::from(u32::MAX)) as u32;
        let window = stage_output_buffer(
            &self.device,
            u64::from(weight_capacity.min(count)) * 8,
            "geometry_weight_window",
        );
        let sum = stage_output_buffer(&self.device, 8, "geometry_weight_sum");
        let mut first = 0;
        while first < count {
            self.check_cancelled()?;
            let n = weight_capacity.min(count - first);
            let uniform = self.geometry_uniform(
                "geometry_gaussian_weight_params",
                &GaussianWeightsParams {
                    c: -0.5 / (sigma * sigma),
                    radius,
                    count: n,
                    first,
                    _pad: 0,
                },
            );
            self.geometry_pass_linear(
                encoder,
                GAUSSIAN_WEIGHTS_SHADER,
                &[UNIFORM, WRITE],
                &[&uniform, &window],
                n,
                "geometry_gaussian_weights",
            );
            let uniform = self.geometry_uniform(
                "geometry_gaussian_sum_params",
                &Words {
                    a: n,
                    b: 0,
                    c: first,
                    d: 0,
                },
            );
            self.geometry_pass_groups(
                encoder,
                GAUSSIAN_NORMALIZE_SHADER,
                &[UNIFORM, WRITE, WRITE],
                &[&uniform, &window, &sum],
                1,
                "geometry_gaussian_sum",
            );
            weights.copy_from(encoder, &window, 0, u64::from(first) * 8, u64::from(n) * 8);
            self.geometry_submit_pending(encoder)?;
            first += n;
        }
        let mut first = 0;
        while first < count {
            self.check_cancelled()?;
            let n = weight_capacity.min(count - first);
            weights.copy_to(encoder, u64::from(first) * 8, &window, 0, u64::from(n) * 8);
            let uniform = self.geometry_uniform(
                "geometry_gaussian_normalize_params",
                &Words {
                    a: n,
                    b: 1,
                    c: first,
                    d: 0,
                },
            );
            self.geometry_pass_groups(
                encoder,
                GAUSSIAN_NORMALIZE_SHADER,
                &[UNIFORM, WRITE, WRITE],
                &[&uniform, &window, &sum],
                1,
                "geometry_gaussian_normalize",
            );
            weights.copy_from(encoder, &window, 0, u64::from(first) * 8, u64::from(n) * 8);
            self.geometry_submit_pending(encoder)?;
            first += n;
        }
        let capacity = (limit / 24).min(u64::from(u32::MAX)) as u32;
        let pixels = u64::from(width) * u64::from(height);
        let mut first = 0;
        while first < pixels {
            self.check_cancelled()?;
            let n = u64::from((capacity / count).max(1)).min(pixels - first) as u32;
            let taps_per_chunk = (capacity / n).min(count).max(1);
            let input = stage_output_buffer(
                &self.device,
                u64::from(n) * u64::from(taps_per_chunk) * 24,
                "geometry_gaussian_window",
            );
            let acc = stage_output_buffer(&self.device, u64::from(n) * 24, "geometry_gaussian_acc");
            let mut tap = 0;
            while tap < count {
                self.check_cancelled()?;
                let taps = taps_per_chunk.min(count - tap);
                weights.copy_to(encoder, u64::from(tap) * 8, &window, 0, u64::from(taps) * 8);
                for l in 0..n {
                    let pixel = first + u64::from(l);
                    let x = (pixel % u64::from(width)) as u32;
                    let y = (pixel / u64::from(width)) as u32;
                    for t in 0..taps {
                        let offset = i64::from(tap + t) - i64::from(radius);
                        let (sx, sy) = if axis == 0 {
                            (x, mirror(i64::from(y) + offset, height))
                        } else {
                            (mirror(i64::from(x) + offset, width), y)
                        };
                        source.copy_to(
                            encoder,
                            (u64::from(sy) * u64::from(width) + u64::from(sx)) * 24,
                            &input,
                            u64::from(t * n + l) * 24,
                            24,
                        );
                    }
                }
                let uniform = self.geometry_uniform(
                    "geometry_gaussian_line_params",
                    &Words {
                        a: n,
                        b: taps,
                        c: tap,
                        d: 0,
                    },
                );
                self.geometry_pass_linear(
                    encoder,
                    GAUSSIAN_LINE_SHADER,
                    &[UNIFORM, READ, READ, WRITE],
                    &[&uniform, &input, &window, &acc],
                    n,
                    "geometry_gaussian_line",
                );
                self.geometry_submit_pending(encoder)?;
                tap += taps;
            }
            destination.copy_from(encoder, &acc, 0, first * 24, u64::from(n) * 24);
            self.geometry_submit_pending(encoder)?;
            first += u64::from(n);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn geometry_spline_axis(
        &self,
        encoder: &mut ObservedEncoder,
        source: &GeometryTransport,
        destination: &GeometryTransport,
        width: u32,
        height: u32,
        axis: u32,
        limit: u64,
    ) -> Result<(), String> {
        let (lines, len) = if axis == 0 {
            (width, height)
        } else {
            (height, width)
        };
        if len == 1 {
            for (input, output) in source.pages.iter().zip(&destination.pages) {
                self.check_cancelled()?;
                encoder.copy_buffer_to_buffer(input, 0, output, 0, input.size());
                self.geometry_submit_pending(encoder)?;
            }
            return Ok(());
        }
        let capacity = (limit / 8).min(u64::from(u32::MAX)) as u32;
        let a = stage_output_buffer(
            &self.device,
            u64::from(capacity.min(len)) * 8,
            "geometry_spline_line_a",
        );
        let b = stage_output_buffer(
            &self.device,
            u64::from(capacity.min(len)) * 8,
            "geometry_spline_line_b",
        );
        let out = stage_output_buffer(
            &self.device,
            u64::from(capacity.min(len)) * 8,
            "geometry_spline_line_out",
        );
        let state = stage_output_buffer(&self.device, 24, "geometry_spline_state");
        let z = -0.267949192431122706472553658494127633f64;
        let zn = z.powf((len - 1) as f64);
        for line in 0..lines {
            self.check_cancelled()?;
            for channel in 0..3u32 {
                self.check_cancelled()?;
                let at = |i: u32| -> u64 {
                    let pixel = if axis == 0 {
                        u64::from(i) * u64::from(width) + u64::from(line)
                    } else {
                        u64::from(line) * u64::from(width) + u64::from(i)
                    };
                    (pixel * 3 + u64::from(channel)) * 8
                };
                // Whole-line closed-form initialization, then carried forward
                // and reverse sweeps. Window boundaries never become endpoints.
                source.copy_to(encoder, at(0), &a, 0, 8);
                source.copy_to(encoder, at(len - 1), &b, 0, 8);
                let run =
                    |encoder: &mut ObservedEncoder, mode, first, count| -> Result<(), String> {
                        self.check_cancelled()?;
                        let uniform = self.geometry_uniform(
                            "geometry_spline_line_params",
                            &SplineLineParams {
                                len,
                                count,
                                mode,
                                first,
                                zn,
                            },
                        );
                        self.geometry_pass_groups(
                            encoder,
                            SPLINE_LINE_SHADER,
                            &[UNIFORM, READ, READ, WRITE, WRITE],
                            &[&uniform, &a, &b, &out, &state],
                            1,
                            "geometry_spline_line",
                        );
                        self.geometry_submit_pending(encoder)
                    };
                run(encoder, 0, 0, 1)?;
                let mut first = 1;
                while first < len - 1 {
                    self.check_cancelled()?;
                    let count = capacity.min(len - 1 - first);
                    for i in 0..count {
                        source.copy_to(encoder, at(first + i), &a, u64::from(i) * 8, 8);
                        source.copy_to(encoder, at(len - 1 - first - i), &b, u64::from(i) * 8, 8);
                    }
                    run(encoder, 1, first, count)?;
                    first += count;
                }
                let mut first = 0;
                while first < len {
                    self.check_cancelled()?;
                    let count = capacity.min(len - first);
                    for i in 0..count {
                        source.copy_to(encoder, at(first + i), &a, u64::from(i) * 8, 8);
                    }
                    run(encoder, 2, first, count)?;
                    for i in 0..count {
                        destination.copy_from(encoder, &out, u64::from(i) * 8, at(first + i), 8);
                    }
                    first += count;
                }
                destination.copy_to(encoder, at(len - 2), &a, 0, 8);
                destination.copy_to(encoder, at(len - 1), &b, 0, 8);
                run(encoder, 3, len - 1, 1)?;
                destination.copy_from(encoder, &out, 0, at(len - 1), 8);
                let mut end = len - 1;
                while end > 0 {
                    self.check_cancelled()?;
                    let count = capacity.min(end);
                    let first = end - count;
                    for i in 0..count {
                        destination.copy_to(encoder, at(first + i), &a, u64::from(i) * 8, 8);
                    }
                    run(encoder, 4, first, count)?;
                    for i in 0..count {
                        destination.copy_from(encoder, &out, u64::from(i) * 8, at(first + i), 8);
                    }
                    end = first;
                }
                self.geometry_submit_pending(encoder)?;
            }
        }
        Ok(())
    }

    fn geometry_uniform<T: bytemuck::Pod>(&self, label: &'static str, value: &T) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::bytes_of(value),
                usage: wgpu::BufferUsages::UNIFORM,
            })
    }

    /// One dispatch of a 256-wide kernel over `n_values` invocations.
    #[allow(clippy::too_many_arguments)]
    fn geometry_pass_linear(
        &self,
        encoder: &mut ObservedEncoder,
        shader: &'static str,
        bindings: &[wgpu::BufferBindingType],
        buffers: &[&wgpu::Buffer],
        n_values: u32,
        label: &'static str,
    ) {
        let (cached, bind_group) = self.geometry_bindings(shader, bindings, buffers, label);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&cached.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        dispatch_linear(&mut pass, n_values);
    }

    /// One dispatch of a single-invocation-per-group kernel.
    #[allow(clippy::too_many_arguments)]
    fn geometry_pass_groups(
        &self,
        encoder: &mut ObservedEncoder,
        shader: &'static str,
        bindings: &[wgpu::BufferBindingType],
        buffers: &[&wgpu::Buffer],
        groups: u32,
        label: &'static str,
    ) {
        let (cached, bind_group) = self.geometry_bindings(shader, bindings, buffers, label);
        let (x, y) = dispatch_grid(groups);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&cached.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(x, y, 1);
    }

    fn geometry_bindings(
        &self,
        shader: &'static str,
        bindings: &[wgpu::BufferBindingType],
        buffers: &[&wgpu::Buffer],
        label: &'static str,
    ) -> (super::super::cache::CachedPipelineRef, wgpu::BindGroup) {
        let cached = self.cached_pipeline(shader, bindings);
        let entries: Vec<wgpu::BindGroupEntry<'_>> = buffers
            .iter()
            .enumerate()
            .map(|(index, buffer)| wgpu::BindGroupEntry {
                binding: index as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &cached.layout,
            entries: &entries,
        });
        (cached, bind_group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::ComputeBackend;
    use spektrafilm_backend::InputResizeGpuParams;
    use spektrafilm_backend::device_image::{Precision, RetainedImageMeta, SampleDomain};
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{from_f32, from_f64, to_f64};
    use spektrafilm_math::resize::rescale_spline3;

    fn space() -> &'static spektrafilm_math::colorspace::RgbColorSpace {
        spektrafilm_math::colorspace::resolve("sRGB").unwrap()
    }

    fn upload(gpu: &WgpuBackend, image: &ImageBuf) -> DeviceImage {
        gpu.upload_retained(
            image,
            &RetainedImageMeta {
                width: image.width,
                height: image.height,
                domain: SampleDomain::Linear,
                precision: Precision::F32,
                space: space(),
                pixel_pitch_um: Some(12.0),
            },
        )
        .unwrap()
    }

    fn run(gpu: &WgpuBackend, image: &ImageBuf, params: &InputGeometryGpuParams) -> ImageBuf {
        let carrier = upload(gpu, image);
        let out = gpu
            .encode_input_geometry(&carrier, params)
            .unwrap()
            .expect("the wgpu backend has device geometry");
        gpu.materialize_buffer(&out).unwrap()
    }

    // Device tests already own DEVICE_LOCK through `adapter`, so the process
    // cap cannot race another GPU test. Restore it even when an assertion fails.
    fn run_bounded(
        gpu: &WgpuBackend,
        image: &ImageBuf,
        params: &InputGeometryGpuParams,
        limit: u64,
    ) -> ImageBuf {
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    match &self.0 {
                        Some(value) => {
                            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", value)
                        }
                        None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
                    }
                }
            }
        }
        let _restore = Restore(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", limit.to_string());
        }
        let carrier = upload(gpu, image);
        let input = gpu.buffer_payload(&carrier).unwrap();
        assert!(input.pages.len() > 1, "fixture must cross source pages");
        assert!(input.pages.iter().all(|page| page.buffer.size() <= limit));
        let out = gpu
            .encode_input_geometry(&carrier, params)
            .unwrap()
            .unwrap();
        let output = gpu.buffer_payload(&out).unwrap();
        assert!(output.pages.iter().all(|page| page.buffer.size() <= limit));
        assert_eq!(out.pixel_pitch_um(), Some(params.pixel_pitch_um));
        gpu.materialize_buffer(&out).unwrap()
    }

    fn spline(width: u32, height: u32) -> InputResizeGpuParams {
        InputResizeGpuParams {
            width,
            height,
            method: InputResizeMethod::Spline3,
        }
    }

    fn triangle_preview(width: u32, height: u32) -> InputResizeGpuParams {
        InputResizeGpuParams {
            width,
            height,
            method: InputResizeMethod::PreviewTriangle,
        }
    }

    /// Signed quarter-step samples in `[-1, 1.5]` so every value is exactly
    /// representable in f32: the comparison isolates the kernel arithmetic
    /// rather than the storage precision of the reference build.
    fn patterned(width: u32, height: u32) -> ImageBuf {
        let mut data = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                data.push(from_f64(((x * 13 + y * 7) % 11) as f64 / 4.0 - 1.0));
                data.push(from_f64(((x * 5 + y * 3) % 7) as f64 / 4.0));
                data.push(from_f64(((x + 2 * y) % 5) as f64 / 2.0 - 0.5));
            }
        }
        ImageBuf::from_data(width, height, data)
    }

    fn max_abs_diff(actual: &ImageBuf, expected: &ImageBuf) -> f64 {
        actual
            .data
            .iter()
            .zip(&expected.data)
            .map(|(a, b)| (to_f64(*a) - to_f64(*b)).abs())
            .fold(0.0, f64::max)
    }

    fn assert_within(actual: &ImageBuf, expected: &ImageBuf, budget: f64, what: &str) {
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height),
            "{what}: geometry"
        );
        let worst = max_abs_diff(actual, expected);
        assert!(worst <= budget, "{what}: max |Δ| = {worst} > {budget}");
    }

    /// Crop reference: the rotated frame cropped to the same rectangle by
    /// copying samples, exactly like `resizing::crop_image`.
    fn reference_crop(rotated: &ImageBuf, rect: [u32; 4]) -> ImageBuf {
        let [x0, y0, width, height] = rect;
        let mut data = Vec::with_capacity((width * height * 3) as usize);
        for y in y0..y0 + height {
            for x in x0..x0 + width {
                data.extend_from_slice(&rotated.get(x, y));
            }
        }
        ImageBuf::from_data(width, height, data)
    }

    /// The preview reference's own f32 size rule: factor from the long edge,
    /// truncated dimensions.
    fn preview_size(width: u32, height: u32, max_size: u32) -> (u32, u32) {
        let max_size = max_size as f32;
        let (w, h) = (width as f32, height as f32);
        if w.max(h) <= max_size {
            return (width, height);
        }
        let scale = max_size / w.max(h);
        (((w * scale) as u32).max(1), ((h * scale) as u32).max(1))
    }

    fn reference_preview(source: &ImageBuf, width: u32, height: u32) -> ImageBuf {
        use image::{ImageBuffer, Rgb};
        let samples: Vec<f32> = source.data.iter().map(|&v| to_f64(v) as f32).collect();
        let image =
            ImageBuffer::<Rgb<f32>, _>::from_raw(source.width, source.height, samples).unwrap();
        let out =
            image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle);
        ImageBuf::from_data(
            width,
            height,
            out.into_raw().into_iter().map(from_f32).collect(),
        )
    }

    #[test]
    fn rotation_and_crop_are_verbatim_reference_pixel_moves() {
        let gpu = crate::test_support::adapter();
        let source = patterned(5, 3);
        for turns in 0..4u32 {
            let rotated = source.rotated_quarter_turns(turns as i32);
            // Off-centre, odd-sized rectangle in the rotated frame.
            let rect = [1, 1, rotated.width - 1, rotated.height - 2];
            let params = InputGeometryGpuParams {
                rotation_quarter_turns: turns,
                crop: Some(rect),
                resize: None,
                pixel_pitch_um: 12.0,
            };
            let out = run(&gpu, &source, &params);
            let expected = reference_crop(&rotated, rect);
            assert_eq!(
                (out.width, out.height),
                (expected.width, expected.height),
                "turns {turns}"
            );
            assert_eq!(
                out.data, expected.data,
                "turns {turns}: samples must be copied"
            );
            for limit in [12, 25, 101] {
                let paged = run_bounded(&gpu, &source, &params, limit);
                assert_eq!(paged.data, expected.data, "turns {turns}, binding {limit}");
            }
        }
    }

    #[test]
    fn portrait_rotation_crops_the_rotated_frame() {
        let gpu = crate::test_support::adapter();
        let source = patterned(4, 9);
        let rotated = source.rotated_quarter_turns(1);
        // 4x9 becomes 9x4, so the crop's vertical extent is capped at four.
        let rect = [0, 1, 6, 3];
        let params = InputGeometryGpuParams {
            rotation_quarter_turns: 1,
            crop: Some(rect),
            resize: None,
            pixel_pitch_um: 12.0,
        };
        let out = run(&gpu, &source, &params);
        let expected = reference_crop(&rotated, rect);
        assert_eq!((out.width, out.height), (6, 3));
        assert_eq!(out.data, expected.data);
    }

    #[test]
    fn no_op_geometry_reuses_the_retained_source() {
        let gpu = crate::test_support::adapter();
        let source = patterned(6, 4);
        let carrier = upload(&gpu, &source);
        let out = gpu
            .encode_input_geometry(
                &carrier,
                &InputGeometryGpuParams {
                    rotation_quarter_turns: 0,
                    crop: None,
                    resize: None,
                    pixel_pitch_um: 12.0,
                },
            )
            .unwrap()
            .unwrap();
        assert!(
            std::sync::Arc::ptr_eq(carrier.payload().unwrap(), out.payload().unwrap()),
            "no-op geometry must not copy or reupload the retained source"
        );
        // Cancelled derived operations must not publish even an identity
        // carrier, and cancellation must not contaminate the source/backend.
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let cancelled = gpu.with_cancellation(Some(token.clone())).unwrap();
        let params = InputGeometryGpuParams {
            rotation_quarter_turns: 0,
            crop: None,
            resize: None,
            pixel_pitch_um: 12.0,
        };
        assert!(
            matches!(cancelled.input_geometry(&carrier, &params), Err(error) if error == "operation cancelled")
        );
        // The private submission helper needs the concrete backend. Its only
        // cancellation owner is the pipe; do not duplicate the backend field.
        let mut submission = gpu.derived_explicit();
        submission.pipe = submission.pipe.with_cancellation(Some(token.clone()));
        let mut encoder = submission
            .device
            .create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(
            &gpu.buffer_payload(&carrier).unwrap().pages[0].buffer,
            0,
            &stage_output_buffer(&submission.device, 12, "cancelled_geometry_destination"),
            0,
            12,
        );
        assert_eq!(
            submission
                .geometry_submit_pending(&mut encoder)
                .unwrap_err(),
            "operation cancelled"
        );
        assert_eq!(gpu.materialize_buffer(&carrier).unwrap().data, source.data);
        token.store(false, std::sync::atomic::Ordering::Relaxed);
        let reused = cancelled
            .input_geometry(&carrier, &params)
            .unwrap()
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(
            carrier.payload().unwrap(),
            reused.payload().unwrap()
        ));
    }

    #[test]
    fn spline_upscale_matches_the_reference_on_fractional_sizes() {
        let gpu = crate::test_support::adapter();
        let source = patterned(7, 5);
        let expected = rescale_spline3(&source, 1.5).unwrap();
        let params = InputGeometryGpuParams {
            rotation_quarter_turns: 0,
            crop: None,
            resize: Some(spline(expected.width, expected.height)),
            pixel_pitch_um: 12.0,
        };
        let out = run(&gpu, &source, &params);
        assert_within(&out, &expected, 1e-6, "spline upscale");
    }

    #[test]
    fn spline_downscale_antialiases_like_the_reference_and_clips_to_the_input_range() {
        let gpu = crate::test_support::adapter();
        // A single strong signed sample: the Gaussian antialias spreads it and
        // the readout must stay inside the source interval.
        let mut source = patterned(16, 16);
        source.set(7, 7, [from_f64(-40.0), from_f64(30.0), from_f64(-0.25)]);
        let expected = rescale_spline3(&source, 0.5).unwrap();
        let params = InputGeometryGpuParams {
            rotation_quarter_turns: 0,
            crop: None,
            resize: Some(spline(expected.width, expected.height)),
            pixel_pitch_um: 12.0,
        };
        let out = run(&gpu, &source, &params);
        assert_within(&out, &expected, 1e-6, "spline downscale");
        // Non-square crop and extrema at opposite physical edges exercise
        // global range carry, mirror support, and both whole-line recursions.
        let mut full = patterned(11, 9);
        full.set(1, 1, [from_f64(-40.0), from_f64(30.0), from_f64(-0.25)]);
        full.set(9, 7, [from_f64(50.0), from_f64(-20.0), from_f64(0.25)]);
        full.set(
            0,
            0,
            [from_f64(-5000.0), from_f64(5000.0), from_f64(-5000.0)],
        );
        let crop = [1, 1, 9, 7];
        let cropped = reference_crop(&full, crop);
        for scale in [0.5, 1.5] {
            let expected = rescale_spline3(&cropped, scale).unwrap();
            let params = InputGeometryGpuParams {
                rotation_quarter_turns: 0,
                crop: Some(crop),
                resize: Some(spline(expected.width, expected.height)),
                pixel_pitch_um: 7.25,
            };
            // 41-byte cap makes 40-byte f64 pages: RGB triples cross pages.
            for limit in [24, 41, 101] {
                let bounded = run_bounded(&gpu, &full, &params, limit);
                assert_within(
                    &bounded,
                    &expected,
                    1e-6,
                    &format!("bounded cropped spline {scale}, {limit}"),
                );
            }
        }
        for channel in 0..3 {
            let (low, high) = source
                .extract_channel(channel)
                .into_iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), v| {
                    (low.min(to_f64(v)), high.max(to_f64(v)))
                });
            for value in out.extract_channel(channel) {
                let value = to_f64(value);
                assert!(
                    value >= low && value <= high,
                    "channel {channel} sample {value} outside the input interval [{low}, {high}]"
                );
            }
        }
    }

    #[test]
    fn spline_singleton_axes_match_the_reference() {
        let gpu = crate::test_support::adapter();
        for (width, height, scale) in [(1u32, 6u32, 2.0f64), (6, 1, 2.0), (1, 5, 0.5), (5, 1, 0.5)]
        {
            let source = patterned(width, height);
            let expected = rescale_spline3(&source, scale).unwrap();
            let params = InputGeometryGpuParams {
                rotation_quarter_turns: 0,
                crop: None,
                resize: Some(spline(expected.width, expected.height)),
                pixel_pitch_um: 12.0,
            };
            let out = run(&gpu, &source, &params);
            assert_within(
                &out,
                &expected,
                1e-6,
                &format!("spline {width}x{height} at {scale}"),
            );
            let bounded = run_bounded(&gpu, &source, &params, 24);
            assert_within(
                &bounded,
                &expected,
                1e-6,
                &format!("bounded singleton spline {width}x{height}"),
            );
        }
    }

    #[test]
    fn preview_triangle_matches_the_reference_resampler() {
        let gpu = crate::test_support::adapter();
        for (width, height, max_size) in [(13u32, 7u32, 8u32), (7, 13, 6), (9, 9, 4)] {
            let source = patterned(width, height);
            let (out_width, out_height) = preview_size(width, height, max_size);
            let expected = reference_preview(&source, out_width, out_height);
            let params = InputGeometryGpuParams {
                rotation_quarter_turns: 0,
                crop: None,
                resize: Some(triangle_preview(out_width, out_height)),
                pixel_pitch_um: 12.0,
            };
            let out = run(&gpu, &source, &params);
            assert_within(
                &out,
                &expected,
                1e-6,
                &format!("preview {width}x{height} to {out_width}x{out_height}"),
            );
            for limit in [12, 101] {
                let bounded = run_bounded(&gpu, &source, &params, limit);
                assert_within(
                    &bounded,
                    &expected,
                    1e-6,
                    &format!("bounded preview {width}x{height}, {limit}"),
                );
            }
        }
    }

    #[test]
    fn preview_upscale_matches_the_reference_resampler() {
        let gpu = crate::test_support::adapter();
        let source = patterned(5, 4);
        let expected = reference_preview(&source, 15, 12);
        let params = InputGeometryGpuParams {
            rotation_quarter_turns: 0,
            crop: None,
            resize: Some(triangle_preview(15, 12)),
            pixel_pitch_um: 12.0,
        };
        let out = run(&gpu, &source, &params);
        assert_within(&out, &expected, 1e-6, "preview upscale");
        for limit in [12, 101] {
            let bounded = run_bounded(&gpu, &source, &params, limit);
            assert_within(
                &bounded,
                &expected,
                1e-6,
                &format!("bounded preview upscale {limit}"),
            );
        }
    }
}
