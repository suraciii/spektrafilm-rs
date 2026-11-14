//! Scan-stage appearance profiles and their built-in catalog.
//!
//! The profile owns a resolved transform. It is not a user profile library:
//! custom edits are captured by look or section presets.
use serde::{Deserialize, Serialize};

use super::validation::{ValidationError, ValidationReason};

/// Maximum number of curve knots accepted by the v1 transform.
pub const MAX_SCAN_PROFILE_KNOTS: usize = 9;

/// Minimum input spacing the v1 transform accepts between adjacent knots.
/// The editor uses the same value, so an accepted curve is always editable.
pub const MIN_SCAN_PROFILE_KNOT_GAP: f32 = 1.0 / 1024.0;

/// One channel of a piecewise-linear scan-stage curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanCurve {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
}

impl ScanCurve {
    pub fn new(x: Vec<f32>, y: Vec<f32>) -> Self {
        Self { x, y }
    }

    fn from_slices(x: &[f32], y: &[f32]) -> Self {
        Self {
            x: x.to_vec(),
            y: y.to_vec(),
        }
    }
}

/// The resolved Scan Profile transform. `matrix_curve` is built-in/read-only;
/// `bypass` is the absence of a transform. The `kind` field discriminates the
/// active variant; variant-specific fields are optional and validated together.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanProfileTransform {
    pub kind: ScanTransformKind,
    #[serde(default)]
    pub curves: Option<[ScanCurve; 3]>,
    #[serde(default)]
    pub matrix: Option<[[f32; 3]; 3]>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanTransformKind {
    #[default]
    Bypass,
    Curve,
    MatrixCurve,
}

impl ScanProfileTransform {
    pub fn bypass() -> Self {
        Self::default()
    }

    pub fn curve(curves: [ScanCurve; 3]) -> Self {
        Self {
            kind: ScanTransformKind::Curve,
            curves: Some(curves),
            matrix: None,
        }
    }

    pub fn matrix_curve(matrix: [[f32; 3]; 3], curves: [ScanCurve; 3]) -> Self {
        Self {
            kind: ScanTransformKind::MatrixCurve,
            curves: Some(curves),
            matrix: Some(matrix),
        }
    }

    pub fn is_bypass(&self) -> bool {
        self.kind == ScanTransformKind::Bypass
    }
}

/// Look-owned Scan Profile controls. The transform is authoritative; the
/// built-in name is derived by exact comparison, never stored as a reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanProfileParams {
    pub strength: f32,
    pub transform: ScanProfileTransform,
}

impl Default for ScanProfileParams {
    fn default() -> Self {
        Self {
            strength: 0.0,
            transform: ScanProfileTransform::bypass(),
        }
    }
}

impl ScanProfileParams {
    pub fn is_bypass(&self) -> bool {
        self.transform.is_bypass()
    }

    pub fn is_effective(&self) -> bool {
        !self.is_bypass() && self.strength > 0.0
    }

    pub fn effective_name(&self) -> ScanProfileName {
        if self.is_bypass() {
            return ScanProfileName::None;
        }
        if self.transform == *BUILTIN_SP3000_TRANSFORM {
            ScanProfileName::Sp3000Inspired
        } else {
            ScanProfileName::Custom
        }
    }

    pub fn set_bypass(&mut self) {
        *self = Self::default();
    }

    pub fn set_builtin(&mut self, name: ScanProfileName) -> bool {
        match name {
            ScanProfileName::None => {
                self.set_bypass();
                true
            }
            ScanProfileName::Sp3000Inspired => {
                *self = builtin_sp3000_inspired();
                true
            }
            ScanProfileName::Custom => false,
        }
    }

    pub(crate) fn validate_detailed(&self) -> Result<(), ValidationError> {
        if !self.strength.is_finite() || !(0.0..=1.0).contains(&self.strength) {
            return Err(ValidationError::new(
                ValidationReason::NumericRange,
                "scanner.scan_profile.strength",
                "scanner.scan_profile.strength: value must be finite and within 0..=1",
            ));
        }
        match self.transform.kind {
            ScanTransformKind::Bypass => {
                if self.transform.curves.is_some() || self.transform.matrix.is_some() {
                    return Err(ValidationError::new(
                        ValidationReason::UnsupportedValue,
                        "scanner.scan_profile.transform",
                        "scanner.scan_profile.transform: bypass carries no curves or matrix",
                    ));
                }
                Ok(())
            }
            ScanTransformKind::Curve => {
                if self.transform.matrix.is_some() {
                    return Err(ValidationError::new(
                        ValidationReason::UnsupportedValue,
                        "scanner.scan_profile.transform.matrix",
                        "scanner.scan_profile.transform: curve carries no matrix",
                    ));
                }
                let curves = self.transform.curves.as_ref().ok_or_else(|| {
                    ValidationError::new(
                        ValidationReason::LeafRequired,
                        "scanner.scan_profile.transform.curves",
                        "scanner.scan_profile.transform: curve requires curves",
                    )
                })?;
                validate_curves(curves)
            }
            ScanTransformKind::MatrixCurve => {
                let matrix = self.transform.matrix.as_ref().ok_or_else(|| {
                    ValidationError::new(
                        ValidationReason::LeafRequired,
                        "scanner.scan_profile.transform.matrix",
                        "scanner.scan_profile.transform: matrix_curve requires matrix",
                    )
                })?;
                if matrix.iter().flatten().any(|value| !value.is_finite()) {
                    return Err(ValidationError::new(
                        ValidationReason::Finite,
                        "scanner.scan_profile.transform.matrix",
                        "scanner.scan_profile.transform.matrix: values must be finite",
                    ));
                }
                let curves = self.transform.curves.as_ref().ok_or_else(|| {
                    ValidationError::new(
                        ValidationReason::LeafRequired,
                        "scanner.scan_profile.transform.curves",
                        "scanner.scan_profile.transform: matrix_curve requires curves",
                    )
                })?;
                validate_curves(curves)
            }
        }
    }
}

fn validate_curves(curves: &[ScanCurve; 3]) -> Result<(), ValidationError> {
    for (channel, curve) in curves.iter().enumerate() {
        let x_path = format!("scanner.scan_profile.transform.curves.{channel}.x");
        let y_path = format!("scanner.scan_profile.transform.curves.{channel}.y");
        if curve.x.len() != curve.y.len()
            || curve.x.len() < 2
            || curve.x.len() > MAX_SCAN_PROFILE_KNOTS
        {
            return Err(ValidationError::new(
                ValidationReason::NumericRange,
                x_path,
                "scanner.scan_profile.transform.curves: x and y must have equal lengths of 2..=9",
            ));
        }
        if curve.x.first() != Some(&0.0) || curve.x.last() != Some(&1.0) {
            return Err(ValidationError::new(
                ValidationReason::NumericRange,
                x_path,
                "scanner.scan_profile.transform.curves: x must start at 0 and end at 1",
            ));
        }
        if curve
            .x
            .windows(2)
            .any(|pair| pair[1] - pair[0] < MIN_SCAN_PROFILE_KNOT_GAP)
        {
            return Err(ValidationError::new(
                ValidationReason::NumericRange,
                x_path,
                "scanner.scan_profile.transform.curves: x must increase by at least 1/1024",
            ));
        }
        if curve.y.iter().any(|value| !value.is_finite()) {
            return Err(ValidationError::new(
                ValidationReason::Finite,
                y_path,
                "scanner.scan_profile.transform.curves: y must be finite",
            ));
        }
        if curve.y.windows(2).any(|pair| pair[1] < pair[0]) {
            return Err(ValidationError::new(
                ValidationReason::NumericRange,
                y_path,
                "scanner.scan_profile.transform.curves: y must be monotone",
            ));
        }
        if curve.y.iter().any(|value| !(0.0..=1.0).contains(value)) {
            return Err(ValidationError::new(
                ValidationReason::NumericRange,
                y_path,
                "scanner.scan_profile.transform.curves: y must stay within 0..=1",
            ));
        }
    }
    Ok(())
}

/// Stable catalog identifiers exposed to the selector and CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanProfileName {
    None,
    Sp3000Inspired,
    Custom,
}

impl ScanProfileName {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sp3000Inspired => "sp3000_inspired",
            Self::Custom => "custom",
        }
    }

    pub fn from_builtin_name(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "sp3000_inspired" => Some(Self::Sp3000Inspired),
            _ => None,
        }
    }
}

/// Cached transform for exact comparison in `effective_name`, avoiding an
/// allocation on every GUI frame.
static BUILTIN_SP3000_TRANSFORM: std::sync::LazyLock<ScanProfileTransform> =
    std::sync::LazyLock::new(|| builtin_sp3000_inspired().transform);

/// The built-in SP-3000-inspired curve. Knots are stored per channel and
/// include the domain endpoints; outside-domain values clamp to those endpoints.
pub fn builtin_sp3000_inspired() -> ScanProfileParams {
    const R_X: [f32; 9] = [
        0.0,
        0.004_776_953,
        0.019_382_361,
        0.287_440_9,
        0.462_077_05,
        0.630_757_15,
        0.775_822_34,
        0.930_110_93,
        1.0,
    ];
    const R_Y: [f32; 9] = [
        0.004_391_442,
        0.004_391_442,
        0.009_721_218,
        0.242_281_2,
        0.434_153_7,
        0.637_596_9,
        0.799_102_84,
        0.806_952_36,
        0.806_952_36,
    ];
    const G_X: [f32; 9] = [
        0.0,
        0.003_676_507,
        0.015_208_514,
        0.230_740_07,
        0.462_077_05,
        0.597_201_9,
        0.768_151_3,
        0.879_622_34,
        1.0,
    ];
    const G_Y: [f32; 9] = [
        0.004_776_953,
        0.004_776_953,
        0.012_286_487,
        0.226_965_89,
        0.473_531_54,
        0.564_711_63,
        0.768_151_3,
        0.838_799_1,
        0.838_799_1,
    ];
    const B_X: [f32; 9] = [
        0.0,
        0.002_731_743,
        0.007_499_031_7,
        0.124_771_84,
        0.401_977_87,
        0.564_711_63,
        0.760_524_63,
        0.846_873_34,
        1.0,
    ];
    const B_Y: [f32; 9] = [
        0.004_024_717,
        0.004_024_717,
        0.008_568_125,
        0.147_027_27,
        0.473_531_54,
        0.597_201_9,
        0.783_537_9,
        0.830_769_96,
        0.830_769_96,
    ];
    ScanProfileParams {
        strength: 0.6,
        transform: ScanProfileTransform::curve([
            ScanCurve::from_slices(&R_X, &R_Y),
            ScanCurve::from_slices(&G_X, &G_Y),
            ScanCurve::from_slices(&B_X, &B_Y),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_is_valid_and_round_trips() {
        let profile = builtin_sp3000_inspired();
        profile.validate_detailed().unwrap();
        assert_eq!(profile.effective_name(), ScanProfileName::Sp3000Inspired);
        let json = serde_json::to_string(&profile).unwrap();
        let back: ScanProfileParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn curve_validation_rejects_missing_endpoints_and_reversals() {
        let mut profile = builtin_sp3000_inspired();
        let curves = profile.transform.curves.as_mut().unwrap();
        curves[0].x[0] = 0.1;
        assert!(profile.validate_detailed().is_err());

        let mut profile = builtin_sp3000_inspired();
        let curves = profile.transform.curves.as_mut().unwrap();
        curves[1].y[2] = curves[1].y[1] - 0.1;
        assert!(profile.validate_detailed().is_err());
    }

    #[test]
    fn curve_validation_bounds_outputs_and_input_spacing() {
        let curve = |x: Vec<f32>, y: Vec<f32>| ScanCurve::new(x, y);
        let mut high = ScanProfileParams {
            strength: 1.0,
            transform: ScanProfileTransform::curve([
                curve(vec![0.0, 1.0], vec![0.0, 1.2]),
                curve(vec![0.0, 1.0], vec![0.0, 1.0]),
                curve(vec![0.0, 1.0], vec![0.0, 1.0]),
            ]),
        };
        assert!(high.validate_detailed().is_err());
        high.transform.curves.as_mut().unwrap()[0].y[1] = 1.0;

        let crowded = ScanProfileParams {
            strength: 1.0,
            transform: ScanProfileTransform::curve([
                curve(vec![0.0, 0.0005, 1.0], vec![0.0, 0.5, 1.0]),
                curve(vec![0.0, 1.0], vec![0.0, 1.0]),
                curve(vec![0.0, 1.0], vec![0.0, 1.0]),
            ]),
        };
        assert!(crowded.validate_detailed().is_err());
        assert!(high.validate_detailed().is_ok());
    }

    #[test]
    fn bypass_and_custom_names_are_derived() {
        assert_eq!(
            ScanProfileParams::default().effective_name(),
            ScanProfileName::None
        );
        let mut profile = builtin_sp3000_inspired();
        profile.transform = ScanProfileTransform::bypass();
        assert_eq!(profile.effective_name(), ScanProfileName::None);
        profile.transform = builtin_sp3000_inspired().transform.clone();
        profile.strength = 0.2;
        assert_eq!(profile.effective_name(), ScanProfileName::Sp3000Inspired);
    }

    #[test]
    fn strength_zero_keeps_builtin_name_but_is_not_effective() {
        let mut profile = builtin_sp3000_inspired();
        profile.strength = 0.0;
        // The name is derived from the transform, not the strength.
        assert_eq!(profile.effective_name(), ScanProfileName::Sp3000Inspired);
        // But the stage is a no-op at strength 0.
        assert!(!profile.is_effective());
    }

    #[test]
    fn custom_transform_reports_custom_name() {
        let mut profile = builtin_sp3000_inspired();
        profile.transform.curves.as_mut().unwrap()[0].y[4] += 0.01;
        assert_eq!(profile.effective_name(), ScanProfileName::Custom);
    }
}
