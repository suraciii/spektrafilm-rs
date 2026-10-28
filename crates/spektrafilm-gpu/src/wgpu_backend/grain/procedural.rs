use super::raster::{RasterCommands, paged_transport, transport, uniform};
use super::*;

/// The transport geometry is always the original film geometry. Storage
/// windows only transport samples and never redefine film coordinates.
pub(in crate::wgpu_backend) struct GrainV2State {
    output: Vec<super::super::paged::ImagePage>,
    commands: RasterCommands,
    pub(super) n_bytes: u64,
}

pub(in crate::wgpu_backend) fn build_grain_v2_state(
    _device: &ObservedDevice,
    params: &spektrafilm_backend::GrainV2GpuParams,
    width: u32,
    height: u32,
    input: &(impl RasterStorage + ?Sized),
    output_space: Option<&spektrafilm_math::colorspace::RgbColorSpace>,
    backend: &WgpuBackend,
) -> Result<GrainV2State, String> {
    build_bounded_grain_v2(
        params,
        width,
        height,
        input,
        output_space,
        backend,
        super::super::bounded::storage_binding_limit(backend)?,
    )
}

fn build_bounded_grain_v2(
    params: &spektrafilm_backend::GrainV2GpuParams,
    width: u32,
    height: u32,
    input: &(impl RasterStorage + ?Sized),
    output_space: Option<&spektrafilm_math::colorspace::RgbColorSpace>,
    backend: &WgpuBackend,
    limit: u64,
) -> Result<GrainV2State, String> {
    let n = width
        .checked_mul(height)
        .filter(|&n| n > 0 && n <= u32::MAX / 3)
        .ok_or("procedural grain dimensions overflow")?;
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
        commands.dispatch(
            &filter,
            &[&p, &source_window, &dummy_weights, &target_window],
            count,
            false,
            backend,
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
    let canvas_pass =
        backend.cached_pipeline_at(include_str!("grain_v2.wgsl"), &canvas_layout, "canvas_main");
    let grain = backend.cached_pipeline_at(
        include_str!("grain_v2.wgsl"),
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
    let scratch_canvas = transport(backend, band_bytes.max(16), "grain_procedural_band")?;
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
                        0,
                        0,
                    ],
                );
                commands.dispatch(&mean_pass, &[&p, buffer, &partials], count, false, backend);
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
                0,
                0,
            ],
        );
        commands.dispatch(&reduce_pass, &[&p, &scratch, &partials], 1, true, backend);
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
        commands.dispatch(
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
    Ok(GrainV2State {
        output,
        commands,
        n_bytes,
    })
}

fn resolution_weights(
    width: u32,
    height: u32,
    params: &spektrafilm_backend::GrainV2GpuParams,
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
        params: &spektrafilm_backend::GrainV2GpuParams,
    ) -> Result<u64, String> {
        let n = width
            .checked_mul(height)
            .filter(|&n| n > 0 && n <= u32::MAX / 3)
            .ok_or("procedural grain dimensions overflow")?;
        let limit = super::super::bounded::storage_binding_limit(self)?;
        let page = (limit / 12).min(u64::from(n));
        let (radius, optical, weights) = resolution_weights(width, height, params);
        let pages = u64::from(n).div_ceil(page);
        let mut bytes = u128::from(n) * 48 + u128::from(page) * 24 + 8;
        bytes += u128::from(pages) * 96 + u128::from(pages) * 48;
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

/// Canvas cells the scan addresses, matching the host model's table.
fn canvas_cells(
    width: u32,
    height: u32,
    params: &spektrafilm_backend::GrainV2GpuParams,
) -> (u32, u32) {
    let gsf = (5200.0 / width as f32).max(3100.0 / height as f32);
    let size = [
        (width as f32 * gsf).floor().max(1.0),
        (height as f32 * gsf).floor().max(1.0),
    ];
    let scale = 1.0 + (params.raw_scale - 1.0) / 47.0 * 1.5;
    let used = |extent: f32, limit: f32| {
        ((extent / scale).ceil() as i64 + 2).clamp(1, limit.max(1.0) as i64) as u32
    };
    (used(size[0], size[0]), used(size[1], size[1]))
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
    params: &spektrafilm_backend::GrainV2GpuParams,
) -> Result<Vec<CanvasBand>, String> {
    let row_bytes = u64::from(cells_w) * 8;
    if row_bytes > limit {
        return Err(format!(
            "procedural grain canvas row needs {row_bytes} bytes; device binding limit is {limit}"
        ));
    }
    let rows = u64::from(cells_h);
    // Mirror the composition's cell mapping in binary32 so a band owns exactly
    // the pixels the shader resolves into it.
    let gsf = (5200.0f32 / width as f32).max(3100.0f32 / height as f32);
    let size_y = (height as f32 * gsf).floor().max(1.0);
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
    params: &spektrafilm_backend::GrainV2GpuParams,
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
    let canvas_rows =
        ((height as f32 * (5200.0 / width as f32).max(3100.0 / height as f32)).floor()).max(1.0);
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
        for source_row in first_row..last_row {
            commands.copy(
                original,
                u64::from(source_row * width) * 12,
                source_window,
                u64::from((source_row - first_row) * width) * 12,
                u64::from(width) * 12,
            );
        }
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
        commands.dispatch(
            canvas_pass,
            &[&p, source_window, buffer],
            chunk * cells_w,
            false,
            backend,
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
                    for row in low..high {
                        commands.copy(
                            source,
                            u64::from(row * width) * 12,
                            source_window,
                            u64::from((row - low) * width) * 12,
                            u64::from(width) * 12,
                        );
                    }
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
                    commands.dispatch(
                        &pipe,
                        &[&p, source_window, &weights_buf, &result],
                        rows * width,
                        false,
                        backend,
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
                        commands.dispatch(
                            &pipe,
                            &[&p, source_window, &weights_buf, &result],
                            count,
                            false,
                            backend,
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
                    commands.dispatch(
                        &pipe,
                        &[&p, source_window, &weights_buf, &result],
                        count,
                        false,
                        backend,
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
                    commands.dispatch(
                        &pipe,
                        &[&p, source_window, &weights_buf, &samples, &result],
                        count,
                        false,
                        backend,
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
                commands.dispatch(
                    &pipe,
                    &[&p, source_window, &weights_buf, &samples, &result],
                    1,
                    false,
                    backend,
                );
            }
            commands.copy(&result, 0, destination, u64::from(pixel) * 12, 12);
        }
    }
    Ok(())
}

impl GrainV2State {
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
mod tests {
    use super::*;

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
        let params = spektrafilm_backend::GrainV2GpuParams {
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
        let mut params = spektrafilm_backend::GrainV2GpuParams {
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
        let run = |params: &spektrafilm_backend::GrainV2GpuParams, limit| {
            let state =
                build_bounded_grain_v2(params, width, height, &input, None, &gpu, limit).unwrap();
            let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("procedural_boundary_readback"),
                size: state.n_bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            state.submit(&gpu, &readback).unwrap();
            gpu.read_mapped_f32(&readback, state.n_bytes)
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
            build_bounded_grain_v2(&params, width, height, &input, None, &gpu, 1024).is_err(),
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
