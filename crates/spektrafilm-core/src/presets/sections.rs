use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Film,
    Grain,
    Halation,
    Couplers,
    Base,
    Convert,
    Camera,
    CameraFilter,
    EnlargerFilter,
    Enlarger,
    Scanner,
    Cmyk,
    Print,
    OutputMix,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "section", rename_all = "snake_case")]
pub enum PresetKind {
    Look,
    Section(Section),
}
pub const SECTIONS: &[Section] = &[
    Section::Film,
    Section::Grain,
    Section::Halation,
    Section::Couplers,
    Section::Base,
    Section::Convert,
    Section::Camera,
    Section::CameraFilter,
    Section::EnlargerFilter,
    Section::Enlarger,
    Section::Scanner,
    Section::Cmyk,
    Section::Print,
    Section::OutputMix,
];
impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Self::Film => "film",
            Self::Grain => "grain",
            Self::Halation => "halation",
            Self::Couplers => "couplers",
            Self::Base => "base",
            Self::Convert => "convert",
            Self::Camera => "camera",
            Self::CameraFilter => "camera filter",
            Self::EnlargerFilter => "enlarger filter",
            Self::Enlarger => "enlarger",
            Self::Scanner => "scanner",
            Self::Cmyk => "cmyk ink style",
            Self::Print => "print",
            Self::OutputMix => "output",
        }
    }
    pub fn paths(self) -> &'static [&'static str] {
        match self {
            Self::Film => &[
                "film_render.density_curve_gamma",
                "film_render.development_time",
                "film_render.chemistry",
            ],
            Self::Grain => &["film_render.grain"],
            Self::Halation => &["film_render.halation"],
            Self::Couplers => &["film_render.dir_couplers"],
            Self::Base => &["film_render.base"],
            Self::Convert => &["film_render.convert"],
            Self::Camera => &[
                "camera.exposure_compensation_ev",
                "camera.auto_exposure",
                "camera.auto_exposure_method",
                "camera.capture_illuminant",
            ],
            Self::CameraFilter => &["camera_diffusion_filter"],
            Self::EnlargerFilter => &["enlarger.diffusion_filter"],
            Self::Enlarger => &["enlarger"],
            Self::Scanner => &["scanner"],
            Self::Cmyk => &["cmyk"],
            Self::Print => &["print_render"],
            Self::OutputMix => &["output_mix"],
        }
    }
    fn owns(self, path: &str) -> bool {
        self.paths()
            .iter()
            .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}.")))
            && !(self == Self::Enlarger && path.starts_with("enlarger.diffusion_filter"))
    }
}
fn flatten(prefix: &str, value: &Value, out: &mut Vec<(String, Value)>) {
    if let Some(map) = value.as_object() {
        for (key, value) in map {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            flatten(&path, value, out);
        }
    } else {
        out.push((prefix.into(), value.clone()));
    }
}
pub fn extract_section(section: Section, params: &RuntimeParams) -> Result<Value, String> {
    extract_section_detailed(section, params).map_err(|error| error.to_string())
}
pub fn extract_section_detailed(
    section: Section,
    params: &RuntimeParams,
) -> Result<Value, PresetDocumentError> {
    params.validate_values_detailed()?;
    let root = serde_json::to_value(LookParameters::capture(params)).map_err(|e| {
        PresetDocumentError::new(
            PresetDocumentReason::Parser {
                detail: e.to_string(),
            },
            e.to_string(),
        )
    })?;
    let mut leaves = Vec::new();
    flatten("", &root, &mut leaves);
    let mut payload = serde_json::json!({});
    for (path, value) in leaves {
        // `monochrome` is derived from the film profile and `micro_sublayers`
        // is a no-op; neither has a control surface, so neither is carried.
        let unowned = matches!(
            path.as_str(),
            "film_render.grain.monochrome" | "film_render.grain.micro_sublayers"
        );
        if section.owns(&path) && !unowned {
            crate::params::sources::insert(&mut payload, &path, value);
        }
    }
    Ok(payload)
}
pub fn section_from_payload(payload: &Value) -> Result<Section, String> {
    section_from_payload_detailed(payload).map_err(|error| error.to_string())
}
pub(super) fn section_from_payload_detailed(
    payload: &Value,
) -> Result<Section, PresetDocumentError> {
    super::reject_unowned_preset_fields_detailed(payload)?;
    parameter_edits_detailed(payload)?;
    let mut leaves = Vec::new();
    flatten("", payload, &mut leaves);
    if leaves.is_empty() {
        return Err(PresetDocumentError::new(
            PresetDocumentReason::EmptySection,
            "section payload is empty",
        ));
    }
    SECTIONS
        .iter()
        .copied()
        .find(|section| leaves.iter().all(|(path, _)| section.owns(path)))
        .ok_or_else(|| {
            PresetDocumentError::new(
                PresetDocumentReason::MultipleSections,
                "preset payload must carry editable leaves inside exactly one section",
            )
        })
}
pub fn section_matches(preset: &Preset, params: &RuntimeParams) -> Result<bool, String> {
    section_matches_detailed(preset, params).map_err(|error| error.to_string())
}
pub fn section_matches_detailed(
    preset: &Preset,
    params: &RuntimeParams,
) -> Result<bool, PresetDocumentError> {
    let PresetKind::Section(_) = preset.kind_detailed()? else {
        return Ok(false);
    };
    let PresetParameters::Section(payload) = &preset.parameters else {
        unreachable!()
    };
    params.validate_values_detailed()?;
    let applied = preset.overlay_section_detailed(params)?;
    let expected = serde_json::to_value(LookParameters::capture(&applied)).map_err(|e| {
        PresetDocumentError::new(
            PresetDocumentReason::Parser {
                detail: e.to_string(),
            },
            e.to_string(),
        )
    })?;
    let current = serde_json::to_value(LookParameters::capture(params)).map_err(|e| {
        PresetDocumentError::new(
            PresetDocumentReason::Parser {
                detail: e.to_string(),
            },
            e.to_string(),
        )
    })?;
    let mut leaves = Vec::new();
    flatten("", payload, &mut leaves);
    Ok(leaves.iter().all(|(path, _)| {
        let pointer = format!("/{}", path.replace('.', "/"));
        current.pointer(&pointer) == expected.pointer(&pointer)
    }))
}
pub fn matching_section_preset<'a>(
    section: Section,
    presets: impl IntoIterator<Item = &'a Preset>,
    params: &RuntimeParams,
) -> Result<Option<&'a Preset>, String> {
    matching_section_preset_detailed(section, presets, params).map_err(|error| error.to_string())
}
pub fn matching_section_preset_detailed<'a>(
    section: Section,
    presets: impl IntoIterator<Item = &'a Preset>,
    params: &RuntimeParams,
) -> Result<Option<&'a Preset>, PresetDocumentError> {
    let mut matches = Vec::new();
    for preset in presets {
        if preset.kind_detailed()? == PresetKind::Section(section)
            && section_matches_detailed(preset, params)?
        {
            matches.push(preset);
        }
    }
    matches.sort_by(|a, b| name_order(&a.name, &b.name));
    Ok(matches.into_iter().next())
}
