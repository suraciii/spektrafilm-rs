//! Device input transfer decoding acceptance.
//!
//! The chain-head `input_decode` pass must reproduce
//! `spektrafilm_math::colorspace::cctf_decode` for the input space's CCTF.
//! The comparison is against the CPU reference at the f32 device boundary, so
//! the budget is the f32 rounding of the transfer function itself, not an
//! algorithm tolerance: every space here is a closed-form per-channel
//! function, so any larger difference means the shader implements a different
//! transfer curve.
//!
//! The dispatch below uses the same bounded state constructor and submission
//! path as the chain, so shader, uniform-layout, or paging drift fails here.

use super::*;

/// One sample per decade plus the exact piecewise breakpoints of every
/// supported space, so a wrong branch boundary cannot hide between samples.
fn probe_values() -> Vec<f64> {
    let mut values = vec![
        0.0,
        // sRGB / ACEScct / ProPhoto / Rec2020 breakpoints.
        0.0031308,
        12.92 * 0.0031308,
        0.04045,
        0.155251141552511,
        0.0729055341958355,
        1.0 / 512.0,
        16.0 * (1.0 / 512.0),
        0.018,
        1.099 * 0.018f64.powf(0.45) - 0.099,
        1.0,
        0.5,
    ];
    // Encoded input samples live in roughly [-0.5, 4]; beyond that an
    // exponential decode branch exceeds the f32 range, which is a
    // representation limit of the device format rather than a difference in
    // the transfer function. Cover the working domain densely, including the
    // signed headroom the sign-preserving powers must keep finite.
    let mut v = 1e-6;
    while v <= 4.0 {
        values.push(v);
        values.push(-v);
        v *= 1.6;
    }
    values
}

#[test]
fn device_input_decode_matches_the_cpu_reference() {
    let backend = crate::test_support::adapter();
    let values = probe_values();
    let n = values.len() as u32;
    let data: Vec<f32> = values
        .iter()
        .flat_map(|&v| [v as f32, v as f32, v as f32])
        .collect();
    // The pass writes in place, so each dispatch starts from the same
    // original samples; a fresh copy per kind keeps kinds independent.
    for (kind, cctf) in [
        (0u32, spektrafilm_math::colorspace::Cctf::Linear),
        (1, spektrafilm_math::colorspace::Cctf::Acescct),
        (2, spektrafilm_math::colorspace::Cctf::Srgb),
        (3, spektrafilm_math::colorspace::Cctf::ProPhoto),
        (4, spektrafilm_math::colorspace::Cctf::Rec2020),
        (5, spektrafilm_math::colorspace::Cctf::AdobeRgb1998),
        (6, spektrafilm_math::colorspace::Cctf::Gamma2_6),
    ] {
        let params = spektrafilm_backend::InputDecodeGpuParams { cctf_kind: kind };
        let image = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("input_decode_probe"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            });
        let state = build_input_decode_state(&backend.device, &params, &image, n, &backend)
            .expect("the probe state stays within device binding limits");
        state.submit(&backend).expect("input decode submission");
        let got = backend
            .pipe
            .readback_buffer(&image, u64::from(n) * 12)
            .unwrap();
        let mut worst = 0.0f64;
        let mut worst_at = 0.0f64;
        for (i, &v) in values.iter().enumerate() {
            // The reference decodes the f32 sample the device received.
            let reference = spektrafilm_math::colorspace::cctf_decode(f64::from(v as f32), cctf);
            let diff = (reference - f64::from(got[i * 3])).abs();
            // Absolute floor plus a relative term: near an exact zero of a
            // linear branch (ACEScct at its offset) f32 subtraction cancels
            // catastrophically, so a pure relative metric would reject a
            // 1e-9 absolute difference.
            let error = diff / reference.abs().max(1.0);
            if error > worst {
                worst = error;
                worst_at = v;
            }
        }
        // The device evaluates the transfer function in f32 while the
        // reference is f64. An exponential branch amplifies the f32 rounding
        // of its exponent by ln 2, so ACEScct reaches ~1.1e-6 relative near
        // the top of the encoded domain — the f32 boundary, not a different
        // curve. Any real curve mismatch differs by orders of magnitude, so
        // the project's f32 device-agreement budget of 1e-5 is both
        // meaningful and honest about the precision boundary.
        assert!(
            worst < 1e-5,
            "cctf_kind {kind}: relative error {worst:.3e} at {worst_at} exceeds the f32 \
             transfer-function budget"
        );
    }
}
