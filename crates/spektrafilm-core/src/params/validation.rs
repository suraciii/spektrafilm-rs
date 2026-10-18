use super::{RuntimeParams, Tap};
use crate::WorkflowRoute;
use crate::suggest::{MAX_SUGGESTIONS, closest, error_suffix};

pub(crate) const SUPPORTED_ROUTES: [&str; 6] = [
    "input",
    "input > film > scan",
    "input > film > print > scan",
    "input > convert-film > print > scan",
    "input > convert-film > scan-minus-base",
    "input > convert-film > scan",
];

/// The removed dedicated magazine workflow. Saved configurations that still
/// name it must be migrated to a base workflow plus
/// `magazine_print_color.active`; it is not a selectable route.
pub(crate) const REMOVED_MAGAZINE_ROUTE: &str = "input > film > scan > magazine";

/// Registered RGB-to-raw implementations. Parsing and construction share this
/// vocabulary so a method cannot validate successfully and fail at dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RgbToRawMethod {
    Hanatos2025,
    Mallett2019,
    Arctic2026Alpha02,
    Arctic2026Beta04,
    GaussLasers,
    Jakob2019,
    Otsu2018,
}

impl RgbToRawMethod {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        let method = match value {
            "hanatos2025" => Self::Hanatos2025,
            "mallett2019" => Self::Mallett2019,
            "arctic2026alpha02" => Self::Arctic2026Alpha02,
            "arctic2026beta04" => Self::Arctic2026Beta04,
            "gauss-lasers" => Self::GaussLasers,
            "jakob2019" => Self::Jakob2019,
            "otsu2018" => Self::Otsu2018,
            other => {
                return Err(format!(
                    "settings.rgb_to_raw_method: unsupported method {:?}; supported: {}",
                    other,
                    rgb_to_raw_names().join(", ")
                ));
            }
        };
        Ok(method)
    }
}

pub(crate) fn rgb_to_raw_names() -> [&'static str; 7] {
    [
        "hanatos2025",
        "mallett2019",
        "arctic2026alpha02",
        "arctic2026beta04",
        "gauss-lasers",
        "jakob2019",
        "otsu2018",
    ]
}

/// Per-leaf numeric domains shared by runtime validation, sparse sources and
/// field metadata. Returns `(minimum, maximum)` inclusive bounds; `None`
/// maximum means unbounded above.
pub(crate) fn leaf_numeric_bounds(path: &str) -> Option<(f64, Option<f64>)> {
    match path {
        "output_mix.amount" => Some((0.0, Some(1.0))),
        "film_render.chemistry.developer_exhaustion"
        | "print_render.density_curves_morph.developer_exhaustion" => Some((0.0, Some(1.0))),
        "film_render.grain.fotufilm.amount" | "film_render.grain.fotufilm.layers" => {
            Some((0.0, Some(4.0)))
        }
        "film_render.grain.fotufilm.size" | "film_render.grain.fotufilm.softness" => {
            Some((0.5, Some(4.0)))
        }
        "film_render.grain.fotufilm.color" | "film_render.grain.fotufilm.sublayer_shares" => {
            Some((0.0, Some(1.0)))
        }
        "film_render.grain.fotufilm.rms_density" => Some((0.0, Some(0.1))),
        "film_render.grain.fotufilm.clump_radius_um" => Some((0.0, Some(32.0))),
        "film_render.grain.fotufilm.layer_weights" => Some((0.0, Some(4.0))),
        "film_render.grain.fotufilm.layer_size_ratio"
        | "film_render.grain.fotufilm.coating_density_scale" => Some((0.25, Some(4.0))),
        "film_render.grain.fotufilm.radius_span" => Some((1.0, Some(20.0))),
        "film_render.grain.fotufilm.fog_density" => Some((0.0, Some(0.3))),
        "film_render.grain.v2_size" => Some((1.0, Some(48.0))),
        "film_render.grain.v2_amount"
        | "film_render.grain.v2_shadows"
        | "film_render.grain.v2_midtones"
        | "film_render.grain.v2_highlights"
        | "film_render.grain.v2_chroma"
        | "film_render.grain.v2_resolution_factor" => Some((0.0, Some(100.0))),
        "film_render.grain.v2_resolution_type" => Some((0.0, Some(1.0))),
        "film_render.grain.v2_timer" => Some((0.0, Some(1.0))),
        "film_render.grain.v3_dye_support_um" | "film_render.grain.v3_event_area_um2" => {
            Some((0.0, None))
        }
        "film_render.grain.channel_correlation" => Some((
            0.0,
            Some(super::grain::MEASURED_CHANNEL_CORRELATION_CEILING),
        )),
        // Discovery range only; `validate` and the sparse-source leaf check
        // own the strict positivity.
        "camera.film_format_mm" => Some((0.0, None)),
        "io.input_gamut_compress.hull_detail" => Some((0.0, None)),
        "magazine_print_color.strength" => Some((0.0, Some(1.0))),
        "scanner.black_point" | "scanner.white_point" => Some((0.0, Some(1.0))),
        "scanner.finish_strength" => Some((0.0, Some(1.0))),
        "scanner.conversion_brightness_ev"
        | "scanner.conversion_balance_ev"
        | "scanner.reference_exposure_ev"
        | "scanner.reference_balance_ev" => Some((-16.0, Some(16.0))),
        "scanner.conversion_contrast" | "scanner.conversion_dmax" => Some((0.0, Some(16.0))),
        _ => None,
    }
}

pub(crate) fn fotufilm_minimum_exclusive(path: &str) -> bool {
    matches!(
        path,
        "film_render.grain.fotufilm.rms_density"
            | "film_render.grain.fotufilm.clump_radius_um"
            | "film_render.grain.fotufilm.layer_weights"
            | "film_render.grain.fotufilm.sublayer_shares"
    )
}

/// Shared sparse-source and typed-runtime domains; shares are rejected, not normalized.
pub(crate) fn validate_fotufilm_values(path: &str, values: &[f64]) -> Result<(), String> {
    let (min, max) = leaf_numeric_bounds(path).expect("Fotufilm numeric field domain");
    let max = max.expect("Fotufilm finite maximum");
    for (index, &value) in values.iter().enumerate() {
        if !value.is_finite()
            || value < min
            || value > max
            || (fotufilm_minimum_exclusive(path) && value == min)
        {
            let lower = if fotufilm_minimum_exclusive(path) {
                "("
            } else {
                "["
            };
            return Err(format!(
                "{path}: component {index} must be finite and in {lower}{min}, {max}] (got {value})"
            ));
        }
    }
    if path == "film_render.grain.fotufilm.sublayer_shares"
        && (values.iter().sum::<f64>() - 1.0).abs() > 1e-6
    {
        return Err(format!(
            "{path}: shares must sum to 1 within 1e-6; no automatic normalization"
        ));
    }
    Ok(())
}

impl super::FotufilmGrainParams {
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (path, values) in [
            (
                "film_render.grain.fotufilm.amount",
                std::slice::from_ref(&self.amount),
            ),
            (
                "film_render.grain.fotufilm.size",
                std::slice::from_ref(&self.size),
            ),
            (
                "film_render.grain.fotufilm.softness",
                std::slice::from_ref(&self.softness),
            ),
            (
                "film_render.grain.fotufilm.color",
                std::slice::from_ref(&self.color),
            ),
            ("film_render.grain.fotufilm.layers", self.layers.as_slice()),
            (
                "film_render.grain.fotufilm.rms_density",
                std::slice::from_ref(&self.rms_density),
            ),
            (
                "film_render.grain.fotufilm.clump_radius_um",
                std::slice::from_ref(&self.clump_radius_um),
            ),
            (
                "film_render.grain.fotufilm.layer_weights",
                self.layer_weights.as_slice(),
            ),
            (
                "film_render.grain.fotufilm.layer_size_ratio",
                self.layer_size_ratio.as_slice(),
            ),
            (
                "film_render.grain.fotufilm.radius_span",
                std::slice::from_ref(&self.radius_span),
            ),
            (
                "film_render.grain.fotufilm.sublayer_shares",
                self.sublayer_shares.as_slice(),
            ),
            (
                "film_render.grain.fotufilm.coating_density_scale",
                std::slice::from_ref(&self.coating_density_scale),
            ),
            (
                "film_render.grain.fotufilm.fog_density",
                std::slice::from_ref(&self.fog_density),
            ),
        ] {
            validate_fotufilm_values(path, values)?;
        }
        Ok(())
    }
}

/// Gamut triplets have distinct threshold, limit and power domains. Keep the
/// component bounds shared by runtime construction, source checks and discovery.
#[derive(Clone, Copy, serde::Serialize)]
pub(crate) struct GamutComponentDomain {
    pub component: &'static str,
    pub minimum: f32,
    pub minimum_exclusive: bool,
    pub maximum: Option<f32>,
    pub maximum_exclusive: bool,
}

const GAMUT_COMPONENT_DOMAINS: [GamutComponentDomain; 3] = [
    GamutComponentDomain {
        component: "threshold",
        minimum: 0.0,
        minimum_exclusive: false,
        maximum: Some(1.0),
        maximum_exclusive: true,
    },
    GamutComponentDomain {
        component: "limit",
        minimum: 0.0,
        minimum_exclusive: true,
        maximum: None,
        maximum_exclusive: false,
    },
    GamutComponentDomain {
        component: "power",
        minimum: 0.0,
        minimum_exclusive: true,
        maximum: None,
        maximum_exclusive: false,
    },
];

pub(crate) fn gamut_component_domains(path: &str) -> Option<&'static [GamutComponentDomain; 3]> {
    match path {
        "io.input_gamut_compress.knee" | "io.output_gamut_compress.knee" => {
            Some(&GAMUT_COMPONENT_DOMAINS)
        }
        _ => None,
    }
}

pub(crate) fn validate_gamut_triplet(
    name: &str,
    values: [f32; 3],
) -> Result<(f64, f64, f64), String> {
    for (value, domain) in values.into_iter().zip(GAMUT_COMPONENT_DOMAINS) {
        let below = value < domain.minimum || (domain.minimum_exclusive && value == domain.minimum);
        let above = domain.maximum.is_some_and(|maximum| {
            value > maximum || (domain.maximum_exclusive && value == maximum)
        });
        if !value.is_finite() || below || above {
            let requirement = if domain.maximum.is_some() {
                "finite and in [0, 1)"
            } else {
                "finite and positive"
            };
            return Err(format!(
                "{name} {} must be {requirement}, got {value}",
                domain.component
            ));
        }
    }
    let [threshold, limit, power] = values.map(f64::from);
    Ok((threshold, limit, power))
}

/// Grain Procedural timer override sentinel: 0 selects the seed-derived phase;
/// otherwise the phase must be finite and strictly below 1.
pub(crate) fn validate_v2_timer(value: f64) -> Result<(), String> {
    if value != 0.0 && (!value.is_finite() || !(0.0..1.0).contains(&value)) {
        return Err("film_render.grain.v2_timer: must be 0 or finite in (0,1)".into());
    }
    Ok(())
}
/// Discovery vocabulary for a leaf. Pseudo-tokens document patterns that are
/// validated but not enumerable (for example `BB<kelvin>` blackbody sources).
pub(crate) fn enum_values(path: &str) -> Option<Vec<String>> {
    let values: Vec<String> = match path {
        "scanner.receiver_profile" => [
            "legacy_colorimetric",
            "lab_scan_approx",
            "digital_reference",
        ]
        .iter()
        .map(|v| (*v).into())
        .collect(),
        "scanner.negative_conversion_method" => {
            ["density", "separable", "coupled", "digital_reference"]
                .iter()
                .map(|v| (*v).into())
                .collect()
        }
        "scanner.auto_setup" => ["off", "per_frame"].iter().map(|v| (*v).into()).collect(),
        "scanner.finish" => ["neutral", "lab_scan"]
            .iter()
            .map(|v| (*v).into())
            .collect(),
        "scanner.scan_output" => ["direct_scan", "negative_to_positive"]
            .iter()
            .map(|v| (*v).into())
            .collect(),
        "film_render.grain.engine" => ["spektra", "procedural", "dye", "fotufilm"]
            .iter()
            .map(|v| (*v).into())
            .collect(),
        "film_render.grain.fotufilm.material" => ["auto", "silver", "dye_negative", "dye_reversal"]
            .iter()
            .map(|v| (*v).into())
            .collect(),
        "film_render.grain.v2_mode" => ["analogue", "noise"].iter().map(|v| (*v).into()).collect(),
        "film_render.grain.v2_film_type" => ["negative", "positive"]
            .iter()
            .map(|v| (*v).into())
            .collect(),
        "film_render.grain.v2_profile" => spektrafilm_model::grain::v2::PROFILE_NAMES
            .iter()
            .map(|v| (*v).into())
            .chain(std::iter::once("custom".into()))
            .collect(),
        "settings.rgb_to_raw_method" => rgb_to_raw_names().iter().map(|v| (*v).into()).collect(),
        "camera.color_filter" => crate::spectral_service::available_color_filters()
            .iter()
            .map(|v| (*v).to_owned())
            .collect(),
        "camera.capture_illuminant"
        | "enlarger.illuminant"
        | "film_render.convert.scan_illuminant" => crate::spectral_service::available_illuminants()
            .iter()
            .map(|v| (*v).to_owned())
            .chain(std::iter::once("BB<kelvin> (1667..=25000)".into()))
            .collect(),
        "camera.diffusion_filter.filter_family" | "enlarger.diffusion_filter.filter_family" => {
            ["glimmerglass", "black_pro_mist", "pro_mist", "cinebloom"]
                .iter()
                .map(|v| (*v).into())
                .collect()
        }
        "io.input_color_space" | "io.output_color_space" => [
            "sRGB",
            "DCI-P3",
            "Display P3",
            "Adobe RGB (1998)",
            "ITU-R BT.2020",
            "ProPhoto RGB",
            "ACES2065-1",
        ]
        .iter()
        .map(|v| (*v).into())
        .chain(
            spektrafilm_math::lut_primaries::TRANSPORT_PRIMARIES
                .iter()
                .map(|s| s.name.to_owned()),
        )
        .collect(),
        "io.output_gamut_compress.algorithm" => {
            ["off", "aces_rgc", "oklch", "oklrab", "jzazbz", "cam16ucs"]
                .iter()
                .map(|v| (*v).into())
                .collect()
        }
        "io.input_gamut_compress.algorithm" => {
            ["xy", "oklch", "off"].iter().map(|v| (*v).into()).collect()
        }
        "io.input_gamut_compress.boundary" => ["locus", "inscribed_hull"]
            .iter()
            .map(|v| (*v).into())
            .collect(),
        "camera.auto_exposure_method" => [
            "average",
            "median",
            "center_weighted",
            "partial",
            "matrix",
            "multi_zone",
            "highlight_weighted",
        ]
        .iter()
        .map(|v| (*v).into())
        .collect(),
        "taps.inject" | "taps.collect" => [
            "rgb_in",
            "rgb_pre",
            "log_e_film",
            "cmy_film",
            "log_e_print",
            "cmy_print",
            "rgb_out",
        ]
        .iter()
        .map(|v| (*v).into())
        .collect(),
        _ => return None,
    };
    Some(values)
}

/// Per-leaf vocabulary validation, independent of cross-field state.
pub(crate) fn validate_enum_value(path: &str, value: &str) -> Result<(), String> {
    let valid = match path {
        "scanner.receiver_profile" => matches!(
            value,
            "legacy_colorimetric" | "lab_scan_approx" | "digital_reference"
        ),
        "scanner.negative_conversion_method" => matches!(
            value,
            "density" | "separable" | "coupled" | "digital_reference"
        ),
        "scanner.auto_setup" => matches!(value, "off" | "per_frame"),
        "scanner.finish" => matches!(value, "neutral" | "lab_scan"),
        "scanner.scan_output" => matches!(value, "direct_scan" | "negative_to_positive"),
        "film_render.grain.engine" => {
            matches!(value, "spektra" | "procedural" | "dye" | "fotufilm")
        }
        "film_render.grain.fotufilm.material" => {
            matches!(value, "auto" | "silver" | "dye_negative" | "dye_reversal")
        }
        "film_render.grain.v2_mode" => matches!(value, "analogue" | "noise"),
        "film_render.grain.v2_film_type" => matches!(value, "negative" | "positive"),
        "film_render.grain.v2_profile" => {
            value == "custom" || spektrafilm_model::grain::v2::profile_index(value).is_some()
        }
        "settings.rgb_to_raw_method" => rgb_to_raw_names().contains(&value),
        "camera.color_filter" => crate::spectral_service::is_supported_color_filter(value),
        "camera.capture_illuminant"
        | "enlarger.illuminant"
        | "film_render.convert.scan_illuminant" => {
            crate::spectral_service::is_supported_illuminant(value)
        }
        "camera.diffusion_filter.filter_family" | "enlarger.diffusion_filter.filter_family" => {
            matches!(
                value,
                "glimmerglass" | "black_pro_mist" | "pro_mist" | "cinebloom"
            )
        }
        "io.input_color_space" | "io.output_color_space" => {
            spektrafilm_math::colorspace::resolve(value).is_ok()
        }
        "io.output_gamut_compress.algorithm" => matches!(
            value,
            "off" | "aces_rgc" | "oklch" | "oklrab" | "jzazbz" | "cam16ucs"
        ),
        "io.input_gamut_compress.algorithm" => matches!(value, "xy" | "oklch" | "off"),
        "io.input_gamut_compress.boundary" => matches!(value, "locus" | "inscribed_hull"),
        "camera.auto_exposure_method" => matches!(
            value,
            "average"
                | "median"
                | "center_weighted"
                | "partial"
                | "matrix"
                | "multi_zone"
                | "highlight_weighted"
        ),
        "taps.inject" | "taps.collect" => Tap::parse(value).is_ok(),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        let options = enum_values(path)
            .map(|values| format!("; choose one of: {}", values.join(", ")))
            .unwrap_or_default();
        Err(format!("{path}: unsupported value {value:?}{options}"))
    }
}

impl super::PrintCurvesMorphParams {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        for (name, value) in [
            ("gamma_factor", self.gamma_factor),
            ("gamma_factor_fast", self.gamma_factor_fast),
            ("gamma_factor_slow", self.gamma_factor_slow),
            ("gamma_factor_red", self.gamma_factor_red),
            ("gamma_factor_green", self.gamma_factor_green),
            ("gamma_factor_blue", self.gamma_factor_blue),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!(
                    "{name} must be finite and strictly positive (got {value})"
                ));
            }
        }
        if !(0.0..=1.0).contains(&self.developer_exhaustion) {
            return Err(format!(
                "developer_exhaustion must be in [0, 1] (got {})",
                self.developer_exhaustion
            ));
        }
        Ok(())
    }
}

/// Validate the global exposed-image long edge in millimeters.
pub(crate) fn validate_film_format_mm(value: f64) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(
            "camera.film_format_mm: must be finite and strictly positive (millimeters)".into(),
        );
    }
    Ok(())
}

/// Validate cross-field runtime vocabulary before producing a pipeline artifact.
pub(super) fn validate(params: &RuntimeParams) -> Result<(), String> {
    validate_film_format_mm(f64::from(params.camera.film_format_mm))?;
    params.output_mix.validate()?;
    if params.workflow.route == REMOVED_MAGAZINE_ROUTE {
        return Err(
            "workflow.route: the magazine route was removed; use input > film > scan and set \
             magazine_print_color.active = true"
                .into(),
        );
    }
    params.validate_color()?;
    params
        .film_render
        .chemistry
        .validate()
        .map_err(|error| format!("film_render.chemistry.{error}"))?;
    params
        .print_render
        .density_curves_morph
        .validate()
        .map_err(|error| format!("print_render.density_curves_morph.{error}"))?;
    let grain = &params.film_render.grain;
    grain.fotufilm.validate()?;
    if grain.v2_profile != "custom"
        && spektrafilm_model::grain::v2::profile_index(&grain.v2_profile).is_none()
    {
        return Err(format!(
            "film_render.grain.v2_profile: unknown profile {:?}",
            grain.v2_profile
        ));
    }
    if grain.v2_resolution_type > 1 {
        return Err("film_render.grain.v2_resolution_type: must be 0 or 1".into());
    }
    validate_v2_timer(f64::from(grain.v2_timer))?;
    // Strict positivity is owned here: the metadata bound only reports a
    // discovery range (`0..=inf`).
    if !grain.v3_dye_support_um.is_finite() || grain.v3_dye_support_um <= 0.0 {
        return Err(
            "film_render.grain.v3_dye_support_um: must be finite and strictly positive (micrometers)"
                .into(),
        );
    }
    if !grain.v3_event_area_um2.is_finite() || grain.v3_event_area_um2 <= 0.0 {
        return Err(
            "film_render.grain.v3_event_area_um2: must be finite and strictly positive (square micrometers)"
                .into(),
        );
    }
    // The bound is the measured band, not the structural limit: the split
    // itself tolerates anything below one, but the record coherence above the
    // band is what the evidence attributes to the scanner, so a preset may not
    // reach it.
    let ceiling = super::grain::MEASURED_CHANNEL_CORRELATION_CEILING;
    if !grain.channel_correlation.is_finite()
        || !(0.0..=ceiling).contains(&grain.channel_correlation)
    {
        return Err(format!(
            "film_render.grain.channel_correlation: must be finite and in [0, {ceiling}]"
        ));
    }
    for (name, value) in [
        ("v2_size", grain.v2_size),
        ("v2_amount", grain.v2_amount),
        ("v2_shadows", grain.v2_shadows),
        ("v2_midtones", grain.v2_midtones),
        ("v2_highlights", grain.v2_highlights),
        ("v2_chroma", grain.v2_chroma),
        ("v2_resolution_factor", grain.v2_resolution_factor),
    ] {
        let path = format!("film_render.grain.{name}");
        let (min, max) = leaf_numeric_bounds(&path).expect("grain v2 bounds");
        if let Some(value) = value {
            let value = f64::from(value);
            if !value.is_finite() || !(min..=max.expect("grain v2 maximum")).contains(&value) {
                return Err(format!(
                    "{path}: must be finite and in {min}..={}",
                    max.unwrap()
                ));
            }
        }
    }
    let spectral_shape =
        crate::spectral_service::SpectralShape::new(params.settings.spectral_shape)
            .map_err(|e| format!("settings.spectral_shape: {e}"))?;
    if spectral_shape.bounds != crate::spectral_service::default_spectral_shape() {
        return Err(format!(
            "settings.spectral_shape: only the bundled {:?} grid is supported by the current profile/CMF/LUT contract, got {:?}",
            crate::spectral_service::default_spectral_shape(),
            spectral_shape.bounds
        ));
    }
    let calibration_values = params
        .film_render
        .convert
        .calibration
        .split(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';' | '[' | ']' | '(' | ')'))
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<f64>().map_err(|_| {
                format!("film_render.convert.calibration contains non-numeric token {part:?}")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if calibration_values.len() != 9 || calibration_values.iter().any(|value| !value.is_finite()) {
        return Err(format!(
            "film_render.convert.calibration must contain exactly 9 finite numbers, got {}",
            calibration_values.len()
        ));
    }
    if !crate::spectral_service::is_supported_color_filter(&params.camera.color_filter) {
        let supported = crate::spectral_service::available_color_filters().join(", ");
        return Err(format!(
            "camera.color_filter: unsupported filter {:?}; supported: {supported}",
            params.camera.color_filter
        ));
    }
    validate_enum_value(
        "camera.auto_exposure_method",
        &params.camera.auto_exposure_method,
    )?;
    validate_enum_value("scanner.scan_output", &params.scanner.scan_output)?;
    let scanner = &params.scanner;
    let reference =
        scanner.negative_conversion_method == super::NegativeConversionMethod::DigitalReference;
    if reference != (scanner.receiver_profile == super::ReceiverProfile::DigitalReference) {
        return Err("Digital Reference requires matching digital_reference receiver and positive interpretation".into());
    }
    for value in
        std::iter::once(&scanner.reference_exposure_ev).chain(scanner.reference_balance_ev.iter())
    {
        if !value.is_finite() || !(-16.0..=16.0).contains(value) {
            return Err("scanner.reference_exposure_ev and reference_balance_ev: must be finite and in -16..=16 stops".into());
        }
    }
    if reference {
        if params.workflow.route_enum() != WorkflowRoute::FilmScan {
            return Err("Digital Reference requires input > film > scan and scanner.scan_output=negative_to_positive".into());
        }
        if scanner.scan_output != "negative_to_positive" {
            return Err("Digital Reference requires input > film > scan and scanner.scan_output=negative_to_positive".into());
        }
        if scanner.auto_setup != super::ScannerAutoSetup::Off {
            return Err(
                "Digital Reference Reference Exposure requires scanner.auto_setup=off".into(),
            );
        }
        if scanner.black_point != [0.0; 3] || scanner.white_point != [1.0; 3] {
            return Err("Digital Reference requires identity scanner black/white points".into());
        }
        let defaults = super::ScannerParams::default();
        if scanner.conversion_brightness_ev != defaults.conversion_brightness_ev
            || scanner.conversion_balance_ev != defaults.conversion_balance_ev
            || scanner.conversion_contrast != defaults.conversion_contrast
            || scanner.conversion_dmax != defaults.conversion_dmax
        {
            return Err("Digital Reference requires default reconstruction controls; use reference_exposure_ev and reference_balance_ev".into());
        }
    } else if scanner.reference_exposure_ev != 0.0 || scanner.reference_balance_ev != [0.0; 3] {
        return Err(
            "scanner.reference_exposure_ev and reference_balance_ev require Digital Reference"
                .into(),
        );
    }
    if !scanner.finish_strength.is_finite() || !(0.0..=1.0).contains(&scanner.finish_strength) {
        return Err("scanner.finish_strength: must be finite and in 0..=1".into());
    }
    if scanner.receiver_profile != super::ReceiverProfile::LegacyColorimetric
        && !matches!(
            params.workflow.route.as_str(),
            "input > film > scan"
                | "input > convert-film > scan"
                | "input > convert-film > scan-minus-base"
        )
    {
        return Err("scanner.receiver_profile=lab_scan_approx requires a film scan route".into());
    }
    if scanner.scan_output != "negative_to_positive" {
        for (path, enabled) in [
            (
                "scanner.negative_conversion_method=coupled",
                scanner.negative_conversion_method == super::NegativeConversionMethod::Coupled,
            ),
            (
                "scanner.auto_setup=per_frame",
                scanner.auto_setup == super::ScannerAutoSetup::PerFrame,
            ),
            (
                "scanner.finish=lab_scan",
                scanner.finish == super::ScannerFinish::LabScan,
            ),
        ] {
            if enabled {
                return Err(format!(
                    "{path} requires scanner.scan_output=negative_to_positive"
                ));
            }
        }
    }
    for channel in 0..3 {
        let black = scanner.black_point[channel];
        let white = scanner.white_point[channel];
        if !black.is_finite()
            || !white.is_finite()
            || !(0.0..=1.0).contains(&black)
            || !(0.0..=1.0).contains(&white)
            || black >= white
        {
            return Err(format!(
                "scanner.black_point[{channel}] and scanner.white_point[{channel}]: must be finite and satisfy 0 <= black_point < white_point <= 1"
            ));
        }
    }
    if !scanner.conversion_brightness_ev.is_finite()
        || !(-16.0..=16.0).contains(&scanner.conversion_brightness_ev)
    {
        return Err("scanner.conversion_brightness_ev: must be finite and in -16..=16".into());
    }
    for (channel, value) in scanner.conversion_balance_ev.iter().enumerate() {
        if !value.is_finite() || !(-16.0..=16.0).contains(value) {
            return Err(format!(
                "scanner.conversion_balance_ev[{channel}]: must be finite and in -16..=16"
            ));
        }
    }
    for (name, value) in [
        ("scanner.conversion_contrast", scanner.conversion_contrast),
        ("scanner.conversion_dmax", scanner.conversion_dmax),
    ] {
        if !value.is_finite() || value <= 0.0 || value > 16.0 {
            return Err(format!(
                "{name}: must be finite, strictly positive, and <= 16"
            ));
        }
    }
    RgbToRawMethod::parse(&params.settings.rgb_to_raw_method)?;
    if let Some(capture_illuminant) = params.camera.capture_illuminant.as_deref()
        && !crate::spectral_service::is_supported_illuminant(capture_illuminant)
    {
        let supported = crate::spectral_service::available_illuminants().join(", ");
        return Err(format!(
            "camera.capture_illuminant: unsupported illuminant {capture_illuminant:?}; \
             supported: {supported}"
        ));
    }
    if !crate::spectral_service::is_supported_illuminant(&params.enlarger.illuminant) {
        let supported = crate::spectral_service::available_illuminants().join(", ");
        return Err(format!(
            "enlarger.illuminant: unsupported illuminant {:?}; supported: {supported}",
            params.enlarger.illuminant
        ));
    }
    if !crate::spectral_service::is_supported_illuminant(
        &params.film_render.convert.scan_illuminant,
    ) {
        let supported = crate::spectral_service::available_illuminants().join(", ");
        return Err(format!(
            "film_render.convert.scan_illuminant: unsupported illuminant {:?}; supported: {supported}",
            params.film_render.convert.scan_illuminant
        ));
    }
    const FILTER_FAMILIES: &str = "glimmerglass, black_pro_mist, pro_mist, cinebloom";
    for (label, df) in [
        (
            "camera.diffusion_filter.filter_family",
            &params.camera.diffusion_filter,
        ),
        (
            "enlarger.diffusion_filter.filter_family",
            &params.enlarger.diffusion_filter,
        ),
    ] {
        if df.active
            && df.strength > 0.0
            && df.spatial_scale > 0.0
            && !matches!(
                df.filter_family.as_str(),
                "glimmerglass" | "black_pro_mist" | "pro_mist" | "cinebloom"
            )
        {
            return Err(format!(
                "{label}: unknown diffusion filter family {:?}; available: \
                 {FILTER_FAMILIES}",
                df.filter_family
            ));
        }
    }
    RgbToRawMethod::parse(&params.settings.rgb_to_raw_method)?;
    let magazine = &params.magazine_print_color;
    let (min, max) = leaf_numeric_bounds("magazine_print_color.strength").unwrap();
    if !magazine.strength.is_finite() || !(min..=max.unwrap()).contains(&magazine.strength) {
        return Err("magazine_print_color.strength: must be finite and in 0..=1".into());
    }
    if let Some(t) = params.taps.inject.as_deref() {
        Tap::parse(t).map_err(|e| format!("taps.inject: {e}"))?;
    }
    if !SUPPORTED_ROUTES.contains(&params.workflow.route.as_str()) {
        return Err(format!(
            "workflow.route: unsupported route {:?}; {}",
            params.workflow.route,
            error_suffix(
                &closest(
                    &params.workflow.route,
                    SUPPORTED_ROUTES.iter().map(|route| (*route).to_owned()),
                    MAX_SUGGESTIONS
                ),
                "use spektrafilm process --help to discover supported routes"
            )
        ));
    }
    if let Some(t) = params.taps.collect.as_deref() {
        Tap::parse(t).map_err(|e| format!("taps.collect: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_routes_remain_valid() {
        let mut params = RuntimeParams::default();
        for route in SUPPORTED_ROUTES {
            params.workflow.route = route.to_owned();
            validate(&params).unwrap();
        }
    }

    #[test]
    fn unknown_routes_suggest_supported_routes_or_help() {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scna".into();
        let error = validate(&params).unwrap_err();
        assert!(error.starts_with("workflow.route: unsupported route"));
        assert!(error.contains("did you mean input > film > scan?"));
        params.workflow.route = "unrelated".into();
        let error = validate(&params).unwrap_err();
        assert!(error.contains("spektrafilm process --help"));
        assert!(!error.contains("did you mean"));
    }

    #[test]
    fn camera_metering_method_must_match_the_dispatch_vocabulary() {
        for method in enum_values("camera.auto_exposure_method").unwrap() {
            let mut params = RuntimeParams::default();
            params.camera.auto_exposure = true;
            params.camera.auto_exposure_method = method;
            validate(&params).unwrap();
        }
        let mut params = RuntimeParams::default();
        params.camera.auto_exposure_method = "typo".into();
        let error = validate(&params).unwrap_err();
        assert!(error.starts_with("camera.auto_exposure_method:"));
        assert!(error.contains("average"));
    }
}
