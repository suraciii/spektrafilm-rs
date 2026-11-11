//! Localization adapters from core preset errors to GUI messages.
//!
//! Rendering and library actions consume these messages; neither own the
//! mapping from document/library failure variants to user-facing keys.

pub(super) fn document_error_message(
    error: &spektrafilm_core::presets::PresetDocumentError,
) -> crate::app::Message {
    use spektrafilm_core::presets::PresetDocumentReason;
    match &error.reason {
        PresetDocumentReason::Parser { detail } => {
            crate::app::Message::new("surfaces.presets.document_invalid").arg("detail", detail)
        }
        PresetDocumentReason::Carrier { path } => {
            crate::app::Message::new("surfaces.presets.carrier_invalid").arg("path", path.display())
        }
        PresetDocumentReason::Schema { version } => {
            crate::app::Message::new("surfaces.presets.schema_version").arg("version", version)
        }
        PresetDocumentReason::Name { name } => {
            crate::app::Message::new("surfaces.presets.name_guidance").arg("name", name)
        }
        PresetDocumentReason::ProvenanceVersions => {
            crate::app::Message::new("surfaces.presets.provenance_versions")
        }
        PresetDocumentReason::Shape => crate::app::Message::new("surfaces.presets.shape"),
        PresetDocumentReason::Stock { kind, stock } => {
            crate::app::Message::new("surfaces.presets.stock_invalid")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
        }
        PresetDocumentReason::ContentIdentity { kind, identity } => {
            crate::app::Message::new("surfaces.presets.identity_invalid")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("identity", identity)
        }
        PresetDocumentReason::ScannerProvenance => {
            crate::app::Message::new("surfaces.presets.scanner_provenance")
        }
        PresetDocumentReason::RootObject => {
            crate::app::Message::new("surfaces.presets.root_object")
        }
        PresetDocumentReason::ParametersObject => {
            crate::app::Message::new("surfaces.presets.parameters_object")
        }
        PresetDocumentReason::LegacyField { field } => {
            crate::app::Message::new("surfaces.presets.legacy_field").arg("field", field)
        }
        PresetDocumentReason::UnownedField { path } => {
            crate::app::Message::new("surfaces.presets.unowned_field").arg("path", path)
        }
        PresetDocumentReason::RetiredField { path, replacement } => {
            crate::app::Message::new("surfaces.presets.retired_field")
                .arg("path", path)
                .arg("replacement", replacement)
        }
        PresetDocumentReason::EmptySection => {
            crate::app::Message::new("surfaces.presets.empty_section")
        }
        PresetDocumentReason::MultipleSections => {
            crate::app::Message::new("surfaces.presets.multiple_sections")
        }
        PresetDocumentReason::TomlNull { paths } => {
            crate::app::Message::new("surfaces.presets.toml_null").arg("paths", paths.join(", "))
        }
        PresetDocumentReason::ExpectedLook => {
            crate::app::Message::new("surfaces.presets.expected_look")
        }
        PresetDocumentReason::ExpectedSection => {
            crate::app::Message::new("surfaces.presets.expected_section")
        }
        PresetDocumentReason::Profile { kind, stock, error } => {
            crate::app::Message::new("surfaces.presets.profile_load")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
                .message_arg("detail", crate::app::Message::profile(error.as_ref()))
        }
        PresetDocumentReason::ProfileRead {
            stock,
            path,
            detail,
        } => crate::app::Message::new("surfaces.presets.profile_read")
            .arg("stock", stock)
            .arg("path", path.display())
            .arg("detail", detail),
        PresetDocumentReason::Read { path, detail } => {
            crate::app::Message::new("surfaces.presets.document_read")
                .arg("path", path.display())
                .arg("detail", detail)
        }
        PresetDocumentReason::ProfileIdentity {
            stock,
            expected,
            actual,
        } => crate::app::Message::new("surfaces.presets.profile_identity")
            .arg("stock", stock)
            .arg("expected", expected)
            .arg("actual", actual),
        PresetDocumentReason::ProfileStock { kind, stock } => {
            crate::app::Message::new("surfaces.presets.profile_stock")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
        }
        PresetDocumentReason::ProfileSupport { kind, stock } => {
            crate::app::Message::new("surfaces.presets.profile_support")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
        }
        PresetDocumentReason::ProfileVersion {
            kind,
            stock,
            expected,
            actual,
        } => crate::app::Message::new("surfaces.presets.profile_version")
            .message_arg("kind", crate::app::Message::control(kind))
            .arg("stock", stock)
            .arg("expected", expected)
            .arg("actual", actual),
        PresetDocumentReason::ProfileUnverified { kind, stock } => {
            crate::app::Message::new("surfaces.presets.profile_unverified")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
        }
        PresetDocumentReason::Runtime(error) => crate::app::Message::validation(error),
    }
}

pub(super) fn library_error_message(
    error: &spektrafilm_core::presets::LibraryError,
) -> crate::app::Message {
    use spektrafilm_core::presets::LibraryErrorReason;
    match &error.reason {
        LibraryErrorReason::Document { path, error } => match path {
            Some(path) => crate::app::Message::new("surfaces.presets.fault")
                .arg("path", path.display())
                .message_arg("detail", document_error_message(error)),
            None => document_error_message(error),
        },
        LibraryErrorReason::Fault { path, fault } => {
            crate::app::Message::new("surfaces.presets.fault")
                .arg("path", path.display())
                .message_arg("detail", library_fault_message(Some(fault)))
        }
        LibraryErrorReason::NameConflict { name } => {
            crate::app::Message::new("surfaces.presets.duplicate_name").arg("name", name)
        }
        LibraryErrorReason::UnknownName { name, suggestions } => {
            crate::app::Message::new("surfaces.presets.unknown_name")
                .arg("name", name)
                .arg("suggestions", suggestions.join(", "))
        }
        LibraryErrorReason::MissingPath => {
            crate::app::Message::new("surfaces.presets.missing_path")
        }
        LibraryErrorReason::ExpectedLook { path } => {
            crate::app::Message::new("surfaces.presets.expected_look").arg("path", path.display())
        }
        LibraryErrorReason::Io {
            operation,
            path,
            detail,
        } => crate::app::Message::new(match *operation {
            "create" => "surfaces.presets.io_create",
            "resolve" => "surfaces.presets.io_resolve",
            "open_lock" => "surfaces.presets.io_open_lock",
            "lock" => "surfaces.presets.io_lock",
            "rename" => "surfaces.presets.io_rename",
            "remove" => "surfaces.presets.io_remove",
            "write" => "surfaces.presets.io_write",
            _ => unreachable!("library IO operation is declared at its owner"),
        })
        .arg("path", path.display())
        .arg("detail", detail),
        LibraryErrorReason::ExportName { path, name } => {
            crate::app::Message::new("surfaces.presets.export_name")
                .arg("path", path.display())
                .arg("name", name)
        }
    }
}

pub(super) fn library_fault_message(
    fault: Option<&spektrafilm_core::presets::LibraryFault>,
) -> crate::app::Message {
    use spektrafilm_core::presets::LibraryFault;
    match fault {
        Some(LibraryFault::DirectoryRead { detail }) => {
            crate::app::Message::new("surfaces.presets.directory_read").arg("detail", detail)
        }
        Some(LibraryFault::Document { detail }) => document_error_message(detail),
        Some(LibraryFault::NameMismatch { name, stem }) => {
            crate::app::Message::new("surfaces.presets.name_mismatch")
                .arg("name", name)
                .arg("stem", stem)
        }
        Some(LibraryFault::DuplicateName { name }) => {
            crate::app::Message::new("surfaces.presets.duplicate_library_name").arg("name", name)
        }
        None => crate::app::Message::new("surfaces.presets.invalid_document"),
    }
}
