//! Backend-owned retained image carrier: the device payload and its boundary
//! operations.
//!
//! `spektrafilm-backend::DeviceImage` names the carrier; this module is the
//! wgpu half of it. A payload owns globally indexed RGB f32 pages and the
//! identity of its backend. Full dimensions survive every storage partition.
//! Upload and final materialization are the image transfer boundaries; page
//! extraction and intermediate stage transfers remain device-to-device.

use super::super::ObservedDevice;
use super::super::cache::CachedPipelineRef;
use super::super::pack_matrix_columns;
pub(in crate::wgpu_backend) use super::super::paged::ImagePage;
use super::*;
use spektrafilm_backend::telemetry::ObservationKind;
use spektrafilm_backend::{DeviceImage, DeviceImagePayload, Precision, RetainedImageMeta};
use std::any::Any;
use std::sync::Arc;

#[cfg(test)]
std::thread_local! {
    static UPLOAD_PAGE_HOOK: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Identity of one backend instance. Derived backends share the device, queue,
/// pipeline cache, and pipe, so they must also share the carrier identity: a
/// payload produced by a derived backend is consumable by its parent and by
/// every sibling derived from the same device.
static NEXT_BACKEND_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) fn next_backend_id() -> u64 {
    NEXT_BACKEND_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Device samples with absolute full-image page coordinates.
pub(in crate::wgpu_backend) struct BufferPayload {
    backend_id: u64,
    pub(in crate::wgpu_backend) pages: Vec<ImagePage>,
    pub(in crate::wgpu_backend) bytes: u64,
    pub(in crate::wgpu_backend) n_pixels: u32,
    pub(in crate::wgpu_backend) width: u32,
    pub(in crate::wgpu_backend) height: u32,
}

impl BufferPayload {
    pub(in crate::wgpu_backend) fn from_buffer(
        backend_id: u64,
        buffer: wgpu::Buffer,
        width: u32,
        height: u32,
    ) -> Self {
        let n_pixels = width * height;
        Self {
            backend_id,
            pages: vec![ImagePage {
                first_pixel: 0,
                n_pixels,
                buffer,
            }],
            bytes: u64::from(n_pixels) * 12,
            n_pixels,
            width,
            height,
        }
    }

    pub(in crate::wgpu_backend) fn from_pages(
        backend_id: u64,
        pages: Vec<ImagePage>,
        width: u32,
        height: u32,
    ) -> Self {
        let n_pixels = width * height;
        Self {
            backend_id,
            pages,
            bytes: u64::from(n_pixels) * 12,
            n_pixels,
            width,
            height,
        }
    }
}

impl DeviceImagePayload for BufferPayload {
    fn backend_id(&self) -> u64 {
        self.backend_id
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Retained-carrier boundary operations for the wgpu backend.
impl WgpuBackend {
    /// The one upload a host-decoded input makes. The carrier keeps the
    /// device buffer; the host `ImageBuf` is not retained, so a later stage
    /// cannot quietly read host samples instead of the device's.
    pub(in crate::wgpu_backend) fn upload_retained(
        &self,
        image: &ImageBuf,
        meta: &RetainedImageMeta,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        if meta.width != image.width || meta.height != image.height {
            return Err(format!(
                "retained metadata {}x{} does not match the image {}x{}",
                meta.width, meta.height, image.width, image.height
            ));
        }
        let pages = self.allocate_image_pages(image.width, image.height, "retained_upload")?;
        for page in &pages {
            self.check_cancelled()?;
            let start = page.first_pixel as usize * 3;
            let end = start + page.n_pixels as usize * 3;
            let samples = scalars_to_f32(&image.data[start..end]);
            #[cfg(test)]
            UPLOAD_PAGE_HOOK.with(|slot| {
                if let Some(hook) = slot.borrow_mut().as_mut() {
                    hook();
                }
            });
            self.check_cancelled()?;
            self.queue
                .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&samples));
        }
        self.check_cancelled()?;
        Ok(self.retain_image_pages(
            pages,
            image.width,
            image.height,
            meta.domain,
            meta.space,
            meta.pixel_pitch_um,
        ))
    }

    /// Wrap a device buffer this backend produced as a retained carrier. Zero
    /// copy: the samples never leave the device.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::wgpu_backend) fn retain_buffer(
        &self,
        buf: wgpu::Buffer,
        width: u32,
        height: u32,
        bytes: u64,
        domain: spektrafilm_backend::SampleDomain,
        space: &'static spektrafilm_math::colorspace::RgbColorSpace,
        pixel_pitch_um: Option<f64>,
    ) -> DeviceImage {
        let meta = RetainedImageMeta {
            width,
            height,
            domain,
            precision: Precision::F32,
            space,
            pixel_pitch_um,
        };
        let mut payload = BufferPayload::from_buffer(self.backend_id(), buf, width, height);
        payload.bytes = bytes;
        DeviceImage::retained(meta, Arc::new(payload))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::wgpu_backend) fn retain_image_pages(
        &self,
        pages: Vec<ImagePage>,
        width: u32,
        height: u32,
        domain: spektrafilm_backend::SampleDomain,
        space: &'static spektrafilm_math::colorspace::RgbColorSpace,
        pixel_pitch_um: Option<f64>,
    ) -> DeviceImage {
        DeviceImage::retained(
            RetainedImageMeta {
                width,
                height,
                domain,
                precision: Precision::F32,
                space,
                pixel_pitch_um,
            },
            Arc::new(BufferPayload {
                backend_id: self.backend_id(),
                pages,
                bytes: u64::from(width) * u64::from(height) * 12,
                n_pixels: width * height,
                width,
                height,
            }),
        )
    }

    /// Borrow a carrier's device payload. A carrier this backend did not
    /// produce — host samples, another backend, or another device — is an
    /// error, never a silent fallback to host arithmetic.
    pub(in crate::wgpu_backend) fn buffer_payload<'a>(
        &self,
        image: &'a DeviceImage,
    ) -> Result<&'a BufferPayload, String> {
        let payload = image
            .payload_ref()
            .map_err(|error| format!("retained image unavailable: {error}"))?;
        if payload.backend_id() != self.backend_id() {
            return Err(
                "retained image belongs to a different device; materialize it and upload it \
                 again on this backend"
                    .to_owned(),
            );
        }
        payload
            .as_any()
            .downcast_ref::<BufferPayload>()
            .ok_or_else(|| "retained image payload is not a device buffer".to_owned())
    }

    /// Explicit host materialization of retained samples. The one readback a
    /// production route takes at its final boundary.
    pub(in crate::wgpu_backend) fn materialize_buffer(
        &self,
        image: &DeviceImage,
    ) -> Result<ImageBuf, String> {
        self.check_cancelled()?;
        let payload = self.buffer_payload(image)?;
        let mut samples = Vec::with_capacity((payload.bytes / 4) as usize);
        for page in &payload.pages {
            self.check_cancelled()?;
            samples.extend(f32_to_scalars(self.pipe.readback_buffer_cancellable(
                &page.buffer,
                u64::from(page.n_pixels) * 12,
            )?));
        }
        self.check_cancelled()?;
        Ok(ImageBuf::from_data(image.width(), image.height(), samples))
    }

    /// Identity of this backend instance, shared with every backend derived
    /// from the same device.
    pub(in crate::wgpu_backend) fn backend_id(&self) -> u64 {
        self.backend_id
    }
}

/// Retained device passes a production route needs between its numerical
/// stages: the input exposure scale, the colour-space conversion, and the
/// final output mix. Each consumes and produces carriers, so no pass in this
/// group materializes an image.
impl WgpuBackend {
    /// Multiply every retained sample by `scale` on the device.
    ///
    /// The reference applies the metered exposure to the uploaded samples
    /// before the working geometry and before the film front pass decodes
    /// them, so this pass preserves the sample domain it was given.
    pub(in crate::wgpu_backend) fn scale_retained_image(
        &self,
        image: &DeviceImage,
        scale: f32,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        if self.needs_pointwise_pages(image)? {
            return self.map_pointwise_pages(image, image.domain(), |page| {
                self.scale_retained_image(page, scale)
            });
        }
        let source = self.buffer_payload(image)?;
        let out = stage_output_buffer(&self.device, source.bytes, "retained_scaled");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.check_cancelled()?;
        encoder.copy_buffer_to_buffer(&source.pages[0].buffer, 0, &out, 0, source.bytes);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("retained_scale"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.scale_pipeline().pipeline);
            pass.set_bind_group(0, &self.scale_bind_group(&out, scale, source.n_pixels), &[]);
            dispatch_linear(&mut pass, source.n_pixels);
        }
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        #[cfg(test)]
        self.test_pointwise_submitted("scale");
        self.check_cancelled()?;
        Ok(self.retain_buffer(
            out,
            image.width(),
            image.height(),
            source.bytes,
            image.domain(),
            image.space(),
            image.pixel_pitch_um(),
        ))
    }

    /// Convert retained samples with one source→destination colour transform.
    ///
    /// The three device passes reproduce the reference order exactly: decode
    /// the source transfer, apply the matrix, encode the destination
    /// transfer — the same order `convert_rgb` evaluates.
    pub(in crate::wgpu_backend) fn convert_colorspace_image(
        &self,
        image: &DeviceImage,
        params: &spektrafilm_backend::ConvertColorspaceGpuParams,
        destination: &'static spektrafilm_math::colorspace::RgbColorSpace,
        domain: spektrafilm_backend::SampleDomain,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        if !self.device.context.enabled() {
            return self.convert_colorspace_image_inner(image, params, destination, domain);
        }
        let mut scope = self.device.context.scope(
            "colorspace_convert",
            ObservationKind::Stage,
            self.device.context.purpose(),
        );
        let mut backend = self.derived_explicit();
        backend.explicit = self.explicit;
        backend.context_mut(scope.context().clone());
        let result = backend.convert_colorspace_image_inner(image, params, destination, domain);
        scope.set_complete(result.is_ok());
        result
    }

    fn convert_colorspace_image_inner(
        &self,
        image: &DeviceImage,
        params: &spektrafilm_backend::ConvertColorspaceGpuParams,
        destination: &'static spektrafilm_math::colorspace::RgbColorSpace,
        domain: spektrafilm_backend::SampleDomain,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        let mut current = image.clone();
        if params.decode {
            current = self.transfer_retained(
                &current,
                true,
                params.src_cctf,
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                spektrafilm_backend::SampleDomain::Linear,
            )?;
        }
        current = self.matrix_retained(&current, params.matrix)?;
        if params.encode {
            // The source-to-destination matrix already ran above. The shared
            // transfer shader also multiplies its matrix before encoding.
            current = self.transfer_retained(
                &current,
                false,
                params.dst_cctf,
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                domain,
            )?;
        }
        self.check_cancelled()?;
        Ok(current.with_meta(spektrafilm_backend::RetainedImageMeta {
            width: image.width(),
            height: image.height(),
            domain,
            precision: spektrafilm_backend::Precision::F32,
            space: destination,
            pixel_pitch_um: image.pixel_pitch_um(),
        }))
    }

    /// Blend the processed result with the prepared source on the device,
    /// returning a new carrier. The source carrier is never modified.
    pub(in crate::wgpu_backend) fn mix_output_images(
        &self,
        result: &DeviceImage,
        source: &DeviceImage,
        params: &spektrafilm_backend::OutputMixGpuParams,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        if result.width() != source.width() || result.height() != source.height() {
            return Err("output_mix source and final output geometry differ".into());
        }
        if self.needs_pointwise_pages(result)? || self.needs_pointwise_pages(source)? {
            return self.mix_pointwise_pages(result, source, params);
        }
        let result_payload = self.buffer_payload(result)?;
        let source_payload = self.buffer_payload(source)?;
        let out = stage_output_buffer(&self.device, result_payload.bytes, "retained_mixed");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.check_cancelled()?;
        encoder.copy_buffer_to_buffer(
            &result_payload.pages[0].buffer,
            0,
            &out,
            0,
            result_payload.bytes,
        );
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct MixParams {
            n_pixels: u32,
            encoded: u32,
            cctf_kind: u32,
            amount: f32,
        }
        let mix_params = MixParams {
            n_pixels: result_payload.n_pixels,
            encoded: u32::from(params.encoded),
            cctf_kind: params.cctf_kind,
            amount: params.amount,
        };
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("retained_mix_params"),
                contents: bytemuck::bytes_of(&mix_params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let pipeline = self.cached_pipeline(
            concat!(
                include_str!("../../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                include_str!("../../../../spektrafilm-shaders/wgsl/output_mix.wgsl")
            ),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Storage { read_only: true },
            ],
        );
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("retained_mix"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: out.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: source_payload.pages[0].buffer.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("retained_mix"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            dispatch_linear(&mut pass, result_payload.n_pixels);
        }
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        #[cfg(test)]
        self.test_pointwise_submitted("mix");
        self.check_cancelled()?;
        Ok(self.retain_buffer(
            out,
            result.width(),
            result.height(),
            result_payload.bytes,
            result.domain(),
            result.space(),
            result.pixel_pitch_um(),
        ))
    }

    /// Per-pixel multiply by a constant, in place on a fresh device buffer.
    fn scale_pipeline(&self) -> CachedPipelineRef {
        self.cached_pipeline(
            include_str!("../../../../spektrafilm-shaders/wgsl/retained_scale.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        )
    }

    fn scale_bind_group(
        &self,
        buffer: &wgpu::Buffer,
        scale: f32,
        n_pixels: u32,
    ) -> wgpu::BindGroup {
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct ScaleParams {
            n_pixels: u32,
            scale: f32,
        }
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("retained_scale_params"),
                contents: bytemuck::bytes_of(&ScaleParams { n_pixels, scale }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layout = &self.scale_pipeline_layout();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("retained_scale"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn scale_pipeline_layout(&self) -> wgpu::BindGroupLayout {
        self.scale_pipeline().layout
    }

    /// One transfer pass (`decode`) over a copied device buffer.
    fn transfer_retained(
        &self,
        image: &DeviceImage,
        decode: bool,
        cctf_kind: u32,
        matrix: [[f32; 3]; 3],
        domain: spektrafilm_backend::SampleDomain,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        if self.needs_pointwise_pages(image)? {
            return self.map_pointwise_pages(image, domain, |page| {
                self.transfer_retained(page, decode, cctf_kind, matrix, domain)
            });
        }
        let source = self.buffer_payload(image)?;
        let out = stage_output_buffer(&self.device, source.bytes, "retained_transfer");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.check_cancelled()?;
        encoder.copy_buffer_to_buffer(&source.pages[0].buffer, 0, &out, 0, source.bytes);
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct TransferParams {
            n_pixels: u32,
            mode: u32,
            cctf_kind: u32,
            _pad: u32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
        }
        let columns = pack_matrix_columns(&matrix);
        let params = TransferParams {
            n_pixels: source.n_pixels,
            mode: u32::from(decode),
            cctf_kind,
            _pad: 0,
            col0: columns[0],
            col1: columns[1],
            col2: columns[2],
        };
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("retained_transfer_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let pipeline = self.cached_pipeline(
            concat!(
                include_str!("../../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                include_str!("../../../../spektrafilm-shaders/wgsl/cctf_transfer.wgsl")
            ),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("retained_transfer"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: out.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("retained_transfer"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            dispatch_linear(&mut pass, source.n_pixels);
        }
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        #[cfg(test)]
        self.test_pointwise_submitted("transfer");
        self.check_cancelled()?;
        Ok(self.retain_buffer(
            out,
            image.width(),
            image.height(),
            source.bytes,
            domain,
            image.space(),
            image.pixel_pitch_um(),
        ))
    }

    /// CMYK ink style on retained samples: the terminal appearance of the
    /// input route, evaluated on the device with the same kernel the resident
    /// chain uses at its tail.
    pub(in crate::wgpu_backend) fn cmyk_retained(
        &self,
        image: &DeviceImage,
        params: &spektrafilm_backend::CmykGpuParams,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        if params.strength.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return Ok(image.clone());
        }
        if self.needs_pointwise_pages(image)? {
            return self.map_pointwise_pages(image, image.domain(), |page| {
                self.cmyk_retained(page, params)
            });
        }
        let source = self.buffer_payload(image)?;
        let out = stage_output_buffer(&self.device, source.bytes, "retained_cmyk");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.check_cancelled()?;
        encoder.copy_buffer_to_buffer(&source.pages[0].buffer, 0, &out, 0, source.bytes);
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct CmykParams {
            n_pixels: u32,
            cctf_kind: u32,
            encoded: u32,
            _pad0: u32,
            strength_pad: [f32; 4],
            to_col0: [f32; 4],
            to_col1: [f32; 4],
            to_col2: [f32; 4],
            from_col0: [f32; 4],
            from_col1: [f32; 4],
            from_col2: [f32; 4],
        }
        let columns = |m: &[[f32; 3]; 3]| pack_matrix_columns(m);
        let to = columns(&params.to_lab);
        let from = columns(&params.from_lab);
        let cmyk_params = CmykParams {
            n_pixels: source.n_pixels,
            cctf_kind: params.cctf_kind,
            encoded: u32::from(params.encoded),
            _pad0: 0,
            strength_pad: [params.strength, 0.0, 0.0, 0.0],
            to_col0: to[0],
            to_col1: to[1],
            to_col2: to[2],
            from_col0: from[0],
            from_col1: from[1],
            from_col2: from[2],
        };
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("retained_cmyk_params"),
                contents: bytemuck::bytes_of(&cmyk_params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let pipeline = self.cached_pipeline(
            concat!(
                include_str!("../../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                include_str!("../../../../spektrafilm-shaders/wgsl/cmyk_ink.wgsl")
            ),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("retained_cmyk"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: out.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("retained_cmyk"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            dispatch_linear(&mut pass, source.n_pixels);
        }
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        #[cfg(test)]
        self.test_pointwise_submitted("cmyk");
        self.check_cancelled()?;
        Ok(self.retain_buffer(
            out,
            image.width(),
            image.height(),
            source.bytes,
            image.domain(),
            image.space(),
            image.pixel_pitch_um(),
        ))
    }

    /// One 3×3 matrix pass over a copied device buffer.
    fn matrix_retained(
        &self,
        image: &DeviceImage,
        matrix: [[f32; 3]; 3],
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        if self.needs_pointwise_pages(image)? {
            return self.map_pointwise_pages(image, image.domain(), |page| {
                self.matrix_retained(page, matrix)
            });
        }
        let source = self.buffer_payload(image)?;
        let out = stage_output_buffer(&self.device, source.bytes, "retained_matrix");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct MatrixParams {
            width: u32,
            height: u32,
            m: [f32; 9],
        }
        let params = MatrixParams {
            width: image.width(),
            height: image.height(),
            m: [
                matrix[0][0],
                matrix[0][1],
                matrix[0][2],
                matrix[1][0],
                matrix[1][1],
                matrix[1][2],
                matrix[2][0],
                matrix[2][1],
                matrix[2][2],
            ],
        };
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("retained_matrix_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let pipeline = self.cached_pipeline(
            include_str!("../../../../spektrafilm-shaders/wgsl/colorspace_convert.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("retained_matrix"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: source.pages[0].buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: out.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("retained_matrix"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            dispatch_linear(&mut pass, source.n_pixels);
        }
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        #[cfg(test)]
        self.test_pointwise_submitted("matrix");
        self.check_cancelled()?;
        Ok(self.retain_buffer(
            out,
            image.width(),
            image.height(),
            source.bytes,
            image.domain(),
            image.space(),
            image.pixel_pitch_um(),
        ))
    }
}

/// Allocate a device buffer with the usage a stage output needs. Shared so
/// every device pass produces a buffer the carrier can hold.
pub(in crate::wgpu_backend) fn stage_output_buffer(
    device: &ObservedDevice,
    bytes: u64,
    label: &str,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use spektrafilm_backend::telemetry::{CollectionMode, ObservationContext};
    use spektrafilm_backend::{ComputeBackend, SampleDomain};
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    const MATRIX: [[f32; 3]; 3] = [[0.5, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.5]];
    const VALUES: [f32; 15] = [
        0.125, 0.125, 0.125, 0.25, 0.25, 0.25, 0.375, 0.375, 0.375, 0.5, 0.5, 0.5, 0.75, 0.75, 0.75,
    ];

    struct RestoreLimit(Option<std::ffi::OsString>);
    impl Drop for RestoreLimit {
        fn drop(&mut self) {
            // The explicit adapter holds the process-wide device guard.
            unsafe {
                match &self.0 {
                    Some(value) => std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", value),
                    None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
                }
            }
        }
    }

    struct ResetSubmitHook;
    impl Drop for ResetSubmitHook {
        fn drop(&mut self) {
            WgpuBackend::test_clear_pointwise_submit();
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Operation {
        Scale,
        Decode,
        Encode,
        Matrix,
        Convert,
        Cmyk,
        Mix,
        CmykDisabled,
    }

    const OPERATIONS: [Operation; 8] = [
        Operation::Scale,
        Operation::Decode,
        Operation::Encode,
        Operation::Matrix,
        Operation::Convert,
        Operation::Cmyk,
        Operation::Mix,
        Operation::CmykDisabled,
    ];

    impl Operation {
        fn run(
            self,
            gpu: &WgpuBackend,
            image: &DeviceImage,
            source: &DeviceImage,
        ) -> Result<DeviceImage, String> {
            match self {
                Self::Scale => gpu.scale_retained(image, 2.0).map(|value| value.unwrap()),
                Self::Decode | Self::Encode => gpu.transfer_retained(
                    image,
                    matches!(self, Self::Decode),
                    2,
                    IDENTITY,
                    SampleDomain::Linear,
                ),
                Self::Matrix => gpu.matrix_retained(image, MATRIX),
                Self::Convert => gpu
                    .convert_colorspace_retained(
                        image,
                        &spektrafilm_backend::ConvertColorspaceGpuParams {
                            src_cctf: 2,
                            dst_cctf: 2,
                            decode: true,
                            encode: true,
                            matrix: MATRIX,
                        },
                        image.space(),
                        SampleDomain::Encoded,
                    )
                    .map(|value| value.unwrap()),
                Self::Cmyk | Self::CmykDisabled => ComputeBackend::cmyk_retained(
                    gpu,
                    image,
                    &spektrafilm_backend::CmykGpuParams {
                        strength: if matches!(self, Self::CmykDisabled) {
                            0.0
                        } else {
                            0.5
                        },
                        encoded: false,
                        cctf_kind: 0,
                        to_lab: [[0.96422, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.82521]],
                        from_lab: [
                            [1.0 / 0.96422, 0.0, 0.0],
                            [0.0, 1.0, 0.0],
                            [0.0, 0.0, 1.0 / 0.82521],
                        ],
                    },
                )
                .map(|value| value.unwrap()),
                Self::Mix => gpu
                    .mix_output_retained(
                        image,
                        source,
                        &spektrafilm_backend::OutputMixGpuParams {
                            amount: 0.25,
                            encoded: false,
                            cctf_kind: 0,
                        },
                    )
                    .map(|value| value.unwrap()),
            }
        }

        fn expected(self) -> Vec<f32> {
            use spektrafilm_math::colorspace::{Cctf, cctf_decode, cctf_encode};
            VALUES
                .iter()
                .enumerate()
                .map(|(index, &value)| {
                    let value = f64::from(value);
                    let factor = [0.5, 1.0, 1.5][index % 3];
                    (match self {
                        Self::Scale => value * 2.0,
                        Self::Decode => cctf_decode(value, Cctf::Srgb),
                        Self::Encode => cctf_encode(value, Cctf::Srgb),
                        Self::Matrix => value * factor,
                        Self::Convert => {
                            cctf_encode(cctf_decode(value, Cctf::Srgb) * factor, Cctf::Srgb)
                        }
                        Self::Cmyk => {
                            // Neutral D50 input has zero Lab chroma: only the ink's
                            // specified lightness contrast and half-strength mix remain.
                            let lightness = 116.0 * value.cbrt() - 16.0;
                            let mapped = (50.0 + (lightness - 50.0) * 1.03).max(4.0);
                            value + 0.5 * (((mapped + 16.0) / 116.0).powi(3) - value)
                        }
                        Self::Mix => value * 0.8125,
                        Self::CmykDisabled => value,
                    }) as f32
                })
                .collect()
        }
    }

    fn fixture(gpu: &WgpuBackend) -> (DeviceImage, DeviceImage) {
        let space = spektrafilm_math::colorspace::resolve("sRGB").unwrap();
        let meta = RetainedImageMeta {
            width: 5,
            height: 1,
            domain: SampleDomain::Linear,
            precision: Precision::F32,
            space,
            pixel_pitch_um: Some(6.0),
        };
        let image = gpu
            .upload_retained(
                &ImageBuf::from_data(5, 1, f32_to_scalars(VALUES.to_vec())),
                &meta,
            )
            .unwrap();
        let source = gpu
            .upload_retained(
                &ImageBuf::from_data(
                    5,
                    1,
                    f32_to_scalars(VALUES.iter().map(|value| value * 0.75).collect()),
                ),
                &meta,
            )
            .unwrap();
        (image, source)
    }

    fn cancellable(gpu: &WgpuBackend, token: Arc<AtomicBool>, observed: bool) -> WgpuBackend {
        let mut operation = gpu.derived_explicit();
        operation.pipe = operation.pipe.with_cancellation(Some(token.clone()));
        operation.cancellation = Some(token);
        if observed {
            operation.context_mut(ObservationContext::new(CollectionMode::Summary));
        }
        operation
    }

    fn assert_pixels(gpu: &WgpuBackend, image: &DeviceImage, expected: &[f32]) {
        let host = image.to_host(gpu).unwrap();
        let actual = scalars_to_f32(&host.data);
        assert_eq!(actual.len(), expected.len());
        for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() < 3.0e-6,
                "sample {index}: {actual} != {expected}"
            );
        }
    }

    fn assert_reusable(
        gpu: &WgpuBackend,
        operation: Operation,
        image: &DeviceImage,
        source: &DeviceImage,
    ) {
        let result = operation.run(gpu, image, source).unwrap();
        assert_pixels(gpu, &result, &operation.expected());
        assert_pixels(gpu, image, &VALUES);
        assert_pixels(gpu, source, &VALUES.map(|value| value * 0.75));
    }

    #[test]
    fn retained_pointwise_already_cancelled_stops_all_families_and_preserves_sources() {
        let gpu = crate::test_support::explicit_adapter();
        let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "1048576");
        }
        let (image, source) = fixture(&gpu);
        assert_eq!(gpu.buffer_payload(&image).unwrap().pages.len(), 1);
        let submissions = Arc::new(Mutex::new(Vec::new()));
        gpu.test_on_pointwise_submit({
            let submissions = submissions.clone();
            move |name| submissions.lock().unwrap().push(name)
        });
        let _reset = ResetSubmitHook;
        for observed in [false, true] {
            for operation in OPERATIONS {
                let token = Arc::new(AtomicBool::new(true));
                let cancelled = cancellable(&gpu, token.clone(), observed);
                assert_eq!(
                    operation.run(&cancelled, &image, &source).unwrap_err(),
                    "operation cancelled",
                    "{operation:?}, observed={observed}"
                );
                assert_eq!(
                    cancelled.materialize_buffer(&image).unwrap_err(),
                    "operation cancelled"
                );
                assert!(
                    submissions.lock().unwrap().is_empty(),
                    "already cancelled work submitted"
                );
                // Clearing a token must not poison the shared device.
                let cleared = cancelled.with_cancellation(None).unwrap();
                let scaled = cleared.scale_retained(&image, 2.0).unwrap().unwrap();
                assert_pixels(&gpu, &scaled, &Operation::Scale.expected());
                submissions.lock().unwrap().clear();
                assert_reusable(&gpu, operation, &image.clone(), &source.clone());
                submissions.lock().unwrap().clear();
                assert!(token.load(Ordering::Relaxed));
            }
        }
    }

    #[test]
    fn retained_pointwise_midpage_cancellation_stops_submissions_and_keeps_device_reusable() {
        let gpu = crate::test_support::explicit_adapter();
        let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
        let _reset = ResetSubmitHook;
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "1048576");
        }
        let (image, source) = fixture(&gpu);
        for limit in [1048576, 24] {
            unsafe {
                std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", limit.to_string());
            }
            let (image, source) = if limit == 24 {
                fixture(&gpu)
            } else {
                (image.clone(), source.clone())
            };
            assert_eq!(
                gpu.buffer_payload(&image).unwrap().pages.len(),
                if limit == 24 { 3 } else { 1 }
            );
            for observed in [false, true] {
                for operation in OPERATIONS
                    .into_iter()
                    .filter(|operation| !matches!(operation, Operation::CmykDisabled))
                {
                    let page_copy = if matches!(operation, Operation::Mix) {
                        "mix_page_copy"
                    } else {
                        "pointwise_page_copy"
                    };
                    let kernel = match operation {
                        Operation::Scale => "scale",
                        Operation::Decode | Operation::Encode | Operation::Convert => "transfer",
                        Operation::Matrix => "matrix",
                        Operation::Cmyk => "cmyk",
                        Operation::Mix => "mix",
                        Operation::CmykDisabled => unreachable!(),
                    };
                    // Cancel at the first extraction, first kernel, and final kernel.
                    // The final boundary must discard the carrier, not report success.
                    let total = match (limit == 24, matches!(operation, Operation::Convert)) {
                        (false, false) => 1,
                        (false, true) => 3,
                        (true, false) => 6,
                        (true, true) => 18,
                    };
                    let boundaries: &[usize] = if limit == 24 {
                        &[1, 2, total]
                    } else if total > 1 {
                        &[1, total]
                    } else {
                        &[1]
                    };
                    for &cancel_after in boundaries {
                        let token = Arc::new(AtomicBool::new(false));
                        let submissions = Arc::new(Mutex::new(Vec::new()));
                        gpu.test_on_pointwise_submit({
                            let token = token.clone();
                            let submissions = submissions.clone();
                            move |name| {
                                let mut names = submissions.lock().unwrap();
                                names.push(name);
                                if names.len() == cancel_after {
                                    token.store(true, Ordering::Relaxed);
                                }
                            }
                        });
                        let cancelled = cancellable(&gpu, token.clone(), observed);
                        assert_eq!(
                            operation
                                .run(&cancelled, &image.clone(), &source.clone())
                                .unwrap_err(),
                            "operation cancelled",
                            "{operation:?}, limit={limit}, observed={observed}, cancel_after={cancel_after}"
                        );
                        let names = submissions.lock().unwrap();
                        assert_eq!(
                            names.len(),
                            cancel_after,
                            "work continued after cancellation: {names:?}"
                        );
                        assert_eq!(names[0], if limit == 24 { page_copy } else { kernel });
                        if limit == 24 && cancel_after >= 2 {
                            assert_eq!(names[1], kernel);
                        }
                        drop(names);
                        gpu.test_on_pointwise_submit(|_| {});
                        let replacement = cancelled
                            .with_cancellation(Some(Arc::new(AtomicBool::new(false))))
                            .unwrap();
                        let scaled = replacement.scale_retained(&image, 2.0).unwrap().unwrap();
                        assert_pixels(&gpu, &scaled, &Operation::Scale.expected());
                        assert_reusable(&gpu, operation, &image, &source);
                        assert!(
                            token.load(Ordering::Relaxed),
                            "healthy reuse changed the cancelled operation's token"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn retained_pointwise_readback_cancellation_stops_pages_and_preserves_shared_sources() {
        use std::sync::atomic::AtomicUsize;
        let gpu = crate::test_support::explicit_adapter();
        let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "24");
        }
        let (image, source) = fixture(&gpu);
        assert_eq!(gpu.buffer_payload(&image).unwrap().pages.len(), 3);
        for observed in [false, true] {
            let token = Arc::new(AtomicBool::new(false));
            let readbacks = Arc::new(AtomicUsize::new(0));
            gpu.pipe.test_on_readback({
                let token = token.clone();
                let readbacks = readbacks.clone();
                move || {
                    readbacks.fetch_add(1, Ordering::Relaxed);
                    token.store(true, Ordering::Relaxed);
                }
            });
            let cancelled = cancellable(&gpu, token.clone(), observed);
            assert_eq!(
                image.clone().to_host(&cancelled).unwrap_err(),
                "operation cancelled"
            );
            assert_eq!(
                readbacks.load(Ordering::Relaxed),
                1,
                "materialization read the next page after cancellation"
            );
            gpu.pipe.test_on_readback(|| {});
            assert_reusable(&gpu, Operation::Scale, &image, &source);
            let replacement = cancelled
                .with_cancellation(Some(Arc::new(AtomicBool::new(false))))
                .unwrap();
            let reused = image.to_host(replacement.as_ref()).unwrap();
            assert_eq!(scalars_to_f32(&reused.data).as_ref(), VALUES.as_slice());
            assert!(token.load(Ordering::Relaxed));
        }
    }

    #[test]
    fn retained_upload_final_conversion_cancellation_discards_carrier_and_preserves_reuse() {
        let gpu = crate::test_support::explicit_adapter();
        let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                UPLOAD_PAGE_HOOK.with(|slot| *slot.borrow_mut() = None);
            }
        }
        let _reset = Reset;
        for limit in [1048576, 24] {
            unsafe {
                std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", limit.to_string());
            }
            let samples = ImageBuf::from_data(5, 1, f32_to_scalars(VALUES.to_vec()));
            let meta = RetainedImageMeta {
                width: 5,
                height: 1,
                domain: SampleDomain::Linear,
                precision: Precision::F32,
                space: &spektrafilm_math::colorspace::SRGB,
                pixel_pitch_um: None,
            };
            let final_page = if limit == 24 { 3 } else { 1 };
            let token = Arc::new(AtomicBool::new(false));
            let converted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            UPLOAD_PAGE_HOOK.with(|slot| {
                *slot.borrow_mut() = Some(Box::new({
                    let token = token.clone();
                    let converted = converted.clone();
                    move || {
                        if converted.fetch_add(1, Ordering::Relaxed) + 1 == final_page {
                            token.store(true, Ordering::Relaxed);
                        }
                    }
                }))
            });
            let operation = cancellable(&gpu, token.clone(), true);
            assert_eq!(
                operation.retain_image(&samples, &meta).unwrap_err(),
                "operation cancelled"
            );
            assert_eq!(converted.load(Ordering::Relaxed), final_page);
            UPLOAD_PAGE_HOOK.with(|slot| *slot.borrow_mut() = None);
            let healthy = gpu.retain_image(&samples, &meta).unwrap();
            assert_eq!(
                scalars_to_f32(&healthy.to_host(gpu.backend()).unwrap().data).as_ref(),
                &VALUES
            );
            assert_eq!(scalars_to_f32(&samples.data).as_ref(), &VALUES);
            assert!(token.load(Ordering::Relaxed));
        }
    }
}
