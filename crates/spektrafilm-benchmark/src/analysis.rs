//! Built-in analysis profiles: one named protocol and one region family policy.
//!
//! A profile is a generator, not a document kind. It materializes a complete
//! protocol document; that document is the retained authority. The profile id
//! equals the protocol id it produces, because one id names one protocol.
//!
//! The selected families are part of the materialized protocol, so they
//! contribute to the protocol digest. Two comparisons that measured different
//! families therefore cannot share a digest.

use serde_json::{Value, json};

use crate::common::{Error, Result};

/// One built-in analysis profile.
pub struct Profile {
    /// Profile identity, which is also the protocol identity it produces.
    pub id: &'static str,
    /// Families per population role.
    ///
    /// One table. A role is the only thing that decides what is measured, so no
    /// region can ask for a family its role cannot support, and there is no
    /// second place to look.
    pub roles: &'static [(&'static str, &'static [&'static str])],
}

impl Profile {
    /// The families measured on a population of this role.
    ///
    /// A role the table does not name measures nothing: an unlisted role is an
    /// unlisted question, not one to guess an answer to.
    pub fn families(&self, role: &str) -> &'static [&'static str] {
        self.roles
            .iter()
            .find(|(name, _)| *name == role)
            .map(|(_, families)| *families)
            .unwrap_or(&[])
    }

    /// The whole table as the protocol retains it.
    pub fn family_table(&self) -> Value {
        Value::Object(
            self.roles
                .iter()
                .map(|(role, families)| (role.to_string(), json!(families)))
                .collect(),
        )
    }
}

/// Two sources, compared for appearance.
const APPEARANCE: &[&str] = &["numeric", "color", "tone", "flip"];
/// Two sources, compared for colour and tone only.
const COLOR_TONE: &[&str] = &["numeric", "color", "tone"];
/// One source measured on its own. `color` and `flip` are absent because both
/// are differences between two sources.
const SINGLE_SOURCE: &[&str] = &["numeric", "tone"];

/// Families on a declared region of an appearance comparison.
const APPEARANCE_REGION: &[&str] = &["numeric", "color", "tone"];
/// A flat field is the one role a grain probe is measured on.
const APPEARANCE_FLAT: &[&str] = &["numeric", "color", "tone", "grain"];
const COLOR_TONE_REGION: &[&str] = &["numeric", "color"];
const COLOR_TONE_FLAT: &[&str] = &["numeric", "color", "grain"];
const SINGLE_REGION: &[&str] = &["numeric"];
const SINGLE_FLAT: &[&str] = &["numeric", "color_noise", "grain"];
const SINGLE_TEXTURE: &[&str] = &["numeric", "color_noise"];
const SINGLE_NEUTRAL: &[&str] = &["numeric", "neutrality"];

/// The built-in profiles, in declaration order.
pub const PROFILES: &[Profile] = &[
    Profile {
        id: "lab-appearance-v1",
        roles: &[
            ("full_image", APPEARANCE),
            ("flat_field", APPEARANCE_FLAT),
            ("texture", APPEARANCE_REGION),
            ("neutral", APPEARANCE_REGION),
            ("skin", APPEARANCE_REGION),
            ("shadow", APPEARANCE_REGION),
            ("highlight", APPEARANCE_REGION),
            ("color", APPEARANCE_REGION),
            ("edge", APPEARANCE_REGION),
            ("other", APPEARANCE_REGION),
        ],
    },
    Profile {
        id: "lab-color-tone-v1",
        roles: &[
            ("full_image", COLOR_TONE),
            ("flat_field", COLOR_TONE_FLAT),
            ("texture", COLOR_TONE_REGION),
            ("neutral", COLOR_TONE_REGION),
            ("skin", COLOR_TONE_REGION),
            ("shadow", COLOR_TONE_REGION),
            ("highlight", COLOR_TONE_REGION),
            ("color", COLOR_TONE_REGION),
            ("edge", COLOR_TONE_REGION),
            ("other", COLOR_TONE_REGION),
        ],
    },
    Profile {
        id: "lab-analysis-v1",
        roles: &[
            ("full_image", SINGLE_SOURCE),
            ("flat_field", SINGLE_FLAT),
            ("texture", SINGLE_TEXTURE),
            ("neutral", SINGLE_NEUTRAL),
            ("skin", SINGLE_REGION),
            ("shadow", SINGLE_REGION),
            ("highlight", SINGLE_REGION),
            ("color", SINGLE_REGION),
            ("edge", SINGLE_REGION),
            ("other", SINGLE_REGION),
        ],
    },
];

/// Resolve one built-in profile by identity.
pub fn profile(id: &str) -> Result<&'static Profile> {
    PROFILES
        .iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| {
            Error::new(
                "unknown_analysis",
                format!(
                    "Unknown analysis profile: {id}; available: {}",
                    PROFILES
                        .iter()
                        .map(|profile| profile.id)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })
}

/// Materialize the complete protocol of one profile.
///
/// Every block a selected family reads is present, so a caller never inherits
/// a default. The values are the standard appearance settings, and the family
/// list is what distinguishes the two profiles.
pub fn protocol(profile: &Profile) -> Value {
    json!({
        "id": profile.id,
        "pixels_per_degree": 60.0,
        "region_sampling_scales": [1.0, 0.4, 0.16],
        "grain": {
            "residual": {"method": "flat_mean", "high_pass_radius_px": 4.0, "border_px": 1},
            "spectrum": {"window": "hann"}
        },
        "hotspots": {"count": 6, "min_area_px": 151},
        "delta_e_thresholds": [1.0, 2.0, 3.0, 5.0],
        "families": profile.family_table(),
    })
}

/// One declared region of a generated geometry.
pub struct Region {
    pub id: String,
    pub x: u64,
    pub y: u64,
    pub width: u64,
    pub height: u64,
    pub role: String,
}

/// Materialize the geometry of a generated region set.
///
/// A region declares where it is and what it is a population of. It declares no
/// family: the profile's table decides, so the generator and the engine cannot
/// disagree about what a role measures.
pub fn regions(declared: &[Region]) -> Vec<Value> {
    declared
        .iter()
        .map(|region| {
            json!({
                "id": region.id,
                "role": region.role,
                "geometry": {
                    "x": region.x,
                    "y": region.y,
                    "width": region.width,
                    "height": region.height,
                    "space": "render",
                },
            })
        })
        .collect()
}

/// Validate one declared role against the region role vocabulary.
pub fn validate_role(role: &str) -> Result<()> {
    const ROLES: &[&str] = &[
        "skin",
        "neutral",
        "shadow",
        "highlight",
        "color",
        "flat_field",
        "edge",
        "texture",
        "other",
    ];
    if ROLES.contains(&role) {
        Ok(())
    } else {
        Err(Error::new(
            "invalid_region_role",
            format!(
                "Unknown region role: {role}; available: {}",
                ROLES.join(", ")
            ),
        ))
    }
}

/// Validate one declared region identity.
pub fn validate_identity(id: &str) -> Result<()> {
    if id == "full_image" {
        return Err(Error::new(
            "invalid_region_identity",
            "full_image is reserved for the complete image",
        ));
    }
    let valid = id
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '.' | '-'));
    if valid {
        Ok(())
    } else {
        Err(Error::new(
            "invalid_region_identity",
            format!("Region identity does not match ^[A-Za-z0-9][A-Za-z0-9_.-]*$: {id}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One profile identity names one protocol, and the retained protocol is
    /// the authority, so the two ids must be the same string.
    #[test]
    fn a_profile_id_is_its_protocol_id() {
        let mut seen = std::collections::BTreeSet::new();
        for profile in PROFILES {
            assert!(seen.insert(profile.id), "duplicate profile: {}", profile.id);
            assert_eq!(protocol(profile)["id"], profile.id);
            assert!(
                protocol(profile)["families"]["full_image"]
                    .as_array()
                    .is_some_and(|families| !families.is_empty()),
                "{} selects no family for the whole image",
                profile.id
            );
            // Every writable role has an entry, so no region can ask for a
            // family the table has no answer for.
            for role in crate::documents::region_roles().unwrap() {
                assert!(
                    !profile.families(&role).is_empty() || role == "full_image",
                    "{} has no families for {role}",
                    profile.id
                );
            }
            assert!(profile.id.starts_with("lab-"), "{}", profile.id);
        }
    }
}
