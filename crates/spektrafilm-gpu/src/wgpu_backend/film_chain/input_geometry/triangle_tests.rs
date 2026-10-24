//! Regression cover for the streamed `PreviewTriangle` geometry stage.
//!
//! These tests pin the device work budget of the preview route (no per-tap
//! copies or submissions), the page-seam equality of the streamed accumulation
//! against the unpaged output, the clamp that only the completed horizontal
//! pass applies, and cancellation that lands before the first dispatch without
//! touching retained sources.

use super::tests::{
    assert_within, patterned, reference_preview, run, run_bounded, space, triangle_preview, upload,
};
use super::{READ, TRIANGLE_SHADER, TriangleParams, UNIFORM, WRITE};
use spektrafilm_backend::telemetry::{CollectionMode, ObservationContext, WorkCounters};
use spektrafilm_backend::{ComputeBackend, InputGeometryGpuParams};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Pure-resize geometry request: no rotation, no crop, one preview target.
fn resize_only(width: u32, height: u32) -> InputGeometryGpuParams {
    InputGeometryGpuParams {
        rotation_quarter_turns: 0,
        crop: None,
        resize: Some(triangle_preview(width, height)),
        pixel_pitch_um: 12.0,
    }
}

/// Device work of the geometry stage alone. The source upload happens before
/// observation starts and the totals are read before materialization, so the
/// counters cover exactly the encode: no host round-trip and no readback.
fn measured(source: &ImageBuf, params: &InputGeometryGpuParams) -> (ImageBuf, WorkCounters) {
    let gpu = crate::test_support::explicit_adapter();
    let mut observed = gpu.derived_explicit();
    let carrier = upload(&observed, source);
    let context = ObservationContext::new(CollectionMode::Summary);
    observed.context_mut(context.clone());
    let out = observed
        .encode_input_geometry(&carrier, params)
        .unwrap()
        .expect("the wgpu backend has device geometry");
    let work = context.snapshot().totals;
    (observed.materialize_buffer(&out).unwrap(), work)
}

#[test]
fn triangle_preview_streams_the_geometry_stage_without_per_tap_copies() {
    // The preview cap's long-edge rule turns 256x128 into 64x32, so the
    // source is larger than the cap and every output pixel gathers taps.
    let source = patterned(256, 128);
    let params = resize_only(64, 32);
    let expected = reference_preview(&source, 64, 32);
    let (out, work) = measured(&source, &params);
    assert_within(&out, &expected, 1e-6, "256x128 preview to 64x32");
    // The retired per-tap path issued 91008 buffer copies for this request.
    // The streamed stage binds the source pages read-only and never copies,
    // stages, or reads back.
    assert_eq!(
        work.device_copy_count,
        Some(0),
        "geometry must bind source pages read-only instead of copying taps"
    );
    assert_eq!(
        work.staging_copy_count,
        Some(0),
        "geometry must not stage through MAP_READ buffers"
    );
    assert_eq!(work.readback_count, Some(0), "geometry must stay on device");
    assert_eq!(
        work.image_upload_count,
        Some(0),
        "geometry must not round-trip through the host"
    );
    let dispatches = work.dispatch_count.unwrap_or(u64::MAX);
    let submits = work.submit_count.unwrap_or(u64::MAX);
    assert!(
        dispatches <= 4,
        "single pages need one dispatch per separable pass, got {dispatches}"
    );
    assert!(
        submits <= 2,
        "one geometry submit batches both passes, got {submits}"
    );
}

#[test]
fn triangle_paged_stream_matches_the_unpaged_preview_across_row_seams() {
    let gpu = crate::test_support::explicit_adapter();
    // Each binding cap keeps at most eight pixels per page, so pages of the
    // odd fixtures split mid-row on the source and on the output alike.
    for (source_width, source_height, width, height, caps) in [
        (37u32, 23u32, 29u32, 11u32, [41u64, 101]),
        (11, 7, 23, 19, [41, 101]),
        (1, 17, 1, 5, [12, 101]),
        (17, 1, 5, 1, [12, 101]),
    ] {
        let what = format!("{source_width}x{source_height} to {width}x{height}");
        let source = patterned(source_width, source_height);
        let params = resize_only(width, height);
        let expected = reference_preview(&source, width, height);
        let unpaged = run(&gpu, &source, &params);
        assert_within(&unpaged, &expected, 1e-6, &format!("unpaged {what}"));
        for limit in caps {
            // The carried accumulation must reproduce the unpaged tap order
            // exactly: a mis-set reset or page seam shows up as a nonzero
            // difference here.
            let paged = run_bounded(&gpu, &source, &params, limit);
            assert_within(
                &paged,
                &unpaged,
                0.0,
                &format!("paged {what}, binding {limit}"),
            );
        }
    }
}

#[test]
fn triangle_saturates_only_on_the_completed_horizontal_pass() {
    let gpu = crate::test_support::explicit_adapter();
    // Saturated signed blocks give the final clamp visible work: the raw
    // triangle average is exactly -2 and 3 at the corners, while blend rows
    // and columns keep unclamped values outside [0, 1] in the vertical pass.
    let mut source = patterned(23, 15);
    for y in 0..4 {
        for x in 0..4 {
            source.set(x, y, [from_f64(-2.0); 3]);
            source.set(22 - x, 14 - y, [from_f64(3.0); 3]);
        }
    }
    let params = resize_only(11, 7);
    let expected = reference_preview(&source, 11, 7);
    let out = run(&gpu, &source, &params);
    assert_within(&out, &expected, 1e-6, "clamped preview 23x15 to 11x7");
    let mut saturated = [false, false];
    for &sample in &out.data {
        let value = to_f64(sample);
        assert!(
            (0.0..=1.0).contains(&value),
            "sample {value} escaped the completed horizontal clamp"
        );
        saturated[0] |= value == 0.0;
        saturated[1] |= value == 1.0;
    }
    assert!(
        saturated[0] && saturated[1],
        "the saturated corners must clamp to exactly 0.0 and 1.0"
    );
    for limit in [41u64, 101] {
        // Clamping mid-accumulation would corrupt the paged stream, so the
        // paged output must still equal the unpaged one bit for bit.
        let paged = run_bounded(&gpu, &source, &params, limit);
        assert_within(
            &paged,
            &out,
            0.0,
            &format!("paged clamp result, binding {limit}"),
        );
    }
}

#[test]
fn triangle_cancelled_before_dispatch_keeps_sources_and_healthy_reuse() {
    let gpu = crate::test_support::explicit_adapter();
    let mut observed = gpu.derived_explicit();
    let source = patterned(9, 7);
    let carrier = upload(&observed, &source);
    let params = resize_only(4, 3);
    let context = ObservationContext::new(CollectionMode::Summary);
    observed.context_mut(context.clone());
    let cancelled_before_dispatch = Arc::new(AtomicBool::new(true));
    {
        let cancelled = observed
            .with_cancellation(Some(cancelled_before_dispatch.clone()))
            .unwrap();
        assert!(
            matches!(
                cancelled.input_geometry(&carrier, &params),
                Err(error) if error == "operation cancelled"
            ),
            "a request cancelled up front must not run"
        );
    }
    let work = context.snapshot().totals;
    assert_eq!(
        work.dispatch_count,
        Some(0),
        "cancellation must land before the first dispatch"
    );
    assert_eq!(
        work.submit_count,
        Some(0),
        "cancellation must land before the first submit"
    );
    assert_eq!(
        work.device_copy_count,
        Some(0),
        "cancellation must land before any transfer"
    );
    cancelled_before_dispatch.store(false, Ordering::Relaxed);
    let expected = reference_preview(&source, 4, 3);
    let out = observed
        .input_geometry(&carrier, &params)
        .unwrap()
        .expect("the wgpu backend has device geometry");
    assert_within(
        &observed.materialize_buffer(&out).unwrap(),
        &expected,
        1e-6,
        "recovered preview after cancellation",
    );
    assert_eq!(
        observed.materialize_buffer(&carrier).unwrap().data,
        source.data,
        "the cancelled attempt must leave the retained source unchanged"
    );
}

#[test]
fn triangle_support_past_2_to_24_reproduces_the_reference_nan() {
    let gpu = crate::test_support::explicit_adapter();
    // At width 2^24 + 1 the f32 support of the last output pixel resolves to
    // the single source pixel 2^24 - 1: its triangle weight at the rounded
    // centre is zero, the normalization divides 0 by 0, and the reference
    // emits NaN samples. A kernel that rounds the support against f32(extent)
    // extends the taps and silently emits a finite pixel instead.
    // Synthetic pages keep the raster tiny: the large coordinates stay
    // logical, and only two source samples and one output sample are real.
    let source: [f32; 6] = [0.5, -0.25, 0.75, 1.25, 0.0, -1.0];
    let input = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("triangle_support_source"),
            contents: bytemuck::bytes_of(&source),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let output = gpu
        .allocate_image_pages(1, 1, "triangle_support_output")
        .unwrap();
    let uniform = gpu.geometry_uniform(
        "triangle_support_params",
        &TriangleParams {
            src_width: 16_777_217,
            src_height: 1,
            dst_width: 16_777_217,
            dst_height: 1,
            source_first: 16_777_215,
            source_count: 2,
            output_first: 16_777_215,
            output_count: 1,
            vertical: 0,
            flags: 3,
            _pad: [0; 2],
        },
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    gpu.geometry_pass_linear(
        &mut encoder,
        TRIANGLE_SHADER,
        &[UNIFORM, READ, WRITE],
        &[&uniform, &input, &output[0].buffer],
        1,
        "triangle_support_edge",
    );
    gpu.geometry_submit_pending(&mut encoder).unwrap();
    let out = gpu.retain_image_pages(
        output,
        1,
        1,
        spektrafilm_backend::SampleDomain::Linear,
        space(),
        Some(12.0),
    );
    let pixels = gpu.materialize_buffer(&out).unwrap();
    for &sample in &pixels.data {
        let value = to_f64(sample);
        assert!(
            value.is_nan(),
            "the reference's zero-weight normalization must reproduce NaN, got {value}"
        );
    }
}
