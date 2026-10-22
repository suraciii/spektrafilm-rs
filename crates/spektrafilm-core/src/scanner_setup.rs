//! Vendor-neutral, interpretation-only setup derived from one complete input frame.

use serde::Serialize;
use sha2::Digest;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::to_f64;

use crate::params::{RuntimeParams, ScannerAutoSetup};
use crate::profile::Profile;
const CALIBRATION_IMPLEMENTATION: &str = "modeled-positive-interpretation-v1";
const CONTRAST_MIN: f64 = 0.5;
const CONTRAST_MAX: f64 = 2.0;
const TARGET_LUMA: f64 = 0.18;
const FLAT_SPAN: f64 = 1.0e-6;
const MIN_SAMPLE: f64 = 1.0e-6;
const MAX_DENSITY_EV: f64 = 8.0;
const MAX_BALANCE_EV: f64 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScannerSetup {
    pub policy: ScannerAutoSetup,
    pub density_ev: f64,
    pub balance_ev: [f64; 3],
    pub contrast_scale: f64,
    pub valid_sample_count: usize,
}

impl ScannerSetup {
    pub const IDENTITY: Self = Self {
        policy: ScannerAutoSetup::Off,
        density_ev: 0.0,
        balance_ev: [0.0; 3],
        contrast_scale: 1.0,
        valid_sample_count: 0,
    };

    pub fn from_image(image: &ImageBuf, params: &RuntimeParams) -> Result<Self, String> {
        let policy = params.scanner.auto_setup;
        if policy == ScannerAutoSetup::Off {
            return Ok(Self::IDENTITY);
        }
        let space = colorspace::resolve(&params.io.input_color_space)
            .map_err(|e| format!("scanner auto setup input colour space: {e}"))?;
        let cctf = params.io.input_cctf_decoding.then_some(space.cctf);
        let mut channels = [Vec::new(), Vec::new(), Vec::new()];
        let mut luma = Vec::new();
        for pixel in image.data.as_chunks::<3>().0 {
            let mut rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
            if !rgb.iter().all(|v| v.is_finite()) {
                continue;
            }
            if let Some(cctf) = cctf {
                for value in &mut rgb {
                    *value = colorspace::cctf_decode(*value, cctf);
                }
            }
            if !rgb.iter().all(|v| v.is_finite()) {
                continue;
            }
            let positive = rgb.map(|v| v.max(MIN_SAMPLE));
            for c in 0..3 {
                channels[c].push(positive[c].log2());
            }
            luma.push(
                (0.2126 * positive[0] + 0.7152 * positive[1] + 0.0722 * positive[2])
                    .max(MIN_SAMPLE)
                    .log2(),
            );
        }
        let valid_sample_count = luma.len();
        if valid_sample_count == 0 {
            return Err(
                "scanner.auto_setup requires at least one finite complete-frame RGB sample".into(),
            );
        }
        for values in &mut channels {
            values.sort_unstable_by(f64::total_cmp);
        }
        luma.sort_unstable_by(f64::total_cmp);
        let medians = channels.map(|v| quantile(&v, 0.5));
        let neutral = medians.iter().sum::<f64>() / 3.0;
        let mut balance = medians
            .map(|v| neutral - v)
            .map(|v| v.clamp(-MAX_BALANCE_EV, MAX_BALANCE_EV));
        balance[2] = -(balance[0] + balance[1]);
        if balance[2].abs() > MAX_BALANCE_EV {
            let scale = MAX_BALANCE_EV / balance[2].abs();
            balance = balance.map(|v| v * scale);
        }
        let density =
            (TARGET_LUMA.log2() - quantile(&luma, 0.5)).clamp(-MAX_DENSITY_EV, MAX_DENSITY_EV);
        let span = quantile(&luma, 0.95) - quantile(&luma, 0.05);
        let contrast = if span <= FLAT_SPAN {
            1.0
        } else {
            (4.0 / span).clamp(CONTRAST_MIN, CONTRAST_MAX)
        };
        if !density.is_finite() || !balance.iter().all(|v| v.is_finite()) || !contrast.is_finite() {
            return Err("scanner.auto_setup produced a non-finite or unbounded setup".into());
        }
        Ok(Self {
            policy,
            density_ev: if policy.includes_density() {
                density
            } else {
                0.0
            },
            balance_ev: if policy.includes_color() {
                balance
            } else {
                [0.0; 3]
            },
            contrast_scale: if policy.includes_contrast() {
                contrast
            } else {
                1.0
            },
            valid_sample_count,
        })
    }

    pub fn apply(&self, params: &mut RuntimeParams) {
        if self.policy == ScannerAutoSetup::Off {
            return;
        }
        params.scanner.conversion_brightness_ev += self.density_ev;
        for channel in 0..3 {
            params.scanner.conversion_balance_ev[channel] += self.balance_ev[channel];
        }
        params.scanner.conversion_contrast =
            (params.scanner.conversion_contrast * self.contrast_scale).max(f64::MIN_POSITIVE);
    }

    pub fn measurement_domain(&self) -> &'static str {
        "complete_decoded_input_frame:linear_input_rgb_log2_luminance_percentiles"
    }
    pub fn calibration_identity(profile: &Profile) -> String {
        format!(
            "receiver:{}:profile:{}:{}:{}:{}:{}",
            CALIBRATION_IMPLEMENTATION,
            profile.info.stock.as_deref().unwrap_or("unknown"),
            profile.metadata.version,
            profile.info.densitometer,
            profile.info.viewing_illuminant,
            profile.info.channel_model
        )
    }
    pub fn provenance(
        &self,
        profile: &Profile,
        input_identity: ScannerInputIdentity,
    ) -> ScannerSetupProvenance {
        ScannerSetupProvenance {
            policy: self.policy,
            applied: self.policy != ScannerAutoSetup::Off,
            calibration_identity: Self::calibration_identity(profile),
            input_identity,
            complete_frame_measurement: self.policy != ScannerAutoSetup::Off,
            valid_sample_count: self.valid_sample_count,
            measurement_domain: self.measurement_domain(),
            density_ev: self.policy.includes_density().then_some(self.density_ev),
            balance_ev: self.policy.includes_color().then_some(self.balance_ev),
            contrast_scale: self
                .policy
                .includes_contrast()
                .then_some(self.contrast_scale),
            interpretation_only: true,
            acquisition_changes_modeled: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerInputIdentity {
    pub source: String,
    pub source_sha256: String,
    pub decoded_sha256: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannerSetupProvenance {
    pub policy: ScannerAutoSetup,
    pub applied: bool,
    pub calibration_identity: String,
    pub input_identity: ScannerInputIdentity,
    pub complete_frame_measurement: bool,
    pub valid_sample_count: usize,
    pub measurement_domain: &'static str,
    pub density_ev: Option<f64>,
    pub balance_ev: Option<[f64; 3]>,
    pub contrast_scale: Option<f64>,
    pub interpretation_only: bool,
    pub acquisition_changes_modeled: bool,
}

fn quantile(sorted: &[f64], fraction: f64) -> f64 {
    let position = fraction * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    sorted[lower] * (1.0 - position + lower as f64) + sorted[upper] * (position - lower as f64)
}

pub fn input_identity(
    path: &std::path::Path,
    image: &ImageBuf,
) -> Result<ScannerInputIdentity, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("reading input identity source {}: {e}", path.display()))?;
    let mut source = sha2::Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("reading input identity source {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        source.update(&buffer[..read]);
    }
    let mut decoded = sha2::Sha256::new();
    decoded.update(image.width.to_le_bytes());
    decoded.update(image.height.to_le_bytes());
    for value in &image.data {
        decoded.update(to_f64(*value).to_le_bytes());
    }
    Ok(ScannerInputIdentity {
        source: path.to_string_lossy().into_owned(),
        source_sha256: format!("{:x}", source.finalize()),
        decoded_sha256: format!("{:x}", decoded.finalize()),
        width: image.width,
        height: image.height,
    })
}

pub fn validate_route(film: &Profile, params: &RuntimeParams) -> Result<(), String> {
    if params.scanner.auto_setup == ScannerAutoSetup::Off {
        return Ok(());
    }
    let route = crate::telemetry::WorkflowRoute::parse(params.workflow.route.as_str());
    let route_name = match route {
        crate::telemetry::WorkflowRoute::Input => "input_only",
        crate::telemetry::WorkflowRoute::FilmPrintScan
        | crate::telemetry::WorkflowRoute::ConvertFilmPrintScan => "print_output",
        crate::telemetry::WorkflowRoute::ConvertFilmScanMinusBase => "scan_minus_base",
        _ if params.scanner.negative_conversion_method
            == crate::params::NegativeConversionMethod::DigitalReference =>
        {
            "digital_reference"
        }
        crate::telemetry::WorkflowRoute::FilmScan
        | crate::telemetry::WorkflowRoute::ConvertFilmScan
            if params.scanner.scan_output != "negative_to_positive" =>
        {
            "direct_faithful_scan"
        }
        crate::telemetry::WorkflowRoute::FilmScan
        | crate::telemetry::WorkflowRoute::ConvertFilmScan
            if !film.is_negative() || !film.is_film() || !film.is_filming() =>
        {
            "unsupported_scan_medium"
        }
        crate::telemetry::WorkflowRoute::FilmScan
        | crate::telemetry::WorkflowRoute::ConvertFilmScan => "negative_to_positive",
        _ => "unsupported_route",
    };
    let supported = matches!(
        route,
        crate::telemetry::WorkflowRoute::FilmScan
            | crate::telemetry::WorkflowRoute::ConvertFilmScan
    ) && params.scanner.scan_output == "negative_to_positive"
        && params.scanner.negative_conversion_method
            != crate::params::NegativeConversionMethod::DigitalReference
        && film.is_negative()
        && film.is_film()
        && film.is_filming();
    if supported {
        Ok(())
    } else {
        Err(format!(
            "scanner.auto_setup={} is not valid for route {route_name}; use off or the complete negative film-to-positive interpretation route",
            params.scanner.auto_setup
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::RuntimeParams;
    use spektrafilm_math::precision::from_f64;
    fn params(policy: ScannerAutoSetup) -> RuntimeParams {
        let mut p = RuntimeParams::default();
        p.scanner.auto_setup = policy;
        p.io.scan_film = true;
        p
    }
    fn ramp() -> ImageBuf {
        ImageBuf::from_data(
            101,
            1,
            (0..101)
                .flat_map(|i| {
                    let v = 0.02 + i as f64 * 0.008;
                    [from_f64(v); 3]
                })
                .collect(),
        )
    }
    #[test]
    fn policy_selects_only_requested_components() {
        let image = ramp();
        let d = ScannerSetup::from_image(&image, &params(ScannerAutoSetup::Density)).unwrap();
        assert_ne!(d.density_ev, 0.0);
        assert_eq!(d.balance_ev, [0.0; 3]);
        assert_eq!(d.contrast_scale, 1.0);
        let c = ScannerSetup::from_image(&image, &params(ScannerAutoSetup::DensityColor)).unwrap();
        assert_eq!(c.contrast_scale, 1.0);
        assert_eq!(c.balance_ev.iter().sum::<f64>(), 0.0);
        assert!(
            ScannerSetup::from_image(&image, &params(ScannerAutoSetup::Full))
                .unwrap()
                .contrast_scale
                .is_finite()
        );
    }
    #[test]
    fn flat_frame_and_invalid_frame_contracts_hold() {
        let flat = ImageBuf::from_data(4, 4, vec![from_f64(0.3); 48]);
        let setup = ScannerSetup::from_image(&flat, &params(ScannerAutoSetup::Full)).unwrap();
        assert!(setup.density_ev.is_finite());
        assert_eq!(setup.contrast_scale, 1.0);
        let bad = ImageBuf::from_data(1, 1, vec![from_f64(f64::NAN); 3]);
        assert!(
            ScannerSetup::from_image(&bad, &params(ScannerAutoSetup::Density))
                .unwrap_err()
                .contains("finite")
        );
    }
    #[test]
    fn legacy_alias_serializes_canonically() {
        let p: RuntimeParams =
            serde_json::from_str(r#"{"scanner":{"auto_setup":"per_frame"}}"#).unwrap();
        assert_eq!(p.scanner.auto_setup, ScannerAutoSetup::Full);
        assert_eq!(
            serde_json::to_value(p).unwrap()["scanner"]["auto_setup"],
            "full"
        );
    }
    #[test]
    fn route_validation_accepts_only_negative_positive_interpretation_routes() {
        let data = crate::test_support::data_dir();
        let negative = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let positive = crate::profile::load_profile_by_name(&data, "kodak_ektachrome_100").unwrap();
        let mut params = params(ScannerAutoSetup::Full);
        params.scanner.scan_output = "negative_to_positive".into();

        for route in ["input > film > scan", "input > convert-film > scan"] {
            params.workflow.route = route.into();
            params.io.scan_film = route == "input > film > scan";
            assert!(validate_route(&negative, &params).is_ok(), "{route}");
        }

        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "direct_scan".into();
        assert!(
            validate_route(&negative, &params)
                .unwrap_err()
                .contains("direct_faithful_scan")
        );
        assert!(
            validate_route(&positive, &params)
                .unwrap_err()
                .contains("direct_faithful_scan")
        );

        params.scanner.scan_output = "negative_to_positive".into();
        params.workflow.route = "input > film > print > scan".into();
        params.io.scan_film = false;
        assert!(
            validate_route(&negative, &params)
                .unwrap_err()
                .contains("print_output")
        );

        params.workflow.route = "input > convert-film > scan-minus-base".into();
        assert!(
            validate_route(&negative, &params)
                .unwrap_err()
                .contains("scan_minus_base")
        );

        params.workflow.route = "input".into();
        assert!(
            validate_route(&negative, &params)
                .unwrap_err()
                .contains("input_only")
        );

        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.negative_conversion_method =
            crate::params::NegativeConversionMethod::DigitalReference;
        assert!(
            validate_route(&negative, &params)
                .unwrap_err()
                .contains("digital_reference")
        );
        params.scanner.negative_conversion_method =
            crate::params::NegativeConversionMethod::Density;
        assert!(validate_route(&positive, &params).is_err());
    }
}
