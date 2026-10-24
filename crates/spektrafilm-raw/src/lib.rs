//! Reference RAW decode: linear 16-bit ACES AP0, followed by Lensfun and WB.
use serde::{Deserialize, Serialize};
use spektrafilm_backend::{ComputeBackend, RawDemosaic, RawGpuParams, RawSensor, RawSensorLayout};
use spektrafilm_math::{image::ImageBuf, precision::from_f32};
use std::{
    ffi::{CStr, CString, c_char},
    path::Path,
};

mod lens;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhiteBalance {
    #[default]
    AsShot,
    Daylight,
    Tungsten,
    Custom,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawOptions {
    pub white_balance: WhiteBalance,
    pub temperature: Option<f64>,
    pub tint: Option<f64>,
    pub lens_correction: bool,
}

#[derive(Debug)]
pub struct RawResult {
    /// Linear ACES2065-1 pixels; LibRaw applies camera orientation during decode.
    pub image: ImageBuf,
    pub lens_info: String,
    /// The exact version-tagged model selected during this decode's native
    /// correction, not a later lookup. None means correction was disabled.
    pub lens_model_identity: Option<String>,
}

/// RAW input whose numerical output remains on the selected compute device.
pub struct RetainedRawResult {
    pub image: spektrafilm_backend::DeviceImage,
    pub lens_info: String,
    /// The exact model selected while preparing native correction coefficients.
    /// None means correction was disabled, including on the retained route.
    pub lens_model_identity: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RawError {
    #[error("RAW input: {0}")]
    Io(#[from] std::io::Error),
    #[error("RAW decode: {0}")]
    Native(String),
    #[error("RAW white balance: {0}")]
    WhiteBalance(String),
}

/// The unpacked-sensor record returned by the native bridge. Field order and
/// types mirror `SfRawSensor` in `native/raw.cpp` exactly.
#[repr(C)]
#[derive(Debug)]
struct NativeSensor {
    samples: *mut f32,
    samples_length: u64,
    cfa: *mut u8,
    cfa_rows: u32,
    cfa_columns: u32,
    black_map: *mut u32,
    black_map_rows: u32,
    black_map_columns: u32,
    allocation_row_stride: u32,
    allocation_width: u32,
    allocation_height: u32,
    layout: i32,
    channels: u32,
    width: u32,
    height: u32,
    copied_width: u32,
    copied_height: u32,
    filters: u32,
    fuji_width: u32,
    fuji_layout: i32,
    flip: u32,
    pixel_aspect: f64,
    is_foveon: i32,
    colors: u32,
    black: u32,
    maximum: u32,
    data_maximum: u32,
    channel_black: [u32; 4],
    camera_multipliers: [f32; 4],
    default_multipliers: [f32; 4],
    rgb_camera: [[f32; 4]; 3],
    xyz_camera: [[f32; 4]; 3],
    white: [u16; 64],
    as_shot_wb_applied: i32,
    decoder_flags: i32,
    mix_green: i32,
    zero_is_bad: i32,
    raw_color: i32,
    signed_foveon: i32,
    float_maximum: f32,
}

unsafe extern "C" {
    fn sf_raw_decode(
        bytes: *const u8,
        length: usize,
        camera_wb: i32,
        pixels: *mut *mut f32,
        width: *mut u32,
        height: *mut u32,
        error: *mut c_char,
        error_length: usize,
    ) -> i32;
    fn sf_raw_unpack_sensor(
        bytes: *const u8,
        length: usize,
        result: *mut NativeSensor,
        error: *mut c_char,
        error_length: usize,
    ) -> i32;
    fn sf_raw_sensor_free(sensor: *mut NativeSensor);
    fn sf_raw_correct_lens(
        path: *const c_char,
        pixels: *mut f32,
        width: u32,
        height: u32,
        summary: *mut c_char,
        summary_length: usize,
        identity: *mut *mut c_char,
        error: *mut c_char,
        error_length: usize,
    ) -> i32;
    fn sf_raw_free(pixels: *mut f32);
    fn sf_raw_geometry_reference(input: *const f32, width: u32, height: u32,
        fuji_width: u32, aspect: f64, colors: u32, pixels: *mut *mut f32,
        out_width: *mut u32, out_height: *mut u32) -> i32;
    fn sf_raw_version() -> *const c_char;
    fn sf_raw_lens_model_identity(
        path: *const c_char,
        identity: *mut *mut c_char,
        error: *mut c_char,
        error_length: usize,
    ) -> i32;
    fn sf_raw_free_lens_identity(identity: *mut c_char);
    #[cfg(test)]
    fn sf_raw_float_reference(input: *const f32, count: u32, channels: u32, fmaximum: f32,
        metadata: *const u32, black_map: *const u32, output: *mut *mut f32,
        out_metadata: *mut u32, out_black_map: *mut u32) -> i32;
    #[cfg(test)]
    fn sf_raw_xtrans_reference(input: *const u32, width: u32, height: u32,
        pattern: *const u8, output: *mut *mut f32) -> i32;
    #[cfg(test)]
    fn sf_raw_ordered_xtrans_threading_reference() -> i32;
    #[cfg(test)]
    fn sf_raw_xtrans_threaded_reference(input: *const u32, width: u32, height: u32,
        pattern: *const u8, threads: i32, output: *mut *mut f32) -> i32;
}

pub fn decoder_version() -> String {
    unsafe {
        CStr::from_ptr(sf_raw_version())
            .to_string_lossy()
            .into_owned()
    }
}

/// Source-specific resolved Lensfun camera/lens model, using the correction
/// resolver and database loader. This is identity evidence, not a project asset.
pub fn lens_model_identity(path: &Path) -> Result<String, RawError> {
    let path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| RawError::Native("Path contains NUL".into()))?;
    let mut identity = std::ptr::null_mut();
    let mut error = [0 as c_char; 1024];
    let status = unsafe {
        sf_raw_lens_model_identity(
            path.as_ptr(),
            &mut identity,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status != 0 {
        return Err(RawError::Native(c_string(&error)));
    }
    let text = unsafe { CStr::from_ptr(identity).to_string_lossy().into_owned() };
    unsafe { sf_raw_free_lens_identity(identity) };
    Ok(text)
}

struct NativePixels(*mut f32);
impl Drop for NativePixels {
    fn drop(&mut self) {
        unsafe { sf_raw_free(self.0) };
    }
}

/// Invoke the linked native geometry methods on a synthetic four-channel
/// integer plane, for CPU/device boundary probes without camera-file fixtures.
pub fn geometry_reference(input: &[f32], width: u32, height: u32, fuji_width: u32, pixel_aspect: f64, colors: u32) -> Result<(Vec<f32>, u32, u32), RawError> {
    if width.checked_mul(height).and_then(|v| v.checked_mul(4)).map(|v| v as usize) != Some(input.len())
        || !(1..=4).contains(&colors) || !(2..=u16::MAX as u32).contains(&width)
        || !(2..=u16::MAX as u32).contains(&height) || !pixel_aspect.is_finite() || !(0.1..=10.0).contains(&pixel_aspect)
        || fuji_width >= height || input.iter().any(|&v| !v.is_finite() || !(0.0..=65535.0).contains(&v)) {
        return Err(RawError::Native("Invalid synthetic RAW geometry boundary".into()));
    }
    let mut output = std::ptr::null_mut();
    let (mut ow, mut oh) = (0, 0);
    let status = unsafe { sf_raw_geometry_reference(input.as_ptr(), width, height, fuji_width, pixel_aspect, colors, &mut output, &mut ow, &mut oh) };
    if status != 0 { return Err(RawError::Native("Native RAW geometry boundary failed".into())); }
    let owned = NativePixels(output);
    let values = unsafe { std::slice::from_raw_parts(owned.0, ow as usize * oh as usize * 4) }.to_vec();
    Ok((values, ow, oh))
}

struct OwnedSensor(NativeSensor);
impl Drop for OwnedSensor {
    fn drop(&mut self) {
        unsafe { sf_raw_sensor_free(&mut self.0) };
    }
}

/// Decode through LibRaw's CPU reference pipeline, never infer decoder support
/// from the path suffix.
pub fn load(path: &Path, options: &RawOptions) -> Result<RawResult, RawError> {
    let adaptation = options.adaptation()?;
    let bytes = std::fs::read(path)?;
    let mut pixels = std::ptr::null_mut();
    let (mut width, mut height) = (0, 0);
    let mut error = [0 as c_char; 1024];
    let status = unsafe {
        sf_raw_decode(
            bytes.as_ptr(),
            bytes.len(),
            i32::from(options.white_balance == WhiteBalance::AsShot),
            &mut pixels,
            &mut width,
            &mut height,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status != 0 {
        return Err(RawError::Native(c_string(&error)));
    }
    let owned = NativePixels(pixels);
    let count = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(3))
        .ok_or_else(|| RawError::Native("Image dimensions overflow".into()))?;
    let floats = unsafe { std::slice::from_raw_parts_mut(owned.0, count) };
    let mut lens_info = String::new();
    let mut lens_model_identity = None;
    if options.lens_correction {
        let path = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| RawError::Native("Path contains NUL".into()))?;
        let mut summary = [0 as c_char; 1024];
        let mut identity = std::ptr::null_mut();
        let status = unsafe {
            sf_raw_correct_lens(
                path.as_ptr(),
                floats.as_mut_ptr(),
                width,
                height,
                summary.as_mut_ptr(),
                summary.len(),
                &mut identity,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if status != 0 {
            return Err(RawError::Native(c_string(&error)));
        }
        lens_info = c_string(&summary);
        lens_model_identity =
            Some(unsafe { CStr::from_ptr(identity).to_string_lossy().into_owned() });
        unsafe { sf_raw_free_lens_identity(identity) };
    }
    if let Some(matrix) = adaptation {
        for rgb in floats.as_chunks_mut::<3>().0 {
            let xyz = mv(ACES_TO_XYZ, [rgb[0] as f64, rgb[1] as f64, rgb[2] as f64]);
            let output = mv(XYZ_TO_ACES, mv(matrix, xyz));
            for c in 0..3 {
                rgb[c] = output[c] as f32;
            }
        }
    }
    if matches!(options.white_balance, WhiteBalance::Custom) {
        let tint = options.tint.unwrap_or(1.0);
        // Match numpy.isclose(tint, 1), including its default relative tolerance.
        if (tint - 1.0).abs() > 1.001e-5 {
            for rgb in floats.as_chunks_mut::<3>().0 {
                rgb[1] *= tint as f32;
            }
        }
    }
    let data = floats.iter().copied().map(from_f32).collect();
    Ok(RawResult {
        image: ImageBuf::from_data(width, height, data),
        lens_info,
        lens_model_identity,
    })
}

/// Unpack the sensor plane and metadata without running any numerical stage.
///
/// This is LibRaw's file parsing and compressed-sample unpacking only; no
/// `dcraw_process` call happens here. The returned record is the complete
/// input a device RAW pipeline consumes.
pub fn unpack(path: &Path) -> Result<RawSensor, RawError> {
    let bytes = std::fs::read(path)?;
    let mut native = NativeSensor {
        samples: std::ptr::null_mut(),
        samples_length: 0,
        cfa: std::ptr::null_mut(),
        cfa_rows: 0,
        cfa_columns: 0,
        black_map: std::ptr::null_mut(),
        black_map_rows: 0,
        black_map_columns: 0,
        allocation_row_stride: 0,
        allocation_width: 0,
        allocation_height: 0,
        layout: 0,
        channels: 0,
        width: 0,
        height: 0,
        copied_width: 0,
        copied_height: 0,
        filters: 0,
        fuji_width: 0,
        fuji_layout: 0,
        flip: 0,
        pixel_aspect: 1.0,
        is_foveon: 0,
        colors: 0,
        black: 0,
        maximum: 0,
        data_maximum: 0,
        channel_black: [0; 4],
        camera_multipliers: [0.0; 4],
        default_multipliers: [0.0; 4],
        rgb_camera: [[0.0; 4]; 3],
        xyz_camera: [[0.0; 4]; 3],
        white: [0; 64],
        as_shot_wb_applied: 0,
        decoder_flags: 0,
        mix_green: 0,
        zero_is_bad: 0,
        raw_color: 0,
        signed_foveon: 0,
        float_maximum: 0.0,
    };
    let mut error = [0 as c_char; 1024];
    let status = unsafe {
        sf_raw_unpack_sensor(
            bytes.as_ptr(),
            bytes.len(),
            &mut native,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status != 0 {
        return Err(RawError::Native(c_string(&error)));
    }
    let owned = OwnedSensor(native);
    let native = &owned.0;
    let layout = match native.layout {
        0 => RawSensorLayout::Cfa,
        1 => RawSensorLayout::Interleaved3,
        2 => RawSensorLayout::Interleaved4,
        3 => RawSensorLayout::FloatCfa,
        4 => RawSensorLayout::Float3,
        5 => RawSensorLayout::Float4,
        other => {
            return Err(RawError::Native(format!(
                "LibRaw returned unknown sensor layout {other}"
            )))
        }
    };
    if native.channels != layout.channels() {
        return Err(RawError::Native(
            "LibRaw sensor layout disagrees with its channel count".into(),
        ));
    }
    let (sample_width, sample_height) = if native.fuji_width != 0 {
        (native.copied_width, native.copied_height)
    } else {
        (native.width, native.height)
    };
    let sample_count = (sample_width as usize)
        .checked_mul(sample_height as usize)
        .and_then(|v| v.checked_mul(native.channels as usize))
        .ok_or_else(|| RawError::Native("Image dimensions overflow".into()))?;
    if sample_count != native.samples_length as usize || native.samples.is_null() {
        return Err(RawError::Native(
            "LibRaw returned an inconsistent sensor sample count".into(),
        ));
    }
    let samples = unsafe { std::slice::from_raw_parts(native.samples, sample_count) }.to_vec();
    let cfa_len = (native.cfa_rows as usize) * (native.cfa_columns as usize);
    let cfa = if cfa_len > 0 && !native.cfa.is_null() {
        unsafe { std::slice::from_raw_parts(native.cfa, cfa_len) }.to_vec()
    } else {
        return Err(RawError::Native(
            "LibRaw returned no CFA pattern for the sensor".into(),
        ));
    };
    let black_map_len = (native.black_map_rows as usize) * (native.black_map_columns as usize);
    let black_map = if black_map_len > 0 && !native.black_map.is_null() {
        unsafe { std::slice::from_raw_parts(native.black_map, black_map_len) }.to_vec()
    } else {
        Vec::new()
    };
    Ok(RawSensor {
        samples,
        layout,
        width: native.width,
        height: native.height,
        copied_width: native.copied_width,
        copied_height: native.copied_height,
        allocation_row_stride_bytes: native.allocation_row_stride * layout.channels()
            * if matches!(layout, RawSensorLayout::FloatCfa | RawSensorLayout::Float3 | RawSensorLayout::Float4) {
                4
            } else {
                2
            },
        filters: native.filters,
        cfa,
        cfa_rows: native.cfa_rows,
        cfa_columns: native.cfa_columns,
        fuji_width: native.fuji_width,
        fuji_layout: native.fuji_layout != 0,
        flip: native.flip,
        pixel_aspect: native.pixel_aspect,
        is_foveon: native.is_foveon != 0,
        colors: native.colors,
        black: native.black,
        maximum: native.maximum,
        channel_black: native.channel_black,
        black_map_rows: native.black_map_rows,
        black_map_columns: native.black_map_columns,
        black_map,
        camera_multipliers: native.camera_multipliers,
        default_multipliers: native.default_multipliers,
        rgb_camera: native.rgb_camera,
        xyz_camera: native.xyz_camera,
        white: native.white,
        as_shot_wb_applied: native.as_shot_wb_applied != 0,
        mix_green: native.mix_green != 0,
        fixed_maximum: native.decoder_flags & (1 << 8) != 0,
        zero_is_bad: native.zero_is_bad != 0,
        raw_color: native.raw_color != 0,
        signed_foveon: native.signed_foveon != 0,
        float_maximum: native.float_maximum,
    })
}

/// Resolve the device processing plan from the unpacked sensor and the
/// requested options.
///
/// Every value here is image-independent: the algorithm LibRaw's
/// `user_qual = -1` dispatch selects, the camera→ACES matrix
/// (`convert_to_rgb`'s `out_cam`), the white-balance multipliers
/// (`scale_colors`' selection), and the requested CAT02 adaptation. The device
/// resolves the measured sensor maximum and the resulting integer scale from
/// its own reduction, because that is image arithmetic.
pub fn plan(sensor: &RawSensor, options: &RawOptions) -> Result<RawGpuParams, RawError> {
    let adaptation = options.adaptation()?;
    let adaptation_active = adaptation.is_some();
    let adaptation = narrow_adaptation(adaptation);
    let camera_to_aces = camera_to_aces(sensor);
    let maximum = sensor.maximum.saturating_sub(sensor.black);
    // `pre_interpolate` visits `row = FC(1, 0) >> 1` then every other row, and
    // `col = FC(row, 1) & 1` then every other column; the packed pattern makes
    // both parities constant over the image.
    let green_merge = sensor.filters > 1000 && sensor.colors == 3;
    let green_merge_row = sensor.fc(1, 0) >> 1;
    let green_merge_column = sensor.fc(green_merge_row, 1) & 1;
    Ok(RawGpuParams {
        demosaic: demosaic_for(sensor),
        camera_to_aces,
        adaptation,
        adaptation_active,
        tint: tint_multiplier(options),
        adjust_maximum_threshold: 0.75,
        white_balance: if options.white_balance == WhiteBalance::AsShot && sensor.camera_multipliers[0] <= 0.00001 {
            sensor.default_multipliers
        } else {
            white_balance(sensor, options.white_balance == WhiteBalance::AsShot)
        },
        auto_white_balance: options.white_balance == WhiteBalance::AsShot && sensor.camera_multipliers[0] <= 0.00001,
        maximum,
        green_merge,
        green_merge_row,
        green_merge_column,
        lens: None,
    })
}

/// Decode `path` through the selected compute backend's RAW pipeline.
///
/// Once a GPU backend is selected, unavailable numerical work is an error for
/// automatic and explicit selections alike. CPU execution is selected only
/// before entering this loader, never as a hidden numerical fallback.
pub fn load_with_backend(
    path: &Path,
    options: &RawOptions,
    backend: &dyn ComputeBackend,
) -> Result<RawResult, RawError> {
    options.validate()?;
    if !backend.is_gpu() {
        return load(path, options);
    }
    decode_on_device(path, options, backend)
}

/// Decode RAW without materializing a selected GPU route's output samples.
/// Missing device capability is an error after GPU selection, not permission
/// to execute numerical image stages on the host.
pub fn load_retained_with_backend(
    path: &Path,
    options: &RawOptions,
    backend: &dyn ComputeBackend,
) -> Result<RetainedRawResult, RawError> {
    options.validate()?;
    if !backend.is_gpu() {
        let result = load(path, options)?;
        let meta = spektrafilm_backend::RetainedImageMeta {
            width: result.image.width, height: result.image.height,
            domain: spektrafilm_backend::SampleDomain::Linear,
            precision: spektrafilm_backend::Precision::host(),
            space: &spektrafilm_math::colorspace::ACES2065_1, pixel_pitch_um: None,
        };
        return Ok(RetainedRawResult {
            image: spektrafilm_backend::DeviceImage::host(result.image, meta),
            lens_info: result.lens_info,
            lens_model_identity: result.lens_model_identity,
        });
    }
    let sensor = unpack(path)?;
    let mut params = plan(&sensor, options)?;
    let (lens_info, lens_model_identity) = if options.lens_correction {
        let (width, height) = sensor.output_dimensions().map_err(RawError::Native)?;
        let (correction, summary, identity) = lens::prepare(path, width, height)?;
        params.lens = correction;
        (summary, Some(identity))
    } else { (String::new(), None) };
    let image = backend.raw_process_sensor_retained(&sensor, &params)
        .map_err(RawError::Native)?
        .ok_or_else(|| RawError::Native("the selected backend has no retained RAW pipeline".into()))?;
    Ok(RetainedRawResult { image, lens_info, lens_model_identity })
}

fn decode_on_device(
    path: &Path,
    options: &RawOptions,
    backend: &dyn ComputeBackend,
) -> Result<RawResult, RawError> {
    let sensor = unpack(path)?;
    let mut params = plan(&sensor, options)?;
    let (lens_info, lens_model_identity) = if options.lens_correction {
        let (width, height) = sensor.output_dimensions().map_err(RawError::Native)?;
        let (correction, summary, identity) = lens::prepare(path, width, height)?;
        params.lens = correction;
        (summary, Some(identity))
    } else {
        (String::new(), None)
    };
    match backend
        .raw_process_sensor(&sensor, &params)
        .map_err(RawError::Native)?
    {
        Some(image) => Ok(RawResult {
            image,
            lens_info,
            lens_model_identity,
        }),
        None => Err(RawError::Native(
            "the selected backend has no device RAW pipeline".into(),
        )),
    }
}

/// The algorithm `dcraw_process` selects with `user_qual = -1`.
fn demosaic_for(sensor: &RawSensor) -> RawDemosaic {
    if sensor.filters == 0 {
        return RawDemosaic::None;
    }
    let quality = if sensor.fuji_width != 0 { 2 } else { 3 };
    // dcraw_process dispatches after pre_interpolate has cleared the second
    // green bit. Counting the original code3 falsely selects VNG for Bayer.
    let merge = sensor.filters > 1000 && sensor.colors == 3;
    let filters = if merge { sensor.filters & !((sensor.filters & 0x5555_5555) << 1) } else { sensor.filters };
    let fc = |row: u32, column: u32| {
        (filters >> ((((row << 1) & 14) | (column & 1)) << 1)) & 3
    };
    let mut real_colors = sensor.colors;
    let mut bad_bayer = false;
    if sensor.filters > 1000 {
        for row in 0..4 {
            for column in 0..8 {
                let site = color(sensor, row, column);
                real_colors = real_colors.max(if merge && site == 3 { 2 } else { site + 1 });
                bad_bayer |= fc(row, column) == fc(row + 1, column);
                bad_bayer |= fc(row, column) == fc(row, column + 1);
            }
        }
    }
    if sensor.colors > 3
        || real_colors > 3
        || bad_bayer
        || (sensor.filters != 9 && sensor.filters <= 1000)
    {
        RawDemosaic::Vng
    } else if quality == 2 && sensor.filters > 1000 {
        RawDemosaic::FujiPpg
    } else if sensor.filters == 9 {
        RawDemosaic::XTrans
    } else {
        RawDemosaic::BayerAhd
    }
}

/// LibRaw's `COLOR(row, col)`: `fcol` for irregular CFAs, `FCF` for Fuji
/// geometry, `FC` otherwise.
fn color(sensor: &RawSensor, row: u32, column: u32) -> u32 {
    if sensor.filters < 1000 {
        return u32::from(sensor.cfa_at(row, column));
    }
    if sensor.fuji_width == 0 {
        return sensor.fc(row, column);
    }
    let width = i64::from(sensor.fuji_width);
    let (row, column) = (i64::from(row), i64::from(column));
    let (r, c) = if sensor.fuji_layout {
        (width - 1 - column + (row >> 1), column + ((row + 1) >> 1))
    } else {
        (width - 1 + row - (column >> 1), row + ((column + 1) >> 1))
    };
    sensor.fc(r.max(0) as u32, c.max(0) as u32)
}

/// `convert_to_rgb`'s `out_cam` for ACES AP0 output.
fn camera_to_aces(sensor: &RawSensor) -> [[f32; 4]; 3] {
    if sensor.raw_color {
        return [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    }
    let mut out = sensor.rgb_camera;
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            if (j as u32) < sensor.colors {
                let mut accumulated = 0.0f32;
                for (k, aces) in ACES_RGB[i].iter().enumerate() {
                    accumulated += (*aces * f64::from(sensor.rgb_camera[k][j])) as f32;
                }
                *value = accumulated;
            }
        }
    }
    out
}

/// `scale_colors`' multiplier selection, before normalization by the smallest
/// multiplier and the saturation level.
fn white_balance(sensor: &RawSensor, camera_wb: bool) -> [f32; 4] {
    let mut multipliers = sensor.default_multipliers;
    if camera_wb && sensor.camera_multipliers[0] > 0.00001 {
        let mut sums = [0u32; 8];
        for row in 0..8u32 {
            for column in 0..8u32 {
                let channel = (sensor.fc(row, column) & 3) as usize;
                // Black offsets have already been consumed before scale_colors.
                let value = i64::from(sensor.white[(row * 8 + column) as usize]);
                if value > 0 {
                    sums[channel] += value as u32;
                }
                sums[channel + 4] += 1;
            }
        }
        if sensor.as_shot_wb_applied {
            multipliers = [1.0; 4];
        } else if sums[0] != 0 && sums[1] != 0 && sums[2] != 0 && sums[3] != 0 {
            for channel in 0..4 {
                multipliers[channel] = sums[channel + 4] as f32 / sums[channel] as f32;
            }
        } else if sensor.camera_multipliers[0] > 0.00001 && sensor.camera_multipliers[2] > 0.00001
        {
            multipliers = sensor.camera_multipliers;
        }
    }
    if sensor.as_shot_wb_applied && !camera_wb && sensor.camera_multipliers[..3].iter().all(|&v| v > 0.00001) {
        for (multiplier, &camera_multiplier) in multipliers[..3].iter_mut().zip(&sensor.camera_multipliers) {
            *multiplier /= camera_multiplier;
        }
    }
    if multipliers[1] == 0.0 {
        multipliers[1] = 1.0;
    }
    if multipliers[3] == 0.0 {
        multipliers[3] = if sensor.colors < 4 {
            multipliers[1]
        } else {
            1.0
        };
    }
    multipliers
}

/// The custom green tint the CPU reference applies after adaptation.
fn tint_multiplier(options: &RawOptions) -> f32 {
    if !matches!(options.white_balance, WhiteBalance::Custom) {
        return 1.0;
    }
    let tint = options.tint.unwrap_or(1.0);
    // Match numpy.isclose(tint, 1), including its default relative tolerance.
    if (tint - 1.0).abs() > 1.001e-5 {
        tint as f32
    } else {
        1.0
    }
}

fn narrow_adaptation(matrix: Option<[[f64; 3]; 3]>) -> [[f32; 3]; 3] {
    match matrix {
        Some(matrix) => matrix.map(|row| row.map(|value| value as f32)),
        None => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    }
}

fn c_string(buffer: &[c_char]) -> String {
    unsafe {
        CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

impl RawOptions {
    pub fn validate(&self) -> Result<(), RawError> {
        self.adaptation().map(|_| ())
    }

    fn adaptation(&self) -> Result<Option<[[f64; 3]; 3]>, RawError> {
        let temperature = match self.white_balance {
            WhiteBalance::AsShot | WhiteBalance::Daylight => return Ok(None),
            WhiteBalance::Tungsten => 2850.0,
            WhiteBalance::Custom => self.temperature.ok_or_else(|| {
                RawError::WhiteBalance(
                    "A custom raw white balance requires a temperature value.".into(),
                )
            })?,
        };
        if !temperature.is_finite() || !(1667.0..=25000.0).contains(&temperature) {
            return Err(RawError::WhiteBalance("Temperature must be finite and between 1667 and 25000 K (CIE D / Kang 2002 domain).".into()));
        }
        if self.white_balance == WhiteBalance::Custom && self.tint.is_some_and(|v| !v.is_finite()) {
            return Err(RawError::WhiteBalance("Tint must be finite.".into()));
        }
        let source = whitepoint_xyz(temperature);
        let target = whitepoint_xyz(6504.0);
        if source
            .iter()
            .zip(target)
            .all(|(a, b)| (a - b).abs() <= 1e-8 + 1e-5 * b.abs())
        {
            return Ok(None);
        }
        // colour.chromatic_adaptation(method='Von Kries') defaults to CAT02.
        let s = mv(CAT02, source);
        let t = mv(CAT02, target);
        let mut scaled = CAT02;
        for r in 0..3 {
            let factor = t[r] / s[r];
            for value in &mut scaled[r] {
                *value *= factor;
            }
        }
        Ok(Some(mm(inverse(CAT02), scaled)))
    }
}

pub fn whitepoint_xyz(t: f64) -> [f64; 3] {
    let x = if t >= 4000.0 {
        if t <= 7000.0 {
            -4.6070e9 / t.powi(3) + 2.9678e6 / t.powi(2) + 0.09911e3 / t + 0.244063
        } else {
            -2.0064e9 / t.powi(3) + 1.9018e6 / t.powi(2) + 0.24748e3 / t + 0.237040
        }
    } else {
        -0.2661239e9 / t.powi(3) - 0.2343589e6 / t.powi(2) + 0.8776956e3 / t + 0.179910
    };
    let y = if t >= 4000.0 {
        -3.0 * x * x + 2.87 * x - 0.275
    } else if t <= 2222.0 {
        -1.1063814 * x.powi(3) - 1.34811020 * x * x + 2.18555832 * x - 0.20219683
    } else {
        -0.9549476 * x.powi(3) - 1.37418593 * x * x + 2.09137015 * x - 0.16748867
    };
    [x / y, 1.0, (1.0 - x - y) / y]
}
const ACES_TO_XYZ: [[f64; 3]; 3] = [
    [0.9525523959, 0.0, 0.0000936786],
    [0.3439664498, 0.7281660966, -0.0721325464],
    [0.0, 0.0, 1.0088251844],
];
const XYZ_TO_ACES: [[f64; 3]; 3] = [
    [1.0498110175, 0.0, -0.0000974845],
    [-0.4959030231, 1.3733130458, 0.0982400361],
    [0.0, 0.0, 0.9912520182],
];
const CAT02: [[f64; 3]; 3] = [
    [0.7328, 0.4296, -0.1624],
    [-0.7036, 1.6975, 0.0061],
    [0.003, 0.0136, 0.9834],
];
/// LibRaw's `LibRaw_constants::aces_rgb`: the ACES AP0 primaries adapted to
/// the D65 working white, exactly as `convert_to_rgb()` uses them.
const ACES_RGB: [[f64; 3]; 3] = [
    [0.43968015, 0.38295299, 0.17736686],
    [0.08978964, 0.81343316, 0.09677734],
    [0.01754827, 0.11156156, 0.87089017],
];
fn mv(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}
fn mm(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c])
    })
}
fn inverse(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let cof = std::array::from_fn::<_, 3, _>(|r| {
        std::array::from_fn::<_, 3, _>(|c| {
            m[(r + 1) % 3][(c + 1) % 3] * m[(r + 2) % 3][(c + 2) % 3]
                - m[(r + 1) % 3][(c + 2) % 3] * m[(r + 2) % 3][(c + 1) % 3]
        })
    });
    let determinant = m[0][0] * cof[0][0] + m[0][1] * cof[0][1] + m[0][2] * cof[0][2];
    std::array::from_fn(|r| std::array::from_fn(|c| cof[c][r] / determinant))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sensor(filters: u32, colors: u32) -> RawSensor {
        RawSensor {
            samples: Vec::new(),
            layout: RawSensorLayout::Cfa,
            width: 2,
            height: 2,
            copied_width: 2,
            copied_height: 2,
            allocation_row_stride_bytes: 4,
            filters,
            cfa: vec![0, 1, 1, 2],
            cfa_rows: 2,
            cfa_columns: 2,
            fuji_width: 0,
            fuji_layout: false,
            flip: 0,
            pixel_aspect: 1.0,
            is_foveon: false,
            colors,
            black: 0,
            maximum: 65535,
            channel_black: [0; 4],
            black_map_rows: 0,
            black_map_columns: 0,
            black_map: Vec::new(),
            camera_multipliers: [2.0, 1.0, 1.5, 1.0],
            default_multipliers: [1.0, 1.0, 1.0, 1.0],
            rgb_camera: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            xyz_camera: [[0.0; 4]; 3],
            white: [1000; 64],
            as_shot_wb_applied: false,
            mix_green: false,
            fixed_maximum: false,
            zero_is_bad: false,
            raw_color: false,
            signed_foveon: false,
            float_maximum: 0.0,
        }
    }

    /// `dcraw_process`'s `user_qual = -1` dispatch, one case per branch.
    #[test]
    fn demosaic_dispatch_matches_libraw_native_defaults() {
        // RGGB packing used by LibRaw's Bayer table rows.
        let rggb = 0x9494_9494u32;
        assert_eq!(demosaic_for(&sensor(rggb, 3)), RawDemosaic::BayerAhd);
        assert_eq!(demosaic_for(&sensor(0xe1e1_e1e1, 3)), RawDemosaic::BayerAhd);
        assert_eq!(demosaic_for(&sensor(0xe1e1_e1e1, 4)), RawDemosaic::Vng);
        assert_eq!(demosaic_for(&sensor(9, 3)), RawDemosaic::XTrans);
        assert_eq!(demosaic_for(&sensor(1, 3)), RawDemosaic::Vng);
        assert_eq!(demosaic_for(&sensor(0x4b4b_4b4b, 4)), RawDemosaic::Vng);
        // A Fuji-rotated sensor dispatches PPG when the CFA is a plain Bayer.
        let mut fuji = sensor(rggb, 3);
        fuji.fuji_width = 100;
        assert_eq!(demosaic_for(&fuji), RawDemosaic::FujiPpg);
        // ... and X-Trans still wins its own branch for `filters == 9`.
        let mut xtrans_fuji = sensor(9, 3);
        xtrans_fuji.fuji_width = 100;
        assert_eq!(demosaic_for(&xtrans_fuji), RawDemosaic::XTrans);
    }

    /// `scale_colors`: `highlight == 0` normalizes by the *smallest*
    /// multiplier, then by the black-subtracted saturation level.
    #[test]
    fn scale_multipliers_follow_reference_normalization() {
        let params = RawGpuParams {
            demosaic: RawDemosaic::BayerAhd,
            camera_to_aces: [[0.0; 4]; 3],
            adaptation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            adaptation_active: false,
            tint: 1.0,
            adjust_maximum_threshold: 0.75,
            white_balance: [2.0, 1.0, 1.5, 1.0],
            auto_white_balance: false,
            maximum: 16383,
            green_merge: true,
            green_merge_row: 1,
            green_merge_column: 0,
            lens: None,
        };
        let scale = params.scale_multipliers();
        // dmin = 1.0 → 2.0, 1.0, 1.5, 1.0 scaled by 65535 / 16383.
        assert!((scale[0] - 2.0 * 65535.0 / 16383.0).abs() < 1e-3);
        assert!((scale[1] - 65535.0 / 16383.0).abs() < 1e-3);
        assert!((scale[3] - 65535.0 / 16383.0).abs() < 1e-3);
        // adjust_maximum: a measured maximum below 75% of the white level is
        // ignored, and one above it replaces the white level.
        assert_eq!(params.adjusted_maximum(16383 / 2), 16383);
        assert_eq!(params.adjusted_maximum(16383 * 3 / 4 + 1), 16383 * 3 / 4 + 1);
        assert_eq!(params.adjusted_maximum(0), 16383);
    }

    /// The camera→ACES matrix is `aces_rgb · rgb_cam` in `f32`, exactly as
    /// `convert_to_rgb()` accumulates it.
    #[test]
    fn camera_to_aces_matches_convert_to_rgb() {
        let mut input = sensor(0x9494_9494, 3);
        input.rgb_camera = [
            [0.62, 0.31, 0.06, 0.0],
            [0.10, 0.83, 0.07, 0.0],
            [0.02, 0.12, 0.86, 0.0],
        ];
        let out = camera_to_aces(&input);
        for (out_row, aces_row) in out.iter().zip(&ACES_RGB) {
            for (j, &actual) in out_row[..3].iter().enumerate() {
                let mut expected = 0.0f32;
                for (&coefficient, camera_row) in aces_row.iter().zip(&input.rgb_camera) {
                    expected += (coefficient * f64::from(camera_row[j])) as f32;
                }
                assert_eq!(actual, expected);
            }
        }
        // Columns beyond `colors` keep LibRaw's initial `rgb_cam` copy.
        for (out_row, camera_row) in out.iter().zip(&input.rgb_camera) {
            assert_eq!(out_row[3], camera_row[3]);
        }
    }

    #[test]
    fn tungsten_and_custom_match_reference_adaptation() {
        for (mode, temperature, expected) in [
            (
                WhiteBalance::Tungsten,
                None,
                [0.2995248437, 0.4106940329, 1.1936421394],
            ),
            (
                WhiteBalance::Custom,
                Some(5000.0),
                [0.2118798941, 0.3155113757, 0.5267853737],
            ),
        ] {
            let options = RawOptions {
                white_balance: mode,
                temperature,
                ..Default::default()
            };
            let output = mv(
                XYZ_TO_ACES,
                mv(
                    options.adaptation().unwrap().unwrap(),
                    mv(ACES_TO_XYZ, [0.2f32 as f64, 0.3f32 as f64, 0.4f32 as f64]),
                ),
            );
            for c in 0..3 {
                assert!((output[c] as f32 as f64 - expected[c]).abs() < 6e-8);
            }
        }
    }

    #[test]
    fn custom_requires_valid_temperature_and_finite_tint() {
        let mut options = RawOptions {
            white_balance: WhiteBalance::Custom,
            ..Default::default()
        };
        assert!(
            options
                .validate()
                .unwrap_err()
                .to_string()
                .contains("requires a temperature")
        );
        for temperature in [f64::NAN, 0.0, 25001.0] {
            options.temperature = Some(temperature);
            assert!(options.validate().is_err());
        }
        options.temperature = Some(6504.0);
        options.tint = Some(f64::INFINITY);
        assert!(options.validate().is_err());
        options.tint = Some(1.0);
        assert!(options.validate().is_ok());
    }
    #[test]
    fn native_geometry_dimensions_and_stretch_preserve_reference_samples() {
        for aspect in [0.5, 1.7, 256.0 / 235.0] {
            let width = 32;
            let height = 24;
            let samples: Vec<f32> = (0..width * height * 4).map(|i| (i % 60000) as f32).collect();
            let (output, ow, oh) = geometry_reference(&samples, width, height, 0, aspect, 3).unwrap();
            let mut metadata = sensor(0, 3);
            metadata.width = width;
            metadata.height = height;
            metadata.pixel_aspect = aspect;
            assert_eq!((ow, oh), metadata.stretched_dimensions().unwrap());
            let mut coordinate = 0.0f64;
            let length = if aspect < 1.0 { oh } else { ow };
            for target in 0..length {
                let source = coordinate as u32;
                let (ox, oy, sx, sy) = if aspect < 1.0 { (7, target, 7, source) } else { (target, 7, source, 7) };
                for c in 0..3 {
                    assert_eq!(output[((oy * ow + ox) * 4 + c) as usize], samples[((sy * width + sx) * 4 + c) as usize]);
                }
                coordinate += if aspect < 1.0 { aspect } else { 1.0 / aspect };
            }
        }
    }

    #[test]
    fn device_stretch_matches_native_integer_fraction_boundary() {
        let gpu = spektrafilm_gpu::test_support::adapter();
        for aspect in [0.5, 1.7, 256.0 / 235.0] {
            let mut input = sensor(0, 3);
            input.width = 32;
            input.height = 24;
            input.copied_width = 32;
            input.copied_height = 24;
            input.pixel_aspect = aspect;
            input.layout = RawSensorLayout::Interleaved4;
            input.raw_color = true;
            input.fixed_maximum = true;
            input.samples = (0..32 * 24).flat_map(|i| [(i * 23 + 1000) as f32, (i * 11 + 2000) as f32, (i * 7 + 3000) as f32, 0.0]).collect();
            let params = plan(&input, &RawOptions { white_balance: WhiteBalance::Daylight, ..Default::default() }).unwrap();
            let (expected, width, height) = geometry_reference(&input.samples, 32, 24, 0, aspect, 3).unwrap();
            let actual = gpu.raw_process_sensor(&input, &params).unwrap().unwrap();
            assert_eq!((actual.width, actual.height), (width, height));
            for i in 0..(width * height) as usize { for c in 0..3 {
                assert_eq!(spektrafilm_math::precision::to_f64(actual.data[i * 3 + c]), f64::from(expected[i * 4 + c] / 65535.0));
            }}
        }
    }
    #[test]
    fn float_dng_conversion_matches_linked_native_boundary() {
        let samples = [0.0, 0.25, 1.0, 2.0];
        let metadata = [0x94949494u32, 1, 1, 1, 2, 3, 4, 0, 0];
        let mut output = std::ptr::null_mut();
        let mut result = [0u32; 9];
        let mut map = [0u32];
        let code = unsafe { sf_raw_float_reference(samples.as_ptr(), 4, 1, 2.0, metadata.as_ptr(), map.as_ptr(), &mut output, result.as_mut_ptr(), map.as_mut_ptr()) };
        assert_eq!(code, 0);
        let owned = NativePixels(output);
        let values = unsafe { std::slice::from_raw_parts(owned.0, samples.len()) };
        assert_eq!(values, &[0.0, 2047.0, 8191.0, 16383.0]);
        assert_eq!(f32::from_bits(result[0]), 8191.5);
        assert_eq!(result[2], 16383);
        assert_eq!(result[1], 16382);
    }
    #[test]
    fn native_xtrans_order_is_scoped_and_restores_callers_thread_setting() {
        assert_eq!(unsafe { sf_raw_ordered_xtrans_threading_reference() }, 0);
    }

    #[test]
    fn native_xtrans_multitile_output_is_independent_of_callers_thread_count() {
        let pattern: [u8; 36] = [
            1,0,1,1,2,1, 2,1,2,0,1,0, 1,0,1,1,2,1,
            1,2,1,1,0,1, 0,1,0,2,1,2, 1,2,1,1,0,1,
        ];
        // Two dependent native top rows, with chromatic structure across the
        // overlap; this exercises the actual linked interpolation, not a race
        // assertion whose outcome depends on the machine's scheduler.
        let (width, height) = (512usize, 1012usize);
        let mut input = vec![0u32; width * height * 4];
        for y in 0..height { for x in 0..width {
            let color = pattern[(y % 6) * 6 + x % 6] as usize;
            input[(y * width + x) * 4 + color] = ((x * 997 + y * 1731 + (x * y % 31) * 103 + color * 193) & 65535) as u32;
        }}
        let decode = |threads| {
            let mut output = std::ptr::null_mut();
            assert_eq!(unsafe { sf_raw_xtrans_threaded_reference(input.as_ptr(), width as u32, height as u32, pattern.as_ptr(), threads, &mut output) }, 0);
            NativePixels(output)
        };
        let serial = decode(1);
        let parallel_caller = decode(3);
        let serial = unsafe { std::slice::from_raw_parts(serial.0, input.len()) };
        let parallel_caller = unsafe { std::slice::from_raw_parts(parallel_caller.0, input.len()) };
        assert_eq!(serial, parallel_caller);
    }

    #[test]
    fn linked_native_xtrans_constant_fixture_boundary() {
        let pattern: [u8; 36] = [
            1,0,1,1,2,1,
            2,1,2,0,1,0,
            1,0,1,1,2,1,
            1,2,1,1,0,1,
            0,1,0,2,1,2,
            1,2,1,1,0,1,
        ];
        let mut input = vec![0u32; 512 * 512 * 4];
        for y in 0..512 { for x in 0..512 {
            let color = pattern[(y % 6) * 6 + x % 6] as usize;
            input[(y * 512 + x) * 4 + color] = [101,207,311][color];
        }}
        let mut output = std::ptr::null_mut();
        assert_eq!(unsafe { sf_raw_xtrans_reference(input.as_ptr(), 512, 512, pattern.as_ptr(), &mut output) }, 0);
        let owned = NativePixels(output);
        let pixels = unsafe { std::slice::from_raw_parts(owned.0, input.len()) };
        assert_eq!(&pixels[2566 * 4..2566 * 4 + 3], &[101.0,207.0,311.0]);
    }
}
