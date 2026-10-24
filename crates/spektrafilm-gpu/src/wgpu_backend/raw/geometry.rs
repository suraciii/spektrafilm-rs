use super::*;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GeometryPass {
    dimensions: [u32; 4],
    controls: [u32; 4],
    step_bits: [u32; 4],
    step: [f32; 4],
}

impl WgpuBackend {
    pub(super) fn raw_geometry_pass(
        &self,
        sensor: &RawSensor,
        input: &RawBuffer,
        rotate: bool,
    ) -> Result<Option<(RawBuffer, RawDispatch)>, String> {
        let (iw, ih, ow, oh, mode, axis, step) = if rotate {
            if sensor.fuji_width == 0 {
                return Ok(None);
            }
            let (ow, oh) = sensor.rotated_dimensions()?;
            (sensor.width, sensor.height, ow, oh, 0, 0, 0.5f64.sqrt())
        } else {
            if sensor.pixel_aspect == 1.0 {
                return Ok(None);
            }
            let (iw, ih) = sensor.rotated_dimensions()?;
            let (ow, oh) = sensor.stretched_dimensions()?;
            let vertical = sensor.pixel_aspect < 1.0;
            (
                iw,
                ih,
                ow,
                oh,
                1,
                u32::from(vertical),
                if vertical {
                    sensor.pixel_aspect
                } else {
                    1.0 / sensor.pixel_aspect
                },
            )
        };
        let bytes = u64::from(ow) * u64::from(oh) * 16;
        if let Some(reason) = required_buffer_exceeds_limits(bytes, &self.device.limits()) {
            return Err(format!("RAW geometry working buffer: {reason}"));
        }
        let high = step as f32;
        let bits = step.to_bits();
        let uniform = RawBuffer::upload(
            &self.device,
            "raw_geometry_params",
            bytemuck::bytes_of(&GeometryPass {
                dimensions: [iw, ih, ow, oh],
                controls: [
                    mode,
                    sensor.fuji_width.saturating_sub(1),
                    sensor.colors,
                    axis,
                ],
                step_bits: [bits as u32, (bits >> 32) as u32, 0, 0],
                step: [high, (step - f64::from(high)) as f32, 0.0, 0.0],
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let output = RawBuffer::zeros(&self.device, "raw_geometry_image", bytes);
        let count = if mode == 0 {
            ow * oh
        } else if axis == 1 {
            iw
        } else {
            ih
        };
        let pass = self.raw_dispatch(
            concat!(
                include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_geometry_common.wgsl"),
                include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_geometry.wgsl")
            ),
            &[&uniform, input, &output],
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            count,
            "raw_geometry",
        );
        Ok(Some((output, pass)))
    }

    pub(super) fn raw_camera_conversion(
        &self,
        sensor: &RawSensor,
        params: &RawGpuParams,
        input: &RawBuffer,
    ) -> Result<(RawBuffer, RawDispatch), String> {
        let (width, height) = sensor.rotated_dimensions()?;
        let uniform = RawBuffer::upload(
            &self.device,
            "raw_camera_params",
            bytemuck::bytes_of(&ColorPass {
                out_width: width,
                out_height: height,
                iwidth: width,
                iheight: height,
                flip: 0,
                colors: if sensor.mix_green && !params.green_merge {
                    3
                } else {
                    sensor.colors
                },
                adaptation_active: 0,
                _pad0: 0,
                tint: 1.0,
                _pad1: 0.0,
                _pad2: 0.0,
                _pad3: 0.0,
                camera_to_aces: params.camera_to_aces,
                adaptation: [[0.0; 4]; 3],
                aces_to_xyz: [[0.0; 4]; 3],
                xyz_to_aces: [[0.0; 4]; 3],
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let output = RawBuffer::zeros(
            &self.device,
            "raw_camera_rgb",
            u64::from(width) * u64::from(height) * 16,
        );
        let pass = self.raw_dispatch(
            concat!(
                include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
                include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_camera.wgsl")
            ),
            &[&uniform, input, &output],
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            width * height,
            "raw_camera_rgb",
        );
        Ok((output, pass))
    }
}
