use super::{RuntimeParams, Tap};
use crate::suggest::{MAX_SUGGESTIONS, closest, error_suffix};

/// Semantic runtime validation failure, independent of any presentation locale.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub reason: ValidationReason,
    pub path: String,
    pub arguments: Vec<(&'static str, String)>,
    english: String,
}

impl ValidationError {
    pub(crate) fn new(
        reason: ValidationReason,
        path: impl Into<String>,
        english: impl Into<String>,
    ) -> Self {
        Self {
            reason,
            path: path.into(),
            arguments: Vec::new(),
            english: english.into(),
        }
    }

    pub(crate) fn arg(mut self, name: &'static str, value: impl ToString) -> Self {
        self.arguments.push((name, value.to_string()));
        self
    }

    fn prefix(mut self, prefix: &str) -> Self {
        self.path = format!("{prefix}{}", self.path);
        self.english = format!("{prefix}{}", self.english);
        self
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.english.fmt(f)
    }
}
impl std::error::Error for ValidationError {}

/// Reasons are emitted at the authoritative failing rule, never parsed from prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationReason {
    ComponentRange,
    SharesSum,
    GamutUnitInterval,
    GamutPositive,
    Timer,
    UnsupportedValue,
    FinitePositive,
    Finite,
    UnitInterval,
    FilmFormat,
    RemovedRoute,
    UnknownProfile,
    Binary,
    DyeSupport,
    EventArea,
    Correlation,
    NumericRange,
    SpectralBounds,
    SpectralIntervals,
    SpectralBundled,
    CalibrationToken,
    CalibrationCount,
    ColorFilter,
    OperatorMode,
    ScanRoute,
    StandardLut,
    StyleName,
    StyleMode,
    SourceCompression,
    RgbMethod,
    Illuminant,
    ScanIlluminant,
    DiffusionFamily,
    CmykStrength,
    Tap,
    Route,
    OutputMix,
    ColorSpace,
    InputAlgorithm,
    InputBoundary,
    HullDetail,
    OutputAlgorithm,
    LegacyGroupObject,
    LegacyConflict,
    LegacyRouteConflict,
    LegacyLamp,
    ObjectRequired,
    LegacyExposureConflict,
    LegacyExposureBoolean,
    LegacyExposureMixed,
    LegacySettingsObject,
    RetiredLightness,
    DocumentObject,
    InvalidField,
    UnknownGroup,
    NullNotAllowed,
    NonFinite,
    LeafRequired,
    TypeMismatch,
    F32Range,
    F32Underflow,
    Representation,
    GamutRepresentation,
    OutsideRange,
    RetiredPath,
    NonSettable,
    UnknownPath,
    GroupAssignment,
    NumericFinite,
    ArrayFinite,
    SerializeParameters,
    ApplyParameters,
    LegacyProvenance,
    NativeJson,
    NativeToml,
    DuplicateJsonKey,
    TomlNonFinite,
    TomlDatetime,
    ScanNegativeProfile,
    ScanFiniteDensity,
    ScanProceduralGrain,
}

use ValidationReason as Reason;

pub(crate) const SUPPORTED_ROUTES: [&str; 12] = [
    "input",
    "input > film > scan",
    "input > film > print > scan",
    "input > convert-film > print > scan",
    "input > convert-film > scan-minus-base",
    "input > convert-film > scan",
    "input > film",
    "input > film > print",
    "film > scan",
    "film > print",
    "film > print > scan",
    "print > scan",
];

/// The removed dedicated magazine workflow. Saved configurations that still
/// name it must be migrated to a base workflow plus `cmyk.active`; it is not
/// a selectable route. The constant name keeps the historical spelling of
/// the retired route.
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
        Self::parse_detailed(value).map_err(|error| error.to_string())
    }

    fn parse_detailed(value: &str) -> Result<Self, ValidationError> {
        let method = match value {
            "hanatos2025" => Self::Hanatos2025,
            "mallett2019" => Self::Mallett2019,
            "arctic2026alpha02" => Self::Arctic2026Alpha02,
            "arctic2026beta04" => Self::Arctic2026Beta04,
            "gauss-lasers" => Self::GaussLasers,
            "jakob2019" => Self::Jakob2019,
            "otsu2018" => Self::Otsu2018,
            other => {
                let supported = rgb_to_raw_names().join(", ");
                return Err(ValidationError::new(Reason::RgbMethod, "settings.rgb_to_raw_method", format!(
                    "settings.rgb_to_raw_method: unsupported method {other:?}; supported: {supported}"
                )).arg("value", format!("{other:?}")).arg("supported", supported));
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
        "scanner.operator.contrast" => Some((0.0, None)),
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
        "film_render.halation.source_compression" => Some((0.0, Some(0.999999))),
        "camera.film_format_mm" => Some((0.0, None)),
        "io.input_gamut_compress.hull_detail" => Some((0.0, None)),
        "cmyk.strength" => Some((0.0, Some(1.0))),
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
pub(crate) fn validate_fotufilm_values_detailed(
    path: &str,
    values: &[f64],
) -> Result<(), ValidationError> {
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
            return Err(ValidationError::new(Reason::ComponentRange, path, format!(
                "{path}: component {index} must be finite and in {lower}{min}, {max}] (got {value})"
            )).arg("index", index).arg("lower", lower).arg("min", min).arg("max", max).arg("value", value));
        }
    }
    if path == "film_render.grain.fotufilm.sublayer_shares"
        && (values.iter().sum::<f64>() - 1.0).abs() > 1e-6
    {
        return Err(ValidationError::new(
            Reason::SharesSum,
            path,
            format!("{path}: shares must sum to 1 within 1e-6; no automatic normalization"),
        ));
    }
    Ok(())
}

impl super::FotufilmGrainParams {
    pub(crate) fn validate_detailed(&self) -> Result<(), ValidationError> {
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
            validate_fotufilm_values_detailed(path, values)?;
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

pub(crate) fn validate_gamut_triplet_detailed(
    name: &str,
    values: [f32; 3],
) -> Result<(f64, f64, f64), ValidationError> {
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
            let reason = if domain.maximum.is_some() {
                Reason::GamutUnitInterval
            } else {
                Reason::GamutPositive
            };
            return Err(ValidationError::new(
                reason,
                name,
                format!(
                    "{name} {} must be {requirement}, got {value}",
                    domain.component
                ),
            )
            .arg("component", domain.component)
            .arg("value", value));
        }
    }
    let [threshold, limit, power] = values.map(f64::from);
    Ok((threshold, limit, power))
}

/// Grain Procedural timer override sentinel: 0 selects the seed-derived phase;
/// otherwise the phase must be finite and strictly below 1.
pub(crate) fn validate_v2_timer_detailed(value: f64) -> Result<(), ValidationError> {
    if value != 0.0 && (!value.is_finite() || !(0.0..1.0).contains(&value)) {
        return Err(ValidationError::new(
            Reason::Timer,
            "film_render.grain.v2_timer",
            "film_render.grain.v2_timer: must be 0 or finite in (0,1)",
        )
        .arg("value", value));
    }
    Ok(())
}
/// Discovery vocabulary for a leaf. Pseudo-tokens document patterns that are
/// validated but not enumerable (for example `BB<kelvin>` blackbody sources).
pub(crate) fn enum_values(path: &str) -> Option<Vec<String>> {
    let values: Vec<String> = match path {
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
    validate_enum_value_detailed(path, value).map_err(|error| error.to_string())
}

pub(crate) fn validate_enum_value_detailed(path: &str, value: &str) -> Result<(), ValidationError> {
    let valid = match path {
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
        let supported = enum_values(path)
            .map(|values| values.join(", "))
            .unwrap_or_default();
        let options = if supported.is_empty() {
            String::new()
        } else {
            format!("; choose one of: {supported}")
        };
        Err(ValidationError::new(
            Reason::UnsupportedValue,
            path,
            format!("{path}: unsupported value {value:?}{options}"),
        )
        .arg("value", format!("{value:?}"))
        .arg("supported", supported))
    }
}

/// One numeric rule shared by typed runtime and sparse parameter sources.
pub(crate) fn validate_scan_operator_value_detailed(
    path: &str,
    value: f64,
) -> Result<(), ValidationError> {
    if !value.is_finite() || (path == "scanner.operator.contrast" && value <= 0.0) {
        let domain = if path == "scanner.operator.contrast" {
            "finite and strictly positive"
        } else {
            "finite"
        };
        let reason = if path == "scanner.operator.contrast" {
            Reason::FinitePositive
        } else {
            Reason::Finite
        };
        return Err(ValidationError::new(
            reason,
            path,
            format!("{path}: must be {domain} (got {value})"),
        )
        .arg("value", value));
    }
    Ok(())
}

impl super::ScanOperatorParams {
    pub fn validate(&self) -> Result<(), String> {
        self.validate_detailed().map_err(|error| error.to_string())
    }

    pub fn validate_detailed(&self) -> Result<(), ValidationError> {
        for (path, value) in [
            ("scanner.operator.exposure_ev", self.exposure_ev),
            ("scanner.operator.warmth_ev", self.warmth_ev),
            ("scanner.operator.tint_ev", self.tint_ev),
            ("scanner.operator.contrast", self.contrast),
        ] {
            validate_scan_operator_value_detailed(path, value)?;
        }
        Ok(())
    }
}

impl super::PrintCurvesMorphParams {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.validate_detailed().map_err(|error| error.to_string())
    }

    fn validate_detailed(&self) -> Result<(), ValidationError> {
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
                return Err(ValidationError::new(
                    Reason::FinitePositive,
                    name,
                    format!("{name} must be finite and strictly positive (got {value})"),
                )
                .arg("value", value));
            }
        }
        if !(0.0..=1.0).contains(&self.developer_exhaustion) {
            return Err(ValidationError::new(
                Reason::UnitInterval,
                "developer_exhaustion",
                format!(
                    "developer_exhaustion must be in [0, 1] (got {})",
                    self.developer_exhaustion
                ),
            )
            .arg("value", self.developer_exhaustion));
        }
        Ok(())
    }
}

/// Validate the global exposed-image long edge in millimeters.
pub(crate) fn validate_film_format_mm_detailed(value: f64) -> Result<(), ValidationError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(ValidationError::new(
            Reason::FilmFormat,
            "camera.film_format_mm",
            "camera.film_format_mm: must be finite and strictly positive (millimeters)",
        )
        .arg("value", value));
    }
    Ok(())
}

/// Validate cross-field runtime vocabulary before producing a pipeline artifact.
pub(super) fn validate(params: &RuntimeParams) -> Result<(), String> {
    validate_detailed(params).map_err(|error| error.to_string())
}

pub(super) fn validate_detailed(params: &RuntimeParams) -> Result<(), ValidationError> {
    validate_film_format_mm_detailed(f64::from(params.camera.film_format_mm))?;
    params.output_mix.validate_detailed()?;
    if params.workflow.route == REMOVED_MAGAZINE_ROUTE {
        return Err(ValidationError::new(
            Reason::RemovedRoute,
            "workflow.route",
            "workflow.route: the magazine route was removed; use input > film > scan and set cmyk.active = true",
        ));
    }
    params.validate_color_detailed()?;
    params
        .film_render
        .chemistry
        .validate_detailed()
        .map_err(|error| error.prefix("film_render.chemistry."))?;
    params
        .print_render
        .density_curves_morph
        .validate_detailed()
        .map_err(|error| error.prefix("print_render.density_curves_morph."))?;
    let grain = &params.film_render.grain;
    grain.fotufilm.validate_detailed()?;
    if grain.v2_profile != "custom"
        && spektrafilm_model::grain::v2::profile_index(&grain.v2_profile).is_none()
    {
        return Err(ValidationError::new(
            Reason::UnknownProfile,
            "film_render.grain.v2_profile",
            format!(
                "film_render.grain.v2_profile: unknown profile {:?}",
                grain.v2_profile
            ),
        )
        .arg("value", format!("{:?}", grain.v2_profile)));
    }
    if grain.v2_resolution_type > 1 {
        return Err(ValidationError::new(
            Reason::Binary,
            "film_render.grain.v2_resolution_type",
            "film_render.grain.v2_resolution_type: must be 0 or 1",
        )
        .arg("value", grain.v2_resolution_type));
    }
    validate_v2_timer_detailed(f64::from(grain.v2_timer))?;
    // Strict positivity is owned here: the metadata bound only reports a
    // discovery range (`0..=inf`).
    if !grain.v3_dye_support_um.is_finite() || grain.v3_dye_support_um <= 0.0 {
        return Err(ValidationError::new(Reason::DyeSupport, "film_render.grain.v3_dye_support_um",
            "film_render.grain.v3_dye_support_um: must be finite and strictly positive (micrometers)").arg("value", grain.v3_dye_support_um));
    }
    if !grain.v3_event_area_um2.is_finite() || grain.v3_event_area_um2 <= 0.0 {
        return Err(ValidationError::new(Reason::EventArea, "film_render.grain.v3_event_area_um2",
            "film_render.grain.v3_event_area_um2: must be finite and strictly positive (square micrometers)").arg("value", grain.v3_event_area_um2));
    }
    // The bound is the measured band, not the structural limit: the split
    // itself tolerates anything below one, but the record coherence above the
    // band is what the evidence attributes to the scanner, so a preset may not
    // reach it.
    let ceiling = super::grain::MEASURED_CHANNEL_CORRELATION_CEILING;
    if !grain.channel_correlation.is_finite()
        || !(0.0..=ceiling).contains(&grain.channel_correlation)
    {
        return Err(ValidationError::new(
            Reason::Correlation,
            "film_render.grain.channel_correlation",
            format!("film_render.grain.channel_correlation: must be finite and in [0, {ceiling}]"),
        )
        .arg("max", ceiling)
        .arg("value", grain.channel_correlation));
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
                return Err(ValidationError::new(
                    Reason::NumericRange,
                    &path,
                    format!("{path}: must be finite and in {min}..={}", max.unwrap()),
                )
                .arg("min", min)
                .arg("max", max.unwrap())
                .arg("value", value));
            }
        }
    }
    let spectral_shape =
        crate::spectral_service::SpectralShape::new_detailed(params.settings.spectral_shape)
            .map_err(|error| {
                let mut error = error.prefix("settings.spectral_shape: ");
                error.path = "settings.spectral_shape".into();
                error
            })?;
    if spectral_shape.bounds != crate::spectral_service::default_spectral_shape() {
        let expected = crate::spectral_service::default_spectral_shape();
        return Err(ValidationError::new(Reason::SpectralBundled, "settings.spectral_shape", format!(
            "settings.spectral_shape: only the bundled {expected:?} grid is supported by the current profile/CMF/LUT contract, got {:?}", spectral_shape.bounds
        )).arg("expected", format!("{expected:?}")).arg("value", format!("{:?}", spectral_shape.bounds)));
    }
    let calibration_values = params
        .film_render
        .convert
        .calibration
        .split(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';' | '[' | ']' | '(' | ')'))
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<f64>().map_err(|_| {
                ValidationError::new(
                    Reason::CalibrationToken,
                    "film_render.convert.calibration",
                    format!("film_render.convert.calibration contains non-numeric token {part:?}"),
                )
                .arg("value", format!("{part:?}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if calibration_values.len() != 9 || calibration_values.iter().any(|value| !value.is_finite()) {
        return Err(ValidationError::new(
            Reason::CalibrationCount,
            "film_render.convert.calibration",
            format!(
                "film_render.convert.calibration must contain exactly 9 finite numbers, got {}",
                calibration_values.len()
            ),
        )
        .arg("count", calibration_values.len())
        .arg("value", &params.film_render.convert.calibration));
    }
    if !crate::spectral_service::is_supported_color_filter(&params.camera.color_filter) {
        let supported = crate::spectral_service::available_color_filters().join(", ");
        return Err(ValidationError::new(
            Reason::ColorFilter,
            "camera.color_filter",
            format!(
                "camera.color_filter: unsupported filter {:?}; supported: {supported}",
                params.camera.color_filter
            ),
        )
        .arg("value", format!("{:?}", params.camera.color_filter))
        .arg("supported", supported));
    }
    validate_enum_value_detailed(
        "camera.auto_exposure_method",
        &params.camera.auto_exposure_method,
    )?;
    validate_enum_value_detailed("scanner.scan_output", &params.scanner.scan_output)?;
    params.scanner.operator.validate_detailed()?;
    if !params.scanner.operator.is_default() {
        if params.scanner.scan_output != "negative_to_positive" || !params.scanner.uses_standard() {
            return Err(ValidationError::new(
                Reason::OperatorMode,
                "scanner.operator",
                "scanner.operator: nondefault corrections require scanner.scan_output=negative_to_positive and scanner.conversion_style=standard",
            ));
        }
        crate::pipeline_calibration::validate_scan_output_route_detailed(params)?;
    }
    if params.debug.lut_mode
        && params.scanner.scan_output == "negative_to_positive"
        && params.scanner.uses_standard()
    {
        return Err(ValidationError::new(
            Reason::StandardLut,
            "scanner.conversion_style",
            "scanner.conversion_style=standard measures the whole frame and cannot be baked into a point-wise LUT; select a declared fixed conversion style",
        ));
    }
    let style = &params.scanner.conversion_style;
    if style.is_empty()
        || style.contains('/')
        || style.contains('\\')
        || style.contains("..")
        || style.contains(':')
    {
        return Err(ValidationError::new(
            Reason::StyleName,
            "scanner.conversion_style",
            format!("scanner.conversion_style: {style:?} is not a valid style name"),
        )
        .arg("value", format!("{style:?}")));
    }
    if !params.scanner.uses_standard() && params.scanner.scan_output != "negative_to_positive" {
        return Err(ValidationError::new(
            Reason::StyleMode,
            "scanner.conversion_style",
            "scanner.conversion_style requires scanner.scan_output=negative_to_positive",
        ));
    }
    if !params.film_render.halation.source_compression.is_finite()
        || !(0.0..=0.999999).contains(&params.film_render.halation.source_compression)
    {
        return Err(ValidationError::new(
            Reason::SourceCompression,
            "film_render.halation.source_compression",
            "film_render.halation.source_compression: must be finite and in 0..=0.999999",
        )
        .arg("value", params.film_render.halation.source_compression));
    }
    RgbToRawMethod::parse_detailed(&params.settings.rgb_to_raw_method)?;
    if let Some(capture_illuminant) = params.camera.capture_illuminant.as_deref()
        && !crate::spectral_service::is_supported_illuminant(capture_illuminant)
    {
        let supported = crate::spectral_service::available_illuminants().join(", ");
        return Err(ValidationError::new(Reason::Illuminant, "camera.capture_illuminant", format!(
            "camera.capture_illuminant: unsupported illuminant {capture_illuminant:?}; supported: {supported}"
        )).arg("value", format!("{capture_illuminant:?}")).arg("supported", supported));
    }
    if !crate::spectral_service::is_supported_illuminant(
        &params.film_render.convert.scan_illuminant,
    ) {
        return Err(ValidationError::new(
            Reason::ScanIlluminant,
            "film_render.convert.scan_illuminant",
            format!(
                "film_render.convert.scan_illuminant: unsupported capture illuminant {:?}",
                params.film_render.convert.scan_illuminant
            ),
        )
        .arg(
            "value",
            format!("{:?}", params.film_render.convert.scan_illuminant),
        ));
    }
    if !crate::spectral_service::is_supported_illuminant(&params.enlarger.illuminant) {
        let supported = crate::spectral_service::available_illuminants().join(", ");
        return Err(ValidationError::new(
            Reason::Illuminant,
            "enlarger.illuminant",
            format!(
                "enlarger.illuminant: unsupported illuminant {:?}; supported: {supported}",
                params.enlarger.illuminant
            ),
        )
        .arg("value", format!("{:?}", params.enlarger.illuminant))
        .arg("supported", supported));
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
            return Err(ValidationError::new(
                Reason::DiffusionFamily,
                label,
                format!(
                    "{label}: unknown diffusion filter family {:?}; available: {FILTER_FAMILIES}",
                    df.filter_family
                ),
            )
            .arg("value", format!("{:?}", df.filter_family))
            .arg("supported", FILTER_FAMILIES));
        }
    }
    RgbToRawMethod::parse_detailed(&params.settings.rgb_to_raw_method)?;
    let cmyk = &params.cmyk;
    let (min, max) = leaf_numeric_bounds("cmyk.strength").unwrap();
    if !cmyk.strength.is_finite() || !(min..=max.unwrap()).contains(&cmyk.strength) {
        return Err(ValidationError::new(
            Reason::CmykStrength,
            "cmyk.strength",
            "cmyk.strength: must be finite and in 0..=1",
        )
        .arg("value", cmyk.strength));
    }
    if let Some(t) = params.taps.inject.as_deref() {
        Tap::parse(t).map_err(|error| {
            ValidationError::new(Reason::Tap, "taps.inject", format!("taps.inject: {error}"))
                .arg("value", format!("{t:?}"))
        })?;
    }
    if params.workflow.route_enum() == crate::WorkflowRoute::Custom {
        let suggestions = closest(
            &params.workflow.route,
            SUPPORTED_ROUTES.iter().map(|route| (*route).to_owned()),
            MAX_SUGGESTIONS,
        );
        let suffix = error_suffix(
            &suggestions,
            "use spektrafilm process --help to discover supported routes",
        );
        return Err(ValidationError::new(
            Reason::Route,
            "workflow.route",
            format!(
                "workflow.route: unsupported route {:?}; {suffix}",
                params.workflow.route
            ),
        )
        .arg("value", format!("{:?}", params.workflow.route))
        .arg("supported", SUPPORTED_ROUTES.join(", "))
        .arg("suggestions", suggestions.join(", ")));
    }
    if let Some(t) = params.taps.collect.as_deref() {
        Tap::parse(t).map_err(|error| {
            ValidationError::new(
                Reason::Tap,
                "taps.collect",
                format!("taps.collect: {error}"),
            )
            .arg("value", format!("{t:?}"))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    type ValidationCase = (fn(&mut RuntimeParams), Reason, &'static str);

    #[test]
    fn semantic_failures_retain_canonical_paths_and_english_adapter() {
        let cases: &[ValidationCase] = &[
            (
                |p| p.camera.film_format_mm = 0.0,
                Reason::FilmFormat,
                "camera.film_format_mm",
            ),
            (
                |p| p.output_mix.amount = -1.0,
                Reason::OutputMix,
                "output_mix.amount",
            ),
            (
                |p| p.io.input_color_space = "unknown".into(),
                Reason::ColorSpace,
                "io.input_color_space",
            ),
            (
                |p| p.io.input_gamut_compress.hull_detail = 0.0,
                Reason::HullDetail,
                "io.input_gamut_compress.hull_detail",
            ),
            (
                |p| p.io.output_gamut_compress.knee[0] = 1.0,
                Reason::GamutUnitInterval,
                "io.output_gamut_compress.knee",
            ),
            (
                |p| p.film_render.chemistry.gamma_factor = 0.0,
                Reason::FinitePositive,
                "film_render.chemistry.gamma_factor",
            ),
            (
                |p| p.film_render.grain.fotufilm.amount = -1.0,
                Reason::ComponentRange,
                "film_render.grain.fotufilm.amount",
            ),
            (
                |p| p.settings.spectral_shape = [0.0, 780.0, 5.0],
                Reason::SpectralBounds,
                "settings.spectral_shape",
            ),
            (
                |p| p.settings.spectral_shape = [380.0, 780.0, 3.0],
                Reason::SpectralIntervals,
                "settings.spectral_shape",
            ),
            (
                |p| p.film_render.convert.calibration = "1 2 3".into(),
                Reason::CalibrationCount,
                "film_render.convert.calibration",
            ),
            (
                |p| p.film_render.convert.calibration = "not-a-number".into(),
                Reason::CalibrationToken,
                "film_render.convert.calibration",
            ),
            (
                |p| p.scanner.operator.contrast = 0.0,
                Reason::FinitePositive,
                "scanner.operator.contrast",
            ),
            (
                |p| p.settings.rgb_to_raw_method = "unknown".into(),
                Reason::RgbMethod,
                "settings.rgb_to_raw_method",
            ),
            (
                |p| p.taps.inject = Some("unknown".into()),
                Reason::Tap,
                "taps.inject",
            ),
            (
                |p| p.workflow.route = "unknown".into(),
                Reason::Route,
                "workflow.route",
            ),
        ];
        for &(change, reason, path) in cases {
            let mut params = RuntimeParams::default();
            change(&mut params);
            let error = params.validate_detailed().unwrap_err();
            assert_eq!(error.reason, reason);
            assert_eq!(error.path, path);
            assert_eq!(params.validate().unwrap_err(), error.to_string());
        }
    }

    #[test]
    fn calibration_error_keeps_numeric_count_without_parsing_prose() {
        let mut params = RuntimeParams::default();
        params.film_render.convert.calibration = "1 2 3".into();
        let error = params.validate_detailed().unwrap_err();
        assert_eq!(error.reason, Reason::CalibrationCount);
        assert_eq!(
            error.arguments,
            vec![("count", "3".to_owned()), ("value", "1 2 3".to_owned())]
        );
        assert_eq!(
            error.to_string(),
            "film_render.convert.calibration must contain exactly 9 finite numbers, got 3"
        );
    }

    #[test]
    fn supported_routes_remain_valid() {
        let mut params = RuntimeParams::default();
        for route in [
            "input>film",
            " input\t> film > print ",
            "film>scan",
            "film >print",
            "film>print>scan",
            " print > scan ",
        ] {
            params.workflow.route = route.into();
            validate(&params).unwrap();
        }
        for route in [
            "film",
            "print",
            "input > print",
            "input > film > film",
            "film > > scan",
            "film > scan > print",
            "Film > scan",
            "input > convert-film",
            "input > film > print > scan > film",
        ] {
            params.workflow.route = route.into();
            assert!(validate(&params).is_err(), "invalid finite route {route}");
        }
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

    #[test]
    fn cmyk_strength_bounds_are_enforced() {
        let mut params = RuntimeParams::default();
        for strength in [0.0, 0.5, 1.0] {
            params.cmyk.strength = strength;
            validate(&params).unwrap();
        }
        for strength in [-0.01, 1.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            params.cmyk.strength = strength;
            let error = validate(&params).unwrap_err();
            assert!(
                error.starts_with("cmyk.strength:"),
                "unexpected error for {strength}: {error}"
            );
        }
    }

    #[test]
    fn removed_magazine_route_rejects_with_actionable_replacement() {
        let mut params = RuntimeParams::default();
        params.workflow.route = REMOVED_MAGAZINE_ROUTE.to_owned();
        let error = validate(&params).unwrap_err();
        assert!(error.starts_with("workflow.route:"), "{error}");
        assert!(error.contains("input > film > scan"), "{error}");
        assert!(error.contains("cmyk.active = true"), "{error}");
    }
}
