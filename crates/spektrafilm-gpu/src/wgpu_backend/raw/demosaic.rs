//! Integer-domain ports of LibRaw 0.22.2's `misc_demosaic.cpp` and
//! `xtrans_demosaic.cpp`. CFA-derived tables and green bounds are computed on
//! the device; the host supplies only dispatch geometry and parsed metadata.
//!
//! AHD reuses the enclosing pipeline's established kernels. This module never
//! substitutes it for VNG, PPG, or Markesteijn; precoloured data needs no pass.
use super::*;
use spektrafilm_backend::RawDemosaic;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DemosaicPass {
    colors: u32,
    stage: u32,
    top: u32,
    left: u32,
}

const READ: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: true };
const WRITE: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: false };
const UNIFORM: wgpu::BufferBindingType = wgpu::BufferBindingType::Uniform;
const XTRANS_TILE: u32 = 512;
const XTRANS_STEP: u32 = XTRANS_TILE - 16;
pub(super) const XTRANS_SCRATCH_BYTES: u64 = 11_272_192 * 4;

impl WgpuBackend {
    /// Allocate AHD's candidate planes only for the Bayer-AHD selection.
    // Keep the frame metadata and independent shader bindings explicit at this boundary.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_ahd(
        &self,
        colors: u32,
        frame: &RawFrame,
        frame_buf: &RawBuffer,
        image: &RawBuffer,
        cfa: &RawBuffer,
        xyz: &RawBuffer,
        cbrt: &RawBuffer,
    ) -> Result<Vec<RawDispatch>, String> {
        let count = frame.width * frame.height;
        let plane_bytes = u64::from(count) * 4;
        let rgb_bytes = u64::from(count) * 12;
        for bytes in [plane_bytes, rgb_bytes] {
            if let Some(reason) = required_buffer_exceeds_limits(bytes, &self.device.limits()) {
                return Err(format!("RAW AHD working buffer: {reason}"));
            }
        }
        let green_h = RawBuffer::zeros(&self.device, "raw_green_h", plane_bytes);
        let green_v = RawBuffer::zeros(&self.device, "raw_green_v", plane_bytes);
        let rgb_h = RawBuffer::zeros(&self.device, "raw_rgb_h", rgb_bytes);
        let rgb_v = RawBuffer::zeros(&self.device, "raw_rgb_v", rgb_bytes);
        let homogeneity = RawBuffer::zeros(&self.device, "raw_homogeneity", plane_bytes);
        let mut passes = Vec::with_capacity(7);
        passes.push(self.raw_dispatch(
            BORDER_SHADER,
            &[frame_buf, image, cfa],
            &[UNIFORM, WRITE, READ],
            count,
            "raw_ahd_border",
        ));
        passes.push(self.raw_dispatch(
            GREEN_SHADER,
            &[frame_buf, image, cfa, &green_h, &green_v],
            &[UNIFORM, READ, READ, WRITE, WRITE],
            count,
            "raw_ahd_green",
        ));
        for direction in 0..2u32 {
            let settings = RawBuffer::upload(
                &self.device,
                "raw_ahd_pass",
                bytemuck::bytes_of(&AhdPass { direction, colors, _pad0: 0, _pad1: 0 }),
                wgpu::BufferUsages::UNIFORM,
            );
            let output = if direction == 0 { &rgb_h } else { &rgb_v };
            passes.push(self.raw_dispatch(
                RB_SHADER,
                &[frame_buf, &settings, image, cfa, &green_h, &green_v, output],
                &[UNIFORM, UNIFORM, READ, READ, READ, READ, WRITE],
                count,
                "raw_ahd_rb",
            ));
        }
        for direction in 0..2u32 {
            let settings = RawBuffer::upload(
                &self.device,
                "raw_ahd_pass",
                bytemuck::bytes_of(&AhdPass { direction, colors, _pad0: 0, _pad1: 0 }),
                wgpu::BufferUsages::UNIFORM,
            );
            passes.push(self.raw_dispatch(
                HOMOGENEITY_SHADER,
                &[frame_buf, &settings, &rgb_h, &rgb_v, xyz, cbrt, &homogeneity],
                &[UNIFORM, UNIFORM, READ, READ, READ, READ, WRITE],
                count,
                "raw_ahd_homogeneity",
            ));
        }
        passes.push(self.raw_dispatch(
            COMBINE_SHADER,
            &[frame_buf, &rgb_h, &rgb_v, &homogeneity, image],
            &[UNIFORM, READ, READ, READ, WRITE],
            count,
            "raw_ahd_combine",
        ));
        Ok(passes)
    }

    pub(super) fn demosaic_settings(&self, colors: u32, stage: u32, top: u32, left: u32) -> RawBuffer {
        RawBuffer::upload(
            &self.device,
            "raw_demosaic_settings",
            bytemuck::bytes_of(&DemosaicPass { colors, stage, top, left }),
            wgpu::BufferUsages::UNIFORM,
        )
    }

    fn raw_border_gpu(
        &self,
        colors: u32,
        border: u32,
        frame: &RawFrame,
        frame_buf: &RawBuffer,
        image: &RawBuffer,
        cfa: &RawBuffer,
    ) -> RawDispatch {
        let settings = self.demosaic_settings(colors, border, 0, 0);
        self.raw_dispatch(
            LINEAR_SHADER,
            &[frame_buf, &settings, image, cfa],
            &[UNIFORM, UNIFORM, WRITE, READ],
            frame.width * frame.height,
            "raw_demosaic_border",
        )
    }

    fn raw_linear_gpu(
        &self,
        colors: u32,
        frame: &RawFrame,
        frame_buf: &RawBuffer,
        image: &RawBuffer,
        cfa: &RawBuffer,
    ) -> Vec<RawDispatch> {
        let settings = self.demosaic_settings(colors, 0, 0, 0);
        vec![
            self.raw_border_gpu(colors, 1, frame, frame_buf, image, cfa),
            self.raw_dispatch(
                LINEAR_SHADER,
                &[frame_buf, &settings, image, cfa],
                &[UNIFORM, UNIFORM, WRITE, READ],
                frame.width * frame.height,
                "raw_linear",
            ),
        ]
    }

    /// Dispatch after `scale_colors` / `pre_interpolate`, before `mix_green`
    /// and output conversion. Encode the returned dispatches in order.
    ///
    /// The optional buffer is X-Trans's device-derived validation metadata.
    /// Its i32 at index 146 must be read after submission: nonzero means the
    /// same invalid pattern/bounds that native Markesteijn reports as corrupt.
    /// Reading that status is not host preparation or image arithmetic.
    // Each buffer is an independent kernel binding; a wrapper would only hide transport.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn raw_demosaic_gpu(
        &self,
        mode: RawDemosaic,
        colors: u32,
        frame: &RawFrame,
        frame_buf: &RawBuffer,
        image: &RawBuffer,
        cfa: &RawBuffer,
        xyz: &RawBuffer,
        cbrt: &RawBuffer,
    ) -> Result<(Vec<RawDispatch>, Option<RawBuffer>), String> {
        let count = frame.width * frame.height;
        match mode {
            RawDemosaic::None => Ok((Vec::new(), None)),
            RawDemosaic::BayerAhd => Ok((self.prepare_ahd(colors, frame, frame_buf, image, cfa, xyz, cbrt)?, None)),
            RawDemosaic::Vng => {
                // LibRaw 0.22.2 returns before even running linear on tiny data.
                if frame.width < 8 || frame.height < 8 {
                    return Ok((Vec::new(), None));
                }
                let mut passes = self.raw_linear_gpu(colors, frame, frame_buf, image, cfa);
                let result = RawBuffer::zeros(&self.device, "raw_vng_result", u64::from(count) * 16);
                for stage in 0..2 {
                    let settings = self.demosaic_settings(colors, stage, 0, 0);
                    passes.push(self.raw_dispatch(
                        VNG_SHADER,
                        &[frame_buf, &settings, image, cfa, &result],
                        &[UNIFORM, UNIFORM, WRITE, READ, WRITE],
                        count,
                        "raw_vng",
                    ));
                }
                Ok((passes, None))
            }
            RawDemosaic::FujiPpg => {
                let mut passes = vec![self.raw_border_gpu(colors, 3, frame, frame_buf, image, cfa)];
                for stage in 0..3 {
                    let settings = self.demosaic_settings(colors, stage, 0, 0);
                    passes.push(self.raw_dispatch(
                        PPG_SHADER,
                        &[frame_buf, &settings, image, cfa],
                        &[UNIFORM, UNIFORM, WRITE, READ],
                        count,
                        "raw_ppg",
                    ));
                }
                Ok((passes, None))
            }
            RawDemosaic::XTrans => {
                // These are native corruption preconditions, not a mode decline.
                if frame.width < XTRANS_TILE || frame.height < XTRANS_TILE {
                    return Err("LibRaw Markesteijn requires width and height of at least 512".into());
                }
                if frame.cfa_rows != 6 || frame.cfa_columns != 6 {
                    return Err("LibRaw Markesteijn requires a 6-by-6 X-Trans CFA".into());
                }
                if let Some(reason) = required_buffer_exceeds_limits(XTRANS_SCRATCH_BYTES, &self.device.limits()) {
                    return Err(reason);
                }
                let metadata = RawBuffer::zeros(&self.device, "raw_xtrans_metadata", 147 * 4);
                let scratch = RawBuffer::zeros(&self.device, "raw_xtrans_tile", XTRANS_SCRATCH_BYTES);
                let settings = self.demosaic_settings(colors, 0, 0, 0);
                let mut passes = vec![self.raw_dispatch(
                    XTRANS_PREPARE_SHADER,
                    &[frame_buf, &settings, image, cfa, &metadata],
                    &[UNIFORM, UNIFORM, WRITE, READ, WRITE],
                    1,
                    "raw_xtrans_prepare",
                )];
                // Native row-major tile order also preserves the in-place
                // overlap of the output image with subsequent input tiles.
                for top in (3..frame.height - 19).step_by(XTRANS_STEP as usize) {
                    for left in (3..frame.width - 19).step_by(XTRANS_STEP as usize) {
                        let settings = self.demosaic_settings(colors, 0, top, left);
                        passes.push(self.raw_dispatch(
                            XTRANS_SHADER,
                            &[frame_buf, &settings, image, cfa, &metadata, &scratch, xyz, cbrt],
                            &[UNIFORM, UNIFORM, WRITE, READ, READ, WRITE, READ, READ],
                            1,
                            "raw_xtrans_three_pass",
                        ));
                    }
                }
                passes.push(self.raw_border_gpu(colors, 8, frame, frame_buf, image, cfa));
                Ok((passes, Some(metadata)))
            }
        }
    }
}

const LINEAR_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_linear.wgsl"),
);
const VNG_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_vng.wgsl"),
);
const PPG_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_ppg.wgsl"),
);
const XTRANS_PREPARE_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_xtrans_prepare.wgsl"),
);
pub(super) const XTRANS_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    "fn image_index(p:i32)->u32{return u32(p);}\n",
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_xtrans.wgsl"),
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Invoke the demosaicer directly, before colour conversion can hide an
    /// integer rounding error or an accidentally overwritten measured sample.
    fn interpolate(
        mode: Option<RawDemosaic>,
        size: u32,
        pattern: &[u32],
        rows: u32,
        columns: u32,
        colors: u32,
        values: impl Fn(u32, u32, u32) -> u32,
    ) -> Vec<u32> {
        let gpu = crate::test_support::adapter();
        let mut frame: RawFrame = bytemuck::Zeroable::zeroed();
        frame.width = size;
        frame.height = size;
        frame.cfa_rows = rows;
        frame.cfa_columns = columns;
        let mut input = vec![0u32; (size * size * 4) as usize];
        for y in 0..size {
            for x in 0..size {
                let c = pattern[((y % rows) * columns + x % columns) as usize];
                input[((y * size + x) * 4 + c) as usize] = values(y, x, c);
            }
        }
        let frame_buf = RawBuffer::upload(&gpu.device, "test_raw_frame", bytemuck::bytes_of(&frame), wgpu::BufferUsages::UNIFORM);
        let image = RawBuffer::upload(&gpu.device, "test_raw_image", bytemuck::cast_slice(&input), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let cfa = RawBuffer::upload(&gpu.device, "test_raw_cfa", bytemuck::cast_slice(pattern), wgpu::BufferUsages::STORAGE);
        // Zero XYZ isolates camera colour while retaining native Lab's zero
        // origin. A zero-filled cbrt table instead yields L=-1024, unlike the
        // zeroed native scratch border, and changes edge homogeneity selection.
        let xyz = RawBuffer::zeros(&gpu.device, "test_raw_xyz", 3 * 16);
        let cbrt = RawBuffer::zeros(&gpu.device, "test_raw_cbrt", 65_536 * 4);
        let (passes, status) = match mode {
            Some(mode) => gpu.raw_demosaic_gpu(mode, colors, &frame, &frame_buf, &image, &cfa, &xyz, &cbrt).unwrap(),
            None => (gpu.raw_linear_gpu(colors, &frame, &frame_buf, &image, &cfa), None),
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.raw_dispatch(CBRT_SHADER,
            &[&cbrt], &[WRITE], 65_536, "test_raw_cbrt").encode(&mut encoder, "test_raw_cbrt");
        for pass in &passes {
            pass.encode(&mut encoder, "test_raw_demosaic");
        }
        gpu.queue.submit(Some(encoder.finish()));
        if let Some(status) = status {
            let metadata = gpu.pipe.readback_buffer_as::<i32>(&status.buffer, 147 * 4);
            assert_eq!(metadata[146], 0, "synthetic X-Trans pattern is valid");
        }
        gpu.pipe.readback_buffer_as::<u32>(&image.buffer, u64::from(size * size) * 16)
    }

    #[test]
    fn linear_keeps_native_reciprocal_truncation_and_border_division() {
        let pattern = [0, 1, 2, 3, 1, 2, 3, 0, 2, 3, 0, 1, 3, 0, 1, 2];
        let size = 12u32;
        let output = interpolate(None, size, &pattern, 4, 4, 4, |y, x, c| 7 + y * 193 + x * 29 + c * 11);
        for y in 0..size {
            for x in 0..size {
                let f = pattern[((y % 4) * 4 + x % 4) as usize];
                let border = x == 0 || y == 0 || x + 1 == size || y + 1 == size;
                for c in 0..4u32 {
                    let mut expected = 7 + y * 193 + x * 29 + c * 11;
                    if c != f {
                        let mut sum = 0;
                        let mut weight = 0;
                        for dy in -1i32..=1 {
                            for dx in -1i32..=1 {
                                let yy = y as i32 + dy;
                                let xx = x as i32 + dx;
                                if yy < 0 || xx < 0 || yy >= size as i32 || xx >= size as i32 {
                                    continue;
                                }
                                if pattern[((yy % 4) * 4 + xx % 4) as usize] != c {
                                    continue;
                                }
                                let w = if border { 1 } else { 1 << (u32::from(dy == 0) + u32::from(dx == 0)) };
                                sum += (7 + yy as u32 * 193 + xx as u32 * 29 + c * 11) * w;
                                weight += w;
                            }
                        }
                        expected = if weight == 0 { 0 } else if border { sum / weight } else { (sum * (256 / weight)) >> 8 };
                    }
                    assert_eq!(output[((y * size + x) * 4 + c) as usize], expected, "at ({y},{x}) channel {c}");
                }
            }
        }
    }

    #[test]
    fn vng_tiny_image_returns_before_linear_as_native_does() {
        let output = interpolate(Some(RawDemosaic::Vng), 7, &[0, 1, 3, 2], 2, 2, 4, |_, _, c| 101 + c * 37);
        for y in 0..7 {
            for x in 0..7 {
                let f = [0, 1, 3, 2][((y % 2) * 2 + x % 2) as usize];
                for c in 0..4 {
                    assert_eq!(output[((y * 7 + x) * 4 + c) as usize], if c == f { 101 + c * 37 } else { 0 });
                }
            }
        }
    }

    #[test]
    fn vng_four_color_retains_each_measured_channel() {
        let pattern = [0, 0, 1, 2, 3, 1, 2, 3, 0, 1, 2, 3, 1, 2, 3, 0];
        let size = 16;
        let output = interpolate(Some(RawDemosaic::Vng), size, &pattern, 4, 4, 4, |y, x, c| (y * 1731 + x * 997 + c * 103) & 65535);
        for y in 0..size {
            for x in 0..size {
                let f = pattern[((y % 4) * 4 + x % 4) as usize];
                assert_eq!(output[((y * size + x) * 4 + f) as usize], (y * 1731 + x * 997 + f * 103) & 65535);
            }
        }
        assert!(output.iter().all(|&v| v <= 65535));
    }

    #[test]
    fn ahd_preserves_constant_bayer_channels_including_green_and_border() {
        let output = interpolate(Some(RawDemosaic::BayerAhd), 32, &[0, 1, 1, 2], 2, 2, 3, |_, _, c| [101, 207, 311][c as usize]);
        for (index, pixel) in output.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(&pixel[..3], &[101, 207, 311], "AHD pixel {index}");
        }
    }

    #[test]
    fn ppg_preserves_constant_channels_through_all_three_passes() {
        let output = interpolate(Some(RawDemosaic::FujiPpg), 16, &[0, 1, 1, 2], 2, 2, 3, |_, _, c| [101, 207, 311][c as usize]);
        for pixel in output.as_chunks::<4>().0 {
            assert_eq!(&pixel[..3], &[101, 207, 311]);
        }
    }

    #[test]
    fn xtrans_retains_measured_red_in_every_edge_candidate() {
        let gpu = crate::test_support::adapter();
        let pattern = [1u32,0,1,1,2,1,2,1,2,0,1,0,1,0,1,1,2,1,1,2,1,1,0,1,0,1,0,2,1,2,1,2,1,1,0,1];
        let mut frame: RawFrame = bytemuck::Zeroable::zeroed();
        frame.width=512; frame.height=512; frame.cfa_rows=6; frame.cfa_columns=6;
        let mut input=vec![0u32;512*512*4];
        for y in 0..512 {for x in 0..512 {let c=pattern[(y%6)*6+x%6] as usize;input[(y*512+x)*4+c]=[101,207,311][c];}}
        let image=RawBuffer::upload(&gpu.device,"xtrans_edge_input",bytemuck::cast_slice(&input),wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC);
        let cfa=RawBuffer::upload(&gpu.device,"xtrans_edge_cfa",bytemuck::cast_slice(&pattern),wgpu::BufferUsages::STORAGE);
        let frame_buf=RawBuffer::upload(&gpu.device,"xtrans_edge_frame",bytemuck::bytes_of(&frame),wgpu::BufferUsages::UNIFORM);
        let metadata=RawBuffer::zeros(&gpu.device,"xtrans_edge_metadata",147*4);
        let scratch=RawBuffer::zeros(&gpu.device,"xtrans_edge_candidates",XTRANS_SCRATCH_BYTES);
        let xyz=RawBuffer::zeros(&gpu.device,"xtrans_edge_xyz",48);
        let cbrt=RawBuffer::zeros(&gpu.device,"xtrans_edge_cbrt",65536*4);
        let settings=gpu.demosaic_settings(3,0,0,0);
        let prepare=gpu.raw_dispatch(XTRANS_PREPARE_SHADER,&[&frame_buf,&settings,&image,&cfa,&metadata],&[UNIFORM,UNIFORM,WRITE,READ,WRITE],1,"xtrans_edge_prepare");
        let settings=gpu.demosaic_settings(3,0,3,3);
        let interpolate=gpu.raw_dispatch(XTRANS_SHADER,&[&frame_buf,&settings,&image,&cfa,&metadata,&scratch,&xyz,&cbrt],&[UNIFORM,UNIFORM,WRITE,READ,READ,WRITE,READ,READ],1,"xtrans_edge_interpolate");
        let mut encoder=gpu.device.create_command_encoder(&Default::default());
        gpu.raw_dispatch(CBRT_SHADER,&[&cbrt],&[WRITE],65536,"xtrans_edge_cbrt").encode(&mut encoder,"xtrans_edge_cbrt");
        prepare.encode(&mut encoder,"xtrans_edge_prepare");interpolate.encode(&mut encoder,"xtrans_edge_interpolate");gpu.queue.submit(Some(encoder.finish()));
        let values=gpu.pipe.readback_buffer_as::<u32>(&scratch.buffer,XTRANS_SCRATCH_BYTES);
        let image=gpu.pipe.readback_buffer_as::<u32>(&image.buffer,512*512*16);
        for d in 0..8 {assert_eq!(values[(d*262144+1540)*3],101,"measured edge red direction {d}");}
        assert_eq!(image[(6*512+7)*4],101);
    }

    #[test]
    fn markesteijn_three_pass_preserves_constant_xtrans_including_border() {
        let pattern = [
            1, 0, 1, 1, 2, 1,
            2, 1, 2, 0, 1, 0,
            1, 0, 1, 1, 2, 1,
            1, 2, 1, 1, 0, 1,
            0, 1, 0, 2, 1, 2,
            1, 2, 1, 1, 0, 1,
        ];
        let output = interpolate(Some(RawDemosaic::XTrans), 512, &pattern, 6, 6, 3, |_, _, c| [101, 207, 311][c as usize]);
        for (index, pixel) in output.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(&pixel[..3], &[101, 207, 311], "X-Trans pixel {index}");
        }
    }

    #[test]
    fn precolored_none_does_not_interpolate_missing_channels() {
        let output = interpolate(Some(RawDemosaic::None), 8, &[0], 1, 1, 3, |_, _, _| 619);
        for pixel in output.as_chunks::<4>().0 {
            assert_eq!(pixel, &[619, 0, 0, 0]);
        }
    }

    #[test]
    fn automatic_wb_rejects_whole_saturated_blocks_and_rounds_native_ratios() {
        let gpu = crate::test_support::adapter();
        let mut frame: RawFrame = bytemuck::Zeroable::zeroed();
        frame.width = 16;
        frame.height = 8;
        frame.cfa_rows = 2;
        frame.cfa_columns = 2;
        frame.channels = 1;
        frame.colors = 3;
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct ScaleParams {
            white: [f32; 4],
            maximum: u32,
            fixed_maximum: u32,
            threshold: f32,
            auto_white_balance: u32,
        }
        let params = ScaleParams { white: [1.0, 1.0, 1.0, 0.0], maximum: 1000, fixed_maximum: 1, threshold: 0.75, auto_white_balance: 1 };
        let pattern = [0u32, 1, 3, 2];
        let mut image = vec![0u32; 16 * 8 * 4];
        let mut sums = [0u32; 4];
        for y in 0..8usize {
            for x in 0..16usize {
                let c = pattern[(y % 2) * 2 + x % 2] as usize;
                let value = [97, 203, 311, 419][c] + (y * 3 + x * 7) as u32;
                image[(y * 16 + x) * 4 + c] = value;
                if x < 8 { sums[c] += value; }
            }
        }
        // One sample over maximum-25 rejects the entire right-hand8x8 block.
        image[8 * 4] = 976;
        let frame_buf = RawBuffer::upload(&gpu.device, "test_wb_frame", bytemuck::bytes_of(&frame), wgpu::BufferUsages::UNIFORM);
        let params_buf = RawBuffer::upload(&gpu.device, "test_wb_params", bytemuck::bytes_of(&params), wgpu::BufferUsages::UNIFORM);
        let image_buf = RawBuffer::upload(&gpu.device, "test_wb_image", bytemuck::cast_slice(&image), wgpu::BufferUsages::STORAGE);
        let cfa = RawBuffer::upload(&gpu.device, "test_wb_cfa", bytemuck::cast_slice(&pattern), wgpu::BufferUsages::STORAGE);
        let measured = RawBuffer::upload(&gpu.device, "test_wb_max", bytemuck::bytes_of(&976u32), wgpu::BufferUsages::STORAGE);
        let blocks = RawBuffer::zeros(&gpu.device, "test_wb_blocks", 2 * 8 * 4);
        let scales = RawBuffer::zeros(&gpu.device, "test_wb_scales", 16);
        let paged_scales = RawBuffer::zeros(&gpu.device, "test_wb_paged_scales", 16);
        let totals = RawBuffer::zeros(&gpu.device, "test_wb_totals", 64);
        let mut totals_passes = Vec::new();
        for mode in 0..2u32 {
            let mode = RawBuffer::upload(&gpu.device, "test_wb_mode", bytemuck::cast_slice(&[mode, 0, 0, 0]), wgpu::BufferUsages::UNIFORM);
            totals_passes.push(gpu.raw_dispatch(
                concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale_helpers.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_wb_totals.wgsl")),
                &[&params_buf, &measured, &paged_scales, &frame_buf, &blocks, &totals, &mode],
                &[UNIFORM, READ, WRITE, UNIFORM, READ, WRITE, UNIFORM],
                1,
                "test_auto_wb_totals",
            ));
        }
        let scan = gpu.raw_dispatch(
            concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale_helpers.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_auto_wb.wgsl")),
            &[&frame_buf, &params_buf, &measured, &image_buf, &cfa, &blocks],
            &[UNIFORM, UNIFORM, READ, READ, READ, WRITE],
            2,
            "test_auto_wb_scan",
        );
        let setup = gpu.raw_dispatch(
            concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale_helpers.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale_setup.wgsl")),
            &[&params_buf, &measured, &scales, &frame_buf, &blocks],
            &[UNIFORM, READ, WRITE, UNIFORM, READ],
            1,
            "test_auto_wb_scale",
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        scan.encode(&mut encoder, "test_auto_wb_scan");
        setup.encode(&mut encoder, "test_auto_wb_scale");
        for pass in &totals_passes {
            pass.encode(&mut encoder, "test_auto_wb_totals");
        }
        gpu.queue.submit(Some(encoder.finish()));
        let observed_blocks = gpu.pipe.readback_buffer_as::<u32>(&blocks.buffer, 64);
        assert_eq!(&observed_blocks[..4], &sums);
        assert_eq!(&observed_blocks[4..8], &[16; 4]);
        assert_eq!(&observed_blocks[8..], &[0; 8]);
        let observed = gpu.pipe.readback_buffer_as::<f32>(&scales.buffer, 16);
        let observed_paged = gpu.pipe.readback_buffer_as::<f32>(&paged_scales.buffer, 16);
        assert_eq!(observed.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), observed_paged.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        let white = sums.map(|sum| (16.0f64 / f64::from(sum)) as f32);
        let minimum = white.into_iter().fold(f32::INFINITY, f32::min);
        for c in 0..4 {
            let expected = (white[c] / minimum) * 65535.0 / 1000.0;
            assert_eq!(observed[c].to_bits(), expected.to_bits(), "native white balance channel {c}");
        }
    }
}
