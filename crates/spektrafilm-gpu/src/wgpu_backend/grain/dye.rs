//! The Dye grain field on the device.
//!
//! The engine's law is a continuous, band-limited field sampled on the film lattice;
//! `dye_law` implements it and `dye_law_paged` transports it through pages. This module
//! keeps the entry points the rest of the backend calls.

use super::*;

impl WgpuBackend {
    /// The developed density of a host image, with the engine's grain law applied.
    pub(in crate::wgpu_backend) fn dye_field(
        &self,
        image: &ImageBuf,
        params: &DyeGpuParams,
    ) -> Result<ImageBuf, String> {
        self.dye_law_field(
            image,
            params.source_pitch_um,
            &params.grid,
            params.seed,
            params.expectation,
            params.monochrome,
        )
    }

    /// Admission check for a device field request.
    pub(in crate::wgpu_backend) fn validate_dye_resources(
        &self,
        width: u32,
        height: u32,
        params: &DyeGpuParams,
    ) -> Result<(), String> {
        crate::wgpu_backend::grain::dye_law_paged::law_workspace_bytes(
            self,
            width,
            height,
            params.source_pitch_um,
        )
        .map(|_| ())
    }

    /// Band window plus published pages, the only memory the law adds.
    pub(in crate::wgpu_backend) fn dye_workspace_bytes(
        &self,
        width: u32,
        height: u32,
        params: &DyeGpuParams,
    ) -> Result<u64, String> {
        crate::wgpu_backend::grain::dye_law_paged::law_workspace_bytes(
            self,
            width,
            height,
            params.source_pitch_um,
        )
    }

    /// Copy a rectangle of a published field into its own pages.
    pub(in crate::wgpu_backend) fn dye_crop_pages(
        &self,
        source: &crate::wgpu_backend::film_chain::device_image::BufferPayload,
        rect: [u32; 4],
    ) -> Result<Vec<crate::wgpu_backend::film_chain::device_image::ImagePage>, String> {
        let [x, y, width, height] = rect;
        if x.checked_add(width).is_none_or(|end| end > source.width)
            || y.checked_add(height).is_none_or(|end| end > source.height)
        {
            return Err("film grain Dye: the field crop leaves the source raster".into());
        }
        let pages = self.allocate_image_pages(width, height, "dye_crop")?;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for row in 0..height {
            self.copy_samples_from(
                &mut encoder,
                &source.pages[0].buffer,
                (u64::from(y + row) * u64::from(source.width) + u64::from(x)) * 3,
                &pages,
                u64::from(row) * u64::from(width) * 3,
                u64::from(width) * 3,
            )?;
        }
        self.submit_commands(Some(encoder.finish()))?;
        Ok(pages)
    }

    /// The bounded validation word every device field publishes.
    pub(in crate::wgpu_backend) fn check_dye_status(
        &self,
        status: &wgpu::Buffer,
    ) -> Result<(), String> {
        self.check_cancelled()?;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_status_readback"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(status, 0, &readback, 0, 4);
        self.submit_commands(Some(encoder.finish()))?;
        let code = self.read_mapped_u32(&readback, 4)[0];
        self.check_cancelled()?;
        if code == 0 {
            return Ok(());
        }
        Err(format!("film grain Dye: device field failed ({code:#x})"))
    }
}
