use super::*;
use spektrafilm_math::precision::from_f64;

#[test]
fn metering_matches_reference_patterns_and_black_boundary() {
    let gpu = crate::test_support::adapter();
    let image = ImageBuf::from_data(300, 200, (0..200).flat_map(|y| (0..300).flat_map(move |x| (0..3).map(move |c| from_f64(((x * 7 + y * 13 + c * 29) % 100) as f64 / 100.0)))).collect());
    let space = spektrafilm_math::colorspace::resolve("sRGB").unwrap();
    let mut params = MeterAutoexposureParams {
        width: image.width, height: image.height, max_size: 256,
        y_row: space.matrix_rgb_to_xyz[1], cctf_kind: None,
        method: String::new(), mid_grey: 0.184,
    };
    // Frozen upstream float64 fixture also used by core's meter regression.
    // A 1e-5 EV bound is that fixture's existing reference budget; native-f32
    // source rounding is below this bound, not a different sampling law.
    for (method, expected) in [
        ("average", -1.427705567203), ("median", -1.308011314552),
        ("center_weighted", -1.427664142652), ("partial", -1.425832251352),
        ("matrix", -1.427803049578), ("multi_zone", -1.439398829551),
        ("highlight_weighted", -1.783882472180),
    ] {
        params.method = method.into();
        let actual = build_meter_state(&gpu.device, &params, &image, &gpu).unwrap().meter(&gpu.pipe).unwrap();
        assert!((actual - expected).abs() < 1e-5, "{method}: device {actual}, reference {expected}");
    }
    let black = ImageBuf::new(7, 5);
    params.width = black.width;
    params.height = black.height;
    for method in ["average", "median", "center_weighted", "partial", "matrix", "multi_zone", "highlight_weighted"] {
        params.method = method.into();
        assert_eq!(build_meter_state(&gpu.device, &params, &black, &gpu).unwrap().meter(&gpu.pipe).unwrap(), 0.0, "black {method}");
    }
    let gray = ImageBuf::from_data(7, 5, vec![from_f64(0.5); 7 * 5 * 3]);
    params.cctf_kind = Some(spektrafilm_backend::cctf_kind(space.cctf));
    params.method = "average".into();
    let luminance = spektrafilm_math::colorspace::cctf_decode(0.5, space.cctf) * params.y_row.iter().sum::<f64>();
    let expected = -(luminance / params.mid_grey).log2();
    let actual = build_meter_state(&gpu.device, &params, &gray, &gpu).unwrap().meter(&gpu.pipe).unwrap();
    assert!((actual - expected).abs() < 1e-12, "decoded gray EV {actual} != {expected}");
}

#[test]
fn paged_metering_preserves_full_source_and_reflected_support() {
    let gpu = crate::test_support::adapter();
    let space = spektrafilm_math::colorspace::resolve("sRGB").unwrap();
    // The bright right edge must contribute even though most of the source is
    // dark. A one-row image also exercises reflected vertical support.
    let image = ImageBuf::from_data(2049, 1, (0..2049).flat_map(|x| {
        let value = if x > 1800 { 0.9 } else { 0.025 + f64::from(x % 11) * 0.001 };
        [from_f64(value); 3]
    }).collect());
    let samples = scalars_to_f32(&image.data);
    let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("meter_seam_source"), contents: bytemuck::cast_slice(&samples),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let source = BufferPayload::from_buffer(gpu.backend_id(), buffer, image.width, image.height);
    let mut params = MeterAutoexposureParams {
        width: image.width, height: image.height, max_size: 256,
        y_row: space.matrix_rgb_to_xyz[1], cctf_kind: None,
        method: String::new(), mid_grey: 0.184,
    };
    for method in ["average", "median", "center_weighted", "partial", "matrix", "multi_zone", "highlight_weighted"] {
        params.method = method.into();
        let full = build_meter_state_from_payload(&gpu, &params, &source).unwrap().meter(&gpu.pipe).unwrap();
        // Forces both source pages and vertical row pages, with horizontal
        // support crossing their seams. No process-global environment mutation.
        let paged = build_meter_state_with_limit(&gpu, &params, &source, 4096).unwrap().meter(&gpu.pipe).unwrap();
        assert_eq!(paged.to_bits(), full.to_bits(), "{method}: page boundaries changed EV");
        if method == "average" {
            let dark_ev = -(0.03f64 / params.mid_grey).log2();
            assert!(paged < dark_ev - 1.0, "full-source edge was omitted: EV {paged}");
        }
    }
}

#[test]
fn paged_metering_keeps_frozen_full_source_preview_budget() {
    let gpu = crate::test_support::adapter();
    let image = ImageBuf::from_data(300, 200, (0..200).flat_map(|y| (0..300).flat_map(move |x| (0..3).map(move |c| from_f64(((x * 7 + y * 13 + c * 29) % 100) as f64 / 100.0)))).collect());
    let space = spektrafilm_math::colorspace::resolve("sRGB").unwrap();
    let samples = scalars_to_f32(&image.data);
    let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("meter_full_source_fixture"), contents: bytemuck::cast_slice(&samples),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let source = BufferPayload::from_buffer(gpu.backend_id(), buffer, image.width, image.height);
    let params = MeterAutoexposureParams {
        width: image.width, height: image.height, max_size: 256,
        y_row: space.matrix_rgb_to_xyz[1], cctf_kind: None,
        method: "average".into(), mid_grey: 0.184,
    };
    // Source (720000 bytes) and old vertical scratch exceed this limit;
    // the unchanged full preview luminance grid still fits it.
    let actual = build_meter_state_with_limit(&gpu, &params, &source, 400_000).unwrap().meter(&gpu.pipe).unwrap();
    assert!((actual - -1.427705567203).abs() < 1e-5, "paged full-source EV {actual}");
}

#[test]
fn metering_cancellation_is_operation_scoped_and_survives_derivation() {
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    let base = crate::test_support::adapter();
    let token = Arc::new(AtomicBool::new(false));
    let backend = base.with_cancellation(Some(token.clone())).unwrap();
    let image = ImageBuf::from_data(5, 3, vec![from_f64(0.18); 45]);
    let space = spektrafilm_math::colorspace::resolve("sRGB").unwrap();
    let params = MeterAutoexposureParams {
        width: 5, height: 3, max_size: 256, y_row: space.matrix_rgb_to_xyz[1],
        cctf_kind: None, method: "average".into(), mid_grey: 0.184,
    };
    assert!(backend.meter_autoexposure(&image, &params).unwrap().is_some());
    token.store(true, Ordering::Relaxed);
    let observed = backend.with_observation_context(ObservationContext::default()).unwrap();
    assert_eq!(observed.meter_autoexposure(&image, &params).unwrap_err(), "operation cancelled");
    assert!(base.meter_autoexposure(&image, &params).unwrap().is_some());
    let cleared = observed.with_cancellation(None).unwrap();
    assert!(cleared.meter_autoexposure(&image, &params).unwrap().is_some());
}
