//! Multiple-exposure stack settings, source identity, and composition.
//!
//! The primary photograph is the first shot. Extra shots are referenced by
//! absolute path and content digest so a saved photograph can verify that it
//! still describes the same exposure stack.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use spektrafilm_backend::ComputeBackend;
use spektrafilm_core::params::Tap;
use spektrafilm_core::pipeline::{ExposureShot, RgbExposureShot, ShotMetering};
use spektrafilm_core::runtime::Runtime;
use spektrafilm_math::image::ImageBuf;

use crate::render::InputImage;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MultipleExposureMethod {
    #[default]
    Film,
    Rgb,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MultipleExposureMetering {
    Each,
    LockToFirst,
    #[default]
    Manual,
}

impl MultipleExposureMetering {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Each => "multiple_exposure.metering_each",
            Self::LockToFirst => "multiple_exposure.metering_lock_to_first",
            Self::Manual => "multiple_exposure.metering_manual",
        }
    }

    fn core(self) -> ShotMetering {
        match self {
            Self::Each => ShotMetering::Each,
            Self::LockToFirst => ShotMetering::LockToFirst,
            Self::Manual => ShotMetering::Manual,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MultipleExposureShot {
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
    pub(crate) exposure_ev: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MultipleExposureSettings {
    pub(crate) method: MultipleExposureMethod,
    pub(crate) metering: MultipleExposureMetering,
    pub(crate) primary_exposure_ev: f64,
    /// Extra shots in stack order. The primary photograph is implicit.
    pub(crate) shots: Vec<MultipleExposureShot>,
}

impl Default for MultipleExposureSettings {
    fn default() -> Self {
        Self {
            method: MultipleExposureMethod::Film,
            metering: MultipleExposureMetering::Manual,
            primary_exposure_ev: 0.0,
            shots: Vec::new(),
        }
    }
}

impl MultipleExposureSettings {
    pub(crate) fn active(&self) -> bool {
        !self.shots.is_empty()
    }

    pub(crate) fn total_shots(&self) -> usize {
        self.shots.len() + 1
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.total_shots() < 2 || self.total_shots() > 8 {
            return Err("multiple exposure needs two to eight shots".into());
        }
        if !self.primary_exposure_ev.is_finite()
            || !(-10.0..=10.0).contains(&self.primary_exposure_ev)
        {
            return Err("primary exposure must be finite and in [-10, 10]".into());
        }
        for shot in &self.shots {
            if !shot.exposure_ev.is_finite() || !(-10.0..=10.0).contains(&shot.exposure_ev) {
                return Err(format!(
                    "shot {} exposure must be finite and in [-10, 10]",
                    shot.path.display()
                ));
            }
            if shot.path.as_os_str().is_empty() || shot.sha256.len() != 64 {
                return Err("multiple exposure shot identity is incomplete".into());
            }
        }
        Ok(())
    }

    pub(crate) fn shot_index(&self, path: &Path) -> Option<usize> {
        self.shots.iter().position(|shot| shot.path == path)
    }
}

/// One immutable stack for one render snapshot.
#[derive(Clone)]
pub(crate) struct MultipleExposureStack {
    pub(crate) settings: MultipleExposureSettings,
    pub(crate) extras: Vec<(PathBuf, InputImage)>,
}

/// Session-owned stack plus the loaded extra rasters.
#[derive(Default)]
pub(crate) struct MultipleExposureWork {
    pub(crate) open: bool,
    pub(crate) settings: MultipleExposureSettings,
    pub(crate) images: Vec<(PathBuf, InputImage)>,
    pub(crate) selected: usize,
    pub(crate) thumbnails: std::collections::HashMap<PathBuf, egui::TextureHandle>,
    pub(crate) error: Option<String>,
    /// A reopened record without its shot rasters asks the input worker to
    /// reload them before the next render.
    pub(crate) needs_reload: bool,
}

impl MultipleExposureWork {
    pub(crate) fn active(&self) -> bool {
        self.settings.active()
    }

    pub(crate) fn ensure_thumbnails(
        &mut self,
        primary_path: Option<&Path>,
        primary: Option<&InputImage>,
        backend: &dyn ComputeBackend,
        ctx: &egui::Context,
    ) {
        let mut pending = Vec::new();
        if let Some(path) = primary_path
            && !self.thumbnails.contains_key(path)
            && let Some(image) = primary
        {
            pending.push((path.to_path_buf(), image.clone()));
        }
        for shot in &self.settings.shots {
            if !self.thumbnails.contains_key(&shot.path)
                && let Some(image) = self.image(&shot.path)
            {
                pending.push((shot.path.clone(), image.clone()));
            }
        }
        for (path, image) in pending {
            let Ok(host) = image.to_host(backend) else {
                continue;
            };
            let thumbnail = thumbnail_image(&host, 96);
            let texture = ctx.load_texture(
                format!("multiple-exposure-thumbnail:{}", path.display()),
                thumbnail,
                egui::TextureOptions::LINEAR,
            );
            self.thumbnails.insert(path, texture);
        }
    }

    pub(crate) fn stack(&self) -> MultipleExposureStack {
        let extras = self
            .settings
            .shots
            .iter()
            .filter_map(|shot| {
                self.images
                    .iter()
                    .find(|(path, _)| path == &shot.path)
                    .map(|(path, image)| (path.clone(), image.clone()))
            })
            .collect();
        MultipleExposureStack::new(self.settings.clone(), extras)
    }

    pub(crate) fn image(&self, path: &Path) -> Option<&InputImage> {
        self.images
            .iter()
            .find_map(|(candidate, image)| (candidate == path).then_some(image))
    }

    pub(crate) fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub(crate) fn clear_error(&mut self) {
        self.error = None;
    }
}

impl MultipleExposureStack {
    pub(crate) fn new(
        settings: MultipleExposureSettings,
        extras: Vec<(PathBuf, InputImage)>,
    ) -> Self {
        Self { settings, extras }
    }

    fn image_for(&self, path: &Path) -> Result<&InputImage, String> {
        self.extras
            .iter()
            .find_map(|(candidate, image)| (candidate == path).then_some(image))
            .ok_or_else(|| format!("multiple exposure source is not loaded: {}", path.display()))
    }

    /// Compose the primary photograph and every extra shot, then run the
    /// method chain once. The primary arrives already oriented; extra shots
    /// receive the same quarter turns here. Preview resizing happens here so
    /// Preview and Scan use the same shot order and the same validation.
    pub(crate) fn compose(
        &self,
        primary: &InputImage,
        pipeline: &Runtime,
        backend: &dyn ComputeBackend,
        source_backend: &dyn ComputeBackend,
        preview_max_size: Option<u32>,
        rotation_quarter_turns: u32,
    ) -> Result<ImageBuf, String> {
        self.settings.validate()?;
        let mut images = Vec::with_capacity(self.settings.total_shots());
        images.push(primary.to_host(source_backend)?);
        for shot in &self.settings.shots {
            let mut image = self.image_for(&shot.path)?.to_host(source_backend)?;
            if rotation_quarter_turns != 0 {
                image = image.rotated_quarter_turns(rotation_quarter_turns as i32);
            }
            images.push(image);
        }
        if let Some(max_size) = preview_max_size {
            if max_size == 0 {
                return Err("preview size must be positive".into());
            }
            for image in &mut images {
                *image = spektrafilm_core::params_builder::resize_for_preview(image, max_size);
            }
        }
        let (width, height) = (images[0].width, images[0].height);
        if width == 0 || height == 0 {
            return Err("multiple exposure shots must not be empty".into());
        }
        if images
            .iter()
            .any(|image| image.width != width || image.height != height)
        {
            return Err("multiple exposure shots must share one size".into());
        }
        let mut exposure_ev = Vec::with_capacity(images.len());
        exposure_ev.push(self.settings.primary_exposure_ev);
        exposure_ev.extend(self.settings.shots.iter().map(|shot| shot.exposure_ev));
        let metering = self.settings.metering.core();
        match self.settings.method {
            MultipleExposureMethod::Film => {
                let shots = images
                    .iter()
                    .zip(&exposure_ev)
                    .map(|(image, exposure_ev)| ExposureShot {
                        image,
                        exposure_ev: *exposure_ev,
                        metering,
                    })
                    .collect::<Vec<_>>();
                pipeline
                    .process_exposures_with_report(&shots, backend, Tap::RgbOut)
                    .map(|(output, _)| output)
            }
            MultipleExposureMethod::Rgb => {
                let shots = images
                    .iter()
                    .zip(&exposure_ev)
                    .map(|(image, exposure_ev)| RgbExposureShot {
                        image,
                        exposure_ev: *exposure_ev,
                    })
                    .collect::<Vec<_>>();
                pipeline.process_rgb_exposures(&shots, backend)
            }
        }
    }
}

fn thumbnail_image(source: &ImageBuf, max_size: u32) -> egui::ColorImage {
    let longest = source.width.max(source.height).max(1);
    let scale = (max_size as f64 / longest as f64).min(1.0);
    let width = ((source.width as f64 * scale).round() as u32).max(1);
    let height = ((source.height as f64 * scale).round() as u32).max(1);
    let mut image = egui::ColorImage::new([width as usize, height as usize], egui::Color32::BLACK);
    for y in 0..height {
        let source_y = (y as f64 / scale) as usize;
        for x in 0..width {
            let source_x = (x as f64 / scale) as usize;
            let index = (source_y.min(source.height as usize - 1) * source.width as usize
                + source_x.min(source.width as usize - 1))
                * 3;
            let channel = |offset: usize| {
                (spektrafilm_math::precision::to_f64(source.data[index + offset]) * 255.0)
                    .clamp(0.0, 255.0) as u8
            };
            image.pixels[y as usize * width as usize + x as usize] =
                egui::Color32::from_rgb(channel(0), channel(1), channel(2));
        }
    }
    image
}
