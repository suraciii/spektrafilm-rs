use crate::params::RuntimeParams;
use spektrafilm_model::grain::procedural::{self, PROFILE_NAMES};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HalationProfile {
    pub(crate) name: &'static str,
    pub(crate) first_sigma_um: [f64; 3],
    pub(crate) strength: [f64; 3],
}

pub(crate) const HALATION_PROFILES: [HalationProfile; 6] = [
    HalationProfile {
        name: "still_strong",
        first_sigma_um: [65.0, 65.0, 65.0],
        strength: [0.015, 0.005, 0.0],
    },
    HalationProfile {
        name: "still_weak",
        first_sigma_um: [65.0, 65.0, 65.0],
        strength: [0.08, 0.02, 0.0],
    },
    HalationProfile {
        name: "still_no",
        first_sigma_um: [65.0, 65.0, 65.0],
        strength: [0.30, 0.10, 0.015],
    },
    HalationProfile {
        name: "cine_strong",
        first_sigma_um: [50.0, 50.0, 50.0],
        strength: [0.015, 0.005, 0.0],
    },
    HalationProfile {
        name: "cine_weak",
        first_sigma_um: [50.0, 50.0, 50.0],
        strength: [0.08, 0.02, 0.0],
    },
    HalationProfile {
        name: "cine_no",
        first_sigma_um: [50.0, 50.0, 50.0],
        strength: [0.30, 0.10, 0.015],
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureProfileFamily {
    GrainProcedural,
    Halation,
}

impl FeatureProfileFamily {
    pub const fn id(self) -> &'static str {
        match self {
            Self::GrainProcedural => "grain.procedural",
            Self::Halation => "halation",
        }
    }

    pub fn parse(id: &str) -> Result<Self, String> {
        match id {
            "grain.procedural" => Ok(Self::GrainProcedural),
            "halation" => Ok(Self::Halation),
            _ => Err(format!(
                "unknown feature profile family {id:?}; expected grain.procedural or halation"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureProfileSelection<'a> {
    pub family: FeatureProfileFamily,
    pub name: &'a str,
    pub position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FeatureProfileValues {
    GrainProcedural {
        name: &'static str,
    },
    Halation {
        first_sigma_um: [f64; 3],
        strength: [f64; 3],
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedFeatureProfile {
    pub family: FeatureProfileFamily,
    pub name: &'static str,
    pub values: FeatureProfileValues,
}

pub fn parse_selection(
    value: &str,
    position: usize,
) -> Result<FeatureProfileSelection<'_>, String> {
    let (family, name) = value
        .split_once('=')
        .ok_or_else(|| format!("--feature-profile #{position} {value:?}: expected FAMILY=NAME"))?;
    if family.is_empty() || name.is_empty() || name.contains('=') {
        return Err(format!(
            "--feature-profile #{position} {value:?}: expected non-empty FAMILY=NAME"
        ));
    }
    Ok(FeatureProfileSelection {
        family: FeatureProfileFamily::parse(family)?,
        name,
        position,
    })
}

pub fn resolve(family: FeatureProfileFamily, name: &str) -> Result<ResolvedFeatureProfile, String> {
    match family {
        FeatureProfileFamily::GrainProcedural => {
            if name == "custom" {
                return Ok(ResolvedFeatureProfile {
                    family,
                    name: "custom",
                    values: FeatureProfileValues::GrainProcedural { name: "custom" },
                });
            }
            if procedural::profile_index(name).is_none() {
                return Err(unknown_name(family, name, PROFILE_NAMES.iter().copied()));
            }
            let canonical = name_static(&PROFILE_NAMES, name);
            Ok(ResolvedFeatureProfile {
                family,
                name: canonical,
                values: FeatureProfileValues::GrainProcedural { name: canonical },
            })
        }
        FeatureProfileFamily::Halation => {
            if name == "custom" {
                return Ok(ResolvedFeatureProfile {
                    family,
                    name: "custom",
                    values: FeatureProfileValues::Halation {
                        first_sigma_um: [0.0; 3],
                        strength: [0.0; 3],
                    },
                });
            }
            let profile = HALATION_PROFILES
                .iter()
                .find(|candidate| candidate.name == name)
                .ok_or_else(|| {
                    unknown_name(
                        family,
                        name,
                        HALATION_PROFILES.iter().map(|candidate| candidate.name),
                    )
                })?;
            Ok(ResolvedFeatureProfile {
                family,
                name: profile.name,
                values: FeatureProfileValues::Halation {
                    first_sigma_um: profile.first_sigma_um,
                    strength: profile.strength,
                },
            })
        }
    }
}

pub fn apply(
    params: &mut RuntimeParams,
    selections: &[FeatureProfileSelection<'_>],
) -> Result<(), String> {
    *params = apply_with_requests(params, selections)?.0;
    Ok(())
}

/// Apply selections transactionally and return each selection's requested scope.
/// Resolution is validated for every selection before the candidate is committed.
pub fn apply_with_requests(
    params: &RuntimeParams,
    selections: &[FeatureProfileSelection<'_>],
) -> Result<(RuntimeParams, Vec<serde_json::Value>), String> {
    let mut resolved = Vec::with_capacity(selections.len());
    for selection in selections {
        resolved.push(resolve(selection.family, selection.name)?);
    }
    let mut requested_candidate = params.clone();
    let mut requested = Vec::with_capacity(selections.len());
    for (selection, profile) in selections.iter().zip(&resolved) {
        apply_profile(
            &mut requested_candidate,
            *profile,
            selection.name == "custom",
        );
        requested.push(effective_values(selection.family, &requested_candidate));
    }
    let mut candidate = params.clone();
    let mut last = [None; 2];
    for (index, selection) in selections.iter().enumerate() {
        last[family_index(selection.family)] = Some(index);
    }
    for (index, selection) in selections.iter().enumerate() {
        if last[family_index(selection.family)] == Some(index) {
            apply_profile(&mut candidate, resolved[index], selection.name == "custom");
        }
    }
    crate::params::sources::validate_finite(&candidate)?;
    candidate.validate()?;
    Ok((candidate, requested))
}

pub fn effective_name(family: FeatureProfileFamily, params: &RuntimeParams) -> &'static str {
    match family {
        FeatureProfileFamily::GrainProcedural => {
            if params.film_render.grain.procedural_profile == "custom" {
                "custom"
            } else if procedural::profile_index(&params.film_render.grain.procedural_profile)
                .is_some()
                && optional_procedural_overrides_clear(params)
            {
                name_static(&PROFILE_NAMES, &params.film_render.grain.procedural_profile)
            } else {
                "custom"
            }
        }
        FeatureProfileFamily::Halation => HALATION_PROFILES
            .iter()
            .find(|profile| {
                params.film_render.halation.halation_first_sigma_um == profile.first_sigma_um
                    && params.film_render.halation.halation_strength == profile.strength
            })
            .map(|profile| profile.name)
            .unwrap_or("custom"),
    }
}

pub fn names(family: FeatureProfileFamily) -> &'static [&'static str] {
    match family {
        FeatureProfileFamily::GrainProcedural => &PROFILE_NAMES,
        FeatureProfileFamily::Halation => {
            const NAMES: [&str; 6] = [
                "still_strong",
                "still_weak",
                "still_no",
                "cine_strong",
                "cine_weak",
                "cine_no",
            ];
            &NAMES
        }
    }
}

fn apply_profile(params: &mut RuntimeParams, profile: ResolvedFeatureProfile, custom: bool) {
    match profile.values {
        FeatureProfileValues::GrainProcedural { name } => {
            if custom {
                params.film_render.grain.select_custom_grain_procedural();
            } else {
                params.film_render.grain.procedural_profile = name.to_owned();
                params.film_render.grain.procedural_size = None;
                params.film_render.grain.procedural_amount = None;
                params.film_render.grain.procedural_shadows = None;
                params.film_render.grain.procedural_midtones = None;
                params.film_render.grain.procedural_highlights = None;
                params.film_render.grain.procedural_chroma = None;
                params.film_render.grain.procedural_resolution_factor = None;
            }
        }
        FeatureProfileValues::Halation {
            first_sigma_um,
            strength,
        } => {
            if !custom {
                params.film_render.halation.halation_first_sigma_um = first_sigma_um;
                params.film_render.halation.halation_strength = strength;
            }
        }
    }
    for path in profile_scope(profile.family) {
        params.explicit_section_paths.insert((*path).to_owned());
    }
}

/// Return the canonical serialized leaves owned by a family.
pub fn scope(family: FeatureProfileFamily) -> &'static [&'static str] {
    profile_scope(family)
}
/// Serialize only the current effective leaves owned by a family.
pub fn effective_values(family: FeatureProfileFamily, params: &RuntimeParams) -> serde_json::Value {
    let root = serde_json::to_value(params).expect("runtime parameters serialize");
    let mut values = serde_json::Map::new();
    for path in scope(family) {
        if let Some(value) = crate::params::sources::lookup(&root, path) {
            values.insert((*path).to_owned(), value.clone());
        }
    }
    serde_json::Value::Object(values)
}
fn profile_scope(family: FeatureProfileFamily) -> &'static [&'static str] {
    match family {
        FeatureProfileFamily::GrainProcedural => &[
            "film_render.grain.procedural_profile",
            "film_render.grain.procedural_size",
            "film_render.grain.procedural_amount",
            "film_render.grain.procedural_shadows",
            "film_render.grain.procedural_midtones",
            "film_render.grain.procedural_highlights",
            "film_render.grain.procedural_chroma",
            "film_render.grain.procedural_resolution_factor",
        ],
        FeatureProfileFamily::Halation => &[
            "film_render.halation.halation_first_sigma_um",
            "film_render.halation.halation_strength",
        ],
    }
}
fn family_index(family: FeatureProfileFamily) -> usize {
    match family {
        FeatureProfileFamily::GrainProcedural => 0,
        FeatureProfileFamily::Halation => 1,
    }
}

fn optional_procedural_overrides_clear(params: &RuntimeParams) -> bool {
    let grain = &params.film_render.grain;
    grain.procedural_size.is_none()
        && grain.procedural_amount.is_none()
        && grain.procedural_shadows.is_none()
        && grain.procedural_midtones.is_none()
        && grain.procedural_highlights.is_none()
        && grain.procedural_chroma.is_none()
        && grain.procedural_resolution_factor.is_none()
}

fn name_static(names: &[&'static str], name: &str) -> &'static str {
    names
        .iter()
        .copied()
        .find(|candidate| *candidate == name)
        .unwrap()
}

fn unknown_name<I>(family: FeatureProfileFamily, name: &str, names: I) -> String
where
    I: IntoIterator<Item = &'static str>,
{
    let names = names.into_iter().collect::<Vec<_>>().join(", ");
    format!("unknown feature profile {family:?} name {name:?}; expected one of {names}, or custom")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_removed_dye_family() {
        assert!(parse_selection("grain.dye=reference", 2).is_err());
        assert!(parse_selection("grain=reference", 1).is_err());
        assert!(parse_selection("halation=cine_no=extra", 1).is_err());
        assert!(parse_selection("halation=", 1).is_err());
    }
    #[test]
    fn halation_profile_matches_exact_values() {
        let mut params = RuntimeParams::default();
        let selection = parse_selection("halation=cine_no", 1).unwrap();
        apply(&mut params, &[selection]).unwrap();
        assert_eq!(
            params.film_render.halation.halation_first_sigma_um,
            [50.0; 3]
        );
        assert_eq!(
            params.film_render.halation.halation_strength,
            [0.30, 0.10, 0.015]
        );
        assert_eq!(
            effective_name(FeatureProfileFamily::Halation, &params),
            "cine_no"
        );
    }
    #[test]
    fn catalogs_match_shared_model_and_declared_counts() {
        assert_eq!(names(FeatureProfileFamily::GrainProcedural).len(), 12);
        assert_eq!(names(FeatureProfileFamily::Halation).len(), 6);
        for name in PROFILE_NAMES {
            let resolved = resolve(FeatureProfileFamily::GrainProcedural, name).unwrap();
            assert_eq!(resolved.name, name);
            assert!(matches!(
                resolved.values,
                FeatureProfileValues::GrainProcedural { .. }
            ));
        }
        for name in names(FeatureProfileFamily::Halation) {
            assert!(resolve(FeatureProfileFamily::Halation, name).is_ok());
        }
    }

    #[test]
    fn every_catalog_row_matches_model_values() {
        for (index, name) in PROFILE_NAMES.iter().enumerate() {
            let mut params = RuntimeParams::default();
            let token = format!("grain.procedural={name}");
            let selection = parse_selection(&token, 1).unwrap();
            apply(&mut params, &[selection]).unwrap();
            let expected = procedural::ProceduralGrainParams::for_profile(index);
            let resolved = params.film_render.grain.resolved_grain_procedural();
            assert_eq!(resolved.size, expected.size, "{name}");
            assert_eq!(resolved.amount, expected.amount, "{name}");
            assert_eq!(resolved.shadows, expected.shadows, "{name}");
            assert_eq!(resolved.midtones, expected.midtones, "{name}");
            assert_eq!(resolved.highlights, expected.highlights, "{name}");
            assert_eq!(resolved.color, expected.color, "{name}");
            assert_eq!(
                resolved.resolution_factor, expected.resolution_factor,
                "{name}"
            );
        }
    }
}
