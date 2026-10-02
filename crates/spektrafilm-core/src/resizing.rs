//! Working-image geometry service — Rust mirror of Python 0.3.4's
//! `ResizingService.crop_and_rescale` (`runtime/services/resize.py`)
//! and `utils/crop_resize.py`.
//!
//! Semantics preserved from upstream:
//!
//! * `pixel_size_um = film_format_mm * 1000 / max(shape)` is derived
//!   from the **full** input image, *before* cropping: cropping changes
//!   the field of view, not the film magnification.
//! * The optional normalized crop uses the exact upstream convention
//!   (each `crop_size` component is a fraction of the **long** edge,
//!   `np.round` ties-to-even, clamp-then-shift bounds and numpy
//!   negative-index slice behavior).
//! * The optional upscale divides `pixel_size_um` and resamples with
//!   `skimage.rescale(order=3, channel_axis=2)` semantics
//!   ([`spektrafilm_math::resize::rescale_spline3`]).
//!
//! The returned pixel pitch is pipeline state: every spatial stage
//! (lens blur, halation, DIR couplers, grain, enlarger diffusion)
//! consumes it instead of re-deriving it from the current image
//! dimensions, which would drift whenever a crop is active.

use std::borrow::Cow;

use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::resize::rescale_spline3;

use crate::params::IoParams;

/// Python `ResizingService`: `pixel_size_um = film_format_mm * 1000 /
/// max(shape)` (float64).
pub fn pixel_size_um(film_format_mm: f32, width: u32, height: u32) -> f64 {
    film_format_mm as f64 * 1000.0 / (width.max(height) as f64)
}

/// Round half to even (`np.round` / Python `round`).
fn round_half_even(x: f64) -> f64 {
    let f = x.floor();
    let diff = x - f;
    if diff < 0.5 {
        f
    } else if diff > 0.5 {
        f + 1.0
    } else if (f as i64) % 2 == 0 {
        f
    } else {
        f + 1.0
    }
}

/// numpy basic-slice normalization for `image[start:stop]` on an axis
/// of length `n`: negative bounds wrap once, then clamp.
fn numpy_slice_norm(start: i64, stop: i64, n: i64) -> (i64, i64) {
    let mut start = start;
    let mut stop = stop;
    if start < 0 {
        start += n;
        if start < 0 {
            start = 0;
        }
    } else if start > n {
        start = n;
    }
    if stop < 0 {
        stop += n;
        if stop < 0 {
            stop = 0;
        }
    } else if stop > n {
        stop = n;
    }
    (start, stop)
}

/// Pixel crop rectangle `(x0, y0, width, height)` for a normalized
/// crop, exactly mirroring `spektrafilm.utils.crop_resize.crop_image`
/// (0.3.4):
///
/// * `center` is `(x, y)` in [0, 1]; each `size` component is a
///   fraction of the **long** edge;
/// * `np.round` (ties to even) rounds the center, size and origin;
/// * a negative origin is clamped to 0 first, then shifted back so the
///   crop fits — which can go negative again for crops larger than the
///   image; that residual negative value reproduces numpy's
///   negative-index wrap (e.g. a full-long-edge crop on a non-square
///   image returns the whole axis, and a 0.75-long-edge crop on the
///   short axis takes the trailing slice).
fn crop_rect(
    w: u32,
    h: u32,
    center: [f64; 2],
    size: [f64; 2],
) -> Result<(usize, usize, usize, usize), String> {
    let dims = [h as i64, w as i64];
    if center.iter().chain(size.iter()).any(|v| !v.is_finite() || v.abs() > i32::MAX as f64) {
        return Err("io.crop_center and io.crop_size must contain finite normalized coordinates within image index limits".into());
    }
    let max_dim = h.max(w) as f64;

    // center arrives as (x, y); Python flips to (row, col)
    let cn = [
        round_half_even(h as f64 * center[1]),
        round_half_even(w as f64 * center[0]),
    ];
    // sizes are fractions of the long edge, flipped to (rows, cols)
    let sz = [
        round_half_even(max_dim * size[1]),
        round_half_even(max_dim * size[0]),
    ];
    let x0 = [
        round_half_even(cn[0] - sz[0] / 2.0),
        round_half_even(cn[1] - sz[1] / 2.0),
    ];
    let sz = [sz[0] as i64, sz[1] as i64];
    let mut x0 = [x0[0] as i64, x0[1] as i64];
    if x0[0] < 0 {
        x0[0] = 0;
    }
    if x0[1] < 0 {
        x0[1] = 0;
    }
    for a in 0..2 {
        if x0[a] + sz[a] > dims[a] {
            x0[a] = dims[a] - sz[a]; // may stay negative → numpy wrap below
        }
    }

    let (r0, r1) = numpy_slice_norm(x0[0], x0[0] + sz[0], dims[0]);
    let (c0, c1) = numpy_slice_norm(x0[1], x0[1] + sz[1], dims[1]);
    let ch = r1 - r0;
    let cw = c1 - c0;
    if ch <= 0 || cw <= 0 {
        return Err(format!(
            "io.crop_size {:?} rounds to {}x{} px on a {}x{} image \
             (each component is a fraction of the {} px long edge) — the crop \
             would be empty; increase io.crop_size",
            size,
            cw,
            ch,
            w,
            h,
            max_dim
        ));
    }
    Ok((c0 as usize, r0 as usize, cw as usize, ch as usize))
}

/// Crop `image` by the normalized `center`/`size` rectangle (see
/// [`crop_rect`]). Mirrors Python `crop_image`: pixels are copied
/// verbatim, no interpolation.
pub fn crop_image(image: &ImageBuf, center: [f64; 2], size: [f64; 2]) -> Result<ImageBuf, String> {
    let (x0, y0, cw, ch) = crop_rect(image.width, image.height, center, size)?;
    let src_w = image.width as usize;
    let mut data = Vec::with_capacity(cw * ch * 3);
    for y in y0..y0 + ch {
        let row = (y * src_w + x0) * 3;
        data.extend_from_slice(&image.data[row..row + cw * 3]);
    }
    Ok(ImageBuf::from_data(cw as u32, ch as u32, data))
}

/// Python `ResizingService.crop_and_rescale`: crop + upscale the
/// working image and derive the pixel pitch the spatial stages must
/// use.
///
/// Returns the working image (borrowed when neither crop nor upscale is
/// active) and `pixel_size_um`. Fails with an actionable message when
/// the requested crop would be empty or the upscale factor is not a
/// finite positive value.
pub fn crop_and_rescale<'a>(
    image: &'a ImageBuf,
    io: &IoParams,
    film_format_mm: f32,
) -> Result<(Cow<'a, ImageBuf>, f64), String> {
    // Pitch from the FULL image — before any crop.
    let mut pixel_size_um = pixel_size_um(film_format_mm, image.width, image.height);
    let mut working = Cow::Borrowed(image);
    if io.crop {
        working = Cow::Owned(crop_image(image, io.crop_center, io.crop_size)?);
    }
    // Python compares `!= 1.0` exactly.
    if io.upscale_factor != 1.0 {
        if !io.upscale_factor.is_finite() || io.upscale_factor <= 0.0 {
            return Err(format!(
                "io.upscale_factor must be a finite value > 0 (got {})",
                io.upscale_factor
            ));
        }
        pixel_size_um /= io.upscale_factor;
        working = Cow::Owned(rescale_spline3(&working, io.upscale_factor)?);
    }
    Ok((working, pixel_size_um))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::from_f64;

    /// Grayscale-coded image: pixel (x, y) → [x, y, 0.5] so tests can
    /// identify which source pixel landed where.
    fn coded_img(w: u32, h: u32) -> ImageBuf {
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                data.push(from_f64(x as f64));
                data.push(from_f64(y as f64));
                data.push(from_f64(0.5));
            }
        }
        ImageBuf::from_data(w, h, data)
    }

    fn corners(img: &ImageBuf) -> [(f64, f64); 4] {
        let tl = (img.get(0, 0)[0] as f64, img.get(0, 0)[1] as f64);
        let tr = (
            img.get(img.width - 1, 0)[0] as f64,
            img.get(img.width - 1, 0)[1] as f64,
        );
        let bl = (
            img.get(0, img.height - 1)[0] as f64,
            img.get(0, img.height - 1)[1] as f64,
        );
        let br = (
            img.get(img.width - 1, img.height - 1)[0] as f64,
            img.get(img.width - 1, img.height - 1)[1] as f64,
        );
        [tl, tr, bl, br]
    }

    #[test]
    fn crop_centered_matches_python_reference() {
        // Python: crop_image(np.zeros((100, 200)), (0.5, 0.5), (0.1, 0.1))
        // → rows 40:60, cols 90:110
        let img = coded_img(200, 100);
        let out = crop_image(&img, [0.5, 0.5], [0.1, 0.1]).unwrap();
        assert_eq!((out.width, out.height), (20, 20));
        assert_eq!(corners(&out), [(90.0, 40.0), (109.0, 40.0), (90.0, 59.0), (109.0, 59.0)]);
    }

    #[test]
    fn crop_off_center_matches_python_reference() {
        // center (x=0.9, y=0.1) → rows 0:20, cols 170:190
        let img = coded_img(200, 100);
        let out = crop_image(&img, [0.9, 0.1], [0.1, 0.1]).unwrap();
        assert_eq!((out.width, out.height), (20, 20));
        assert_eq!(corners(&out), [(170.0, 0.0), (189.0, 0.0), (170.0, 19.0), (189.0, 19.0)]);
    }

    #[test]
    fn crop_edges_clamp_flush() {
        // center x=1.0 → right-flush cols 180:200; center y=0 → top rows
        let img = coded_img(200, 100);
        let out = crop_image(&img, [1.0, 0.5], [0.1, 0.1]).unwrap();
        assert_eq!((out.width, out.height), (20, 20));
        assert_eq!(corners(&out), [(180.0, 40.0), (199.0, 40.0), (180.0, 59.0), (199.0, 59.0)]);

        let out = crop_image(&img, [0.0, 0.0], [0.1, 0.1]).unwrap();
        assert_eq!(corners(&out), [(0.0, 0.0), (19.0, 0.0), (0.0, 19.0), (19.0, 19.0)]);
    }

    #[test]
    fn crop_non_square_uses_long_edge_and_bankers_rounding() {
        // 100x60, size (x=0.5, y=0.25): long edge 100 → 50x25 px crop;
        // row origin round(30 - 12.5) = round(17.5) = 18 (ties to even)
        let img = coded_img(100, 60);
        let out = crop_image(&img, [0.5, 0.5], [0.5, 0.25]).unwrap();
        assert_eq!((out.width, out.height), (50, 25));
        assert_eq!(corners(&out), [(25.0, 18.0), (74.0, 18.0), (25.0, 42.0), (74.0, 42.0)]);
    }

    #[test]
    fn crop_full_long_edge_returns_whole_image() {
        // size (1.0, 1.0) on 200x100: the 200 px row crop exceeds the
        // 100 px height; numpy's negative-index wrap lands on rows 0:100
        let img = coded_img(200, 100);
        let out = crop_image(&img, [0.5, 0.5], [1.0, 1.0]).unwrap();
        assert_eq!((out.width, out.height), (200, 100));
        assert_eq!(corners(&out), [(0.0, 0.0), (199.0, 0.0), (0.0, 99.0), (199.0, 99.0)]);
    }

    #[test]
    fn crop_larger_than_short_axis_takes_trailing_slice() {
        // size 0.75 on 200x100 → 150 px on each axis; rows wrap to
        // 50:100 (numpy image[-50:100]), cols clamp to 25:175
        let img = coded_img(200, 100);
        let out = crop_image(&img, [0.5, 0.5], [0.75, 0.75]).unwrap();
        assert_eq!((out.width, out.height), (150, 50));
        assert_eq!(corners(&out), [(25.0, 50.0), (174.0, 50.0), (25.0, 99.0), (174.0, 99.0)]);
    }

    #[test]
    fn crop_bankers_rounding_on_center() {
        // 101x101, center 0.5 → np.round(50.5) = 50; size 0.2 → 20.2 → 20
        let img = coded_img(101, 101);
        let out = crop_image(&img, [0.5, 0.5], [0.2, 0.2]).unwrap();
        assert_eq!((out.width, out.height), (20, 20));
        assert_eq!(corners(&out), [(40.0, 40.0), (59.0, 40.0), (40.0, 59.0), (59.0, 59.0)]);
    }

    #[test]
    fn crop_exact_fit_does_not_shift() {
        // 103x101, center (0.33, 0.67), size (0.11, 0.37) → cols 28:39,
        // rows 49:87 (fits exactly, origin round(34 - 5.5) = 28)
        let img = coded_img(103, 101);
        let out = crop_image(&img, [0.33, 0.67], [0.11, 0.37]).unwrap();
        assert_eq!((out.width, out.height), (11, 38));
        assert_eq!(corners(&out), [(28.0, 49.0), (38.0, 49.0), (28.0, 86.0), (38.0, 86.0)]);
    }

    #[test]
    fn crop_smaller_than_one_pixel_is_an_actionable_error() {
        // 10x10 with size 0.05 → round(10 * 0.05) = round(0.5) = 0 px
        let img = coded_img(10, 10);
        let err = crop_image(&img, [0.5, 0.5], [0.05, 0.05]).unwrap_err();
        assert!(err.contains("crop_size"), "{err}");
        assert!(err.contains("increase io.crop_size"), "{err}");
    }

    #[test]
    fn pixel_size_uses_full_image_and_survives_crop() {
        let io = IoParams {
            crop: true,
            crop_center: [0.5, 0.5],
            crop_size: [0.1, 0.1],
            ..IoParams::default()
        };
        let img = coded_img(200, 100);
        let (working, pix) = crop_and_rescale(&img, &io, 35.0).unwrap();
        // 35 mm across the 200 px long edge; the 20x20 crop must NOT
        // change the pitch (field of view changes, not magnification)
        assert_eq!((working.width, working.height), (20, 20));
        assert!((pix - 35.0 * 1000.0 / 200.0).abs() < 1e-12, "{pix}");
    }

    #[test]
    fn upscale_divides_pixel_size_and_rescales() {
        let io = IoParams {
            upscale_factor: 2.0,
            ..IoParams::default()
        };
        let img = coded_img(100, 50);
        let (working, pix) = crop_and_rescale(&img, &io, 35.0).unwrap();
        assert_eq!((working.width, working.height), (200, 100));
        assert!((pix - 35.0 * 1000.0 / 100.0 / 2.0).abs() < 1e-12, "{pix}");
    }

    #[test]
    fn crop_and_upscale_combine_in_python_order() {
        // crop first, then rescale the cropped buffer; pitch from the
        // full image, divided by the upscale factor only
        let io = IoParams {
            crop: true,
            crop_center: [0.5, 0.5],
            crop_size: [0.5, 0.5],
            upscale_factor: 2.0,
            ..IoParams::default()
        };
        let img = coded_img(200, 100);
        let (working, pix) = crop_and_rescale(&img, &io, 35.0).unwrap();
        assert_eq!((working.width, working.height), (200, 200));
        assert!((pix - 35.0 * 1000.0 / 200.0 / 2.0).abs() < 1e-12, "{pix}");
    }

    #[test]
    fn no_crop_no_upscale_borrows_unchanged() {
        let io = IoParams::default();
        let img = coded_img(64, 32);
        let (working, pix) = crop_and_rescale(&img, &io, 24.0).unwrap();
        assert_eq!((working.width, working.height), (64, 32));
        assert!(matches!(working, Cow::Borrowed(_)));
        assert!((pix - 24.0 * 1000.0 / 64.0).abs() < 1e-12, "{pix}");
    }

    #[test]
    fn invalid_upscale_factor_is_an_actionable_error() {
        for bad in [0.0f64, -2.0, f64::NAN, f64::INFINITY] {
            let io = IoParams {
                upscale_factor: bad,
                ..IoParams::default()
            };
            let img = coded_img(8, 8);
            let err = crop_and_rescale(&img, &io, 35.0).unwrap_err();
            assert!(err.contains("upscale_factor"), "{err}");
        }
    }

    #[test]
    fn crop_pixels_are_verbatim_copies() {
        let img = coded_img(200, 100);
        let out = crop_image(&img, [0.5, 0.5], [0.1, 0.1]).unwrap();
        for y in 0..out.height {
            for x in 0..out.width {
                let got = out.get(x, y);
                let want = img.get(x + 90, y + 40);
                for c in 0..3 {
                    assert_eq!(got[c], want[c], "({x},{y},ch{c})");
                }
            }
        }
    }
}
