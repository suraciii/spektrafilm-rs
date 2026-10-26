//! Bounded windows of one globally addressed field, transported through pages.
use super::*;
use crate::wgpu_backend::film_chain::device_image::{BufferPayload, ImagePage};

struct PagedBand {
    source: std::sync::Arc<BufferPayload>,
    geometry: DyeBand,
    input: wgpu::Buffer,
    layers: LayerPass,
    accumulate: AccumulatePass,
    spatial: DyeSpatialState,
    response: Option<optical::OpticalPass>,
    output: wgpu::Buffer,
    readout: dye_spatial::SpatialPass,
}

impl PagedBand {
    fn encode(
        &self,
        backend: &WgpuBackend,
        encoder: &mut ObservedEncoder,
        pages: &[ImagePage],
        width: u32,
    ) -> Result<(), String> {
        let b = self.geometry;
        backend.copy_window_to(
            encoder,
            &self.source,
            b.source_x,
            b.source_y,
            b.source_cols,
            b.source_rows,
            &self.input,
        )?;
        self.layers.encode(encoder);
        self.spatial.encode_remap(encoder);
        self.accumulate.encode(encoder);
        self.spatial.encode_formation(encoder);
        if let Some(response) = &self.response {
            response.encode(encoder);
        }
        self.readout.encode(encoder);
        if b.readout_flags & 2 != 0 {
            return Ok(());
        }
        for row in 0..b.output_rows {
            backend.copy_samples_from(
                encoder,
                &self.output,
                u64::from(row) * u64::from(b.output_cols) * 3,
                pages,
                (u64::from(b.output_y + row) * u64::from(width) + u64::from(b.output_x)) * 3,
                u64::from(b.output_cols) * 3,
            )?;
        }
        Ok(())
    }
}

pub(in crate::wgpu_backend) struct DyePagedState {
    source_validation: Vec<LayerPass>,
    bands: Vec<PagedBand>,
    pub(in crate::wgpu_backend) output_pages: Vec<ImagePage>,
    pub(in crate::wgpu_backend) width: u32,
    status: wgpu::Buffer,
}

impl DyePagedState {
    pub(in crate::wgpu_backend) fn status_buffer(&self) -> wgpu::Buffer {
        self.status.clone()
    }
    pub(in crate::wgpu_backend) fn output_pages(&self) -> Vec<ImagePage> {
        self.output_pages.clone()
    }

    pub(in crate::wgpu_backend) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        if !self.source_validation.is_empty() {
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            for validation in &self.source_validation {
                backend.check_cancelled()?;
                validation.encode(&mut encoder);
            }
            backend.submit_commands(Some(encoder.finish()))?;
        }
        for band in &self.bands {
            backend.check_cancelled()?;
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            band.encode(backend, &mut encoder, &self.output_pages, self.width)?;
            backend.submit_commands(Some(encoder.finish()))?;
        }
        backend.check_cancelled()?;
        Ok(())
    }
}

impl WgpuBackend {
    pub(in crate::wgpu_backend) fn prepare_dye_paged(
        &self,
        width: u32,
        height: u32,
        input: &BufferPayload,
        params: &DyeGpuParams,
        response: Option<&spektrafilm_backend::DyeResponseGpuParams<'_>>,
    ) -> Result<DyePagedState, String> {
        if input.width != width || input.height != height {
            return Err(
                "film grain Dye: paged source does not match the declared full-source raster"
                    .into(),
            );
        }
        let plan = self.dye_plan(width, height, params)?;
        let response = response
            .map(|r| optical::ResponseSetup::new(self, params, r))
            .transpose()?;
        if let Some(certification) = self.pipe.dye_certification.get() {
            certification.clone()?;
        } else {
            let certification = (|| {
                self.certify_dye_counts()?;
                self.certify_dye_layers()?;
                self.certify_dye_accumulation()?;
                self.certify_dye_spatial()
            })();
            // Cancellation belongs to this request, not the shared device.
            self.check_cancelled()?;
            self.pipe
                .dye_certification
                .get_or_init(|| certification)
                .clone()?;
        }
        let tables = self.pipe.static_buffer(
            "dye_channel_tables",
            bytemuck::cast_slice(&params.tables.tables),
            wgpu::BufferUsages::STORAGE,
        );
        let curves = self.pipe.static_buffer(
            "dye_channel_curves",
            bytemuck::cast_slice(&params.tables.curves),
            wgpu::BufferUsages::STORAGE,
        );
        let scratch = |label, size| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let status = scratch("dye_status", 4);
        let [sl, cl, c, h, f] = plan.max_band_bytes;
        // RGB source windows are smaller than their nine-lane layer targets.
        let input_window = scratch("dye_source_window", sl / 3);
        let source_layers = scratch("dye_source_layers", sl);
        let source_validation = if params.monochrome {
            input
                .pages
                .iter()
                .map(|page| {
                    dye_layers::build_source_validation(
                        self,
                        page.n_pixels,
                        &tables,
                        &curves,
                        &page.buffer,
                        &source_layers,
                        &status,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let cell_layers = scratch("dye_cell_layers", cl);
        let cells = scratch("dye_cell_density", c);
        let horizontal = scratch("dye_horizontal", h);
        let formed = scratch("dye_formed", f);
        let band_output = scratch("dye_band_readout", plan.max_output_bytes);
        let output_pages =
            self.allocate_image_pages(params.grid.width, params.grid.height, "dye_output")?;
        let mut bands = Vec::with_capacity(plan.bands.len());
        let source = std::sync::Arc::new(BufferPayload::from_pages(
            self.backend_id(),
            input.pages.clone(),
            width,
            height,
        ));
        for band in plan.bands {
            let layers = dye_layers::build_layers(
                self,
                [band.source_cols, band.source_rows, 0, 0],
                [band.source_cols, band.source_rows],
                params.monochrome,
                &tables,
                &curves,
                &input_window,
                &source_layers,
                &status,
            );
            let accumulate =
                self.build_dye_accumulate(params, &band, &tables, &cell_layers, &cells, &status);
            let spatial = dye_spatial::build_spatial(
                self,
                params,
                &band,
                width,
                height,
                &source_layers,
                &cell_layers,
                &cells,
                &horizontal,
                &formed,
                &band_output,
            );
            let response_band = response
                .as_ref()
                .map(|r| r.build(self, &band, &formed, &horizontal, &status));
            let readout = dye_spatial::build_band_readout(
                self,
                params,
                &band,
                [width, height],
                if response.is_some() {
                    &horizontal
                } else {
                    &formed
                },
                &band_output,
                response.is_some(),
                true,
            );
            bands.push(PagedBand {
                source: source.clone(),
                geometry: band,
                input: input_window.clone(),
                layers,
                accumulate,
                spatial,
                response: response_band,
                output: band_output.clone(),
                readout,
            });
        }
        Ok(DyePagedState {
            source_validation,
            bands,
            output_pages,
            width: params.grid.width,
            status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paged_source_preserves_the_same_stochastic_field() {
        check_paged_source(false);
        check_paged_source(true);
    }

    fn check_paged_source(monochrome: bool) {
        let backend = crate::test_support::adapter();
        let knots = [0.0, 1.0];
        let inv = [1.0];
        let zero = [0.0; 2];
        let tables = DyeChannelTablesGpu::pack(std::array::from_fn(|_| DyeChannelParts {
            min: 0.1,
            cap_total: 1.0,
            cap: [1.0, 0.0, 0.0],
            frac: [1.0, 0.0, 0.0],
            area: [0.2; 3],
            uniformity: 0.0,
            lookup_sign: 1.0,
            knots: &knots,
            inv_dx: &inv,
            layers: [&knots, &zero, &zero],
        }));
        let mut params = DyeGpuParams {
            tables,
            cell_um: 1.0,
            taps: vec![0.25, 0.5, 0.25],
            source_pitch_um: 1.0,
            grid: DyeReadoutGrid {
                origin_um: [-0.25, 0.375],
                pitch_um: [0.875, 1.125],
                width: 17,
                height: 11,
            },
            seed: 0x1234_5678_9abc_def0,
            channel_correlation: 0.3,
            expectation: false,
            monochrome: false,
        };
        params.monochrome = monochrome;
        let image = ImageBuf::from_data(
            16,
            12,
            (0..16 * 12 * 3)
                .map(|i| {
                    let index = if monochrome { i / 3 } else { i };
                    spektrafilm_math::precision::from_f64(0.2 + f64::from(index % 17) / 40.0)
                })
                .collect(),
        );
        let contiguous = backend.pipe.upload_image(
            scalars_to_f32(&image.data).as_ref(),
            image.pixel_count() as u32,
        );
        let singleton = BufferPayload::from_buffer(backend.backend_id(), contiguous.buf, 16, 12);
        let reference = backend
            .prepare_dye_paged(16, 12, &singleton, &params, None)
            .unwrap();
        reference.submit(&backend).unwrap();
        backend
            .check_dye_status(&reference.status_buffer())
            .unwrap();
        let expected: Vec<_> = reference
            .output_pages
            .iter()
            .flat_map(|page| {
                backend
                    .pipe
                    .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12)
            })
            .collect();
        let mut pages = Vec::new();
        for (i, chunk) in image.data.chunks(19 * 3).enumerate() {
            let values = scalars_to_f32(chunk);
            let buffer = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("dye_test_source_page"),
                    contents: bytemuck::cast_slice(&values),
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                });
            pages.push(ImagePage {
                first_pixel: (i * 19) as u32,
                n_pixels: (chunk.len() / 3) as u32,
                buffer,
            });
        }
        let input = BufferPayload::from_pages(backend.backend_id(), pages, 16, 12);
        let paged = backend
            .prepare_dye_paged(16, 12, &input, &params, None)
            .unwrap();
        paged.submit(&backend).unwrap();
        backend.check_dye_status(&paged.status_buffer()).unwrap();
        let actual: Vec<_> = paged
            .output_pages
            .iter()
            .flat_map(|page| {
                backend
                    .pipe
                    .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12)
            })
            .collect();
        if monochrome {
            assert!(
                actual
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .all(|p| p[0] == p[1] && p[0] == p[2])
            );
        }
        assert_eq!(
            expected, actual,
            "page transport must preserve one executor's stochastic draws and global footprints"
        );
        if monochrome {
            // The last page splits raster rows and lies outside this crop and
            // its formation halo. Original BW samples there still need validation.
            params.grid.origin_um = [0.0; 2];
            params.grid.width = 1;
            params.grid.height = 1;
            let last = input.pages.last().unwrap();
            backend.queue.write_buffer(
                &last.buffer,
                u64::from(last.n_pixels - 1) * 12 + 4,
                bytemuck::bytes_of(&0.5_f32),
            );
            let cropped = backend
                .prepare_dye_paged(16, 12, &input, &params, None)
                .unwrap();
            cropped.submit(&backend).unwrap();
            let error = backend
                .check_dye_status(&cropped.status_buffer())
                .unwrap_err();
            assert!(error.contains("lanes disagree"), "{error}");
        }
    }
}
