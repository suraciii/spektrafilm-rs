//! Versioned provenance of the neutral scan model retained by recipes and
//! GUI state. The metadata is derived from the built-in implementation so
//! callers cannot claim a measured device profile by supplying strings.
use serde::Serialize;

/// Provenance of the one built-in scanner model.
#[derive(Debug, Clone, Serialize)]
pub struct ScannerProvenance {
    /// Model identity: lab scan readout v1.
    pub model: &'static str,
    /// Sensor model: CIE 1931 colorimetric reading.
    pub sensor: &'static str,
    /// White calibration reference: the clear film base reading.
    pub white_reference: &'static str,
    /// Where the chromatic adaptation ends: the output colour space white.
    pub output_white: &'static str,
    /// Device matrix: neutral model ships identity.
    pub matrix: &'static str,
}

impl ScannerProvenance {
    pub fn from_params(_params: &crate::params::RuntimeParams) -> Self {
        Self::LAB_SCAN_V1
    }

    /// Validate metadata from a versioned legacy capture before re-deriving
    /// it. Never bless arbitrary measured-device strings during migration.
    pub fn migrate_legacy(
        value: &serde_json::Value,
        lamp: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        Self::migrate_legacy_detailed(value, lamp).map_err(|error| error.to_string())
    }

    pub fn migrate_legacy_detailed(
        value: &serde_json::Value,
        lamp: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, crate::params::ValidationError> {
        use crate::params::{ValidationError as Error, ValidationReason as Reason};
        let current = serde_json::to_value(Self::LAB_SCAN_V1).map_err(|error| {
            Error::new(
                Reason::SerializeParameters,
                "scanner_provenance",
                error.to_string(),
            )
            .arg("detail", error)
        })?;
        let mut legacy = current.clone();
        legacy["model"] = "neutral_scan_v2".into();
        let mut with_lamp = legacy.clone();
        with_lamp["lamp"] = lamp.cloned().unwrap_or_else(|| "D55".into());
        if value != &legacy && value != &with_lamp && value != &current {
            return Err(Error::new(
                Reason::LegacyProvenance,
                "scanner_provenance",
                "legacy scanner provenance does not match the built-in readout model",
            ));
        }
        Ok(current)
    }

    const LAB_SCAN_V1: Self = Self {
        model: "lab_scan_v1",
        sensor: "cie_1931_cmf",
        white_reference: "clear_film_base",
        output_white: "output_color_space_white",
        matrix: "identity",
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_is_derived_from_the_builtin_model() {
        let params = crate::params::RuntimeParams::default();
        let provenance = ScannerProvenance::from_params(&params);
        assert_eq!(provenance.model, "lab_scan_v1");
        assert_eq!(provenance.white_reference, "clear_film_base");
        assert_eq!(provenance.output_white, "output_color_space_white");
        assert_eq!(provenance.matrix, "identity");
    }
}
