//! Full-render export preferences and a cancellable options draft.
use std::path::{Path, PathBuf};

use eframe::egui;
use serde_json::{Value, json};
use spektrafilm_core::image_io::{BitDepth, JpegSubsampling};

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
    pub(crate) fn argument(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Exr => "exr",
        }
    }
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Exr => "exr",
        }
    }
    pub(crate) fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Png => &["png"],
            Self::Tiff => &["tif", "tiff"],
            Self::Exr => &["exr"],
        }
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
    fn depths(self) -> &'static [BitDepth] {
        match self {
            Self::Jpeg | Self::Png => &[BitDepth::Eight],
            Self::Tiff => &[BitDepth::Eight, BitDepth::Sixteen, BitDepth::ThirtyTwo],
            Self::Exr => &[BitDepth::Sixteen, BitDepth::ThirtyTwo],
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

    /// Apply the same format constraints as core image I/O before submission.
    pub(crate) fn normalize(&mut self) {
        self.jpeg_quality = self.jpeg_quality.clamp(1, 100);
        match self.format {
            ExportFormat::Jpeg | ExportFormat::Png => {
                self.depth = BitDepth::Eight;
                self.saving_cctf_encoding = true;
            }
            ExportFormat::Exr => {
                if self.depth == BitDepth::Eight {
                    self.depth = BitDepth::Sixteen;
                }
                self.compression = ExportCompression::Zip;
                self.saving_cctf_encoding = false;
                if !EXR_COLOR_SPACES.contains(&self.saving_color_space.as_str()) {
                    self.saving_color_space = "sRGB".into();
                }
            }
            ExportFormat::Tiff => {}
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
    pub(crate) fn show(&mut self, ctx: &egui::Context) -> Option<ExportOptions> {
        let draft = self.draft.as_mut()?;
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new("Export options")
            .id(egui::Id::new("full_render_export_options"))
            .open(&mut open).collapsible(false).resizable(false)
            .show(ctx, |ui| {
                egui::ComboBox::from_label("Backend").selected_text(draft.backend.label()).show_ui(ui, |ui| {
                    for backend in [ExportBackend::Cpu, ExportBackend::Gpu] {
                        ui.selectable_value(&mut draft.backend, backend, backend.label());
                    }
                });
                egui::ComboBox::from_label("Format").selected_text(draft.format.label()).show_ui(ui, |ui| {
                    for format in [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Tiff, ExportFormat::Exr] {
                        ui.selectable_value(&mut draft.format, format, format.label());
                    }
                });
                draft.normalize();
                egui::ComboBox::from_label("Bit depth").selected_text(format!("{} bit", draft.depth.bits())).show_ui(ui, |ui| {
                    for &depth in draft.format.depths() {
                        ui.selectable_value(&mut draft.depth, depth, format!("{} bit", depth.bits()));
                    }
                });
                if draft.format == ExportFormat::Jpeg {
                    ui.label("JPEG uses lossy compression, including at quality 100. Use PNG, TIFF or EXR to preserve image detail.");
                    ui.add(egui::Slider::new(&mut draft.jpeg_quality, 1..=100).text("JPEG quality"));
                    egui::ComboBox::from_label("JPEG subsampling").selected_text(draft.jpeg_subsampling.as_str()).show_ui(ui, |ui| {
                        for subsampling in [JpegSubsampling::Yuv444, JpegSubsampling::Yuv420] {
                            ui.selectable_value(&mut draft.jpeg_subsampling, subsampling, subsampling.as_str());
                        }
                    });
                }
                if draft.format == ExportFormat::Tiff {
                    egui::ComboBox::from_label("Compression").selected_text(draft.compression.argument()).show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.compression, ExportCompression::Zip, "ZIP");
                        ui.selectable_value(&mut draft.compression, ExportCompression::None, "None");
                    });
                }
                let spaces = if draft.format == ExportFormat::Exr { EXR_COLOR_SPACES } else { COLOR_SPACES };
                egui::ComboBox::from_label("Saving color space").selected_text(&draft.saving_color_space).show_ui(ui, |ui| {
                    for &space in spaces {
                        ui.selectable_value(&mut draft.saving_color_space, space.to_owned(), space);
                    }
                });
                match draft.format {
                    ExportFormat::Exr => { ui.label("EXR uses linear color and ZIP compression."); }
                    ExportFormat::Jpeg | ExportFormat::Png => { ui.label("JPEG and PNG use encoded color."); }
                    ExportFormat::Tiff => { ui.checkbox(&mut draft.saving_cctf_encoding, "Apply saving CCTF encoding"); }
                }
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
