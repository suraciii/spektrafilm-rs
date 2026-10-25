use super::super::super::*;
use spektrafilm_math::precision::{Scalar, from_f64};
use std::num::NonZeroU64;

#[path = "blur_bounded.rs"]
mod bounded;

const FIR_LAYOUT: [wgpu::BufferBindingType; 4] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const IMAGE_LAYOUT: [wgpu::BufferBindingType; 3] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const FIR_WORKGROUP_SIZE: u32 = 256;
const IIR_WORKGROUP_SIZE: u32 = 64;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct FirParams {
    width: u32,
    height: u32,
    radius: u32,
    channel: u32,
    vertical: u32,
    grid_width: u32,
    source_base: u32,
    destination_base: u32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct IirParams {
    width: u32,
    height: u32,
    channel: u32,
    vertical: u32,
    a: f32,
    b1: f32,
    b2: f32,
    b3: f32,
    grid_width: u32,
    source_base: u32,
    destination_base: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CopyParams {
    width: u32,
    height: u32,
    channel: u32,
    grid_width: u32,
    source_base: u32,
    destination_base: u32,
    _pad: [u32; 2],
}

#[derive(Copy, Clone)]
enum Method {
    Fir,
    Iir,
    Copy,
}

/// One storage binding over an image-sized sample region. Device storage
/// bindings must start on a `min_storage_buffer_offset_alignment` boundary, so
/// the binding may begin before the requested region; `base` is that residual
/// in f32 samples and the shaders add it to every index.
#[derive(Copy, Clone)]
struct BindingRange {
    offset: u64,
    size: u64,
    base: u32,
}

struct Region {
    width: u32,
    height: u32,
    source: BindingRange,
    destination: BindingRange,
    scratch: BindingRange,
}

struct Pass {
    _params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    method: Method,
    workgroups: (u32, u32),
}

struct ChannelPasses {
    first: Pass,
    second: Option<Pass>,
    _kernel: Option<wgpu::Buffer>,
}

/// Reference Gaussian blur over resident, interleaved f32 RGB samples.
///
/// `sigma[c] < 3` uses the reference reflect-boundary FIR (vertical then
/// horizontal); `sigma[c] >= 3` uses the sample-replicating Young-van Vliet
/// recursion (horizontal then vertical); `sigma[c] <= 0` copies the channel
/// unchanged, as the reference does. All three channels are filtered with
/// their own sigma and pass order.
///
/// `source`, `destination`, and `scratch` MUST be distinct buffers. Small jobs
/// bind aligned image regions; larger jobs gather/scatter raw target-channel
/// bits with bounded GPU kernels. COPY-only or oversized physical pages use
/// bounded bulk-copy adapters. Source requires COPY_SRC, destination requires
/// COPY_SRC | COPY_DST. Offsets count float samples, without alignment rules.
///
/// Arithmetic is binary32. A `precision-f64` host build evaluates its
/// reference on f64 samples, so this port serves the native-f32 reference
/// domain and is not a bit-compatible f64 oracle.
pub(in crate::wgpu_backend) struct GrainBlurState {
    execution: Execution,
}

// Both variants retain device handles for the job lifetime. Keep them inline:
// boxing Direct would add a host allocation to every direct blur just to shrink
// the bounded variant's stack representation.
#[allow(clippy::large_enum_variant)]
enum Execution {
    Direct(DirectBlurState),
    Bounded(bounded::BoundedBlur),
}

struct DirectBlurState {
    fir: CachedPipelineRef,
    iir: CachedPipelineRef,
    copy: CachedPipelineRef,
    channels: [ChannelPasses; 3],
    _owned_scratch: Option<wgpu::Buffer>,
}

/// Build a Gaussian blur with job-owned intermediate storage.
#[allow(clippy::too_many_arguments)]
pub(in crate::wgpu_backend) fn build_grain_blur(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    source: &wgpu::Buffer,
    destination: &wgpu::Buffer,
    sigma: [f64; 3],
    source_offset: u32,
    destination_offset: u32,
) -> Result<GrainBlurState, String> {
    let bytes = region_bytes(width, height)?;
    if needs_bounded(backend, source_offset, destination_offset, bytes)?
        || !source.usage().contains(wgpu::BufferUsages::STORAGE)
        || !destination.usage().contains(wgpu::BufferUsages::STORAGE)
    {
        return build_bounded(
            backend,
            width,
            height,
            source,
            destination,
            None,
            sigma,
            source_offset,
            destination_offset,
        );
    }
    let scratch = backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("grain_gaussian_scratch"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let mut state = build_grain_blur_with_scratch(
        backend,
        width,
        height,
        source,
        destination,
        &scratch,
        sigma,
        source_offset,
        destination_offset,
    )?;
    if let Execution::Direct(direct) = &mut state.execution {
        direct._owned_scratch = Some(scratch);
    }
    Ok(state)
}

/// Build a Gaussian blur using caller-owned scratch for sequential filter jobs.
///
/// Direct execution uses width*height*3 scratch samples. Bounded execution
/// reuses full-size COPY_SRC | COPY_DST scratch as its paged intermediate,
/// or allocates one if unavailable. STORAGE usage permits direct capped views;
/// COPY-only scratch uses the same bounded bulk adapter as external images.
/// Keep supplied buffers alive until encoded work completes. Offsets count f32.
#[allow(clippy::too_many_arguments)]
pub(in crate::wgpu_backend) fn build_grain_blur_with_scratch(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    source: &wgpu::Buffer,
    destination: &wgpu::Buffer,
    scratch: &wgpu::Buffer,
    sigma: [f64; 3],
    source_offset: u32,
    destination_offset: u32,
) -> Result<GrainBlurState, String> {
    let bytes = region_bytes(width, height)?;
    if needs_bounded(backend, source_offset, destination_offset, bytes)?
        || !source.usage().contains(wgpu::BufferUsages::STORAGE)
        || !destination.usage().contains(wgpu::BufferUsages::STORAGE)
        || !scratch.usage().contains(wgpu::BufferUsages::STORAGE)
    {
        return build_bounded(
            backend,
            width,
            height,
            source,
            destination,
            Some(scratch),
            sigma,
            source_offset,
            destination_offset,
        );
    }
    let region = validate_resources(
        backend,
        width,
        height,
        source,
        destination,
        scratch,
        source_offset,
        destination_offset,
    )?;
    if sigma.iter().any(|value| !value.is_finite()) {
        return Err("Gaussian blur sigma values must be finite".into());
    }
    let max_groups = backend.device.limits().max_compute_workgroups_per_dimension;
    let fir = backend.cached_pipeline(include_str!("blur_fir.wgsl"), &FIR_LAYOUT);
    let iir = backend.cached_pipeline(include_str!("blur_iir.wgsl"), &IMAGE_LAYOUT);
    let copy = backend.cached_pipeline(include_str!("blur_copy.wgsl"), &IMAGE_LAYOUT);
    let channels = [
        build_channel(
            backend,
            &region,
            source,
            destination,
            scratch,
            0,
            sigma[0],
            &fir,
            &iir,
            &copy,
            max_groups,
        )?,
        build_channel(
            backend,
            &region,
            source,
            destination,
            scratch,
            1,
            sigma[1],
            &fir,
            &iir,
            &copy,
            max_groups,
        )?,
        build_channel(
            backend,
            &region,
            source,
            destination,
            scratch,
            2,
            sigma[2],
            &fir,
            &iir,
            &copy,
            max_groups,
        )?,
    ];
    Ok(GrainBlurState {
        execution: Execution::Direct(DirectBlurState {
            fir,
            iir,
            copy,
            channels,
            _owned_scratch: None,
        }),
    })
}

/// Exact reference blur over RGB pages, including layer-stack sample offsets.
/// No physical image-sized buffer is allocated or bound. Source pages require
/// COPY_SRC; destination pages require COPY_SRC | COPY_DST.
#[allow(clippy::too_many_arguments)]
pub(in crate::wgpu_backend) fn build_grain_blur_paged(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    source: &[crate::wgpu_backend::paged::ImagePage],
    destination: &[crate::wgpu_backend::paged::ImagePage],
    sigma: [f64; 3],
    source_sample: u64,
    destination_sample: u64,
) -> Result<GrainBlurState, String> {
    build_grain_blur_paged_with_scratch(
        backend,
        width,
        height,
        source,
        destination,
        None,
        sigma,
        source_sample,
        destination_sample,
    )
}

/// Reuse paged full-image scratch across sequential jobs (for example halation
/// bounces). Scratch must be distinct and provide COPY_SRC | COPY_DST.
#[allow(clippy::too_many_arguments)]
pub(in crate::wgpu_backend) fn build_grain_blur_paged_with_scratch(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    source: &[crate::wgpu_backend::paged::ImagePage],
    destination: &[crate::wgpu_backend::paged::ImagePage],
    scratch: Option<&[crate::wgpu_backend::paged::ImagePage]>,
    sigma: [f64; 3],
    source_sample: u64,
    destination_sample: u64,
) -> Result<GrainBlurState, String> {
    Ok(GrainBlurState {
        execution: Execution::Bounded(bounded::BoundedBlur::build_paged(
            backend,
            width,
            height,
            source,
            destination,
            scratch,
            sigma,
            source_sample,
            destination_sample,
            super::super::super::bounded::storage_binding_limit(backend)?,
        )?),
    })
}

/// Bytes of one image-sized RGB sample region, validated against the shader's
/// u32 index range and the i32 reflection arithmetic.
fn region_bytes(width: u32, height: u32) -> Result<u64, String> {
    if width == 0 || height == 0 {
        return Err("Gaussian blur dimensions must be nonzero".into());
    }
    if width > i32::MAX as u32 / 2 || height > i32::MAX as u32 / 2 {
        return Err("Gaussian blur dimensions exceed the reflection coordinate range".into());
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| "Gaussian blur pixel count overflows".to_owned())?;
    if pixels > u64::from(u32::MAX) / 3 {
        return Err("Gaussian blur samples exceed the WGSL u32 index range".into());
    }
    pixels
        .checked_mul(12)
        .ok_or_else(|| "Gaussian blur resource size overflows".to_owned())
}

fn check_region_limit(backend: &WgpuBackend, bytes: u64) -> Result<(), String> {
    let limit = super::super::super::bounded::storage_binding_limit(backend)?;
    if bytes > limit {
        return Err(format!(
            "Gaussian blur binding requires {bytes} bytes; limit is {limit}"
        ));
    }
    Ok(())
}

fn needs_bounded(
    backend: &WgpuBackend,
    source: u32,
    destination: u32,
    bytes: u64,
) -> Result<bool, String> {
    let limit = super::super::super::bounded::storage_binding_limit(backend)?;
    let alignment = storage_alignment(backend)?;
    Ok([source, destination]
        .iter()
        .any(|offset| bytes + u64::from(*offset) * 4 % alignment > limit))
}

#[allow(clippy::too_many_arguments)]
fn build_bounded(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    source: &wgpu::Buffer,
    destination: &wgpu::Buffer,
    scratch: Option<&wgpu::Buffer>,
    sigma: [f64; 3],
    source_offset: u32,
    destination_offset: u32,
) -> Result<GrainBlurState, String> {
    Ok(GrainBlurState {
        execution: Execution::Bounded(bounded::BoundedBlur::build(
            backend,
            width,
            height,
            source,
            destination,
            scratch,
            sigma,
            source_offset,
            destination_offset,
            super::super::super::bounded::storage_binding_limit(backend)?,
        )?),
    })
}

fn storage_alignment(backend: &WgpuBackend) -> Result<u64, String> {
    let alignment = u64::from(backend.device.limits().min_storage_buffer_offset_alignment);
    let alignment = alignment.max(4);
    if alignment % 4 != 0 {
        return Err(format!(
            "Gaussian blur requires a sample-addressable storage alignment; device reports {alignment}"
        ));
    }
    Ok(alignment)
}

/// Binding for `bytes` starting at `offset_samples` in `buffer`. The binding
/// begins at the greatest aligned offset not after the request, and `base`
/// carries the residual so the shader still addresses the requested samples.
fn binding_range(
    backend: &WgpuBackend,
    name: &str,
    buffer: &wgpu::Buffer,
    offset_samples: u32,
    bytes: u64,
) -> Result<BindingRange, String> {
    if !buffer.usage().contains(wgpu::BufferUsages::STORAGE) {
        return Err(format!("Gaussian blur {name} buffer lacks STORAGE usage"));
    }
    let alignment = storage_alignment(backend)?;
    let requested = u64::from(offset_samples)
        .checked_mul(4)
        .ok_or_else(|| format!("Gaussian blur {name} offset overflows"))?;
    if requested > buffer.size() {
        return Err(format!(
            "Gaussian blur {name} offset {requested} bytes exceeds buffer size {}",
            buffer.size()
        ));
    }
    let offset = requested - requested % alignment;
    let residual = requested - offset;
    let size = residual
        .checked_add(bytes)
        .ok_or_else(|| format!("Gaussian blur {name} binding size overflows"))?;
    if offset
        .checked_add(size)
        .is_none_or(|end| end > buffer.size())
    {
        return Err(format!(
            "Gaussian blur {name} binding offset {offset} with {size} bytes exceeds buffer size {}",
            buffer.size()
        ));
    }
    let limit = super::super::super::bounded::storage_binding_limit(backend)?;
    if size > limit {
        return Err(format!(
            "Gaussian blur {name} binding of {size} bytes exceeds device storage binding limit {limit}"
        ));
    }
    Ok(BindingRange {
        offset,
        size,
        base: u32::try_from(residual / 4)
            .map_err(|_| format!("Gaussian blur {name} binding residual exceeds u32 samples"))?,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_resources(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    source: &wgpu::Buffer,
    destination: &wgpu::Buffer,
    scratch: &wgpu::Buffer,
    source_offset: u32,
    destination_offset: u32,
) -> Result<Region, String> {
    let bytes = region_bytes(width, height)?;
    let source_range = binding_range(backend, "source", source, source_offset, bytes)?;
    let destination_range = binding_range(
        backend,
        "destination",
        destination,
        destination_offset,
        bytes,
    )?;
    let scratch_range = binding_range(backend, "scratch", scratch, 0, bytes)?;
    Ok(Region {
        width,
        height,
        source: source_range,
        destination: destination_range,
        scratch: scratch_range,
    })
}

fn bounded_grid(workgroups: u64, limit: u32) -> Result<(u32, u32), String> {
    if workgroups == 0 || limit == 0 {
        return Err("Gaussian blur has an empty dispatch or no device workgroups".into());
    }
    let x = workgroups.min(u64::from(limit));
    let y = workgroups.div_ceil(x);
    if y > u64::from(limit) {
        return Err("Gaussian blur dispatch exceeds device workgroup limits".into());
    }
    Ok((x as u32, y as u32))
}

#[allow(clippy::too_many_arguments)]
fn build_channel(
    backend: &WgpuBackend,
    region: &Region,
    source: &wgpu::Buffer,
    destination: &wgpu::Buffer,
    scratch: &wgpu::Buffer,
    channel: u32,
    sigma_f64: f64,
    fir: &CachedPipelineRef,
    iir: &CachedPipelineRef,
    copy: &CachedPipelineRef,
    max_groups: u32,
) -> Result<ChannelPasses, String> {
    let pixels = u64::from(region.width) * u64::from(region.height);
    let sigma: Scalar = from_f64(sigma_f64);
    if sigma_f64 <= 0.0 {
        let grid = bounded_grid(pixels.div_ceil(u64::from(FIR_WORKGROUP_SIZE)), max_groups)?;
        return Ok(ChannelPasses {
            first: make_copy_pass(backend, copy, region, channel, grid, source, destination),
            second: None,
            _kernel: None,
        });
    }
    if sigma < from_f64(3.0) {
        let (radius, weights) = gaussian_kernel(sigma_f64)?;
        let kernel_bytes = u64::try_from(weights.len())
            .ok()
            .and_then(|count| count.checked_mul(4))
            .ok_or_else(|| "Gaussian FIR kernel size overflows".to_owned())?;
        check_region_limit(backend, kernel_bytes)?;
        let kernel = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grain_gaussian_kernel"),
                contents: bytemuck::cast_slice(&weights),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let grid = bounded_grid(pixels.div_ceil(u64::from(FIR_WORKGROUP_SIZE)), max_groups)?;
        let vertical = make_fir_pass(
            backend,
            fir,
            region,
            radius,
            channel,
            true,
            grid,
            source,
            &region.source,
            &kernel,
            (scratch, &region.scratch),
        );
        let horizontal = make_fir_pass(
            backend,
            fir,
            region,
            radius,
            channel,
            false,
            grid,
            scratch,
            &region.scratch,
            &kernel,
            (destination, &region.destination),
        );
        Ok(ChannelPasses {
            first: vertical,
            second: Some(horizontal),
            _kernel: Some(kernel),
        })
    } else {
        let coefficients = iir_coefficients(sigma_f64);
        if ![
            coefficients.0,
            coefficients.1,
            coefficients.2,
            coefficients.3,
        ]
        .iter()
        .all(|value| value.is_finite())
        {
            return Err("Gaussian IIR coefficients exceed the f32 device range".into());
        }
        let horizontal_grid = bounded_grid(
            u64::from(region.height).div_ceil(u64::from(IIR_WORKGROUP_SIZE)),
            max_groups,
        )?;
        let vertical_grid = bounded_grid(
            u64::from(region.width).div_ceil(u64::from(IIR_WORKGROUP_SIZE)),
            max_groups,
        )?;
        let horizontal = make_iir_pass(
            backend,
            iir,
            region,
            channel,
            false,
            coefficients,
            horizontal_grid,
            source,
            &region.source,
            (scratch, &region.scratch),
        );
        let vertical = make_iir_pass(
            backend,
            iir,
            region,
            channel,
            true,
            coefficients,
            vertical_grid,
            scratch,
            &region.scratch,
            (destination, &region.destination),
        );
        Ok(ChannelPasses {
            first: horizontal,
            second: Some(vertical),
            _kernel: None,
        })
    }
}

fn gaussian_kernel(sigma_f64: f64) -> Result<(u32, Vec<f32>), String> {
    let sigma: Scalar = from_f64(sigma_f64);
    let radius_value = from_f64(3.0) * sigma + from_f64(0.5);
    if !radius_value.is_finite() || radius_value < from_f64(0.0) {
        return Err("Gaussian FIR radius is not representable".into());
    }
    let radius = radius_value as usize;
    let size = radius
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| "Gaussian FIR kernel size overflows".to_owned())?;
    let mut weights = Vec::with_capacity(size);
    let half = from_f64(-0.5);
    for index in 0..size {
        let x = from_f64(index as f64) - from_f64(radius as f64);
        let scaled = x / sigma;
        weights.push((half * (scaled * scaled)).exp());
    }
    let sum: Scalar = weights.iter().copied().sum();
    if !sum.is_finite() || sum == from_f64(0.0) {
        return Err("Gaussian FIR kernel normalization is not finite".into());
    }
    for weight in &mut weights {
        *weight /= sum;
    }
    Ok((
        u32::try_from(radius).map_err(|_| "Gaussian FIR radius exceeds u32".to_owned())?,
        weights.into_iter().map(|weight| weight as f32).collect(),
    ))
}

fn iir_coefficients(sigma_f64: f64) -> (f32, f32, f32, f32) {
    let sigma: Scalar = from_f64(sigma_f64);
    let q = if sigma >= from_f64(2.5) {
        from_f64(0.98711) * sigma - from_f64(0.96330)
    } else {
        from_f64(3.97156) - from_f64(4.14554) * (from_f64(1.0) - from_f64(0.26891) * sigma).sqrt()
    };
    let q2 = q * q;
    let q3 = q2 * q;
    let b0 =
        from_f64(1.57825) + from_f64(2.44413) * q + from_f64(1.4281) * q2 + from_f64(0.422205) * q3;
    let b1_raw = from_f64(2.44413) * q + from_f64(2.85619) * q2 + from_f64(1.26661) * q3;
    let b2_raw = -(from_f64(1.4281) * q2 + from_f64(1.26661) * q3);
    let b3_raw = from_f64(0.422205) * q3;
    let a = from_f64(1.0) - (b1_raw + b2_raw + b3_raw) / b0;
    (
        a as f32,
        (b1_raw / b0) as f32,
        (b2_raw / b0) as f32,
        (b3_raw / b0) as f32,
    )
}

fn bind<'a>(buffer: &'a wgpu::Buffer, range: &BindingRange) -> wgpu::BindingResource<'a> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset: range.offset,
        size: NonZeroU64::new(range.size),
    })
}

#[allow(clippy::too_many_arguments)]
fn make_fir_pass(
    backend: &WgpuBackend,
    pipeline: &CachedPipelineRef,
    region: &Region,
    radius: u32,
    channel: u32,
    vertical: bool,
    grid: (u32, u32),
    source: &wgpu::Buffer,
    source_range: &BindingRange,
    kernel: &wgpu::Buffer,
    destination: (&wgpu::Buffer, &BindingRange),
) -> Pass {
    let (destination_buffer, destination_range) = destination;
    let params = backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_gaussian_fir_params"),
            contents: bytemuck::bytes_of(&FirParams {
                width: region.width,
                height: region.height,
                radius,
                channel,
                vertical: u32::from(vertical),
                grid_width: grid.0,
                source_base: source_range.base,
                destination_base: destination_range.base,
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let bind_group = backend
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grain_gaussian_fir_bind_group"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: bind(source, source_range),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: kernel.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: bind(destination_buffer, destination_range),
                },
            ],
        });
    Pass {
        _params: params,
        bind_group,
        method: Method::Fir,
        workgroups: grid,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_iir_pass(
    backend: &WgpuBackend,
    pipeline: &CachedPipelineRef,
    region: &Region,
    channel: u32,
    vertical: bool,
    coefficients: (f32, f32, f32, f32),
    grid: (u32, u32),
    source: &wgpu::Buffer,
    source_range: &BindingRange,
    destination: (&wgpu::Buffer, &BindingRange),
) -> Pass {
    let (destination_buffer, destination_range) = destination;
    let params = backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_gaussian_iir_params"),
            contents: bytemuck::bytes_of(&IirParams {
                width: region.width,
                height: region.height,
                channel,
                vertical: u32::from(vertical),
                a: coefficients.0,
                b1: coefficients.1,
                b2: coefficients.2,
                b3: coefficients.3,
                grid_width: grid.0,
                source_base: source_range.base,
                destination_base: destination_range.base,
                _pad: 0,
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let bind_group = backend
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grain_gaussian_iir_bind_group"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: bind(source, source_range),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: bind(destination_buffer, destination_range),
                },
            ],
        });
    Pass {
        _params: params,
        bind_group,
        method: Method::Iir,
        workgroups: grid,
    }
}

fn make_copy_pass(
    backend: &WgpuBackend,
    pipeline: &CachedPipelineRef,
    region: &Region,
    channel: u32,
    grid: (u32, u32),
    source: &wgpu::Buffer,
    destination: &wgpu::Buffer,
) -> Pass {
    let params = backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_gaussian_copy_params"),
            contents: bytemuck::bytes_of(&CopyParams {
                width: region.width,
                height: region.height,
                channel,
                grid_width: grid.0,
                source_base: region.source.base,
                destination_base: region.destination.base,
                _pad: [0; 2],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let bind_group = backend
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grain_gaussian_copy_bind_group"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: bind(source, &region.source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: bind(destination, &region.destination),
                },
            ],
        });
    Pass {
        _params: params,
        bind_group,
        method: Method::Copy,
        workgroups: grid,
    }
}

impl GrainBlurState {
    /// Submit bounded gather/filter/scatter pages in queue order, observing
    /// cancellation between submissions without reading image intermediates.
    pub(in crate::wgpu_backend) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        match &self.execution {
            Execution::Bounded(bounded) => bounded.submit(backend),
            Execution::Direct(direct) => {
                for channel in &direct.channels {
                    for pass in std::iter::once(&channel.first).chain(channel.second.iter()) {
                        backend.check_cancelled()?;
                        let mut encoder =
                            backend.device.create_command_encoder(&Default::default());
                        direct.encode_pass(&mut encoder, pass);
                        backend.check_cancelled()?;
                        backend.queue.submit(Some(encoder.finish()));
                    }
                }
                backend.check_cancelled()
            }
        }
    }
}

impl DirectBlurState {
    fn encode_pass(&self, encoder: &mut ObservedEncoder, pass: &Pass) {
        let (pipeline, label) = match pass.method {
            Method::Fir => (&self.fir, "grain_blur_fir"),
            Method::Iir => (&self.iir, "grain_blur_iir"),
            Method::Copy => (&self.copy, "grain_blur_copy"),
        };
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        compute.set_pipeline(&pipeline.pipeline);
        compute.set_bind_group(0, &pass.bind_group, &[]);
        compute.dispatch_workgroups(pass.workgroups.0, pass.workgroups.1, 1);
    }
}

// The device executes binary32 while `precision-f64` switches the reference to
// f64 samples and coefficients, so these comparisons belong to the native-f32
// reference domain (see the module docs). The f64 boundary is validated
// separately.
#[cfg(all(test, not(feature = "precision-f64")))]
mod tests {
    use super::*;
    use spektrafilm_math::gaussian::gaussian_blur_channel;
    use spektrafilm_math::precision::to_f64;

    const TOLERANCE: f64 = 2e-5;

    /// Run the device blur over a synthetic interleaved image and compare every
    /// channel with `spektrafilm_math::gaussian::gaussian_blur_channel`, which
    /// owns the reference FIR/IIR laws. Offsets place the image inside larger
    /// buffers so sample-unit addressing and untouched neighbours are exercised.
    fn compare_with_reference(
        gpu: &crate::test_support::DeviceBackend,
        width: u32,
        height: u32,
        sigma: [f64; 3],
        source_offset: u32,
        destination_offset: u32,
    ) {
        let pixels = (width as usize) * (height as usize);
        let samples = pixels * 3;
        let source_len = samples + source_offset as usize + 4;
        let destination_len = samples + destination_offset as usize + 4;
        let source: Vec<f32> = (0..source_len)
            .map(|index| ((index * 37 + 11) % 97) as f32 / 97.0)
            .collect();
        let destination: Vec<f32> = vec![123.0; destination_len];
        let source_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grain_blur_test_source"),
                contents: bytemuck::cast_slice(&source),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });
        let destination_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grain_blur_test_destination"),
                contents: bytemuck::cast_slice(&destination),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });
        let state = build_grain_blur(
            gpu,
            width,
            height,
            &source_buffer,
            &destination_buffer,
            sigma,
            source_offset,
            destination_offset,
        )
        .expect("Gaussian blur state");
        state.submit(gpu).expect("Gaussian blur submission");
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let bytes = destination_len as u64 * 4;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grain_blur_test_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&destination_buffer, 0, &readback, 0, bytes);
        gpu.queue.submit(Some(encoder.finish()));
        let actual = gpu.read_mapped_f32(&readback, bytes);

        for (channel, &channel_sigma) in sigma.iter().enumerate() {
            let reference_input: Vec<Scalar> = (0..pixels)
                .map(|pixel| {
                    from_f64(f64::from(
                        source[source_offset as usize + pixel * 3 + channel],
                    ))
                })
                .collect();
            let expected = gaussian_blur_channel(&reference_input, width, height, channel_sigma);
            for (pixel, &reference) in expected.iter().enumerate() {
                let index = destination_offset as usize + pixel * 3 + channel;
                let want = to_f64(reference);
                let got = f64::from(actual[index]);
                assert!(
                    (got - want).abs() < TOLERANCE,
                    "sigma {sigma:?} channel {channel} pixel {pixel} of {width}x{height}: device {got} != reference {want}"
                );
            }
        }
        assert!(
            actual[..destination_offset as usize]
                .iter()
                .all(|&value| value == 123.0),
            "destination prefix samples were overwritten"
        );
        assert!(
            actual[destination_offset as usize + samples..]
                .iter()
                .all(|&value| value == 123.0),
            "destination suffix samples were overwritten"
        );
    }

    #[test]
    fn fir_matches_reference_across_support_thresholds() {
        let gpu = crate::test_support::adapter();
        // Radius floor(3σ+0.5) is 1, 1, and 9; the last kernel is wider than
        // the image height and must reflect repeatedly.
        compare_with_reference(&gpu, 11, 7, [0.1667, 0.4, 2.99], 5, 7);
    }

    #[test]
    fn iir_matches_reference_from_dispatch_threshold() {
        let gpu = crate::test_support::adapter();
        // σ exactly 3.0 takes the reference IIR branch; large σ has no radius
        // cap and must not be truncated.
        compare_with_reference(&gpu, 11, 7, [3.0, 8.0, 20.0], 0, 3);
    }

    #[test]
    fn singleton_geometry_matches_reference() {
        let gpu = crate::test_support::adapter();
        compare_with_reference(&gpu, 1, 1, [0.4, 3.0, 9.0], 2, 1);
        compare_with_reference(&gpu, 1, 9, [0.4, 3.0, 9.0], 0, 0);
        compare_with_reference(&gpu, 9, 1, [0.4, 3.0, 9.0], 3, 2);
    }

    #[test]
    fn mixed_sigma_channels_keep_their_own_order_and_identity() {
        let gpu = crate::test_support::adapter();
        // Channel 0 is FIR, channel 1 is IIR, channel 2 is the zero-σ copy.
        // The reference applies the same per-channel dispatch.
        compare_with_reference(&gpu, 9, 5, [0.4, 3.5, 0.0], 3, 9);
    }
}
