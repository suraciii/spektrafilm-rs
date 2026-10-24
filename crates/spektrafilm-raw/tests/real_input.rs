//! Opt-in real-input boundary. Fixture bytes remain external; run with an
//! explicitly frozen SPEKTRAFILM_RAW_PARITY_FIXTURE and retain its hash.
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::precision::to_f64;
use spektrafilm_raw::{RawOptions, WhiteBalance};

#[test]
#[ignore = "requires an external frozen camera RAW fixture and GPU adapter"]
fn native_and_retained_device_outputs_report_comparable_samples() {
    let path = std::env::var_os("SPEKTRAFILM_RAW_PARITY_FIXTURE").expect("set camera RAW fixture");
    let path = std::path::Path::new(&path);
    let gpu = spektrafilm_gpu::test_support::adapter();
    let sensor = spektrafilm_raw::unpack(path).unwrap();
    let demosaic = spektrafilm_raw::plan(&sensor, &RawOptions::default()).unwrap().demosaic;
    eprintln!(
        "RAW sensor {:?} decoder={} colors={} filters={:#010x} channels={} layout={:?} is_foveon={} float_maximum={} maximum={} black={} channel_black={:?} cfa={}x{} fuji={} aspect={} native_default_demosaic={}",
        path, spektrafilm_raw::decoder_version(), sensor.colors, sensor.filters,
        sensor.layout.channels(), sensor.layout, sensor.is_foveon,
        sensor.float_maximum, sensor.maximum, sensor.black, sensor.channel_black,
        sensor.cfa_rows, sensor.cfa_columns, sensor.fuji_width, sensor.pixel_aspect,
        demosaic.name(),
    );
    let selected_wb = std::env::var("SPEKTRAFILM_RAW_PARITY_WB").ok().map(|value| match value.as_str() {
        "as_shot" => WhiteBalance::AsShot,
        "daylight" => WhiteBalance::Daylight,
        "tungsten" => WhiteBalance::Tungsten,
        "custom" => WhiteBalance::Custom,
        _ => panic!("SPEKTRAFILM_RAW_PARITY_WB must be as_shot, daylight, tungsten, or custom"),
    });
    let selected_lens = std::env::var("SPEKTRAFILM_RAW_PARITY_LENS").ok().map(|value| value.parse::<bool>().expect("SPEKTRAFILM_RAW_PARITY_LENS must be true or false"));
    for white_balance in [WhiteBalance::AsShot, WhiteBalance::Daylight, WhiteBalance::Tungsten, WhiteBalance::Custom] {
        if selected_wb.is_some_and(|selected| selected != white_balance) { continue; }
        for lens_correction in [false, true] {
            if selected_lens.is_some_and(|selected| selected != lens_correction) { continue; }
            let options = RawOptions { white_balance, temperature: Some(5000.0), tint: Some(1.05), lens_correction };
            let native = spektrafilm_raw::load(path, &options).unwrap();
            let retained = spektrafilm_raw::load_retained_with_backend(path, &options, &*gpu).unwrap();
            assert!(retained.image.is_retained());
            let actual = gpu.materialize_retained(&retained.image).unwrap();
            assert_eq!((native.image.width, native.image.height), (actual.width, actual.height));
            assert_eq!(native.lens_info, retained.lens_info);
            assert_eq!(native.lens_model_identity, retained.lens_model_identity);
            assert_eq!(retained.lens_model_identity.is_some(), lens_correction);
            let mut total = 0.0f64;
            let mut maximum = 0.0f64;
            let mut outliers: Vec<(usize, f64)> = Vec::with_capacity(8);
            for (index, (a, b)) in native.image.data.iter().zip(&actual.data).enumerate() {
                let difference = (to_f64(*a) - to_f64(*b)).abs();
                assert!(difference.is_finite());
                total += difference;
                maximum=maximum.max(difference);
                if difference > 0.0 && (outliers.len() < 8 || difference > outliers.last().unwrap().1) {
                    outliers.push((index, difference));
                    outliers.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
                    outliers.truncate(8);
                }
            }
            eprintln!("RAW {:?} cfa={}x{} fuji={} aspect={} wb={:?} lens={} dimensions={}x{} mean_abs={:.12e} max_abs={:.12e}", path, sensor.cfa_rows, sensor.cfa_columns, sensor.fuji_width, sensor.pixel_aspect, white_balance, lens_correction, actual.width, actual.height, total / actual.data.len() as f64, maximum);
            eprintln!("RAW lens summary: {:?}", native.lens_info);
            for (index, difference) in outliers {
                let pixel = index / 3;
                let rgb = pixel * 3;
                eprintln!("RAW outlier x={} y={} channel={} difference={:.12e} native_rgb={:?} device_rgb={:?}", pixel % actual.width as usize, pixel / actual.width as usize, index % 3, difference, &native.image.data[rgb..rgb + 3], &actual.data[rgb..rgb + 3]);
            }
            // Existing normalized-ACES RAW acceptance bounds, over every RGB
            // sample; geometric extents and lens identity are checked above.
            assert!(total / actual.data.len() as f64 <= 0.005, "RAW mean absolute error exceeds 0.005");
            assert!(maximum <= 0.05, "RAW maximum absolute error exceeds 0.05");
        }
    }
}
