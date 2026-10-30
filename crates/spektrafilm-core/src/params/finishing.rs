//! Shared, look-owned controls for the completed positive image.
use super::{ValidationError, ValidationReason};
use serde::{Deserialize, Serialize};

pub const EXPAND_ENDPOINT_MIN: f64 = -0.25;
pub const FINISHING_CONTROL_MIN: f64 = -1.0;
pub const FINISHING_CONTROL_MAX: f64 = 1.0;
pub const EXPAND_ENDPOINT_MAX: f64 = 1.25;
pub const EXPAND_MIN_SPAN: f64 = 1.0 / 1024.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FinishingParams {
    pub brightness: f64,
    pub contrast: f64,
    pub warmth: f64,
    pub saturation: f64,
    pub expand: ExpandParams,
}

impl Default for FinishingParams {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 0.0,
            warmth: 0.0,
            saturation: 0.0,
            expand: ExpandParams::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExpandParams {
    pub enabled: bool,
    pub black: f64,
    pub white: f64,
}

impl Default for ExpandParams {
    fn default() -> Self {
        Self {
            enabled: false,
            black: 0.0,
            white: 1.0,
        }
    }
}

impl FinishingParams {
    /// Complete documents must not replace absent controls with sparse defaults.
    pub fn require_complete(value: &serde_json::Value) -> Result<(), String> {
        let group = value
            .as_object()
            .ok_or("finishing: complete group required")?;
        for name in ["brightness", "contrast", "warmth", "saturation", "expand"] {
            if !group.contains_key(name) {
                return Err(format!("finishing.{name}: required in a complete document"));
            }
        }
        let expand = group["expand"]
            .as_object()
            .ok_or("finishing.expand: complete group required")?;
        for name in ["enabled", "black", "white"] {
            if !expand.contains_key(name) {
                return Err(format!(
                    "finishing.expand.{name}: required in a complete document"
                ));
            }
        }
        Ok(())
    }

    pub fn deserialize_complete<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::require_complete(&value).map_err(serde::de::Error::custom)?;
        serde_json::from_value(value).map_err(serde::de::Error::custom)
    }

    pub fn is_neutral(&self) -> bool {
        self.brightness == 0.0
            && self.contrast == 0.0
            && self.warmth == 0.0
            && self.saturation == 0.0
            && !self.expand.enabled
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_detailed().map_err(|error| error.to_string())
    }

    pub fn validate_detailed(&self) -> Result<(), ValidationError> {
        for (path, value) in [
            ("finishing.brightness", self.brightness),
            ("finishing.contrast", self.contrast),
            ("finishing.warmth", self.warmth),
            ("finishing.saturation", self.saturation),
            ("finishing.expand.black", self.expand.black),
            ("finishing.expand.white", self.expand.white),
        ] {
            let (min, max) =
                super::validation::leaf_numeric_bounds(path).expect("finishing numeric bounds");
            let max = max.expect("finishing maximum");
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(ValidationError::new(
                    ValidationReason::NumericRange,
                    path,
                    format!("{path}: must be finite and in {min}..={max}"),
                )
                .arg("min", min)
                .arg("max", max)
                .arg("value", value));
            }
        }
        if self.expand.white - self.expand.black < EXPAND_MIN_SPAN {
            return Err(ValidationError::new(
                ValidationReason::FinishingExpandSpan,
                "finishing.expand",
                format!("finishing.expand: white must exceed black by at least {EXPAND_MIN_SPAN}"),
            )
            .arg("minimum_span", EXPAND_MIN_SPAN)
            .arg("black", self.expand.black)
            .arg("white", self.expand.white));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_expand_retains_references_but_still_validates_them() {
        let mut p = FinishingParams::default();
        p.expand.black = 0.8;
        p.expand.white = 0.7;
        assert!(p.is_neutral());
        assert_eq!(
            p.validate_detailed().unwrap_err().reason,
            ValidationReason::FinishingExpandSpan
        );
        p.expand.white = p.expand.black + EXPAND_MIN_SPAN;
        assert!(p.validate().is_ok());
        p.expand.enabled = true;
        assert!(!p.is_neutral());
        p.expand.black = f64::NAN;
        assert!(p.validate().is_err());
    }

    #[test]
    fn sparse_edits_share_domains_and_preserve_inactive_values() {
        use crate::params::sources::ParameterEdits;
        let mut params = crate::params::RuntimeParams::default();
        params.workflow.route = "input".into();
        ParameterEdits::parse("finishing.brightness=0.5,finishing.expand.black=0.25")
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(params.finishing.brightness, 0.5);
        assert_eq!(params.finishing.expand.black, 0.25);
        assert!(!params.finishing.expand.enabled);
        assert!(params.validate().is_ok());
        assert!(ParameterEdits::parse("finishing.warmth=1.01").is_err());
        assert!(ParameterEdits::parse("finishing.expand.black=null").is_err());
        assert!(ParameterEdits::parse("finishing.unknown=0").is_err());
        ParameterEdits::parse("finishing.expand.white=0.25")
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(
            params.validate_detailed().unwrap_err().reason,
            ValidationReason::FinishingExpandSpan
        );
    }
}
