//! Display color transforms: input/output viewing rasters and the optional
//! ICC display profile. Never touches export pixels.
use spektrafilm_math::image::ImageBuf;

use super::raster::{DisplayRaster, capped_raster};
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

/// A disabled transform explicitly views values in the output space. An
/// enabled transform always starts from the separate encoded-sRGB preview;
/// an ICC profile only adds the final device conversion.
pub fn prepare_display_raster(
    raw_output: DisplayRaster,
    srgb_preview: Option<DisplayRaster>,
    enabled: bool,
    selected_profile: Option<&std::path::Path>,
) -> (DisplayRaster, String) {
    if !enabled {
        return (
            raw_output,
            "Display transform: disabled; viewing output-space values".into(),
        );
    }
    let Some(srgb) = srgb_preview else {
        return (
            raw_output,
            "Display transform: missing sRGB preview; viewing output-space values".into(),
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
            match apply_display_profile(&srgb, &path) {
                Ok(raster) => {
                    return (
                        raster,
                        format!("Display transform: active ({})", path.display()),
                    );
                }
                Err(e) => {
                    return (
                        srgb,
                        format!("Display transform: ICC failed ({e}); viewing sRGB preview"),
                    );
                }
            }
        }
        return (
            srgb,
            "Display transform: sRGB preview; no display profile".into(),
        );
    }
    #[cfg(not(windows))]
    {
        if let Some(path) = selected_profile {
            match apply_display_profile(&srgb, path) {
                Ok(raster) => {
                    return (
                        raster,
                        format!("Display transform: active ({})", path.display()),
                    );
                }
                Err(e) => {
                    return (
                        srgb,
                        format!("Display transform: ICC failed ({e}); viewing sRGB preview"),
                    );
                }
            }
        }
        (
            srgb,
            "Display transform: sRGB preview; no display profile".into(),
        )
    }
}

pub fn input_display_raster(
    image: &ImageBuf,
    space: &str,
    decode: bool,
    max_edge: usize,
) -> Result<DisplayRaster, String> {
    use spektrafilm_math::colorspace::{display_rgb, resolve};
    let space = resolve(space)?;
    let raw = capped_raster(image, max_edge)?;
    DisplayRaster::new(
        raw.size,
        raw.rgb
            .iter()
            .map(|p| {
                display_rgb(p.map(|v| v as f64), space, decode).map(|v| v.clamp(0.0, 1.0) as f32)
            })
            .collect(),
    )
}
pub fn output_display_raster(
    image: &ImageBuf,
    space: &str,
    encoded: bool,
    enabled: bool,
    profile: Option<&std::path::Path>,
    max_edge: usize,
) -> Result<(DisplayRaster, String), String> {
    let raw = capped_raster(image, max_edge)?;
    let srgb = enabled
        .then(|| input_display_raster(image, space, encoded, max_edge))
        .transpose()?;
    Ok(prepare_display_raster(raw, srgb, enabled, profile))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(windows))]
    #[test]
    fn enabled_non_srgb_output_matches_colour_conversion() {
        use spektrafilm_math::colorspace::{display_rgb, resolve};

        let image = ImageBuf::from_data(1, 1, vec![0.75 as _, 0.20 as _, 0.10 as _]);
        let source = image.data.clone();
        let (actual, status) =
            output_display_raster(&image, "Display P3", true, true, None, 1).unwrap();
        let space = resolve("Display P3").unwrap();
        let expected = display_rgb([0.75, 0.20, 0.10], space, true);

        for (actual, expected) in actual.rgb[0].into_iter().zip(expected) {
            assert!((actual as f64 - expected.clamp(0.0, 1.0)).abs() < 2e-6);
        }
        assert!(status.contains("sRGB preview"));
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
        let raw = capped_raster(&image, 1).unwrap();
        let (preview, _) =
            output_display_raster(&image, "Display P3", false, true, None, 1).unwrap();
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
            output_display_raster(&image, "Display P3", false, false, None, 1).unwrap();

        assert_eq!(raster.rgb[0], [1.3, -0.2, 0.4]);
        assert!(status.contains("output-space values"));
    }
}
