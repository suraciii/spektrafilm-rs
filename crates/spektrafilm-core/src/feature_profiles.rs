use crate::params::RuntimeParams;
use spektrafilm_model::grain::v2::{self, PROFILE_NAMES};

/// One built-in Dye grain appearance row.
///
/// The field types mirror the runtime leaves the row writes:
/// `v3_dye_support_um` is `f32` while `v3_event_area_um2` and
/// `channel_correlation` are `f64`, so the catalog values compare exactly
/// against `RuntimeParams` without a cast on either side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DyeProfile {
    pub(crate) name: &'static str,
    pub(crate) support_um: f32,
    pub(crate) event_area_um2: f64,
    pub(crate) channel_correlation: f64,
}

/// The authoritative Dye catalog, in stable selector order.
///
/// The `fine`, `coarse`, `subtle`, and `strong` rows were measured under one
/// fixed CPU scan condition on the `kodak_ultramax_400` stock with the row's
/// own three leaves varied. The `correlated` row uses the independently
/// accepted `channel_correlation` ceiling and was exercised in the CPU/GPU Dye
/// acceptance matrix. The `reference` row additionally carries the
/// `kodak_portra_800` reference validation of the shipped defaults. Each row
/// keeps the `reference` value for the controls it does not vary.
pub(crate) const DYE_PROFILES: [DyeProfile; 6] = [
    DyeProfile {
        name: "reference",
        support_um: 3.5,
        event_area_um2: 1.28,
        channel_correlation: 0.0,
    },
    DyeProfile {
        name: "fine",
        support_um: 1.0,
        event_area_um2: 1.28,
        channel_correlation: 0.0,
    },
    DyeProfile {
        name: "coarse",
        support_um: 8.0,
        event_area_um2: 1.28,
        channel_correlation: 0.0,
    },
    DyeProfile {
        name: "subtle",
        support_um: 3.5,
        event_area_um2: 0.32,
        channel_correlation: 0.0,
    },
    DyeProfile {
        name: "strong",
        support_um: 3.5,
        event_area_um2: 5.12,
        channel_correlation: 0.0,
    },
    DyeProfile {
        name: "correlated",
        support_um: 3.5,
        event_area_um2: 1.28,
        channel_correlation: 0.45,
    },
];

/// Stable selector order for the Dye family, derived from [`DYE_PROFILES`]
/// so the name list can never drift from the catalog.
const DYE_PROFILE_NAMES: [&str; 6] = [
    DYE_PROFILES[0].name,
    DYE_PROFILES[1].name,
    DYE_PROFILES[2].name,
    DYE_PROFILES[3].name,
    DYE_PROFILES[4].name,
    DYE_PROFILES[5].name,
];

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
    GrainDye,
    Halation,
}

impl FeatureProfileFamily {
    pub const fn id(self) -> &'static str {
        match self {
            Self::GrainProcedural => "grain.procedural",
            Self::GrainDye => "grain.dye",
            Self::Halation => "halation",
        }
    }

    pub fn parse(id: &str) -> Result<Self, String> {
        match id {
            "grain.procedural" => Ok(Self::GrainProcedural),
            "grain.dye" => Ok(Self::GrainDye),
            "halation" => Ok(Self::Halation),
            _ => Err(format!(
                "unknown feature profile family {id:?}; expected grain.procedural, grain.dye, or halation"
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
    GrainDye {
        support_um: f32,
        event_area_um2: f64,
        channel_correlation: f64,
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
            if v2::profile_index(name).is_none() {
                return Err(unknown_name(family, name, PROFILE_NAMES.iter().copied()));
            }
            let canonical = name_static(&PROFILE_NAMES, name);
            Ok(ResolvedFeatureProfile {
                family,
                name: canonical,
                values: FeatureProfileValues::GrainProcedural { name: canonical },
            })
        }
        FeatureProfileFamily::GrainDye => {
            // `custom` carries the `reference` row as its payload so the
            // resolver stays total, but `apply_profile` never writes it: the
            // current scoped values are preserved.
            if name == "custom" {
                let reference = &DYE_PROFILES[0];
                debug_assert_eq!(reference.name, "reference");
                return Ok(ResolvedFeatureProfile {
                    family,
                    name: "custom",
                    values: FeatureProfileValues::GrainDye {
                        support_um: reference.support_um,
                        event_area_um2: reference.event_area_um2,
                        channel_correlation: reference.channel_correlation,
                    },
                });
            }
            let profile = DYE_PROFILES
                .iter()
                .find(|candidate| candidate.name == name)
                .ok_or_else(|| {
                    unknown_name(
                        family,
                        name,
                        DYE_PROFILES.iter().map(|candidate| candidate.name),
                    )
                })?;
            Ok(ResolvedFeatureProfile {
                family,
                name: profile.name,
                values: FeatureProfileValues::GrainDye {
                    support_um: profile.support_um,
                    event_area_um2: profile.event_area_um2,
                    channel_correlation: profile.channel_correlation,
                },
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
    let mut last = [None; 3];
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
            if params.film_render.grain.v2_profile == "custom" {
                "custom"
            } else if v2::profile_index(&params.film_render.grain.v2_profile).is_some()
                && optional_procedural_overrides_clear(params)
            {
                name_static(&PROFILE_NAMES, &params.film_render.grain.v2_profile)
            } else {
                "custom"
            }
        }
        FeatureProfileFamily::GrainDye => DYE_PROFILES
            .iter()
            .find(|profile| {
                params.film_render.grain.v3_dye_support_um == profile.support_um
                    && params.film_render.grain.v3_event_area_um2 == profile.event_area_um2
                    && params.film_render.grain.channel_correlation == profile.channel_correlation
            })
            .map(|profile| profile.name)
            .unwrap_or("custom"),
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
        FeatureProfileFamily::GrainDye => &DYE_PROFILE_NAMES,
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
                params.film_render.grain.select_custom_grain_v2();
            } else {
                params.film_render.grain.v2_profile = name.to_owned();
                params.film_render.grain.v2_size = None;
                params.film_render.grain.v2_amount = None;
                params.film_render.grain.v2_shadows = None;
                params.film_render.grain.v2_midtones = None;
                params.film_render.grain.v2_highlights = None;
                params.film_render.grain.v2_chroma = None;
                params.film_render.grain.v2_resolution_factor = None;
            }
        }
        FeatureProfileValues::GrainDye {
            support_um,
            event_area_um2,
            channel_correlation,
        } => {
            if !custom {
                params.film_render.grain.v3_dye_support_um = support_um;
                params.film_render.grain.v3_event_area_um2 = event_area_um2;
                params.film_render.grain.channel_correlation = channel_correlation;
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
            "film_render.grain.v2_profile",
            "film_render.grain.v2_size",
            "film_render.grain.v2_amount",
            "film_render.grain.v2_shadows",
            "film_render.grain.v2_midtones",
            "film_render.grain.v2_highlights",
            "film_render.grain.v2_chroma",
            "film_render.grain.v2_resolution_factor",
        ],
        FeatureProfileFamily::GrainDye => &[
            "film_render.grain.v3_dye_support_um",
            "film_render.grain.v3_event_area_um2",
            "film_render.grain.channel_correlation",
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
        FeatureProfileFamily::GrainDye => 1,
        FeatureProfileFamily::Halation => 2,
    }
}

fn optional_procedural_overrides_clear(params: &RuntimeParams) -> bool {
    let grain = &params.film_render.grain;
    grain.v2_size.is_none()
        && grain.v2_amount.is_none()
        && grain.v2_shadows.is_none()
        && grain.v2_midtones.is_none()
        && grain.v2_highlights.is_none()
        && grain.v2_chroma.is_none()
        && grain.v2_resolution_factor.is_none()
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
    use crate::params::GrainEngine;
    #[test]
    fn parses_known_families_and_rejects_ambiguous_values() {
        let parsed = parse_selection("grain.dye=reference", 2).unwrap();
        assert_eq!(parsed.family, FeatureProfileFamily::GrainDye);
        assert_eq!(parsed.name, "reference");
        assert!(parse_selection("grain=reference", 1).is_err());
        assert!(parse_selection("grain.dye=reference=extra", 1).is_err());
        assert!(parse_selection("grain.dye=", 1).is_err());
    }

    #[test]
    fn applies_profiles_transactionally_and_preserves_scope() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.active = false;
        params.film_render.grain.engine = GrainEngine::Spektra;
        params.film_render.grain.rms_granularity = [9.0; 3];
        params.camera.film_format_mm = 42.0;
        params.film_render.halation.active = false;
        let selection = parse_selection("grain.dye=reference", 1).unwrap();
        apply(&mut params, &[selection]).unwrap();
        assert!(!params.film_render.grain.active);
        assert_eq!(params.film_render.grain.engine, GrainEngine::Spektra);
        assert_eq!(params.film_render.grain.v3_dye_support_um, 3.5);
        assert_eq!(params.film_render.grain.v3_event_area_um2, 1.28);
        assert_eq!(params.film_render.grain.channel_correlation, 0.0);
        assert_eq!(params.film_render.grain.rms_granularity, [9.0; 3]);
        assert_eq!(params.camera.film_format_mm, 42.0);
        assert!(!params.film_render.halation.active);
    }

    #[test]
    fn dye_selection_is_scope_exact_for_a_non_reference_row() {
        // A non-reference row writes the same three leaves and nothing else:
        // the resolver is a table lookup, not a reference-only special case.
        let mut params = RuntimeParams::default();
        params.film_render.grain.active = false;
        params.film_render.grain.engine = GrainEngine::Spektra;
        params.film_render.grain.rms_granularity = [9.0; 3];
        params.film_render.grain.density_min = [0.05; 3];
        params.film_render.grain.particle_scale_sublayers = [4.0, 0.15, 0.03];
        params.camera.film_format_mm = 42.0;
        params.random_seed = 12345;
        let selection = parse_selection("grain.dye=correlated", 1).unwrap();
        apply(&mut params, &[selection]).unwrap();
        assert_eq!(params.film_render.grain.v3_dye_support_um, 3.5);
        assert_eq!(params.film_render.grain.v3_event_area_um2, 1.28);
        assert_eq!(params.film_render.grain.channel_correlation, 0.45);
        assert!(!params.film_render.grain.active);
        assert_eq!(params.film_render.grain.engine, GrainEngine::Spektra);
        assert_eq!(params.film_render.grain.rms_granularity, [9.0; 3]);
        assert_eq!(params.film_render.grain.density_min, [0.05; 3]);
        assert_eq!(
            params.film_render.grain.particle_scale_sublayers,
            [4.0, 0.15, 0.03]
        );
        assert_eq!(params.camera.film_format_mm, 42.0);
        assert_eq!(params.random_seed, 12345);
        // The validator accepts every catalog row as applied.
        params.validate().unwrap();
    }

    #[test]
    fn last_selection_wins_per_family_and_custom_preserves_current_values() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.v3_dye_support_um = 7.0;
        let first = parse_selection("grain.dye=reference", 1).unwrap();
        let second = parse_selection("grain.dye=custom", 2).unwrap();
        apply(&mut params, &[first, second]).unwrap();
        assert_eq!(params.film_render.grain.v3_dye_support_um, 7.0);
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
        assert_eq!(
            names(FeatureProfileFamily::GrainDye),
            &[
                "reference",
                "fine",
                "coarse",
                "subtle",
                "strong",
                "correlated"
            ]
        );
        assert_eq!(names(FeatureProfileFamily::Halation).len(), 6);
        for name in PROFILE_NAMES {
            let resolved = resolve(FeatureProfileFamily::GrainProcedural, name).unwrap();
            assert_eq!(resolved.name, name);
            assert!(matches!(
                resolved.values,
                FeatureProfileValues::GrainProcedural { .. }
            ));
        }
        for name in names(FeatureProfileFamily::GrainDye) {
            assert!(resolve(FeatureProfileFamily::GrainDye, name).is_ok());
        }
        for name in names(FeatureProfileFamily::Halation) {
            assert!(resolve(FeatureProfileFamily::Halation, name).is_ok());
        }
    }

    #[test]
    fn custom_marks_scope_explicit_without_changing_values() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.v3_dye_support_um = 7.0;
        let selection = parse_selection("grain.dye=custom", 1).unwrap();
        apply(&mut params, &[selection]).unwrap();
        assert_eq!(params.film_render.grain.v3_dye_support_um, 7.0);
        assert!(
            params
                .explicit_section_paths
                .contains("film_render.grain.v3_dye_support_um")
        );
    }
    #[test]
    fn every_catalog_row_matches_model_values() {
        for (index, name) in PROFILE_NAMES.iter().enumerate() {
            let mut params = RuntimeParams::default();
            let token = format!("grain.procedural={name}");
            let selection = parse_selection(&token, 1).unwrap();
            apply(&mut params, &[selection]).unwrap();
            let expected = v2::GrainV2Params::for_profile(index);
            let resolved = params.film_render.grain.resolved_grain_v2();
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

    #[test]
    fn dye_and_halation_catalogs_match_exact_values() {
        for row in DYE_PROFILES {
            let token = format!("grain.dye={}", row.name);
            let mut params = RuntimeParams::default();
            params.film_render.grain.v3_dye_support_um = 7.0;
            params.film_render.grain.v3_event_area_um2 = 9.5;
            params.film_render.grain.channel_correlation = 0.2;
            apply(&mut params, &[parse_selection(&token, 1).unwrap()]).unwrap();
            params
                .validate()
                .unwrap_or_else(|error| panic!("{} must validate: {error}", row.name));
            // Only the three scope leaves change; the values are the row's.
            assert_eq!(
                params.film_render.grain.v3_dye_support_um, row.support_um,
                "{}",
                row.name
            );
            assert_eq!(
                params.film_render.grain.v3_event_area_um2, row.event_area_um2,
                "{}",
                row.name
            );
            assert_eq!(
                params.film_render.grain.channel_correlation, row.channel_correlation,
                "{}",
                row.name
            );
            // The row resolves back to its own canonical name.
            assert_eq!(
                effective_name(FeatureProfileFamily::GrainDye, &params),
                row.name,
                "{}",
                row.name
            );
        }
        for name in names(FeatureProfileFamily::Halation) {
            let mut params = RuntimeParams::default();
            let token = format!("halation={name}");
            let selection = parse_selection(&token, 1).unwrap();
            apply(&mut params, &[selection]).unwrap();
            let row = HALATION_PROFILES
                .iter()
                .find(|row| row.name == *name)
                .unwrap();
            assert_eq!(
                params.film_render.halation.halation_first_sigma_um,
                row.first_sigma_um
            );
            assert_eq!(params.film_render.halation.halation_strength, row.strength);
        }
    }

    #[test]
    fn dye_catalog_holds_the_reference_defaults_and_measured_rows() {
        let row = |name: &str| {
            DYE_PROFILES
                .iter()
                .find(|row| row.name == name)
                .unwrap_or_else(|| panic!("dye catalog must hold {name}"))
        };
        // `reference` keeps the shipped runtime defaults untouched.
        let defaults = crate::params::GrainParams::default();
        let reference = row("reference");
        assert_eq!(reference.support_um, defaults.v3_dye_support_um);
        assert_eq!(reference.event_area_um2, defaults.v3_event_area_um2);
        assert_eq!(reference.channel_correlation, defaults.channel_correlation);
        // The measured rows: one size axis, one strength axis, the ceiling.
        assert_eq!(row("fine").support_um, 1.0);
        assert_eq!(row("coarse").support_um, 8.0);
        assert_eq!(row("subtle").event_area_um2, 0.32);
        assert_eq!(row("strong").event_area_um2, 5.12);
        assert_eq!(
            row("correlated").channel_correlation,
            crate::params::grain::MEASURED_CHANNEL_CORRELATION_CEILING
        );
    }

    #[test]
    fn dye_edit_off_any_row_reports_custom() {
        for row in DYE_PROFILES {
            let mut params = RuntimeParams::default();
            let token = format!("grain.dye={}", row.name);
            apply(&mut params, &[parse_selection(&token, 1).unwrap()]).unwrap();
            params.film_render.grain.v3_dye_support_um += 0.5;
            assert_eq!(
                effective_name(FeatureProfileFamily::GrainDye, &params),
                "custom",
                "{}",
                row.name
            );
        }
    }
}
