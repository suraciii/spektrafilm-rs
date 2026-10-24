//! Pointwise carrier operations preserve page identity, not image geometry.
use super::paged::ImagePage;
use super::*;
use spektrafilm_backend::{DeviceImage, SampleDomain};
impl WgpuBackend {
    pub(super) fn mix_pointwise_pages(
        &self,
        result: &DeviceImage,
        source: &DeviceImage,
        params: &spektrafilm_backend::OutputMixGpuParams,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        let result_payload = self.buffer_payload(result)?;
        let source_payload = self.buffer_payload(source)?;
        let outputs = self.allocate_image_pages(result.width(), result.height(), "mixed_page")?;
        let mut pages = Vec::with_capacity(outputs.len());
        for page in outputs {
            self.check_cancelled()?;
            let source_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mix_source_page"),
                size: u64::from(page.n_pixels) * 12,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.check_cancelled()?;
            self.copy_samples_to(
                &mut encoder,
                result_payload,
                u64::from(page.first_pixel) * 3,
                u64::from(page.n_pixels) * 3,
                &page.buffer,
                0,
            )?;
            self.check_cancelled()?;
            self.copy_samples_to(
                &mut encoder,
                source_payload,
                u64::from(page.first_pixel) * 3,
                u64::from(page.n_pixels) * 3,
                &source_buffer,
                0,
            )?;
            self.check_cancelled()?;
            self.queue.submit(Some(encoder.finish()));
            #[cfg(test)]
            self.test_pointwise_submitted("mix_page_copy");
            self.check_cancelled()?;
            let a = self.retain_buffer(
                page.buffer,
                page.n_pixels,
                1,
                u64::from(page.n_pixels) * 12,
                result.domain(),
                result.space(),
                result.pixel_pitch_um(),
            );
            let b = self.retain_buffer(
                source_buffer,
                page.n_pixels,
                1,
                u64::from(page.n_pixels) * 12,
                source.domain(),
                source.space(),
                source.pixel_pitch_um(),
            );
            let mixed = self.mix_output_images(&a, &b, params)?;
            self.check_cancelled()?;
            let payload = self.buffer_payload(&mixed)?;
            pages.push(ImagePage {
                first_pixel: page.first_pixel,
                n_pixels: page.n_pixels,
                buffer: payload.pages[0].buffer.clone(),
            });
        }
        self.check_cancelled()?;
        Ok(self.retain_image_pages(
            pages,
            result.width(),
            result.height(),
            result.domain(),
            result.space(),
            result.pixel_pitch_um(),
        ))
    }

    pub(super) fn needs_pointwise_pages(&self, image: &DeviceImage) -> Result<bool, String> {
        let source = self.buffer_payload(image)?;
        Ok(source.pages.len() != 1 || source.bytes > super::bounded::storage_binding_limit(self)?)
    }
    pub(super) fn map_pointwise_pages(
        &self,
        image: &DeviceImage,
        domain: SampleDomain,
        operation: impl Fn(&DeviceImage) -> Result<DeviceImage, String>,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        let source = self.buffer_payload(image)?;
        let pages = self.allocate_image_pages(image.width(), image.height(), "pointwise_page")?;
        let mut outputs = Vec::with_capacity(pages.len());
        for page in pages {
            self.check_cancelled()?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.check_cancelled()?;
            self.copy_samples_to(
                &mut encoder,
                source,
                u64::from(page.first_pixel) * 3,
                u64::from(page.n_pixels) * 3,
                &page.buffer,
                0,
            )?;
            self.check_cancelled()?;
            self.queue.submit(Some(encoder.finish()));
            #[cfg(test)]
            self.test_pointwise_submitted("pointwise_page_copy");
            self.check_cancelled()?;
            let carrier = self.retain_image_pages(
                vec![ImagePage {
                    first_pixel: 0,
                    n_pixels: page.n_pixels,
                    buffer: page.buffer,
                }],
                page.n_pixels,
                1,
                image.domain(),
                image.space(),
                image.pixel_pitch_um(),
            );
            let result = operation(&carrier)?;
            self.check_cancelled()?;
            let payload = self.buffer_payload(&result)?;
            if payload.pages.len() != 1 || payload.n_pixels != page.n_pixels {
                return Err("pointwise device operation changed its page footprint".to_owned());
            }
            outputs.push(ImagePage {
                first_pixel: page.first_pixel,
                n_pixels: page.n_pixels,
                buffer: payload.pages[0].buffer.clone(),
            });
        }
        self.check_cancelled()?;
        Ok(self.retain_image_pages(
            outputs,
            image.width(),
            image.height(),
            domain,
            image.space(),
            image.pixel_pitch_um(),
        ))
    }
}

// Run at the real submission boundary, including through derived backends.
// Thread-local storage keeps this test-only observer scoped to its caller.
#[cfg(test)]
type PointwiseSubmitHook = Box<dyn FnMut(&'static str)>;

#[cfg(test)]
thread_local! {
    static POINTWISE_SUBMIT_HOOK: std::cell::RefCell<Option<PointwiseSubmitHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
impl WgpuBackend {
    pub(super) fn test_clear_pointwise_submit() {
        POINTWISE_SUBMIT_HOOK.with(|slot| *slot.borrow_mut() = None);
    }

    pub(super) fn test_on_pointwise_submit(&self, hook: impl FnMut(&'static str) + 'static) {
        POINTWISE_SUBMIT_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }

    pub(super) fn test_pointwise_submitted(&self, operation: &'static str) {
        POINTWISE_SUBMIT_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().as_mut() {
                hook(operation);
            }
        });
    }
}
