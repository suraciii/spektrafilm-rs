//! Saving sample conversion on the device: writer primaries/transfer, output
//! clipping, and requested bit-depth packing.
//!
//! The conversion is one pass over retained samples (or one uploaded host
//! image), and its output already has the writer's sample type, so the final
//! readback hands the native writer exactly what it packs. No host stage
//! re-applies clipping or quantization, and no intermediate image is
//! materialized between the film chain and the writer.
//!
//! The pass is also exposed as builder + encode state
//! ([`WgpuBackend::encode_output_samples_buffer`]) so a resident route can
//! chain it after its last stage and read the typed payload once, at the
//! writer boundary.

use super::device_image::stage_output_buffer;
use super::*;
use spektrafilm_backend::{
    OutputSampleData, OutputSampleGpuParams, OutputSampleKind, OutputSampleValues,
};

/// `cctf_lib.wgsl` + the conversion/packing kernel. Concatenated by source so
/// the pipeline cache key is the resulting literal's address.
const OUTPUT_SAMPLE_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/output_sample.wgsl")
);

/// Samples packed into one output word for each writer sample type.
fn lanes(kind: OutputSampleKind) -> u32 {
    match kind {
        OutputSampleKind::U8 => 4,
        OutputSampleKind::U16 | OutputSampleKind::Half => 2,
        OutputSampleKind::F32 => 1,
    }
}

fn kind_code(kind: OutputSampleKind) -> u32 {
    match kind {
        OutputSampleKind::U8 => 0,
        OutputSampleKind::U16 => 1,
        OutputSampleKind::Half => 2,
        OutputSampleKind::F32 => 3,
    }
}

/// Word count and byte size of one packed conversion output.
fn output_words(n_samples: u64, kind: OutputSampleKind) -> u64 {
    n_samples.div_ceil(u64::from(lanes(kind)))
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SampleParams {
    n_samples: u32,
    src_cctf: u32,
    decode: u32,
    dst_cctf: u32,
    encode: u32,
    kind: u32,
    lanes: u32,
    n_words: u32,
    col0: [f32; 4],
    col1: [f32; 4],
    col2: [f32; 4],
}

impl WgpuBackend {
    /// Build and encode the saving conversion into a native writer payload
    /// buffer, without reading it back.
    ///
    /// `source` stays on the device: it is the retained output buffer or one
    /// upload of a host image. The returned buffer holds packed
    /// [`OutputSampleKind`] samples, so a resident route can submit its final
    /// job and read back only at the writer boundary.
    pub(super) fn encode_output_samples_buffer(
        &self,
        source: &wgpu::Buffer,
        n_pixels: u64,
        params: &OutputSampleGpuParams,
    ) -> Result<wgpu::Buffer, String> {
        self.check_cancelled()?;
        #[cfg(test)]
        self.pipe.test_operation("output_samples")?;
        let n_samples = n_pixels
            .checked_mul(3)
            .ok_or_else(|| "saving conversion: sample count overflow".to_owned())?;
        if n_samples == 0 {
            return Err("saving conversion: image has no samples".to_owned());
        }
        if n_samples > u64::from(u32::MAX) {
            return Err(format!(
                "saving conversion: {n_samples} samples exceed the device dispatch limit"
            ));
        }
        let source_bytes = n_samples * 4;
        if source.size() < source_bytes {
            return Err(format!(
                "saving conversion: source buffer holds {} bytes, {source_bytes} expected",
                source.size()
            ));
        }
        let lane_count = lanes(params.kind);
        let word_count = output_words(n_samples, params.kind);
        let word_bytes = word_count * 4;

        let uniform = SampleParams {
            n_samples: n_samples as u32,
            src_cctf: params.src_cctf,
            decode: u32::from(params.decode),
            dst_cctf: params.dst_cctf,
            encode: u32::from(params.encode),
            kind: kind_code(params.kind),
            lanes: lane_count,
            n_words: word_count as u32,
            col0: [
                params.matrix[0][0],
                params.matrix[1][0],
                params.matrix[2][0],
                0.0,
            ],
            col1: [
                params.matrix[0][1],
                params.matrix[1][1],
                params.matrix[2][1],
                0.0,
            ],
            col2: [
                params.matrix[0][2],
                params.matrix[1][2],
                params.matrix[2][2],
                0.0,
            ],
        };
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("output_sample_params"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        // The writer samples stay a plain storage buffer: `read_output_samples`
        // copies them into the pipe's pooled readback buffer, whose own usage
        // supplies `MAP_READ | COPY_DST`.
        let target = stage_output_buffer(&self.device, word_bytes, "output_samples");

        let cached = self.cached_pipeline(
            OUTPUT_SAMPLE_SHADER,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("output_sample_bind_group"),
            layout: &cached.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: source.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: target.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("output_samples"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&cached.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            dispatch_linear(&mut pass, word_count as u32);
        }
        self.check_cancelled()?;
        self.submit_commands(Some(encoder.finish()))?;
        Ok(target)
    }

    /// The one terminal readback of already converted writer samples.
    pub(super) fn read_output_samples(
        &self,
        words: &wgpu::Buffer,
        width: u32,
        height: u32,
        kind: OutputSampleKind,
    ) -> Result<OutputSampleData, String> {
        let n_samples = u64::from(width) * u64::from(height) * 3;
        let word_count = output_words(n_samples, kind);
        let word_bytes = word_count * 4;
        let packed: Vec<u32> = self.pipe.readback_buffer_cancellable(words, word_bytes)?;
        Ok(unpack_samples(kind, width, height, &packed))
    }

    /// Saving conversion of host samples: one upload, the same device pass,
    /// one typed readback for the writer.
    pub(in crate::wgpu_backend) fn output_samples_from_host(
        &self,
        image: &ImageBuf,
        params: &OutputSampleGpuParams,
    ) -> Result<OutputSampleData, String> {
        let n_pixels = u64::from(image.width) * u64::from(image.height);
        if image.data.len() as u64 != n_pixels * 3 {
            return Err(
                "saving conversion: image sample count does not match its dimensions".to_owned(),
            );
        }
        let mut values = empty_samples(params.kind);
        let pixels = super::super::bounded::chunk_elements(
            super::super::bounded::storage_binding_limit(self)?,
            12,
        )?;
        for samples in image.data.chunks(pixels * 3) {
            self.check_cancelled()?;
            let samples = scalars_to_f32(samples);
            let source = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("output_sample_input"),
                    contents: bytemuck::cast_slice(&samples),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                });
            let n = (samples.len() / 3) as u32;
            let words = self.encode_output_samples_buffer(&source, u64::from(n), params)?;
            append_samples(
                &mut values,
                self.read_output_samples(&words, n, 1, params.kind)?.values,
            );
        }
        self.check_cancelled()?;
        Ok(OutputSampleData {
            width: image.width,
            height: image.height,
            kind: params.kind,
            values,
        })
    }

    /// Saving conversion of a retained carrier. The payload's backend identity
    /// is re-checked, so a foreign device buffer is an error rather than a
    /// quiet host conversion.
    pub(in crate::wgpu_backend) fn output_samples_from_retained(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        params: &OutputSampleGpuParams,
    ) -> Result<OutputSampleData, String> {
        let payload = self.buffer_payload(image)?;
        let domain = image.domain();
        if matches!(
            domain,
            spektrafilm_backend::SampleDomain::Density
                | spektrafilm_backend::SampleDomain::LogExposure
        ) {
            return Err(
                "saving conversion: retained samples are diagnostic exposure/density, not output RGB".to_owned(),
            );
        }
        // The request, not the carrier, owns the transfer decision: the
        // pinned saving no-op must not decode an encoded source that already
        // carries the destination transfer state. The carrier's declared
        // state is checked against it so a mislabeled carrier fails instead
        // of being silently reinterpreted.
        let carrier_encoded = domain == spektrafilm_backend::SampleDomain::Encoded;
        if params.decode && !carrier_encoded {
            return Err(
                "saving conversion: the request decodes the source transfer but the carrier                  is already linear"
                    .to_owned(),
            );
        }
        let mut values = empty_samples(params.kind);
        for page in &payload.pages {
            self.check_cancelled()?;
            let words =
                self.encode_output_samples_buffer(&page.buffer, u64::from(page.n_pixels), params)?;
            append_samples(
                &mut values,
                self.read_output_samples(&words, page.n_pixels, 1, params.kind)?
                    .values,
            );
        }
        self.check_cancelled()?;
        Ok(OutputSampleData {
            width: image.width(),
            height: image.height(),
            kind: params.kind,
            values,
        })
    }
}

/// Unpack the conversion's words into the writer payload. Only bit extraction
/// happens here: the device already applied clipping and quantization.
fn unpack_samples(
    kind: OutputSampleKind,
    width: u32,
    height: u32,
    words: &[u32],
) -> OutputSampleData {
    let count = width as usize * height as usize * 3;
    let values = match kind {
        OutputSampleKind::U8 => OutputSampleValues::U8(
            (0..count)
                .map(|i| (words[i / 4] >> (8 * (i % 4))) as u8)
                .collect(),
        ),
        OutputSampleKind::U16 => OutputSampleValues::U16(
            (0..count)
                .map(|i| (words[i / 2] >> (16 * (i % 2))) as u16)
                .collect(),
        ),
        OutputSampleKind::Half => OutputSampleValues::Half(
            (0..count)
                .map(|i| (words[i / 2] >> (16 * (i % 2))) as u16)
                .collect(),
        ),
        OutputSampleKind::F32 => OutputSampleValues::F32(
            words
                .iter()
                .take(count)
                .map(|w| f32::from_bits(*w))
                .collect(),
        ),
    };
    OutputSampleData {
        width,
        height,
        kind,
        values,
    }
}

fn empty_samples(kind: OutputSampleKind) -> OutputSampleValues {
    match kind {
        OutputSampleKind::U8 => OutputSampleValues::U8(Vec::new()),
        OutputSampleKind::U16 => OutputSampleValues::U16(Vec::new()),
        OutputSampleKind::Half => OutputSampleValues::Half(Vec::new()),
        OutputSampleKind::F32 => OutputSampleValues::F32(Vec::new()),
    }
}

fn append_samples(destination: &mut OutputSampleValues, chunk: OutputSampleValues) {
    match (destination, chunk) {
        (OutputSampleValues::U8(a), OutputSampleValues::U8(b)) => a.extend(b),
        (OutputSampleValues::U16(a), OutputSampleValues::U16(b)) => a.extend(b),
        (OutputSampleValues::Half(a), OutputSampleValues::Half(b)) => a.extend(b),
        (OutputSampleValues::F32(a), OutputSampleValues::F32(b)) => a.extend(b),
        _ => unreachable!("device conversion chunks retain their requested sample kind"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::telemetry::ObservationContext;
    use spektrafilm_backend::{ComputeBackend, SampleDomain};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[test]
    fn output_page_cancellation_discards_partial_samples_and_keeps_device_reusable() {
        let gpu = crate::test_support::explicit_adapter();
        let values = [0.0f32, 0.25, 0.5, 0.75, 1.0, 0.125, 0.5, 0.0, 1.0];
        let pages = values
            .chunks(3)
            .enumerate()
            .map(|(index, samples)| super::super::super::paged::ImagePage {
                first_pixel: index as u32,
                n_pixels: 1,
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("cancelled_writer_page"),
                        contents: bytemuck::cast_slice(samples),
                        usage: wgpu::BufferUsages::STORAGE
                            | wgpu::BufferUsages::COPY_SRC
                            | wgpu::BufferUsages::COPY_DST,
                    }),
            })
            .collect();
        let image = gpu.retain_image_pages(
            pages,
            3,
            1,
            SampleDomain::Linear,
            spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
            None,
        );
        let params = OutputSampleGpuParams {
            src_cctf: 0,
            decode: false,
            dst_cctf: 0,
            encode: false,
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            kind: OutputSampleKind::F32,
        };
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
        let operation = gpu.with_cancellation(Some(cancellation.clone())).unwrap();
        let observed = operation
            .with_observation_context(ObservationContext::default())
            .unwrap();
        assert_eq!(
            observed
                .convert_retained_samples(&image, &params)
                .unwrap_err(),
            "operation cancelled"
        );
        assert_eq!(
            readbacks.load(Ordering::Relaxed),
            1,
            "cancelled writer must not read the next page"
        );
        gpu.pipe.test_on_readback(|| {});
        let replacement = observed
            .with_cancellation(Some(Arc::new(AtomicBool::new(false))))
            .unwrap();
        let cleared = observed.with_cancellation(None).unwrap();
        for backend in [gpu.backend(), replacement.as_ref(), cleared.as_ref()] {
            let samples = backend
                .convert_retained_samples(&image, &params)
                .unwrap()
                .unwrap();
            let OutputSampleValues::F32(actual) = samples.values else {
                panic!("expected float writer samples")
            };
            assert_eq!(actual, values);
        }
        assert_eq!(
            scalars_to_f32(&image.to_host(gpu.backend()).unwrap().data).as_ref(),
            values.as_slice()
        );
        assert!(
            cancellation.load(Ordering::Relaxed),
            "healthy reuse must not reset another operation's token"
        );
        gpu.pipe.test_fail_operation("output_samples");
        assert!(
            cleared
                .convert_retained_samples(&image, &params)
                .unwrap_err()
                .contains("injected device resource failure")
        );
        assert_eq!(gpu.pipe.test_failure_attempts(), 1);
    }
}
