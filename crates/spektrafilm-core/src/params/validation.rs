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
                    "settings.rgb_to_raw_method: unsupported method {:?}; supported: \
                     hanatos2025, mallett2019, arctic2026alpha02, arctic2026beta04, \
                     gauss-lasers, jakob2019, otsu2018",
                    other
                ));
            }
        };
        Ok(method)
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
    if grain.v2_timer != 0.0
        && (!grain.v2_timer.is_finite() || !(0.0..1.0).contains(&grain.v2_timer))
    {
        return Err("film_render.grain.v2_timer: must be 0 or finite in (0,1)".into());
    }
    for (name, value, min, max) in [
        ("v2_size", grain.v2_size, 1.0, 48.0),
        ("v2_amount", grain.v2_amount, 0.0, 100.0),
        ("v2_shadows", grain.v2_shadows, 0.0, 100.0),
        ("v2_midtones", grain.v2_midtones, 0.0, 100.0),
        ("v2_highlights", grain.v2_highlights, 0.0, 100.0),
        ("v2_chroma", grain.v2_chroma, 0.0, 100.0),
        (
            "v2_resolution_factor",
            grain.v2_resolution_factor,
            0.0,
            100.0,
        ),
    ] {
        if let Some(value) = value {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(format!(
                    "film_render.grain.{name}: must be finite and in {min}..={max}"
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
    if !matches!(
        params.settings.rgb_to_raw_method.as_str(),
        "hanatos2025"
            | "mallett2019"
            | "arctic2026alpha02"
            | "arctic2026beta04"
            | "gauss-lasers"
            | "jakob2019"
            | "otsu2018"
    ) {
        return Err(format!(
            "settings.rgb_to_raw_method: unsupported method {:?}; supported: \
             hanatos2025, mallett2019, arctic2026alpha02, arctic2026beta04, \
             gauss-lasers, jakob2019, otsu2018",
            params.settings.rgb_to_raw_method
        ));
    }
    let magazine = &params.magazine_print_color;
    if !magazine.strength.is_finite() || !(0.0..=1.0).contains(&magazine.strength) {
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
