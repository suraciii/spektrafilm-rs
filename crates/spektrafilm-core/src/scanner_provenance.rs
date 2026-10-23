//! Versioned provenance of the neutral scan model retained by recipes and
//! GUI state. The metadata is derived from the built-in implementation so
//! callers cannot claim a measured device profile by supplying strings.
use serde::Serialize;

/// Provenance of the one built-in scanner model.
#[derive(Debug, Clone, Serialize)]
pub struct ScannerProvenance {
    /// Model identity: neutral scan model v2.
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
        Self::NEUTRAL_SCAN_V2
    }

    const NEUTRAL_SCAN_V2: Self = Self {
        model: "neutral_scan_v2",
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
        assert_eq!(provenance.model, "neutral_scan_v2");
        assert_eq!(provenance.white_reference, "clear_film_base");
        assert_eq!(provenance.output_white, "output_color_space_white");
        assert_eq!(provenance.matrix, "identity");
    }
}
