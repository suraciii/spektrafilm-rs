//! Display color transforms: input/output viewing rasters and the optional
//! ICC display profile. Never touches export pixels.
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;

use super::raster::{DisplayRaster, capped_raster_with};
/// Primary display profile discovery matching Pillow get_display_profile().
#[cfg(windows)]
pub fn discover_display_profile() -> Result<Option<std::path::PathBuf>, String> {
    use std::{ffi::c_void, os::windows::ffi::OsStringExt};
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDC(hwnd: *mut c_void) -> *mut c_void;
        fn ReleaseDC(hwnd: *mut c_void, dc: *mut c_void) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn GetICMProfileW(dc: *mut c_void, size: *mut u32, path: *mut u16) -> i32;
    }
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return Err("Cannot acquire Windows display context".into());
        }
        let mut len = 0;
        GetICMProfileW(dc, &mut len, std::ptr::null_mut());
        if len == 0 {
            ReleaseDC(std::ptr::null_mut(), dc);
            return Ok(None);
        }
        let mut path = vec![0_u16; len as usize];
        let ok = GetICMProfileW(dc, &mut len, path.as_mut_ptr());
        ReleaseDC(std::ptr::null_mut(), dc);
        if ok == 0 {
            return Err("Cannot discover Windows display ICC profile".into());
        }
        let end = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        Ok(Some(std::ffi::OsString::from_wide(&path[..end]).into()))
    }
}

/// Takes already encoded sRGB viewing pixels, never an export ImageBuf.
/// ICC conversion is quantized to RGB8 as in the pinned Pillow workflow.
pub fn apply_display_profile(
    raster: &DisplayRaster,
    path: &std::path::Path,
) -> Result<DisplayRaster, String> {
    use lcms2::{Intent, PixelFormat, Profile, Transform};
    let source = Profile::new_srgb();
    let destination = Profile::new_file(path).map_err(|e| e.to_string())?;
    let transform: Transform<[u8; 3], [u8; 3]> = Transform::new(
        &source,
        PixelFormat::RGB_8,
        &destination,
        PixelFormat::RGB_8,
        Intent::Perceptual,
    )
    .map_err(|e| e.to_string())?;
    let mut rgb: Vec<[u8; 3]> = raster
        .rgb
        .iter()
        .map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8))
        .collect();
    transform.transform_in_place(&mut rgb);
    DisplayRaster::new(
        raster.size,
        rgb.into_iter()
            .map(|p| p.map(|v| v as f32 / 255.0))
            .collect(),
    )
}

/// Viewer ICC conversion from authored profile data. The host parses LCMS
/// stages; the device prepares numerical tables and evaluates image samples.
/// A selected GPU error propagates rather than running LCMS over image data.
/// The CPU backend retains its independent LCMS reference implementation.
pub fn apply_display_profile_with(
    backend: &dyn ComputeBackend,
    raster: &DisplayRaster,
    path: &std::path::Path,
) -> Result<Option<DisplayRaster>, String> {
    use lcms2::Profile;
    let source = Profile::new_srgb();
    let destination = Profile::new_file(path).map_err(|e| e.to_string())?;
    let resolved = resolve_viewer_icc_pipeline(&source, &destination)?;
    let device = backend
        .viewer_icc_transform(&raster.rgb, &resolved.params())
        .map_err(|error| format!("viewer ICC transform: {error}"))?;
    let Some(rgb) = device else {
        return Ok(None);
    };
    Ok(Some(DisplayRaster::new(raster.size, rgb)?))
}

#[path = "icc_pipeline.rs"]
mod icc_pipeline;
pub(crate) use icc_pipeline::ResolvedIccPipeline;

/// Extract authored LCMS profile data; device preparation owns curve inversion,
/// optimizer shapers, prelinearization, and generated interpolation lattices.
pub(crate) fn resolve_viewer_icc_pipeline(
    source: &lcms2::Profile,
    destination: &lcms2::Profile,
) -> Result<ResolvedIccPipeline, String> {
    icc_pipeline::resolve(source, destination)
}

/// Prepares a disposable viewing raster. The CPU backend uses the independent
/// LCMS reference; a selected GPU never falls back to host image arithmetic.
pub fn prepare_display_raster_with(
    backend: &dyn ComputeBackend,
    raw_output: DisplayRaster,
    srgb_preview: Option<DisplayRaster>,
    enabled: bool,
    selected_profile: Option<&std::path::Path>,
) -> (DisplayRaster, crate::app::Message) {
    if !enabled {
        return (
            raw_output,
            crate::app::Message::new("surfaces.display.disabled"),
        );
    }
    let Some(srgb) = srgb_preview else {
        return (
            raw_output,
            crate::app::Message::new("surfaces.display.missing_preview"),
        );
    };
    #[cfg(windows)]
    {
        let discovered = if selected_profile.is_none() {
            discover_display_profile()
        } else {
            Ok(None)
        };
        let profile = selected_profile
            .map(std::path::Path::to_path_buf)
            .or_else(|| discovered.ok().flatten());
        if let Some(path) = profile {
            match apply_display_profile_with(backend, &srgb, &path) {
                Ok(Some(raster)) => {
                    return (
                        raster,
                        crate::app::Message::new("surfaces.display.active")
                            .arg("path", path.display()),
                    );
                }
                Ok(None) if backend.is_gpu() => {
                    return (
                        srgb,
                        crate::app::Message::new("surfaces.display.failed")
                            .arg("error", "selected GPU has no ICC transform"),
                    );
                }
                Ok(None) => match apply_display_profile(&srgb, &path) {
                    Ok(raster) => {
                        return (
                            raster,
                            crate::app::Message::new("surfaces.display.active")
                                .arg("path", path.display()),
                        );
                    }
                    Err(e) => {
                        return (
                            srgb,
                            crate::app::Message::new("surfaces.display.failed").arg("error", e),
                        );
                    }
                },
                Err(e) => {
                    return (
                        srgb,
                        crate::app::Message::new("surfaces.display.failed").arg("error", e),
                    );
                }
            }
        }
        return (
            srgb,
            crate::app::Message::new("surfaces.display.no_profile"),
        );
    }
    #[cfg(not(windows))]
    {
        if let Some(path) = selected_profile {
            match apply_display_profile_with(backend, &srgb, path) {
                Ok(Some(raster)) => {
                    return (
                        raster,
                        crate::app::Message::new("surfaces.display.active")
                            .arg("path", path.display()),
                    );
                }
                Ok(None) if backend.is_gpu() => {
                    return (
                        srgb,
                        crate::app::Message::new("surfaces.display.failed")
                            .arg("error", "selected GPU has no ICC transform"),
                    );
                }
                Ok(None) => match apply_display_profile(&srgb, path) {
                    Ok(raster) => {
                        return (
                            raster,
                            crate::app::Message::new("surfaces.display.active")
                                .arg("path", path.display()),
                        );
                    }
                    Err(e) => {
                        return (
                            srgb,
                            crate::app::Message::new("surfaces.display.failed").arg("error", e),
                        );
                    }
                },
                Err(e) => {
                    return (
                        srgb,
                        crate::app::Message::new("surfaces.display.failed").arg("error", e),
                    );
                }
            }
        }
        (
            srgb,
            crate::app::Message::new("surfaces.display.no_profile"),
        )
    }
}

/// Retained viewer preparation. Geometry and profile parsing are host controls;
/// source samples, resampling, display and ICC evaluation stay on the device.
pub(crate) fn output_display_raster_retained(
    backend: &dyn ComputeBackend,
    image: &spektrafilm_backend::DeviceImage,
    space: &str,
    encoded: bool,
    enabled: bool,
    profile: Option<&std::path::Path>,
    max_edge: usize,
) -> Result<(DisplayRaster, crate::app::Message), String> {
    let space = spektrafilm_math::colorspace::resolve(space)?;
    let factor =
        (max_edge.max(1) as f64 / image.width().max(image.height()).max(1) as f64).min(1.0);
    let width = (image.width() as f64 * factor).round().max(1.0) as u32;
    let height = (image.height() as f64 * factor).round().max(1.0) as u32;
    let display = enabled.then(|| spektrafilm_backend::DisplayTransformGpuParams {
        src_cctf: spektrafilm_backend::cctf_kind(space.cctf),
        decode: encoded,
        matrix: std::array::from_fn(|i| {
            std::array::from_fn(|j| space.display_matrix()[i][j] as f32)
        }),
        clamp: true,
    });
    #[cfg(windows)]
    let discovered = if enabled && profile.is_none() {
        discover_display_profile().ok().flatten()
    } else {
        None
    };
    #[cfg(windows)]
    let profile = profile.or(discovered.as_deref());
    let mut status = crate::app::Message::new(if enabled {
        "surfaces.display.no_profile"
    } else {
        "surfaces.display.disabled"
    });
    let resolved = if enabled {
        if let Some(path) = profile {
            use lcms2::Profile;
            let source = Profile::new_srgb();
            let profile =
                Profile::new_file(path)
                    .map_err(|e| e.to_string())
                    .and_then(|destination| {
                        let output = if matches!(
                            destination.device_class(),
                            lcms2::ProfileClassSignature::LinkClass
                                | lcms2::ProfileClassSignature::AbstractClass
                        ) {
                            destination.pcs()
                        } else {
                            destination.color_space()
                        };
                        if output != lcms2::ColorSpaceSignature::RgbData {
                            return Err("ICC output format is not RGB".to_owned());
                        }
                        Ok(destination)
                    });
            match profile {
                Ok(destination) => {
                    status = crate::app::Message::new("surfaces.display.active")
                        .arg("path", path.display());
                    Some(resolve_viewer_icc_pipeline(&source, &destination)?)
                }
                Err(e) => {
                    status = crate::app::Message::new("surfaces.display.failed").arg("error", e);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };
    let icc = resolved.as_ref().map(|r| r.params());
    let image =
        backend.viewer_fit_retained(image, width, height, display.as_ref(), icc.as_ref())?;
    Ok((DisplayRaster::device(image), status))
}

/// Prepares an input viewing raster on the selected backend. A selected GPU
/// uses retained preparation and propagates errors without a host fallback.
pub fn input_display_raster_with(
    backend: &dyn ComputeBackend,
    image: &ImageBuf,
    space: &str,
    decode: bool,
    max_edge: usize,
) -> Result<DisplayRaster, String> {
    if backend.is_gpu() {
        return output_display_raster_with(backend, image, space, decode, true, None, max_edge)
            .map(|(r, _)| r);
    }
    use spektrafilm_math::colorspace::{display_rgb, resolve};
    let space = resolve(space)?;
    let raw = capped_raster_with(backend, image, max_edge)?;
    let src_cctf = spektrafilm_backend::cctf_kind(space.cctf);
    let matrix =
        std::array::from_fn(|i| std::array::from_fn(|j| space.display_matrix()[i][j] as f32));
    // The device transform is the viewer's display transform, not the saving
    // conversion: decode, display matrix, sRGB encode, and the display clamp.
    let device = backend
        .viewer_display_transform(
            &raw.rgb,
            &spektrafilm_backend::DisplayTransformGpuParams {
                src_cctf,
                decode,
                matrix,
                clamp: true,
            },
        )
        .map_err(|error| format!("viewer display transform: {error}"))?;
    let transformed = match device {
        Some(rgb) => rgb,
        None => raw
            .rgb
            .iter()
            .map(|p| {
                display_rgb(p.map(|v| v as f64), space, decode).map(|v| v.clamp(0.0, 1.0) as f32)
            })
            .collect(),
    };
    DisplayRaster::new(raw.size, transformed)
}

pub fn output_display_raster_with(
    backend: &dyn ComputeBackend,
    image: &ImageBuf,
    space: &str,
    encoded: bool,
    enabled: bool,
    profile: Option<&std::path::Path>,
    max_edge: usize,
) -> Result<(DisplayRaster, crate::app::Message), String> {
    if backend.is_gpu() {
        let color_space = spektrafilm_math::colorspace::resolve(space)?;
        let retained = backend.retain_image(
            image,
            &spektrafilm_backend::RetainedImageMeta {
                width: image.width,
                height: image.height,
                domain: if encoded {
                    spektrafilm_backend::SampleDomain::Encoded
                } else {
                    spektrafilm_backend::SampleDomain::Linear
                },
                precision: spektrafilm_backend::Precision::host(),
                space: color_space,
                pixel_pitch_um: None,
            },
        )?;
        return output_display_raster_retained(
            backend, &retained, space, encoded, enabled, profile, max_edge,
        );
    }
    let raw = capped_raster_with(backend, image, max_edge)?;
    let srgb = enabled
        .then(|| input_display_raster_with(backend, image, space, encoded, max_edge))
        .transpose()?;
    Ok(prepare_display_raster_with(
        backend, raw, srgb, enabled, profile,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::cpu_backend::CpuBackend;

    #[cfg(not(windows))]
    #[test]
    fn enabled_non_srgb_output_matches_colour_conversion() {
        use spektrafilm_math::colorspace::{display_rgb, resolve};

        let image = ImageBuf::from_data(1, 1, vec![0.75 as _, 0.20 as _, 0.10 as _]);
        let source = image.data.clone();
        let (actual, status) =
            output_display_raster_with(&CpuBackend, &image, "Display P3", true, true, None, 1)
                .unwrap();
        let space = resolve("Display P3").unwrap();
        let expected = display_rgb([0.75, 0.20, 0.10], space, true);

        for (actual, expected) in actual.rgb[0].into_iter().zip(expected) {
            assert!((actual as f64 - expected.clamp(0.0, 1.0)).abs() < 2e-6);
        }
        assert!(status.to_string().contains("sRGB preview"));
        assert_eq!(image.data, source);
    }

    // Windows discovers and applies the runner's display ICC profile whenever no
    // profile is selected, so the preview is the profile-converted raster rather
    // than the sRGB expectation below. The discovery path is Windows-only by
    // design (`discover_display_profile`); keep this assertion off that platform.
    #[cfg(not(windows))]
    #[test]
    fn enabled_preview_clips_out_of_gamut_display_values_only() {
        use spektrafilm_math::colorspace::{display_rgb, resolve};

        let image = ImageBuf::from_data(1, 1, vec![1.3 as _, -0.2 as _, 0.4 as _]);
        let source = image.data.clone();
        let raw = capped_raster_with(&CpuBackend, &image, 1).unwrap();
        let (preview, _) =
            output_display_raster_with(&CpuBackend, &image, "Display P3", false, true, None, 1)
                .unwrap();
        let space = resolve("Display P3").unwrap();
        let expected = display_rgb([1.3, -0.2, 0.4], space, false);

        assert_eq!(raw.rgb[0], [1.3, -0.2, 0.4]);
        for (actual, expected) in preview.rgb[0].into_iter().zip(expected) {
            assert!((actual as f64 - expected.clamp(0.0, 1.0)).abs() < 2e-6);
            assert!((0.0..=1.0).contains(&actual));
        }
        assert_eq!(image.data, source);
    }

    #[test]
    fn disabled_transform_keeps_output_space_values() {
        let image = ImageBuf::from_data(1, 1, vec![1.3 as _, -0.2 as _, 0.4 as _]);
        let (raster, status) =
            output_display_raster_with(&CpuBackend, &image, "Display P3", false, false, None, 1)
                .unwrap();

        assert_eq!(raster.rgb[0], [1.3, -0.2, 0.4]);
        assert!(status.to_string().contains("output-space values"));
    }

    /// The device display transform must reproduce the viewer's host
    /// `display_rgb`: the same display matrix, the same source transfer
    /// decode, the same sRGB encode, and the same 0..=1 display clamp. A
    /// device path that approximated any of the three with a different
    /// matrix or transfer function changes what the viewer shows, so the
    /// comparison is against the reference itself, not a tolerance chosen
    /// after the fact.
    #[test]
    fn device_display_transform_matches_the_host_reference() {
        use spektrafilm_math::colorspace::{display_rgb, resolve};

        // The software Vulkan driver is not safe under concurrent device use
        // in one process; the guard serializes every device test in this
        // binary and must outlive the backend.
        let backend = spektrafilm_gpu::test_support::adapter();
        let spaces = ["sRGB", "Display P3", "Adobe RGB (1998)", "ITU-R BT.2020"];
        // Out-of-gamut and negative inputs: the clamp and the sign-preserving
        // transfer powers are exactly where an approximation would diverge.
        let samples: Vec<[f32; 3]> = [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.75, 0.2, 0.1],
            [0.18, 0.5, 0.9],
            [1.3, -0.2, 0.4],
            [-0.05, 0.02, 1.1],
            [0.5, 0.5, 0.5],
        ]
        .iter()
        .map(|p| p.map(|v| v as f32))
        .collect();
        for name in spaces {
            let space = resolve(name).unwrap();
            for decode in [false, true] {
                let device = backend
                    .viewer_display_transform(
                        &samples,
                        &spektrafilm_backend::DisplayTransformGpuParams {
                            src_cctf: spektrafilm_backend::cctf_kind(space.cctf),
                            decode,
                            matrix: std::array::from_fn(|i| {
                                std::array::from_fn(|j| space.display_matrix()[i][j] as f32)
                            }),
                            clamp: true,
                        },
                    )
                    .expect("device display transform")
                    .expect("WGPU provides the device display transform");
                assert_eq!(device.len(), samples.len());
                let mut worst = 0.0f64;
                for (got, sample) in device.iter().zip(samples.iter()) {
                    let want = display_rgb(sample.map(f64::from), space, decode)
                        .map(|v| v.clamp(0.0, 1.0));
                    for c in 0..3 {
                        worst = worst.max((f64::from(got[c]) - want[c]).abs());
                    }
                }
                // f32 evaluation of the reference's decode, matrix, encode and
                // clamp. Measured 3.4e-7 over the 8 space/decode combinations.
                assert!(
                    worst < 1e-6,
                    "{name} (decode {decode}): the device display transform differs from \
                     the reference by {worst:.3e}"
                );
            }
        }
    }

    /// The viewer transform is disposable by construction: the transform reads
    /// the resampled raster and never writes the image, so the samples the
    /// pipeline retained and the samples the writer saves are unchanged.
    #[test]
    fn device_display_transform_never_writes_the_source_image() {
        use spektrafilm_math::precision::from_f64;

        let backend = spektrafilm_gpu::test_support::adapter();
        let image = ImageBuf::from_data(
            3,
            2,
            (0..18).map(|i| from_f64(f64::from(i) / 17.0)).collect(),
        );
        let retained = image.data.clone();
        let raster =
            input_display_raster_with(backend.backend(), &image, "Display P3", true, 8).unwrap();
        // The display raster is the transformed one, so the viewer shows the
        // device result and not the source values.
        let viewed = raster
            .retained
            .as_ref()
            .unwrap()
            .to_host(backend.backend())
            .unwrap();
        assert_ne!(viewed.get(0, 0), image.get(0, 0));
        assert_eq!(image.data, retained);
    }

    mod icc_parity {
        include!("icc_parity.rs");
    }
}
