use super::super::blur::GaussianScratch;
use super::super::grain::raster::RasterStorage;
use super::super::page_views::Access::{Read, ReadWrite, Write};
use super::super::paged::ImagePage;
use super::*;
use spektrafilm_backend::Tap;

fn tap_index(tap: Tap) -> usize {
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
enum ChainStep {
    InputDecode,
    Inverse,
    Front,
    Highlight,
    CameraDiffusion,
    CameraBlur,
    Halation,
    Log,
    FilmDensity,
    Dir,
    GrainV1,
    Dye,
    Fotufilm,
    DyePrintLog,
    EnlargerLut,
    PrintRoundtrip,
    Print,
    EnlargerDiffusion,
    Collapse,
    PrintDensity,
    Readout,
    ScannerLut,
    Scan,
    ScannerBlur,
    Glare,
    Gamut,
    Unsharp,
    GrainV2,
    Transfer,
    Cmyk,
}

impl ChainStep {
    fn source_stage(self) -> bool {
        matches!(
            self,
            Self::InputDecode
                | Self::Inverse
                | Self::Front
                | Self::Highlight
                | Self::CameraDiffusion
                | Self::CameraBlur
                | Self::Halation
                | Self::Log
                | Self::FilmDensity
                | Self::Dir
                | Self::GrainV1
                | Self::Dye
                | Self::Fotufilm
        )
    }

    fn gaussian(self, p: &spektrafilm_backend::FilmChainParams<'_>) -> bool {
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

    fn name(self) -> &'static str {
        match self {
            Self::InputDecode => "input_decode",
            Self::Inverse => "convert_inverse_solver",
            Self::Front => "front_transform",
            Self::Highlight => "highlight_boost",
            Self::CameraDiffusion => "camera_diffusion",
            Self::CameraBlur => "camera_lens_blur",
            Self::Halation => "halation",
            Self::Log => "log_exposure",
            Self::FilmDensity => "film_density",
            Self::Dir => "dir_couplers",
            Self::GrainV1 => super::super::grain::GRAIN_V1_STAGE_NAME,
            Self::Dye => "grain_dye",
            Self::Fotufilm => "grain_fotufilm",
            Self::DyePrintLog => "dye_print_response_log",
            Self::EnlargerLut => "enlarger_lut",
            Self::PrintRoundtrip => "print_lut_roundtrip",
            Self::Print => "print_spectral",
            Self::EnlargerDiffusion => "enlarger_diffusion",
            Self::Collapse => "dye_aperture_collapse",
            Self::PrintDensity => "print_density",
            Self::Readout => "scanner_density",
            Self::ScannerLut => "scanner_lut",
            Self::Scan => "scan_spectral",
            Self::ScannerBlur => "scanner_lens_blur",
            Self::Glare => "glare",
            Self::Gamut => "output_gamut",
            Self::Unsharp => "unsharp_mask",
            Self::GrainV2 => "grain_v2",
            Self::Transfer => "output_transfer",
            Self::Cmyk => "cmyk",
        }
    }
}

/// The fixed photography topology is assembled once, before any GPU allocation.
/// Boundaries slice these same stage rules; setup has no second activation rules.
fn chain_steps(p: &spektrafilm_backend::FilmChainParams<'_>) -> Result<Vec<ChainStep>, String> {
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
    if p.grain_v1.is_some() {
        steps.push(GrainV1);
    }
    if p.grain_dye.is_some() {
        steps.push(Dye);
    }
    if p.grain_fotufilm.is_some() {
        steps.push(Fotufilm);
    }
    boundaries[tap_index(Tap::CmyFilm)] = steps.len();
    if !p.scan_film {
        if p.grain_dye_cell_spectral {
            steps.push(DyePrintLog);
        } else if p.enlarger_lut.is_some() {
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
        if p.grain_dye_aperture.is_some() {
            steps.push(Collapse);
        }
        boundaries[tap_index(Tap::LogEPrint)] = steps.len();
        steps.push(PrintDensity);
    }
    boundaries[tap_index(Tap::CmyPrint)] = steps.len();
    if !(p.grain_dye_cell_spectral && p.scan_film) {
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
    if p.gamut.is_some() {
        steps.push(Gamut);
    }
    if p.unsharp.is_some() {
        steps.push(Unsharp);
    }
    if p.grain_v2.is_some() {
        steps.push(GrainV2);
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
struct ChainGeometry {
    source: (u32, u32),
    field: (u32, u32),
    tail: (u32, u32),
}

fn raster_bytes((width, height): (u32, u32)) -> Result<u64, String> {
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
    fn plan(
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
        let tail = if steps.contains(&ChainStep::Collapse) {
            let factor = p.grain_dye_aperture.ok_or("Dye aperture missing")?;
            if factor == 0 {
                return Err("Dye aperture must be nonzero".into());
            }
            (field.0 / factor, field.1 / factor)
        } else {
            field
        };
        raster_bytes(tail)?;
        Ok(Self {
            source,
            field,
            tail,
        })
    }

    fn gaussian_extent(
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

/// Only cross-stage rasters live here. Per-stage states and bindgroups are
/// constructed inside their execution closure and dropped immediately afterward.
#[derive(Default)]
struct ChainRasters {
    a: Vec<ImagePage>,
    b: Vec<ImagePage>,
    field: Vec<ImagePage>,
    fine: Vec<ImagePage>,
    tail_a: Vec<ImagePage>,
    tail_b: Vec<ImagePage>,
    tail_entered: bool,
}

fn ensure_pages(
    backend: &WgpuBackend,
    pages: &mut Vec<ImagePage>,
    geometry: (u32, u32),
    label: &str,
) -> Result<(), String> {
    if pages.is_empty() {
        *pages = backend.allocate_image_pages(geometry.0, geometry.1, label)?;
    }
    Ok(())
}

impl ChainRasters {
    fn enter_tail(&mut self, backend: &WgpuBackend, geometry: ChainGeometry) -> Result<(), String> {
        if self.tail_entered {
            return Ok(());
        }
        if self.field.is_empty() {
            self.field = self.a.clone();
        }
        if self.tail_a.is_empty() && geometry.tail == geometry.source {
            self.tail_a = self.a.clone();
        }
        if self.tail_b.is_empty() && geometry.tail == geometry.source {
            self.tail_b = self.b.clone();
        }
        ensure_pages(backend, &mut self.tail_b, geometry.tail, "field_tail_rgb")?;
        // All exposure/development readers have finished encoding; queue order
        // permits same-geometry reuse, while submitted commands retain ownership.
        self.a.clear();
        self.b.clear();
        self.tail_entered = true;
        Ok(())
    }

    fn fine_output(
        &mut self,
        backend: &WgpuBackend,
        geometry: ChainGeometry,
    ) -> Result<(), String> {
        if self.fine.is_empty() {
            self.fine = if geometry.field == geometry.tail {
                self.tail_b.clone()
            } else {
                backend.allocate_image_pages(
                    geometry.field.0,
                    geometry.field.1,
                    "field_fine_exposure",
                )?
            };
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct ChainStageAllocation {
    peak: u64,
    pending: u64,
}

struct ChainPlan {
    steps: Vec<ChainStep>,
    geometry: ChainGeometry,
    allocations: Vec<ChainStageAllocation>,
}

impl WgpuBackend {
    fn chain_front(
        &self,
        p: &spektrafilm_backend::FilmChainParams<'_>,
        rasters: &ChainRasters,
        n: u32,
    ) -> Result<(), String> {
        let params = buffers::front(self, p);
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        match &p.front {
            spektrafilm_backend::FrontPass::Hanatos2025 { generated, .. } => {
                let generated = generated.ok_or_else(|| "selected GPU requires a device-prepared front spectral table; prepare calibration with the selected backend".to_owned())?;
                if generated.values.backend_id() != self.backend_id() {
                    return Err("front spectral table belongs to another device; rebuild calibration with the selected backend".into());
                }
                let lut = &generated
                    .values
                    .as_any()
                    .downcast_ref::<super::super::preparation::PreparationTable>()
                    .ok_or("front spectral table has an invalid device payload")?
                    .buffer;
                let pipe = self.cached_pointwise_pipeline(
                    include_str!(
                        "../../../../spektrafilm-shaders/wgsl/spectral/hanatos2025_rgb_to_raw.wgsl"
                    ),
                    &[wgpu::BufferBindingType::Uniform, ro, ro, rw],
                    &[(1, "rgb_in"), (3, "raw_out")],
                );
                self.chain_pointwise(
                    pipe,
                    &[&params, &rasters.a, lut, &rasters.b],
                    &[(1, Read), (3, Write)],
                    n,
                    true,
                    "front_transform",
                )?
                .submit(self)
            }
            spektrafilm_backend::FrontPass::Mallett2019 { .. } => {
                let pipe = self.cached_pointwise_pipeline(
                    include_str!(
                        "../../../../spektrafilm-shaders/wgsl/spectral/mallett_rgb_to_raw.wgsl"
                    ),
                    &[wgpu::BufferBindingType::Uniform, ro, rw],
                    &[(1, "rgb_in"), (2, "raw_out")],
                );
                self.chain_pointwise(
                    pipe,
                    &[&params, &rasters.a, &rasters.b],
                    &[(1, Read), (2, Write)],
                    n,
                    true,
                    "front_transform",
                )?
                .submit(self)
            }
            spektrafilm_backend::FrontPass::ScanInverse(_) => unreachable!("inverse topology"),
        }
    }

    fn chain_density(
        &self,
        input: &dyn RasterStorage,
        output: &dyn RasterStorage,
        geometry: (u32, u32),
        density: (&[f64], &[[f64; 3]], f64),
        label: &str,
    ) -> Result<(), String> {
        let (axis, curves, gamma) = density;
        let (params, axis, curves) =
            buffers::density(self, geometry.0, geometry.1, axis, curves, gamma);
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let pipe = self.cached_pointwise_pipeline(
            include_str!("../../../../spektrafilm-shaders/wgsl/spectral/density_curve_interp.wgsl"),
            &[wgpu::BufferBindingType::Uniform, ro, ro, ro, rw],
            &[(1, "log_raw"), (4, "output")],
        );
        self.chain_pointwise(
            pipe,
            &[&params, input, &axis, &curves, output],
            &[(1, Read), (4, Write)],
            geometry.0 * geometry.1,
            true,
            label,
        )?
        .submit(self)
    }

    fn chain_gaussian_interval(
        p: &spektrafilm_backend::FilmChainParams<'_>,
        steps: &[ChainStep],
    ) -> Option<(usize, usize)> {
        Some((
            steps.iter().position(|s| s.gaussian(p))?,
            steps.iter().rposition(|s| s.gaussian(p))?,
        ))
    }

    fn chain_resource_plan(
        &self,
        p: &spektrafilm_backend::FilmChainParams<'_>,
        steps: &[ChainStep],
        geometry: ChainGeometry,
    ) -> Result<(u64, Vec<ChainStageAllocation>), String> {
        let add = |a: u64, b: u64| {
            a.checked_add(b)
                .ok_or_else(|| "film chain working-set byte size overflow".to_owned())
        };
        let twice = |bytes: u64| {
            bytes
                .checked_mul(2)
                .ok_or_else(|| "film chain working-set byte size overflow".to_owned())
        };
        let source_bytes = raster_bytes(geometry.source)?;
        let field_bytes = raster_bytes(geometry.field)?;
        let tail_bytes = raster_bytes(geometry.tail)?;
        let gaussian_interval = Self::chain_gaussian_interval(p, steps);
        let gaussian = if gaussian_interval.is_some() {
            twice(raster_bytes(geometry.gaussian_extent(p, steps)?)?)?
        } else {
            0
        };
        // Native device accounting already owns retained input/cache/carriers.
        // Here each distinct future raster is counted once, never once per
        // cloned page view. Source aliases become tail slots in queue order.
        let field_is_distinct = steps.contains(&ChainStep::Dye)
            || steps.contains(&ChainStep::Fotufilm)
            || p.inject == Tap::CmyFilm;
        let mut tail_resident = twice(tail_bytes)?;
        if field_is_distinct {
            tail_resident = add(tail_resident, field_bytes)?;
        }
        if geometry.field != geometry.tail
            && steps.iter().any(|s| {
                matches!(
                    s,
                    ChainStep::DyePrintLog | ChainStep::Print | ChainStep::EnlargerLut
                )
            })
        {
            tail_resident = add(tail_resident, field_bytes)?;
        }
        let source_resident = twice(source_bytes)?;
        let mut peak = source_bytes; // immutable-input copy, including suffix-only spans
        let mut allocations = Vec::with_capacity(steps.len());
        let mut entered_tail = false;
        for (index, &step) in steps.iter().enumerate() {
            let extent = if step.source_stage() {
                geometry.source
            } else if matches!(
                step,
                ChainStep::DyePrintLog
                    | ChainStep::Print
                    | ChainStep::EnlargerLut
                    | ChainStep::PrintRoundtrip
                    | ChainStep::EnlargerDiffusion
            ) {
                geometry.field
            } else {
                geometry.tail
            };
            let mut resident = if step.source_stage() {
                source_resident
            } else {
                tail_resident
            };
            if !step.source_stage() && !entered_tail && geometry.tail != geometry.source {
                // enter_tail allocates the RGB destination before relinquishing
                // the source pair; its density destination is allocated later.
                resident = resident.max(add(
                    add(
                        source_resident,
                        if field_is_distinct { field_bytes } else { 0 },
                    )?,
                    tail_bytes,
                )?);
                entered_tail = true;
            }
            let stage = match step {
                ChainStep::Dye => {
                    let wavelengths = p.grain_dye_cell_spectral.then(|| {
                        let weights = if p.scan_film {
                            spektrafilm_math::spectral::N_WAVELENGTHS
                        } else {
                            p.print_sensitivity.len()
                        };
                        let illuminant = if p.scan_film {
                            p.scan_illuminant.len()
                        } else {
                            p.print_illuminant.len()
                        };
                        p.film_channel_density.len().min(illuminant).min(weights)
                    });
                    self.dye_workspace_bytes(
                        extent.0,
                        extent.1,
                        p.grain_dye.as_ref().ok_or("Dye parameters missing")?,
                        wavelengths,
                    )?
                }
                ChainStep::Fotufilm => self.fotufilm_workspace_bytes(
                    extent.0,
                    extent.1,
                    p.grain_fotufilm
                        .as_ref()
                        .ok_or("Fotufilm parameters missing")?,
                )?,
                _ => self.chain_effect_workspace_bytes(step.name(), p, extent.0, extent.1)?,
            };
            let live_gaussian = gaussian_interval
                .filter(|&(first, last)| first <= index && index <= last)
                .map_or(0, |_| gaussian);
            let stage_peak = add(add(resident, live_gaussian)?, stage)?;
            peak = peak.max(stage_peak);
            let mut pending = if matches!(step, ChainStep::Dye | ChainStep::Fotufilm) {
                stage
                    .checked_sub(field_bytes)
                    .ok_or("field workspace omits its output allocation")?
            } else {
                stage
            };
            // The pair stops being an application-owned shared carrier after
            // its last spatial consumer, but that consumer's submitted work can
            // still retain it. Charge it once until that submission retires.
            if gaussian_interval.is_some_and(|(_, last)| index == last) {
                pending = add(pending, gaussian)?;
            }
            allocations.push(ChainStageAllocation {
                peak: stage_peak,
                pending,
            });
        }
        if let Some(rect) = p.grain_dye_crop {
            let selected = match p.collect {
                Tap::LogEFilm => source_bytes,
                Tap::CmyFilm => field_bytes,
                _ => tail_bytes,
            };
            peak = peak.max(add(selected, raster_bytes((rect[2], rect[3]))?)?);
        }
        // The existing static cache's own cap bounds cold immutable uploads;
        // this is capacity headroom, not cumulative telemetry or VRAM residency.
        Ok((add(peak, 8 << 20)?, allocations))
    }

    pub(super) fn run_film_chain_inner(
        &self,
        p: &spektrafilm_backend::FilmChainParams<'_>,
        input: &spektrafilm_backend::DeviceImage,
        setup_context: &spektrafilm_backend::telemetry::ObservationContext,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.check_cancelled()?;
        let t_start = std::time::Instant::now();
        let steps = chain_steps(p)?;
        if input.is_retained() {
            self.buffer_payload(input)?;
        }
        if p.inject == p.collect {
            return Ok(input.clone());
        }
        let geometry = ChainGeometry::plan(p, &steps)?;
        if (input.width(), input.height()) != geometry.source {
            return Err(format!(
                "film chain input carrier is {}x{} but the resolved working geometry is {}x{}",
                input.width(),
                input.height(),
                geometry.source.0,
                geometry.source.1
            ));
        }
        if (steps.contains(&ChainStep::FilmDensity) && p.density_cpu_stage.is_some())
            || (p.collect == Tap::RgbOut && p.scan_acquisition_cpu_stage.is_some())
        {
            return Err("selected GPU film chain cannot execute a CPU image-stage fence; retry with an explicit CPU request".into());
        }
        if steps.contains(&ChainStep::Dye) {
            self.validate_dye_resources(
                geometry.source.0,
                geometry.source.1,
                p.grain_dye.as_ref().ok_or("Dye parameters missing")?,
            )?;
        }
        let (planned_peak_bytes, allocations) = self.chain_resource_plan(p, &steps, geometry)?;
        // Admission owns the retained-allocation inventory on portable backends
        // and uses the native counter instead when available. This plan contains
        // additional allocations only; input/carriers/caches are not charged twice.
        let operation = self.admit_operation(planned_peak_bytes)?;
        let backend = operation.backend(self);
        backend.execute_film_chain(
            p,
            input,
            setup_context,
            ChainPlan {
                steps,
                geometry,
                allocations,
            },
            t_start,
            &operation,
        )
    }

    fn execute_film_chain(
        &self,
        p: &spektrafilm_backend::FilmChainParams<'_>,
        input: &spektrafilm_backend::DeviceImage,
        setup_context: &spektrafilm_backend::telemetry::ObservationContext,
        plan: ChainPlan,
        t_start: std::time::Instant,
        operation: &super::super::resources::OperationGuard,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        let ChainPlan {
            steps,
            geometry,
            allocations,
        } = plan;
        let mut rasters = ChainRasters::default();
        let mut uploaded =
            self.allocate_image_pages(geometry.source.0, geometry.source.1, "chain_input_copy")?;
        if let Ok(payload) = self.buffer_payload(input) {
            if payload.bytes != raster_bytes(geometry.source)? {
                return Err("film chain retained input footprint does not match geometry".into());
            }
            let mut copy = super::super::grain::raster::RasterCommands::default();
            copy.copy(&payload.pages, 0, &uploaded, 0, payload.bytes);
            copy.submit(self)?;
        } else if let Some(image) = input.host_image() {
            for page in &uploaded {
                let first = page.first_pixel as usize * 3;
                let values = scalars_to_f32(&image.data[first..first + page.n_pixels as usize * 3]);
                self.queue
                    .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&values));
            }
        } else {
            return Err("film chain input is not available on this device".into());
        }
        match p.inject {
            Tap::LogEFilm => rasters.b = std::mem::take(&mut uploaded),
            Tap::LogEPrint => rasters.tail_b = std::mem::take(&mut uploaded),
            Tap::CmyPrint => rasters.tail_a = std::mem::take(&mut uploaded),
            Tap::CmyFilm => rasters.field = std::mem::take(&mut uploaded),
            _ => rasters.a = std::mem::take(&mut uploaded),
        }
        let (width, height) = geometry.source;
        let n = width * height;
        // Shared only by consecutive spatial stages. It never owns a published
        // carrier, and is released at the last spatial stage, not at chain end.
        let gaussian_geometry = geometry.gaussian_extent(p, &steps)?;
        let mut scratch = GaussianScratch::new(gaussian_geometry.0, gaussian_geometry.1);
        let mut pending_stages: Vec<(super::super::resources::Retirement, u64)> = Vec::new();
        for (index, &step) in steps.iter().enumerate() {
            self.check_cancelled()?;
            let allocation = allocations[index];
            pending_stages.retain(|(submission, _)| !submission.is_complete());
            let retained = pending_stages
                .iter()
                .try_fold(0u64, |sum, (_, bytes)| sum.checked_add(*bytes))
                .ok_or("film chain pending workspace byte size overflow")?;
            if allocation
                .peak
                .checked_add(retained)
                .is_none_or(|bytes| bytes > operation.reserved_bytes())
            {
                // Queue order permits reuse; returning old temporary allocations
                // to the application budget additionally requires retirement.
                // This wait is only a pressure path, never a per-stage fence.
                operation.retire_pending();
                pending_stages.clear();
            }
            let before_submission = operation.last_submission();
            let device_step = super::super::pipe::ChainStage::Device {
                name: step.name(),
                execute: Box::new(|| {
                    use ChainStep::*;
                    if step.source_stage() {
                        if !matches!(
                            step,
                            Log | Highlight | CameraDiffusion | CameraBlur | Halation | FilmDensity
                        ) {
                            ensure_pages(self, &mut rasters.a, geometry.source, "chain_work_a")?;
                        }
                        if !matches!(step, InputDecode | GrainV1 | Dye | Fotufilm) {
                            ensure_pages(self, &mut rasters.b, geometry.source, "chain_work_b")?;
                        }
                        if step == FilmDensity {
                            ensure_pages(self, &mut rasters.a, geometry.source, "chain_work_a")?;
                        }
                    } else {
                        rasters.enter_tail(self, geometry)?;
                    }
                    match step {
                        InputDecode => build_input_decode_state(
                            &self.device,
                            p.input_decode.as_ref().ok_or("input decode missing")?,
                            &rasters.a,
                            n,
                            self,
                        )?
                        .submit(self),
                        Inverse => {
                            let spektrafilm_backend::FrontPass::ScanInverse(params) = &p.front
                            else {
                                unreachable!()
                            };
                            self.build_convert_inverse_state(params, &rasters.a, &rasters.b, n)?
                                .submit(self)?;
                            let mut copy = super::super::grain::raster::RasterCommands::default();
                            copy.copy(&rasters.b, 0, &rasters.a, 0, raster_bytes(geometry.source)?);
                            copy.submit(self)
                        }
                        Front => self.chain_front(p, &rasters, n),
                        Highlight => build_highlight_boost_state(
                            &self.device,
                            p.highlight_boost.as_ref().ok_or("highlight missing")?,
                            n,
                            &rasters.b,
                            self,
                        )?
                        .submit(self),
                        CameraDiffusion => self.diffusion_transport(
                            width,
                            height,
                            &rasters.b,
                            &rasters.b,
                            p.camera_diffusion
                                .as_ref()
                                .ok_or("camera diffusion missing")?,
                            None,
                        ),
                        CameraBlur | ScannerBlur => {
                            let (extent, image, sigma, label) = if step == CameraBlur {
                                (
                                    geometry.source,
                                    &rasters.b,
                                    p.camera_lens_blur_px.ok_or("camera blur missing")?,
                                    "camera_lens",
                                )
                            } else {
                                (
                                    geometry.tail,
                                    &rasters.tail_b,
                                    p.scanner_lens_blur_px.ok_or("scanner blur missing")?,
                                    "scanner_lens",
                                )
                            };
                            let buffers = scratch.buffers(self, extent.0, extent.1)?;
                            build_simple_blur_state(
                                self,
                                sigma,
                                extent.0,
                                extent.1,
                                image,
                                &buffers.output,
                                &buffers.intermediate,
                                label,
                            )?
                            .submit(self, image)
                        }
                        Halation => build_halation_state(
                            p.halation.as_ref().ok_or("halation missing")?,
                            width,
                            height,
                            &rasters.b,
                            &mut scratch,
                            self,
                        )?
                        .submit(self),
                        Log => {
                            let params = buffers::uniform(
                                self,
                                "log10_params",
                                bytemuck::cast_slice(&[n, 0, 0, 0, 0, 0, 0, 0]),
                            );
                            let pipe = self.cached_pointwise_pipeline(include_str!("../../../../spektrafilm-shaders/wgsl/spectral/log10_inplace.wgsl"),
                                &[wgpu::BufferBindingType::Uniform, wgpu::BufferBindingType::Storage { read_only: false }], &[(1, "data")]);
                            self.chain_pointwise(
                                pipe,
                                &[&params, &rasters.b],
                                &[(1, ReadWrite)],
                                n,
                                false,
                                "log_exposure",
                            )?
                            .submit(self)
                        }
                        FilmDensity => self.chain_density(
                            &rasters.b,
                            &rasters.a,
                            geometry.source,
                            (
                                p.film_log_exposure,
                                p.film_density_curves_normalized,
                                p.film_gamma,
                            ),
                            "film_density",
                        ),
                        Dir => build_dir_state(
                            &self.device,
                            p.dir_couplers.as_ref().ok_or("DIR missing")?,
                            width,
                            height,
                            &rasters.a,
                            &rasters.b,
                            &mut scratch,
                            self,
                        )?
                        .submit_passes(self),
                        GrainV1 => build_spektra_grain_state(
                            &self.device,
                            width,
                            height,
                            &rasters.a,
                            p.grain_v1.as_ref().ok_or("grain missing")?,
                            self,
                        )?
                        .submit(self),
                        Dye => {
                            let source = super::device_image::BufferPayload::from_pages(
                                self.backend_id(),
                                rasters.a.clone(),
                                width,
                                height,
                            );
                            let response = if p.grain_dye_cell_spectral {
                                Some(if p.scan_film {
                                    spektrafilm_backend::DyeResponseGpuParams::Scan {
                                        channel_density: p.film_channel_density,
                                        base_density: p.film_base_density,
                                        illuminant: p.scan_illuminant,
                                        normalization: p.scan_normalization,
                                        xyz_to_rgb: p.scan_xyz_to_rgb,
                                    }
                                } else {
                                    spektrafilm_backend::DyeResponseGpuParams::Print {
                                        channel_density: p.film_channel_density,
                                        base_density: p.film_base_density,
                                        illuminant: p.print_illuminant,
                                        sensitivity: p.print_sensitivity,
                                        normalization_factor: p.print_normalization_factor,
                                        exposure_scale: p.print_exposure_scale,
                                        preflash: p.preflash,
                                    }
                                })
                            } else {
                                None
                            };
                            let state = self.prepare_dye_paged(
                                width,
                                height,
                                &source,
                                p.grain_dye.as_ref().ok_or("Dye missing")?,
                                response.as_ref(),
                            )?;
                            state.submit(self)?;
                            self.check_dye_status(&state.status_buffer())?;
                            rasters.field = state.output_pages();
                            if p.grain_dye_cell_spectral && p.scan_film {
                                // The response output is already scanner RGB. Keep a
                                // distinct terminal carrier, even when extents alias.
                                ensure_pages(
                                    self,
                                    &mut rasters.tail_b,
                                    geometry.tail,
                                    "field_tail_rgb",
                                )?;
                                let mut copy =
                                    super::super::grain::raster::RasterCommands::default();
                                copy.copy(
                                    &rasters.field,
                                    0,
                                    &rasters.tail_b,
                                    0,
                                    raster_bytes(geometry.field)?,
                                );
                                copy.submit(self)?;
                            }
                            Ok(())
                        }
                        Fotufilm => {
                            let source = super::device_image::BufferPayload::from_pages(
                                self.backend_id(),
                                rasters.a.clone(),
                                width,
                                height,
                            );
                            let state = self.prepare_fotufilm_paged(
                                width,
                                height,
                                &source,
                                p.grain_fotufilm.as_ref().ok_or("Fotufilm missing")?,
                            )?;
                            state.submit(self)?;
                            rasters.field = state.output_pages();
                            Ok(())
                        }
                        DyePrintLog => {
                            rasters.fine_output(self, geometry)?;
                            let source = super::device_image::BufferPayload::from_pages(
                                self.backend_id(),
                                rasters.field.clone(),
                                geometry.field.0,
                                geometry.field.1,
                            );
                            let pages = self.dye_log10_pages(&source)?;
                            let mut copy = super::super::grain::raster::RasterCommands::default();
                            copy.copy(&pages, 0, &rasters.fine, 0, raster_bytes(geometry.field)?);
                            copy.submit(self)
                        }
                        EnlargerLut => {
                            rasters.fine_output(self, geometry)?;
                            self.spectral_lut_transport(
                                &rasters.field,
                                &rasters.fine,
                                geometry.field.0 * geometry.field.1,
                                |source, output, count| {
                                    self.execute_enlarger_lut_resident(
                                        source,
                                        output,
                                        count,
                                        p.enlarger_lut.as_ref().ok_or("enlarger LUT missing")?,
                                    )
                                },
                            )
                        }
                        PrintRoundtrip => self
                            .print_lut_roundtrip(
                                &rasters.fine,
                                geometry.field.0 * geometry.field.1,
                                p.print_exposure_scale,
                            )?
                            .submit(self),
                        Print => {
                            rasters.fine_output(self, geometry)?;
                            let [params, cd, bd, illuminant, sensitivity] =
                                buffers::print(self, p, geometry.field.0, geometry.field.1);
                            let ro = wgpu::BufferBindingType::Storage { read_only: true };
                            let rw = wgpu::BufferBindingType::Storage { read_only: false };
                            let pipe = self.cached_pointwise_pipeline(include_str!("../../../../spektrafilm-shaders/wgsl/spectral/print_spectral_calibrated.wgsl"),
                                &[wgpu::BufferBindingType::Uniform, ro, ro, ro, ro, ro, rw], &[(1, "density_cmy"), (6, "output")]);
                            self.chain_pointwise(
                                pipe,
                                &[
                                    &params,
                                    &rasters.field,
                                    &cd,
                                    &bd,
                                    &illuminant,
                                    &sensitivity,
                                    &rasters.fine,
                                ],
                                &[(1, Read), (6, Write)],
                                geometry.field.0 * geometry.field.1,
                                true,
                                "print_spectral",
                            )?
                            .submit(self)
                        }
                        EnlargerDiffusion => self.diffusion_transport(
                            geometry.field.0,
                            geometry.field.1,
                            &rasters.fine,
                            &rasters.fine,
                            p.enlarger_diffusion
                                .as_ref()
                                .ok_or("enlarger diffusion missing")?,
                            Some(p.print_exposure_scale),
                        ),
                        Collapse => {
                            let source = super::device_image::BufferPayload::from_pages(
                                self.backend_id(),
                                rasters.fine.clone(),
                                geometry.field.0,
                                geometry.field.1,
                            );
                            let (pages, _, _) = self.dye_aperture_collapse_pages(
                                &source,
                                p.grain_dye_aperture.ok_or("aperture missing")?,
                            )?;
                            let mut copy = super::super::grain::raster::RasterCommands::default();
                            copy.copy(&pages, 0, &rasters.tail_b, 0, raster_bytes(geometry.tail)?);
                            copy.submit(self)
                        }
                        PrintDensity => {
                            ensure_pages(
                                self,
                                &mut rasters.tail_a,
                                geometry.tail,
                                "field_tail_density",
                            )?;
                            self.chain_density(
                                &rasters.tail_b,
                                &rasters.tail_a,
                                geometry.tail,
                                (p.print_log_exposure, p.print_density_curves, p.print_gamma),
                                "print_density",
                            )
                        }
                        Readout => {
                            let params =
                                p.scan_readout.ok_or("scanner density parameters missing")?;
                            let source = if p.scan_film {
                                &rasters.field
                            } else {
                                &rasters.tail_a
                            };
                            let state = self.build_scan_density_state(
                                params,
                                source,
                                &rasters.tail_b,
                                geometry.tail.0,
                                geometry.tail.1,
                                p.scan_preparation_key,
                            )?;
                            self.pipe
                                .run_chain(vec![super::super::pipe::ChainStage::Device {
                                    name: "scanner_optics",
                                    execute: Box::new(|| state.submit_optics(self)),
                                }])?;
                            if state.has_setup() {
                                self.pipe.run_chain(vec![
                                    super::super::pipe::ChainStage::Device {
                                        name: "standard_setup",
                                        execute: Box::new(|| {
                                            state.submit_setup_observed(self, setup_context)
                                        }),
                                    },
                                ])?;
                            }
                            state.submit_inverse(self)?;
                            state.check(
                                self,
                                params,
                                p.scan_preparation_key,
                                geometry.tail.0,
                                geometry.tail.1,
                            )
                        }
                        ScannerLut => {
                            let source = if p.scan_film {
                                &rasters.field
                            } else {
                                &rasters.tail_a
                            };
                            self.spectral_lut_transport(
                                source,
                                &rasters.tail_b,
                                geometry.tail.0 * geometry.tail.1,
                                |source, output, count| {
                                    self.execute_scanner_lut_resident(
                                        source,
                                        output,
                                        count,
                                        p.scanner_lut.as_ref().ok_or("scanner LUT missing")?,
                                    )
                                },
                            )
                        }
                        Scan => {
                            let source = if p.scan_film {
                                &rasters.field
                            } else {
                                &rasters.tail_a
                            };
                            self.build_scan_direct_state(
                                p,
                                source,
                                &rasters.tail_b,
                                geometry.tail.0,
                                geometry.tail.1,
                            )?
                            .submit(self)
                        }
                        Glare => build_glare_state(
                            &self.device,
                            p.glare.as_ref().ok_or("glare missing")?,
                            geometry.tail.0,
                            geometry.tail.1,
                            &rasters.tail_b,
                            &mut scratch,
                            self,
                        )?
                        .submit(self),
                        Gamut => build_gamut_state(
                            &self.device,
                            p.gamut.as_ref().ok_or("gamut missing")?,
                            &rasters.tail_b,
                            geometry.tail.0 * geometry.tail.1,
                            self,
                        )?
                        .submit(self),
                        Unsharp => build_unsharp_state(
                            &self.device,
                            p.unsharp.as_ref().ok_or("unsharp missing")?,
                            geometry.tail.0,
                            geometry.tail.1,
                            &rasters.tail_b,
                            &mut scratch,
                            self,
                        )?
                        .submit(self),
                        GrainV2 => build_grain_v2_state(
                            &self.device,
                            p.grain_v2.as_ref().ok_or("grain missing")?,
                            geometry.tail.0,
                            geometry.tail.1,
                            &rasters.tail_b,
                            Some(p.scan_output_space),
                            self,
                        )?
                        .submit(self, &rasters.tail_b),
                        Transfer => self
                            .chain_output_transfer(
                                &rasters.tail_b,
                                geometry.tail.0 * geometry.tail.1,
                                p.output_transfer
                                    .as_ref()
                                    .ok_or("output transfer missing")?,
                            )?
                            .submit(self),
                        Cmyk => self
                            .chain_cmyk(
                                &rasters.tail_b,
                                geometry.tail.0 * geometry.tail.1,
                                p.cmyk.as_ref().ok_or("CMYK missing")?,
                            )?
                            .submit(self),
                    }
                }),
            };
            self.pipe.run_chain(vec![device_step])?;
            if let Some(submission) = operation.last_submission()
                && !submission.is_complete()
                && before_submission
                    .as_ref()
                    .is_none_or(|before| !before.same_submission(&submission))
            {
                pending_stages.push((submission, allocation.pending));
            }
            // Release cross-stage owners at their actual last consumer. Later
            // states have not been built and cannot accidentally pin old pages.
            let remaining = &steps[index + 1..];
            let source_consumer = |s: &ChainStep| s.source_stage();
            if !remaining.iter().any(source_consumer) {
                if rasters.field.is_empty() && p.collect != Tap::LogEFilm {
                    rasters.field = rasters.a.clone();
                }
                if remaining.is_empty() {
                    rasters.a.clear();
                    if p.collect != Tap::LogEFilm {
                        rasters.b.clear();
                    }
                }
            }
            if !remaining.iter().any(|s| {
                matches!(
                    s,
                    ChainStep::DyePrintLog
                        | ChainStep::EnlargerLut
                        | ChainStep::Print
                        | ChainStep::Readout
                        | ChainStep::ScannerLut
                        | ChainStep::Scan
                )
            }) && p.collect != Tap::CmyFilm
            {
                rasters.field.clear();
            }
            if !remaining.iter().any(|s| {
                matches!(
                    s,
                    ChainStep::PrintRoundtrip | ChainStep::EnlargerDiffusion | ChainStep::Collapse
                )
            }) {
                rasters.fine.clear();
            }
            if !remaining.iter().any(|s| {
                matches!(
                    s,
                    ChainStep::PrintDensity
                        | ChainStep::Readout
                        | ChainStep::ScannerLut
                        | ChainStep::Scan
                )
            }) && p.collect != Tap::CmyPrint
            {
                rasters.tail_a.clear();
            }
            if !remaining.iter().any(|s| s.gaussian(p)) {
                scratch = GaussianScratch::new(gaussian_geometry.0, gaussian_geometry.1);
            }
        }
        self.check_cancelled()?;
        let (selected, extent) = match p.collect {
            Tap::LogEFilm => (rasters.b, geometry.source),
            Tap::CmyFilm => (rasters.field, geometry.field),
            Tap::LogEPrint => (rasters.tail_b, geometry.tail),
            Tap::CmyPrint => (rasters.tail_a, geometry.tail),
            Tap::RgbOut => (rasters.tail_b, geometry.tail),
            Tap::RgbIn | Tap::RgbPre => unreachable!("equal boundary returned before allocation"),
        };
        tracing::debug!(target: "spektrafilm_gpu::wgpu_backend", cpu_setup_ms = format!("{:.1}", t_start.elapsed().as_secs_f32() * 1000.0), "film chain timings");
        let encoded = match p.output_transfer.as_ref() {
            Some(transfer) => !transfer.decode,
            None => p.grain_v2.is_some(),
        };
        let domain = match p.collect {
            Tap::LogEFilm | Tap::LogEPrint => spektrafilm_backend::SampleDomain::LogExposure,
            Tap::CmyFilm | Tap::CmyPrint => spektrafilm_backend::SampleDomain::Density,
            Tap::RgbOut if encoded => spektrafilm_backend::SampleDomain::Encoded,
            _ => spektrafilm_backend::SampleDomain::Linear,
        };
        let pixel_pitch_um = p
            .grain_dye
            .as_ref()
            .filter(|_| tap_index(p.collect) >= tap_index(Tap::CmyFilm))
            .map_or(p.pixel_pitch_um, |dye| {
                let axis = usize::from(p.image_height > p.image_width);
                dye.grid.pitch_um[axis] * f64::from(p.grain_dye_aperture.unwrap_or(1))
            });
        let pixel_pitch_um = p
            .grain_fotufilm
            .as_ref()
            .filter(|_| tap_index(p.collect) >= tap_index(Tap::CmyFilm))
            .map_or(pixel_pitch_um, |grain| {
                let axis = usize::from(p.image_height > p.image_width);
                grain.pitch_um[axis] * grain.size.clamp(0.5, 4.0)
            });
        let payload = super::device_image::BufferPayload::from_pages(
            self.backend_id(),
            selected,
            extent.0,
            extent.1,
        );
        let (pages, width, height) = if let Some(rect) = p.grain_dye_crop {
            pending_stages.retain(|(submission, _)| !submission.is_complete());
            let pending = pending_stages
                .iter()
                .try_fold(0u64, |sum, (_, bytes)| sum.checked_add(*bytes))
                .ok_or("film chain pending workspace byte size overflow")?;
            let crop_peak = payload
                .bytes
                .checked_add(raster_bytes((rect[2], rect[3]))?)
                .and_then(|bytes| bytes.checked_add(pending));
            if crop_peak.is_none_or(|bytes| bytes > operation.reserved_bytes()) {
                operation.retire_pending();
            }
            (self.dye_crop_pages(&payload, rect)?, rect[2], rect[3])
        } else {
            (payload.pages, extent.0, extent.1)
        };
        Ok(self.retain_image_pages(
            pages,
            width,
            height,
            domain,
            if p.collect == Tap::RgbOut {
                p.scan_output_space
            } else {
                input.space()
            },
            Some(pixel_pitch_um),
        ))
    }
}

// ── Bounded-execution preflight (pure, device-free) ──────────────────────────
//
// Resident rasters bind compatible STORAGE views directly. This scalar reports
// the largest active immutable binding, not aggregate operation capacity; the
// checked stage plan above owns carrier, workspace, and retirement headroom.

/// Floor covering fixed parameter blocks. Configuration-sized kernels and
/// fields are validated by their owning stage and counted by the operation
/// workspace plan; this value is not a bound on Gaussian support.
const MAX_FIXED_AUX_BYTES: u64 = 4 * 1024;

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
    // Spektra v1's device pass uploads the composite curve x column
    // (`knots` × 3) and the layer curve tensor (`knots` × 9) as f64 storage.
    let grain_v1_curve_bytes = p.grain_v1.as_ref().map_or(0, |gp| {
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
        needed(ChainStep::GrainV1, grain_v1_curve_bytes),
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

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::{ComputeBackend, FilmChainParams, FrontPass};
    use spektrafilm_backend::{Precision, RetainedImageMeta, SampleDomain};
    use spektrafilm_math::colorspace;
    use spektrafilm_math::image::ImageBuf;

    const MAT3: [[f64; 3]; 3] = [[0.0; 3]; 3];

    /// Minimal chain params: empty lookup tables, no optional stages. The
    /// geometry is declared directly so oversized sizes cost no allocation.
    fn params<'a>(width: u32, height: u32) -> FilmChainParams<'a> {
        FilmChainParams {
            inject: Tap::RgbPre,
            collect: Tap::RgbOut,
            enlarger_lut: None,
            scanner_lut: None,
            image_width: width,
            image_height: height,
            pixel_pitch_um: 1.0,
            front: FrontPass::Mallett2019 { matrix: MAT3 },
            film_log_exposure: &[],
            film_density_curves_normalized: &[],
            film_gamma: 1.0,
            film_channel_density: &[],
            film_base_density: &[],
            print_illuminant: &[],
            print_sensitivity: &[],
            print_normalization_factor: 1.0,
            print_exposure_scale: 1.0,
            print_log_exposure: &[],
            print_density_curves: &[],
            grain_fotufilm: None,
            print_gamma: 1.0,
            print_channel_density: &[],
            print_base_density: &[],
            preflash: [0.0; 3],
            scan_illuminant: &[],
            scan_normalization: 1.0,
            scan_xyz_to_rgb: &MAT3,
            scan_cat: &MAT3,
            scan_base_xyz_to_rgb: &MAT3,
            scan_film: false,
            halation: None,
            dir_couplers: None,
            glare: None,
            gamut: None,
            unsharp: None,
            grain_v2: None,
            grain_v1: None,
            scan_output_space: &spektrafilm_math::colorspace::SRGB,
            camera_lens_blur_px: None,
            camera_diffusion: None,
            enlarger_diffusion: None,
            scanner_lens_blur_px: None,
            highlight_boost: None,
            grain_dye: None,
            grain_dye_cell_spectral: false,
            grain_dye_aperture: None,
            grain_dye_crop: None,
            density_cpu_stage: None,
            scan_acquisition_cpu_stage: None,
            scan_readout: None,
            scan_preparation_key: None,
            output_transfer: None,
            cmyk: None,
            input_decode: None,
        }
    }

    #[test]
    fn prefix_tap_does_not_allocate_or_validate_inactive_tail_resources() {
        use spektrafilm_backend::telemetry::{CollectionMode, ObservationContext};

        let gpu = crate::test_support::explicit_adapter();
        let mut p = params(256, 8);
        p.front = FrontPass::Mallett2019 {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        p.collect = Tap::LogEFilm;
        // These active-looking tail parameters must never construct resources
        // for a prefix tap. Empty density/spectral tables are intentionally
        // unusable downstream, so executing the tail would also fail.
        p.scanner_lens_blur_px = Some(8.0);
        p.grain_dye_aperture = Some(0);
        let host = ImageBuf::from_data(256, 8, f32_to_scalars(vec![1.0; 256 * 8 * 3]));
        let input = gpu
            .retain_image(
                &host,
                &RetainedImageMeta {
                    width: 256,
                    height: 8,
                    domain: SampleDomain::Linear,
                    precision: Precision::host(),
                    space: &colorspace::SRGB,
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        let context = ObservationContext::new(CollectionMode::Summary);
        let mut observed = gpu.derived_explicit();
        observed.context_mut(context.clone());
        let output = observed.run_film_chain_retained(&input, &p).unwrap();
        let bytes = context.snapshot().totals.buffer_create_bytes.unwrap();
        assert!(
            bytes <= 2 * raster_bytes((256, 8)).unwrap() + 4096,
            "prefix allocated {bytes} bytes"
        );
        assert_eq!(output.domain(), SampleDomain::LogExposure);
        assert!(
            output
                .to_host(gpu.backend())
                .unwrap()
                .data
                .iter()
                .all(|&v| v == 0.0)
        );
        assert_eq!(input.to_host(gpu.backend()).unwrap().data, host.data);
    }

    #[test]
    fn sequential_lens_jobs_keep_output_alive_after_stage_and_scratch_drop() {
        let gpu = crate::test_support::explicit_adapter();
        let mut host = ImageBuf::new(256, 8);
        let center = (4 * 256 + 128) * 3;
        host.data[center..center + 3].copy_from_slice(&[0.25 as _, 0.5 as _, 1.0 as _]);
        let pages = gpu
            .allocate_image_pages(256, 8, "lens_lifetime_input")
            .unwrap();
        for page in &pages {
            let first = page.first_pixel as usize * 3;
            let values = scalars_to_f32(&host.data[first..first + page.n_pixels as usize * 3]);
            gpu.queue
                .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&values));
        }
        {
            let mut scratch = GaussianScratch::new(256, 8);
            for sigma in [1.0, 2.0] {
                let buffers = scratch.buffers(&gpu, 256, 8).unwrap();
                build_simple_blur_state(
                    &gpu,
                    sigma,
                    256,
                    8,
                    &pages,
                    &buffers.output,
                    &buffers.intermediate,
                    "sequential_lens",
                )
                .unwrap()
                .submit(&gpu, &pages)
                .unwrap();
            }
        }
        let output =
            gpu.retain_image_pages(pages, 256, 8, SampleDomain::Linear, &colorspace::SRGB, None);
        let expected = gpu.gaussian_blur_gpu(&host, 1.0).unwrap();
        let expected = gpu.gaussian_blur_gpu(&expected, 2.0).unwrap();
        for (actual, expected) in output
            .to_host(gpu.backend())
            .unwrap()
            .data
            .iter()
            .zip(&expected.data)
        {
            assert!((*actual as f64 - *expected as f64).abs() < 2e-6);
        }
    }

    #[test]
    fn tap_plan_limits_geometry_validation_to_actual_stage_span() {
        let mut p = params(8, 4);
        p.collect = Tap::LogEFilm;
        p.scanner_lens_blur_px = Some(3.0);
        p.grain_dye_aperture = Some(0);
        let prefix = chain_steps(&p).unwrap();
        assert_eq!(prefix, [ChainStep::Front, ChainStep::Log]);
        assert_eq!(ChainGeometry::plan(&p, &prefix).unwrap().tail, (8, 4));
        p.collect = Tap::RgbOut;
        assert!(
            ChainGeometry::plan(&p, &chain_steps(&p).unwrap())
                .unwrap_err()
                .contains("nonzero")
        );
        p.grain_dye_aperture = None;
        p.inject = Tap::LogEPrint;
        p.collect = Tap::CmyPrint;
        assert_eq!(chain_steps(&p).unwrap(), [ChainStep::PrintDensity]);
        for extent in [(0, 1), (u32::MAX, 2), (u32::MAX / 3 + 1, 1)] {
            assert!(
                raster_bytes(extent).is_err(),
                "accepted invalid geometry {extent:?}"
            );
        }
    }

    #[test]
    fn shared_gaussian_capacity_is_charged_only_from_first_to_last_consumer() {
        let gpu = crate::test_support::explicit_adapter();
        let mut p = params(8, 4);
        p.camera_lens_blur_px = Some(1.0);
        p.collect = Tap::CmyFilm;
        let steps = [
            ChainStep::FilmDensity,
            ChainStep::CameraBlur,
            ChainStep::FilmDensity,
        ];
        let geometry = ChainGeometry::plan(&p, &steps).unwrap();
        let (_, baseline) = gpu
            .chain_resource_plan(&p, &[ChainStep::FilmDensity], geometry)
            .unwrap();
        let (_, plan) = gpu.chain_resource_plan(&p, &steps, geometry).unwrap();
        assert_eq!(
            WgpuBackend::chain_gaussian_interval(&p, &steps),
            Some((1, 1))
        );
        assert_eq!(
            plan[0].peak, baseline[0].peak,
            "pair allocated before first Gaussian"
        );
        assert_eq!(
            plan[2].peak, baseline[0].peak,
            "pair charged after its last use"
        );
        let stage = gpu
            .chain_effect_workspace_bytes("camera_lens_blur", &p, 8, 4)
            .unwrap();
        assert_eq!(
            plan[1].pending,
            stage + 2 * raster_bytes((8, 4)).unwrap(),
            "last Gaussian submission must retain pair capacity until retirement"
        );
        let prefix = [
            ChainStep::CameraBlur,
            ChainStep::FilmDensity,
            ChainStep::GrainV1,
        ];
        assert_eq!(
            WgpuBackend::chain_gaussian_interval(&p, &prefix),
            Some((0, 0))
        );
    }

    #[test]
    fn retained_tap_segments_preserve_exposure_and_development_boundaries() {
        use spektrafilm_backend::ComputeBackend;
        let backend = crate::test_support::adapter();
        let knots = [-1.0, 0.0, 1.0];
        let curves = [[0.0; 3], [1.0; 3], [2.0; 3]];
        let spectral = [[0.0; 3]; 81];
        let base = [0.0; 81];
        let light = [1.0; 81];
        let inactive_lut = spektrafilm_math::spectral::TcLut {
            size: 1024,
            channels: 3,
            data: Vec::new(),
        };
        let mut p = params(1, 1);
        p.front = FrontPass::Mallett2019 {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        p.film_log_exposure = &knots;
        p.film_density_curves_normalized = &curves;
        p.print_log_exposure = &knots;
        p.print_density_curves = &curves;
        p.film_channel_density = &spectral;
        p.print_channel_density = &spectral;
        p.film_base_density = &base;
        p.print_base_density = &base;
        p.print_illuminant = &light;
        p.scan_illuminant = &light;
        p.print_sensitivity = &spectral;
        p.collect = Tap::LogEFilm;
        let source = ImageBuf::from_data(1, 1, f32_to_scalars(vec![0.1, 1.0, 10.0]));
        let input = backend
            .retain_image(
                &source,
                &RetainedImageMeta {
                    width: 1,
                    height: 1,
                    domain: SampleDomain::Linear,
                    precision: Precision::host(),
                    space: &colorspace::SRGB,
                    pixel_pitch_um: Some(1.0),
                },
            )
            .unwrap();
        let repeated = ImageBuf::from_data(3, 1, source.data.repeat(3));
        let split = (0..3)
            .map(|i| {
                let samples: Vec<f32> = repeated.data[i * 3..i * 3 + 3]
                    .iter()
                    .map(|&v| v as f32)
                    .collect();
                super::super::super::paged::ImagePage {
                    first_pixel: i as u32,
                    n_pixels: 1,
                    buffer: backend
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("split_source_pixel"),
                            contents: bytemuck::cast_slice(&samples),
                            usage: wgpu::BufferUsages::STORAGE
                                | wgpu::BufferUsages::COPY_SRC
                                | wgpu::BufferUsages::COPY_DST,
                        }),
                }
            })
            .collect();
        let paged = backend.retain_image_pages(
            split,
            3,
            1,
            SampleDomain::Linear,
            &colorspace::SRGB,
            Some(1.0),
        );
        let contiguous = backend
            .retain_image(
                &repeated,
                &RetainedImageMeta {
                    width: 3,
                    height: 1,
                    domain: SampleDomain::Linear,
                    precision: Precision::host(),
                    space: &colorspace::SRGB,
                    pixel_pitch_um: Some(1.0),
                },
            )
            .unwrap();
        p.image_width = 3;
        for carrier in [&contiguous, &paged] {
            p.collect = Tap::CmyFilm;
            let first = backend.run_film_chain_retained(carrier, &p).unwrap();
            let second = backend.run_film_chain_retained(carrier, &p).unwrap();
            assert_eq!(
                first.to_host(backend.backend()).unwrap().data,
                second.to_host(backend.backend()).unwrap().data
            );
            assert_eq!(
                carrier.to_host(backend.backend()).unwrap().data,
                repeated.data
            );
            let mixed = backend
                .mix_output_images(
                    &first,
                    carrier,
                    &spektrafilm_backend::OutputMixGpuParams {
                        amount: 0.0,
                        encoded: false,
                        cctf_kind: 0,
                    },
                )
                .unwrap();
            assert_eq!(
                mixed.to_host(backend.backend()).unwrap().data,
                repeated.data
            );
            p.density_cpu_stage = Some(Box::new(|_| panic!("forbidden CPU image stage")));
            assert!(backend.run_film_chain_retained(carrier, &p).is_err());
            assert_eq!(
                carrier.to_host(backend.backend()).unwrap().data,
                repeated.data
            );
            p.density_cpu_stage = None;
        }
        p.image_width = 1;
        p.collect = Tap::LogEFilm;
        let mut identity_params = params(1, 1);
        identity_params.inject = Tap::CmyFilm;
        identity_params.collect = Tap::CmyFilm;
        let identity = backend
            .run_film_chain_retained(&input, &identity_params)
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(
            input.payload().unwrap(),
            identity.payload().unwrap()
        ));
        assert_eq!(identity.domain(), input.domain());
        assert_eq!(identity.pixel_pitch_um(), input.pixel_pitch_um());
        assert_eq!((identity.width(), identity.height()), (1, 1));
        let exposure = backend.run_film_chain_retained(&input, &p).unwrap();
        assert_eq!(exposure.domain(), SampleDomain::LogExposure);
        for (actual, expected) in exposure
            .to_host(backend.backend())
            .unwrap()
            .data
            .iter()
            .zip([-1.0, 0.0, 1.0])
        {
            assert!((*actual as f64 - expected).abs() < 2e-6);
        }
        p.inject = Tap::LogEFilm;
        p.collect = Tap::CmyFilm;
        // An injected development span must not validate or upload the inactive
        // front transform, including its missing device-generated calibration.
        p.front = FrontPass::Hanatos2025 {
            tc_lut: &inactive_lut,
            generated: None,
            rgb_to_adapted_xyz: MAT3,
        };
        assert_eq!(required_chain_buffers(&p).static_bytes, MAX_FIXED_AUX_BYTES);
        // Re-running the front/log passes would produce the wrong density;
        // running the scanner tail would produce zeros with this scan matrix.
        let density = backend.run_film_chain_retained(&exposure, &p).unwrap();
        assert_eq!(density.domain(), SampleDomain::Density);
        for (actual, expected) in density
            .to_host(backend.backend())
            .unwrap()
            .data
            .iter()
            .zip([0.0, 1.0, 2.0])
        {
            assert!((*actual as f64 - expected).abs() < 2e-6);
        }
        p.inject = Tap::LogEPrint;
        p.collect = Tap::CmyPrint;
        let paper = backend.run_film_chain_retained(&exposure, &p).unwrap();
        assert_eq!(paper.domain(), SampleDomain::Density);
        for (actual, expected) in paper
            .to_host(backend.backend())
            .unwrap()
            .data
            .iter()
            .zip([0.0, 1.0, 2.0])
        {
            assert!((*actual as f64 - expected).abs() < 2e-6);
        }
        p.inject = Tap::LogEFilm;
        p.collect = Tap::CmyFilm;
        p.density_cpu_stage = Some(Box::new(|_| {
            panic!("GPU diagnostics must not run CPU image arithmetic")
        }));
        let error = backend.run_film_chain_retained(&exposure, &p).unwrap_err();
        assert!(error.contains("CPU image-stage fence"), "{error}");
    }

    #[test]
    fn dye_carriers_preserve_segmented_routes_crops_and_prior_outputs() {
        use spektrafilm_backend::dye_params::{
            DyeChannelParts, DyeChannelTablesGpu, DyeGpuParams, DyeReadoutGrid,
        };

        let gpu = crate::test_support::adapter();
        let knots = [-1.0, 0.0, 1.0];
        let curves = [[0.0; 3], [1.0; 3], [2.0; 3]];
        let spectral = [[0.1, 0.2, 0.3]; 81];
        let base = [0.0; 81];
        let light = [1.0; 81];
        let sensitivity = [[0.01, 0.02, 0.03]; 81];
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let chain_params = |width, height| {
            let mut p = params(width, height);
            p.film_log_exposure = &knots;
            p.film_density_curves_normalized = &curves;
            p.print_log_exposure = &knots;
            p.print_density_curves = &curves;
            p.film_channel_density = &spectral;
            p.print_channel_density = &spectral;
            p.film_base_density = &base;
            p.print_base_density = &base;
            p.print_illuminant = &light;
            p.scan_illuminant = &light;
            p.print_sensitivity = &sensitivity;
            p.scan_xyz_to_rgb = &identity;
            p.scan_cat = &identity;
            p.scan_base_xyz_to_rgb = &identity;
            p
        };
        let retain = |image: &ImageBuf, domain| {
            gpu.retain_image(
                image,
                &RetainedImageMeta {
                    width: image.width,
                    height: image.height,
                    domain,
                    precision: Precision::host(),
                    space: &colorspace::SRGB,
                    pixel_pitch_um: Some(1.0),
                },
            )
            .unwrap()
        };
        let host = ImageBuf::from_data(
            9,
            7,
            (0..9 * 7 * 3)
                .map(|i| spektrafilm_math::precision::from_f64(-0.75 + f64::from(i % 17) / 64.0))
                .collect(),
        );
        let input = retain(&host, SampleDomain::LogExposure);
        let mut p = chain_params(9, 7);
        p.inject = Tap::LogEFilm;
        p.collect = Tap::CmyFilm;
        let density = gpu
            .run_film_chain_retained(&input, &p)
            .unwrap()
            .to_host(gpu.backend())
            .unwrap();
        let dye_knots = [0.0, 1.0];
        let inverse = [1.0];
        let zero = [0.0; 2];
        for (width, height) in [(9, 7), (11, 9)] {
            let dye = DyeGpuParams {
                monochrome: false,
                law: spektrafilm_backend::dye_params::DyeFieldLaw::Capacity(
                    DyeChannelTablesGpu::pack(std::array::from_fn(|_| DyeChannelParts {
                        min: 0.1,
                        cap_total: 1.0,
                        cap: [1.0, 0.0, 0.0],
                        frac: [1.0, 0.0, 0.0],
                        area: [0.2; 3],
                        uniformity: 0.0,
                        lookup_sign: 1.0,
                        knots: &dye_knots,
                        inv_dx: &inverse,
                        layers: [&dye_knots, &zero, &zero],
                    })),
                ),
                cell_um: 1.0,
                taps: vec![0.25, 0.5, 0.25],
                source_pitch_um: 1.0,
                grid: DyeReadoutGrid {
                    origin_um: [-0.25, 0.375],
                    pitch_um: [0.875, 1.125],
                    width,
                    height,
                },
                seed: 0x1234_5678_9abc_def0,
                channel_correlation: 0.3,
                expectation: false,
            };
            let expected_field = gpu.dye_field(&density, &dye).unwrap();
            let mut downstream = chain_params(width, height);
            downstream.inject = Tap::CmyFilm;
            let field = retain(&expected_field, SampleDomain::Density);
            let expected = gpu
                .run_film_chain_retained(&field, &downstream)
                .unwrap()
                .to_host(gpu.backend())
                .unwrap();
            downstream.grain_dye = Some(dye.clone());
            let resumed = gpu.run_film_chain_retained(&field, &downstream).unwrap();
            assert_eq!(resumed.to_host(gpu.backend()).unwrap().data, expected.data);
            p.grain_dye = Some(dye);
            p.collect = Tap::RgbOut;
            let result = gpu.run_film_chain_retained(&input, &p).unwrap();
            assert_eq!(result.to_host(gpu.backend()).unwrap().data, expected.data);

            p.collect = Tap::CmyFilm;
            let field_result = gpu.run_film_chain_retained(&input, &p).unwrap();
            assert_eq!(field_result.domain(), SampleDomain::Density);
            assert_eq!(
                field_result.to_host(gpu.backend()).unwrap().data,
                expected_field.data
            );
            p.collect = Tap::RgbOut;
            p.grain_dye_crop = Some([1, 1, width - 2, height - 2]);
            let crop = gpu.run_film_chain_retained(&input, &p).unwrap();
            let expected_crop: Vec<_> = (1..height - 1)
                .flat_map(|y| {
                    let first = (y * width + 1) as usize * 3;
                    expected.data[first..first + (width - 2) as usize * 3]
                        .iter()
                        .copied()
                })
                .collect();
            assert_eq!((crop.width(), crop.height()), (width - 2, height - 2));
            assert_eq!(crop.to_host(gpu.backend()).unwrap().data, expected_crop);

            p.grain_dye_crop = None;
            p.grain_dye.as_mut().unwrap().seed += 1;
            let next = gpu.run_film_chain_retained(&input, &p).unwrap();
            assert_ne!(next.to_host(gpu.backend()).unwrap().data, expected.data);
            assert_eq!(result.to_host(gpu.backend()).unwrap().data, expected.data);
            assert_eq!(
                field_result.to_host(gpu.backend()).unwrap().data,
                expected_field.data
            );
            assert_eq!(crop.to_host(gpu.backend()).unwrap().data, expected_crop);
            assert_eq!(input.to_host(gpu.backend()).unwrap().data, host.data);
        }
    }

    #[test]
    fn cancelled_identity_tap_discards_carrier_without_poisoning_reuse() {
        use spektrafilm_backend::ComputeBackend;
        use std::sync::{Arc, atomic::AtomicBool};
        let gpu = crate::test_support::explicit_adapter();
        let image = ImageBuf::from_data(1, 1, f32_to_scalars(vec![0.125, 0.5, 0.75]));
        let source = gpu
            .retain_image(
                &image,
                &RetainedImageMeta {
                    width: 1,
                    height: 1,
                    domain: SampleDomain::Linear,
                    precision: Precision::F32,
                    space: &colorspace::SRGB,
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        let mut p = params(1, 1);
        p.collect = p.inject;
        let mut cancelled = gpu.derived_explicit();
        cancelled.pipe = gpu
            .pipe
            .with_cancellation(Some(Arc::new(AtomicBool::new(true))));
        assert_eq!(
            cancelled.run_film_chain_retained(&source, &p).unwrap_err(),
            "operation cancelled"
        );
        let healthy = gpu.run_film_chain_retained(&source, &p).unwrap();
        assert!(Arc::ptr_eq(
            source.payload().unwrap(),
            healthy.payload().unwrap()
        ));
        assert_eq!(
            scalars_to_f32(&healthy.to_host(gpu.backend()).unwrap().data).as_ref(),
            &[0.125, 0.5, 0.75]
        );
    }

    #[test]
    fn diagnostic_taps_reject_absent_or_reversed_topology_boundaries() {
        let mut p = params(1, 1);
        p.inject = Tap::CmyFilm;
        p.collect = Tap::LogEFilm;
        assert!(validate_tap_span(&p).unwrap_err().contains("no node path"));
        p.scan_film = true;
        p.inject = Tap::LogEPrint;
        p.collect = Tap::LogEPrint;
        assert!(
            validate_tap_span(&p)
                .unwrap_err()
                .contains("not part of this topology")
        );
    }

    #[test]
    fn required_chain_buffers_mirror_the_chain_arithmetic() {
        const W: u32 = 8;
        const H: u32 = 4;
        let log_exp = [0.0f64; 5];
        let tc_lut = spektrafilm_math::spectral::TcLut {
            size: 16,
            channels: 3,
            data: vec![0.0; 16 * 16 * 3],
        };
        let p = FilmChainParams {
            front: FrontPass::Hanatos2025 {
                tc_lut: &tc_lut,
                generated: None,
                rgb_to_adapted_xyz: MAT3,
            },
            film_log_exposure: &log_exp,
            ..params(W, H)
        };
        let required = required_chain_buffers(&p);
        // A non-empty `[[f64; 3]]` table narrows to 12 bytes per row and
        // outgrows the fixed-aux bound, so the table path itself is pinned.
        let density_rows = [[0.0f64; 3]; 400];
        let table_params = FilmChainParams {
            film_channel_density: &density_rows,
            ..params(W, H)
        };
        assert_eq!(
            required_chain_buffers(&table_params).static_bytes,
            400u64 * 12
        );
        // Resource planning follows declared lattice geometry even when a
        // generated retained table has no host sample vector.
        assert_eq!(required.static_bytes, MAX_FIXED_AUX_BYTES);
    }
}
