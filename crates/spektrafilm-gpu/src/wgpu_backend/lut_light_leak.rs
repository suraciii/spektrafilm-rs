use super::highlight::{PointwiseAccess, pointwise_job};
use super::*;
use spektrafilm_backend::LutLightLeakGpuParams;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    n: u32,
    first_pixel: u32,
    image_width: u32,
    image_height: u32,
    gain: f32,
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    pos_x: f32,
    pos_y: f32,
    angle_deg: f32,
    width: f32,
    spread: f32,
    texture_amount: f32,
    texture_scale: f32,
    seed: u32,
}

impl WgpuBackend {
    pub(super) fn lut_light_leak_gpu(
        &self,
        img: &ImageBuf,
        params: &LutLightLeakGpuParams,
    ) -> Result<ImageBuf, String> {
        if img.width == 0 || img.height == 0 || params.gain <= 0.0 {
            return Ok(img.clone());
        }
        if params.tint.iter().all(|value| *value <= 0.0) {
            return Ok(img.clone());
        }
        let source = self.upload_rgb_pages(img, "lut_light_leak_source")?;
        let destination =
            self.allocate_image_pages(img.width, img.height, "lut_light_leak_destination")?;
        let n = u64::from(img.width) * u64::from(img.height);
        let job = pointwise_job(
            self,
            n,
            12,
            include_str!("lut_light_leak.wgsl"),
            &Params {
                n: 0,
                first_pixel: 0,
                image_width: img.width,
                image_height: img.height,
                gain: params.gain,
                tint_r: params.tint[0],
                tint_g: params.tint[1],
                tint_b: params.tint[2],
                pos_x: params.position[0],
                pos_y: params.position[1],
                angle_deg: params.angle_deg,
                width: params.width,
                spread: params.spread,
                texture_amount: params.texture_amount,
                texture_scale: params.texture_scale,
                seed: params.seed,
            },
            &[
                (&source, PointwiseAccess::Read),
                (&destination, PointwiseAccess::Write),
            ],
            Some(1),
            "lut_light_leak",
        )?;
        job.execute(self)?;
        self.read_pages(&destination, img.width, img.height)
    }
}
