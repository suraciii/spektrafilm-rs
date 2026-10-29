//! Diagnostic input names migrate here; engine code uses canonical names.

/// Read a Dye diagnostic, falling back to its historical spelling only when
/// the canonical variable is absent. Invalid Unicode remains an error.
pub fn diagnostic_env(name: &str) -> Result<String, std::env::VarError> {
    read_with(name, |key| std::env::var(key))
}

fn read_with(
    name: &str,
    mut read: impl FnMut(&str) -> Result<String, std::env::VarError>,
) -> Result<String, std::env::VarError> {
    let legacy = match name {
        "SPEKTRAFILM_GRAIN_DYE_CELL_UM" => Some("SPEKTRAFILM_GRAIN_V3_CELL_UM"),
        "SPEKTRAFILM_GRAIN_DYE_EXPECTATION" => Some("SPEKTRAFILM_GRAIN_V3_EXPECTATION"),
        "SPEKTRAFILM_GRAIN_DYE_CELL_SPECTRAL" => Some("SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL"),
        "SPEKTRAFILM_GRAIN_DYE_MASS_LAW" => Some("SPEKTRAFILM_GRAIN_V3_MASS_LAW"),
        _ => None,
    };
    match read(name) {
        Err(std::env::VarError::NotPresent) if legacy.is_some() => read(legacy.unwrap()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_diagnostic_precedes_historical_spelling_without_environment_mutation() {
        for suffix in ["CELL_UM", "EXPECTATION", "CELL_SPECTRAL", "MASS_LAW"] {
            let canonical = format!("SPEKTRAFILM_GRAIN_DYE_{suffix}");
            let historical = format!("SPEKTRAFILM_GRAIN_V3_{suffix}");
            assert_eq!(
                read_with(&canonical, |key| if key == historical {
                    Ok("old".into())
                } else {
                    Err(std::env::VarError::NotPresent)
                })
                .unwrap(),
                "old"
            );
            assert_eq!(
                read_with(&canonical, |key| Ok(if key == canonical {
                    "new"
                } else {
                    "old"
                }
                .into()))
                .unwrap(),
                "new"
            );
            assert!(matches!(
                read_with(&canonical, |key| if key == canonical {
                    Err(std::env::VarError::NotUnicode("invalid".into()))
                } else {
                    Ok("old".into())
                }),
                Err(std::env::VarError::NotUnicode(_))
            ));
        }
    }
}
