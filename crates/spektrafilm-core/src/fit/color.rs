//! Patch colorimetry for the fit objective: linear sRGB to CIELAB, CIEDE2000
//! differences, and median aggregation.

use serde::Serialize;
use spektrafilm_math::spectral::colorspace_to_xyz_f64;

/// Convert a linear sRGB triple to CIELAB. Negative values clip at zero, as an
/// out-of-gamut scan cannot carry them into the reference.
pub fn linear_srgb_to_lab(rgb: [f64; 3]) -> [f64; 3] {
    let matrix = colorspace_to_xyz_f64("sRGB");
    let rgb = rgb.map(|value| value.max(0.0));
    let mut xyz = [0.0f64; 3];
    for (row, xyz_value) in xyz.iter_mut().enumerate() {
        *xyz_value = (0..3)
            .map(|channel| matrix[row][channel] * rgb[channel])
            .sum();
    }
    crate::lut_qa::xyz_to_lab(xyz)
}

/// CIEDE2000 between two CIELAB triples.
pub fn delta_e(a: [f64; 3], b: [f64; 3]) -> f64 {
    crate::lut_qa::delta_e_2000(a, b)
}

/// Per-patch CIELAB differences of a fit result against the reference patches.
#[derive(Debug, Clone, Serialize)]
pub struct PatchDifferences {
    pub de: Vec<f64>,
    pub dl: Vec<f64>,
    pub dc: Vec<f64>,
    pub dh: Vec<f64>,
    pub reference_l: Vec<f64>,
    pub reference_c: Vec<f64>,
    pub reference_h: Vec<f64>,
}

/// Build the per-patch differences of `output` against `reference`.
pub fn differences(output: &[[f64; 3]], reference: &[[f64; 3]]) -> PatchDifferences {
    let mut out = PatchDifferences {
        de: Vec::with_capacity(output.len()),
        dl: Vec::with_capacity(output.len()),
        dc: Vec::with_capacity(output.len()),
        dh: Vec::with_capacity(output.len()),
        reference_l: Vec::with_capacity(output.len()),
        reference_c: Vec::with_capacity(output.len()),
        reference_h: Vec::with_capacity(output.len()),
    };
    for (a, b) in output.iter().zip(reference) {
        let chroma_a = a[1].hypot(a[2]);
        let chroma_b = b[1].hypot(b[2]);
        let hue_a = a[2].atan2(a[1]).to_degrees();
        let hue_b = b[2].atan2(b[1]).to_degrees();
        out.de.push(delta_e(*a, *b));
        out.dl.push(a[0] - b[0]);
        out.dc.push(chroma_a - chroma_b);
        out.dh.push((hue_a - hue_b + 540.0) % 360.0 - 180.0);
        out.reference_l.push(b[0]);
        out.reference_c.push(chroma_b);
        out.reference_h.push(hue_b);
    }
    out
}

/// Median with the mean of the two middle values for an even count.
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("fit metrics are finite"));
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: [f64; 3], want: [f64; 3]) {
        for channel in 0..3 {
            assert!(
                (got[channel] - want[channel]).abs() < 1e-6,
                "{got:?} vs {want:?}"
            );
        }
    }

    #[test]
    fn linear_srgb_to_lab_matches_the_srgb_reference() {
        close(
            linear_srgb_to_lab([1.0, 1.0, 1.0]),
            [100.0, 0.00772827, 0.00353528],
        );
        close(
            linear_srgb_to_lab([0.18, 0.18, 0.18]),
            [49.49610761, 0.00436355, 0.00199609],
        );
        close(
            linear_srgb_to_lab([0.7, 0.2, 0.1]),
            [61.57476106, 33.40395263, 34.67194027],
        );
    }

    #[test]
    fn negative_output_clips_like_the_reference() {
        assert_eq!(
            linear_srgb_to_lab([-0.2, -0.2, -0.2]),
            linear_srgb_to_lab([0.0, 0.0, 0.0])
        );
    }

    #[test]
    fn differences_report_hue_and_chroma_against_the_reference() {
        let differences = differences(&[[52.0, 0.0, 20.0]], &[[50.0, 20.0, 0.0]]);
        assert!((differences.dl[0] - 2.0).abs() < 1e-9);
        assert!(differences.dc[0].abs() < 1e-9);
        assert!((differences.dh[0] - 90.0).abs() < 1e-9);
        assert!((differences.reference_c[0] - 20.0).abs() < 1e-9);
        assert!(differences.de[0] > 0.0);
    }

    #[test]
    fn median_averages_the_two_middle_values() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
    }
}
