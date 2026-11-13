use super::blur::GaussianScratch;
use super::highlight::{PointwiseAccess, pointwise_job};
use super::paged::ImagePage;
use super::*;
use spektrafilm_backend::LutHalationGpuParams;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaskParams {
    n: u32,
    low: f32,
    high: f32,
    transition: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct AddParams {
    n: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

impl WgpuBackend {
    fn upload_rgb_pages(&self, img: &ImageBuf, label: &str) -> Result<Vec<ImagePage>, String> {
        let pages = self.allocate_image_pages(img.width, img.height, label)?;
        for page in &pages {
            let start = page.first_pixel as usize * 3;
            let end = start + page.n_pixels as usize * 3;
            let samples = scalars_to_f32(&img.data[start..end]);
            self.queue
                .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&samples));
        }
        Ok(pages)
    }

    fn read_pages(&self, pages: &[ImagePage], width: u32, height: u32) -> Result<ImageBuf, String> {
        let mut samples = Vec::with_capacity((width as usize) * (height as usize) * 3);
        for page in pages {
            samples.extend(f32_to_scalars(self.pipe.readback_buffer_cancellable(
                &page.buffer,
                u64::from(page.n_pixels) * 12,
            )?));
        }
        Ok(ImageBuf::from_data(width, height, samples))
    }

    pub(super) fn lut_halation_gpu(
        &self,
        img: &ImageBuf,
        params: &LutHalationGpuParams,
    ) -> Result<ImageBuf, String> {
        if img.width == 0 || img.height == 0 {
            return Ok(img.clone());
        }
        let base = self.upload_rgb_pages(img, "lut_halation_base")?;
        let mask = self.allocate_image_pages(img.width, img.height, "lut_halation_mask")?;
        let original =
            self.allocate_image_pages(img.width, img.height, "lut_halation_mask_original")?;
        let n = u64::from(img.width) * u64::from(img.height);
        let make_mask = pointwise_job(
            self,
            n,
            12,
            include_str!("lut_halation_mask.wgsl"),
            &MaskParams {
                n: 0,
                low: params.mask_low,
                high: params.mask_high,
                transition: params.mask_transition,
            },
            &[
                (&base, PointwiseAccess::Read),
                (&mask, PointwiseAccess::Write),
                (&original, PointwiseAccess::Write),
            ],
            None,
            "lut_halation_mask",
        )?;
        make_mask.execute(self)?;

        let mut scratch = GaussianScratch::new(img.width, img.height);
        let state = super::halation::build_halation_state(
            &params.halation,
            img.width,
            img.height,
            &mask,
            &mut scratch,
            self,
        )?;
        state.submit(self)?;

        let add = pointwise_job(
            self,
            n,
            12,
            include_str!("lut_halation_add.wgsl"),
            &AddParams {
                n: 0,
                _pad0: 0,
                _pad1: 0,
                _pad2: 0,
            },
            &[
                (&base, PointwiseAccess::ReadWrite),
                (&mask, PointwiseAccess::Read),
                (&original, PointwiseAccess::Read),
            ],
            None,
            "lut_halation_add",
        )?;
        add.execute(self)?;
        self.read_pages(&base, img.width, img.height)
    }
}
