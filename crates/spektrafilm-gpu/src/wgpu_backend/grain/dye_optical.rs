use super::*;
use spektrafilm_backend::DyeResponseGpuParams;

use crate::wgpu_backend::film_chain::device_image::{BufferPayload, ImagePage};
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ResponseUniform {
    dims: [u32; 4],
    controls: [f32; 4],
    preflash: [f32; 4],
    mins: [f32; 4],
    matrix: [[f32; 4]; 3],
}

pub(super) const RESPONSE_PARAMS_BYTES: u64 = std::mem::size_of::<ResponseUniform>() as u64;

pub(super) struct ResponseSetup {
    uniform: ResponseUniform,
    spectrum: wgpu::Buffer,
}

pub(in crate::wgpu_backend) struct OpticalPass {
    pipeline: CachedPipelineRef,
    group: wgpu::BindGroup,
    items: u32,
}

impl OpticalPass {
    pub(in crate::wgpu_backend) fn encode(&self, encoder: &mut ObservedEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("dye_optical"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        dispatch_linear(&mut pass, self.items);
    }
}

fn group(
    backend: &WgpuBackend,
    pipeline: &CachedPipelineRef,
    buffers: &[&wgpu::Buffer],
) -> wgpu::BindGroup {
    let entries: Vec<_> = buffers
        .iter()
        .enumerate()
        .map(|(i, buffer)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    backend
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_optical"),
            layout: &pipeline.layout,
            entries: &entries,
        })
}

impl ResponseSetup {
    pub(super) fn new(
        backend: &WgpuBackend,
        params: &DyeGpuParams,
        response: &DyeResponseGpuParams<'_>,
    ) -> Result<Self, String> {
        let mut uniform = ResponseUniform {
            dims: [
                0,
                0,
                0,
                if matches!(params.law, DyeFieldLaw::ScalarRecord { .. }) {
                    2
                } else {
                    0
                },
            ],
            controls: [0.0; 4],
            preflash: [0.0; 4],
            mins: match &params.law {
                DyeFieldLaw::Continuous => [0.0, 0.0, 0.0, 0.0],
                DyeFieldLaw::Capacity(tables) => [
                    tables.tables[0].min,
                    tables.tables[1].min,
                    tables.tables[2].min,
                    0.0,
                ],
                DyeFieldLaw::ScalarRecord { .. } => [0.0; 4],
            },
            matrix: [[0.0; 4]; 3],
        };
        let scan_weights: [[f64; 3]; spektrafilm_math::spectral::N_WAVELENGTHS];
        let (cd, bd, illuminant, weights): (_, _, _, &[[f64; 3]]) = match response {
            DyeResponseGpuParams::Print {
                channel_density,
                base_density,
                illuminant,
                sensitivity,
                normalization_factor,
                exposure_scale,
                preflash,
            } => {
                uniform.controls = [
                    *normalization_factor as f32,
                    *exposure_scale as f32,
                    0.0,
                    0.0,
                ];
                uniform.preflash[..3].copy_from_slice(&preflash.map(|v| v as f32));
                (*channel_density, *base_density, *illuminant, *sensitivity)
            }
            DyeResponseGpuParams::Scan {
                channel_density,
                base_density,
                illuminant,
                normalization,
                xyz_to_rgb,
            } => {
                uniform.dims[2] = 1;
                uniform.controls[0] = *normalization as f32;
                for (column, values) in uniform.matrix.iter_mut().enumerate() {
                    for row in 0..3 {
                        values[row] = xyz_to_rgb[row][column] as f32;
                    }
                }
                scan_weights = std::array::from_fn(|wl| {
                    [
                        f64::from(spektrafilm_math::spectral::CMF_X[wl]),
                        f64::from(spektrafilm_math::spectral::CMF_Y[wl]),
                        f64::from(spektrafilm_math::spectral::CMF_Z[wl]),
                    ]
                });
                (
                    *channel_density,
                    *base_density,
                    *illuminant,
                    scan_weights.as_slice(),
                )
            }
        };
        let n = cd.len().min(illuminant.len()).min(weights.len());
        let scalar_record = matches!(params.law, DyeFieldLaw::ScalarRecord { .. });
        if n == 0 || (!scalar_record && !bd.is_empty() && bd.len() < n) {
            return Err(
                "film grain Dye: optical response has inconsistent wavelength tables".into(),
            );
        }
        uniform.dims[1] = n as u32;
        if uniform
            .controls
            .iter()
            .chain(&uniform.preflash)
            .chain(uniform.matrix.iter().flatten())
            .any(|v| !v.is_finite())
            || (uniform.dims[2] == 1 && uniform.controls[0] <= 0.0)
        {
            return Err(
                "film grain Dye: optical response controls are not representable in f32".into(),
            );
        }
        if uniform.dims[2] == 1
            && matches!(params.law, DyeFieldLaw::ScalarRecord { .. })
            && let Some(coefficients) =
                spektrafilm_backend::scalar_scan::ScalarScanCoefficients::prepare(
                    cd, bd, illuminant, weights,
                )
        {
            let values = [
                coefficients.coefficient,
                coefficients.xyz[0],
                coefficients.xyz[1],
                coefficients.xyz[2],
            ];
            let packed = values.map(|v| v as f32);
            let representable = values
                .iter()
                .zip(packed)
                .all(|(&value, packed)| packed.is_finite() && (value == 0.0 || packed.is_normal()));
            let bounds = coefficients.exponent_bounds(
                f64::from(f32::MIN_POSITIVE).log10() + 1.0,
                f64::from(f32::MAX).log10() - 1.0,
            );
            let lower = (bounds[0] as f32).next_up();
            let upper = (bounds[1] as f32).next_down();
            // The f64 helper proves algebraic separation. Also exclude
            // f32 spectral inputs/base powers that lose normal precision
            // before an otherwise safe exponent could rescale them.
            let normal_spectrum = (0..n).all(|wl| {
                let base = if bd.len() >= n { bd[wl] } else { 0.0 };
                if cd[wl].iter().any(|v| v.is_nan()) || base.is_nan() || illuminant[wl].is_nan() {
                    return true;
                }
                let lamp = illuminant[wl] as f32;
                (10.0f64.powf(-base) as f32).is_normal()
                    && (illuminant[wl] == 0.0 || lamp.is_normal())
            });
            if representable
                && normal_spectrum
                && lower.is_finite()
                && upper.is_finite()
                && lower <= upper
            {
                uniform.dims[3] |= 1;
                uniform.preflash = packed;
                uniform.controls[2] = lower;
                uniform.controls[3] = upper;
            }
        }
        let mut spectrum = Vec::with_capacity(n * 8);
        for wl in 0..n {
            // CPU integration maps a NaN spectral transmission to zero:
            // any missing dye/base coefficient removes the entire wavelength,
            // rather than making the absent absorber transparent.
            let base = if scalar_record && bd.len() < n {
                0.0
            } else {
                bd.get(wl).copied().unwrap_or(0.0)
            };
            let missing =
                cd[wl].iter().any(|v| v.is_nan()) || base.is_nan() || illuminant[wl].is_nan();
            if scalar_record && missing {
                spectrum.extend([0.0_f32; 5]);
                spectrum.extend(weights[wl].map(|v| v as f32));
                continue;
            }
            let sanitize = |v: f64| if v.is_nan() { 0.0 } else { v as f32 };
            spectrum.extend(cd[wl].map(sanitize));
            spectrum.push(sanitize(base));
            spectrum.push(if missing { 0.0 } else { illuminant[wl] as f32 });
            spectrum.extend(weights[wl].map(|v| v as f32));
        }
        let bytes = spectrum.len() as u64 * 4;
        let limits = backend.device.raw.limits();
        if spectrum.iter().any(|v| !v.is_finite())
            || bytes
                > limits
                    .max_buffer_size
                    .min(u64::from(limits.max_storage_buffer_binding_size))
        {
            return Err(
                "film grain Dye: optical response spectrum exceeds device representation limits"
                    .into(),
            );
        }
        Ok(Self {
            uniform,
            spectrum: backend.pipe.static_buffer(
                "dye_response_spectrum",
                bytemuck::cast_slice(&spectrum),
                wgpu::BufferUsages::STORAGE,
            ),
        })
    }

    #[cfg(test)]
    pub(super) fn uses_scalar_scan(&self) -> bool {
        self.uniform.dims[3] & 1 != 0
    }

    pub(super) fn build(
        &self,
        backend: &WgpuBackend,
        band: &DyeBand,
        formed: &wgpu::Buffer,
        linear: &wgpu::Buffer,
        status: &wgpu::Buffer,
    ) -> OpticalPass {
        let mut uniform = self.uniform;
        uniform.dims[0] = band.cols * band.rows;
        let params = backend.pipe.static_buffer(
            "dye_cell_response_params",
            bytemuck::bytes_of(&uniform),
            wgpu::BufferUsages::UNIFORM,
        );
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let pipeline = backend.cached_pipeline(
            include_str!("dye_cell_response.wgsl"),
            &[wgpu::BufferBindingType::Uniform, ro, ro, rw, rw],
        );
        OpticalPass {
            group: group(
                backend,
                &pipeline,
                &[&params, formed, &self.spectrum, linear, status],
            ),
            pipeline,
            items: uniform.dims[0],
        }
    }
}

impl WgpuBackend {
    pub(in crate::wgpu_backend) fn prepare_dye_aperture_collapse(
        &self,
        width: u32,
        height: u32,
        input: &wgpu::Buffer,
        factor: u32,
    ) -> Result<(OpticalPass, wgpu::Buffer, u32, u32), String> {
        if factor == 0 || !width.is_multiple_of(factor) || !height.is_multiple_of(factor) {
            return Err(
                "film grain Dye aperture: fine image dimensions must be divisible by the factor"
                    .into(),
            );
        }
        let out_width = width / factor;
        let out_height = height / factor;
        let bytes = u64::from(out_width) * u64::from(out_height) * 12;
        let limits = self.device.raw.limits();
        if bytes == 0
            || bytes
                > limits
                    .max_buffer_size
                    .min(u64::from(limits.max_storage_buffer_binding_size))
        {
            return Err(
                "film grain Dye aperture: collapsed output exceeds device storage limit".into(),
            );
        }
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_aperture_output"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let params = self.pipe.static_buffer(
            "dye_aperture_params",
            bytemuck::cast_slice(&[width, out_width, out_height, factor]),
            wgpu::BufferUsages::UNIFORM,
        );
        let pipeline = self.cached_pipeline(
            include_str!("dye_aperture_collapse.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let pass = OpticalPass {
            group: group(self, &pipeline, &[&params, input, &output]),
            pipeline,
            items: out_width * out_height,
        };
        Ok((pass, output, out_width, out_height))
    }

    pub fn dye_aperture_collapse(&self, image: &ImageBuf, factor: u32) -> Result<ImageBuf, String> {
        let input = self.upload_dye_optical_pages(image)?;
        let (pages, width, height) = self.dye_aperture_collapse_pages(&input, factor)?;
        self.materialize_dye_optical_pages(pages, width, height)
    }

    pub(in crate::wgpu_backend) fn prepare_dye_log10(
        &self,
        width: u32,
        height: u32,
        input: &wgpu::Buffer,
    ) -> Result<(OpticalPass, wgpu::Buffer, u32, u32), String> {
        if width == 0 || height == 0 {
            return Err("film grain Dye: logarithm raster dimensions must be positive".into());
        }
        let bytes = u64::from(width) * u64::from(height) * 12;
        let limits = self.device.raw.limits();
        if bytes
            > limits
                .max_buffer_size
                .min(u64::from(limits.max_storage_buffer_binding_size))
        {
            return Err(
                "film grain Dye: optical utility output exceeds device storage limit".into(),
            );
        }
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_optical_output"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let words = [width, width, height, 0, 0, 0, 0, 0];
        let params = self.pipe.static_buffer(
            "dye_optical_utility_params",
            bytemuck::cast_slice(&words),
            wgpu::BufferUsages::UNIFORM,
        );
        let pipeline = self.cached_pipeline(
            include_str!("dye_optical_utility.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let pass = OpticalPass {
            group: group(self, &pipeline, &[&params, input, &output]),
            pipeline,
            items: width * height,
        };
        Ok((pass, output, width, height))
    }

    fn materialize_dye_utility(
        &self,
        image: &ImageBuf,
        rect: [u32; 4],
        logarithm: bool,
    ) -> Result<ImageBuf, String> {
        let input = self.upload_dye_optical_pages(image)?;
        let pages = if logarithm {
            self.dye_log10_pages(&input)?
        } else {
            self.dye_crop_pages(&input, rect)?
        };
        self.materialize_dye_optical_pages(pages, rect[2], rect[3])
    }

    pub fn dye_log10(&self, image: &ImageBuf) -> Result<ImageBuf, String> {
        self.materialize_dye_utility(image, [0, 0, image.width, image.height], true)
    }

    pub fn dye_crop(&self, image: &ImageBuf, rect: [u32; 4]) -> Result<ImageBuf, String> {
        self.materialize_dye_utility(image, rect, false)
    }

    fn upload_dye_optical_pages(&self, image: &ImageBuf) -> Result<BufferPayload, String> {
        let pages = self.allocate_image_pages(image.width, image.height, "dye_optical_source")?;
        for page in &pages {
            self.check_cancelled()?;
            let start = page.first_pixel as usize * 3;
            let end = start + page.n_pixels as usize * 3;
            let samples = scalars_to_f32(&image.data[start..end]);
            self.queue
                .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&samples));
        }
        Ok(BufferPayload::from_pages(
            self.backend_id(),
            pages,
            image.width,
            image.height,
        ))
    }

    fn materialize_dye_optical_pages(
        &self,
        pages: Vec<ImagePage>,
        width: u32,
        height: u32,
    ) -> Result<ImageBuf, String> {
        let mut samples = Vec::with_capacity(width as usize * height as usize * 3);
        for page in pages {
            self.check_cancelled()?;
            samples.extend(
                self.pipe.readback_buffer_cancellable::<f32>(
                    &page.buffer,
                    u64::from(page.n_pixels) * 12,
                )?,
            );
        }
        self.check_cancelled()?;
        Ok(ImageBuf::from_data(width, height, f32_to_scalars(samples)))
    }

    pub(in crate::wgpu_backend) fn dye_crop_pages(
        &self,
        input: &BufferPayload,
        rect: [u32; 4],
    ) -> Result<Vec<ImagePage>, String> {
        let [x, y, width, height] = rect;
        if width == 0
            || height == 0
            || x.checked_add(width).is_none_or(|v| v > input.width)
            || y.checked_add(height).is_none_or(|v| v > input.height)
        {
            return Err("film grain Dye: final crop is outside its readout raster".into());
        }
        let pages = self.allocate_image_pages(width, height, "dye_crop_page")?;
        let capacity = crate::wgpu_backend::bounded::storage_binding_limit(self)? / 12;
        let scratch = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_crop_window"),
            size: u64::from(width).min(capacity) * 12,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        for row in 0..height {
            let mut col = 0;
            while col < width {
                self.check_cancelled()?;
                let count = (u64::from(width - col).min(capacity)) as u32;
                let mut encoder = self.device.create_command_encoder(&Default::default());
                self.copy_window_to(&mut encoder, input, x + col, y + row, count, 1, &scratch)?;
                self.copy_samples_from(
                    &mut encoder,
                    &scratch,
                    0,
                    &pages,
                    (u64::from(row) * u64::from(width) + u64::from(col)) * 3,
                    u64::from(count) * 3,
                )?;
                self.submit_commands(Some(encoder.finish()))?;
                col += count;
            }
        }
        self.check_cancelled()?;
        Ok(pages)
    }

    pub(in crate::wgpu_backend) fn dye_log10_pages(
        &self,
        input: &BufferPayload,
    ) -> Result<Vec<ImagePage>, String> {
        let outputs = self.allocate_image_pages(input.width, input.height, "dye_log_page")?;
        let mut pages = Vec::with_capacity(outputs.len());
        for page in outputs {
            self.check_cancelled()?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.copy_samples_to(
                &mut encoder,
                input,
                u64::from(page.first_pixel) * 3,
                u64::from(page.n_pixels) * 3,
                &page.buffer,
                0,
            )?;
            let (pass, output, _, _) = self.prepare_dye_log10(page.n_pixels, 1, &page.buffer)?;
            pass.encode(&mut encoder);
            self.submit_commands(Some(encoder.finish()))?;
            pages.push(ImagePage {
                first_pixel: page.first_pixel,
                n_pixels: page.n_pixels,
                buffer: output,
            });
        }
        self.check_cancelled()?;
        Ok(pages)
    }

    pub(in crate::wgpu_backend) fn dye_aperture_collapse_pages(
        &self,
        input: &BufferPayload,
        factor: u32,
    ) -> Result<(Vec<ImagePage>, u32, u32), String> {
        if factor == 0
            || !input.width.is_multiple_of(factor)
            || !input.height.is_multiple_of(factor)
        {
            return Err(
                "film grain Dye aperture: fine image dimensions must be divisible by the factor"
                    .into(),
            );
        }
        let width = input.width / factor;
        let height = input.height / factor;
        let capacity = crate::wgpu_backend::bounded::storage_binding_limit(self)? / 12;
        let block = u64::from(factor) * u64::from(factor);
        let columns = u32::try_from(capacity / block)
            .unwrap_or(u32::MAX)
            .min(width);
        if columns == 0 {
            return Err(
                "film grain Dye aperture: one output footprint exceeds device storage limits"
                    .into(),
            );
        }
        let pages = self.allocate_image_pages(width, height, "dye_aperture_page")?;
        let scratch = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_aperture_window"),
            size: u64::from(columns) * block * 12,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let full =
            self.prepare_dye_aperture_collapse(columns * factor, factor, &scratch, factor)?;
        let remainder = width % columns;
        let tail = if remainder > 0 {
            Some(self.prepare_dye_aperture_collapse(
                remainder * factor,
                factor,
                &scratch,
                factor,
            )?)
        } else {
            None
        };
        for row in 0..height {
            let mut col = 0;
            while col < width {
                self.check_cancelled()?;
                let count = columns.min(width - col);
                let fine_width = count * factor;
                let mut encoder = self.device.create_command_encoder(&Default::default());
                self.copy_window_to(
                    &mut encoder,
                    input,
                    col * factor,
                    row * factor,
                    fine_width,
                    factor,
                    &scratch,
                )?;
                let (pass, output, _, _) = if count == columns {
                    &full
                } else {
                    tail.as_ref().expect("nonempty final aperture block")
                };
                pass.encode(&mut encoder);
                self.copy_samples_from(
                    &mut encoder,
                    output,
                    0,
                    &pages,
                    (u64::from(row) * u64::from(width) + u64::from(col)) * 3,
                    u64::from(count) * 3,
                )?;
                self.submit_commands(Some(encoder.finish()))?;
                col += count;
            }
        }
        self.check_cancelled()?;
        Ok((pages, width, height))
    }
}
