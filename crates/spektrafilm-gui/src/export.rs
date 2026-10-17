//! Full-render export preferences and a cancellable options draft.
use std::path::{Path, PathBuf};

use crate::controls::enum_choice;
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

#[derive(Clone, Debug)]
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
            state["rust"] = json!({"version": 1});
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

/// Pixel payload of the exported samples at the input's full resolution.
///
/// The export renders the retained input, so the row reflects the input's
/// dimensions rather than the reduced-size preview. Every container writes
/// three samples per pixel. Compressed containers write a smaller file whose
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

#[derive(Default)]
pub(crate) struct ExportDialog {
    draft: Option<ExportOptions>,
}
impl ExportDialog {
    pub(crate) fn open(&mut self, committed: &ExportOptions) {
        let mut draft = committed.clone();
        draft.normalize();
        self.draft = Some(draft);
    }
    pub(crate) fn is_open(&self) -> bool {
        self.draft.is_some()
    }

    /// Closing or cancelling drops the draft; only Export returns preferences.
    ///
    /// `dimensions` is the resolution of the input the export renders. The
    /// dialog is modal but does not block drag-and-drop, so a new image can
    /// arrive while it is open; the estimate reads the current input every
    /// frame instead of the one the dialog was opened with.
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        dimensions: [u32; 2],
    ) -> Option<ExportOptions> {
        let draft = self.draft.as_mut()?;
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new("Export options")
            .id(egui::Id::new("full_render_export_options"))
            .open(&mut open).collapsible(false).resizable(false)
            .show(ctx, |ui| {
                let cpu_label: &str = if cfg!(feature = "precision-f64") { "CPU (f64)" } else { "CPU (f32)" };
                enum_choice(ui, "Backend", &mut draft.backend, &[
                    (ExportBackend::Cpu, cpu_label),
                    (ExportBackend::Gpu, "GPU (WGPU f32)"),
                ], "CPU: highest precision, slower. GPU: faster, f32 precision.");
                enum_choice(ui, "Format", &mut draft.format, &[
                    (ExportFormat::Jpeg, ExportFormat::Jpeg.label()),
                    (ExportFormat::Png, ExportFormat::Png.label()),
                    (ExportFormat::Tiff, ExportFormat::Tiff.label()),
                    (ExportFormat::Exr, ExportFormat::Exr.label()),
                ], "JPEG: lossy 8-bit for web. PNG: lossless 8-bit. TIFF: 8/16-bit ZIP. EXR: 16/32-bit linear for compositing.");
                draft.normalize();
                let depth_choices: &[(BitDepth, &str)] = match draft.format {
                    ExportFormat::Jpeg | ExportFormat::Png => &[(BitDepth::Eight, "8 bit")],
                    ExportFormat::Tiff => &[(BitDepth::Eight, "8 bit"), (BitDepth::Sixteen, "16 bit"), (BitDepth::ThirtyTwo, "32 bit")],
                    ExportFormat::Exr => &[(BitDepth::Sixteen, "16 bit"), (BitDepth::ThirtyTwo, "32 bit")],
                };
                enum_choice(ui, "Bit depth", &mut draft.depth, depth_choices, "Select the bit depth.");
                if draft.format == ExportFormat::Jpeg {
                    ui.label("JPEG uses lossy compression, including at quality 100. Use PNG, TIFF or EXR to preserve image detail.");
                    ui.add(egui::Slider::new(&mut draft.jpeg_quality, 1..=100).text("JPEG quality"));
                    enum_choice(ui, "JPEG subsampling", &mut draft.jpeg_subsampling, &[
                        (JpegSubsampling::Yuv444, "4:4:4"),
                        (JpegSubsampling::Yuv420, "4:2:0"),
                    ], "Select the JPEG chroma subsampling.");
                }
                if draft.format == ExportFormat::Tiff {
                    enum_choice(ui, "Compression", &mut draft.compression, &[
                        (ExportCompression::Zip, "ZIP"),
                        (ExportCompression::None, "None"),
                    ], "Select the TIFF compression method.");
                }
                let spaces = if draft.format == ExportFormat::Exr { EXR_COLOR_SPACES } else { COLOR_SPACES };
                egui::ComboBox::from_label("Saving color space")
                    .selected_text(&draft.saving_color_space)
                    .show_ui(ui, |ui| {
                    ui.label(
                        egui::RichText::new("sRGB for web · Adobe RGB for print · ACES2065-1 for archival")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                    for &space in spaces {
                        ui.selectable_value(&mut draft.saving_color_space, space.to_owned(), space);
                    }
                });
                match draft.format {
                    ExportFormat::Exr => { ui.label("EXR uses linear color and ZIP compression."); }
                    ExportFormat::Jpeg | ExportFormat::Png => { ui.label("JPEG and PNG use encoded color."); }
                    ExportFormat::Tiff => { ui.checkbox(&mut draft.saving_cctf_encoding, "Apply saving CCTF encoding"); }
                }
                let depth_label = match draft.depth {
                    BitDepth::Eight => "8 bit",
                    BitDepth::Sixteen => "16 bit",
                    BitDepth::ThirtyTwo => "32 bit",
                };
                let format_label = draft.format.label();
                let summary = match draft.format {
                    ExportFormat::Exr => format!("{format_label} · {depth_label} · {} · linear · ZIP", draft.saving_color_space),
                    ExportFormat::Jpeg => format!("{format_label} · 8 bit · {} · quality {} · {}", draft.saving_color_space, draft.jpeg_quality, draft.jpeg_subsampling.as_str()),
                    ExportFormat::Png => format!("{format_label} · 8 bit · {} · encoded", draft.saving_color_space),
                    ExportFormat::Tiff => format!("{format_label} · {depth_label} · {} · {} · {}", draft.saving_color_space, if draft.saving_cctf_encoding { "CCTF" } else { "linear" }, match draft.compression { ExportCompression::Zip => "ZIP", ExportCompression::None => "None" }),
                };
                ui.label(egui::RichText::new(format!("Summary: {summary}")).small().color(egui::Color32::GRAY));
                let compressed = container_compresses(draft);
                let estimate = format_size(estimated_output_bytes(draft, dimensions));
                let row = if compressed {
                    format!("Estimated output size: ≈ {estimate} before compression")
                } else {
                    format!("Estimated output size: ≈ {estimate}")
                };
                ui.label(egui::RichText::new(row).small().color(egui::Color32::GRAY))
                    .on_hover_text(if compressed {
                        "Pixel payload at the input's full resolution. This container compresses the samples, so the file is smaller; the exact size depends on image content."
                    } else {
                        "Pixel payload at the input's full resolution. This container stores the samples uncompressed, so the file matches apart from headers."
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    submit = ui.button("Export…").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if !open || cancel || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
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
}
