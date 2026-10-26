use rayon::prelude::*;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{Scalar, exp10_inplace, from_f32, from_f64, to_f32, to_f64};
use spektrafilm_math::spectral;

use crate::{ComputeBackend, Lut3D};

// Pull in BLAS-src so `cblas_dgemm` is available at link time. The
// provider crate (Accelerate on macOS, OpenBLAS elsewhere) is selected
// in this crate's Cargo.toml.
#[cfg(not(target_os = "windows"))]
#[allow(unused_imports)]
use blas_src as _;

#[derive(Clone, Copy)]
enum Transpose {
    None,
    Ordinary,
}

/// Single-call dgemm `C[M×N] = A[M×K] · op(B)` in row-major layout.
///
/// We deliberately issue ONE `cblas_dgemm` over the full M rather than
/// splitting M across rayon and dispatching one dgemm per chunk: macOS
/// Accelerate's BLAS is not safe to enter concurrently from multiple
/// threads — concurrent calls corrupt memory and segfault
/// intermittently (reproduced on the scan_film path; latent for every
/// concurrent caller). A single call lets Accelerate parallelise
/// internally and keeps numpy bit-parity, since each output row's
/// K-element dot product was always computed inside one dgemm anyway.
///
/// `trans_b` controls whether B is transposed on the fly; `b` is
/// passed verbatim to BLAS in row-major layout with leading dimension
/// `ldb`.
#[allow(clippy::too_many_arguments)]
fn dgemm_blas(
    a: &[f64],
    b: &[f64],
    c: &mut [f64],
    m: usize,
    n: usize,
    k: usize,
    trans_b: Transpose,
    ldb: i32,
) {
    assert_eq!(a.len(), m * k, "dgemm_blas: a.len() != m*k");
    assert_eq!(c.len(), m * n, "dgemm_blas: c.len() != m*n");
    #[cfg(not(target_os = "windows"))]
    unsafe {
        cblas::dgemm(
            cblas::Layout::RowMajor,
            cblas::Transpose::None,
            match trans_b {
                Transpose::None => cblas::Transpose::None,
                Transpose::Ordinary => cblas::Transpose::Ordinary,
            },
            m as i32,
            n as i32,
            k as i32,
            1.0,
            a,
            k as i32,
            b,
            ldb,
            0.0,
            c,
            n as i32,
        );
    }
    #[cfg(target_os = "windows")]
    {
        let _ = ldb;
        c.par_chunks_exact_mut(n)
            .enumerate()
            .for_each(|(row, crow)| {
                for col in 0..n {
                    let mut sum = 0.0f64;
                    for kk in 0..k {
                        let bv = match trans_b {
                            Transpose::None => b[kk * n + col],
                            Transpose::Ordinary => b[col * k + kk],
                        };
                        sum += a[row * k + kk] * bv;
                    }
                    crow[col] = sum;
                }
            });
    }
}

pub struct CpuBackend;

impl ComputeBackend for CpuBackend {
    fn colorspace_convert(
        &self,
        img: &ImageBuf,
        matrix: &[[f32; 3]; 3],
    ) -> Result<ImageBuf, String> {
        let m: [[Scalar; 3]; 3] = [
            [
                from_f32(matrix[0][0]),
                from_f32(matrix[0][1]),
                from_f32(matrix[0][2]),
            ],
            [
                from_f32(matrix[1][0]),
                from_f32(matrix[1][1]),
                from_f32(matrix[1][2]),
            ],
            [
                from_f32(matrix[2][0]),
                from_f32(matrix[2][1]),
                from_f32(matrix[2][2]),
            ],
        ];
        let mut out = img.clone();
        out.par_pixels_mut().for_each(|px| {
            let r = px[0];
            let g = px[1];
            let b = px[2];
            px[0] = m[0][0] * r + m[0][1] * g + m[0][2] * b;
            px[1] = m[1][0] * r + m[1][1] * g + m[1][2] * b;
            px[2] = m[2][0] * r + m[2][1] * g + m[2][2] * b;
        });
        Ok(out)
    }

    fn cctf_encode_srgb(&self, img: &ImageBuf) -> Result<ImageBuf, String> {
        let mut out = img.clone();
        out.data
            .par_iter_mut()
            .for_each(|v| *v = spektrafilm_math::precision::srgb_encode(*v));
        Ok(out)
    }

    fn cctf_decode_srgb(&self, img: &ImageBuf) -> Result<ImageBuf, String> {
        let mut out = img.clone();
        out.data
            .par_iter_mut()
            .for_each(|v| *v = spektrafilm_math::precision::srgb_decode(*v));
        Ok(out)
    }

    fn gaussian_blur(&self, img: &ImageBuf, sigma: f32) -> Result<ImageBuf, String> {
        if sigma <= 0.0 {
            return Ok(img.clone());
        }
        Ok(spektrafilm_math::gaussian::gaussian_blur(img, sigma))
    }

    fn table_lookup(
        &self,
        img: &ImageBuf,
        table_x: &[f32],
        table_y: &[[f32; 3]],
    ) -> Result<ImageBuf, String> {
        let mut out = img.clone();
        out.par_pixels_mut().for_each(|px| {
            for c in 0..3 {
                let y_col: Vec<f32> = table_y.iter().map(|row| row[c]).collect();
                px[c] = from_f32(spektrafilm_math::interp::interp_1d(
                    table_x,
                    &y_col,
                    to_f32(px[c]),
                ));
            }
        });
        Ok(out)
    }

    fn lut3d_interp(&self, img: &ImageBuf, lut: &Lut3D) -> Result<ImageBuf, String> {
        let mut out = img.clone();
        let size = lut.size as usize;
        out.par_pixels_mut().for_each(|px| {
            let rgb = spektrafilm_math::lut::trilinear_3d(
                &lut.data,
                size,
                to_f32(px[0]),
                to_f32(px[1]),
                to_f32(px[2]),
            );
            px[0] = from_f32(rgb[0]);
            px[1] = from_f32(rgb[1]);
            px[2] = from_f32(rgb[2]);
        });
        Ok(out)
    }

    fn name(&self) -> &str {
        if cfg!(feature = "precision-f64") {
            "CPU (f64 reference, rayon)"
        } else {
            "CPU (f32, rayon)"
        }
    }
}

/// Tiled spectral integration shared by the print and scan CPU paths.
///
/// For each pixel: `proj · (10^(-(density·channel_densityᵀ + base)) · illuminant)`,
/// i.e. GEMM(density, channel_densityᵀ) → `10^(-…)·illuminant` (NaN→0) →
/// GEMM(·, proj). Returns the projected `n_pix × 3` buffer (XYZ via the CMFs,
/// or raw via the print sensitivity).
///
/// Tiles over pixels so the `tile × n_wl` spectral intermediate stays bounded —
/// the full `n_pix × n_wl` buffer is ~15 GB at 24 MP and thrashes swap. BLAS is
/// internally threaded so tiles run sequentially with no throughput loss, and
/// the vForce passes still parallelise within each tile. Bit-identical to the
/// untiled version (each output pixel is computed independently).
#[allow(clippy::too_many_arguments)]
fn spectral_to_proj_tiled(
    density_f64: &[f64],
    cd_flat: &[f64],
    base_density: &[f64],
    has_base: bool,
    illuminant: &[f64],
    proj_flat: &[f64],
    n_pix: usize,
    n_wl: usize,
) -> Vec<f64> {
    // ~256K px/tile → 256K × 81 × 8 B ≈ 170 MB spectral scratch, reused.
    const TILE: usize = 1 << 18;
    const ROWS_PER_TASK: usize = 4096;
    let mut out = vec![0.0f64; n_pix * 3];
    let mut spectral = vec![0.0f64; TILE.min(n_pix.max(1)) * n_wl];
    let mut start = 0;
    while start < n_pix {
        let tpx = (n_pix - start).min(TILE);
        let spec = &mut spectral[..tpx * n_wl];
        // GEMM 1: spec [tpx × n_wl] = density [tpx × 3] · channel_densityᵀ [3 × n_wl]
        dgemm_blas(
            &density_f64[start * 3..(start + tpx) * 3],
            cd_flat,
            spec,
            tpx,
            n_wl,
            3,
            Transpose::Ordinary,
            3,
        );
        // Three-pass vForce: -(d + base) → 10^x → ×illuminant (NaN→0).
        spec.par_chunks_mut(ROWS_PER_TASK * n_wl).for_each(|chunk| {
            if has_base {
                for row in chunk.chunks_exact_mut(n_wl) {
                    for wl in 0..n_wl {
                        row[wl] = -(row[wl] + base_density[wl]);
                    }
                }
            } else {
                for v in chunk.iter_mut() {
                    *v = -*v;
                }
            }
            exp10_inplace(chunk);
            for row in chunk.chunks_exact_mut(n_wl) {
                for wl in 0..n_wl {
                    let t = row[wl] * illuminant[wl];
                    row[wl] = if t.is_nan() { 0.0 } else { t };
                }
            }
        });
        // GEMM 2: out [tpx × 3] = spec [tpx × n_wl] · proj [n_wl × 3]
        dgemm_blas(
            spec,
            proj_flat,
            &mut out[start * 3..(start + tpx) * 3],
            tpx,
            3,
            n_wl,
            Transpose::None,
            3,
        );
        start += tpx;
    }
    out
}

/// Same spectral-integration core as `scan_spectral_cpu`, but stops at
/// `log_xyz = log10(max(xyz / normalization, 0) + 1e-10)` — i.e. it
/// mirrors Python's `cmy_to_log_xyz` *exactly*. Used to build the
/// scanner LUT so the PCHIP-interpolated function is the same one
/// Python interpolates (not the post-`10^x`-CAT-matrix surface, which
/// would diverge sub-LSB from Python's output).
///
/// Returns a flat `n_pix × 3` f64 row-major buffer of log10 XYZ values.
pub fn scan_log_xyz_cpu(
    density_cmy: &ImageBuf,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    illuminant: &[f64],
    normalization: f64,
) -> Vec<f64> {
    let cmfs: Vec<[f64; 3]> = (0..spectral::N_WAVELENGTHS)
        .map(|i| {
            [
                spectral::CMF_X_F64[i],
                spectral::CMF_Y_F64[i],
                spectral::CMF_Z_F64[i],
            ]
        })
        .collect();
    scan_log_xyz_cpu_with_cmfs(
        density_cmy,
        channel_density,
        base_density,
        illuminant,
        &cmfs,
        normalization,
    )
}

/// Runtime-grid variant of [`scan_log_xyz_cpu`]. No wavelength limit is applied
/// beyond the lengths of the supplied channel, illuminant, and CMF arrays.
pub fn scan_log_xyz_cpu_with_cmfs(
    density_cmy: &ImageBuf,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    illuminant: &[f64],
    cmfs: &[[f64; 3]],
    normalization: f64,
) -> Vec<f64> {
    let n_wl = channel_density.len().min(illuminant.len()).min(cmfs.len());
    let has_base = !base_density.is_empty() && base_density.len() >= n_wl;
    let n_pix = (density_cmy.width as usize) * (density_cmy.height as usize);

    let density_f64: Vec<f64> = density_cmy.data.par_iter().map(|&v| to_f64(v)).collect();
    let cd_flat: Vec<f64> = channel_density[..n_wl]
        .iter()
        .flat_map(|r| [r[0], r[1], r[2]])
        .collect();
    let cmf_flat: Vec<f64> = cmfs[..n_wl].iter().flatten().copied().collect();
    let mut xyz_flat = spectral_to_proj_tiled(
        &density_f64,
        &cd_flat,
        base_density,
        has_base,
        illuminant,
        &cmf_flat,
        n_pix,
        n_wl,
    );

    // Final step — exactly Python's `np.log10(np.fmax(xyz, 0.0) + 1e-10)`
    // where `xyz` is the post-normalisation value. No 10^x roundtrip
    // here: the LUT stores log_xyz, the consumer applies 10^x once
    // post-interpolation.
    let inv_norm = 1.0 / normalization;
    xyz_flat.par_iter_mut().for_each(|v| {
        let after_norm = *v * inv_norm;
        *v = (after_norm.max(0.0) + 1e-10).log10();
    });
    xyz_flat
}

/// Render-invariant direct scanner preparation. The generic spectral fallback
/// retains flattened tables, while a proven scalar basis avoids spectral scratch.
pub struct PreparedScanCpu<'a> {
    base_density: &'a [f64],
    illuminant: &'a [f64],
    cd_flat: Vec<f64>,
    cmf_flat: Vec<f64>,
    n_wl: usize,
    has_base: bool,
    scalar: Option<(crate::scalar_scan::ScalarScanCoefficients, [f64; 2])>,
    normalization: f64,
    cat: [[f64; 3]; 3],
    xyz_to_rgb: [f64; 9],
}

impl<'a> PreparedScanCpu<'a> {
    pub fn new(
        channel_density: &[[f64; 3]],
        base_density: &'a [f64],
        illuminant: &'a [f64],
        normalization: f64,
        cat: &[[f64; 3]; 3],
        xyz_to_rgb: &[[f64; 3]; 3],
    ) -> Self {
        let cmfs: [[f64; 3]; spectral::N_WAVELENGTHS] = std::array::from_fn(|i| {
            [
                spectral::CMF_X_F64[i],
                spectral::CMF_Y_F64[i],
                spectral::CMF_Z_F64[i],
            ]
        });
        Self::with_cmfs(
            channel_density,
            base_density,
            illuminant,
            &cmfs,
            normalization,
            cat,
            xyz_to_rgb,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_cmfs(
        channel_density: &[[f64; 3]],
        base_density: &'a [f64],
        illuminant: &'a [f64],
        cmfs: &[[f64; 3]],
        normalization: f64,
        cat: &[[f64; 3]; 3],
        xyz_to_rgb: &[[f64; 3]; 3],
    ) -> Self {
        let n_wl = channel_density.len().min(illuminant.len()).min(cmfs.len());
        Self {
            base_density,
            illuminant,
            cd_flat: channel_density[..n_wl].iter().flatten().copied().collect(),
            cmf_flat: cmfs[..n_wl].iter().flatten().copied().collect(),
            n_wl,
            has_base: !base_density.is_empty() && base_density.len() >= n_wl,
            scalar: crate::scalar_scan::ScalarScanCoefficients::prepare(
                channel_density,
                base_density,
                illuminant,
                cmfs,
            )
            .map(|scalar| {
                // Keep one decade inside the normal f64 exponent domain. Both
                // factored and original per-row powers must stay in this domain;
                // outside it the generic path preserves its operation sequencing.
                let bounds =
                    scalar.exponent_bounds(f64::MIN_POSITIVE.log10() + 1.0, f64::MAX.log10() - 1.0);
                (scalar, bounds)
            }),
            normalization,
            cat: *cat,
            xyz_to_rgb: std::array::from_fn(|i| xyz_to_rgb[i / 3][i % 3]),
        }
    }

    pub fn capture(&self, density_cmy: &ImageBuf) -> ImageBuf {
        let n_pix = density_cmy.width as usize * density_cmy.height as usize;
        if let Some((scalar, bounds)) = self.scalar {
            let mut out = ImageBuf::new(density_cmy.width, density_cmy.height);
            let inv_norm = 1.0 / self.normalization;
            let safe = out
                .data
                .par_chunks_exact_mut(3)
                .zip(density_cmy.data.par_chunks_exact(3))
                .try_for_each(|(dst, src)| {
                    // NaN * 0 is masked by the generic spectral path, not ignored.
                    if !src.iter().all(|&v| to_f64(v).is_finite()) {
                        return Err(());
                    }
                    let exponent = -scalar.coefficient * to_f64(src[0]);
                    if !exponent.is_finite() || exponent < bounds[0] || exponent > bounds[1] {
                        return Err(());
                    }
                    let transmission = 10.0f64.powf(exponent);
                    if !transmission.is_finite() || transmission == 0.0 {
                        return Err(());
                    }
                    let xyz = scalar.xyz.map(|coefficient| coefficient * transmission);
                    if !xyz.iter().all(|value| value.is_finite()) {
                        return Err(());
                    }
                    let adapted = normalize_adapt_xyz(xyz, inv_norm, &self.cat);
                    // Keep CAT and output conversion separate without launching
                    // a threaded BLAS kernel for each small field tile.
                    for (row, value) in dst.iter_mut().enumerate() {
                        let matrix = &self.xyz_to_rgb[row * 3..row * 3 + 3];
                        *value = from_f64(
                            matrix[0] * adapted[0]
                                + matrix[1] * adapted[1]
                                + matrix[2] * adapted[2],
                        );
                    }
                    Ok(())
                });
            if safe.is_ok() {
                return out;
            }
        }
        let density_f64: Vec<f64> = density_cmy.data.par_iter().map(|&v| to_f64(v)).collect();
        let xyz = spectral_to_proj_tiled(
            &density_f64,
            &self.cd_flat,
            self.base_density,
            self.has_base,
            self.illuminant,
            &self.cmf_flat,
            n_pix,
            self.n_wl,
        );
        finish_scan_cpu(
            density_cmy,
            xyz,
            self.normalization,
            &self.cat,
            &self.xyz_to_rgb,
        )
    }
}

/// CPU spectral scanning: density CMY → RGB via 81-wavelength spectral integration.
/// All inputs are f64 for Python parity. Python: `density_to_light` → einsum CMFs / norm.
/// CPU spectral scanning: print/film density → final RGB via spectral
/// integration. BLAS-backed GEMMs for numpy parity (same Accelerate
/// BLAS that numpy uses on macOS).
///
/// Algorithm:
///   1. density_spectral = density_cmy @ channel_density.T + base_density
///   2. light             = 10^(-density_spectral) * illuminant   (NaN → 0)
///   3. xyz               = light @ CMF                            (CMF = [n_wl × 3] of X,Y,Z)
///   4. rgb               = (xyz / normalization) @ xyz_to_rgb.T
pub fn scan_spectral_cpu(
    density_cmy: &ImageBuf,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    illuminant: &[f64],
    normalization: f64,
    cat: &[[f64; 3]; 3],
    xyz_to_rgb: &[[f64; 3]; 3],
) -> ImageBuf {
    let cmfs: Vec<[f64; 3]> = (0..spectral::N_WAVELENGTHS)
        .map(|i| {
            [
                spectral::CMF_X_F64[i],
                spectral::CMF_Y_F64[i],
                spectral::CMF_Z_F64[i],
            ]
        })
        .collect();
    scan_spectral_cpu_with_cmfs(
        density_cmy,
        channel_density,
        base_density,
        illuminant,
        &cmfs,
        normalization,
        cat,
        xyz_to_rgb,
    )
}

/// Runtime-grid variant of [`scan_spectral_cpu`]. CMFs are supplied per
/// wavelength and therefore scanning is not truncated to the historical grid.
#[allow(clippy::too_many_arguments)]
pub fn scan_spectral_cpu_with_cmfs(
    density_cmy: &ImageBuf,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    illuminant: &[f64],
    cmfs: &[[f64; 3]],
    normalization: f64,
    cat: &[[f64; 3]; 3],
    xyz_to_rgb: &[[f64; 3]; 3],
) -> ImageBuf {
    let n_wl = channel_density.len().min(illuminant.len()).min(cmfs.len());
    let has_base = !base_density.is_empty() && base_density.len() >= n_wl;
    let n_pix = (density_cmy.width as usize) * (density_cmy.height as usize);

    // ── density_spectral = density_cmy @ channel_density.T + base_density
    // Promote density_cmy to f64 directly — never round-trip through
    // f32, even with Scalar=f64 (`to_f32(v) as f64` quietly narrows).
    // The `as f64` conversion is per-element and embarrassingly parallel;
    // the prior serial collect ran ~60M conversions on one core.
    // ── xyz = (10^(-(density·channel_densityᵀ + base)) · illuminant) @ CMF.
    // Use f64 CMF constants — the f32 ones drop ~7 digits per sample and
    // accumulate ~5e-6 of drift after the 81-wavelength reduction. The
    // spectral integration is tiled (see `spectral_to_proj_tiled`) so the
    // intermediate stays bounded at high resolution.
    let density_f64: Vec<f64> = density_cmy.data.par_iter().map(|&v| to_f64(v)).collect();
    let cd_flat: Vec<f64> = channel_density[..n_wl]
        .iter()
        .flat_map(|r| [r[0], r[1], r[2]])
        .collect();
    let cmf_flat: Vec<f64> = cmfs[..n_wl].iter().flatten().copied().collect();
    let xyz_flat = spectral_to_proj_tiled(
        &density_f64,
        &cd_flat,
        base_density,
        has_base,
        illuminant,
        &cmf_flat,
        n_pix,
        n_wl,
    );

    let matrix = std::array::from_fn(|i| xyz_to_rgb[i / 3][i % 3]);
    finish_scan_cpu(density_cmy, xyz_flat, normalization, cat, &matrix)
}

fn normalize_adapt_xyz(xyz: [f64; 3], inv_norm: f64, cat: &[[f64; 3]; 3]) -> [f64; 3] {
    let normalized = xyz.map(|value| {
        let log_xyz = ((value * inv_norm).max(0.0) + 1e-10).log10();
        10.0f64.powf(log_xyz)
    });
    std::array::from_fn(|row| {
        cat[row][0] * normalized[0] + cat[row][1] * normalized[1] + cat[row][2] * normalized[2]
    })
}

fn finish_scan_cpu(
    density_cmy: &ImageBuf,
    mut xyz_flat: Vec<f64>,
    normalization: f64,
    cat: &[[f64; 3]; 3],
    xyz_to_rgb_flat: &[f64; 9],
) -> ImageBuf {
    let n_pix = (density_cmy.width as usize) * (density_cmy.height as usize);
    // Python's `_density_to_rgb` does:
    //   log_xyz = log10(max(xyz/norm, 0) + 1e-10)
    //   xyz_again = 10**log_xyz
    //   rgb = colour.XYZ_to_RGB(xyz_again, ...)
    // The log/exp roundtrip introduces a small but reproducible
    // precision noise — `10**log10(x + 1e-10)` ≠ `x` after f64
    // arithmetic. We replicate it bit-for-bit so the residual error
    // at the final stage matches numpy.
    // Fuse the normalization log/exp roundtrip with the per-pixel CAT
    // multiply. xyz_flat is interleaved [X,Y,Z]; one parallel chunk
    // per pixel handles all three components, replacing two serial
    // passes (3·n_pix scalar log10+powf, then 9·n_pix muls).
    let inv_norm = 1.0 / normalization;
    xyz_flat.par_chunks_exact_mut(3).for_each(|px| {
        let adapted = normalize_adapt_xyz([px[0], px[1], px[2]], inv_norm, cat);
        px.copy_from_slice(&adapted);
    });

    // ── rgb = xyz_adapted @ xyz_to_rgb.T
    // The prepared matrix is row-major, as in the generic reference.
    let mut rgb_flat = vec![0.0f64; n_pix * 3];
    dgemm_blas(
        &xyz_flat,
        xyz_to_rgb_flat,
        &mut rgb_flat,
        n_pix,
        3,
        3,
        Transpose::Ordinary,
        3,
    );

    let mut rgb = ImageBuf::new(density_cmy.width, density_cmy.height);
    rgb.data
        .par_iter_mut()
        .zip(rgb_flat.par_iter())
        .for_each(|(dst, &src)| *dst = from_f64(src));
    rgb
}

/// CPU spectral printing: film density → print log-exposure via spectral integration.
/// All inputs are f64 for Python parity. Uses BLAS GEMM for the two
/// reductions, matching numpy's einsum bit-for-bit on macOS Accelerate.
///
/// Algorithm (mirrors `spektrafilm/model/emulsion.py:compute_density_spectral`
///   + `spektrafilm/utils/conversions.py:density_to_light` + the
///     `einsum("ijk,kl->ijl", light, sensitivity)` in `printing.py`):
///   1. density_spectral = density_cmy @ channel_density.T + base_density
///   2. light             = 10^(-density_spectral) * illuminant   (NaN → 0)
///   3. raw               = light @ sensitivity
///   4. output            = log10(max(raw * normalization, 0) + 1e-10)
pub fn print_spectral_cpu(
    density_cmy: &ImageBuf,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    illuminant: &[f64],
    sensitivity: &[[f64; 3]],
    normalization_factor: f64,
    preflash: [f64; 3],
) -> ImageBuf {
    let n_wl = channel_density
        .len()
        .min(illuminant.len())
        .min(sensitivity.len());
    let has_base = !base_density.is_empty() && base_density.len() >= n_wl;
    let n_pix = (density_cmy.width as usize) * (density_cmy.height as usize);

    // density_cmy stored as Scalar (interleaved RGB). Promote to f64
    // directly — `to_f32(v) as f64` quietly round-trips through f32
    // even with Scalar=f64 and was the seed of ~1e-9 print drift.
    let density_f64: Vec<f64> = density_cmy.data.par_iter().map(|&v| to_f64(v)).collect();

    // channel_density: Vec<[f64; 3]> is already row-major [n_wl × 3].
    // For the GEMM density_cmy @ channel_density.T we pass TransB so BLAS
    // transposes channel_density on the fly.
    let cd_flat: Vec<f64> = channel_density[..n_wl]
        .iter()
        .flat_map(|r| [r[0], r[1], r[2]])
        .collect();

    // raw [n_pix × 3] = (10^(-(density·channel_densityᵀ + base)) · illuminant) @ sensitivity.
    // Tiled over pixels so the [tile × n_wl] spectral intermediate stays bounded
    // at high resolution (see `spectral_to_proj_tiled`).
    let sens_flat: Vec<f64> = sensitivity[..n_wl]
        .iter()
        .flat_map(|r| [r[0], r[1], r[2]])
        .collect();
    let raw_flat = spectral_to_proj_tiled(
        &density_f64,
        &cd_flat,
        base_density,
        has_base,
        illuminant,
        &sens_flat,
        n_pix,
        n_wl,
    );

    // Apply normalization, add the constant preflash exposure, then
    // log10(max(., 0) + 1e-10), in parallel. Preflash is a per-channel
    // constant broadcast over every pixel (Python `raw += preflash`); adding
    // it here — before the inner log10 — bakes it into LUT nodes when this
    // path generates a print LUT, matching upstream `_film_cmy_to_print_log_raw`.
    let mut output = ImageBuf::new(density_cmy.width, density_cmy.height);
    output
        .data
        .par_chunks_exact_mut(3)
        .zip(raw_flat.par_chunks_exact(3))
        .for_each(|(dst, src)| {
            for c in 0..3 {
                let v = src[c] * normalization_factor + preflash[c];
                dst[c] = from_f64((v.max(0.0) + 1e-10).log10());
            }
        });
    output
}
