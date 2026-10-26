use super::bounded::{chunk_elements, storage_binding_limit};
use super::page_views::StorageView;
use super::*;
use spektrafilm_backend::{EnlargerSpectralLutGpuParams, ScannerSpectralLutGpuParams};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Generate {
    size: u32,
    n: u32,
    n_wl: u32,
    flags: u32,
    origin: [u32; 4],
    extent: [u32; 4],
    pad: [u32; 4],
    minimum: [f64; 4],
    maximum: [f64; 4],
    preflash: [f64; 4],
    normalization: [f64; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Prepare {
    size: u32,
    n: u32,
    pad: [u32; 2],
    origin: [u32; 4],
    extent: [u32; 4],
    source_origin: [u32; 4],
    source_extent: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Interpolate {
    n: u32,
    size: u32,
    size_m1: u32,
    scanner: u32,
    pad: [u32; 4],
    minimum: [f64; 4],
    scale: [f64; 4],
    origin: [u32; 4],
    extent: [u32; 4],
    cat: [[f64; 4]; 3],
    rgb: [[f64; 4]; 3],
}

#[derive(Clone, Copy)]
struct ScannerMatrices<'a> {
    cat: &'a [[f64; 3]; 3],
    xyz_to_rgb: &'a [[f64; 3]; 3],
}

impl WgpuBackend {
    pub(super) fn enlarger_resident(
        &self,
        image: &ImageBuf,
        p: &EnlargerSpectralLutGpuParams<'_>,
    ) -> Result<ImageBuf, String> {
        self.spectral_lut_host(image, |source, output, n| {
            self.execute_enlarger_lut_resident(source, output, n, p)
        })
    }

    pub(super) fn scanner_resident(
        &self,
        image: &ImageBuf,
        p: &ScannerSpectralLutGpuParams<'_>,
    ) -> Result<ImageBuf, String> {
        self.spectral_lut_host(image, |source, output, n| {
            self.execute_scanner_lut_resident(source, output, n, p)
        })
    }

    fn spectral_lut_host(
        &self,
        image: &ImageBuf,
        execute: impl Fn(&StorageView, &StorageView, u32) -> Result<(), String>,
    ) -> Result<ImageBuf, String> {
        let pixels = chunk_elements(storage_binding_limit(self)?.min(8 * 1024 * 1024), 12)?;
        let mut output = Vec::with_capacity(image.data.len());
        for samples in image.data.chunks(pixels * 3) {
            let input = scalars_to_f32(samples);
            let n = (input.len() / 3) as u32;
            let input = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("spectral_lut_input"),
                    contents: bytemuck::cast_slice(&input),
                    usage: wgpu::BufferUsages::STORAGE,
                });
            let result = self.enlarger_scratch(u64::from(n) * 12, "spectral_lut_output");
            execute(
                &StorageView::entire(&input),
                &StorageView::entire(&result),
                n,
            )?;
            output.extend(f32_to_scalars(
                self.pipe.readback_buffer(&result, u64::from(n) * 12),
            ));
        }
        self.device.context.record_executor(Executor::Gpu, None);
        Ok(ImageBuf::from_data(image.width, image.height, output))
    }

    /// Submit bounded generation/preparation/interpolation bricks. The output
    /// view survives every sparse brick dispatch; aliases are adapted upstream.
    pub(super) fn execute_enlarger_lut_resident(
        &self,
        source: &StorageView,
        output: &StorageView,
        n: u32,
        p: &EnlargerSpectralLutGpuParams<'_>,
    ) -> Result<(), String> {
        self.execute_spectral_lut(source, output, n, p, None)
    }

    /// Same retained contract as the enlarger, followed by 10^log_xyz, CAT,
    /// then XYZ-to-RGB, in that order (the matrices are not precombined).
    pub(super) fn execute_scanner_lut_resident(
        &self,
        source: &StorageView,
        output: &StorageView,
        n: u32,
        p: &ScannerSpectralLutGpuParams<'_>,
    ) -> Result<(), String> {
        let cmfs: [[f64; 3]; spektrafilm_math::spectral::N_WAVELENGTHS] =
            std::array::from_fn(|i| {
                [
                    spektrafilm_math::spectral::CMF_X_F64[i],
                    spektrafilm_math::spectral::CMF_Y_F64[i],
                    spektrafilm_math::spectral::CMF_Z_F64[i],
                ]
            });
        let spectral = EnlargerSpectralLutGpuParams {
            channel_density: p.channel_density,
            base_density: p.base_density,
            illuminant: p.illuminant,
            sensitivity: &cmfs,
            normalization: 1.0 / p.normalization,
            preflash: [0.0; 3],
            minimum: p.minimum,
            maximum: p.maximum,
            size: p.size,
        };
        self.execute_spectral_lut(
            source,
            output,
            n,
            &spectral,
            Some(ScannerMatrices {
                cat: p.cat,
                xyz_to_rgb: p.xyz_to_rgb,
            }),
        )
    }

    fn execute_spectral_lut(
        &self,
        input: &StorageView,
        output: &StorageView,
        n: u32,
        p: &EnlargerSpectralLutGpuParams<'_>,
        scanner: Option<ScannerMatrices<'_>>,
    ) -> Result<(), String> {
        if !self.device.features().contains(wgpu::Features::SHADER_F64) {
            return Err("spectral LUT preparation requires device f64".to_owned());
        }
        let enlarger = scanner.is_none();
        let size = p.size;
        if size < 2
            || u64::from(size)
                .checked_pow(3)
                .is_none_or(|nodes| nodes > u64::from(u32::MAX) / 3)
        {
            return Err("spectral LUT size is not device indexable".to_owned());
        }
        let limit = storage_binding_limit(self)?;
        let bytes = u64::from(n) * 12;
        if input.buffer == output.buffer
            || input.size > limit
            || output.size > limit
            || input.size < u64::from(input.base) * 4 + bytes
            || output.size < u64::from(output.base) * 4 + bytes
            || input
                .offset
                .checked_add(input.size)
                .is_none_or(|end| end > input.buffer.size())
            || output
                .offset
                .checked_add(output.size)
                .is_none_or(|end| end > output.buffer.size())
            || !input.buffer.usage().contains(wgpu::BufferUsages::STORAGE)
            || !output.buffer.usage().contains(wgpu::BufferUsages::STORAGE)
        {
            return Err(format!(
                "spectral LUT image chunk needs {bytes} bytes per binding; limit {limit}"
            ));
        }
        if n == 0 {
            return Ok(());
        }
        // Each source halo retains binary64 log samples. Endpoint slopes use
        // three samples; two neighbours each side cover every interior brick.
        let workspace_limit = limit.min(super::resources::WORKSPACE_BINDING_BYTES / 8);
        let minimum_source = u64::from(size.min(6)).pow(3) * 24;
        if minimum_source > workspace_limit {
            return Err(format!(
                "spectral LUT minimum halo working set needs {minimum_source} bytes per binding; limit {limit}"
            ));
        }
        let mut edge = 2u32;
        while edge < size && u64::from((edge + 5).min(size)).pow(3) * 24 <= workspace_limit {
            edge += 1;
        }
        let n_wl = p
            .channel_density
            .len()
            .min(p.illuminant.len())
            .min(p.sensitivity.len());
        let coefficient = |values: &[f64]| -> Result<wgpu::Buffer, String> {
            if std::mem::size_of_val(values) as u64 > limit {
                return Err("spectral LUT coefficient exceeds bounded binding".to_owned());
            }
            let zero = [0.0f64];
            Ok(self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("spectral_lut_coefficients"),
                    contents: bytemuck::cast_slice(if values.is_empty() { &zero } else { values }),
                    usage: wgpu::BufferUsages::STORAGE,
                }))
        };
        // The reference density-to-light stage replaces missing-row NaN light
        // with zero before projection. Do not send those NaNs into driver
        // floating-point control flow or the log range-reduction loops.
        let has_base = p.base_density.len() >= n_wl;
        let active: Vec<_> = (0..n_wl)
            .filter(|&i| {
                !p.channel_density[i].iter().any(|v| v.is_nan())
                    && (!has_base || !p.base_density[i].is_nan())
                    && !p.illuminant[i].is_nan()
            })
            .collect();
        let cd: Vec<f64> = active.iter().flat_map(|&i| p.channel_density[i]).collect();
        let sensitivity: Vec<f64> = active.iter().flat_map(|&i| p.sensitivity[i]).collect();
        let base: Vec<f64> = active
            .iter()
            .map(|&i| if has_base { p.base_density[i] } else { 0.0 })
            .collect();
        let lamp: Vec<f64> = active.iter().map(|&i| p.illuminant[i]).collect();
        let n_wl = active.len();
        let cd = coefficient(&cd)?;
        let bd = coefficient(&base)?;
        let illuminant = coefficient(&lamp)?;
        let sensitivity = coefficient(&sensitivity)?;
        let read = wgpu::BufferBindingType::Storage { read_only: true };
        let write = wgpu::BufferBindingType::Storage { read_only: false };
        let uniform = wgpu::BufferBindingType::Uniform;
        let generation = self.cached_pipeline(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                include_str!("enlarger_generate.wgsl")
            ),
            &[uniform, read, read, read, read, write],
        );
        let preparation = self.cached_pipeline(
            include_str!("enlarger_prepare.wgsl"),
            &[uniform, read, write, write, write, write, write, write],
        );
        let interpolation = self.cached_pointwise_pipeline(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/spectral_lut_f64.wgsl")
            ),
            &[uniform, read, read, read, read, read, read, read, write],
            &[(1, "input"), (8, "output")],
        );
        let mut bases = [0u32; 16];
        bases[1] = input.base;
        bases[8] = output.base;
        let bases = self.enlarger_uniform(&bases);
        for x in (0..size - 1).step_by((edge - 1) as usize) {
            for y in (0..size - 1).step_by((edge - 1) as usize) {
                for z in (0..size - 1).step_by((edge - 1) as usize) {
                    self.check_cancelled()?;
                    let origin = [x, y, z, 0];
                    let extent = [
                        edge.min(size - x),
                        edge.min(size - y),
                        edge.min(size - z),
                        0,
                    ];
                    let so = [
                        x.saturating_sub(2),
                        y.saturating_sub(2),
                        z.saturating_sub(2),
                        0,
                    ];
                    let se = [
                        (x + extent[0] + 2).min(size) - so[0],
                        (y + extent[1] + 2).min(size) - so[1],
                        (z + extent[2] + 2).min(size) - so[2],
                        0,
                    ];
                    let source_n = se[0] * se[1] * se[2];
                    let nodes = extent[0] * extent[1] * extent[2];
                    let cells = (extent[0] - 1) * (extent[1] - 1) * (extent[2] - 1);
                    let source = self
                        .enlarger_scratch(u64::from(source_n) * 24, "spectral_lut_sampled_halo");
                    let lut = self.enlarger_scratch(u64::from(nodes) * 24, "spectral_lut_lattice");
                    let sx = self.enlarger_scratch(u64::from(nodes) * 24, "spectral_lut_slope_x");
                    let sy = self.enlarger_scratch(u64::from(nodes) * 24, "spectral_lut_slope_y");
                    let sz = self.enlarger_scratch(u64::from(nodes) * 24, "spectral_lut_slope_z");
                    let lo = self.enlarger_scratch(u64::from(cells) * 24, "spectral_lut_cell_min");
                    let hi = self.enlarger_scratch(u64::from(cells) * 24, "spectral_lut_cell_max");
                    let row = |v: [f64; 3]| [v[0], v[1], v[2], 0.0];
                    // The CPU grid uses Scalar, while scanner log_xyz stays f64 and
                    // enlarger log_raw crosses an ImageBuf boundary before preparation.
                    let narrow = u32::from(!cfg!(feature = "precision-f64"));
                    let g = self.enlarger_uniform(&Generate {
                        size,
                        n: source_n,
                        n_wl: n_wl as u32,
                        flags: narrow | if enlarger { narrow * 2 } else { 0 },
                        origin: so,
                        extent: se,
                        pad: [0; 4],
                        minimum: row(p.minimum),
                        maximum: row(p.maximum),
                        preflash: row(p.preflash),
                        normalization: [p.normalization, 0.0, 0.0, 0.0],
                    });
                    let prep = self.enlarger_uniform(&Prepare {
                        size,
                        n: nodes,
                        pad: [0; 2],
                        origin,
                        extent,
                        source_origin: so,
                        source_extent: se,
                    });
                    let (cat, rgb) = scanner.map_or(([[0.0; 4]; 3], [[0.0; 4]; 3]), |matrices| {
                        (matrices.cat.map(row), matrices.xyz_to_rgb.map(row))
                    });
                    let ip = self.enlarger_uniform(&Interpolate {
                        n,
                        size,
                        size_m1: size - 1,
                        scanner: u32::from(scanner.is_some()),
                        pad: [0; 4],
                        minimum: row(p.minimum),
                        scale: row(std::array::from_fn(|c| {
                            (size - 1) as f64 / (p.maximum[c] - p.minimum[c])
                        })),
                        origin,
                        extent,
                        cat,
                        rgb,
                    });
                    let mut encoder = self.device.create_command_encoder(&Default::default());
                    self.enlarger_pass(
                        &mut encoder,
                        &generation,
                        &[&g, &cd, &bd, &illuminant, &sensitivity, &source],
                        source_n,
                        "spectral_lut_generate",
                    );
                    self.enlarger_pass(
                        &mut encoder,
                        &preparation,
                        &[&prep, &source, &lut, &sx, &sy, &sz, &lo, &hi],
                        nodes,
                        "spectral_lut_prepare",
                    );
                    let buffers = [&ip, &lut, &sx, &sy, &sz, &lo, &hi];
                    let entries = [
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffers[0].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: input.binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: buffers[1].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: buffers[2].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: buffers[3].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: buffers[4].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: buffers[5].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: buffers[6].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 8,
                            resource: output.binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 9,
                            resource: bases.as_entire_binding(),
                        },
                    ];
                    let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("spectral_lut_interpolate"),
                        layout: &interpolation.layout,
                        entries: &entries,
                    });
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("spectral_lut_interpolate"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(&interpolation.pipeline);
                        pass.set_bind_group(0, &group, &[]);
                        dispatch_linear(&mut pass, n);
                    }
                    self.queue.submit(Some(encoder.finish()));
                }
            }
        }
        Ok(())
    }

    fn enlarger_scratch(&self, bytes: u64, label: &str) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    fn enlarger_uniform<P: bytemuck::Pod>(&self, p: &P) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("spectral_lut_params"),
                contents: bytemuck::bytes_of(p),
                usage: wgpu::BufferUsages::UNIFORM,
            })
    }
    fn enlarger_pass(
        &self,
        encoder: &mut ObservedEncoder,
        pipeline: &CachedPipelineRef,
        buffers: &[&wgpu::Buffer],
        n: u32,
        label: &str,
    ) {
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &pipeline.layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &group, &[]);
        dispatch_linear(&mut pass, n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectral_lut_uniforms_match_wgsl_alignment() {
        // vec4<f64> aligns to 32, unlike Rust's [f64;4]. These offsets
        // catch silent device reads from the preceding extent/padding.
        assert_eq!(std::mem::offset_of!(Generate, minimum), 64);
        assert_eq!(std::mem::offset_of!(Generate, maximum), 96);
        assert_eq!(std::mem::offset_of!(Generate, preflash), 128);
        assert_eq!(std::mem::offset_of!(Generate, normalization), 160);
        assert_eq!(std::mem::size_of::<Generate>(), 192);
        assert_eq!(std::mem::offset_of!(Interpolate, minimum), 32);
        assert_eq!(std::mem::offset_of!(Interpolate, scale), 64);
        assert_eq!(std::mem::offset_of!(Interpolate, cat), 128);
        assert_eq!(std::mem::offset_of!(Interpolate, rgb), 224);
        assert_eq!(std::mem::size_of::<Interpolate>(), 320);
    }

    #[test]
    fn spectral_transport_preserves_all_brick_owners_and_sentinels() {
        let backend = crate::test_support::adapter();
        if !backend
            .device
            .features()
            .contains(wgpu::Features::SHADER_F64)
        {
            return;
        }
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
        let p = EnlargerSpectralLutGpuParams {
            channel_density: &[[0.1, 0.2, 0.3]],
            base_density: &[0.02],
            illuminant: &[1.0],
            sensitivity: &[[1.0, 1.2, 0.8]],
            normalization: 1.0,
            preflash: [0.0; 3],
            minimum: [0.0; 3],
            maximum: [1.0; 3],
            size: 8,
        };
        // The first and last pixels belong to widely separated lattice bricks.
        let values = [0.0f32, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0];
        let run = |limit: &str, storage: bool| {
            unsafe {
                std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", limit);
            }
            let mut usage = wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
            if storage {
                usage |= wgpu::BufferUsages::STORAGE;
            }
            let source = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("lut_brick_source"),
                    contents: bytemuck::cast_slice(&values),
                    usage,
                });
            let sentinel = [-777.0f32; 12];
            let output = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("lut_brick_output"),
                    contents: bytemuck::cast_slice(&sentinel),
                    usage,
                });
            let output_pages = vec![super::super::paged::ImagePage {
                first_pixel: 0,
                n_pixels: 3,
                buffer: output.clone(),
            }];
            backend
                .spectral_lut_transport(&source, &output_pages, 3, |source, output, n| {
                    backend.execute_enlarger_lut_resident(source, output, n, &p)
                })
                .unwrap();
            let result = backend.pipe.readback_buffer(&output, output.size());
            assert_eq!(&result[9..], &sentinel[9..]);
            assert!(
                result[..9]
                    .iter()
                    .all(|value| value.is_finite() && *value != -777.0)
            );
            result[..9]
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        };
        let one_brick = run("20000", true);
        // At 10000 bytes the eight-node lattice needs multiple halo bricks.
        assert_eq!(run("10000", true), one_brick);
        assert_eq!(run("10000", false), one_brick);
    }
}
