use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn profile() -> EmbeddedProfile {
    // A deterministic in-memory material avoids mutable external libraries.
    let mut profile: crate::profile::Profile = serde_json::from_value(json!({
        "metadata": {}, "info": {},
        "data": {
            "wavelengths": (0..81).map(|i|380.0 + 5.0 * i as f64).collect::<Vec<_>>(),
            "log_sensitivity": vec![vec![0.0;3];81],
            "channel_density": vec![vec![1.0,0.5,0.25];81],
            "base_density": vec![0.1;81],
            "log_exposure": [-1.0,1.0],
            "density_curves": [[0.1,0.1,0.1],[1.0,1.0,1.0]]
        }
    })).unwrap();
    profile.data.base_density = vec![0.1; 81];
    EmbeddedProfile::new(profile).unwrap()
}

fn state() -> MaterialState {
    let mut baseline = RuntimeParams::default();
    baseline.workflow.route = "input > film".into();
    let mut header = StageHeader::new(
        MaterialKind::Film,
        baseline,
        profile(),
        None,
        None,
        StageGeometry {
            source_width: 3,
            source_height: 2,
            pixel_size_um: 6.0,
            pending_crop: None,
            print_spatial_completed: false,
        },
        InterpretationContext {
            exposure: StageExposure {
                auto_enabled: false,
                method: "center_weighted".into(),
                metered_ev: 0.0,
            },
            camera_compensation_ev: 0.75,
            print_exposure_factor: 1.0,
            print_illuminant: Vec::new(),
            preflash_raw: [0.0; 3],
            midgray_density: vec![0.5; 81],
            compensated_midgray_density: vec![0.7; 81],
            prepared_source_color_space: None,
        },
        42,
    );
    header.provenance.backend = "cpu".into();
    header.provenance.input_identity = "sha256:fixture".into();
    MaterialState {
        header,
        density: ImageBuf::from_data(3, 2, (0..18).map(|i| (i as f64 / 8.0 - 0.5) as _).collect()),
        prepared_source: None,
    }
}

fn edit_manifest(path: &Path, f: impl FnOnce(&mut Value)) {
    let manifest = path.join("stage.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    f(&mut value);
    std::fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn moved_artifact_preserves_values_profiles_and_seed_without_external_sources() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original.sfstage");
    let expected = state();
    write(&original, &expected).unwrap();
    let moved = root.path().join("moved.sfstage");
    std::fs::rename(original, &moved).unwrap();
    let loaded = read(&moved).unwrap();
    assert_eq!(loaded.density.width, 3);
    assert_eq!(loaded.density.height, 2);
    assert_eq!(loaded.density.data, expected.density.data);
    assert_eq!(loaded.header.seed, 42);
    assert_eq!(loaded.header.baseline.random_seed, 42);
    assert_eq!(loaded.header.film.profile.data.base_density, vec![0.1; 81]);
    assert_eq!(loaded.header.context.camera_compensation_ev, 0.75);
    assert!(loaded.header.storage_conversion.is_none());
}

#[test]
fn native_f32_and_f64_preserve_bits_and_report_actual_conversion() {
    let root = tempfile::tempdir().unwrap();
    for (name, samples) in [
        (
            "f32.sfstage",
            NativeSamples::F32(vec![-0.0, f32::from_bits(1), 1.125]),
        ),
        (
            "f64.sfstage",
            NativeSamples::F64(vec![-0.0, f64::from_bits(1), 1.125]),
        ),
    ] {
        let path = root.path().join(name);
        let raster = NativeRaster {
            width: 1,
            height: 1,
            samples,
        };
        write_native(&path, &state().header, &raster, None).unwrap();
        let header = read_header(&path).unwrap();
        let pinned = publication::root(&path).unwrap();
        let restored = payload::read_raster(&pinned, header.density.as_ref().unwrap(), true).unwrap();
        match (&raster.samples, restored.samples) {
            (NativeSamples::F32(a), NativeSamples::F32(b)) => assert_eq!(
                a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            ),
            (NativeSamples::F64(a), NativeSamples::F64(b)) => assert_eq!(
                a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            ),
            _ => panic!("storage precision changed"),
        }
        let loaded = read(&path).unwrap();
        assert_eq!(
            loaded.header.storage_conversion.is_some(),
            raster.scalar_type() != ScalarType::computation()
        );
        assert_eq!(
            loaded.header.provenance.computation_precision,
            ScalarType::computation()
        );
    }
}

#[test]
fn header_inspection_does_not_claim_unchecked_payload_integrity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("film.sfstage");
    write(&path, &state()).unwrap();
    let payload = path.join("density.bin");
    let mut bytes = std::fs::read(&payload).unwrap();
    bytes[0] ^= 1;
    std::fs::write(&payload, bytes).unwrap();
    assert!(read_header(&path).is_ok());
    assert!(
        read(&path)
            .unwrap_err()
            .to_string()
            .contains("SHA-256 mismatch")
    );
}

#[test]
fn malformed_envelopes_and_missing_model_state_fail_before_payload_use() {
    let root = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, Box<dyn FnOnce(&mut Value)>)> = vec![
        ("schema", Box::new(|v| v["schema_version"] = json!(2))),
        (
            "representation",
            Box::new(|v| v["representation_version"] = json!(2)),
        ),
        ("unknown", Box::new(|v| v["surprise"] = json!(true))),
        (
            "nested_unknown",
            Box::new(|v| v["film"]["profile"]["info"]["surprise"] = json!(true)),
        ),
        (
            "missing_context",
            Box::new(|v| {
                v["context"]
                    .as_object_mut()
                    .unwrap()
                    .remove("midgray_density");
            }),
        ),
        (
            "missing_parameter",
            Box::new(|v| {
                v["baseline"]["camera"]
                    .as_object_mut()
                    .unwrap()
                    .remove("exposure_compensation_ev");
            }),
        ),
        (
            "null_geometry",
            Box::new(|v| v["geometry"]["pixel_size_um"] = Value::Null),
        ),
        (
            "profile_nan",
            Box::new(|v| v["film"]["profile"]["data"]["log_sensitivity"][0][0] = Value::Null),
        ),
        (
            "unsafe",
            Box::new(|v| v["density"]["path"] = json!("../density.bin")),
        ),
        (
            "absolute",
            Box::new(|v| v["density"]["path"] = json!("/tmp/density.bin")),
        ),
        (
            "windows",
            Box::new(|v| v["density"]["path"] = json!("C:\\density.bin")),
        ),
        (
            "layout",
            Box::new(|v| v["density"]["channel_order"] = json!("RGB")),
        ),
        (
            "overflow",
            Box::new(|v| {
                v["density"]["width"] = json!(u32::MAX);
                v["density"]["height"] = json!(u32::MAX);
            }),
        ),
        (
            "crop",
            Box::new(|v| v["geometry"]["pending_crop"] = json!([2, 1, 2, 2])),
        ),
        (
            "profile_hash",
            Box::new(|v| v["film"]["resolved_base_density"][0] = json!(0.2)),
        ),
    ];
    for (name, mutation) in cases {
        let path = root.path().join(name);
        write(&path, &state()).unwrap();
        edit_manifest(&path, mutation);
        assert!(
            read_header(&path).is_err(),
            "accepted invalid artifact {name}"
        );
    }
}

#[test]
fn duplicate_escaped_keys_are_not_silently_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("film.sfstage");
    write(&path, &state()).unwrap();
    let manifest = path.join("stage.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let duplicate = text.replacen(
        "\"schema_version\": 1",
        "\"schema_version\": 2, \"schema_\\u0076ersion\": 1",
        1,
    );
    std::fs::write(manifest, duplicate).unwrap();
    assert!(
        read_header(&path)
            .unwrap_err()
            .to_string()
            .contains("duplicate JSON")
    );
}

#[test]
fn truncated_and_hash_consistent_nonfinite_payloads_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("film.sfstage");
    write(&path, &state()).unwrap();
    let payload = path.join("density.bin");
    let bytes = std::fs::read(&payload).unwrap();
    std::fs::write(&payload, &bytes[..bytes.len() - 1]).unwrap();
    assert!(read_header(&path).is_err());
    let mut poisoned = bytes;
    #[cfg(feature = "precision-f64")]
    poisoned[..8].copy_from_slice(&f64::NAN.to_le_bytes());
    #[cfg(not(feature = "precision-f64"))]
    poisoned[..4].copy_from_slice(&f32::INFINITY.to_le_bytes());
    std::fs::write(&payload, &poisoned).unwrap();
    edit_manifest(&path, |v| {
        v["density"]["sha256"] = json!(format!("{:x}", Sha256::digest(&poisoned)))
    });
    assert!(read(&path).unwrap_err().to_string().contains("non-finite"));
}

#[test]
fn failed_write_and_destination_race_preserve_existing_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("film.sfstage");
    let mut invalid = state();
    invalid.density.data[0] = f64::NAN as _;
    assert!(write(&path, &invalid).is_err());
    assert!(!path.exists());
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        0,
        "failed publication left staging files"
    );
    validate_destination(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("sentinel"), b"preserve").unwrap();
    let staging = tempfile::Builder::new()
        .prefix(".sfstage-staging-")
        .tempdir_in(root.path())
        .unwrap();
    std::fs::write(staging.path().join("stage.json"), b"not published").unwrap();
    assert!(publication::publish(staging.path(), &path).is_err());
    assert_eq!(std::fs::read(path.join("sentinel")).unwrap(), b"preserve");
    assert!(!path.join("stage.json").exists());
    assert!(read_header(staging.path()).is_err());
    assert!(write(&path, &state()).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_members_and_symlink_ancestors_never_escape_artifact_root() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    for nested in [false, true] {
        let path = root.path().join(if nested { "nested" } else { "direct" });
        write(&path, &state()).unwrap();
        let external = root
            .path()
            .join(if nested { "external2" } else { "external1" });
        std::fs::create_dir(&external).unwrap();
        std::fs::rename(path.join("density.bin"), external.join("density.bin")).unwrap();
        if nested {
            symlink(&external, path.join("members")).unwrap();
            edit_manifest(&path, |v| {
                v["density"]["path"] = json!("members/density.bin")
            });
        } else {
            symlink(external.join("density.bin"), path.join("density.bin")).unwrap();
        }
        assert!(read_header(&path).is_err());
        assert!(read(&path).is_err());
    }
}

#[test]
fn prepared_source_is_a_real_aligned_conditional_dependency() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mixed.sfstage");
    let mut material = state();
    material.header.baseline.output_mix.amount = 0.5;
    assert!(write(&path, &material).is_err());
    material.prepared_source = Some(material.density.clone());
    material.header.context.prepared_source_color_space = Some("sRGB".into());
    write(&path, &material).unwrap();
    assert_eq!(
        read(&path).unwrap().prepared_source.unwrap().data,
        material.density.data
    );
    std::fs::remove_file(path.join("prepared-source.bin")).unwrap();
    assert!(read_header(&path).is_err());
}

#[test]
fn partial_member_write_failure_never_creates_completed_manifest() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("destination.sfstage");
    let staging = tempfile::Builder::new()
        .prefix(".sfstage-staging-")
        .tempdir_in(root.path())
        .unwrap();
    // An existing member forces create_new to fail inside the actual writer.
    std::fs::write(staging.path().join("density.bin"), b"sentinel").unwrap();
    let raster = NativeRaster {
        width: 1,
        height: 1,
        samples: NativeSamples::F32(vec![0.0; 3]),
    };
    assert!(payload::write_raster(staging.path(), "density.bin", &raster, true).is_err());
    assert_eq!(
        std::fs::read(staging.path().join("density.bin")).unwrap(),
        b"sentinel"
    );
    assert!(!staging.path().join("stage.json").exists());
    assert!(!path.exists());
    assert!(read_header(staging.path()).is_err());
}

#[cfg(unix)]
#[test]
fn destination_symlink_race_preserves_link_and_target() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("destination.sfstage");
    validate_destination(&destination).unwrap();
    let target = root.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("sentinel"), b"preserve").unwrap();
    symlink(&target, &destination).unwrap();
    let staging = tempfile::tempdir_in(root.path()).unwrap();
    assert!(publication::publish(staging.path(), &destination).is_err());
    assert!(
        std::fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(target.join("sentinel")).unwrap(), b"preserve");
    assert!(!target.join("stage.json").exists());
}

#[test]
fn nullable_profile_measurements_and_resolved_monochrome_are_not_rgb() {
    let root = tempfile::tempdir().unwrap();
    let mut material = state();
    let mut source = material.header.film.profile.clone();
    source.info.channel_model = "bw".into();
    source.data.log_sensitivity = vec![vec![0.0];81];
    source.data.channel_density = vec![vec![1.0];81];
    source.data.density_curves = vec![vec![0.1],vec![1.0]];
    source.data.base_density[0] = f64::NAN;
    source.data.base_density_rows[0][0] = f64::NAN;
    let prepared = crate::profile::resolve_for_render(source.clone(), None);
    material.header.film = EmbeddedProfile::with_source(prepared, &source).unwrap();
    material.header.context.midgray_density[0] = f64::NAN;
    let path = root.path().join("monochrome.sfstage");
    write(&path, &material).unwrap();
    let loaded = read(&path).unwrap();
    assert_eq!(
        loaded.header.density.as_ref().unwrap().channel_order,
        "monochrome_record_replicated"
    );
    assert!(loaded.header.film.profile.data.base_density[0].is_nan());
    assert!(loaded.header.context.midgray_density[0].is_nan());
    assert!(loaded.header.film.matches_source(&source).unwrap());
    source.data.density_curves[0][0] += 0.1;
    assert!(!loaded.header.film.matches_source(&source).unwrap());
}

#[test]
fn retained_source_grid_matches_pending_delivered_crop_not_full_film() {
    let root = tempfile::tempdir().unwrap();
    let mut material = state();
    material.header.geometry.pending_crop = Some([1, 0, 2, 1]);
    material.prepared_source = Some(ImageBuf::from_data(2, 1, vec![0.25; 6]));
    material.header.context.prepared_source_color_space = Some("sRGB".into());
    let path = root.path().join("cropped.sfstage");
    write(&path, &material).unwrap();
    let loaded = read(&path).unwrap();
    assert_eq!((loaded.density.width, loaded.density.height), (3, 2));
    assert_eq!(
        (
            loaded.prepared_source.as_ref().unwrap().width,
            loaded.prepared_source.as_ref().unwrap().height
        ),
        (2, 1)
    );
    edit_manifest(&path, |v| {
        v["context"]["prepared_source_color_space"] = Value::Null
    });
    assert!(read_header(&path).is_err());
}

#[test]
fn source_identity_is_distinct_from_prepared_material_identity() {
    let source = profile().profile;
    let mut prepared = source.clone();
    prepared.data.density_curves[0][0] += 0.3;
    let embedded = EmbeddedProfile::with_source(prepared, &source).unwrap();
    assert!(embedded.matches_source(&source).unwrap());
    assert_ne!(embedded.sha256, embedded.source_sha256);
    let mut changed_source = source;
    changed_source.data.density_curves[0][0] += 0.1;
    assert!(!embedded.matches_source(&changed_source).unwrap());
}

#[test]
fn continuation_keeps_original_seed_and_effective_parameters_as_provenance() {
    let root = tempfile::tempdir().unwrap();
    let mut material = state();
    let original = StageLineage {
        seed: material.header.seed,
        provenance: material.header.provenance.clone(),
        effective_parameters: material.header.baseline.clone(),
    };
    material.header.lineage.push(original);
    material.header.seed = 84;
    // Model the newly produced print envelope without invoking pipeline math.
    material.header.state = MaterialKind::Print;
    material.header.boundary = "cmy_print".into();
    material.header.baseline.workflow.route = "film > print".into();
    material.header.provenance.producing_route = "film > print".into();
    material.header.provenance.inherited_operations = vec!["film".into()];
    let mut print = material.header.film.profile.clone();
    print.info.stage = "printing".into();
    print.info.support = "paper".into();
    material.header.print = Some(EmbeddedProfile::new(print).unwrap());
    material.header.context.print_illuminant = vec![1.0; 81];
    let path = root.path().join("print.sfstage");
    write(&path, &material).unwrap();
    let loaded = read(&path).unwrap();
    assert_eq!(loaded.header.seed, 84);
    assert_eq!(loaded.header.lineage[0].seed, 42);
    assert_eq!(
        loaded.header.lineage[0].effective_parameters.workflow.route,
        "input > film"
    );
    assert_eq!(
        loaded.header.lineage[0].provenance.producing_route,
        "input > film"
    );
    edit_manifest(&path, |v| v["lineage"] = json!([]));
    assert!(read_header(&path).is_err());
}

#[cfg(not(feature = "precision-f64"))]
#[test]
fn narrowing_f64_to_f32_rejects_overflow_instead_of_inventing_finite_density() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("wide.sfstage");
    let raster = NativeRaster {
        width: 1,
        height: 1,
        samples: NativeSamples::F64(vec![f64::MAX, 0.0, 1.0]),
    };
    write_native(&path, &state().header, &raster, None).unwrap();
    assert!(read_header(&path).is_ok());
    assert!(read(&path).unwrap_err().to_string().contains("exceed f32"));
}

#[test]
fn deferred_print_name_never_becomes_a_frozen_material_dependency() {
    let root = tempfile::tempdir().unwrap();
    let mut material = state();
    material.header.downstream_print_name = Some("not-installed-future-print".into());
    let path = root.path().join("film.sfstage");
    write(&path, &material).unwrap();
    let loaded = read(&path).unwrap();
    assert_eq!(loaded.header.downstream_print_name.as_deref(), Some("not-installed-future-print"));
    assert!(loaded.header.print.is_none());
    assert!(loaded.header.original_print.is_none());
    edit_manifest(&path, |v| v["downstream_print_name"] = json!("  "));
    assert!(read_header(&path).is_err());
}

#[test]
fn prepared_print_requires_complete_illumination_but_raw_selection_does_not() {
    let root = tempfile::tempdir().unwrap();
    let mut material = state();
    let mut print = material.header.film.profile.clone();
    print.info.stage = "printing".into();
    print.info.support = "paper".into();
    let selection = EmbeddedProfile::new(print).unwrap();
    material.header.original_print = Some(selection.clone());
    let selection_only = root.path().join("selection.sfstage");
    write(&selection_only, &material).unwrap();
    assert!(read(&selection_only).is_ok());
    material.header.print = Some(selection);
    let prepared = root.path().join("prepared.sfstage");
    assert!(write(&prepared, &material).is_err());
    assert!(!prepared.exists());
    material.header.context.print_illuminant = vec![1.0;81];
    write(&prepared, &material).unwrap();
    edit_manifest(&prepared, |v| v["context"]["print_illuminant"] = json!([]));
    assert!(read_header(&prepared).is_err());
}

#[test]
fn profile_axes_cannot_relabel_a_different_spectral_sampling_grid() {
    for wavelengths in [vec![400.0,500.0,600.0], (0..81).map(|i|381.0 + i as f64 * 5.0).collect()] {
        let mut wrong = profile().profile;
        let count = wavelengths.len();
        wrong.data.wavelengths = wavelengths;
        wrong.data.log_sensitivity = vec![vec![0.0;3];count];
        wrong.data.channel_density = vec![vec![1.0;3];count];
        wrong.data.base_density_rows = vec![vec![0.1];count];
        wrong.data.base_density = vec![0.1;count];
        assert!(EmbeddedProfile::new(wrong).unwrap_err().to_string().contains("wavelength axis"));
    }
}

#[cfg(unix)]
#[test]
fn pinned_root_survives_path_and_ancestor_symlink_substitution() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    std::fs::create_dir(&parent).unwrap();
    let path = parent.join("film.sfstage");
    let original = state();
    write(&path, &original).unwrap();
    let pinned = publication::root(&path).unwrap();
    let header = read_header_from_root(&pinned).unwrap();
    let moved_parent = root.path().join("original-parent");
    std::fs::rename(&parent, &moved_parent).unwrap();
    let outside = root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let mut different = state();
    different.header.seed = 99;
    different.density.data.fill(9.0);
    write(outside.join("film.sfstage"), &different).unwrap();
    symlink(&outside, &parent).unwrap();
    // Existing read uses the acquired directory object, not either pathname.
    assert_eq!(read_header_from_root(&pinned).unwrap().seed, 42);
    let (density, _) = payload::read_raster(&pinned, header.density.as_ref().unwrap(), true).unwrap().into_image().unwrap();
    assert_eq!(density.data, original.density.data);
    // A new read refuses to follow the substituted ancestor at acquisition.
    assert!(read_header(&path).is_err());
    let moved = moved_parent.join("film.sfstage");
    std::fs::rename(&moved, moved_parent.join("original.sfstage")).unwrap();
    symlink(outside.join("film.sfstage"), &moved).unwrap();
    assert_eq!(read_header_from_root(&pinned).unwrap().seed, 42);
    assert!(read_header(&moved).is_err());
}
