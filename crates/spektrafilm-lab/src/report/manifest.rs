//! The report's input model.
//!
//! The page exists so a person can look at photographs: a case holds one
//! reference, the variants it is compared with, and the places to look at.
//! Anything a reader cannot see is not part of this model.
use crate::common::{Error, Result};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(crate) const FORMAT: &str = "spektrafilm.report.v1";
pub(crate) const DEFAULT_TITLE: &str = "SpektraFilm 人工审查";

#[derive(Clone, Debug)]
pub(crate) struct Image {
    pub path: String,
    pub fit: Option<String>,
    pub label: String,
}

#[derive(Clone, Debug)]
pub(crate) struct View {
    pub id: String,
    pub label: String,
    pub crop: Option<[u32; 4]>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Case {
    pub id: String,
    pub title: String,
    pub aligned: bool,
    pub reference: Image,
    pub variants: Vec<Image>,
    pub views: Vec<View>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Manifest {
    pub title: String,
    pub cases: Vec<Case>,
}

pub(crate) fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn image(image: &Image, what: &str) -> Result<()> {
    if !safe_relative(&image.path) {
        return Err(Error::new(
            "invalid_manifest",
            format!("{what} path must be a relative file path"),
        ));
    }
    if let Some(fit) = &image.fit
        && !safe_relative(fit)
    {
        return Err(Error::new(
            "invalid_manifest",
            format!("{what} fit path must be a relative file path"),
        ));
    }
    Ok(())
}

impl Manifest {
    /// The model is only worth having if it cannot express a page a person
    /// cannot read: one reference, known identities, places that exist.
    pub(crate) fn validate(&self) -> Result<()> {
        // A comparison that failed has no case to show. An empty page is the
        // honest result; the record carries the failure.
        let mut ids = BTreeSet::new();
        for case in &self.cases {
            if case.id.trim().is_empty() || !ids.insert(case.id.clone()) {
                return Err(Error::new(
                    "invalid_manifest",
                    "case ids must be unique and non-empty",
                ));
            }
            image(&case.reference, "reference")?;
            for variant in &case.variants {
                image(variant, "variant")?;
            }
            if case.views.is_empty() {
                return Err(Error::new(
                    "invalid_manifest",
                    format!("case {} has no place to look at", case.id),
                ));
            }
            let mut views = BTreeSet::new();
            for view in &case.views {
                if view.id.trim().is_empty() || !views.insert(view.id.clone()) {
                    return Err(Error::new(
                        "invalid_manifest",
                        format!("case {} has duplicate or empty view ids", case.id),
                    ));
                }
                if let Some([_, _, width, height]) = view.crop
                    && (width == 0 || height == 0)
                {
                    return Err(Error::new(
                        "invalid_manifest",
                        format!("view {} has an empty crop", view.id),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn to_json(&self) -> Value {
        let image = |image: &Image| {
            let mut value = json!({"path": image.path, "label": image.label});
            if let Some(fit) = &image.fit {
                value["fit"] = json!(fit);
            }
            value
        };
        json!({
            "format": FORMAT,
            "title": self.title,
            "cases": self.cases.iter().map(|case| json!({
                "id": case.id,
                "title": case.title,
                "aligned": case.aligned,
                "reference": image(&case.reference),
                "variants": case.variants.iter().map(image).collect::<Vec<_>>(),
                "views": case.views.iter().map(|view| {
                    let mut value = json!({"id": view.id, "label": view.label});
                    if let Some(crop) = view.crop {
                        value["crop"] = json!(crop);
                    }
                    if !view.notes.is_empty() {
                        value["notes"] = json!(view.notes);
                    }
                    value
                }).collect::<Vec<_>>(),
                "notes": case.notes,
            })).collect::<Vec<_>>(),
        })
    }
}
