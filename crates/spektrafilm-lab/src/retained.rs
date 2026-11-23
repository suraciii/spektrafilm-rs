//! Publishing a retained record.
//!
//! One writer decides the identity of every record this crate publishes: the
//! digest covers the document without its own `id`, the declared kind is what
//! validates it, and the file only appears after validation passes.

use crate::{Error, Result, common, documents};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub fn finalize(root: &Path, name: &str, value: &mut Value) -> Result<()> {
    value
        .as_object_mut()
        .ok_or_else(|| Error::new("document_type", "Manifest must be an object"))?
        .remove("id");
    value["id"] = json!(common::digest(value)?);
    // The record names its own kind, so the published file and the validated
    // kind cannot drift apart.
    let kind = value["kind"].as_str().unwrap_or("comparison").to_owned();
    documents::validate_generated_document(&kind, value, root)?;
    let temp = root.join(format!("{name}.tmp"));
    common::dump(&temp, value)?;
    fs::rename(temp, root.join(name))?;
    Ok(())
}
