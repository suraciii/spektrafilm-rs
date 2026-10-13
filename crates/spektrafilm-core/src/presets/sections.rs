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
            ],
            Self::CameraFilter => &["camera_diffusion_filter"],
            Self::EnlargerFilter => &["enlarger.diffusion_filter"],
            Self::Enlarger => &["enlarger"],
            Self::Scanner => &["scanner"],
            Self::Print => &["print_render"],
            Self::OutputMix => &["output_mix"],
        }
    }
    fn owns(self, path: &str) -> bool {
        self.paths()
            .iter()
            .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}.")))
            && !(self == Self::Enlarger
                && (path.starts_with("enlarger.diffusion_filter")
                    || crate::print_filtration::is_neutral_baseline(path)))
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
fn insert(root: &mut Value, path: &str, value: Value) {
    let mut parts = path.split('.').peekable();
    let mut current = root;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current[part] = value;
            return;
        }
        if current.get(part).is_none() {
            current[part] = serde_json::json!({});
        }
        current = &mut current[part];
    }
}
pub fn extract_section(section: Section, params: &RuntimeParams) -> Result<Value, String> {
    crate::params::sources::validate_finite(params)?;
    let root = serde_json::to_value(LookParameters::capture(params)).map_err(|e| e.to_string())?;
    let mut leaves = Vec::new();
    flatten("", &root, &mut leaves);
    let mut payload = serde_json::json!({});
    for (path, value) in leaves {
        if section.owns(&path) && path != "film_render.grain.monochrome" {
            insert(&mut payload, &path, value);
        }
    }
    Ok(payload)
}
pub fn section_from_payload(payload: &Value) -> Result<Section, String> {
    parameter_edits(payload)?;
    let mut leaves = Vec::new();
    flatten("", payload, &mut leaves);
    if leaves.is_empty() {
        return Err("section payload is empty".into());
    }
    SECTIONS
        .iter()
        .copied()
        .find(|section| leaves.iter().all(|(path, _)| section.owns(path)))
        .ok_or_else(|| {
            "preset payload must carry editable leaves inside exactly one section".into()
        })
}
pub fn section_matches(preset: &Preset, params: &RuntimeParams) -> Result<bool, String> {
    let PresetKind::Section(_) = preset.kind()? else {
        return Ok(false);
    };
    let PresetParameters::Section(payload) = &preset.parameters else {
        unreachable!()
    };
    crate::params::sources::validate_finite(params)?;
    let applied = preset.overlay_section(params)?;
    let expected = comparable_section_value(&applied)?;
    let current = comparable_section_value(params)?;
    let mut leaves = Vec::new();
    flatten("", payload, &mut leaves);
    Ok(leaves.iter().all(|(path, _)| {
        let pointer = format!("/{}", path.replace('.', "/"));
        current.pointer(&pointer) == expected.pointer(&pointer)
    }))
}
/// The value a section payload is compared against: the look capture. The
/// neutral baselines stay out of it, because no preset carries them and the
/// resolved paper supplies them.
fn comparable_section_value(params: &RuntimeParams) -> Result<Value, String> {
    serde_json::to_value(LookParameters::capture(params)).map_err(|e| e.to_string())
}
pub fn matching_section_preset<'a>(
    section: Section,
    presets: impl IntoIterator<Item = &'a Preset>,
    params: &RuntimeParams,
) -> Result<Option<&'a Preset>, String> {
    let mut matches = Vec::new();
    for preset in presets {
        if preset.kind()? == PresetKind::Section(section) && section_matches(preset, params)? {
            matches.push(preset);
        }
    }
    matches.sort_by(|a, b| name_order(&a.name, &b.name));
    Ok(matches.into_iter().next())
}
