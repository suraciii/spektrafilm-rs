//! Device-free film-chain stage plan and buffer preflight.
//!
//! Topology, geometry, and required binding sizes are derived here without a
//! device; `passes` owns allocation and execution.

use spektrafilm_backend::Tap;

pub(super) fn tap_index(tap: Tap) -> usize {
    match tap {
        Tap::RgbIn => 0,
        Tap::RgbPre => 1,
        Tap::LogEFilm => 2,
        Tap::CmyFilm => 3,
        Tap::LogEPrint => 4,
        Tap::CmyPrint => 5,
        Tap::RgbOut => 6,
    }
}

fn validate_tap_span(p: &spektrafilm_backend::FilmChainParams<'_>) -> Result<(), String> {
    for tap in [p.inject, p.collect] {
        if p.scan_film && matches!(tap, Tap::LogEPrint | Tap::CmyPrint) {
            return Err(format!(
                "tap {tap:?} is not part of this topology (scan_film = true)"
            ));
        }
    }
    if tap_index(p.collect) < tap_index(p.inject) {
        return Err(format!(
            "no node path reaches tap {:?} from {:?}",
            p.collect, p.inject
        ));
    }
    if p.inject == Tap::RgbIn && p.inject != p.collect {
        return Err(
            "film chain rgb_in must pass through retained metering and geometry before dispatch"
                .into(),
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChainStep {
    InputDecode,
    Inverse,
    Front,
    Highlight,
    CameraDiffusion,
    CameraBlur,
    LightLeak,
    Halation,
    Log,
    FilmDensity,
    Dir,
    SpektraGrain,
    Dye,
    Fotufilm,
    EnlargerLut,
    PrintRoundtrip,
    Print,
    EnlargerDiffusion,
    PrintDensity,
    Readout,
    ScannerLut,
    Scan,
    ScannerBlur,
    Glare,
    ScanProfile,
    Finishing,
    Gamut,
    Unsharp,
    ProceduralGrain,
    ExposureGrain,
    Transfer,
    Cmyk,
}

impl ChainStep {
    pub(super) fn source_stage(self) -> bool {
        matches!(
            self,
            Self::InputDecode
                | Self::Inverse
                | Self::Front
                | Self::Highlight
                | Self::CameraDiffusion
                | Self::CameraBlur
                | Self::LightLeak
                | Self::Halation
                | Self::Log
                | Self::FilmDensity
                | Self::Dir
                | Self::SpektraGrain
                | Self::Dye
                | Self::Fotufilm
        )
    }

    pub(super) fn gaussian(self, p: &spektrafilm_backend::FilmChainParams<'_>) -> bool {
        match self {
            Self::CameraBlur | Self::ScannerBlur | Self::Unsharp => true,
            Self::Halation => p
                .halation
                .as_ref()
                .is_some_and(super::super::halation::uses_gaussian),
            Self::Dir => p
                .dir_couplers
                .as_ref()
                .is_some_and(|d| d.diffusion_size_px > 0.0),
            Self::Glare => p.glare.as_ref().is_some_and(|g| g.blur_px > 0.0),
            _ => false,
        }
    }

    pub(super) fn gpu_stage(self) -> spektrafilm_backend::telemetry::GpuStage {
        use spektrafilm_backend::telemetry::GpuStage;
        match self {
            Self::InputDecode | Self::Inverse => GpuStage::Input,
            Self::Front
            | Self::Highlight
            | Self::CameraDiffusion
            | Self::CameraBlur
            | Self::LightLeak
            | Self::Log => GpuStage::FilmExpose,
            Self::Halation => GpuStage::Halation,
            Self::FilmDensity => GpuStage::FilmDevelop,
            Self::Dir => GpuStage::Dir,
            Self::SpektraGrain => GpuStage::GrainSpektra,
            // Both engines contain more than one semantic stage.
            Self::Dye | Self::Fotufilm => GpuStage::Other,
            Self::EnlargerLut | Self::PrintRoundtrip | Self::Print | Self::EnlargerDiffusion => {
                GpuStage::PrintExpose
            }
            Self::PrintDensity => GpuStage::PrintDevelop,
            Self::Readout | Self::ScannerLut | Self::Scan | Self::ScannerBlur => GpuStage::Scan,
            Self::Glare => GpuStage::Glare,
            Self::ScanProfile => GpuStage::Output,
            Self::Gamut => GpuStage::Gamut,
            Self::Unsharp => GpuStage::Unsharp,
            Self::ProceduralGrain => GpuStage::GrainProcedural,
            Self::ExposureGrain => GpuStage::GrainExposure,
            Self::Finishing => GpuStage::Output,
            Self::Transfer | Self::Cmyk => GpuStage::Output,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::InputDecode => "input_decode",
            Self::Inverse => "convert_inverse_solver",
            Self::Front => "front_transform",
            Self::Highlight => "highlight_boost",
            Self::CameraDiffusion => "camera_diffusion",
            Self::CameraBlur => "camera_lens_blur",
            Self::LightLeak => "light_leak",
            Self::Halation => "halation",
            Self::Log => "log_exposure",
            Self::FilmDensity => "film_density",
            Self::Dir => "dir_couplers",
            Self::SpektraGrain => super::super::grain::GRAIN_SPEKTRA_STAGE_NAME,
            Self::Dye => "grain_dye",
            Self::Fotufilm => "grain_fotufilm",
            Self::EnlargerLut => "enlarger_lut",
            Self::PrintRoundtrip => "print_lut_roundtrip",
            Self::Print => "print_spectral",
            Self::EnlargerDiffusion => "enlarger_diffusion",
            Self::PrintDensity => "print_density",
            Self::Readout => "scanner_density",
            Self::ScannerLut => "scanner_lut",
            Self::Scan => "scan_spectral",
            Self::ScannerBlur => "scanner_lens_blur",
            Self::Glare => "glare",
            Self::ScanProfile => "scan_profile",
            Self::Finishing => "finishing",
            Self::Gamut => "output_gamut",
            Self::Unsharp => "unsharp_mask",
            Self::ProceduralGrain => "grain_procedural",
            Self::ExposureGrain => "grain_exposure",
            Self::Transfer => "output_transfer",
            Self::Cmyk => "cmyk",
        }
    }
}

/// The fixed photography topology is assembled once, before any GPU allocation.
/// Boundaries slice these same stage rules; setup has no second activation rules.
pub(super) fn chain_steps(
    p: &spektrafilm_backend::FilmChainParams<'_>,
) -> Result<Vec<ChainStep>, String> {
    validate_tap_span(p)?;
    use ChainStep::*;
    let mut steps = Vec::new();
    let mut boundaries = [0usize; 7];
    if p.input_decode.is_some() {
        steps.push(InputDecode);
    }
    let inverse = matches!(p.front, spektrafilm_backend::FrontPass::ScanInverse(_));
    if inverse {
        steps.push(Inverse);
    } else {
        steps.push(Front);
        if p.highlight_boost.is_some() {
            steps.push(Highlight);
        }
        if p.camera_diffusion.is_some() {
            steps.push(CameraDiffusion);
        }
        if p.camera_lens_blur_px.is_some_and(|s| s > 0.0) {
            steps.push(CameraBlur);
        }
        if p.light_leak.is_some() {
            steps.push(LightLeak);
        }
        if p.halation.is_some() {
            steps.push(Halation);
        }
        steps.push(Log);
    }
    boundaries[tap_index(Tap::LogEFilm)] = steps.len();
    if !inverse {
        steps.push(FilmDensity);
        if p.dir_couplers.is_some() {
            steps.push(Dir);
        }
    }
    if p.grain_spektra.is_some() {
        steps.push(SpektraGrain);
    }
    if p.grain_dye.is_some() {
        steps.push(Dye);
    }
    if p.grain_fotufilm.is_some() {
        steps.push(Fotufilm);
    }
    boundaries[tap_index(Tap::CmyFilm)] = steps.len();
    if !p.scan_film {
        if p.enlarger_lut.is_some() {
            steps.push(EnlargerLut);
            if p.enlarger_diffusion.is_none() {
                steps.push(PrintRoundtrip);
            }
        } else {
            steps.push(Print);
        }
        if p.enlarger_diffusion.is_some() {
            steps.push(EnlargerDiffusion);
        }
        boundaries[tap_index(Tap::LogEPrint)] = steps.len();
        steps.push(PrintDensity);
    }
    boundaries[tap_index(Tap::CmyPrint)] = steps.len();
    if true {
        if p.scan_readout.is_some() {
            steps.push(Readout);
        } else if p.scanner_lut.is_some() {
            steps.push(ScannerLut);
        } else {
            steps.push(Scan);
        }
    }
    if p.scanner_lens_blur_px.is_some_and(|s| s > 0.0) {
        steps.push(ScannerBlur);
    }
    if p.glare.is_some() {
        steps.push(Glare);
    }
    if p.scan_profile.is_some() {
        steps.push(ScanProfile);
    }
    if p.finishing.is_some() {
        steps.push(Finishing);
    }
    if p.gamut.is_some() {
        steps.push(Gamut);
    }
    if p.unsharp.is_some() {
        steps.push(Unsharp);
    }
    if p.grain_procedural.is_some() {
        steps.push(ProceduralGrain);
    }
    if p.grain_exposure.is_some() {
        steps.push(ExposureGrain);
    }
    if p.output_transfer.as_ref().is_some_and(|t| !t.decode) {
        steps.push(Transfer);
    }
    if p.cmyk.is_some() {
        steps.push(Cmyk);
    }
    if p.output_transfer.as_ref().is_some_and(|t| t.decode) {
        steps.push(Transfer);
    }
    boundaries[tap_index(Tap::RgbOut)] = steps.len();
    Ok(steps[boundaries[tap_index(p.inject)]..boundaries[tap_index(p.collect)]].to_vec())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ChainGeometry {
    pub(super) source: (u32, u32),
    pub(super) field: (u32, u32),
    pub(super) tail: (u32, u32),
}

pub(super) fn raster_bytes((width, height): (u32, u32)) -> Result<u64, String> {
    let pixels = width
        .checked_mul(height)
        .filter(|&n| n > 0)
        .ok_or("film chain geometry is empty or exceeds the u32 pixel address space")?;
    // RGB scalar indices in the existing kernels are u32 too.
    pixels
        .checked_mul(3)
        .ok_or("film chain RGB geometry exceeds the u32 scalar address space")?;
    u64::from(pixels)
        .checked_mul(12)
        .ok_or_else(|| "film chain raster byte size overflow".into())
}

impl ChainGeometry {
    pub(super) fn plan(
        p: &spektrafilm_backend::FilmChainParams<'_>,
        steps: &[ChainStep],
    ) -> Result<Self, String> {
        let source = (p.image_width, p.image_height);
        raster_bytes(source)?;
        let field = if steps.contains(&ChainStep::Dye) {
            let grid = &p.grain_dye.as_ref().ok_or("Dye parameters missing")?.grid;
            (grid.width, grid.height)
        } else if steps.contains(&ChainStep::Fotufilm) {
            let grain = p
                .grain_fotufilm
                .as_ref()
                .ok_or("Fotufilm parameters missing")?;
            (grain.width, grain.height)
        } else {
            source
        };
        raster_bytes(field)?;
        let tail = field;
        raster_bytes(tail)?;
        Ok(Self {
            source,
            field,
            tail,
        })
    }

    pub(super) fn gaussian_extent(
        self,
        p: &spektrafilm_backend::FilmChainParams<'_>,
        steps: &[ChainStep],
    ) -> Result<(u32, u32), String> {
        let mut extent = (1, 1);
        for step in steps.iter().copied().filter(|s| s.gaussian(p)) {
            let candidate = if step.source_stage() {
                self.source
            } else {
                self.tail
            };
            if raster_bytes(candidate)? > raster_bytes(extent)? {
                extent = candidate;
            }
        }
        Ok(extent)
    }
}

#[derive(Clone, Copy)]
pub(super) struct ChainStageAllocation {
    pub(super) peak: u64,
    pub(super) pending: u64,
}

pub(super) struct ChainPlan {
    pub(super) steps: Vec<ChainStep>,
    pub(super) geometry: ChainGeometry,
    pub(super) allocations: Vec<ChainStageAllocation>,
}

// ── Bounded-execution preflight (pure, device-free) ──────────────────────────
//
// Resident rasters bind compatible STORAGE views directly. This scalar reports
// the largest active immutable binding, not aggregate operation capacity; the
// checked stage plan above owns carrier, workspace, and retirement headroom.

/// Floor covering fixed parameter blocks. Configuration-sized kernels and
/// fields are validated by their owning stage and counted by the operation
/// workspace plan; this value is not a bound on Gaussian support.
pub(super) const MAX_FIXED_AUX_BYTES: u64 = 4 * 1024;

/// Largest immutable binding in the actual tap span, derived without a device.
pub(in crate::wgpu_backend) struct RequiredChainBuffers {
    /// Per-binding limit preflight only; never an aggregate memory budget.
    pub(in crate::wgpu_backend) static_bytes: u64,
}

/// Derives the required device buffer sizes from the chain parameters.
/// Sizes mirror `run_film_chain_inner`: every lookup value is narrowed to
/// one f32 on upload, so `n` f64 samples become `4n` bytes and an
/// `[[f64; 3]]` table of `n` rows becomes `12n`.
pub(in crate::wgpu_backend) fn required_chain_buffers(
    p: &spektrafilm_backend::FilmChainParams<'_>,
) -> RequiredChainBuffers {
    let Ok(steps) = chain_steps(p) else {
        // The execution entry reports the topology error before allocation.
        return RequiredChainBuffers { static_bytes: 0 };
    };
    let active = |step| steps.contains(&step);
    let needed = |step, bytes| if active(step) { bytes } else { 0 };
    let f32s = |count: usize| (count as u64).saturating_mul(4);
    let table = |rows: usize| f32s(rows).saturating_mul(3);
    // `scan_cd`/`scan_bd` feed from the film or the print spectral data,
    // matching the `scan_film` selection in `run_film_chain_inner`.
    let scan_wavelengths = if p.scan_film {
        p.film_channel_density.len()
    } else {
        p.print_channel_density.len()
    };
    let tc_lut_bytes = match &p.front {
        spektrafilm_backend::FrontPass::Hanatos2025 { tc_lut, .. } => (tc_lut.size as u64)
            .saturating_mul(tc_lut.size as u64)
            .saturating_mul(12),
        spektrafilm_backend::FrontPass::Mallett2019 { .. }
        | spektrafilm_backend::FrontPass::ScanInverse(_) => 0,
    };
    let cmax_bytes = p.gamut.map_or(0, |gp| f32s(gp.cmax.len()));
    // Spektra's device pass uploads the composite curve x column
    // (`knots` × 3) and the layer curve tensor (`knots` × 9) as f64 storage.
    let grain_spektra_curve_bytes = p.grain_spektra.as_ref().map_or(0, |gp| {
        gp.curve_x.len() as u64 * 8 + gp.curve_layers.len() as u64 * 8
    });
    let scan_axes_bytes = p
        .scan_readout
        .map_or(0, |p| (p.density.len().max(p.exposure.len()) as u64) * 8);
    let convert_table_bytes = match &p.front {
        spektrafilm_backend::FrontPass::ScanInverse(p) => {
            (p.dye.len().max(p.cmfs.len()) as u64) * 3 * 8
        }
        _ => 0,
    };
    let static_bytes = [
        needed(ChainStep::FilmDensity, f32s(p.film_log_exposure.len())),
        needed(
            ChainStep::FilmDensity,
            table(p.film_density_curves_normalized.len()),
        ),
        needed(ChainStep::Print, table(p.film_channel_density.len())),
        needed(ChainStep::Print, f32s(p.film_channel_density.len())),
        needed(ChainStep::Print, f32s(p.print_illuminant.len())),
        needed(ChainStep::Print, table(p.print_sensitivity.len())),
        needed(ChainStep::PrintDensity, f32s(p.print_log_exposure.len())),
        needed(ChainStep::PrintDensity, table(p.print_density_curves.len())),
        needed(ChainStep::Scan, table(scan_wavelengths)),
        needed(ChainStep::Scan, f32s(scan_wavelengths)),
        needed(ChainStep::Scan, f32s(p.scan_illuminant.len())),
        needed(
            ChainStep::Scan,
            f32s(spektrafilm_math::spectral::N_WAVELENGTHS),
        ),
        needed(ChainStep::Front, tc_lut_bytes),
        needed(ChainStep::Gamut, cmax_bytes),
        needed(ChainStep::SpektraGrain, grain_spektra_curve_bytes),
        needed(
            ChainStep::ExposureGrain,
            8192 * 4
                * if p
                    .grain_exposure
                    .as_ref()
                    .is_some_and(|g| g.color_variation > 0.0)
                {
                    2
                } else {
                    1
                },
        ),
        needed(ChainStep::Readout, scan_axes_bytes),
        needed(ChainStep::Inverse, convert_table_bytes),
        MAX_FIXED_AUX_BYTES,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    RequiredChainBuffers { static_bytes }
}

/// Compare an immutable unpaged coefficient binding with absolute buffer and
/// storage-binding limits. Image views and DIR coefficient pages are checked
/// by their owners against their actual interval sizes.
pub(in crate::wgpu_backend) fn required_buffer_exceeds_limits(
    required_bytes: u64,
    limits: &wgpu::Limits,
) -> Option<String> {
    if required_bytes > limits.max_buffer_size {
        return Some(format!(
            "required buffer of {required_bytes} bytes exceeds max_buffer_size ({} bytes)",
            limits.max_buffer_size
        ));
    }
    if required_bytes > u64::from(limits.max_storage_buffer_binding_size) {
        return Some(format!(
            "required buffer of {required_bytes} bytes exceeds max_storage_buffer_binding_size \
             ({} bytes)",
            limits.max_storage_buffer_binding_size
        ));
    }
    None
}
