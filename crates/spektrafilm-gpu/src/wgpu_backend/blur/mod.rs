use super::grain::RasterStorage;
use super::grain::raster::RasterCommands;
use super::paged::ImagePage;
use super::*;

/// One operation's sequential Gaussian working set, sized before any job is built.
/// Queue submission order permits reuse without a host wait; jobs retain their
/// own handles, and no allocation is shared with another operation.
pub(super) struct GaussianScratch {
    width: u32,
    height: u32,
    pages: Option<GaussianBuffers>,
}

#[derive(Default)]
pub(super) struct GaussianBuffers {
    pub(super) intermediate: Vec<ImagePage>,
    pub(super) output: Vec<ImagePage>,
}

impl GaussianScratch {
    pub(super) fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pages: None,
        }
    }

    /// Allocate only for an active Gaussian. A smaller raster gets its own
    /// exact page extent while retaining the maximum-capacity backing buffers.
    pub(super) fn buffers(
        &mut self,
        backend: &WgpuBackend,
        width: u32,
        height: u32,
    ) -> Result<GaussianBuffers, String> {
        let count = u64::from(width) * u64::from(height);
        if count == 0 || count > u64::from(self.width) * u64::from(self.height) {
            return Err("Gaussian scratch geometry exceeds its operation capacity".into());
        }
        if self.pages.is_none() {
            self.pages = Some(GaussianBuffers {
                intermediate: backend.allocate_image_pages(
                    self.width,
                    self.height,
                    "operation_gaussian_intermediate",
                )?,
                output: backend.allocate_image_pages(
                    self.width,
                    self.height,
                    "operation_gaussian_output",
                )?,
            });
        }
        let pages = self.pages.as_ref().expect("Gaussian pages allocated above");
        let view = |pages: &[ImagePage]| {
            pages
                .iter()
                .take_while(|page| u64::from(page.first_pixel) < count)
                .map(|page| ImagePage {
                    first_pixel: page.first_pixel,
                    n_pixels: u64::from(page.n_pixels).min(count - u64::from(page.first_pixel))
                        as u32,
                    buffer: page.buffer.clone(),
                })
                .collect()
        };
        Ok(GaussianBuffers {
            intermediate: view(&pages.intermediate),
            output: view(&pages.output),
        })
    }
}

/// One exact reference Gaussian over interleaved RGB storage pages.
///
/// The filter itself lives in `grain::spektra::blur`: FIR below sigma 3 with
/// half-sample reflection and the reference `floor(3σ + 0.5)` radius, the
/// Young-van Vliet recursion at and above 3, and a per-channel copy for a
/// non-positive sigma. There is no radius cap, so a large width is a recursion
/// rather than a truncated kernel.
pub(super) struct BlurJob {
    pub(super) state: crate::wgpu_backend::grain::spektra::blur::GrainBlurState,
    /// Caller-owned paged scratch; kept alive for the encoded work.
    pub(super) _scratch: Vec<ImagePage>,
}

/// Build an exact Gaussian job from source to output through `scratch`.
///
/// `sigma` is per channel; each channel uses the reference FIR below 3 and the
/// Young-van Vliet recursion at and above 3, with a copy for a non-positive
/// sigma. The source, scratch, and output storage must be distinct.
#[allow(clippy::too_many_arguments)]
pub(super) fn make_blur_job(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    sigma: [f64; 3],
    src: &(impl RasterStorage + ?Sized),
    scratch: &(impl RasterStorage + ?Sized),
    out: &(impl RasterStorage + ?Sized),
    label: &str,
) -> Result<BlurJob, String> {
    let source = src.pages();
    let destination = out.pages();
    let scratch = scratch.pages();
    let state = crate::wgpu_backend::grain::spektra::blur::build_grain_blur_paged_with_scratch(
        backend,
        width,
        height,
        &source,
        &destination,
        Some(&scratch),
        sigma,
        0,
        0,
    )
    .map_err(|reason| format!("{label} Gaussian blur: {reason}"))?;
    Ok(BlurJob {
        state,
        _scratch: scratch,
    })
}

impl BlurJob {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.state.submit(backend)
    }
}

pub(super) struct SimpleBlurState {
    dst: Vec<ImagePage>,
    blur: BlurJob,
    n_bytes: u64,
}

/// Blur `src` into `dst` and copy that result into the chain's main buffer.
///
/// `dst` and `scratch` are caller-owned so one chain frame reuses the same
/// allocations for the camera and scanner lens stages. Both must be distinct
/// from `src`, which the reference filter requires.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_simple_blur_state(
    backend: &WgpuBackend,
    sigma: f32,
    width: u32,
    height: u32,
    src: &(impl RasterStorage + ?Sized),
    dst: &(impl RasterStorage + ?Sized),
    scratch: &(impl RasterStorage + ?Sized),
    label: &str,
) -> Result<SimpleBlurState, String> {
    let n_bytes = (width as u64) * (height as u64) * 3 * 4;
    let blur = make_blur_job(
        backend,
        width,
        height,
        [f64::from(sigma); 3],
        src,
        scratch,
        dst,
        label,
    )?;
    Ok(SimpleBlurState {
        dst: dst.pages(),
        blur,
        n_bytes,
    })
}

impl SimpleBlurState {
    /// Complete the reference blur, then copy its output in bounded batches.
    pub(super) fn submit(
        &self,
        backend: &WgpuBackend,
        dst_main: &(impl RasterStorage + ?Sized),
    ) -> Result<(), String> {
        self.blur.submit(backend)?;
        let limit = super::bounded::storage_binding_limit(backend)?;
        let chunk_bytes = super::bounded::chunk_elements(limit, 12)? as u64 * 12;
        let mut first = 0;
        while first < self.n_bytes {
            backend.check_cancelled()?;
            let bytes = chunk_bytes.min(self.n_bytes - first);
            let mut copy = RasterCommands::default();
            copy.copy(&self.dst, first, dst_main, first, bytes);
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            copy.encode(&mut encoder);
            backend.submit_commands(Some(encoder.finish()))?;
            first += bytes;
        }
        Ok(())
    }
}
impl WgpuBackend {
    /// Device Gaussian blur through the reference FIR/IIR filter.
    pub fn gaussian_blur_gpu(&self, img: &ImageBuf, sigma: f32) -> Result<ImageBuf, String> {
        if sigma <= 0.0 {
            return Ok(img.clone());
        }
        if let Some(reason) = self.image_binding_limit_reason(img) {
            return Err(format!("gaussian_blur: {reason}"));
        }
        let (backend, _batch) = self.observed_batch("gaussian_blur");
        backend
            .try_gaussian_blur(img, sigma)
            .map_err(|reason| format!("gaussian_blur: {reason}"))
    }

    fn try_gaussian_blur(&self, img: &ImageBuf, sigma: f32) -> Result<ImageBuf, String> {
        #[cfg(test)]
        self.pipe.test_operation("gaussian_blur")?;
        let w = img.width;
        let h = img.height;
        let img_bytes = (w as u64) * (h as u64) * 3 * 4;
        let make_buf = |label: &str| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: img_bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let buf_in = make_buf("blur_in");
        let buf_out = make_buf("blur_out");
        let input_f32 = scalars_to_f32(&img.data);
        self.queue
            .write_buffer(&buf_in, 0, bytemuck::cast_slice(&input_f32));
        let blur = crate::wgpu_backend::grain::spektra::blur::build_grain_blur(
            self,
            w,
            h,
            &buf_in,
            &buf_out,
            [sigma as f64; 3],
            0,
            0,
        )?;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur_readback"),
            size: img_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        blur.submit(self)?;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&buf_out, 0, &readback, 0, img_bytes);
        self.submit_commands(Some(encoder.finish()))?;
        let out_f32 = self.read_mapped_f32(&readback, img_bytes);
        Ok(ImageBuf::from_data(w, h, f32_to_scalars(out_f32)))
    }

    pub fn gaussian_blur_multi_gpu(
        &self,
        img: &ImageBuf,
        sigmas: &[f32],
    ) -> Result<Vec<ImageBuf>, String> {
        if sigmas.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(reason) = self.image_binding_limit_reason(img) {
            return Err(format!("gaussian_blur_multi: {reason}"));
        }
        let (backend, _batch) = self.observed_batch("gaussian_blur_multi");
        backend
            .try_gaussian_blur_multi(img, sigmas)
            .map_err(|reason| format!("gaussian_blur_multi: {reason}"))
    }

    /// Blur `img` at every sigma with one upload and shared scratch, using
    /// bounded device submissions and one terminal readback per sigma.
    fn try_gaussian_blur_multi(
        &self,
        img: &ImageBuf,
        sigmas: &[f32],
    ) -> Result<Vec<ImageBuf>, String> {
        #[cfg(test)]
        self.pipe.test_operation("gaussian_blur_multi")?;
        use crate::wgpu_backend::grain::spektra::blur::build_grain_blur_with_scratch;
        let w = img.width;
        let h = img.height;
        let img_bytes = (w as u64) * (h as u64) * 3 * 4;
        let make_buf = |label: &str| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: img_bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let buf_in = make_buf("blur_multi_in");
        let scratch = make_buf("blur_multi_scratch");
        let input_f32 = scalars_to_f32(&img.data);
        self.queue
            .write_buffer(&buf_in, 0, bytemuck::cast_slice(&input_f32));
        let bufs_out: Vec<_> = (0..sigmas.len())
            .map(|i| make_buf(&format!("blur_multi_out_{i}")))
            .collect();
        let jobs: Vec<_> = sigmas
            .iter()
            .enumerate()
            .map(|(i, &sigma)| {
                build_grain_blur_with_scratch(
                    self,
                    w,
                    h,
                    &buf_in,
                    &bufs_out[i],
                    &scratch,
                    [sigma as f64; 3],
                    0,
                    0,
                )
            })
            .collect::<Result<_, _>>()?;
        let readbacks: Vec<_> = (0..sigmas.len())
            .map(|i| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("blur_multi_readback_{i}")),
                    size: img_bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        for ((job, readback), out) in jobs.iter().zip(&readbacks).zip(&bufs_out) {
            job.submit(self)?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(out, 0, readback, 0, img_bytes);
            self.submit_commands(Some(encoder.finish()))?;
        }
        for rb in &readbacks {
            let slice = rb.slice(..);
            slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
        }
        self.device.poll(wgpu::Maintain::Wait);
        let mut out_imgs = Vec::with_capacity(sigmas.len());
        for rb in &readbacks {
            let slice = rb.slice(..);
            let data = slice.get_mapped_range();
            self.device.materialized(img_bytes);
            let chunk: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
            out_imgs.push(ImageBuf::from_data(w, h, f32_to_scalars(chunk)));
            drop(data);
            rb.unmap();
        }
        Ok(out_imgs)
    }
}

#[cfg(all(test, not(feature = "precision-f64")))]
mod tests {
    use super::*;

    struct RestoreLimit(Option<std::ffi::OsString>);

    impl Drop for RestoreLimit {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(value) => std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", value),
                    None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
                }
            }
        }
    }

    #[test]
    fn sequential_gaussians_reuse_maximum_geometry_without_overwriting_results() {
        let gpu = crate::test_support::adapter();
        let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
        // The adapter lock serializes process-wide device-test limit overrides.
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "192");
        }
        let mut scratch = GaussianScratch::new(17, 9);
        assert!(
            scratch.pages.is_none(),
            "inactive optics must not allocate scratch"
        );
        let full = scratch.buffers(&gpu, 17, 9).unwrap();
        assert!(full.intermediate.len() > 1);
        assert!(scratch.buffers(&gpu, 18, 9).is_err());

        // Build every job before submission, just as the resident chain does.
        // The middle job models a scanner tail larger than its film source.
        let mut jobs = Vec::new();
        for (width, height, sigma) in [(7, 3, 0.8), (17, 9, 3.5), (9, 5, 1.7)] {
            let buffers = scratch.buffers(&gpu, width, height).unwrap();
            for (view, capacity) in [
                (&buffers.intermediate, &full.intermediate),
                (&buffers.output, &full.output),
            ] {
                assert_eq!(
                    view.iter().map(|page| page.n_pixels).sum::<u32>(),
                    width * height
                );
                for (page, original) in view.iter().zip(capacity) {
                    assert_eq!(
                        page.buffer, original.buffer,
                        "a job allocated fresh scratch"
                    );
                    assert!(page.n_pixels <= original.n_pixels);
                }
            }
            assert_ne!(buffers.output[0].buffer, buffers.intermediate[0].buffer);
            let values: Vec<f32> = (0..width * height * 3)
                .map(|i| (i % 31) as f32 * 0.04 - 0.3 + if i == 48 { 3.0 } else { 0.0 })
                .collect();
            let image = ImageBuf::from_data(width, height, values.clone());
            let expected = spektrafilm_math::gaussian::gaussian_blur(&image, f64::from(sigma));
            let source = gpu
                .allocate_image_pages(width, height, "sequential_gaussian_source")
                .unwrap();
            for page in &source {
                let start = page.first_pixel as usize * 3;
                let end = start + page.n_pixels as usize * 3;
                gpu.queue
                    .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&values[start..end]));
            }
            let state = build_simple_blur_state(
                &gpu,
                sigma,
                width,
                height,
                &source,
                &buffers.output,
                &buffers.intermediate,
                "sequential_gaussian",
            )
            .unwrap();
            jobs.push((state, source, expected));
        }
        for (state, source, _) in &jobs {
            state.submit(&gpu, source).unwrap();
        }
        // Read only after all reuse, so later Gaussian writes cannot hide a
        // premature overwrite of an earlier stage's completed output.
        for (_, source, expected) in jobs {
            let actual: Vec<f32> = source
                .iter()
                .flat_map(|page| {
                    gpu.pipe
                        .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12)
                })
                .collect();
            for (index, (&got, &want)) in actual.iter().zip(&expected.data).enumerate() {
                assert!(
                    got.is_finite() && (got - want).abs() < 2e-5,
                    "sample {index}: {got} != {want}"
                );
            }
        }
    }
}
