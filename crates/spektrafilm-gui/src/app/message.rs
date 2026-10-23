//! Retained product messages, formatted only at the presentation boundary.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Message {
    Text {
        key: &'static str,
        arguments: Vec<(&'static str, Message)>,
    },
    Control(String),
    Detail(String),
}

impl Default for Message {
    fn default() -> Self {
        Self::Detail(String::new())
    }
}

#[derive(Debug)]
struct MessageContext {
    context: Message,
    source: anyhow::Error,
}
impl fmt::Display for MessageContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.context.fmt(formatter)
    }
}
impl std::error::Error for MessageContext {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl Message {
    pub(crate) fn context(self, error: impl Into<anyhow::Error>) -> anyhow::Error {
        anyhow::Error::new(MessageContext {
            context: self,
            source: error.into(),
        })
    }

    pub(crate) fn new(key: &'static str) -> Self {
        Self::Text {
            key,
            arguments: Vec::new(),
        }
    }

    pub(crate) fn arg(self, name: &'static str, value: impl ToString) -> Self {
        self.message_arg(name, Self::Detail(value.to_string()))
    }

    pub(crate) fn message_arg(mut self, name: &'static str, value: Message) -> Self {
        if let Self::Text { arguments, .. } = &mut self {
            arguments.push((name, value));
        }
        self
    }

    pub(crate) fn control(value: &str) -> Self {
        Self::Control(value.to_owned())
    }

    pub(crate) fn image_io(error: &spektrafilm_core::image_io::ImageIoError) -> Self {
        use spektrafilm_core::image_io::{ImageIoError as E, SaveOptionViolation as V};
        match error {
            E::UnsupportedFormat(value) => {
                Self::new("operations.image.unsupported_format").arg("value", value)
            }
            E::UnsupportedDepth(value) => {
                Self::new("operations.image.unsupported_depth").arg("value", value)
            }
            E::ExrDepth(value) => Self::new("operations.image.exr_depth").arg("value", value),
            E::InvalidExport { format, reason } => {
                let reason = Self::new(match reason {
                    V::Depth => "operations.image.depth",
                    V::RequiresEncoded => "operations.image.encoded",
                    V::RequiresLinear => "operations.image.linear",
                    V::ExrColorSpace => "operations.image.exr_space",
                    V::JpegQualityNotApplicable => "operations.image.jpeg_quality_only",
                    V::JpegSubsamplingNotApplicable => "operations.image.jpeg_subsampling_only",
                    V::JpegQualityRange => "operations.image.jpeg_quality_range",
                    V::CompressionNotConfigurable => "operations.image.compression_fixed",
                    V::ExrCompression => "operations.image.exr_compression",
                });
                Self::new("operations.image.invalid_export")
                    .arg("format", format)
                    .message_arg("reason", reason)
            }
            E::InvalidPath => Self::new("operations.image.invalid_path"),
            E::InvalidImage => Self::new("operations.image.invalid_image"),
            E::InvalidColorSpace(value) => {
                Self::new("operations.image.invalid_color_space").arg("detail", value)
            }
            E::Native {
                operation,
                path,
                message,
            } => {
                let key = match *operation {
                    "load" => "operations.image.load",
                    "save" => "operations.image.save",
                    "staging" => "operations.image.staging",
                    "publication" => "operations.image.publication",
                    "metadata finalization" => "operations.image.metadata_finalization",
                    _ => "operations.image.native",
                };
                Self::new(key)
                    .arg("path", path)
                    .message_arg("detail", Self::bridge_error(message))
            }
        }
    }

    pub(crate) fn bridge_error(error: &spektrafilm_core::image_io::BridgeError) -> Self {
        use spektrafilm_core::image_io::BridgeReason as R;
        let mut message = (!error.diagnostic_detail().is_empty())
            .then(|| Self::Detail(error.diagnostic_detail().to_owned()));
        for reason in error.reasons() {
            let key = match reason {
                R::OpenInput => "operations.bridge.open_input",
                R::TooSmall => "operations.bridge.too_small",
                R::Container => "operations.bridge.container",
                R::XmpInitialization => "operations.bridge.xmp_init",
                R::Dimensions => "operations.bridge.dimensions",
                R::OpenOutput => "operations.bridge.open_output",
                R::OutputPlugin => "operations.bridge.output_plugin",
                R::CheckedWrites => "operations.bridge.checked_writes",
                R::OpenContainer => "operations.bridge.open_container",
                R::WriteData => "operations.bridge.write_data",
                R::FinalizeContainer => "operations.bridge.finalize_container",
                R::FinalizeFile => "operations.bridge.finalize_file",
                R::UnreadableAfterMetadata => "operations.bridge.unreadable_metadata",
                R::InvalidAfterMetadata => "operations.bridge.invalid_metadata",
                R::IncompleteAfterMetadata => "operations.bridge.incomplete_metadata",
                R::FinalizeRead => "operations.bridge.finalize_read",
                R::Timestamp => "operations.bridge.timestamp",
                R::MissingDiagnostic => "operations.bridge.no_detail",
            };
            let cause = Self::new(key);
            message = Some(match message {
                Some(detail) => Self::new("operations.error_chain")
                    .message_arg("context", cause)
                    .message_arg("detail", detail),
                None => cause,
            });
        }
        message.unwrap_or_else(|| Self::new("operations.bridge.no_detail"))
    }

    pub(crate) fn profile(error: &spektrafilm_core::profile::ProfileError) -> Self {
        use spektrafilm_core::profile::{ProfileError as E, ProfileValidationError as V};
        use spektrafilm_model::density_curves::DensityCurveError as D;
        let reason = match error {
            E::Io(path, detail) => {
                return Self::new("validation.profile.io")
                    .arg("path", path)
                    .arg("detail", detail);
            }
            E::Parse(path, detail) => {
                return Self::new("validation.profile.parse")
                    .arg("path", path)
                    .arg("detail", detail);
            }
            E::Serialize(path, detail) => {
                return Self::new("validation.profile.serialize")
                    .arg("path", path)
                    .arg("detail", detail);
            }
            E::Validation(reason) => reason,
        };
        let detail = match reason {
            V::Unsupported {
                field,
                value,
                allowed,
            } => Self::new("validation.profile.unsupported")
                .arg("field", field)
                .arg("value", format!("{value:?}"))
                .arg("supported", allowed.join(", ")),
            V::Empty(field) => Self::new("validation.profile.empty").arg("field", field),
            V::DensityLength { actual, expected } => Self::new("validation.profile.density_length")
                .arg("actual", actual)
                .arg("expected", expected),
            V::Shape {
                field,
                rows,
                channels,
            } => Self::new("validation.profile.shape")
                .arg("field", field)
                .arg("rows", rows)
                .arg("channels", channels),
            V::BaseRows { expected } => {
                Self::new("validation.profile.base_rows").arg("expected", expected)
            }
            V::MidscaleLength { actual, expected } => {
                Self::new("validation.profile.midscale_length")
                    .arg("actual", actual)
                    .arg("expected", expected)
            }
            V::BwDensityColumns { expected } => {
                Self::new("validation.profile.bw_density_columns").arg("expected", expected)
            }
            V::BwBaseColumns { expected } => {
                Self::new("validation.profile.bw_base_columns").arg("expected", expected)
            }
            V::ColorChannels => Self::new("validation.profile.color_channels"),
            V::Suffix(value) => Self::new("validation.profile.suffix").arg("value", value),
            V::MissingStock => Self::new("validation.profile.missing_stock"),
            V::DensityModel(reason) => match reason {
                D::AlphaValue(value) => {
                    Self::new("validation.profile.alpha_value").arg("value", value)
                }
                D::ModelType(value) => {
                    Self::new("validation.profile.model_type").arg("value", format!("{value:?}"))
                }
                D::ChannelCounts => Self::new("validation.profile.channel_counts"),
                D::LayerCounts(channel) => {
                    Self::new("validation.profile.layer_counts").arg("channel", channel)
                }
                D::Sigma {
                    channel,
                    layer,
                    value,
                } => Self::new("validation.profile.sigma")
                    .arg("channel", channel)
                    .arg("layer", layer)
                    .arg("value", value),
                D::EqualLayers => Self::new("validation.profile.equal_layers"),
                D::AlphaShape => Self::new("validation.profile.alpha_shape"),
                D::AlphaBounds(value) => {
                    Self::new("validation.profile.alpha_bounds").arg("value", value)
                }
            },
        };
        Self::new("validation.profile.invalid").message_arg("detail", detail)
    }

    pub(crate) fn validation(error: &spektrafilm_core::params::ValidationError) -> Self {
        use spektrafilm_core::params::ValidationReason as R;
        let key = match error.reason {
            R::ComponentRange => "validation.component_range",
            R::SharesSum => "validation.shares_sum",
            R::GamutUnitInterval => "validation.gamut_unit_interval",
            R::GamutPositive => "validation.gamut_positive",
            R::Timer => "validation.timer",
            R::UnsupportedValue => "validation.unsupported_value",
            R::FinitePositive => "validation.finite_positive",
            R::Finite => "validation.finite",
            R::UnitInterval => "validation.unit_interval",
            R::FilmFormat => "validation.film_format",
            R::RemovedRoute => "validation.removed_route",
            R::UnknownProfile => "validation.unknown_profile",
            R::Binary => "validation.binary",
            R::DyeSupport => "validation.dye_support",
            R::EventArea => "validation.event_area",
            R::Correlation => "validation.correlation",
            R::NumericRange => "validation.numeric_range",
            R::SpectralBounds => "validation.spectral_bounds",
            R::SpectralIntervals => "validation.spectral_intervals",
            R::SpectralBundled => "validation.spectral_bundled",
            R::CalibrationToken => "validation.calibration_token",
            R::CalibrationCount => "validation.calibration_count",
            R::ColorFilter => "validation.color_filter",
            R::OperatorMode => "validation.operator_mode",
            R::ScanRoute => "validation.scan_route",
            R::StandardLut => "validation.standard_lut",
            R::StyleName => "validation.style_name",
            R::StyleMode => "validation.style_mode",
            R::SourceCompression => "validation.source_compression",
            R::RgbMethod => "validation.rgb_method",
            R::Illuminant => "validation.illuminant",
            R::ScanIlluminant => "validation.scan_illuminant",
            R::DiffusionFamily => "validation.diffusion_family",
            R::CmykStrength => "validation.cmyk_strength",
            R::Tap => "validation.tap",
            R::Route => "validation.route",
            R::OutputMix => "validation.output_mix",
            R::ColorSpace => "validation.color_space",
            R::InputAlgorithm => "validation.input_algorithm",
            R::InputBoundary => "validation.input_boundary",
            R::HullDetail => "validation.hull_detail",
            R::OutputAlgorithm => "validation.output_algorithm",
            R::LegacyGroupObject => "validation.legacy_group_object",
            R::LegacyConflict => "validation.legacy_conflict",
            R::LegacyRouteConflict => "validation.legacy_route_conflict",
            R::LegacyLamp => "validation.legacy_lamp",
            R::ObjectRequired => "validation.object_required",
            R::LegacyExposureConflict => "validation.legacy_exposure_conflict",
            R::LegacyExposureBoolean => "validation.legacy_exposure_boolean",
            R::LegacyExposureMixed => "validation.legacy_exposure_mixed",
            R::LegacySettingsObject => "validation.legacy_settings_object",
            R::RetiredLightness => "validation.retired_lightness",
            R::DocumentObject => "validation.document_object",
            R::InvalidField => "validation.invalid_field",
            R::UnknownGroup => "validation.unknown_group",
            R::ScanNegativeProfile => "validation.scan_negative_profile",
            R::ScanFiniteDensity => "validation.scan_finite_density",
            R::ScanProceduralGrain => "validation.scan_procedural_grain",
            R::NullNotAllowed => "validation.null_not_allowed",
            R::NonFinite => "validation.non_finite",
            R::LeafRequired => "validation.leaf_required",
            R::TypeMismatch => "validation.type_mismatch",
            R::F32Range => "validation.f32_range",
            R::F32Underflow => "validation.f32_underflow",
            R::Representation => "validation.representation",
            R::GamutRepresentation => "validation.gamut_representation",
            R::OutsideRange => "validation.outside_range",
            R::RetiredPath => "validation.retired_path",
            R::NonSettable => "validation.non_settable",
            R::UnknownPath => "validation.unknown_path",
            R::GroupAssignment => "validation.group_assignment",
            R::NumericFinite => "validation.numeric_finite",
            R::ArrayFinite => "validation.array_finite",
            R::SerializeParameters => "validation.serialize_parameters",
            R::ApplyParameters => "validation.apply_parameters",
            R::LegacyProvenance => "validation.legacy_provenance",
            R::NativeJson => "validation.native_json",
            R::NativeToml => "validation.native_toml",
            R::DuplicateJsonKey => "validation.duplicate_json_key",
            R::TomlNonFinite => "validation.toml_non_finite",
            R::TomlDatetime => "validation.toml_datetime",
        };
        let mut message = Self::new(key).arg("path", &error.path);
        for (name, value) in &error.arguments {
            message = if *name == "component" {
                let key = match value.as_str() {
                    "threshold" => "validation.component.threshold",
                    "limit" => "validation.component.limit",
                    "power" => "validation.component.power",
                    _ => unreachable!("registered gamut component"),
                };
                message.message_arg(name, Self::new(key))
            } else {
                message.arg(name, value)
            };
        }
        if let Some((_, suggestions)) = error
            .arguments
            .iter()
            .find(|(name, value)| *name == "suggestions" && !value.is_empty())
        {
            return Self::new("validation.suggestions")
                .message_arg("context", message)
                .arg("suggestions", suggestions);
        }
        message
    }

    pub(crate) fn error(error: &anyhow::Error) -> Self {
        fn retain(error: &(dyn std::error::Error + 'static)) -> Message {
            if let Some(error) = error.downcast_ref::<MessageContext>() {
                return Message::new("operations.error_chain")
                    .message_arg("context", error.context.clone())
                    .message_arg("detail", Message::error(&error.source));
            }
            if let Some(error) = error.downcast_ref::<spektrafilm_core::image_io::ImageIoError>() {
                return Message::image_io(error);
            }
            if let Some(error) = error.downcast_ref::<spektrafilm_core::params::ValidationError>() {
                return Message::validation(error);
            }
            if let Some(error) = error.downcast_ref::<spektrafilm_core::profile::ProfileError>() {
                return Message::profile(error);
            }
            let context = error
                .downcast_ref::<Message>()
                .cloned()
                .unwrap_or_else(|| Message::Detail(error.to_string()));
            match error.source() {
                Some(source) => Message::new("operations.error_chain")
                    .message_arg("context", context)
                    .message_arg("detail", retain(source)),
                None => context,
            }
        }
        retain(error.as_ref())
    }

    pub(crate) fn localized(&self, ctx: &egui::Context) -> String {
        self.for_locale(crate::i18n::locale(ctx))
    }

    pub(crate) fn for_locale(&self, locale: crate::i18n::Locale) -> String {
        match self {
            Self::Detail(value) => value.clone(),
            Self::Control(value) => crate::controls::localized_for(locale, value).into_owned(),
            Self::Text { key, arguments } => {
                let template = crate::i18n::text_for(locale, key);
                if arguments.is_empty() {
                    return template.into_owned();
                }
                let names: Vec<_> = arguments.iter().map(|(name, _)| *name).collect();
                let values: Vec<_> = arguments
                    .iter()
                    .map(|(_, value)| value.for_locale(locale))
                    .collect();
                rust_i18n::replace_patterns(&template, &names, &values)
            }
        }
    }
}

impl From<String> for Message {
    fn from(value: String) -> Self {
        Self::Detail(value)
    }
}
impl From<&str> for Message {
    fn from(value: &str) -> Self {
        Self::Detail(value.to_owned())
    }
}
impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.for_locale(crate::i18n::Locale::English).fmt(f)
    }
}
impl std::error::Error for Message {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_operation_relabels_without_mutating_arguments() {
        let message = Message::new("operations.rendered")
            .message_arg("operation", Message::new("operations.scan"))
            .arg("width", 12)
            .arg("height", 8)
            .arg("megapixels", "0.0");
        let retained = message.clone();
        let english = message.for_locale(crate::i18n::Locale::English);
        let chinese = message.for_locale(crate::i18n::Locale::SimplifiedChinese);
        assert_ne!(english, chinese);
        assert!(english.contains("12 × 8"));
        assert!(chinese.contains("12 × 8"));
        assert_eq!(message, retained);
    }

    #[test]
    fn raw_library_detail_survives_localized_context() {
        let detail = "io error 13: /tmp/用户-input.tiff";
        let error = Message::new("operations.state.json").context(anyhow::anyhow!(detail));
        let message = Message::error(&error);
        let english = message.for_locale(crate::i18n::Locale::English);
        let chinese = message.for_locale(crate::i18n::Locale::SimplifiedChinese);
        assert_ne!(english, chinese);
        assert!(english.contains(detail));
        assert!(chinese.contains(detail));
    }

    #[test]
    fn typed_image_errors_relabel_and_preserve_exact_values() {
        use spektrafilm_core::image_io::{ImageIoError, SaveOptionViolation};
        let errors = [
            ImageIoError::UnsupportedFormat("unsupported-用户".into()),
            ImageIoError::UnsupportedDepth(12),
            ImageIoError::ExrDepth(8),
            ImageIoError::InvalidExport {
                format: "PNG",
                reason: SaveOptionViolation::RequiresEncoded,
            },
            ImageIoError::InvalidPath,
            ImageIoError::InvalidImage,
            ImageIoError::InvalidColorSpace("rejected-space".into()),
            ImageIoError::Native {
                operation: "publication",
                path: "/tmp/用户.png".into(),
                message: "OS error 13".into(),
            },
        ];
        for error in errors {
            let message = Message::error(&anyhow::Error::new(error));
            assert_ne!(
                message.for_locale(crate::i18n::Locale::English),
                message.for_locale(crate::i18n::Locale::SimplifiedChinese)
            );
        }
        let message = Message::image_io(&ImageIoError::Native {
            operation: "save",
            path: "/tmp/用户.png".into(),
            message: "OS error 13".into(),
        });
        for locale in [
            crate::i18n::Locale::English,
            crate::i18n::Locale::SimplifiedChinese,
        ] {
            let text = message.for_locale(locale);
            assert!(text.contains("/tmp/用户.png"));
            assert!(text.contains("OS error 13"));
        }
    }

    #[test]
    fn authored_bridge_failures_relabel_without_interpreting_diagnostic_prose() {
        let directory = tempfile::tempdir().unwrap();
        for (name, bytes) in [
            ("空.png", &b""[..]),
            ("随机.png", &b"not an image container"[..]),
        ] {
            let path = directory.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            let error = match spektrafilm_core::image_io::load(&path) {
                Err(error) => error,
                Ok(_) => panic!("invalid image unexpectedly loaded"),
            };
            let message = Message::image_io(&error);
            let english = message.for_locale(crate::i18n::Locale::English);
            let chinese = message.for_locale(crate::i18n::Locale::SimplifiedChinese);
            assert_ne!(english, chinese);
            assert!(english.contains(name));
            assert!(chinese.contains(name));
            assert!(!chinese.contains("File is too small"));
            assert!(!chinese.contains("Unrecognized image container"));
        }
    }

    #[test]
    fn nested_localized_context_keeps_operation_path_and_typed_leaf() {
        let leaf =
            spektrafilm_core::image_io::ImageIoError::UnsupportedFormat("拒绝-format".into());
        let error = Message::new("operations.save_error").context(leaf);
        let error = Message::new("operations.state.read")
            .arg("path", "/tmp/用户-input.tiff")
            .context(error);
        let message = Message::error(&error);
        let english = message.for_locale(crate::i18n::Locale::English);
        let chinese = message.for_locale(crate::i18n::Locale::SimplifiedChinese);
        assert_ne!(english, chinese);
        for text in [&english, &chinese] {
            assert!(text.contains("/tmp/用户-input.tiff"));
            assert!(text.contains("拒绝-format"));
        }
        assert!(
            chinese.contains(
                &crate::i18n::text_for(
                    crate::i18n::Locale::SimplifiedChinese,
                    "operations.save_error"
                )
                .replace("%{detail}", "")
            )
        );
        assert!(!chinese.contains("Unsupported image extension"));
    }
}
