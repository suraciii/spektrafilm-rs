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
    /// Measurement families of the implicit full-image population.
    pub full_image: &'static [&'static str],
    /// Measurement families of a declared region.
    pub regions: &'static [&'static str],
}

/// Measurement families of the standard appearance protocol.
const APPEARANCE: &[&str] = &["numeric", "color", "tone", "flip"];
/// Measurement families of a deterministic colour-and-tone comparison.
const COLOR_TONE: &[&str] = &["numeric", "color", "tone"];
/// Measurement families of one source measured on its own. `color` and `flip`
/// are absent because both are differences between two sources. The whole
/// image is not a neutral patch or a flat field, so the structural families
/// appear only on the regions a caller declares.
const SINGLE_SOURCE: &[&str] = &["numeric", "tone"];

/// The built-in profiles, in declaration order.
pub const PROFILES: &[Profile] = &[
    Profile {
        id: "lab-appearance-v1",
        full_image: APPEARANCE,
        regions: &["numeric", "color", "tone"],
    },
    Profile {
        id: "lab-color-tone-v1",
        full_image: COLOR_TONE,
        regions: &["numeric", "color"],
    },
    Profile {
        id: "lab-analysis-v1",
        full_image: SINGLE_SOURCE,
        regions: &["numeric", "neutrality", "color_noise"],
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
        "measurements": profile.full_image,
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
/// A `flat_field` region selects the grain family beside the profile's region
/// families, because that role is the one a grain probe is measured on.
pub fn regions(profile: &Profile, declared: &[Region]) -> Vec<Value> {
    declared
        .iter()
        .map(|region| {
            let mut families: Vec<&str> = profile.regions.to_vec();
            if region.role == "flat_field" && !families.contains(&"grain") {
                families.push("grain");
            }
            json!({
                "id": region.id,
                "role": region.role,
                "measurements": families,
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
            "invalid_region",
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
            "invalid_region",
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
            "invalid_region",
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
                protocol(profile)["measurements"]
                    .as_array()
                    .is_some_and(|families| !families.is_empty()),
                "{} selects no family",
                profile.id
            );
            assert!(profile.id.starts_with("lab-"), "{}", profile.id);
        }
    }
}
