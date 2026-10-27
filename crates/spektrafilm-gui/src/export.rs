//! Full-render export preferences and a cancellable options draft.
//! Search locates this draft's real controls without changing its format;
//! inapplicable codec values remain readable, and only submission commits it.
use std::path::{Path, PathBuf};

use crate::controls::{self, enum_choice};
use crate::theme;
use eframe::egui;
use serde_json::{Value, json};
use spektrafilm_core::image_io::{
    BitDepth, Compression, ImageFormat, JpegSubsampling, SaveOptions,
};

const COLOR_SPACES: &[&str] = &[
    "sRGB",
    "DCI-P3",
    "Display P3",
    "Adobe RGB (1998)",
    "ITU-R BT.2020",
    "ProPhoto RGB",
    "ACES2065-1",
    "DaVinci Wide Gamut",
    "V-Gamut",
];
const EXR_COLOR_SPACES: &[&str] = &["sRGB", "ACES2065-1"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExportBackend {
    #[default]
    Cpu,
    Gpu,
}
impl ExportBackend {
    pub(crate) fn argument(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cpu => {
                if cfg!(feature = "precision-f64") {
                    "CPU (f64)"
                } else {
                    "CPU (f32)"
                }
            }
            Self::Gpu => "GPU (WGPU f32)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExportFormat {
    Jpeg,
    #[default]
    Png,
    Tiff,
    Exr,
}
impl ExportFormat {
    pub(crate) fn image_format(self) -> ImageFormat {
        match self {
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Png => ImageFormat::Png,
            Self::Tiff => ImageFormat::Tiff,
            Self::Exr => ImageFormat::Exr,
        }
    }
    pub(crate) fn argument(self) -> &'static str {
        self.image_format().name()
    }
    /// Extension appended when the destination carries none. TIFF keeps the
    /// longer spelling here (not core's first loader extension `tif`).
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Exr => "exr",
        }
    }
    pub(crate) fn extensions(self) -> &'static [&'static str] {
        self.image_format().extensions()
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Tiff => "TIFF",
            Self::Exr => "EXR",
        }
    }
    pub(crate) fn output_path(self, path: &Path) -> PathBuf {
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                self.extensions()
                    .iter()
                    .any(|legal| ext.eq_ignore_ascii_case(legal))
            })
        {
            path.to_path_buf()
        } else {
            path.with_extension(self.extension())
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExportCompression {
    #[default]
    Zip,
    None,
}
impl ExportCompression {
    pub(crate) fn argument(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExportOptions {
    pub(crate) backend: ExportBackend,
    pub(crate) format: ExportFormat,
    pub(crate) depth: BitDepth,
    pub(crate) jpeg_quality: u8,
    pub(crate) jpeg_subsampling: JpegSubsampling,
    pub(crate) compression: ExportCompression,
    pub(crate) saving_color_space: String,
    pub(crate) saving_cctf_encoding: bool,
}

impl ExportOptions {
    pub(crate) fn from_state(state: &Value) -> Self {
        let rust = &state["rust"];
        let mut options = Self {
            backend: if rust["export_backend"].as_str() == Some("gpu") {
                ExportBackend::Gpu
            } else {
                ExportBackend::Cpu
            },
            format: match rust["export_format"].as_str() {
                Some("jpeg") => ExportFormat::Jpeg,
                Some("tiff") => ExportFormat::Tiff,
                Some("exr") => ExportFormat::Exr,
                _ => ExportFormat::Png,
            },
            depth: match rust["save_bit_depth"].as_u64() {
                Some(8) => BitDepth::Eight,
                Some(32) => BitDepth::ThirtyTwo,
                _ => BitDepth::Sixteen,
            },
            jpeg_quality: rust["jpeg_quality"].as_u64().unwrap_or(95).clamp(1, 100) as u8,
            jpeg_subsampling: if rust["jpeg_subsampling"].as_str() == Some("420") {
                JpegSubsampling::Yuv420
            } else {
                JpegSubsampling::Yuv444
            },
            compression: if rust["export_compression"].as_str() == Some("none") {
                ExportCompression::None
            } else {
                ExportCompression::Zip
            },
            saving_color_space: rust["export_saving_color_space"]
                .as_str()
                .or_else(|| state["simulation"]["saving_color_space"].as_str())
                .unwrap_or("sRGB")
                .to_owned(),
            saving_cctf_encoding: rust["export_saving_cctf_encoding"]
                .as_bool()
                .or_else(|| state["simulation"]["saving_cctf_encoding"].as_bool())
                .unwrap_or(true),
        };
        options.normalize();
        options
    }

    /// Apply the same container rules as core image I/O, coercing the draft
    /// selection rather than rejecting it: the dialog must stay usable after
    /// any combination of choices.
    pub(crate) fn normalize(&mut self) {
        use spektrafilm_core::image_io::SaveOptionViolation as V;
        self.jpeg_quality = self.jpeg_quality.clamp(1, 100);
        let format = self.image_format();
        let violations =
            spektrafilm_core::image_io::save_option_violations(format, &self.save_options(), false);
        for violation in violations {
            match violation {
                V::Depth if format == ImageFormat::Exr => self.depth = BitDepth::Sixteen,
                V::Depth => self.depth = BitDepth::Eight,
                V::RequiresEncoded => self.saving_cctf_encoding = true,
                V::RequiresLinear => self.saving_cctf_encoding = false,
                V::ExrColorSpace => self.saving_color_space = "sRGB".into(),
                V::ExrCompression | V::CompressionNotConfigurable => {
                    self.compression = ExportCompression::Zip
                }
                V::JpegQualityNotApplicable
                | V::JpegSubsamplingNotApplicable
                | V::JpegQualityRange => {}
            }
        }
    }

    /// Container the writer receives for this format.
    pub(crate) fn image_format(&self) -> ImageFormat {
        self.format.image_format()
    }

    /// Writer options for this format — the one mapping from export-dialog
    /// selections to the shared image writer, consumed by both the saving
    /// telemetry configuration and the export worker.
    pub(crate) fn save_options(&self) -> SaveOptions<'_> {
        SaveOptions {
            depth: self.depth,
            color_space: &self.saving_color_space,
            cctf_encoding: self.saving_cctf_encoding,
            jpeg_quality: (self.format == ExportFormat::Jpeg).then_some(self.jpeg_quality),
            jpeg_subsampling: (self.format == ExportFormat::Jpeg).then_some(self.jpeg_subsampling),
            compression: matches!(self.format, ExportFormat::Tiff | ExportFormat::Exr).then_some(
                match self.compression {
                    ExportCompression::Zip => Compression::Zip,
                    ExportCompression::None => Compression::None,
                },
            ),
        }
    }

    /// Export preferences live in the Rust extension; Save uses simulation settings.
    pub(crate) fn write_state(&self, state: &mut Value) {
        if !state["rust"].is_object() {
            state["rust"] = json!({"version": crate::state::CURRENT_STATE_VERSION});
        }
        let rust = &mut state["rust"];
        rust["save_bit_depth"] = json!(self.depth.bits());
        rust["export_format"] = json!(self.format.argument());
        rust["jpeg_quality"] = json!(self.jpeg_quality);
        rust["jpeg_subsampling"] = json!(match self.jpeg_subsampling {
            JpegSubsampling::Yuv444 => "444",
            JpegSubsampling::Yuv420 => "420",
        });
        rust["export_compression"] = json!(self.compression.argument());
        rust["export_backend"] = json!(self.backend.argument());
        rust["export_saving_color_space"] = json!(self.saving_color_space);
        rust["export_saving_cctf_encoding"] = json!(self.saving_cctf_encoding);
    }
}

/// Resolve delivery geometry using the same crop rectangle and rescale
/// rounding as the renderer, without touching source pixels.
pub(crate) fn output_dimensions(
    source: [u32; 2],
    io: &spektrafilm_core::params::IoParams,
) -> Result<[u32; 2], String> {
    let (_, _, width, height) =
        spektrafilm_core::resizing::requested_crop_rect(source[0], source[1], io)?;
    let (width, height) = spektrafilm_math::resize::rescaled_dimensions(
        width as u32,
        height as u32,
        io.upscale_factor,
    )?;
    Ok([width, height])
}

/// Pixel payload of exported samples at the accepted delivery resolution.
///
/// The dimensions already include requested crop and upscale. Every
/// container writes three samples per pixel. Compressed containers write a
/// smaller file whose
/// exact size depends on image content, so the payload is the
/// content-independent bound the row can state without compressing.
pub(crate) fn estimated_output_bytes(options: &ExportOptions, dimensions: [u32; 2]) -> u64 {
    let bytes_per_sample = match options.depth {
        BitDepth::Eight => 1,
        BitDepth::Sixteen => 2,
        BitDepth::ThirtyTwo => 4,
    };
    dimensions[0] as u64 * dimensions[1] as u64 * 3 * bytes_per_sample
}

/// Whether the chosen container compresses the samples it writes.
///
/// TIFF is the only container whose compression the dialog can turn off, so
/// its estimate is the file size; every other container always codes the
/// samples, and the payload is only a bound.
fn container_compresses(options: &ExportOptions) -> bool {
    match options.format {
        ExportFormat::Tiff => options.compression == ExportCompression::Zip,
        _ => true,
    }
}

/// Binary-prefixed byte count for the estimated-size row.
fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KB * KB * KB {
        format!("{:.1} GB", value / (KB * KB * KB))
    } else if value >= KB * KB {
        format!("{:.1} MB", value / (KB * KB))
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Editor column width shared with the choice and switch rows, so a
/// read-only value lines up with the editable rows' editors.
fn editor_width(ui: &egui::Ui) -> f32 {
    ui.available_width()
        .clamp(theme::WIDTH_COMBO_MIN, theme::WIDTH_COMBO_MAX)
}

/// A one-point boundary without the default separator's extra layout space.
fn quiet_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), theme::STROKE_DEFAULT),
        egui::Sense::hover(),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(theme::STROKE_DEFAULT, theme::BORDER),
    );
}

fn section_heading(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(controls::localized(ui.ctx(), label))
            .color(theme::FG_SECONDARY)
            .size(theme::SIZE_LABEL)
            .strong(),
    );
}

fn section_break(ui: &mut egui::Ui) {
    // Ordinary body rows already contribute eight points after their content:
    // complete a 16-point section end, then leave 12 points before the title.
    ui.add_space(theme::SPACE_2);
    quiet_separator(ui);
    ui.add_space(theme::SPACE_1);
}

/// A read-only option row: product label and fixed value in the shared
/// editor column, locatable at its canonical path. The value carries the
/// row's accessible name like an editor does, so the row reads as the
/// option's state rather than a disabled editor.
fn readonly_row(ui: &mut egui::Ui, label: &str, value: &str, help: &str, path: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(controls::localized(ui.ctx(), label)).color(theme::FG_SECONDARY),
        )
        .on_hover_text(help);
        let cell = editor_width(ui);
        ui.add_space((ui.available_width() - cell).max(0.0));
        let response = ui
            .add_sized(
                egui::vec2(cell, theme::HEIGHT_CONTROL),
                egui::Label::new(
                    egui::RichText::new(value)
                        .color(theme::FG_SECONDARY)
                        .size(theme::SIZE_LABEL),
                )
                .halign(egui::Align::Max)
                .sense(egui::Sense::focusable_noninteractive()),
            )
            .on_hover_text(help);
        let name = controls::localized(ui.ctx(), label);
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_label(name.as_ref());
            node.set_value(value);
        });
        controls::locate_row(ui, path, &response);
    });
}

#[derive(Default)]
pub(crate) struct ExportDialog {
    draft: Option<ExportOptions>,
    /// Set while a freshly opened draft still owes the dialog its one-time
    /// initial focus. A canonical locator request owns the focus instead.
    initial_focus: bool,
}
impl ExportDialog {
    pub(crate) fn open(&mut self, committed: &ExportOptions) {
        let mut draft = committed.clone();
        draft.normalize();
        self.draft = Some(draft);
        self.initial_focus = true;
    }
    pub(crate) fn is_open(&self) -> bool {
        self.draft.is_some()
    }

    /// Closing or cancelling drops the draft; only Export returns preferences.
    ///
    /// `dimensions` is the delivery resolution after crop and upscale. A new
    /// accepted source or adjustment can arrive while the dialog is open, so
    /// callers resolve the current geometry every frame.
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        dimensions: [u32; 2],
        source: Option<&Path>,
    ) -> Option<ExportOptions> {
        let draft = self.draft.as_mut()?;
        let mut submit = false;
        let mut cancel = false;
        let mut close = false;
        let title = crate::i18n::text(ctx, "surfaces.export.title");
        let close_label = controls::localized(ctx, "Close export");
        let source_name = source
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| crate::i18n::text(ctx, "result.export.accepted_source"));
        let source_text = crate::tr!(ctx, "result.export.source", source = source_name);
        let dimensions_text = crate::tr!(
            ctx,
            "result.export.dimensions",
            width = dimensions[0],
            height = dimensions[1]
        );
        // A canonical locator request owns the focus of its frame; otherwise
        // the dialog places it once, on its explicit close action.
        let locator_requested = ctx.data(|data| {
            data.get_temp::<(String, String)>(egui::Id::new("control_focus_request"))
                .is_some()
        });
        let initial_focus = self.initial_focus && !locator_requested;
        self.initial_focus = false;
        // A real modal: its backdrop takes every outside click, so the
        // workspace behind the draft cannot change until the dialog is
        // dismissed. `Modal` centers the sheet like the prototype's
        // `min(500px, 100%)` dialog.
        let frame = theme::dialog_frame();
        let modal = egui::Modal::new(egui::Id::new("full_render_export_options"))
            .frame(frame)
            .show(ctx, |ui| {
                let screen = ui.ctx().screen_rect();
                // Width includes the modal border, rather than adding the
                // popup's default inset to the proposal's 500-point sheet.
                let border = 2.0 * theme::STROKE_DEFAULT;
                let width = ((screen.width() - 2.0 * theme::SPACE_DIALOG).min(500.0) - border)
                    .max(1.0);
                let max_height = (screen.height() * 0.9)
                    .min(screen.height() - 2.0 * theme::SPACE_DIALOG)
                    - border;
                ui.set_min_width(width);
                ui.set_max_width(width);
                ui.set_max_height(max_height);
                ui.spacing_mut().item_spacing.y = 0.0;
                // 56-point header: one control-height row centered between
                // 14-point vertical insets, with an icon-only close action.
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(
                        theme::SPACE_DIALOG as i8,
                        ((theme::HEIGHT_DIALOG_HEADER - theme::HEIGHT_CONTROL) / 2.0) as i8,
                    ))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.set_min_height(theme::HEIGHT_CONTROL);
                            ui.label(
                                egui::RichText::new(title.as_ref())
                                    .size(theme::SIZE_TITLE)
                                    .strong(),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let close_button = controls::icon_button(
                                    ui,
                                    close_label.as_ref(),
                                    controls::Icon::Close,
                                );
                                if initial_focus {
                                    close_button.request_focus();
                                }
                                close = close_button.clicked();
                            });
                        });
                    });
                quiet_separator(ui);
                // Only the body scrolls. Header, borders, body insets and
                // the padded 28-point action row have their own reservation.
                let body_height = (max_height
                    - theme::HEIGHT_DIALOG_HEADER
                    - theme::HEIGHT_DIALOG_FOOTER
                    - 2.0 * theme::STROKE_DEFAULT
                    - 2.0 * theme::SPACE_DIALOG)
                    .max(1.0);
                let body = egui::Frame::NONE
                    .inner_margin(theme::SPACE_DIALOG as i8)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("export-options-scroll")
                            .auto_shrink([false, true])
                            .max_height(body_height)
                            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(theme::SPACE_3, theme::SPACE_2);
                section_heading(ui, "File and container");
                enum_choice(ui, "Format", &mut draft.format, &[
                    (ExportFormat::Jpeg, ExportFormat::Jpeg.label()),
                    (ExportFormat::Png, ExportFormat::Png.label()),
                    (ExportFormat::Tiff, ExportFormat::Tiff.label()),
                    (ExportFormat::Exr, ExportFormat::Exr.label()),
                ], "JPEG: lossy 8-bit for web. PNG: lossless 8-bit. TIFF: 8/16-bit integer or 32-bit float, ZIP or uncompressed. EXR: 16/32-bit linear for compositing.");
                draft.normalize();
                let depth_choices: &[(BitDepth, &str)] = match draft.format {
                    ExportFormat::Jpeg | ExportFormat::Png => &[],
                    ExportFormat::Tiff => &[(BitDepth::Eight, "8 bit"), (BitDepth::Sixteen, "16 bit"), (BitDepth::ThirtyTwo, "32 bit")],
                    ExportFormat::Exr => &[(BitDepth::Sixteen, "16 bit"), (BitDepth::ThirtyTwo, "32 bit")],
                };
                if depth_choices.is_empty() {
                    let fixed = crate::i18n::text(ui.ctx(), "surfaces.export.depth_fixed");
                    let value = crate::i18n::text(ui.ctx(), "surfaces.export.bit8");
                    readonly_row(ui, "Bit depth", &value, &fixed, "extras.rust.save_bit_depth");
                } else {
                    enum_choice(ui, "Bit depth", &mut draft.depth, depth_choices, "Select the bit depth.");
                }
                let jpeg = draft.format == ExportFormat::Jpeg;
                if jpeg {
                    ui.label(
                        egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.export.jpeg_warning"))
                            .small()
                            .color(theme::FG_SECONDARY),
                    );
                    // The shared numeric owner models a parameter row: a
                    // labeled value entry with edit transactions and history
                    // gestures. This draft option is not a parameter row, and
                    // its editor is the labeled slider acceptance queries by
                    // role and name, so the dialog keeps it as the option's
                    // own editing owner.
                    let label = crate::i18n::text(ui.ctx(), "surfaces.export.jpeg_quality");
                    let help = crate::i18n::text(ui.ctx(), "surfaces.export.jpeg_quality_help");
                    ui.horizontal(|ui| {
                        let label_response = ui.label(
                            egui::RichText::new(label.as_ref()).color(theme::FG_SECONDARY),
                        ).on_hover_text(help.as_ref());
                        let cell = editor_width(ui);
                        ui.add_space((ui.available_width() - cell).max(0.0));
                        ui.scope(|ui| {
                            ui.spacing_mut().slider_width = (cell
                                - ui.spacing().interact_size.x
                                - ui.spacing().item_spacing.x)
                                .max(1.0);
                            let slider = egui::Slider::new(&mut draft.jpeg_quality, 1..=100);
                            let quality = ui.add(slider)
                                .labelled_by(label_response.id)
                                .on_hover_text(help.as_ref());
                            let enabled = ui.is_enabled();
                            let value = f64::from(draft.jpeg_quality);
                            quality.widget_info(|| egui::WidgetInfo::slider(enabled, value, label.as_ref()));
                            ui.ctx().accesskit_node_builder(quality.id, |node| {
                                node.set_label(label.as_ref());
                            });
                            controls::locate_row(ui, "extras.rust.jpeg_quality", &quality);
                        });
                    });
                    enum_choice(ui, "JPEG subsampling", &mut draft.jpeg_subsampling, &[
                        (JpegSubsampling::Yuv444, "4:4:4"),
                        (JpegSubsampling::Yuv420, "4:2:0"),
                    ], "Select the JPEG chroma subsampling.");
                } else {
                    // Another container's codec options are not editable
                    // here. One short read-only row keeps both canonical
                    // targets locatable and states the retained values,
                    // instead of two full rows of disabled editors.
                    let inactive = crate::i18n::text(ui.ctx(), "surfaces.export.jpeg_inactive");
                    let not_applicable = controls::localized(ui.ctx(), "not applicable");
                    let quality_name = controls::localized(ui.ctx(), "JPEG quality");
                    let subsampling_name = controls::localized(ui.ctx(), "JPEG subsampling");
                    let quality_value = draft.jpeg_quality.to_string();
                    let subsampling_value = draft.jpeg_subsampling.as_str().to_owned();
                    ui.horizontal_wrapped(|ui| {
                        for (name, value, path) in [
                            (quality_name, quality_value, "extras.rust.jpeg_quality"),
                            (
                                subsampling_name,
                                subsampling_value,
                                "extras.rust.jpeg_subsampling",
                            ),
                        ] {
                            let text = format!("{name}: {value} · {not_applicable}");
                            let response = ui
                                .add(
                                    egui::Label::new(
                                        egui::RichText::new(text)
                                            .small()
                                            .color(theme::FG_SECONDARY),
                                    )
                                    .sense(egui::Sense::focusable_noninteractive()),
                                )
                                .on_hover_text(inactive.as_ref());
                            controls::locate_row(ui, path, &response);
                        }
                    });
                }
                match draft.format {
                    ExportFormat::Tiff => {
                        enum_choice(ui, "Compression", &mut draft.compression, &[
                            (ExportCompression::Zip, "ZIP"),
                            (ExportCompression::None, "None"),
                        ], "Select the TIFF compression method.");
                    }
                    format => {
                        let (value, help) = match format {
                            ExportFormat::Exr => {
                                let help = crate::i18n::text(ui.ctx(), "surfaces.export.compression_exr");
                                ("ZIP", help)
                            }
                            ExportFormat::Jpeg => {
                                let help = crate::i18n::text(ui.ctx(), "surfaces.export.compression_jpeg");
                                ("JPEG lossy", help)
                            }
                            ExportFormat::Png => {
                                let help = crate::i18n::text(ui.ctx(), "surfaces.export.compression_png");
                                ("PNG lossless", help)
                            }
                            ExportFormat::Tiff => unreachable!(),
                        };
                        let value = controls::localized(ui.ctx(), value);
                        readonly_row(ui, "Compression", &value, &help, "extras.rust.export_compression");
                    }
                }
                section_break(ui);
                section_heading(ui, "Color and computation");
                let spaces = if draft.format == ExportFormat::Exr { EXR_COLOR_SPACES } else { COLOR_SPACES };
                // The shared choice owner models label, selector, and help;
                // this selector additionally offers selection guidance inside
                // its popup, so the dialog keeps its own owner for it.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.export.color_space"))
                            .color(theme::FG_SECONDARY),
                    )
                    .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.export.space_help"));
                    let cell = editor_width(ui);
                    ui.add_space((ui.available_width() - cell).max(0.0));
                    let color_space = egui::ComboBox::from_id_salt("Saving color space")
                        .width(cell)
                        .selected_text(&draft.saving_color_space)
                        .show_ui(ui, |ui| {
                            ui.label(
                                egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.export.space_hint"))
                                    .small()
                                    .color(theme::FG_MUTED),
                            );
                            for &space in spaces {
                                ui.selectable_value(&mut draft.saving_color_space, space.to_owned(), space);
                            }
                        })
                        .response
                        .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.export.space_help"));
                    controls::locate_row(ui, "extras.rust.export_saving_color_space", &color_space);
                    controls::focus_response(ui, "Saving color space", &color_space);
                    controls::name_choice(ui, "Saving color space");
                });
                // The shared switch owner models this boolean draft option:
                // the same presentation, state, and accessible naming an
                // inspector boolean gets. Its response keeps the row
                // locatable by canonical path through the draft's real owner.
                let cctf_help = crate::i18n::text(ui.ctx(), "surfaces.export.cctf_help");
                match draft.format {
                    ExportFormat::Tiff => {
                        let encoding = controls::toggle_row(
                            ui,
                            "Apply saving CCTF encoding",
                            &mut draft.saving_cctf_encoding,
                            &cctf_help,
                        );
                        controls::locate_row(ui, "extras.rust.export_saving_cctf_encoding", &encoding);
                    }
                    ExportFormat::Jpeg | ExportFormat::Png => {
                        let fixed = crate::i18n::text(ui.ctx(), "surfaces.export.encoded_fixed");
                        let value = controls::localized(ui.ctx(), "encoded");
                        readonly_row(
                            ui,
                            "Apply saving CCTF encoding",
                            &value,
                            &fixed,
                            "extras.rust.export_saving_cctf_encoding",
                        );
                    }
                    ExportFormat::Exr => {
                        let fixed = crate::i18n::text(ui.ctx(), "surfaces.export.exr_fixed");
                        let value = crate::i18n::text(ui.ctx(), "surfaces.export.linear");
                        readonly_row(
                            ui,
                            "Apply saving CCTF encoding",
                            &value,
                            &fixed,
                            "extras.rust.export_saving_cctf_encoding",
                        );
                    }
                }
                let cpu_label: &str = if cfg!(feature = "precision-f64") { "CPU (f64)" } else { "CPU (f32)" };
                enum_choice(ui, "Backend", &mut draft.backend, &[
                    (ExportBackend::Cpu, cpu_label),
                    (ExportBackend::Gpu, "GPU (WGPU f32)"),
                ], "CPU: highest precision, slower. GPU: faster, f32 precision.");
                ui.label(egui::RichText::new(crate::i18n::text(
                    ui.ctx(), "result.export.guidance",
                )).small().color(theme::FG_SECONDARY));
                draft.normalize();
                let depth_label = match draft.depth {
                    BitDepth::Eight => "8 bit",
                    BitDepth::Sixteen => "16 bit",
                    BitDepth::ThirtyTwo => "32 bit",
                };
                let format_label = draft.format.label();
                let summary = match draft.format {
                    ExportFormat::Exr => crate::tr!(ui.ctx(), "surfaces.export.summary_exr", format = format_label, depth = depth_label, space = &draft.saving_color_space),
                    ExportFormat::Jpeg => crate::tr!(ui.ctx(), "surfaces.export.summary_jpeg", format = format_label, space = &draft.saving_color_space, quality = draft.jpeg_quality, subsampling = draft.jpeg_subsampling.as_str()),
                    ExportFormat::Png => crate::tr!(ui.ctx(), "surfaces.export.summary_png", format = format_label, space = &draft.saving_color_space),
                    ExportFormat::Tiff => crate::tr!(ui.ctx(), "surfaces.export.summary_tiff", format = format_label, depth = depth_label, space = &draft.saving_color_space, encoding = if draft.saving_cctf_encoding { std::borrow::Cow::Borrowed("CCTF") } else { crate::i18n::text(ui.ctx(), "surfaces.export.linear") }, compression = match draft.compression { ExportCompression::Zip => std::borrow::Cow::Borrowed("ZIP"), ExportCompression::None => crate::i18n::text(ui.ctx(), "surfaces.export.none") }),
                };
                section_break(ui);
                section_heading(ui, "Summary and actions");
                ui.label(egui::RichText::new(source_text.as_ref()).small().color(theme::FG_SECONDARY));
                ui.label(egui::RichText::new(dimensions_text.as_ref()).small().color(theme::FG_SECONDARY));
                ui.label(egui::RichText::new(crate::tr!(ui.ctx(), "surfaces.export.summary", summary = summary)).small().color(theme::FG_SECONDARY))
                    .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.export.summary_help"));
                let compressed = container_compresses(draft);
                let estimate = format_size(estimated_output_bytes(draft, dimensions));
                let row = if compressed {
                    crate::tr!(ui.ctx(), "surfaces.export.size_compressed", size = estimate)
                } else {
                    crate::tr!(ui.ctx(), "surfaces.export.size", size = estimate)
                };
                ui.label(egui::RichText::new(row).small().color(theme::FG_MUTED))
                    .on_hover_text(if compressed {
                        crate::i18n::text(ui.ctx(), "surfaces.export.size_compressed_help")
                    } else {
                        crate::i18n::text(ui.ctx(), "surfaces.export.size_help")
                    });
                            })
                    })
                    .inner;
                // The body's visible viewport is what the locator and the
                // acceptance queries address, not the full scroll content.
                let viewport = ui.interact(
                    body.inner_rect,
                    ui.id().with("export-options-viewport"),
                    egui::Sense::hover(),
                );
                let content = controls::localized(ui.ctx(), "Export options content");
                viewport.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Other, true, content.as_ref())
                });
                ui.ctx().accesskit_node_builder(viewport.id, |node| {
                    node.set_role(egui::accesskit::Role::ScrollView);
                    node.set_label(content.as_ref());
                });
                quiet_separator(ui);
                egui::Frame::NONE
                    .inner_margin(theme::SPACE_DIALOG as i8)
                    .show(ui, |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), theme::HEIGHT_CONTROL),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                                // The opener and submit are two distinct steps.
                                let export = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.export.submit"))
                                            .color(theme::CONTRAST_ON_ACCENT),
                                    ).fill(theme::ACCENT)
                                        .min_size(egui::vec2(0.0, theme::HEIGHT_CONTROL)),
                                ).on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.export.submit_help"));
                                controls::locate_row(ui, "action.export", &export);
                                submit = export.clicked();
                                let discard = ui.add(
                                    egui::Button::new(crate::i18n::text(ui.ctx(), "surfaces.action.cancel"))
                                        .min_size(egui::vec2(0.0, theme::HEIGHT_CONTROL)),
                                ).on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.export.discard_help"));
                                controls::locate_row(ui, "action.cancel_export_draft", &discard);
                                cancel = discard.clicked();
                            },
                        );
                    });
            });
        // The modal root carries the dialog's accessible name and bounds.
        modal.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, title.as_ref())
        });
        ctx.accesskit_node_builder(modal.response.id, |node| {
            node.set_role(egui::accesskit::Role::Dialog);
            node.set_label(title.as_ref());
        });
        // Escape cancels like an explicit close. A click on the backdrop is
        // an incidental click outside the dialog: it must neither dismiss the
        // dialog nor reach the workspace the modal covers.
        let escape = modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if close || cancel || escape {
            self.draft = None;
            return None;
        }
        if submit {
            let mut options = self.draft.take()?;
            options.normalize();
            return Some(options);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(
        depth: BitDepth,
        format: ExportFormat,
        compression: ExportCompression,
    ) -> ExportOptions {
        ExportOptions {
            backend: ExportBackend::Cpu,
            format,
            depth,
            jpeg_quality: 90,
            jpeg_subsampling: JpegSubsampling::Yuv444,
            compression,
            saving_color_space: "sRGB".to_owned(),
            saving_cctf_encoding: true,
        }
    }

    fn tiff(depth: BitDepth) -> ExportOptions {
        options(depth, ExportFormat::Tiff, ExportCompression::Zip)
    }

    /// The row states the payload of the full-resolution input, so a wrong
    /// depth mapping, channel count, or dimension order must fail here.
    #[test]
    fn estimated_size_scales_with_resolution_channels_and_depth() {
        assert_eq!(
            estimated_output_bytes(&tiff(BitDepth::Eight), [100, 10]),
            3_000
        );
        assert_eq!(
            estimated_output_bytes(&tiff(BitDepth::Sixteen), [100, 10]),
            6_000
        );
        assert_eq!(
            estimated_output_bytes(&tiff(BitDepth::ThirtyTwo), [100, 10]),
            12_000
        );
        assert_eq!(
            estimated_output_bytes(&tiff(BitDepth::Eight), [200, 10]),
            6_000
        );
        // No image loaded: the export trigger is disabled, and the row must
        // not invent a size.
        assert_eq!(estimated_output_bytes(&tiff(BitDepth::Eight), [0, 0]), 0);
    }

    /// Only an uncompressed TIFF states a size the file will actually match;
    /// every other container codes the samples and needs the qualifier.
    #[test]
    fn only_uncompressed_tiff_skips_the_compression_qualifier() {
        let zip = |format| options(BitDepth::Sixteen, format, ExportCompression::Zip);
        let none = |format| options(BitDepth::Sixteen, format, ExportCompression::None);
        assert!(!container_compresses(&none(ExportFormat::Tiff)));
        assert!(container_compresses(&zip(ExportFormat::Tiff)));
        assert!(container_compresses(&zip(ExportFormat::Jpeg)));
        assert!(container_compresses(&zip(ExportFormat::Png)));
        assert!(container_compresses(&zip(ExportFormat::Exr)));
    }

    #[test]
    fn format_size_uses_binary_prefixes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(3_000), "2.9 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }

    /// A large input must not read as bytes; the row picks the unit that keeps
    /// the number readable.
    #[test]
    fn format_size_scales_to_the_megabyte_range() {
        // 4000x3000 at 16 bit: 72 MB of samples.
        assert_eq!(
            format_size(estimated_output_bytes(
                &tiff(BitDepth::Sixteen),
                [4000, 3000]
            )),
            "68.7 MB"
        );
    }

    /// The fixed rows state exactly what the writer receives: a normalized
    /// draft carries no real writer-format violation for any format, while
    /// the non-applicable codec values stay in the draft.
    #[test]
    fn normalized_drafts_satisfy_the_real_writer_for_every_format() {
        let formats = [
            ExportFormat::Jpeg,
            ExportFormat::Png,
            ExportFormat::Tiff,
            ExportFormat::Exr,
        ];
        let depths = [BitDepth::Eight, BitDepth::Sixteen, BitDepth::ThirtyTwo];
        let compressions = [ExportCompression::Zip, ExportCompression::None];
        for format in formats {
            for depth in depths {
                for compression in compressions {
                    for encoding in [true, false] {
                        let mut draft = options(depth, format, compression);
                        draft.jpeg_quality = 61;
                        draft.jpeg_subsampling = JpegSubsampling::Yuv420;
                        draft.saving_cctf_encoding = encoding;
                        draft.saving_color_space = "Display P3".to_owned();
                        draft.normalize();
                        let violations = spektrafilm_core::image_io::save_option_violations(
                            draft.image_format(),
                            &draft.save_options(),
                            false,
                        );
                        assert!(
                            violations.is_empty(),
                            "{format:?} keeps a writer violation after normalize"
                        );
                        assert_eq!(draft.jpeg_quality, 61);
                        assert_eq!(draft.jpeg_subsampling, JpegSubsampling::Yuv420);
                    }
                }
            }
        }
    }
}
