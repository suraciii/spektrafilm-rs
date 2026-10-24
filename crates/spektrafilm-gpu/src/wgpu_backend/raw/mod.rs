//! The device RAW sensor pipeline: LibRaw's post-unpack numerical stages on
//! WGPU.
//!
//! Native sensor metadata selects AHD, VNG (including linear preparation), PPG,
//! Markesteijn X-Trans, or precoloured processing. Numerical stages remain in
//! their native integer domains until the camera RGB bitmap boundary.
//!
//! The ingress completes float normalization, black removal, missing-site
//! correction, global maximum and automatic-WB reductions, and scale resolution
//! before interpolation. Geometry preserves Fuji rotation -> camera ushort ->
//! native stretch -> orientation/normalization. Lensfun follows orientation and
//! precedes CAT02/tint. The retained entry returns device pages; only explicit
//! materialization downloads output samples.
//!
//! Full-image bindings are used only when all relevant planes fit. Otherwise
//! `tiled_sensor`, `tiled_demosaic`, and `tiled_output` preserve full-source
//! reductions, CFA phase, interpolation halos, native X-Trans scan order, and
//! recurrent geometry using bounded pages and device-generated sparse gathers.
//! The host handles coefficients and fixed-size control/status metadata, not
//! image arithmetic. Numerical parity remains an exercised acceptance question,
//! especially for platform-specific native Lensfun approximations.

use super::*;
use spektrafilm_backend::{RawDemosaic, RawGpuParams, RawSensor};

use crate::wgpu_backend::film_chain::required_buffer_exceeds_limits;

mod demosaic;
mod geometry;
mod lens;
mod paged;
mod tiled_sensor;
mod tiled;
mod tiled_demosaic;
mod tiled_output;
mod float_normalization;
const CBRT_SHADER: &str = concat!(include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_cbrt.wgsl"));

/// LibRaw's `ACES_TO_XYZ` (ACES2065-1 → CIE XYZ): the constant
/// `raw_convert_to_rgb.wgsl` reads to reach XYZ before adapting.
const ACES_TO_XYZ: [[f32; 3]; 3] = [
    [0.9525523959, 0.0, 0.0000936786],
    [0.3439664498, 0.7281660966, -0.0721325464],
    [0.0, 0.0, 1.0088251844],
];

/// LibRaw's `XYZ_TO_ACES` (CIE XYZ → ACES2065-1): the constant
/// `raw_convert_to_rgb.wgsl` reads to return to ACES after adapting.
const XYZ_TO_ACES: [[f32; 3]; 3] = [
    [1.0498110175, 0.0, -0.0000974845],
    [-0.4959030231, 1.3733130458, 0.0982400361],
    [0.0, 0.0, 0.9912520182],
];

/// `raw_common.wgsl`'s `RawFrame`, in the shader's field order.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct RawFrame {
    width: u32,
    height: u32,
    cfa_rows: u32,
    cfa_columns: u32,
    black_rows: u32,
    black_columns: u32,
    green_merge: u32,
    has_black_map: u32,
    merge_row: u32,
    merge_column: u32,
    source_width: u32,
    source_height: u32,
    channels: u32,
    fuji_width: u32,
    fuji_layout: u32,
    colors: u32,
    subtract_black_map: u32,
    mix_green: u32,
}

/// `raw_ahd_rb.wgsl`'s `AhdPass`, in the shader's field order.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct AhdPass {
    direction: u32,
    colors: u32,
    _pad0: u32,
    _pad1: u32,
}

/// `raw_convert_to_rgb.wgsl`'s `ColorPass`, in the shader's field order.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ColorPass {
    out_width: u32,
    out_height: u32,
    iwidth: u32,
    iheight: u32,
    flip: u32,
    colors: u32,
    adaptation_active: u32,
    _pad0: u32,
    tint: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
    camera_to_aces: [[f32; 4]; 3],
    adaptation: [[f32; 4]; 3],
    aces_to_xyz: [[f32; 4]; 3],
    xyz_to_aces: [[f32; 4]; 3],
}


/// One prepared dispatch: a pipeline, its bind group, and the invocation
/// count the kernel walks.
pub(super) struct RawDispatch {
    pipeline: CachedPipelineRef,
    bind_group: wgpu::BindGroup,
    count: u32,
}

impl RawDispatch {
    fn encode(&self, encoder: &mut ObservedEncoder, label: &'static str) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        dispatch_linear(&mut pass, self.count);
    }
}

/// A device buffer, kept alive by the dispatch that binds it. Zeroed for a
/// scratch plane the device writes before any read.
pub(super) struct RawBuffer {
    buffer: wgpu::Buffer,
}

impl RawBuffer {
    fn upload(
        device: &ObservedDevice,
        label: &'static str,
        contents: &[u8],
        usage: wgpu::BufferUsages,
    ) -> Self {
        Self {
            buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            }),
        }
    }

    fn zeros(device: &ObservedDevice, label: &'static str, size: u64) -> Self {
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                mapped_at_creation: false,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            }),
        }
    }
}

impl WgpuBackend {
    /// Run LibRaw's post-unpack numerical stages on the device.
    ///
    /// The reference is `dcraw_process` with `user_qual = -1` for a
    /// three-colour Bayer sensor: black subtraction and the measured maximum,
    /// `scale_colors`' integer scaling, `pre_interpolate`'s green merge,
    /// `border_interpolate(5)`, AHD, and `convert_to_rgb`. The kernels own the
    /// arithmetic; this function prepares coefficients, allocates the working
    /// buffers, dispatches the stages in the reference's order, and reads back
    /// the oriented RGB plane.
    ///
    /// Returns `Err` when the request cannot be served exactly — an empty
    /// sensor, a zero CFA tile, a black map that disagrees with its declared
    /// extents, or a working buffer beyond the device's limits — rather than
    /// dispatching a stage over a wrong read set.
    fn raw_process_sensor_buffer(
        &self,
        sensor: &RawSensor,
        params: &RawGpuParams,
    ) -> Result<(wgpu::Buffer, u32, u32), String> {
        self.check_cancelled()?;
        let width = sensor.width;
        let height = sensor.height;
        let pixels = (width as u64)
            .checked_mul(height as u64)
            .ok_or("RAW sensor dimensions overflow")?;
        if pixels == 0 {
            return Err("RAW sensor has no active image".into());
        }
        if sensor.cfa_rows == 0 || sensor.cfa_columns == 0 {
            return Err("RAW sensor has no CFA pattern".into());
        }
        if sensor.cfa.len() as u64 != (sensor.cfa_rows as u64) * (sensor.cfa_columns as u64) {
            return Err("RAW sensor CFA tile disagrees with its declared extents".into());
        }
        if sensor.black_map.len() as u64
            != (sensor.black_map_rows as u64) * (sensor.black_map_columns as u64)
        {
            return Err("RAW sensor black map disagrees with its declared extents".into());
        }
        if sensor.colors == 0 || sensor.colors > 4 || sensor.cfa.iter().any(|&c| c > 3) {
            return Err("RAW sensor has invalid colour metadata".into());
        }
        let sample_sites = if sensor.fuji_width != 0 { u64::from(sensor.copied_width) * u64::from(sensor.copied_height) } else { pixels };
        if sensor.samples.len() as u64 != sample_sites * u64::from(sensor.layout.channels()) {
            return Err("RAW sensor sample plane disagrees with its layout and dimensions".into());
        }
        let (output_width, output_height) = sensor.output_dimensions()?;
        for bytes in [sensor.samples.len() as u64 * 4, pixels * 16,
            u64::from(output_width) * u64::from(output_height) * 12] {
            if let Some(reason) = required_buffer_exceeds_limits(bytes, &self.device.limits()) {
                return Err(format!("RAW image working buffer: {reason}"));
            }
        }
        let pixel_count = pixels as u32;
        let device = &self.device;
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;

        let frame = RawFrame {
            width,
            height,
            cfa_rows: sensor.cfa_rows,
            cfa_columns: sensor.cfa_columns,
            black_rows: sensor.black_map_rows,
            black_columns: sensor.black_map_columns,
            green_merge: u32::from(params.green_merge),
            has_black_map: u32::from(sensor.black_map_rows * sensor.black_map_columns > 0),
            merge_row: params.green_merge_row,
            merge_column: params.green_merge_column,
            source_width: if sensor.fuji_width != 0 { sensor.copied_width } else { width },
            source_height: if sensor.fuji_width != 0 { sensor.copied_height } else { height },
            channels: sensor.layout.channels(),
            fuji_width: sensor.fuji_width,
            fuji_layout: u32::from(sensor.fuji_layout),
            colors: sensor.colors,
            subtract_black_map: u32::from(sensor.layout.channels() > 1 || sensor.zero_is_bad),
            mix_green: u32::from(sensor.mix_green && !params.green_merge) | (u32::from(sensor.signed_foveon) << 1),
        };
        let frame_buf = RawBuffer::upload(
            device,
            "raw_frame",
            bytemuck::bytes_of(&frame),
            wgpu::BufferUsages::UNIFORM,
        );

        // `raw2image_ex`'s inputs: one f32 sample per site, the CFA code per
        // site, and the per-channel black offsets `adjust_bl` normalized.
        let samples = RawBuffer::upload(
            device,
            "raw_samples",
            bytemuck::cast_slice(&sensor.samples),
            storage,
        );
        let cfa: Vec<u32> = sensor.cfa.iter().map(|&c| u32::from(c)).collect();
        let cfa_buf = RawBuffer::upload(device, "raw_cfa", bytemuck::cast_slice(&cfa), storage);
        let black_offsets = if sensor.zero_is_bad { [0; 4] } else { sensor.channel_black };
        let channel_black = RawBuffer::upload(device, "raw_channel_black", bytemuck::cast_slice(&black_offsets), storage);
        // A storage buffer cannot be bound empty, and the scale stage only
        // reads the map when `has_black_map` is set, so a sensor with no
        // repeating black map gets one never-addressed zero element.
        let black_map_bytes: Vec<u8> = if sensor.black_map.is_empty() {
            vec![0u8; 4]
        } else {
            bytemuck::cast_slice(&sensor.black_map).to_vec()
        };
        let black_map = RawBuffer::upload(device, "raw_black_map", &black_map_bytes, storage);

        // The AHD homogeneity statistic's inputs: `cielab()`'s camera→XYZ
        // matrix, one vec4 per row, and its cube-root table.
        let xyz_camera: Vec<[f32; 4]> = sensor
            .xyz_camera
            .iter()
            .map(|row| [row[0], row[1], row[2], row[3]])
            .collect();
        let xyz_buf = RawBuffer::upload(
            device,
            "raw_xyz_camera",
            bytemuck::cast_slice(&xyz_camera),
            storage,
        );
        let cbrt_buf = RawBuffer::zeros(device, "raw_cbrt", 65_536 * 4);
        let cbrt_setup = self.raw_dispatch(CBRT_SHADER,
            &[&cbrt_buf], &[wgpu::BufferBindingType::Storage { read_only: false }], 65_536, "raw_cbrt");

        // The working planes: `image` is the reference's `ushort(*)[4]`, so
        // four u32 per site with one spare channel. The AHD candidate buffers
        // are its `ushort(*)[3]` planes, and the homogeneity map is one u32
        // per site holding both directions' counts.
        let image_bytes = pixels * 16;
        if let Some(reason) = required_buffer_exceeds_limits(image_bytes, &device.limits()) {
            return Err(format!("RAW sensor working buffer: {reason}"));
        }
        let image = RawBuffer::zeros(device, "raw_image", image_bytes);
        let max_value = RawBuffer::zeros(device, "raw_max_value", 4);

        let deblack = self.raw_dispatch(
            DEBLACK_SHADER,
            &[
                &frame_buf,
                &samples,
                &cfa_buf,
                &channel_black,
                &image,
                &max_value,
                &black_map,
            ],
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Storage { read_only: true },
            ],
            frame.source_width * frame.source_height,
            "raw_deblack",
        );
        let zeroes = if sensor.zero_is_bad {
            #[repr(C)]
            #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
            struct ZeroParams { black: [u32; 4], filters: u32, count: u32, pad: [u32; 2] }
            let uniform = RawBuffer::upload(device, "raw_zeroes_params", bytemuck::bytes_of(&ZeroParams {
                black: sensor.channel_black, filters: sensor.filters, count: pixel_count, pad: [0; 2],
            }), wgpu::BufferUsages::UNIFORM);
            Some(self.raw_dispatch(concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_zeroes.wgsl")),
                &[&frame_buf, &uniform, &image, &max_value, &black_map],
                &[wgpu::BufferBindingType::Uniform, wgpu::BufferBindingType::Uniform, wgpu::BufferBindingType::Storage { read_only: false }, wgpu::BufferBindingType::Storage { read_only: false }, wgpu::BufferBindingType::Storage { read_only: true }], 1, "raw_zeroes"))
        } else { None };

        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct ScaleParams { white: [f32; 4], maximum: u32, fixed_maximum: u32, threshold: f32, pad: u32 }
        let scale_params = RawBuffer::upload(device, "raw_scale_params", bytemuck::bytes_of(&ScaleParams {
            white: params.white_balance, maximum: params.maximum,
            fixed_maximum: u32::from(sensor.fixed_maximum), threshold: params.adjust_maximum_threshold, pad: u32::from(params.auto_white_balance),
        }), wgpu::BufferUsages::UNIFORM);
        let scale_buf = RawBuffer::zeros(device, "raw_scale_factors", 16);
        let blocks = width.div_ceil(8) * height.div_ceil(8);
        let block_sums = RawBuffer::zeros(device, "raw_wb_blocks", if params.auto_white_balance { u64::from(blocks) * 32 } else { 4 });
        let auto_wb = if params.auto_white_balance {
            Some(self.raw_dispatch(concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale_helpers.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_auto_wb.wgsl")),
                &[&frame_buf, &scale_params, &max_value, &image, &cfa_buf, &block_sums],
                &[wgpu::BufferBindingType::Uniform, wgpu::BufferBindingType::Uniform, wgpu::BufferBindingType::Storage { read_only: true }, wgpu::BufferBindingType::Storage { read_only: true }, wgpu::BufferBindingType::Storage { read_only: true }, wgpu::BufferBindingType::Storage { read_only: false }], blocks, "raw_auto_wb"))
        } else { None };
        let scale_setup = self.raw_dispatch(concat!(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale_helpers.wgsl"), include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale_setup.wgsl")),
            &[&scale_params, &max_value, &scale_buf, &frame_buf, &block_sums],
            &[wgpu::BufferBindingType::Uniform, wgpu::BufferBindingType::Storage { read_only: true }, wgpu::BufferBindingType::Storage { read_only: false }, wgpu::BufferBindingType::Uniform, wgpu::BufferBindingType::Storage { read_only: true }], 1, "raw_scale_setup");

        let scale_pass = self.raw_dispatch(
            SCALE_SHADER,
            &[&frame_buf, &scale_buf, &cfa_buf, &black_map, &image],
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            pixel_count,
            "raw_scale",
        );
        let (other_demosaic, demosaic_status) = if params.demosaic == RawDemosaic::BayerAhd {
            (self.prepare_ahd(sensor.colors, &frame, &frame_buf, &image, &cfa_buf, &xyz_buf, &cbrt_buf)?, None)
        } else {
            self.raw_demosaic_gpu(params.demosaic, sensor.colors, &frame, &frame_buf, &image, &cfa_buf, &xyz_buf, &cbrt_buf)?
        };
        let mix_green = if sensor.mix_green && !params.green_merge {
            let uniform = RawBuffer::upload(device, "raw_mix_green_params", bytemuck::bytes_of(&[pixel_count, 0, 0, 0]), wgpu::BufferUsages::UNIFORM);
            Some(self.raw_dispatch(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_mix_green.wgsl"), &[&uniform, &image],
                &[wgpu::BufferBindingType::Uniform, wgpu::BufferBindingType::Storage { read_only: false }], pixel_count, "raw_mix_green"))
        } else { None };
        let rotated = self.raw_geometry_pass(sensor, &image, true)?;
        let camera_input = rotated.as_ref().map_or(&image, |(buffer, _)| buffer);
        let (camera_rgb, camera_pass) = self.raw_camera_conversion(sensor, params, camera_input)?;
        let stretched = self.raw_geometry_pass(sensor, &camera_rgb, false)?;
        let color_input = stretched.as_ref().map_or(&camera_rgb, |(buffer, _)| buffer);
        let (iwidth, iheight) = sensor.stretched_dimensions()?;
        let (out_width, out_height) = sensor.output_dimensions()?;
        let color_pass = ColorPass {
            out_width,
            out_height,
            iwidth,
            iheight,
            flip: sensor.flip,
            colors: sensor.colors,
            adaptation_active: 0,
            _pad0: 0,
            tint: params.tint,
            _pad1: 0.0,
            _pad2: 0.0,
            _pad3: 0.0,
            camera_to_aces: params.camera_to_aces,
            adaptation: pack_rows(&params.adaptation),
            aces_to_xyz: pack_rows(&ACES_TO_XYZ),
            xyz_to_aces: pack_rows(&XYZ_TO_ACES),
        };
        let color_buf = RawBuffer::upload(
            device,
            "raw_color_pass",
            bytemuck::bytes_of(&color_pass),
            wgpu::BufferUsages::UNIFORM,
        );
        let output_bytes = (out_width as u64) * (out_height as u64) * 3 * 4;
        let output = RawBuffer::zeros(device, "raw_output", output_bytes);
        let convert = self.raw_dispatch(
            CONVERT_SHADER,
            &[&color_buf, color_input, &output],
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            out_width * out_height,
            "raw_convert_to_rgb",
        );
        let lens_output = params.lens.as_ref().map(|_| RawBuffer::zeros(device, "raw_lens_output", output_bytes));
        let lens_passes = match (&params.lens, &lens_output) {
            (Some(lens), Some(target)) => self.prepare_lens_passes(lens, &output, target),
            _ => Vec::new(),
        };
        let final_output = lens_output.as_ref().unwrap_or(&output);
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct Adaptation { matrix: [[f32; 4]; 3], count: u32, active: u32, tint: f32, pad: u32 }
        let adaptation_uniform = RawBuffer::upload(device, "raw_adaptation_params", bytemuck::bytes_of(&Adaptation {
            matrix: pack_rows(&params.adaptation), count: out_width * out_height,
            active: u32::from(params.adaptation_active), tint: params.tint, pad: 0,
        }), wgpu::BufferUsages::UNIFORM);
        let adaptation_pass = self.raw_dispatch(include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_adaptation.wgsl"),
            &[&adaptation_uniform, final_output], &[wgpu::BufferBindingType::Uniform, wgpu::BufferBindingType::Storage { read_only: false }], out_width * out_height, "raw_adaptation");

        let mut encoder = self.device.create_command_encoder(&Default::default());
        deblack.encode(&mut encoder, "raw_deblack");
        if let Some(pass) = &zeroes { pass.encode(&mut encoder, "raw_zeroes"); }
        if let Some(pass) = &auto_wb { pass.encode(&mut encoder, "raw_auto_wb"); }
        scale_setup.encode(&mut encoder, "raw_scale_setup");
        scale_pass.encode(&mut encoder, "raw_scale");
        cbrt_setup.encode(&mut encoder, "raw_cbrt");
        for dispatch in &other_demosaic {
            self.check_cancelled()?;
            dispatch.encode(&mut encoder, "raw_demosaic");
        }
        if let Some(pass) = &mix_green { pass.encode(&mut encoder, "raw_mix_green"); }
        if let Some((_, pass)) = &rotated { pass.encode(&mut encoder, "raw_fuji_rotate"); }
        camera_pass.encode(&mut encoder, "raw_camera_rgb");
        if let Some((_, pass)) = &stretched { pass.encode(&mut encoder, "raw_stretch"); }
        convert.encode(&mut encoder, "raw_convert_to_rgb");
        for pass in &lens_passes { pass.encode(&mut encoder, "raw_lens"); }
        adaptation_pass.encode(&mut encoder, "raw_adaptation");
        self.raw_submit(encoder)?;

        if let Some(status) = demosaic_status {
            let metadata = self.pipe.readback_buffer_cancellable::<i32>(&status.buffer, 147 * 4)?;
            if metadata.get(146).copied().unwrap_or(1) != 0 {
                return Err("LibRaw X-Trans interpolation rejected the CFA pattern".into());
            }
        }
        let output = lens_output.unwrap_or(output);
        self.check_cancelled()?;
        Ok((output.buffer, out_width, out_height))
    }

    pub(in crate::wgpu_backend) fn raw_process_sensor_gpu(&self, sensor: &RawSensor, params: &RawGpuParams) -> Result<ImageBuf, String> {
        let retained = self.raw_process_sensor_retained_gpu(sensor, params)?;
        self.materialize_buffer(&retained)
    }

    pub(in crate::wgpu_backend) fn raw_process_sensor_retained_gpu(&self, sensor: &RawSensor, params: &RawGpuParams) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.check_cancelled()?;
        let limit = super::bounded::storage_binding_limit(self)?;
        let (output_width, output_height) = sensor.output_dimensions()?;
        let large = u64::from(sensor.width) * u64::from(sensor.height) * 16 > limit
            || sensor.samples.len() as u64 * 4 > limit
            || u64::from(output_width) * u64::from(output_height) * 16 > limit
            || matches!(sensor.layout, spektrafilm_backend::RawSensorLayout::FloatCfa | spektrafilm_backend::RawSensorLayout::Float3 | spektrafilm_backend::RawSensorLayout::Float4);
        if large { return self.raw_process_paged(sensor, params); }
        let (buffer, width, height) = self.raw_process_sensor_buffer(sensor, params)?;
        self.check_cancelled()?;
        Ok(self.retain_buffer(buffer, width, height, u64::from(width) * u64::from(height) * 12,
            spektrafilm_backend::SampleDomain::Linear, &spektrafilm_math::colorspace::ACES2065_1, None))
    }

    /// Keep each RAW submission tied to this derived backend's operation token.
    fn raw_submit(&self, encoder: ObservedEncoder) -> Result<(), String> {
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        self.check_cancelled()
    }

    /// Compile one RAW pass and bind its buffers in the shader's order.
    pub(super) fn raw_dispatch(
        &self,
        shader: &'static str,
        buffers: &[&RawBuffer],
        types: &[wgpu::BufferBindingType],
        count: u32,
        label: &'static str,
    ) -> RawDispatch {
        let pipeline = self.cached_pipeline(shader, types);
        let entries: Vec<wgpu::BindGroupEntry> = buffers
            .iter()
            .enumerate()
            .map(|(i, buffer)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: buffer.buffer.as_entire_binding(),
            })
            .collect();
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &pipeline.layout,
            entries: &entries,
        });
        RawDispatch {
            pipeline,
            bind_group,
            count,
        }
    }
}

/// Widen each row-major 3×3 to the three vec4 rows the shader's
/// `array<vec4<f32>, 3>` members use, so `matrix[i]` is row `i` and `dot()`
/// over `.xyz` is the standard matrix-vector product.
fn pack_rows(matrix: &[[f32; 3]; 3]) -> [[f32; 4]; 3] {
    [
        [matrix[0][0], matrix[0][1], matrix[0][2], 0.0],
        [matrix[1][0], matrix[1][1], matrix[1][2], 0.0],
        [matrix[2][0], matrix[2][1], matrix[2][2], 0.0],
    ]
}

/// `raw2image_ex`'s inline copy and the `C.data_maximum` reduction.
const DEBLACK_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_deblack.wgsl"),
);

/// `scale_colors_loop` plus `pre_interpolate`'s green merge.
const SCALE_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_scale.wgsl"),
);

/// `border_interpolate(5)`.
const BORDER_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_border_interpolate.wgsl"),
);

/// `ahd_interpolate_green_h_and_v`.
const GREEN_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_ahd_green.wgsl"),
);

/// `ahd_interpolate_r_and_b_in_rgb_and_convert_to_cielab`, one direction per
/// dispatch.
const RB_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_ahd_rb.wgsl"),
);

/// `ahd_interpolate_build_homogeneity_map`, one direction per dispatch.
const HOMOGENEITY_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lab.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_ahd_homogeneity.wgsl"),
);

/// `ahd_interpolate_combine_homogeneous_pixels`.
const COMBINE_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_ahd_combine.wgsl"),
);

/// `convert_to_rgb`, the 16-bit quantization, and the orientation mapping.
const CONVERT_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_convert_to_rgb.wgsl"),
);

#[cfg(test)]
mod tests;
