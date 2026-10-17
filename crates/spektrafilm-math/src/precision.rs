//! Precision type alias for the pixel pipeline.
//!
//! Default: f32 for GPU speed and memory efficiency.
//! With `--features precision-f64`: f64 for 1:1 parity with Python.
//!
//! The calibration chain (Hanatos2025 tc/b, CAT02 adaptation, exposure factor)
//! always uses f64 regardless of this flag.
//!
//! Also hosts the whole-array vectorised transcendentals ([`exp10_inplace`])
//! used by the film and print density paths.

#[cfg(feature = "precision-f64")]
pub type Scalar = f64;
#[cfg(not(feature = "precision-f64"))]
pub type Scalar = f32;

/// Convert f64 to Scalar (no-op in f64 mode, truncation in f32 mode).
#[inline(always)]
pub fn from_f64(v: f64) -> Scalar {
    v as Scalar
}

/// Convert Scalar to f64 (no-op in f64 mode, promotion in f32 mode).
///
/// The cast is redundant with `--features precision-f64` (`Scalar = f64`) and
/// required with the default `Scalar = f32`, so it must stay.
#[inline(always)]
#[allow(clippy::unnecessary_cast)]
pub fn to_f64(v: Scalar) -> f64 {
    v as f64
}

/// Convert f32 to Scalar.
#[inline(always)]
pub fn from_f32(v: f32) -> Scalar {
    v as Scalar
}

/// Convert Scalar to f32.
///
/// The cast is redundant with the default `Scalar = f32` and required with
/// `--features precision-f64` (`Scalar = f64`), so it must stay.
#[inline(always)]
#[allow(clippy::unnecessary_cast)]
pub fn to_f32(v: Scalar) -> f32 {
    v as f32
}

/// Zero constant.
pub const ZERO: Scalar = 0.0;
/// One constant.
pub const ONE: Scalar = 1.0;

/// Scalar-aware pow(10, x).
#[inline(always)]
pub fn pow10(x: Scalar) -> Scalar {
    from_f64(10.0).powf(x)
}

/// Scalar-aware log10.
#[inline(always)]
pub fn log10(x: Scalar) -> Scalar {
    x.log10()
}

/// Scalar-aware powf.
#[inline(always)]
pub fn powf(base: Scalar, exp: Scalar) -> Scalar {
    base.powf(exp)
}

/// sRGB OETF (linear → gamma) for Scalar.
#[inline(always)]
pub fn srgb_encode(x: Scalar) -> Scalar {
    let threshold = from_f64(0.0031308);
    if x <= threshold {
        from_f64(12.92) * x
    } else {
        from_f64(1.055) * powf(x, from_f64(1.0 / 2.4)) - from_f64(0.055)
    }
}

/// sRGB EOTF (gamma → linear) for Scalar.
#[inline(always)]
pub fn srgb_decode(x: Scalar) -> Scalar {
    let threshold = from_f64(0.04045);
    if x <= threshold {
        x / from_f64(12.92)
    } else {
        powf((x + from_f64(0.055)) / from_f64(1.055), from_f64(2.4))
    }
}

// ── Vectorised whole-array transcendentals ────────────────────────────────
//
// SIMD-vectorised whole-array transcendentals via Apple's vForce (part of
// the Accelerate framework). vForce is what numpy reaches for under the
// hood; calling it directly lets us match numpy's per-element wall time for
// `10^x`, which would otherwise be 5–10× slower as scalar `f64::powf` calls
// from Rust.
//
// `exp10` is implemented via `vvpow(y, exp, base, &n)` with a reusable
// buffer of 10.0 in `base`. vvpow calls libm `pow` per lane, so the result
// is bit-identical to `10.0f64.powf(x)` (we verified empirically —
// substituting `exp(x · ln 10)` regressed parity from "5 pixels out of 1.5M"
// to ~7 % off-by->2). The `base` allocation per call (≤ chunk size, capped
// at 64 K f64s = 512 KB) is negligible vs the vvpow throughput.
//
// On non-Apple targets the fallbacks loop over the slice scalar-style so
// callers can use these unconditionally.

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    /// `y[i] = base[i] ^ exp[i]`. Note the order: exponent first, base
    /// second — Apple's vForce convention, not the math one.
    fn vvpow(y: *mut f64, exp: *const f64, base: *const f64, n: *const i32);
}

/// `out[i] = 10^x[i]` for i in 0..len, in place. Bit-identical to
/// libm `pow(10, x)`.
#[inline]
pub fn exp10_inplace(buf: &mut [f64]) {
    #[cfg(target_vendor = "apple")]
    {
        // Process in cache-friendly chunks. 64 K f64s is 512 KB —
        // fits in L2 on every Apple Silicon chip and lets the base
        // buffer stay tiny vs the working set.
        const CHUNK: usize = 1 << 16;
        let base = vec![10.0f64; CHUNK.min(buf.len()).max(1)];
        for slice in buf.chunks_mut(CHUNK) {
            let n = slice.len() as i32;
            unsafe { vvpow(slice.as_mut_ptr(), slice.as_ptr(), base.as_ptr(), &n) };
        }
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        for v in buf.iter_mut() {
            *v = 10.0f64.powf(*v);
        }
    }
}
