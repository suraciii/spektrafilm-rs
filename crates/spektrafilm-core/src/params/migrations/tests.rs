use super::*;
use crate::params::{PrintExposureMode, RuntimeParams};
#[test]
fn complete_migration_rejects_cmyk_conflicts_before_changing_exposure() {
    for (fields, reason) in [
        (
            serde_json::json!({"magazine_print_color": {}, "cmyk": {}}),
            Reason::LegacyConflict,
        ),
        (
            serde_json::json!({"magazine_print_color": 42}),
            Reason::LegacyGroupObject,
        ),
        (
            serde_json::json!({
                "workflow": {"route": "input > film > scan > magazine"},
                "magazine_print_color": {"active": false}
            }),
            Reason::LegacyRouteConflict,
        ),
    ] {
        let mut value = fields;
        value["enlarger"] = serde_json::json!({
            "normalize_print_exposure": true,
            "print_exposure_compensation": false
        });
        value["io"] = serde_json::json!({
            "output_gamut_compress": {"lightness_compression": null}
        });
        let original = value.clone();
        let error = normalize_legacy_complete_params_detailed(&mut value).unwrap_err();
        assert_eq!(error.reason, reason);
        assert_eq!(value, original);
    }
}

#[test]
fn legacy_conversion_paths_rename_and_preserve_the_four_states() {
    for (pair, mode, tracking) in [
        (
            serde_json::json!({"normalize_print_exposure": true, "print_exposure_compensation": true}),
            "auto",
            "follow_mode",
        ),
        (
            serde_json::json!({"normalize_print_exposure": false, "print_exposure_compensation": false}),
            "manual",
            "follow_mode",
        ),
        (
            serde_json::json!({"normalize_print_exposure": true, "print_exposure_compensation": false}),
            "auto",
            "never",
        ),
        (
            serde_json::json!({"normalize_print_exposure": false, "print_exposure_compensation": true}),
            "manual",
            "always",
        ),
        // Sparse legacy documents default each absent leaf to true.
        (
            serde_json::json!({"normalize_print_exposure": false}),
            "manual",
            "always",
        ),
        (
            serde_json::json!({"print_exposure_compensation": false}),
            "auto",
            "never",
        ),
    ] {
        let mut value = serde_json::json!({ "enlarger": pair });
        normalize_legacy_complete_params(&mut value).unwrap();
        let params: RuntimeParams = serde_json::from_value(value).unwrap();
        assert_eq!(params.enlarger.print_exposure_mode.as_str(), mode);
        assert_eq!(
            params.settings.print_exposure_ev_tracking.as_str(),
            tracking
        );
    }
    // Idempotent on already-canonical documents.
    let mut canonical = serde_json::to_value(RuntimeParams::default()).unwrap();
    normalize_legacy_complete_params(&mut canonical).unwrap();
    let round: RuntimeParams = serde_json::from_value(canonical).unwrap();
    assert_eq!(round.enlarger.print_exposure_mode, PrintExposureMode::Auto);
    // Both spellings together are a hard error, not a silent overwrite.
    let mut both = serde_json::json!({
        "enlarger": {
            "normalize_print_exposure": true,
            "print_exposure_compensation": false,
            "print_exposure_mode": "auto",
        }
    });
    assert!(normalize_legacy_complete_params(&mut both).is_err());
    // Legacy scanner conversion leaves are retired with the scan
    // surface: they stay untouched and fail unknown-field validation.
    let mut retired = serde_json::json!({
        "scanner": { "positive_interpretation": "coupled" }
    });
    assert!(normalize_legacy_complete_params(&mut retired).is_ok());
    assert!(serde_json::from_value::<RuntimeParams>(retired).is_err());
}
#[test]
fn legacy_cmyk_migration_is_transactional_and_idempotent() {
    // Legacy group migrates onto the canonical name.
    let mut value = serde_json::json!({
        "magazine_print_color": {"active": true, "strength": 0.75}
    });
    normalize_legacy_complete_params(&mut value).unwrap();
    let params: RuntimeParams = serde_json::from_value(value.clone()).unwrap();
    assert!(params.cmyk.active);
    assert_eq!(params.cmyk.strength, 0.75);
    assert!(value.get("magazine_print_color").is_none());
    assert_eq!(value["cmyk"]["active"], true);
    assert_eq!(value["cmyk"]["strength"], 0.75);

    // Idempotent: re-running on the migrated document is a no-op.
    let migrated = value.clone();
    normalize_legacy_complete_params(&mut value).unwrap();
    assert_eq!(value, migrated);

    // Legacy route migrates onto the base route with cmyk active.
    let mut value = serde_json::json!({
        "workflow": {"route": "input > film > scan > magazine"}
    });
    normalize_legacy_complete_params(&mut value).unwrap();
    let params: RuntimeParams = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(params.workflow.route, "input > film > scan");
    assert!(params.cmyk.active);
    assert_eq!(value["workflow"]["route"], "input > film > scan");
    assert_eq!(value["cmyk"]["active"], true);

    // Idempotent on the migrated route document.
    let migrated = value.clone();
    normalize_legacy_complete_params(&mut value).unwrap();
    assert_eq!(value, migrated);

    // Unrelated fields are preserved.
    let mut value = serde_json::json!({
        "magazine_print_color": {"active": true, "strength": 0.5},
        "io": {"output_color_space": "ProPhoto RGB"},
        "camera": {"film_format_mm": 120.0}
    });
    normalize_legacy_complete_params(&mut value).unwrap();
    let params: RuntimeParams = serde_json::from_value(value).unwrap();
    assert_eq!(params.io.output_color_space, "ProPhoto RGB");
    assert_eq!(params.camera.film_format_mm, 120.0);
    assert!(params.cmyk.active);
    assert_eq!(params.cmyk.strength, 0.5);
}

#[test]
fn legacy_cmyk_conflicts_reject_atomically() {
    // Both spellings together are a hard error.
    let mut both = serde_json::json!({
        "magazine_print_color": {"active": true, "strength": 0.75},
        "cmyk": {"active": false, "strength": 1.0}
    });
    let original = both.clone();
    let error = normalize_legacy_complete_params(&mut both).unwrap_err();
    assert!(error.contains("magazine_print_color"), "{error}");
    assert!(error.contains("cmyk"), "{error}");
    assert_eq!(both, original);

    // A legacy route and a legacy group with an explicit active leaf
    // conflict: the route implies cmyk.active = true.
    let mut conflict = serde_json::json!({
        "workflow": {"route": "input > film > scan > magazine"},
        "magazine_print_color": {"active": false}
    });
    let original = conflict.clone();
    let error = normalize_legacy_complete_params(&mut conflict).unwrap_err();
    assert!(error.contains("retired route"), "{error}");
    assert_eq!(conflict, original);

    // A non-object legacy group is rejected before mutation.
    let mut bad = serde_json::json!({"magazine_print_color": 42});
    let original = bad.clone();
    assert!(normalize_legacy_complete_params(&mut bad).is_err());
    assert_eq!(bad, original);
}
#[test]
fn legacy_conversion_rejections_leave_documents_unchanged() {
    // A mixed print-exposure pair needs the settings group; a non-object
    // there must fail before any mutation instead of panicking.
    let mut value = serde_json::json!({
        "enlarger": {"normalize_print_exposure": true, "print_exposure_compensation": false},
        "settings": 42
    });
    let original = value.clone();
    let error = normalize_legacy_conversion_paths(&mut value, true).unwrap_err();
    assert!(error.contains("settings"), "{error}");
    assert_eq!(value, original);

    // Combining legacy conversion paths with a rejected retired lightness
    // leaf must fail atomically, before any rename.
    let mut both = serde_json::json!({
        "scanner": {"positive_density_ev": 0.4},
        "enlarger": {"normalize_print_exposure": true, "print_exposure_compensation": true},
        "io": {"output_gamut_compress": {"lightness_compression": [0.95, 1.0, 1.6]}}
    });
    let original = both.clone();
    let error = normalize_legacy_complete_params(&mut both).unwrap_err();
    assert!(error.contains("lightness_compression"), "{error}");
    assert_eq!(both, original);
}

#[test]
fn complete_migration_preserves_error_precedence_without_partial_changes() {
    let mut value = serde_json::json!({
        "enlarger": {"normalize_print_exposure": "yes"},
        "magazine_print_color": {},
        "cmyk": {},
        "io": {"output_gamut_compress": {"lightness_compression": [0.95, 1.0, 1.6]}}
    });
    let original = value.clone();
    let error = normalize_legacy_complete_params_detailed(&mut value).unwrap_err();
    assert_eq!(error.reason, Reason::RetiredLightness);
    assert_eq!(value, original);

    value["io"]["output_gamut_compress"]["lightness_compression"] = Value::Null;
    let original = value.clone();
    let error = normalize_legacy_complete_params_detailed(&mut value).unwrap_err();
    assert_eq!(error.reason, Reason::LegacyExposureBoolean);
    assert_eq!(error.path, "enlarger");
    assert_eq!(value, original);
}

#[test]
fn complete_migration_composes_rules_without_filling_omissions() {
    let mut value = serde_json::json!({
        "enlarger": {"normalize_print_exposure": false, "print_exposure_compensation": true},
        "workflow": {"route": "input > film > scan > magazine"},
        "magazine_print_color": {"strength": 0.625},
        "io": {"output_gamut_compress": {"lightness_compression": null}},
        "camera": {"exposure_compensation_ev": 0.5000000000000001}
    });
    normalize_legacy_complete_params_detailed(&mut value).unwrap();
    let expected = serde_json::json!({
        "enlarger": {"print_exposure_mode": "manual"},
        "settings": {"print_exposure_ev_tracking": "always"},
        "workflow": {"route": "input > film > scan"},
        "cmyk": {"strength": 0.625, "active": true},
        "io": {"output_gamut_compress": {}},
        "camera": {"exposure_compensation_ev": 0.5000000000000001}
    });
    assert_eq!(value, expected);
    normalize_legacy_complete_params_detailed(&mut value).unwrap();
    assert_eq!(value, expected);
}
