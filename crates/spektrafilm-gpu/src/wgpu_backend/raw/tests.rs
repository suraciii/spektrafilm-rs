//! Device RAW sensor pipeline acceptance.
//!
//! The pipeline reproduces `dcraw_process`'s selected algorithm on device:
//! black subtraction with the measured maximum, `scale_colors`' integer
//! scaling, `pre_interpolate`'s green merge, `border_interpolate(5)`, AHD, and
//! `convert_to_rgb`. These tests exercise the *orchestration* — pass order,
//! buffer extents, uniform contents, readback geometry — and check that the
//! result is a well-formed oriented RGB image.
//!
//! They do **not** assert numerical parity with LibRaw's CPU reference: that
//! comparison needs real sensor metadata (camera matrices, multipliers, black
//! levels) and a frozen fixture image, which is a separate acceptance step.

use super::*;
use spektrafilm_math::precision::to_f64;

/// An RGGB Bayer pattern packed the way LibRaw's `filters` packs it:
/// `FC(row, col) = (filters >> (((row << 1) & 14) | (col & 1)) << 1) & 3`.
const RGGB: u32 = 0x1616_1616;

/// The synthetic sensor the orchestration test runs over.
///
/// A small square Bayer plane with no black offsets and no repeating black
/// map, identity camera matrices, and `maximum = 65535`, so every stage has
/// real work to do without depending on file-parsed metadata.
pub(super) fn synthetic_sensor(size: u32) -> RawSensor {
    let pixels = (size * size) as usize;
    // A deterministic spread across the 16-bit domain, so the black
    // subtraction, the integer scale, and the AHD interpolation all see
    // non-degenerate values.
    let samples: Vec<f32> = (0..pixels)
        .map(|i| {
            let x = (i % size as usize) as f32;
            let y = (i / size as usize) as f32;
            ((x * 37.0 + y * 11.0 + ((i * 7) % 23) as f32) * 977.0) % 65536.0
        })
        .collect();
    RawSensor {
        samples,
        layout: spektrafilm_backend::RawSensorLayout::Cfa,
        width: size,
        height: size,
        copied_width: size,
        copied_height: size,
        allocation_row_stride_bytes: size * 2,
        filters: RGGB,
        cfa: vec![0, 1, 1, 2],
        cfa_rows: 2,
        cfa_columns: 2,
        fuji_width: 0,
        fuji_layout: false,
        flip: 0,
        pixel_aspect: 1.0,
        is_foveon: false,
        colors: 3,
        black: 0,
        maximum: 65535,
        channel_black: [0; 4],
        black_map_rows: 0,
        black_map_columns: 0,
        black_map: Vec::new(),
        camera_multipliers: [2.0, 1.0, 1.5, 1.0],
        default_multipliers: [1.0, 1.0, 1.0, 1.0],
        rgb_camera: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        xyz_camera: [
            [0.4124, 0.3576, 0.1805, 0.0],
            [0.2126, 0.7152, 0.0722, 0.0],
            [0.0193, 0.1192, 0.9505, 0.0],
        ],
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

/// The plan `spektrafilm-raw` resolves for an as-shot Bayer sensor: AHD, an
/// identity CAT02 adaptation, and the per-channel integer scale.
pub(super) fn synthetic_params(sensor: &RawSensor) -> RawGpuParams {
    RawGpuParams {
        demosaic: spektrafilm_backend::RawDemosaic::BayerAhd,
        camera_to_aces: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        adaptation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        adaptation_active: false,
        tint: 1.0,
        adjust_maximum_threshold: 0.75,
        white_balance: [1.0, 1.0, 1.0, 1.0],
        auto_white_balance: false,
        maximum: sensor.maximum - sensor.black,
        green_merge: sensor.filters > 1000 && sensor.colors == 3,
        green_merge_row: sensor.fc(1, 0) >> 1,
        green_merge_column: sensor.fc(sensor.fc(1, 0) >> 1, 1) & 1,
        lens: None,
    }
}

/// The whole pipeline runs and returns a well-formed oriented RGB image.
///
/// This is an orchestration smoke test, not a numerical parity test: it checks
/// the pass order is executable, the working buffers are sized correctly, the
/// measured maximum feeds the integer scale, and the readback lands at the
/// oriented dimensions with finite in-range samples. It does not claim
/// bit-for-bit agreement with LibRaw's `dcraw_process`.
#[test]
fn raw_sensor_pipeline_produces_a_well_formed_rgb_image() {
    // The software Vulkan driver is not safe under concurrent device use in
    // one process, so the adapter (which holds the serialization guard) is
    // held for the whole test body.
    let gpu = crate::test_support::adapter();
    let sensor = synthetic_sensor(8);
    let params = synthetic_params(&sensor);
    let image = gpu
        .raw_process_sensor_gpu(&sensor, &params)
        .expect("the device RAW pipeline runs the AHD stages");

    // `flip = 0`, so the oriented dimensions are the sensor's own.
    assert_eq!(image.width, sensor.width);
    assert_eq!(image.height, sensor.height);
    assert_eq!(image.data.len(), 8 * 8 * 3);

    // The reference's output domain: `convert_to_rgb` clips to 16 bits and
    // divides by 65535 before any later stage, so every sample is finite and
    // inside 0..=1. A well-formed Bayer plane also has real signal, not an
    // all-zero readback.
    let mut nonzero = 0usize;
    for value in &image.data {
        let value = to_f64(*value);
        assert!(value.is_finite(), "a RAW device sample is finite");
        assert!(
            (0.0..=1.0).contains(&value),
            "a RAW device sample is inside the reference's 16-bit domain: {value}"
        );
        if value > 0.0 {
            nonzero += 1;
        }
    }
    assert!(nonzero > 0, "the device RAW pipeline produced no signal");
}

/// The orientation mapping swaps the output axes and reads the unswapped
/// plane, exactly as LibRaw's `flip_index` does for `flip` bit 4.
#[test]
fn raw_sensor_pipeline_orients_the_output_for_a_transposing_flip() {
    let gpu = crate::test_support::adapter();
    let mut sensor = synthetic_sensor(8);
    sensor.flip = 4;
    let params = synthetic_params(&sensor);
    let image = gpu
        .raw_process_sensor_gpu(&sensor, &params)
        .expect("the device RAW pipeline runs the AHD stages");

    // Bit 4 swaps the axes, so the oriented raster is the transposed one.
    assert_eq!(image.width, sensor.height);
    assert_eq!(image.height, sensor.width);
    assert_eq!(image.data.len(), 8 * 8 * 3);
    for value in &image.data {
        let value = to_f64(*value);
        assert!(value.is_finite());
        assert!((0.0..=1.0).contains(&value));
    }
}

/// A request the pipeline cannot serve exactly is a named failure, not a
/// dispatch over a wrong read set: the caller keeps the host reference.
#[test]
fn raw_sensor_pipeline_rejects_an_unusable_sensor() {
    let gpu = crate::test_support::adapter();
    let mut sensor = synthetic_sensor(8);
    let params = synthetic_params(&sensor);
    sensor.cfa_rows = 0;
    sensor.cfa_columns = 0;
    assert!(
        gpu.raw_process_sensor_gpu(&sensor, &params).is_err(),
        "a sensor with no CFA pattern must not dispatch"
    );

    let mut sensor = synthetic_sensor(8);
    sensor.cfa_rows = 4;
    sensor.cfa_columns = 4;
    assert!(
        gpu.raw_process_sensor_gpu(&sensor, &params).is_err(),
        "a CFA tile that disagrees with its declared extents must not dispatch"
    );
}

#[test]
fn fuji_source_rectangle_is_placed_on_native_diamond_coordinates() {
    let gpu = crate::test_support::adapter();
    for layout in [false, true] {
        let mut sensor = synthetic_sensor(16);
        sensor.width = 15;
        sensor.height = 14;
        sensor.fuji_width = 8;
        sensor.fuji_layout = layout;
        sensor.copied_width = if layout { 8 } else { 16 };
        sensor.copied_height = if layout { 12 } else { 6 };
        sensor.samples = vec![12000.0; (sensor.copied_width * sensor.copied_height) as usize];
        let mut params = synthetic_params(&sensor);
        params.demosaic = RawDemosaic::FujiPpg;
        let output = gpu.raw_process_sensor_gpu(&sensor, &params).unwrap();
        assert_eq!(
            (output.width, output.height),
            sensor.output_dimensions().unwrap()
        );
        assert!(output.data.iter().any(|&v| to_f64(v) > 0.0));
        assert!(output.data.iter().all(|&v| to_f64(v).is_finite()));
    }
}

#[test]
fn pixel_aspect_stretch_precedes_orientation() {
    let gpu = crate::test_support::adapter();
    let mut sensor = synthetic_sensor(16);
    sensor.pixel_aspect = 0.5;
    sensor.flip = 4;
    let output = gpu
        .raw_process_sensor_gpu(&sensor, &synthetic_params(&sensor))
        .unwrap();
    assert_eq!((output.width, output.height), (32, 16));
}

#[test]
fn bounded_raw_pipeline_preserves_full_source_scale_and_cfa_coordinates() {
    let gpu = crate::test_support::adapter();
    let mut sensor = synthetic_sensor(64);
    sensor.channel_black = [11, 17, 23, 29];
    sensor.black_map_rows = 3;
    sensor.black_map_columns = 2;
    sensor.black_map = vec![0, 2, 3, 7, 11, 13];
    let mut params = synthetic_params(&sensor);
    params.auto_white_balance = true;
    for mode in [
        RawDemosaic::BayerAhd,
        RawDemosaic::Vng,
        RawDemosaic::FujiPpg,
    ] {
        params.demosaic = mode;
        let (full, width, height) = gpu.raw_process_sensor_buffer(&sensor, &params).unwrap();
        let expected = gpu
            .pipe
            .readback_buffer_as::<f32>(&full, u64::from(width) * u64::from(height) * 12)
            .unwrap();
        let actual = gpu.raw_process_paged(&sensor, &params).unwrap();
        let actual = gpu.materialize_buffer(&actual).unwrap();
        assert_eq!((actual.width, actual.height), (width, height));
        for (i, (a, b)) in actual.data.iter().zip(&expected).enumerate() {
            assert_eq!(to_f64(*a), f64::from(*b), "paged {mode:?} sample {i}");
        }
    }
}

#[test]
fn raw_cancelled_before_start_rejects_contiguous_and_paged_without_poisoning_reuse() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let gpu = crate::test_support::adapter();
    let sensor = synthetic_sensor(16);
    let params = synthetic_params(&sensor);
    let cancellation = Arc::new(AtomicBool::new(true));
    let mut operation = gpu.derived_explicit();
    operation.pipe = operation.pipe.with_cancellation(Some(cancellation.clone()));
    operation.cancellation = Some(cancellation.clone());
    assert_eq!(
        operation
            .raw_process_sensor_retained_gpu(&sensor, &params)
            .err()
            .unwrap(),
        "operation cancelled"
    );
    assert_eq!(
        operation.raw_process_paged(&sensor, &params).err().unwrap(),
        "operation cancelled"
    );
    cancellation.store(false, Ordering::Relaxed);
    let actual = operation.raw_process_sensor_gpu(&sensor, &params).unwrap();
    let expected = gpu.raw_process_sensor_gpu(&sensor, &params).unwrap();
    assert_eq!(actual.data, expected.data);
}

#[test]
fn lab_table_preserves_native_float_rounding_at_ahd_threshold() {
    let gpu = crate::test_support::adapter();
    let table = RawBuffer::zeros(&gpu.device, "raw_native_lab_table", 65536 * 4);
    let pass = gpu.raw_dispatch(
        CBRT_SHADER,
        &[&table],
        &[wgpu::BufferBindingType::Storage { read_only: false }],
        65536,
        "raw_native_lab_table",
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    pass.encode(&mut encoder, "raw_native_lab_table");
    gpu.queue.submit(Some(encoder.finish()));
    let values = gpu
        .pipe
        .readback_buffer_as::<u32>(&table.buffer, 65536 * 4)
        .unwrap();
    // Measured linked LibRaw0.22.2 pow(float,float), at the five-pixel AHD
    // homogeneity population from natural.KDC. One ULP flips the selected RGB.
    assert_eq!(
        [values[19944], values[20008], values[19828]],
        [1059860948, 1059873006, 1059839026]
    );
}
