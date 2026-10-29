//! Borrowed spectral inputs and immutable prepared representations.

use crate::params::{InputGamutCompressParams, RuntimeParams};
use crate::profile::Profile;
use spektrafilm_backend::{ComputeBackend, PreparedTable};
use spektrafilm_math::spectral::TcLut;
use std::sync::Arc;

/// The accepted source snapshot owns the spectral material identity.
#[derive(Clone, Copy)]
pub(super) struct SpectralMaterial<'a> {
    identity: &'a Profile,
    pub log_sensitivity: &'a [Vec<f64>],
    pub monochrome: bool,
    pub reference_illuminant: &'a str,
    pub stock: Option<&'a str>,
    pub window: &'a [f64],
    pub surface: &'a [Vec<f64>],
}

impl PartialEq for SpectralMaterial<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.identity, other.identity)
    }
}

/// A temporary view used by both spectral construction and reuse decisions.
#[derive(Clone, Copy, PartialEq)]
pub struct SpectralInputs<'a> {
    pub(super) material: SpectralMaterial<'a>,
    pub(super) capture_illuminant: Option<&'a str>,
    pub(super) method: &'a str,
    pub(super) apply_window: bool,
    pub(super) apply_surface: bool,
    pub(super) gaussian_blur: f32,
    pub(super) use_cat16: bool,
    pub(super) filter_uv: [f32; 3],
    pub(super) filter_ir: [f32; 3],
    pub(super) color_filter: &'a str,
    pub(super) gamut: Option<&'a InputGamutCompressParams>,
}

impl<'a> SpectralInputs<'a> {
    /// Project source data and controls without cloning profiles or parameters.
    pub fn new(film: &'a Profile, params: &'a RuntimeParams, backend: &dyn ComputeBackend) -> Self {
        let hanatos = params.settings.rgb_to_raw_method == "hanatos2025";
        let tc = params.settings.rgb_to_raw_method != "mallett2019";
        let filter = |values: [f32; 3]| {
            if values[0] > 0.0 {
                values
            } else {
                // A finite identity edge preserves the other active filter's arithmetic.
                [0.0, 0.0, 1.0]
            }
        };
        Self {
            material: SpectralMaterial {
                identity: film,
                log_sensitivity: &film.data.log_sensitivity,
                monochrome: film.is_bw(),
                reference_illuminant: &film.info.reference_illuminant,
                stock: film.info.stock.as_deref(),
                window: &film.data.hanatos2025_adaptation_window_params,
                surface: &film.data.hanatos2025_adaptation_surface_params,
            },
            capture_illuminant: params
                .workflow
                .route_enum()
                .exposes_film()
                .then_some(params.camera.capture_illuminant.as_deref())
                .flatten(),
            method: &params.settings.rgb_to_raw_method,
            apply_window: hanatos && params.settings.apply_hanatos2025_adaptation_window,
            apply_surface: hanatos && params.settings.apply_hanatos2025_adaptation_surface,
            gaussian_blur: if hanatos {
                params.settings.spectral_gaussian_blur
            } else {
                0.0
            },
            use_cat16: tc && backend.is_gpu() && params.settings.use_cat16,
            filter_uv: filter(params.camera.filter_uv),
            filter_ir: filter(params.camera.filter_ir),
            color_filter: &params.camera.color_filter,
            gamut: (tc
                && params.io.input_gamut_compress.active
                && params.io.input_gamut_compress.algorithm != "off")
                .then_some(&params.io.input_gamut_compress),
        }
    }
}

/// CPU samples and a generated device table have different storage contracts.
#[derive(Clone)]
pub enum TcPreparation {
    Host(Arc<TcLut>),
    Device {
        /// Layout passed to the existing backend ABI; contains no host samples.
        descriptor: TcLut,
        table: PreparedTable,
    },
}

#[derive(Clone)]
pub enum SpectralTransform {
    Tc(TcPreparation),
    Mallett([[f64; 3]; 3]),
}

/// Completed spectral preparation, including the scalar reference's device identity.
#[derive(Clone)]
pub struct SpectralPreparation {
    pub front_illuminant: String,
    pub transform: SpectralTransform,
    pub(super) preparation_device: Option<u64>,
}

impl SpectralPreparation {
    /// Host samples or the layout descriptor for a device-owned TC table.
    pub fn tc_lut(&self) -> Option<&TcLut> {
        match &self.transform {
            SpectralTransform::Tc(TcPreparation::Host(table)) => Some(table),
            SpectralTransform::Tc(TcPreparation::Device { descriptor, .. }) => Some(descriptor),
            SpectralTransform::Mallett(_) => None,
        }
    }

    pub fn prepared_tc_lut(&self) -> Option<&PreparedTable> {
        match &self.transform {
            SpectralTransform::Tc(TcPreparation::Device { table, .. }) => Some(table),
            _ => None,
        }
    }

    pub fn mallett_core(&self) -> Option<&[[f64; 3]; 3]> {
        match &self.transform {
            SpectralTransform::Mallett(matrix) => Some(matrix),
            SpectralTransform::Tc(_) => None,
        }
    }

    pub fn compatible_with(&self, backend: &dyn ComputeBackend) -> bool {
        self.preparation_device == backend.preparation_device_id()
            && self.prepared_tc_lut().is_none_or(|table| {
                Some(table.values.backend_id()) == backend.preparation_device_id()
            })
    }
}
