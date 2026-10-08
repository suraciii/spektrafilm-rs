//! Backend-neutral preparation shared by the per-stage CPU path and the
//! GPU-resident chain builder.
//!
//! Each helper here is the single authority for a derivation that both
//! execution routes need (scan colour context, glare offsets). The math is
//! lifted verbatim from the per-stage scanning stage — including its f64
//! operation order — so the CPU reference path is bit-identical before and
//! after this module existed; the GPU-resident path consumes the same values
//! and narrows to f32 at its own boundary.

use spektrafilm_math::colorspace;
use spektrafilm_math::spectral::{self, N_WAVELENGTHS};

/// Colour context of one scan pass: viewing illuminant, its normalization,
/// the derived chromatic-adaptation matrix, and the output XYZ→RGB matrix.
#[derive(Debug, Clone)]
pub struct ScanColorContext {
    /// Resolved viewing (or scan-override) illuminant, f64 per wavelength.
    pub illuminant: Vec<f64>,
    /// Wavelength count shared by the illuminant, the profile's dye
    /// densities, and the historical 81-sample grid.
    pub n_wl: usize,
    /// `Σ illu·CMF_Y` — Python parity (81-element sum in f64).
    pub normalization: f64,
    /// Illuminant XYZ integrated against the CMFs and divided by
    /// `normalization` (Y = 1).
    pub illu_xyz: [f64; 3],
    /// Viewing white derived through the XYZ→xy→XYZ roundtrip that
    /// renormalizes Y to exactly 1 (matches Python's contract chain).
    pub viewing_white: [f64; 3],
    /// Chromatic adaptation from the viewing white to the output white.
    pub adapt: [[f64; 3]; 3],
    /// Output colour space XYZ→RGB matrix.
    pub base_xyz_to_rgb: [[f64; 3]; 3],
}

impl ScanColorContext {
    /// Build the context for one scan pass. `illuminant` is already resolved
    /// (the per-stage path may pass a scan override; the resident chain
    /// passes the profile's viewing illuminant). `channel_density_len` is
    /// the scanned profile's per-wavelength dye-density row count.
    pub fn build(
        illuminant: Vec<f64>,
        channel_density_len: usize,
        output_color_space: &str,
    ) -> Self {
        let n_wl = illuminant
            .len()
            .min(channel_density_len)
            .min(N_WAVELENGTHS);

        // Python parity — the normalization sums exactly n_wl terms in f64.
        let normalization: f64 = (0..n_wl)
            .map(|i| illuminant[i] * spectral::CMF_Y_F64[i])
            .sum();

        // Illuminant XYZ (Y normalized to 1) — same runtime derivation as
        // Python's integrate/contract chain, including the division by
        // `normalization` before the xy roundtrip below.
        let mut illu_xyz = [0.0f64; 3];
        for i in 0..n_wl {
            illu_xyz[0] += illuminant[i] * spectral::CMF_X_F64[i];
            illu_xyz[1] += illuminant[i] * spectral::CMF_Y_F64[i];
            illu_xyz[2] += illuminant[i] * spectral::CMF_Z_F64[i];
        }
        for c in 0..3 {
            illu_xyz[c] /= normalization;
        }

        // XYZ_to_xy then xy_to_xyz roundtrip (renormalizes Y to exactly 1).
        let sum_xyz = illu_xyz[0] + illu_xyz[1] + illu_xyz[2];
        let vx = illu_xyz[0] / sum_xyz;
        let vy = illu_xyz[1] / sum_xyz;
        let viewing_white = [vx / vy, 1.0f64, (1.0 - vx - vy) / vy];

        let output_space = colorspace::resolve(output_color_space)
            .expect("output color space must be validated before scanning");
        let adapt =
            colorspace::chromatic_adaptation_matrix_f64(viewing_white, output_space.whitepoint_xyz());

        Self {
            illuminant,
            n_wl,
            normalization,
            illu_xyz,
            viewing_white,
            adapt,
            base_xyz_to_rgb: output_space.matrix_xyz_to_rgb,
        }
    }
}

/// Per-channel RGB glare offset `M · illuminant_xyz`, applied as
/// `rgb += g * offset / 100`. The two matrix applications follow the
/// per-stage scan order — CAT first, then the output XYZ→RGB matrix —
/// keeping the reference rounding sequence authoritative.
pub fn glare_rgb_offset_f64(ctx: &ScanColorContext) -> [f64; 3] {
    let a = ctx.adapt;
    let m = ctx.base_xyz_to_rgb;
    let xyz = ctx.illu_xyz;
    let xyz_adapt = [
        a[0][0] * xyz[0] + a[0][1] * xyz[1] + a[0][2] * xyz[2],
        a[1][0] * xyz[0] + a[1][1] * xyz[1] + a[1][2] * xyz[2],
        a[2][0] * xyz[0] + a[2][1] * xyz[1] + a[2][2] * xyz[2],
    ];
    [
        m[0][0] * xyz_adapt[0] + m[0][1] * xyz_adapt[1] + m[0][2] * xyz_adapt[2],
        m[1][0] * xyz_adapt[0] + m[1][1] * xyz_adapt[1] + m[1][2] * xyz_adapt[2],
        m[2][0] * xyz_adapt[0] + m[2][1] * xyz_adapt[1] + m[2][2] * xyz_adapt[2],
    ]
}
