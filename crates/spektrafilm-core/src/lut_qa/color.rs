//! Canonical color-primary matrices used by LUT QA metrics.
//!
//! QA works in f64 and consumes the same linear RGB ↔ XYZ matrices as the
//! runtime transport registry, so the matrices are projected from
//! `spektrafilm_math::colorspace::resolve` instead of being duplicated here.
//! Only the whitepoint and primary chromaticities — which the math registry
//! does not carry — stay QA-local.
use crate::lut_transport::ColorSpaceEntry;
use spektrafilm_math::colorspace::RgbColorSpace;

#[derive(Clone, Copy)]
pub(super) struct Color {
    pub(super) m: [[f64; 3]; 3],
    pub(super) inv: [[f64; 3]; 3],
    pub(super) white: [f64; 2],
    pub(super) pri: [[f64; 2]; 3],
}

/// QA-only chromaticities keyed by the pinned primaries name: the whitepoint
/// and the three primary chromaticities that the gamut and adaptation checks
/// read. Matrices are taken from the shared registry.
fn chromaticities(primaries: &str) -> ([f64; 2], [[f64; 2]; 3]) {
    match primaries {
        "sRGB" => ([0.3127, 0.329], [[0.64, 0.33], [0.3, 0.6], [0.15, 0.06]]),
        "ITU-R BT.709" => ([0.3127, 0.329], [[0.64, 0.33], [0.3, 0.6], [0.15, 0.06]]),
        "ITU-R BT.2020" => (
            [0.3127, 0.329],
            [[0.708, 0.292], [0.17, 0.797], [0.131, 0.046]],
        ),
        "Display P3" => ([0.3127, 0.329], [[0.68, 0.32], [0.265, 0.69], [0.15, 0.06]]),
        "DCI-P3" => ([0.314, 0.351], [[0.68, 0.32], [0.265, 0.69], [0.15, 0.06]]),
        "Adobe RGB (1998)" => ([0.3127, 0.329], [[0.64, 0.33], [0.21, 0.71], [0.15, 0.06]]),
        "ACEScct" => (
            [0.32168, 0.33767],
            [[0.713, 0.293], [0.165, 0.83], [0.128, 0.044]],
        ),
        "ARRI Wide Gamut 3" => (
            [0.3127, 0.329],
            [[0.684, 0.313], [0.221, 0.848], [0.0861, -0.102]],
        ),
        "ARRI Wide Gamut 4" => (
            [0.3127, 0.329],
            [[0.7347, 0.2653], [0.1424, 0.8576], [0.0991, -0.0308]],
        ),
        "S-Gamut3" => ([0.3127, 0.329], [[0.73, 0.28], [0.14, 0.855], [0.1, -0.05]]),
        "S-Gamut3.Cine" => (
            [0.3127, 0.329],
            [[0.766, 0.275], [0.225, 0.8], [0.089, -0.087]],
        ),
        "V-Gamut" => ([0.3127, 0.329], [[0.73, 0.28], [0.165, 0.84], [0.1, -0.03]]),
        "F-Gamut" => (
            [0.3127, 0.329],
            [[0.708, 0.292], [0.17, 0.797], [0.131, 0.046]],
        ),
        "ProPhoto RGB" => (
            [0.3457, 0.3585],
            [[0.7347, 0.2653], [0.1596, 0.8404], [0.0366, 0.0001]],
        ),
        "ACEScg" => (
            [0.32168, 0.33767],
            [[0.713, 0.293], [0.165, 0.83], [0.128, 0.044]],
        ),
        "ACES2065-1" => (
            [0.32168, 0.33767],
            [[0.7347, 0.2653], [0.0, 1.0], [0.0001, -0.077]],
        ),
        "Cinema Gamut" => ([0.3127, 0.329], [[0.74, 0.27], [0.17, 1.14], [0.08, -0.1]]),
        "REDWideGamutRGB" => (
            [0.3127, 0.329],
            [
                [0.780308, 0.304253],
                [0.121595, 1.493994],
                [0.095612, -0.084589],
            ],
        ),
        "DaVinci Wide Gamut" => (
            [0.3127, 0.329],
            [[0.8, 0.313], [0.1682, 0.9877], [0.079, -0.1155]],
        ),
        "Blackmagic Wide Gamut" => (
            [0.312717, 0.3290312],
            [
                [0.7177215, 0.3171181],
                [0.228041, 0.861569],
                [0.1005841, -0.0820452],
            ],
        ),
        "P3-D65" => ([0.3127, 0.329], [[0.68, 0.32], [0.265, 0.69], [0.15, 0.06]]),
        "DJI D-Gamut" => ([0.3127, 0.329], [[0.71, 0.31], [0.21, 0.88], [0.09, -0.08]]),
        other => unreachable!("unregistered QA primaries: {other}"),
    }
}

fn resolved(cs: &ColorSpaceEntry) -> &'static RgbColorSpace {
    spektrafilm_math::colorspace::resolve(cs.primaries)
        .unwrap_or_else(|_| panic!("unregistered QA primaries: {}", cs.primaries))
}

pub(super) fn color(cs: &ColorSpaceEntry) -> Color {
    let space = resolved(cs);
    let (white, pri) = chromaticities(cs.primaries);
    Color {
        m: space.matrix_rgb_to_xyz,
        inv: space.matrix_xyz_to_rgb,
        white,
        pri,
    }
}
