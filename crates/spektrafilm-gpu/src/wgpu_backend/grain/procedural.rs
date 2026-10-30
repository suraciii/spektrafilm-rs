use super::raster::{RasterCommands, paged_transport, transport, uniform};
use super::*;

/// The transport geometry is always the original film geometry. Storage
/// windows only transport samples and never redefine film coordinates.
pub(in crate::wgpu_backend) struct ProceduralGrainState {
    output: Vec<super::super::paged::ImagePage>,
    commands: RasterCommands,
    pub(super) n_bytes: u64,
}

pub(in crate::wgpu_backend) fn build_grain_procedural_state(
    _device: &ObservedDevice,
    params: &spektrafilm_backend::ProceduralGrainGpuParams,
    width: u32,
    height: u32,
    input: &(impl RasterStorage + ?Sized),
    output_space: Option<&spektrafilm_math::colorspace::RgbColorSpace>,
    backend: &WgpuBackend,
) -> Result<ProceduralGrainState, String> {
    build_bounded_grain_procedural(
        params,
        width,
        height,
        input,
        output_space,
        backend,
        super::super::bounded::storage_binding_limit(backend)?,
    )
}

fn build_bounded_grain_procedural(
    params: &spektrafilm_backend::ProceduralGrainGpuParams,
    width: u32,
    height: u32,
    input: &(impl RasterStorage + ?Sized),
    output_space: Option<&spektrafilm_math::colorspace::RgbColorSpace>,
    backend: &WgpuBackend,
    limit: u64,
) -> Result<ProceduralGrainState, String> {
    let n = width
        .checked_mul(height)
        .filter(|&n| n > 0 && n <= u32::MAX / 3)
        .ok_or("procedural grain dimensions overflow")?;
    validate_coordinate_scale(params)?;
    // A caller may offer a budget larger than the device can bind; the band
    // plan must respect the device either way.
    let limit = limit.min(super::super::bounded::storage_binding_limit(backend)?);
    let n_bytes = u64::from(n) * 12;
    let page = (limit / 12).min(u64::from(n)) as u32;
    let original = paged_transport(backend, n_bytes, "grain_encoded_transport")?;
    let horizontal = paged_transport(backend, n_bytes, "grain_horizontal_transport")?;
    let filtered = paged_transport(backend, n_bytes, "grain_filtered_transport")?;
    let output = paged_transport(backend, n_bytes, "grain_output_transport")?;
    let source_window = transport(backend, u64::from(page) * 12, "grain_source_window")?;
    let target_window = transport(backend, u64::from(page) * 12, "grain_target_window")?;
    let working = transport(backend, u64::from(page) * 12, "grain_working_window")?;
    let grain_output = transport(backend, u64::from(page) * 12, "grain_composed_window")?;
    let storage = wgpu::BufferBindingType::Storage { read_only: true };
    let writable = wgpu::BufferBindingType::Storage { read_only: false };
    let filter = backend.cached_pipeline(
        include_str!("film_resolution.wgsl"),
        &[wgpu::BufferBindingType::Uniform, storage, storage, writable],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct FilterParams {
        dimensions: [u32; 4],
        taps: [u32; 4],
        matrix: [[f32; 4]; 3],
        window: [u32; 4],
    }
    let (radius, optical, weights) = resolution_weights(width, height, params);
    let dummy_weights = backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_prepare_dummy_weights"),
            contents: bytemuck::cast_slice(&[[0.0f32; 2]]),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let curve = output_space.map_or(0, |space| space.cctf.code());
    let matrix = output_space.map_or([[0.0; 4]; 3], |space| {
        space
            .rgb_to_rgb_identity()
            .map(|row| [row[0] as f32, row[1] as f32, row[2] as f32, 0.0])
    });
    let mut commands = RasterCommands::default();
    for first in (0..n).step_by(page as usize) {
        let count = page.min(n - first);
        let p = uniform(
            backend,
            &FilterParams {
                dimensions: [width, height, 2, optical as u32],
                taps: [0, curve, 0, 0],
                matrix,
                window: [first, count, first, 0],
            },
        );
        commands.copy(
            input,
            u64::from(first) * 12,
            &source_window,
            0,
            u64::from(count) * 12,
        );
        commands.dispatch_named(
            &filter,
            &[&p, &source_window, &dummy_weights, &target_window],
            count,
            false,
            backend,
            Some("procedural_input_transform"),
        );
        commands.copy(
            &target_window,
            0,
            &original,
            u64::from(first) * 12,
            u64::from(count) * 12,
        );
    }
    if radius > 0.0 {
        build_resolution(
            &mut commands,
            backend,
            width,
            height,
            &original,
            &horizontal,
            &filtered,
            &source_window,
            page,
            limit,
            optical,
            &weights,
        )?;
    }
    let storage = wgpu::BufferBindingType::Storage { read_only: true };
    let writable = wgpu::BufferBindingType::Storage { read_only: false };
    let canvas_layout = [wgpu::BufferBindingType::Uniform, storage, writable];
    let canvas_pass = backend.cached_pipeline_at(
        include_str!("procedural.wgsl"),
        &canvas_layout,
        "canvas_main",
    );
    let grain = backend.cached_pipeline_at(
        include_str!("procedural.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            storage,
            writable,
            storage,
            storage,
            writable,
        ],
        "main",
    );
    let mean_pass = backend.cached_pipeline_at(
        include_str!("procedural_mean.wgsl"),
        &[wgpu::BufferBindingType::Uniform, storage, writable],
        "mean_main",
    );
    let reduce_pass = backend.cached_pipeline_at(
        include_str!("procedural_mean.wgsl"),
        &[wgpu::BufferBindingType::Uniform, storage, writable],
        "reduce_main",
    );
    let (cells_w, cells_h) = canvas_cells(width, height, params);
    let cells = u64::from(cells_w) * u64::from(cells_h);
    // Digital grain has no canvas, so its composition reads the neutral half.
    let partials = backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_procedural_mean"),
            contents: bytemuck::cast_slice(&[0.5f32, 0.5, 0.5, 0.0]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
    let scratch = transport(backend, 16, "grain_procedural_scratch")?;
    let bands = if params.mode == 0 {
        canvas_bands(width, height, cells_w, cells_h, limit, params)?
    } else {
        Vec::new()
    };
    // The mean pass walks the pixels once per canvas band and the reduction
    // folds every partial, so size the buffer from the plan rather than the
    // frame: a band's chunk count depends on the band, not on `n`.
    let mut chunks = 0u64;
    for band in &bands {
        let mut first = band.pixels.0;
        while first < band.pixels.1 {
            let count = page.min(band.pixels.1 - first);
            chunks += u64::from(count).div_ceil(256);
            first += count;
        }
    }
    let partials_bytes = (chunks as u32).max(1) as u64 * 16;
    let partials = if chunks > 1 {
        transport(backend, partials_bytes, "grain_procedural_partials")?
    } else {
        partials
    };
    let scale = 1.0 + (params.raw_scale - 1.0) / 47.0 * 1.5;
    let size = canvas_size(width, height);
    let resident = cells * 8 <= limit;
    // A resident canvas is built, measured and composed once. A canvas that
    // does not fit walks bands twice: the first pass measures the exact mean,
    // the second regenerates the same cells and composes. Both passes generate
    // identical cells, so the output does not depend on which path ran.
    let canvas = if params.mode == 0 && resident {
        Some(transport(backend, cells * 8, "grain_procedural_canvas")?)
    } else {
        None
    };
    let band_bytes = bands
        .iter()
        .map(|b| u64::from(b.stored.1 - b.stored.0) * u64::from(cells_w) * 8)
        .max()
        .unwrap_or(0);
    // A resident canvas is composed from its own buffer, so the band buffer is
    // only allocated when the canvas has to be split.
    let scratch_canvas = if canvas_transport_needed(params, resident) {
        transport(backend, band_bytes.max(16), "grain_procedural_band")?
    } else {
        transport(backend, 16, "grain_procedural_band")?
    };
    if params.mode == 0 {
        let mut written = 0u32;
        let buffer = match &canvas {
            Some(canvas) => canvas,
            None => &scratch_canvas,
        };
        for band in &bands {
            build_canvas_band(
                &mut commands,
                backend,
                width,
                height,
                params,
                &original,
                &source_window,
                cells_w,
                cells_h,
                band,
                buffer,
                page as u64,
                &canvas_pass,
            );
            let mut first = band.pixels.0;
            let mut base = written;
            while first < band.pixels.1 {
                let count = page.min(band.pixels.1 - first);
                let p = uniform(
                    backend,
                    &[
                        width,
                        height,
                        first,
                        count,
                        cells_w,
                        cells_h,
                        base,
                        chunks as u32,
                        scale.to_bits(),
                        (band.stored.0 as f32).to_bits(),
                        size[0].to_bits(),
                        size[1].to_bits(),
                    ],
                );
                commands.dispatch_named(
                    &mean_pass,
                    &[&p, buffer, &partials],
                    count,
                    false,
                    backend,
                    Some("procedural_mean"),
                );
                base += u64::from(count).div_ceil(256) as u32;
                written += u64::from(count).div_ceil(256) as u32;
                first += count;
            }
        }
        let p = uniform(
            backend,
            &[
                width,
                height,
                0,
                0,
                cells_w,
                cells_h,
                0,
                chunks as u32,
                scale.to_bits(),
                0,
                size[0].to_bits(),
                size[1].to_bits(),
            ],
        );
        commands.dispatch_named(
            &reduce_pass,
            &[&p, &scratch, &partials],
            1,
            true,
            backend,
            Some("procedural_reduce"),
        );
    }
    let compose_pixel = |first: u32,
                         count: u32,
                         band_rows: (u32, u32),
                         stored_rows: u32,
                         buffer: &wgpu::Buffer,
                         commands: &mut RasterCommands| {
        let mut p = shader_params(width, height, params);
        p.canvas = [cells_w, cells_h, stored_rows, band_rows.0];
        p.window = [first, count, 0, 0];
        let p = uniform(backend, &p);
        commands.copy(
            if radius > 0.0 { &filtered } else { &original },
            u64::from(first) * 12,
            &working,
            0,
            u64::from(count) * 12,
        );
        commands.dispatch_named(
            &grain,
            &[
                &p,
                &source_window,
                buffer,
                &working,
                &partials,
                &grain_output,
            ],
            count,
            false,
            backend,
            Some("procedural_compose"),
        );
        commands.copy(
            &grain_output,
            0,
            &output,
            u64::from(first) * 12,
            u64::from(count) * 12,
        );
    };
    if params.mode != 0 {
        for first in (0..n).step_by(page as usize) {
            let count = page.min(n - first);
            compose_pixel(first, count, (0, cells_h), cells_h, &scratch, &mut commands);
        }
    } else if let Some(canvas) = &canvas {
        for first in (0..n).step_by(page as usize) {
            let count = page.min(n - first);
            compose_pixel(first, count, (0, cells_h), cells_h, canvas, &mut commands);
        }
    } else {
        // Measure again per band and compose what was just generated.
        for band in &bands {
            let rows = band.stored.1 - band.stored.0;
            build_canvas_band(
                &mut commands,
                backend,
                width,
                height,
                params,
                &original,
                &source_window,
                cells_w,
                cells_h,
                band,
                &scratch_canvas,
                page as u64,
                &canvas_pass,
            );
            let mut first = band.pixels.0;
            while first < band.pixels.1 {
                let count = page.min(band.pixels.1 - first);
                compose_pixel(
                    first,
                    count,
                    band.stored,
                    rows,
                    &scratch_canvas,
                    &mut commands,
                );
                first += count;
            }
        }
    }
    Ok(ProceduralGrainState {
        output,
        commands,
        n_bytes,
    })
}

fn resolution_weights(
    width: u32,
    height: u32,
    params: &spektrafilm_backend::ProceduralGrainGpuParams,
) -> (f32, bool, Vec<[f32; 2]>) {
    let amount = params.amount.clamp(0.0, 1.0);
    let gsf = (5200.0 / width as f32).max(3100.0 / height as f32);
    let base = (1.0 + (params.raw_scale - 1.0) / 47.0)
        * (1.0 - params.resolution_factor.clamp(0.0, 100.0) / 100.0)
        / gsf;
    let resolution_scale = if params.resolution_type == 1 {
        1.2
    } else {
        1.6
    };
    let radius = if params.mode != 0 {
        0.0
    } else {
        base * resolution_scale * (0.7 * amount * amount + 0.3 * amount + 0.05)
    };
    let optical = params.resolution_type == 0;
    let weights = if optical {
        spektrafilm_math::grain::optical_weights(radius)
            .into_iter()
            .map(|w| [w, 0.0])
            .collect()
    } else {
        spektrafilm_math::grain::fast_blur_weights(radius)
    };
    (radius, optical, weights)
}

impl WgpuBackend {
    pub(in crate::wgpu_backend) fn procedural_workspace_bytes(
        &self,
        width: u32,
        height: u32,
        params: &spektrafilm_backend::ProceduralGrainGpuParams,
    ) -> Result<u64, String> {
        let n = width
            .checked_mul(height)
            .filter(|&n| n > 0 && n <= u32::MAX / 3)
            .ok_or("procedural grain dimensions overflow")?;
        validate_coordinate_scale(params)?;
        let limit = super::super::bounded::storage_binding_limit(self)?;
        let page = (limit / 12).min(u64::from(n));
        let (radius, optical, weights) = resolution_weights(width, height, params);
        let pages = u64::from(n).div_ceil(page);
        let mut bytes = u128::from(n) * 48 + u128::from(page) * 24 + 8;
        bytes += u128::from(pages) * std::mem::size_of::<ShaderParams>() as u128
            + u128::from(pages) * 48;
        if params.mode == 0 {
            let (cw, ch) = canvas_cells(width, height, params);
            // A resident canvas costs its full extent; a banded one costs the
            // band buffer, which the limit bounds.
            let canvas_bytes = (u128::from(cw) * u128::from(ch) * 8).min(u128::from(limit));
            bytes += canvas_bytes;
            bytes += u128::from(n).div_ceil(256) * 16;
        }
        if radius > 0.0 {
            let reach = if optical {
                weights.len() / 2 + 1
            } else {
                weights
                    .iter()
                    .map(|w| w[1].abs().ceil() as usize + 2)
                    .max()
                    .unwrap_or(1)
            } as u64;
            if (2 * reach + 1) * 24 <= limit && weights.len() as u64 * 8 <= limit {
                let count = (page / 2).saturating_sub(2 * reach).max(1);
                // Axis 0 keeps one job per row segment; axis 1 emits one job per
                // band (or a command pair per pixel on the fallback) plus the
                // band's row copies.
                let (band, banded, window_rows) =
                    column_band_plan(page, width, height, reach as u32);
                let _ = window_rows;
                let jobs = u64::from(height) * u64::from(width).div_ceil(count)
                    + if banded {
                        u64::from(height).div_ceil(u64::from(band)) * 2
                            + u64::from(height).div_ceil(u64::from(band)) * 2 * reach
                    } else {
                        u64::from(width) * u64::from(height).div_ceil(count)
                    };
                bytes += u128::from(page) * 12 + weights.len() as u128 * 8 + u128::from(jobs) * 96;
            } else {
                let taps = if optical {
                    weights.len() / 2
                } else {
                    weights.len()
                } as u64;
                let tap_chunk = (limit / 96).min(taps.max(1));
                if tap_chunk == 0 {
                    return Err("procedural resolution cannot hold one tap".into());
                }
                let partitions = taps.div_ceil(tap_chunk);
                bytes += u128::from(tap_chunk) * 96 + 12;
                bytes += 2
                    * u128::from(n)
                    * (u128::from(taps) * 16 + u128::from(partitions) * u128::from(pages + 1) * 48);
            }
        }
        u64::try_from(bytes).map_err(|_| "procedural grain resource plan overflows".into())
    }
}

// Analogue divide_normal_f32's canvas divisor is virtual_size * clamp(raw_scale,
// .5, 1.4). The dimension guard in build_bounded_grain_procedural bounds virtual_size
// below 2^43. Reject an undefined scale before allocations or device work.
pub(super) fn validate_coordinate_scale(
    params: &spektrafilm_backend::ProceduralGrainGpuParams,
) -> Result<(), String> {
    if params.mode == 0 && !params.raw_scale.is_finite() {
        return Err("procedural analogue grain scale must be finite".into());
    }
    Ok(())
}

/// Canvas cells the scan addresses, matching the host model's table.
fn canvas_cells(
    width: u32,
    height: u32,
    params: &spektrafilm_backend::ProceduralGrainGpuParams,
) -> (u32, u32) {
    let size = canvas_size(width, height);
    let scale = 1.0 + (params.raw_scale - 1.0) / 47.0 * 1.5;
    let used = |extent: f32, limit: f32| {
        ((extent / scale).ceil() as i64 + 2).clamp(1, limit.max(1.0) as i64) as u32
    };
    (used(size[0], size[0]), used(size[1], size[1]))
}

/// Whether the canvas needs its own band buffer rather than the resident one.
fn canvas_transport_needed(
    params: &spektrafilm_backend::ProceduralGrainGpuParams,
    resident: bool,
) -> bool {
    // Digital grain has no table; analogue composes from the resident buffer.
    params.mode == 0 && !resident
}

/// One canvas band: the rows a single canvas buffer holds and the pixels that
/// read them.
///
/// The buffer stores `stored` rows, one more than the band on each side where
/// the canvas allows it, because the composition averages a 3x3 neighbourhood.
/// `pixels` is the pixel range whose canvas row falls inside the band; the
/// mapping is monotone, so each pixel belongs to exactly one band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CanvasBand {
    stored: (u32, u32),
    pixels: (u32, u32),
}

/// Split the canvas into row bands that each fit one binding.
///
/// A canvas that fits stays whole. Otherwise every band stores one extra row
/// above and below (clamped at the canvas edges) so a pixel in the band can
/// still average its full neighbourhood.
fn canvas_bands(
    width: u32,
    height: u32,
    cells_w: u32,
    cells_h: u32,
    limit: u64,
    params: &spektrafilm_backend::ProceduralGrainGpuParams,
) -> Result<Vec<CanvasBand>, String> {
    let row_bytes = u64::from(cells_w) * 8;
    // The canvas pass stages whole source rows, so a window narrower than the
    // frame would silently drop the tail of every row.
    if u64::from(width) * 12 > limit {
        return Err(format!(
            "procedural grain needs a {}-pixel source row in {limit} bytes of staging window",
            width
        ));
    }
    if row_bytes > limit {
        return Err(format!(
            "procedural grain canvas row needs {row_bytes} bytes; device binding limit is {limit}"
        ));
    }
    let rows = u64::from(cells_h);
    // Mirror the composition's cell mapping in binary32 so a band owns exactly
    // the pixels the shader resolves into it.
    let size_y = canvas_size(width, height)[1];
    let scale = 1.0 + (params.raw_scale - 1.0) / 47.0 * 1.5;
    let inv_h = 1.0 / height.saturating_sub(1).max(1) as f32;
    let cell_row = |y: u32| -> i32 { ((y as f32 * inv_h) * (size_y / scale)) as i32 };
    let first_pixel = |row: u32| -> u32 {
        // `cell_row` is monotone, so the first pixel at or past `row` is a bound.
        let (mut low, mut high) = (0u32, height);
        while low < high {
            let mid = low + (high - low) / 2;
            if cell_row(mid) < row as i32 {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    };
    let mut bands = Vec::new();
    if rows * row_bytes <= limit {
        bands.push(CanvasBand {
            stored: (0, cells_h),
            // Pixel ranges are linear sample indices, so a band of rows covers
            // a multiple of the image width.
            pixels: (0, height * width),
        });
        return Ok(bands);
    }
    let stored_rows = limit / row_bytes;
    // The canvas pass bilinearly samples the source rows the band covers, and
    // those rows must fit the staging window as well.
    let source_rows_per_cell_row = u64::from(height).div_ceil(u64::from(cells_h)).max(1);
    // Staging window: the largest whole-row window a binding of `limit` holds.
    let window_rows = limit / (u64::from(width) * 12) / source_rows_per_cell_row;
    let compose_rows = stored_rows
        .saturating_sub(2)
        .min(window_rows.saturating_sub(2))
        .max(1) as u32;
    if stored_rows < 3 {
        return Err(format!(
            "procedural grain canvas band needs 3 rows of {row_bytes} bytes; device binding limit \
             is {limit}"
        ));
    }
    let mut row = 0u32;
    while row < cells_h {
        let next = (row + compose_rows).min(cells_h);
        let stored = (row.saturating_sub(1), (next + 1).min(cells_h));
        // A pixel whose centre clamps to the last canvas row still needs
        // composing, so the final band takes every remaining sample.
        let end = if next == cells_h {
            height * width
        } else {
            first_pixel(next) * width
        };
        let pixels = (first_pixel(row) * width, end);
        if pixels.0 < pixels.1 {
            bands.push(CanvasBand { stored, pixels });
        }
        row = next;
    }
    if bands.is_empty() {
        return Err("procedural grain canvas has no reachable row".into());
    }
    Ok(bands)
}

/// Build one canvas band into `buffer`, staging the source rows it samples.
#[allow(clippy::too_many_arguments)]
fn build_canvas_band(
    commands: &mut RasterCommands,
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    params: &spektrafilm_backend::ProceduralGrainGpuParams,
    original: &[super::super::paged::ImagePage],
    source_window: &wgpu::Buffer,
    cells_w: u32,
    cells_h: u32,
    band: &CanvasBand,
    buffer: &wgpu::Buffer,
    page: u64,
    canvas_pass: &super::super::CachedPipelineRef,
) {
    let (store_from, store_to) = band.stored;
    // Cells address the full virtual canvas, not the stored table.
    let canvas_rows = canvas_size(width, height)[1];
    let to_source = |cell_row: f32| (cell_row / (canvas_rows - 1.0)) * (height as f32 - 1.0) - 0.1;
    let source_rows = |from: u32, to: u32| {
        let first = (to_source(from as f32).floor() as i64 - 1).max(0) as u32;
        let last = ((to_source((to - 1) as f32).ceil() as i64 + 2).max(0) as u32).min(height);
        (first, last)
    };
    // The staging window holds whole rows, so a band is generated in chunks
    // sized to the source rows it samples. A canvas built from a single
    // resident buffer would otherwise stage the whole frame at once.
    let staging_rows = (page / u64::from(width)).max(1) as u32;
    let mut chunk = store_to - store_from;
    let mut row = store_from;
    while row < store_to {
        chunk = chunk.min(store_to - row).max(1);
        let (first_row, last_row) = loop {
            let (first, last) = source_rows(row, row + chunk);
            if last - first <= staging_rows || chunk == 1 {
                break (first, last);
            }
            chunk /= 2;
        };
        // Whole rows form one contiguous interval. RasterCommands splits it
        // only at physical page boundaries, not at every row.
        commands.copy(
            original,
            u64::from(first_row) * u64::from(width) * 12,
            source_window,
            0,
            u64::from(last_row - first_row) * u64::from(width) * 12,
        );
        let mut p = shader_params(width, height, params);
        // `canvas.z` is where this chunk starts inside the band buffer; the
        // cell coordinates themselves stay global.
        p.canvas = [cells_w, cells_h, (row - store_from) * cells_w, 0];
        p.window = [
            row * cells_w,
            chunk * cells_w,
            first_row,
            last_row - first_row,
        ];
        let p = uniform(backend, &p);
        commands.dispatch_named(
            canvas_pass,
            &[&p, source_window, buffer],
            chunk * cells_w,
            false,
            backend,
            Some("procedural_canvas"),
        );
        row += chunk;
    }
}

/// Row-band plan for the vertical Film Resolution pass.
///
/// Returns the band height, whether bands are usable at all, and how many
/// source rows the window must hold.
fn column_band_plan(page: u64, width: u32, height: u32, reach: u32) -> (u32, bool, u64) {
    // A frame that fits the window is one band: its margins clamp to the frame
    // itself, so it needs only `height` rows however thin it is.
    let whole = u64::from(width) * u64::from(height) <= page;
    let band = if whole {
        height
    } else {
        (page / u64::from(width))
            .saturating_sub(2 * u64::from(reach) + 1)
            .max(1) as u32
    };
    // Otherwise the window must hold the band plus both tap margins. Frames too
    // thin or too wide for either keep the per-column path, which costs one
    // command pair per pixel.
    let banded = whole || (u64::from(band) + 2 * u64::from(reach)) * u64::from(width) <= page;
    let window_rows = if whole {
        u64::from(height)
    } else {
        u64::from(band) + 2 * u64::from(reach)
    };
    (band, banded, window_rows)
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GatherParams {
    dimensions: [u32; 4],
    page: [u32; 4],
    controls: [f32; 4],
}

#[allow(clippy::too_many_arguments)]
fn build_resolution(
    commands: &mut RasterCommands,
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    original: &[super::super::paged::ImagePage],
    horizontal: &[super::super::paged::ImagePage],
    filtered: &[super::super::paged::ImagePage],
    source_window: &wgpu::Buffer,
    page: u32,
    limit: u64,
    optical: bool,
    weights: &[[f32; 2]],
) -> Result<(), String> {
    let n = width * height;
    let reach = if optical {
        weights.len() / 2 + 1
    } else {
        weights
            .iter()
            .map(|w| w[1].abs().ceil() as usize + 2)
            .max()
            .unwrap_or(1)
    };
    if (2 * reach + 1) as u64 * 24 <= limit && weights.len() as u64 * 8 <= limit {
        let pipe = backend.cached_pipeline(
            include_str!("film_resolution.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let weights_buf = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grain_resolution_weights"),
                contents: bytemuck::cast_slice(weights),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let result = transport(
            backend,
            u64::from(page) * 12,
            "grain_resolution_axis_result",
        )?;
        // Axis 0 blurs along a row, so each row segment is contiguous. Axis 1
        // blurs down a column, where a column is strided: copying column by
        // column emitted one command per pixel and a second one to write the
        // result back, which is 122 million commands on a 61 MP frame and the
        // reason that frame peaked at 21 GB. Copy a band of whole rows instead
        // and address them with the image width in the shader.
        let (band, banded, _) = column_band_plan(u64::from(page), width, height, reach as u32);
        for axis in 0..2 {
            let source = if axis == 0 { original } else { horizontal };
            let destination = if axis == 0 { horizontal } else { filtered };
            if axis == 1 && banded {
                let mut first_row = 0u32;
                while first_row < height {
                    let rows = band.min(height - first_row);
                    let low = first_row.saturating_sub(reach as u32);
                    let high = height.min(first_row + rows + reach as u32);
                    commands.copy(
                        source,
                        u64::from(low) * u64::from(width) * 12,
                        source_window,
                        0,
                        u64::from(high - low) * u64::from(width) * 12,
                    );
                    let mut data = [0u32; 24];
                    data[..8].copy_from_slice(&[
                        width,
                        height,
                        1,
                        optical as u32,
                        weights.len() as u32,
                        0,
                        0,
                        low,
                    ]);
                    data[20..].copy_from_slice(&[first_row * width, rows * width, high - low, 2]);
                    let p = uniform(backend, &data);
                    commands.dispatch_named(
                        &pipe,
                        &[&p, source_window, &weights_buf, &result],
                        rows * width,
                        false,
                        backend,
                        Some("procedural_resolution_filter"),
                    );
                    commands.copy(
                        &result,
                        0,
                        destination,
                        u64::from(first_row * width) * 12,
                        u64::from(rows * width) * 12,
                    );
                    first_row += rows;
                }
                continue;
            }
            if axis == 1 {
                for line in 0..width {
                    let mut first = 0u32;
                    while first < height {
                        let count = (page / 2)
                            .saturating_sub(2 * reach as u32)
                            .max(1)
                            .min(height - first);
                        let low = first.saturating_sub(reach as u32);
                        let high = height.min(first + count + reach as u32);
                        for side in 0..2 {
                            let across = if side == 0 {
                                line.saturating_sub(1)
                            } else {
                                line
                            };
                            let offset = u64::from(side * (high - low)) * 12;
                            for position in low..high {
                                commands.copy(
                                    source,
                                    u64::from(position * width + across) * 12,
                                    source_window,
                                    offset + u64::from(position - low) * 12,
                                    12,
                                );
                            }
                        }
                        let mut data = [0u32; 24];
                        data[..8].copy_from_slice(&[
                            width,
                            height,
                            1,
                            optical as u32,
                            weights.len() as u32,
                            0,
                            line,
                            low,
                        ]);
                        data[20..].copy_from_slice(&[first, count, high - low, 1]);
                        let p = uniform(backend, &data);
                        commands.dispatch_named(
                            &pipe,
                            &[&p, source_window, &weights_buf, &result],
                            count,
                            false,
                            backend,
                            Some("procedural_resolution_filter"),
                        );
                        for position in 0..count {
                            commands.copy(
                                &result,
                                u64::from(position) * 12,
                                destination,
                                u64::from((first + position) * width + line) * 12,
                                12,
                            );
                        }
                        first += count;
                    }
                }
                continue;
            }
            let span = width;
            let lines = height;
            for line in 0..lines {
                let mut first = 0u32;
                while first < span {
                    let count = (page / 2)
                        .saturating_sub(2 * reach as u32)
                        .max(1)
                        .min(span - first);
                    let low = first.saturating_sub(reach as u32);
                    let high = span.min(first + count + reach as u32);
                    for side in 0..2 {
                        let across = if side == 0 {
                            line.saturating_sub(1)
                        } else {
                            line
                        };
                        let offset = u64::from(side * (high - low)) * 12;
                        commands.copy(
                            source,
                            u64::from(across * width + low) * 12,
                            source_window,
                            offset,
                            u64::from(high - low) * 12,
                        );
                    }
                    let mut data = [0u32; 24];
                    data[..8].copy_from_slice(&[
                        width,
                        height,
                        0,
                        optical as u32,
                        weights.len() as u32,
                        0,
                        line,
                        low,
                    ]);
                    data[20..].copy_from_slice(&[first, count, high - low, 1]);
                    let p = uniform(backend, &data);
                    commands.dispatch_named(
                        &pipe,
                        &[&p, source_window, &weights_buf, &result],
                        count,
                        false,
                        backend,
                        Some("procedural_resolution_filter"),
                    );
                    commands.copy(
                        &result,
                        0,
                        destination,
                        u64::from(line * width + first) * 12,
                        u64::from(count) * 12,
                    );
                    first += count;
                }
            }
        }
        return Ok(());
    }
    let taps = if optical {
        weights.len() / 2
    } else {
        weights.len()
    } as u32;
    let tap_chunk = (limit / 96).min(u64::from(taps.max(1))) as u32;
    let samples = transport(
        backend,
        u64::from(tap_chunk) * 96,
        "grain_resolution_tap_samples",
    )?;
    let result = transport(backend, 12, "grain_resolution_pixel")?;
    let pipe = backend.cached_pipeline(
        include_str!("resolution_window.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    for axis in 0..2 {
        let source = if axis == 0 { original } else { horizontal };
        let destination = if axis == 0 { horizontal } else { filtered };
        for pixel in 0..n {
            for first_tap in (0..taps).step_by(tap_chunk as usize) {
                let count = tap_chunk.min(taps - first_tap);
                let packed: Vec<[f32; 4]> = (first_tap..first_tap + count)
                    .map(|t| {
                        if optical {
                            [
                                weights[(t + taps) as usize][0],
                                weights[t as usize][0],
                                0.0,
                                0.0,
                            ]
                        } else {
                            [weights[t as usize][0], weights[t as usize][1], 0.0, 0.0]
                        }
                    })
                    .collect();
                let weights_buf =
                    backend
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("grain_resolution_tap_weights"),
                            contents: bytemuck::cast_slice(&packed),
                            usage: wgpu::BufferUsages::STORAGE,
                        });
                for first in (0..n).step_by(page as usize) {
                    let page_count = page.min(n - first);
                    let p = uniform(
                        backend,
                        &[
                            width,
                            height,
                            axis,
                            optical as u32,
                            pixel,
                            first_tap,
                            count,
                            taps,
                            first,
                            page_count,
                            0u32,
                            0,
                        ],
                    );
                    commands.copy(
                        source,
                        u64::from(first) * 12,
                        source_window,
                        0,
                        u64::from(page_count) * 12,
                    );
                    commands.dispatch_named(
                        &pipe,
                        &[&p, source_window, &weights_buf, &samples, &result],
                        count,
                        false,
                        backend,
                        Some("procedural_resolution_gather"),
                    );
                }
                let p = uniform(
                    backend,
                    &[
                        width,
                        height,
                        axis,
                        optical as u32,
                        pixel,
                        first_tap,
                        count,
                        taps,
                        0u32,
                        0,
                        1,
                        u32::from(first_tap + count == taps),
                    ],
                );
                commands.dispatch_named(
                    &pipe,
                    &[&p, source_window, &weights_buf, &samples, &result],
                    1,
                    false,
                    backend,
                    Some("procedural_resolution_filter"),
                );
            }
            commands.copy(&result, 0, destination, u64::from(pixel) * 12, 12);
        }
    }
    Ok(())
}

impl ProceduralGrainState {
    pub(in crate::wgpu_backend) fn submit(
        &self,
        backend: &WgpuBackend,
        output: &(impl RasterStorage + ?Sized),
    ) -> Result<(), String> {
        self.commands.submit(backend)?;
        let mut copies = RasterCommands::default();
        copies.copy(&self.output, 0, output, 0, self.n_bytes);
        copies.submit(backend)
    }
}

#[cfg(test)]
mod logic_tests {
    #[test]
    fn analogue_coordinate_scale_rejects_nonfinite_before_dispatch() {
        let mut params = test_params();
        params.mode = 0;
        for scale in [f32::NEG_INFINITY, f32::INFINITY, f32::NAN] {
            params.raw_scale = scale;
            assert!(super::validate_coordinate_scale(&params).is_err());
        }
        for scale in [-f32::MAX, -0.0, 0.0, f32::MIN_POSITIVE, 0.5, 1.4, f32::MAX] {
            params.raw_scale = scale;
            assert!(super::validate_coordinate_scale(&params).is_ok());
        }
    }

    use super::*;

    #[test]
    fn rectangular_canvas_and_generator_geometry_share_host_dimensions() {
        let mut params = test_params();
        params.raw_scale = 1.1;
        params.cluster_size = 0.8;
        for (width, height, size) in [
            (768, 512, [5200.0f32, 3466.0]),
            (512, 768, [5200.0f32, 7800.0]),
        ] {
            let uniform = shader_params(width, height, &params);
            assert_eq!(&uniform.flags[..2], &size);
            let scaled = size.map(|v| v * params.raw_scale);
            assert_eq!(
                uniform.generator_geometry,
                [
                    scaled[0] / params.cluster_size / params.raw_scale,
                    scaled[1] / params.cluster_size / params.raw_scale,
                    scaled[0] / scaled[1],
                    1.0f32 / 256.0 / params.cluster_size,
                ]
            );
        }
    }

    /// The band plan decides between one bulk pass and a command per pixel, so
    /// pin the geometries that used to fall to the slow side.
    #[test]
    fn column_band_plan_covers_thin_and_wide_frames() {
        // A whole frame fits the window: one band, even though it is thinner
        // than the tap margins.
        let (_, banded, rows) = column_band_plan(20_000 * 2, 20_000, 2, 8);
        assert!(banded, "a frame that fits the window must band");
        assert_eq!(rows, 2);
        // Fewer columns than one window: no band can hold a row plus margins.
        let (_, banded, _) = column_band_plan(1_728, 20_000, 2, 8);
        assert!(!banded, "a row wider than the window cannot band");
        // An ordinary 61 MP frame bands with full margins.
        let (band, banded, rows) = column_band_plan(11_000_000, 9_568, 6_376, 8);
        assert!(banded);
        assert_eq!(rows, u64::from(band) + 16);
    }

    /// Bands must tile the frame exactly and keep one row of overlap, or a
    /// pixel reads a neighbour's buffer.
    #[test]
    fn canvas_bands_tile_the_frame_and_keep_their_margins() {
        let params = spektrafilm_backend::ProceduralGrainGpuParams {
            mode: 0,
            resolution_type: 0,
            amount: 1.0,
            shadows: 1.0,
            midtones: 1.0,
            highlights: 1.0,
            raw_scale: 1.0,
            cluster_size: 1.6,
            rotation: 0.0,
            color: 1.0,
            resolution_factor: 1.0,
            seed: 42,
            timer: None,
            colored: true,
            clustered: false,
        };
        let (w, h) = (37u32, 19u32);
        let (cw, ch) = canvas_cells(w, h, &params);
        assert!(u64::from(cw) * u64::from(ch) * 8 > 8 << 20);
        let bands = canvas_bands(w, h, cw, ch, 8 << 20, &params).unwrap();
        assert!(bands.len() > 1);
        assert_eq!(
            bands[0].pixels.0, 0,
            "first band starts at the first sample"
        );
        assert_eq!(
            bands.last().unwrap().pixels.1,
            w * h,
            "last band reaches the last sample"
        );
        for pair in bands.windows(2) {
            assert_eq!(pair[0].pixels.1, pair[1].pixels.0, "no gap or overlap");
            assert!(
                pair[1].stored.0 + 1 >= pair[0].stored.1 - 1,
                "neighbouring bands keep their margins"
            );
        }
        assert_eq!(bands[0].stored.0, 0);
        assert_eq!(bands.last().unwrap().stored.1, ch);
    }

    pub(super) fn test_params() -> spektrafilm_backend::ProceduralGrainGpuParams {
        spektrafilm_backend::ProceduralGrainGpuParams {
            mode: 0,
            resolution_type: 0,
            amount: 1.0,
            shadows: 1.0,
            midtones: 1.0,
            highlights: 1.0,
            raw_scale: 1.0,
            cluster_size: 1.6,
            rotation: 0.0,
            color: 1.0,
            resolution_factor: 1.0,
            seed: 42,
            timer: None,
            colored: true,
            clustered: false,
        }
    }

    #[test]
    fn column_band_plan_switches_at_margin_and_whole_frame_limits() {
        assert_eq!(column_band_plan(89, 10, 20, 4), (1, false, 9));
        assert_eq!(column_band_plan(90, 10, 20, 4), (1, true, 9));
        assert_eq!(column_band_plan(200, 10, 20, 4), (20, true, 20));
        assert_eq!(column_band_plan(19, 10, 2, 4), (1, false, 9));
        assert_eq!(column_band_plan(20, 10, 2, 4), (2, true, 2));
    }

    #[test]
    fn canvas_band_limits_reject_undersized_windows_and_allow_exact_fit() {
        let params = test_params();
        assert!(canvas_bands(4, 4, 2, 1, 47, &params).is_err());
        assert!(canvas_bands(4, 4, 6, 1, 48, &params).is_ok());
        assert!(canvas_bands(4, 4, 10, 4, 48, &params).is_err());
        assert!(canvas_bands(4, 4, 6, 4, 143, &params).is_err());

        let bands = canvas_bands(4, 4, 6, 4, 144, &params).unwrap();
        assert_eq!(bands.first().unwrap().pixels.0, 0);
        assert_eq!(bands.last().unwrap().pixels.1, 16);
        for band in &bands {
            assert!(u64::from(band.stored.1 - band.stored.0) * 48 <= 144);
        }
        for pair in bands.windows(2) {
            assert_eq!(pair[0].pixels.1, pair[1].pixels.0);
        }
        assert_eq!(
            canvas_bands(4, 4, 6, 4, 192, &params).unwrap(),
            vec![CanvasBand {
                stored: (0, 4),
                pixels: (0, 16)
            }]
        );
    }

    #[test]
    fn canvas_transport_is_only_needed_for_nonresident_analog_grain() {
        let mut params = test_params();
        assert!(canvas_transport_needed(&params, false));
        assert!(!canvas_transport_needed(&params, true));
        params.mode = 1;
        assert!(!canvas_transport_needed(&params, false));
        assert!(!canvas_transport_needed(&params, true));
    }

    /// Every sample must be composed by exactly one band, and every band must
    /// store the rows its pixels read. Brute force over the same mapping the
    /// shader uses, across geometries that put the canvas on both sides of the
    /// binding limit.
    #[test]
    fn canvas_bands_cover_every_sample_once() {
        for (w, h) in [(37u32, 19u32), (1, 1), (2, 512), (512, 2), (97, 41)] {
            let (cw, ch) = canvas_cells(
                w,
                h,
                &spektrafilm_backend::ProceduralGrainGpuParams {
                    mode: 0,
                    resolution_type: 0,
                    amount: 1.0,
                    shadows: 1.0,
                    midtones: 1.0,
                    highlights: 1.0,
                    raw_scale: 1.0,
                    cluster_size: 1.6,
                    rotation: 0.0,
                    color: 1.0,
                    resolution_factor: 1.0,
                    seed: 42,
                    timer: None,
                    colored: true,
                    clustered: false,
                },
            );
            for limit in [1 << 20, 4 << 20, 16 << 20, 1 << 40] {
                let params = spektrafilm_backend::ProceduralGrainGpuParams {
                    mode: 0,
                    resolution_type: 0,
                    amount: 1.0,
                    shadows: 1.0,
                    midtones: 1.0,
                    highlights: 1.0,
                    raw_scale: 1.0,
                    cluster_size: 1.6,
                    rotation: 0.0,
                    color: 1.0,
                    resolution_factor: 1.0,
                    seed: 42,
                    timer: None,
                    colored: true,
                    clustered: false,
                };
                let Ok(bands) = canvas_bands(w, h, cw, ch, limit, &params) else {
                    continue;
                };
                // Brute force: the same f32 mapping the shader resolves.
                let gsf = (5200.0f32 / w as f32).max(3100.0f32 / h as f32);
                let size_y = (h as f32 * gsf).floor().max(1.0);
                let scale = 1.0 + (1.0 - 1.0) / 47.0 * 1.5;
                let inv_h = 1.0 / h.saturating_sub(1).max(1) as f32;
                let mut seen = vec![0u32; (w * h) as usize];
                for band in &bands {
                    for index in band.pixels.0..band.pixels.1 {
                        let y = index / w;
                        let centre = ((y as f32 * inv_h) * (size_y / scale)) as i32;
                        let clamped = centre.clamp(0, ch as i32 - 1) as u32;
                        for dy in -1i32..=1 {
                            let row = (clamped as i32 + dy).clamp(0, ch as i32 - 1) as u32;
                            assert!(
                                row >= band.stored.0 && row < band.stored.1,
                                "{w}x{h} limit {limit}: pixel row {y} reads row {row} \
                                 outside band {:?}",
                                band.stored
                            );
                        }
                        seen[index as usize] += 1;
                    }
                }
                assert!(
                    seen.iter().all(|&count| count == 1),
                    "{w}x{h} limit {limit}: every sample must be composed exactly once"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contiguous_row_gathers_match_row_sized_copies_across_physical_pages() {
        use super::super::super::paged::ImagePage;
        use super::super::raster::RasterCommand;

        let gpu = crate::test_support::adapter();
        let (width, height) = (13u32, 37u32);
        let row_bytes = u64::from(width) * 12;
        let values: Vec<f32> = (0..width * height * 3)
            .map(|i| (i % 71) as f32 / 71.0 - 0.25)
            .collect();
        // Neither physical source nor destination pages end on image rows.
        let input_pages: Vec<_> = values
            .chunks(123)
            .enumerate()
            .map(|(i, chunk)| ImagePage {
                first_pixel: i as u32 * 41,
                n_pixels: chunk.len() as u32 / 3,
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("procedural_row_copy_source"),
                        contents: bytemuck::cast_slice(chunk),
                        usage: wgpu::BufferUsages::COPY_SRC,
                    }),
            })
            .collect();
        let make_pages = |label| -> Vec<ImagePage> {
            (0..width * height)
                .step_by(47)
                .map(|first| {
                    let n_pixels = 47.min(width * height - first);
                    ImagePage {
                        first_pixel: first,
                        n_pixels,
                        buffer: transport(&gpu, u64::from(n_pixels) * 12, label).unwrap(),
                    }
                })
                .collect()
        };
        let horizontal = make_pages("procedural_row_copy_horizontal");
        let filtered = make_pages("procedural_row_copy_filtered");
        let page = width * 12;
        let window = transport(&gpu, u64::from(page) * 12, "procedural_row_copy_window").unwrap();
        // A partial canvas band with enough cells to require several source
        // chunks; its coordinates stay absolute, independent of copy batching.
        let canvas = transport(&gpu, 3 * 13600 * 8, "procedural_row_copy_canvas").unwrap();
        let params = super::logic_tests::test_params();
        let canvas_pass = gpu.cached_pipeline_at(
            include_str!("procedural.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            "canvas_main",
        );
        for canvas_job in [false, true] {
            let mut commands = RasterCommands::default();
            if canvas_job {
                build_canvas_band(
                    &mut commands,
                    &gpu,
                    width,
                    height,
                    &params,
                    &input_pages,
                    &window,
                    3,
                    14800,
                    &CanvasBand {
                        stored: (400, 14000),
                        pixels: (0, width * height),
                    },
                    &canvas,
                    u64::from(page),
                    &canvas_pass,
                );
            } else {
                assert!(column_band_plan(u64::from(page), width, height, 2).1);
                build_resolution(
                    &mut commands,
                    &gpu,
                    width,
                    height,
                    &input_pages,
                    &horizontal,
                    &filtered,
                    &window,
                    page,
                    u64::from(page) * 12,
                    true,
                    &[[0.25, 0.0], [0.5, 0.0], [0.25, 0.0]],
                )
                .unwrap();
            }
            let mut row_sized = RasterCommands::default();
            let mut copies = 0;
            let mut dispatches = 0u32;
            let mut gathered = vec![u64::MAX; (u64::from(page) * 3) as usize];
            for command in &commands.0 {
                match command {
                    RasterCommand::Copy {
                        source,
                        source_offset,
                        destination,
                        destination_offset,
                        bytes,
                    } => {
                        copies += 1;
                        if *destination == window {
                            let source_page = input_pages
                                .iter()
                                .chain(&horizontal)
                                .find(|p| p.buffer == *source)
                                .unwrap();
                            let first_sample =
                                u64::from(source_page.first_pixel) * 3 + source_offset / 4;
                            for sample in 0..bytes / 4 {
                                gathered[(destination_offset / 4 + sample) as usize] =
                                    first_sample + sample;
                            }
                        }
                        let mut offset = 0;
                        while offset < *bytes {
                            let count = row_bytes.min(bytes - offset);
                            row_sized.0.push(RasterCommand::Copy {
                                source: source.clone(),
                                source_offset: source_offset + offset,
                                destination: destination.clone(),
                                destination_offset: destination_offset + offset,
                                bytes: count,
                            });
                            offset += count;
                        }
                    }
                    RasterCommand::Dispatch {
                        pipeline,
                        group,
                        count,
                        groups,
                        label,
                    } => {
                        let expected_rows = if canvas_job {
                            // Hand-derived source rows for canvas chunks
                            // 400..3800, 3800..7200, 7200..10600, 10600..14000.
                            Some([(0u32, 12u32), (8, 20), (16, 28), (24, 36)][dispatches as usize])
                        } else if dispatches >= height {
                            let first = (dispatches - height) * 7;
                            Some((first.saturating_sub(2), height.min(first + 9)))
                        } else {
                            None // The horizontal pass stages two row segments.
                        };
                        if let Some((low, high)) = expected_rows {
                            let expected: Vec<_> =
                                (u64::from(low * width) * 3..u64::from(high * width) * 3).collect();
                            assert_eq!(
                                &gathered[..expected.len()],
                                &expected,
                                "wrong source range at dispatch {dispatches}, canvas={canvas_job}"
                            );
                            assert!(
                                gathered[expected.len()..].iter().all(|&x| x == u64::MAX),
                                "extra source rows at dispatch {dispatches}, canvas={canvas_job}"
                            );
                        }
                        gathered.fill(u64::MAX);
                        dispatches += 1;
                        row_sized.0.push(RasterCommand::Dispatch {
                            pipeline: pipeline.clone(),
                            group: group.clone(),
                            count: *count,
                            groups: *groups,
                            label: *label,
                        });
                    }
                }
            }
            assert_eq!(dispatches, if canvas_job { 4 } else { height + 6 });
            // Copy-only pages above deliberately force physical intersections.
            // At least one gather must still cover several rows in one command.
            assert!(commands.0.iter().any(|c| matches!(c,
                RasterCommand::Copy { destination, bytes, .. }
                    if *destination == window && *bytes > row_bytes)));
            assert!(row_sized.0.len() > commands.0.len());
            assert!(copies > 1);
            let bytes = if canvas_job {
                canvas.size()
            } else {
                u64::from(width * height) * 12
            };
            let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("procedural_row_copy_readback"),
                size: bytes * 2,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            // Poison every writable transport between variants, including
            // scratch and per-axis results retained through copy commands.
            // Different poisons expose a missing write instead of reusing the
            // first execution's output. Immutable source pages are COPY_SRC only.
            let mut writable = Vec::new();
            for command in &commands.0 {
                if let RasterCommand::Copy {
                    source,
                    destination,
                    ..
                } = command
                {
                    for buffer in [source, destination] {
                        if buffer.usage().contains(wgpu::BufferUsages::COPY_DST)
                            && !writable.contains(buffer)
                        {
                            writable.push(buffer.clone());
                        }
                    }
                }
            }
            if canvas_job {
                writable.push(canvas.clone());
            }
            for (index, variant) in [&commands, &row_sized].into_iter().enumerate() {
                for buffer in &writable {
                    let poison = vec![
                        if index == 0 { -123.0f32 } else { 321.0 };
                        buffer.size() as usize / 4
                    ];
                    gpu.queue
                        .write_buffer(buffer, 0, bytemuck::cast_slice(&poison));
                }
                variant.submit(&gpu).unwrap();
                let mut copy = RasterCommands::default();
                if canvas_job {
                    copy.copy(&canvas, 0, &readback, index as u64 * bytes, bytes);
                } else {
                    copy.copy(&filtered, 0, &readback, index as u64 * bytes, bytes);
                }
                copy.submit(&gpu).unwrap();
            }
            let actual = gpu.read_mapped_f32(&readback, bytes * 2).unwrap();
            let (bulk, rows) = actual.split_at(bytes as usize / 4);
            assert_eq!(
                bulk.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                rows.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn procedural_windows_preserve_absolute_coordinates_and_resolution() {
        let gpu = crate::test_support::adapter();
        let (width, height) = (17, 7);
        let values: Vec<f32> = (0..width * height * 3)
            .map(|i| 0.08 + (i % 31) as f32 * 0.02)
            .collect();
        let input: Vec<_> = values
            .chunks(21)
            .enumerate()
            .map(|(index, samples)| super::super::super::paged::ImagePage {
                first_pixel: index as u32 * 7,
                n_pixels: samples.len() as u32 / 3,
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("procedural_physical_seam"),
                        contents: bytemuck::cast_slice(samples),
                        usage: wgpu::BufferUsages::COPY_SRC,
                    }),
            })
            .collect();
        let mut params = spektrafilm_backend::ProceduralGrainGpuParams {
            mode: 0,
            resolution_type: 0,
            amount: 0.7,
            shadows: 0.6,
            midtones: 0.5,
            highlights: 0.4,
            raw_scale: 1.1,
            cluster_size: 0.8,
            rotation: 0.4,
            color: 0.5,
            resolution_factor: 20.0,
            seed: 19,
            timer: None,
            colored: true,
            clustered: true,
        };
        let run = |params: &spektrafilm_backend::ProceduralGrainGpuParams, limit| {
            let state =
                build_bounded_grain_procedural(params, width, height, &input, None, &gpu, limit)
                    .unwrap();
            let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("procedural_boundary_readback"),
                size: state.n_bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            state.submit(&gpu, &readback).unwrap();
            gpu.read_mapped_f32(&readback, state.n_bytes).unwrap()
        };
        // Analogue composes over a canvas table that must fit one storage
        // binding, so its limits start above the table. Digital grain has no
        // table and keeps the small-limit sweep.
        // A near-unit size keeps the canvas at the full virtual resolution,
        // which for this 17x7 frame would exceed the binding limit by itself;
        // production profiles sit near 1.35 and stay well inside it.
        params.raw_scale = 12.0;
        let (cells_w, cells_h) = canvas_cells(width, height, &params);
        let canvas_bytes = u64::from(cells_w) * u64::from(cells_h) * 8;
        for (mode, resolution) in [(0, 0), (0, 1)] {
            params.mode = mode;
            params.resolution_type = resolution;
            assert_eq!(
                run(&params, canvas_bytes),
                run(&params, canvas_bytes + 4096),
                "mode {mode}, resolution {resolution}"
            );
        }
        // A canvas that cannot fit one binding is banded, and a band must
        // compose the same image as a resident canvas.
        params.raw_scale = 1.1;
        params.mode = 0;
        params.resolution_type = 0;
        let (cells_w, cells_h) = canvas_cells(width, height, &params);
        let oversized = u64::from(cells_w) * u64::from(cells_h) * 8;
        assert!(oversized > 128 << 20, "fixture must exceed one binding");
        assert_eq!(
            run(&params, oversized),
            run(&params, 8 << 20),
            "a banded canvas must compose the same image"
        );
        // Only a single canvas row that cannot fit one binding is refused.
        assert!(
            build_bounded_grain_procedural(&params, width, height, &input, None, &gpu, 1024)
                .is_err(),
            "a canvas row wider than the window must be refused"
        );
        params.mode = 1;
        params.resolution_type = 0;
        assert_eq!(
            run(&params, 65536),
            run(&params, 1728),
            "digital grain must not depend on the transport window"
        );
    }
}
#[cfg(test)]
mod parity_tests {
    use super::*;

    // Exercise the production WGSL helper, not a Rust transcription of it.
    #[test]
    fn procedural_coordinate_division_matches_binary32() {
        let gpu = crate::test_support::restricted_adapter();
        let mut pairs: Vec<[f32; 2]> = vec![
            [0.0, 5200.0],
            [1.0, f32::from_bits((171u32 << 23) - 1)],
            [-1.0, f32::from_bits((171u32 << 23) - 1)],
            [f32::from_bits(0x3fffffff), f32::from_bits(0x3ffffffe)],
            [-0.0, 7280.0],
            [1.0, 7280.0],
            [37.0, 7280.0],
            [f32::from_bits(0x3fffffff), 1.0],
            [1.0, f32::from_bits(0x3f800001)],
            [i32::MAX as f32, 2.0f32.powi(43)],
            [-2147483648.0, 1550.0],
        ];
        for extent in [7280.0, 5005.0, 3101.0 * 1.4, u32::MAX as f32 * 5200.0] {
            for position in [0.0, 1.0, 255.0, 5200.0, 1048575.0, 16777215.0] {
                pairs.push([position, extent]);
            }
        }
        for index in 0..257u32 {
            pairs.push([(index * 104729 % 5201) as f32, 4800.0 + index as f32 * 1.4]);
        }
        let mut state = 90210u32;
        for _ in 0..4096 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let a = (state as i32) as f32;
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let b = f32::from_bits(((137 + state % 33) << 23) | (state & 0x7fffff));
            pairs.push([a, b]);
        }
        let mut shader = include_str!("procedural.wgsl").to_owned();
        shader.push_str("\n@compute @workgroup_size(64) fn division_test(@builtin(global_invocation_id) id:vec3<u32>){if(id.x<arrayLength(&canvas_cells)){canvas_cells[id.x]=bitcast<u32>(divide_normal_f32(source_rgb[2u*id.x],source_rgb[2u*id.x+1u]));}}");
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("procedural_division_test"),
                source: wgpu::ShaderSource::Wgsl(shader.into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("procedural_division_test"),
                layout: None,
                module: &module,
                entry_point: Some("division_test"),
                compilation_options: Default::default(),
                cache: None,
            });
        let input = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&pairs),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let bytes = pairs.len() as u64 * 4;
        let output = transport(&gpu, bytes, "division_output").unwrap();
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
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
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups((pairs.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        gpu.queue.submit(Some(encoder.finish()));
        let values = gpu.read_mapped_f32(&readback, bytes).unwrap();
        for (i, (pair, value)) in pairs.iter().zip(values).enumerate() {
            assert_eq!(
                value.to_bits(),
                (pair[0] / pair[1]).to_bits(),
                "pair {i}: {pair:?}"
            );
        }
    }

    #[test]
    fn procedural_planner_rejects_nonfinite_scale_before_weights() {
        let gpu = crate::test_support::adapter();
        let mut params = super::logic_tests::test_params();
        params.mode = 0;
        params.resolution_factor = 0.0;
        params.amount = 1.0;
        for resolution in [0, 1] {
            params.resolution_type = resolution;
            for scale in [f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
                params.raw_scale = scale;
                assert!(gpu.procedural_workspace_bytes(17, 7, &params).is_err());
            }
        }
    }

    // Uninstrumented production canvas entry: keep the compiler context that
    // exposed the (11,50) nested-hash regression. CPU Procedural generator reference,
    // seed42, RGB/luma=.6, virtual sizes5200x5200 and5200x3466. Goldens
    // are half-rounded current CPU Procedural generator outputs at (i%64,i/64).
    // The second case exercises the unclustered vector*rotation*scale chain.
    #[test]
    fn procedural_canvas_hash_sentinels() {
        let gpu = crate::test_support::adapter();
        let mut params = super::logic_tests::test_params();
        params.seed = 42;
        params.timer = None;
        params.raw_scale = 12.0;
        params.cluster_size = 1.6;
        params.rotation = 1.0;
        params.color = 0.65;
        params.colored = true;
        params.clustered = true;
        check_canvas_sentinels(
            &gpu,
            params,
            512,
            512,
            [
                [14333, 14343, 46399],
                [15196, 14518, 11404],
                [15871, 14679, 45977],
                [14275, 12932, 14452],
                [11465, 11770, 15140],
            ],
        );
        params.rotation = 0.7;
        params.clustered = false;
        check_canvas_sentinels(
            &gpu,
            params,
            384,
            256,
            [
                [15005, 14418, 15785],
                [14286, 43709, 15024],
                [14292, 14426, 14899],
                [44485, 46450, 14098],
                [48416, 46571, 45389],
            ],
        );
    }

    fn check_canvas_sentinels(
        gpu: &WgpuBackend,
        params: spektrafilm_backend::ProceduralGrainGpuParams,
        width: u32,
        height: u32,
        expected: [[u32; 3]; 5],
    ) {
        let mut packed = super::super::shader_params(width, height, &params);
        packed.window = [0, 4096, 0, height];
        packed.canvas = [64, 64, 0, 0];
        let source = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&vec![0.6f32; (width * height * 3) as usize]),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let buffer = transport(gpu, 4096 * 8, "canvas_sentinels").unwrap();
        let uniform = uniform(gpu, &packed);
        let pipeline = gpu.cached_pipeline_at(
            include_str!("procedural.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            "canvas_main",
        );
        let bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: source.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4096 * 8,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(16, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&buffer, 0, &readback, 0, 4096 * 8);
        gpu.queue.submit(Some(encoder.finish()));
        let words: Vec<_> = gpu
            .read_mapped_f32(&readback, 4096 * 8)
            .unwrap()
            .into_iter()
            .map(f32::to_bits)
            .collect();
        for (i, rgb) in [0, 37, 563, 3211, 4095].into_iter().zip(expected) {
            assert_eq!(
                [
                    words[2 * i] & 65535,
                    words[2 * i] >> 16,
                    words[2 * i + 1] & 65535
                ],
                rgb,
                "cell {i}"
            );
            assert_eq!(words[2 * i + 1] >> 16, 0);
        }
    }
}
