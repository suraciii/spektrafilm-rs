//! Backend-owned retained image carrier: the device payload and its boundary
//! operations.
//!
//! `spektrafilm-backend::DeviceImage` names the carrier; this module is the
//! wgpu half of it. A payload owns globally indexed RGB f32 pages and the
//! identity of its backend. Full dimensions survive every storage partition.
//! Upload and final materialization are the image transfer boundaries; page
//! extraction and intermediate stage transfers remain device-to-device.

use super::*;
use super::super::ObservedDevice;
use super::super::cache::CachedPipelineRef;
use super::super::pack_matrix_columns;
pub(in crate::wgpu_backend) use super::super::paged::ImagePage;
use spektrafilm_backend::telemetry::ObservationKind;
use spektrafilm_backend::{
    DeviceImage, DeviceImagePayload, Precision, RetainedImageMeta,
};
use std::any::Any;
use std::sync::Arc;

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
    pub(in crate::wgpu_backend) fn from_buffer(backend_id: u64, buffer: wgpu::Buffer, width: u32, height: u32) -> Self {
        let n_pixels = width * height;
        Self { backend_id, pages: vec![ImagePage { first_pixel: 0, n_pixels, buffer }],
            bytes: u64::from(n_pixels) * 12, n_pixels, width, height }
    }

    pub(in crate::wgpu_backend) fn from_pages(backend_id: u64, pages: Vec<ImagePage>, width: u32, height: u32) -> Self {
        let n_pixels = width * height;
        Self { backend_id, pages, bytes: u64::from(n_pixels) * 12, n_pixels, width, height }
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
            self.queue.write_buffer(&page.buffer, 0, bytemuck::cast_slice(&samples));
        }
        Ok(self.retain_image_pages(pages, image.width, image.height, meta.domain, meta.space, meta.pixel_pitch_um))
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
        &self, pages: Vec<ImagePage>, width: u32, height: u32,
        domain: spektrafilm_backend::SampleDomain,
        space: &'static spektrafilm_math::colorspace::RgbColorSpace,
        pixel_pitch_um: Option<f64>,
    ) -> DeviceImage {
        DeviceImage::retained(RetainedImageMeta {
            width, height, domain, precision: Precision::F32, space, pixel_pitch_um,
        }, Arc::new(BufferPayload {
            backend_id: self.backend_id(), pages, bytes: u64::from(width) * u64::from(height) * 12,
            n_pixels: width * height, width, height,
        }))
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
    pub(in crate::wgpu_backend) fn materialize_buffer(&self, image: &DeviceImage) -> Result<ImageBuf, String> {
        let payload = self.buffer_payload(image)?;
        let mut samples = Vec::with_capacity((payload.bytes / 4) as usize);
        for page in &payload.pages {
            self.check_cancelled()?;
            samples.extend(f32_to_scalars(self.pipe.readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12)));
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
        if self.needs_pointwise_pages(image)? {
            return self.map_pointwise_pages(image, image.domain(), |page| self.scale_retained_image(page, scale));
        }
        let source = self.buffer_payload(image)?;
        let out = stage_output_buffer(&self.device, source.bytes, "retained_scaled");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&source.pages[0].buffer, 0, &out, 0, source.bytes);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("retained_scale"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.scale_pipeline().pipeline);
            pass.set_bind_group(
                0,
                &self.scale_bind_group(&out, scale, source.n_pixels),
                &[],
            );
            dispatch_linear(&mut pass, source.n_pixels);
        }
        self.queue.submit(Some(encoder.finish()));
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
        self.queue.submit(Some(encoder.finish()));
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

    fn scale_bind_group(&self, buffer: &wgpu::Buffer, scale: f32, n_pixels: u32) -> wgpu::BindGroup {
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
        if self.needs_pointwise_pages(image)? {
            return self.map_pointwise_pages(image, domain, |page| self.transfer_retained(page, decode, cctf_kind, matrix, domain));
        }
        let source = self.buffer_payload(image)?;
        let out = stage_output_buffer(&self.device, source.bytes, "retained_transfer");
        let mut encoder = self.device.create_command_encoder(&Default::default());
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
        self.queue.submit(Some(encoder.finish()));
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
        if params.strength.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return Ok(image.clone());
        }
        if self.needs_pointwise_pages(image)? {
            return self.map_pointwise_pages(image, image.domain(), |page| self.cmyk_retained(page, params));
        }
        let source = self.buffer_payload(image)?;
        let out = stage_output_buffer(&self.device, source.bytes, "retained_cmyk");
        let mut encoder = self.device.create_command_encoder(&Default::default());
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
        self.queue.submit(Some(encoder.finish()));
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
        if self.needs_pointwise_pages(image)? {
            return self.map_pointwise_pages(image, image.domain(), |page| self.matrix_retained(page, matrix));
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
                matrix[0][0], matrix[0][1], matrix[0][2],
                matrix[1][0], matrix[1][1], matrix[1][2],
                matrix[2][0], matrix[2][1], matrix[2][2],
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
        self.queue.submit(Some(encoder.finish()));
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
pub(in crate::wgpu_backend) fn stage_output_buffer(device: &ObservedDevice, bytes: u64, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

