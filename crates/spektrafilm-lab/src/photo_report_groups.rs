//! Groups mode for the photo report: one authored reading sequence.
//!
//! A group names the photos that belong together and the layout that states how
//! they relate. `strip` and `split` assert that the pixels correspond, so both
//! require an authored crop and validate it inside every photo; `pair` and
//! `single` assert nothing about correspondence.
//!
//! The numbers a group shows are authored display values. This module validates
//! their shape and publishes them; it never measures, compares, or derives one.

use super::DecodedPhoto;
use crate::common::Result;
use crate::documents::err;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) const BODY: &str = include_str!("../assets/photo-report-groups.html");
pub(super) const CSS: &str = include_str!("../assets/photo-report-groups.css");
pub(super) const JS: &str = include_str!("../assets/photo-report-groups.js");

const LAYOUTS: [&str; 4] = ["strip", "split", "pair", "single"];

/// One authored group: a label, how its photos relate, and what is shown.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReportGroup {
    id: String,
    label: String,
    layout: String,
    note: Option<String>,
    crop: Option<[u64; 4]>,
    metrics: Option<Vec<String>>,
    items: Vec<GroupItem>,
}

/// One photo occurrence inside a group.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GroupItem {
    photo: String,
    label: Option<String>,
    values: Option<Vec<String>>,
}

/// Reject a shape the generator cannot render, before any destination exists.
pub(super) fn validate(groups: &[ReportGroup], photos: &[DecodedPhoto]) -> Result<()> {
    if groups.is_empty() {
        return Err(err(
            "invalid_document",
            "groups must declare at least one group",
        ));
    }
    let mut seen = BTreeSet::new();
    for (index, group) in groups.iter().enumerate() {
        let owner = format!("group {index}");
        if !seen.insert(group.id.clone()) {
            return Err(err(
                "invalid_document",
                format!("duplicate group id {:?}", group.id),
            ));
        }
        super::identifier(&group.id, "group")?;
        if group.label.trim().is_empty() {
            return Err(err(
                "schema_validation",
                format!("{owner} label must be nonempty"),
            ));
        }
        if !LAYOUTS.contains(&group.layout.as_str()) {
            return Err(err(
                "schema_validation",
                format!(
                    "{owner} layout {:?} is not one of {}",
                    group.layout,
                    LAYOUTS.join(", ")
                ),
            ));
        }
        let expected = match group.layout.as_str() {
            "single" => 1,
            "split" => 2,
            _ => 0,
        };
        if expected > 0 && group.items.len() != expected {
            return Err(err(
                "schema_validation",
                format!(
                    "{owner} layout {} requires exactly {expected} item(s), found {}",
                    group.layout,
                    group.items.len()
                ),
            ));
        }
        if group.layout == "pair" && group.items.len() < 2 {
            return Err(err(
                "schema_validation",
                format!("{owner} layout pair requires at least two items"),
            ));
        }
        if group.items.is_empty() {
            return Err(err(
                "schema_validation",
                format!("{owner} must declare at least one item"),
            ));
        }
        for (slot, item) in group.items.iter().enumerate() {
            if photos.iter().all(|photo| photo.id != item.photo) {
                return Err(err(
                    "invalid_document",
                    format!("{owner} item {slot} names unknown photo {:?}", item.photo),
                ));
            }
        }
        match (&group.crop, group.layout.as_str()) {
            (None, "strip") | (None, "split") => {
                return Err(err(
                    "schema_validation",
                    format!("{owner} layout {} requires a crop", group.layout),
                ));
            }
            (Some(rect), "strip") | (Some(rect), "split") => {
                let [x, y, w, h] = *rect;
                if w == 0 || h == 0 {
                    return Err(err(
                        "schema_validation",
                        format!("{owner} crop must have positive width and height"),
                    ));
                }
                for item in &group.items {
                    let photo = photos
                        .iter()
                        .find(|photo| photo.id == item.photo)
                        .expect("validated above");
                    if x + w > photo.width as u64 || y + h > photo.height as u64 {
                        return Err(err(
                            "invalid_crop",
                            format!(
                                "{owner} crop {x},{y},{w},{h} escapes photo {:?} ({}×{})",
                                photo.id, photo.width, photo.height
                            ),
                        ));
                    }
                }
            }
            _ => {}
        }
        let declared = group.metrics.as_deref().unwrap_or(&[]);
        let shows_values = group.items.iter().any(|item| item.values.is_some());
        if shows_values && declared.is_empty() {
            return Err(err(
                "schema_validation",
                format!("{owner} declares item values without metrics"),
            ));
        }
        for item in &group.items {
            let values = item.values.as_deref().unwrap_or(&[]);
            if !declared.is_empty() && values.len() != declared.len() {
                return Err(err(
                    "schema_validation",
                    format!(
                        "{owner} item {:?} declares {} value(s) for {} metric(s)",
                        item.photo,
                        values.len(),
                        declared.len()
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// The published shape: every item resolved to its display artifact, so the
/// page needs no lookup and no second source of truth.
pub(super) fn payload(groups: &[ReportGroup], photos: &[DecodedPhoto]) -> Value {
    let records: Vec<Value> = groups
        .iter()
        .map(|group| {
            let items: Vec<Value> = group
                .items
                .iter()
                .map(|item| {
                    let photo = photos
                        .iter()
                        .find(|photo| photo.id == item.photo)
                        .expect("validated before payload");
                    json!({
                        "photo": item.photo,
                        "label": item.label.clone().unwrap_or_else(|| photo.label.clone()),
                        "values": item.values.clone().unwrap_or_default(),
                        "src": photo.display_path,
                        "width": photo.width,
                        "height": photo.height,
                    })
                })
                .collect();
            json!({
                "id": group.id,
                "label": group.label,
                "layout": group.layout,
                "note": group.note,
                "crop": group.crop,
                "metrics": group.metrics.clone().unwrap_or_default(),
                "items": items,
            })
        })
        .collect();
    Value::Array(records)
}
