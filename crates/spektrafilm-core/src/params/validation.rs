use super::{RuntimeParams, Tap};

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
        "film_render.grain.v2_size" => Some((1.0, Some(48.0))),
        "film_render.grain.v2_amount"
        | "film_render.grain.v2_shadows"
        | "film_render.grain.v2_midtones"
        | "film_render.grain.v2_highlights"
        | "film_render.grain.v2_chroma"
        | "film_render.grain.v2_resolution_factor" => Some((0.0, Some(100.0))),
        "film_render.grain.v2_resolution_type" => Some((0.0, Some(1.0))),
        "film_render.grain.v2_timer" => Some((0.0, Some(1.0))),
        // Discovery range only; `validate` owns the strict positivity.
        "film_render.grain.v3_dye_support_um" => Some((0.0, None)),
        "io.input_gamut_compress.hull_detail" => Some((0.0, None)),
        "magazine_print_color.strength" => Some((0.0, Some(1.0))),
        _ => None,
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
        "io.input_gamut_compress.knee"
        | "io.output_gamut_compress.knee"
        | "io.output_gamut_compress.lightness_compression" => Some(&GAMUT_COMPONENT_DOMAINS),
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

/// Grain V2 timer override sentinel: 0 selects the seed-derived phase;
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
        "scanner.scan_output" => ["direct_scan", "positive_scan"]
            .iter()
            .map(|v| (*v).into())
            .collect(),
        "film_render.grain.engine" => ["v1", "v2", "v3"].iter().map(|v| (*v).into()).collect(),
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
        "enlarger.illuminant" | "film_render.convert.scan_illuminant" => {
            crate::spectral_service::available_illuminants()
                .iter()
                .map(|v| (*v).to_owned())
                .chain(std::iter::once("BB<kelvin> (1667..=25000)".into()))
                .collect()
        }
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
        "scanner.scan_output" => matches!(value, "direct_scan" | "positive_scan"),
        "film_render.grain.engine" => matches!(value, "v1" | "v2" | "v3"),
        "film_render.grain.v2_mode" => matches!(value, "analogue" | "noise"),
        "film_render.grain.v2_film_type" => matches!(value, "negative" | "positive"),
        "film_render.grain.v2_profile" => {
            value == "custom" || spektrafilm_model::grain::v2::profile_index(value).is_some()
        }
        "settings.rgb_to_raw_method" => rgb_to_raw_names().contains(&value),
        "camera.color_filter" => crate::spectral_service::is_supported_color_filter(value),
        "enlarger.illuminant" | "film_render.convert.scan_illuminant" => {
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
        Err(format!("{path}: unsupported value {value:?}"))
    }
}

/// Validate wire-level enum values and cross-field constraints before any
/// pipeline artifact is produced. The schema module owns data/defaults;
/// this module owns the accepted runtime vocabulary.
pub(super) fn validate(params: &RuntimeParams) -> Result<(), String> {
    params.validate_color()?;
    let grain = &params.film_render.grain;
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
    validate_enum_value("scanner.scan_output", &params.scanner.scan_output)?;
    RgbToRawMethod::parse(&params.settings.rgb_to_raw_method)?;
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
    if !matches!(
        params.workflow.route.as_str(),
        "input"
            | "input > film > scan"
            | "input > film > print > scan"
            | "input > film > scan > magazine"
            | "input > convert-film > print > scan"
            | "input > convert-film > scan-minus-base"
            | "input > convert-film > scan"
    ) {
        return Err(format!(
            "workflow.route: unsupported route {:?}",
            params.workflow.route
        ));
    }
    if let Some(t) = params.taps.collect.as_deref() {
        Tap::parse(t).map_err(|e| format!("taps.collect: {e}"))?;
    }
    Ok(())
}
