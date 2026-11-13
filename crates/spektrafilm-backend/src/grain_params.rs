//! Device-independent grain parameter records.
//!
//! These records cross the [`ComputeBackend`](crate::ComputeBackend) trait boundary.

/// f32 boundary representation shared by the WGPU Grain Procedural shader and the
/// core parameter mapper. The field groups mirror the shader's 64-byte
/// uniform layout.
#[derive(Debug, Clone, Copy)]
pub struct ProceduralGrainGpuParams {
    pub mode: u32,
    /// Host Film Resolution implementation: 0 = OpticalResolution, 1 = FastBlur.
    pub resolution_type: u32,
    pub amount: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub raw_scale: f32,
    pub cluster_size: f32,
    pub rotation: f32,
    pub color: f32,
    pub resolution_factor: f32,
    pub seed: u32,
    /// Explicit host timer; `None` derives the canonical phase from `seed`.
    pub timer: Option<f32>,
    pub colored: bool,
    pub clustered: bool,
}

/// Device Spektra grain on the film density CMY buffer, in the
/// density-fence position (after the film density curve and DIR, before
/// printing).
///
/// The model's `prepare_grain` resolves all density floors, adjusted maxima,
/// and particle counts from the profile curves and RMS controls. No host
/// image-dependent fields are generated. Three-slot coefficient rows retain
/// the profile carrier layout; only `layer_count` slots are active.
///
/// `curve_x` is the composite axis, negated for positive film, and
/// `curve_layers` is the matching fixed-three-slot layer tensor. Device
/// interpolation uses these f64 tables, then grain working arithmetic is f32.
/// The GPU realization is per-pixel PCG32, including the micro-structure
/// clump field. CPU uses the original sequential MT19937 realization.
#[derive(Debug, Clone, PartialEq)]
pub struct SpektraGrainGpuParams {
    /// Request seed shared by the PCG32 streams.
    pub base_seed: u64,
    /// Active layers in the fixed three-slot profile carrier (1..=3).
    pub layer_count: u32,
    /// Per-channel composite density floor.
    pub density_min: [f64; 3],
    /// Per-channel grain uniformity control.
    pub uniformity: [f64; 3],
    /// Per-sublayer density floor, row-major over sublayer.
    pub density_min_layers: [f64; 9],
    /// Per-sublayer density maximum (floor included), row-major over sublayer.
    pub density_max_adj: [f64; 9],
    /// Per-sublayer particle count per pixel, row-major over sublayer.
    pub n_particles: [f64; 9],
    /// Preserve the CPU's shared channel-zero stream for monochrome material.
    pub monochrome: bool,
    /// Sign of both the composite lookup axis and lookup query.
    pub lookup_sign: f32,
    /// Sublayer particle-field blur control, applied before summing layers.
    pub blur_dye_clouds: f64,
    /// Host input representation only; interpolation tables remain f64 and grain works in f32.
    pub reference_f64: bool,
    /// Micro-structure standard deviation in pixels (`micro_structure[1] ·
    /// 0.001 / pixel_size_um`); the clumping field is drawn only above 0.05.
    pub micro_sigma: f64,
    /// Micro-structure blur radius in pixels (`micro_structure[0] /
    /// pixel_size_um`).
    pub micro_blur_px: f64,
    /// Final grain blur σ in pixels.
    pub grain_blur: f64,
    /// Multiplicative unsharp mask σ in pixels.
    pub usm_sigma: f64,
    /// Multiplicative unsharp mask amount.
    pub usm_amount: f64,
    /// Composite curve x column, `knots` rows × 3 channels.
    pub curve_x: Vec<f64>,
    /// Layer curve tensor, `knots` rows × 3 sublayers × 3 channels.
    pub curve_layers: Vec<f64>,
}

/// One Fotufilm Film grain residual request.
///
/// CPU equivalent: `Bank::apply_image` in `spektrafilm_model::grain::fotufilm`.
/// The host resolves every image-independent quantity from the anchored
/// population and the request, and the device performs every image-dependent
/// one: the stochastic coating formation, the pitch statistics, and the
/// residual readout. No host-prepared grain field, population sample, or
/// per-pixel result crosses the boundary, and the host never evaluates the
/// developed density image.
///
/// The formation records come from `Population::device_records`, so the device
/// sees the same fitted sublayer coefficients, count thresholds and developing
/// fractions the reference forms its bank from. The device conservatively
/// resamples the full developed source onto the readout lattice before adding
/// grain. Source pitch is physical microns; the size-compensated bank coordinates
/// below are converted back to physical coordinates for that resample only.
#[derive(Debug, Clone)]
pub struct FotufilmGpuParams {
    /// Formation seed, the source's pinned `SEED` derived exactly as
    /// `light` derives it.
    pub formation_seed: u32,
    /// Per-record formation coefficients, in record order. A record with no
    /// sublayers forms no bank levels, exactly as `build_bank` skips it.
    pub records: Vec<FotufilmRecordGpuParams>,
    /// Whether the population is monochrome: one record's realization is
    /// shared across output channels.
    pub monochrome: bool,
    /// Source pitch in microns of the density image the device reads.
    pub source_pitch_um: f64,
    /// Full-frame output origin and pitch in bank texels, already divided by
    /// clamped `size`. Despite the historical suffix, these are not unscaled
    /// physical microns; one bank texel represents one micron before scaling.
    pub origin_um: [f64; 2],
    pub pitch_um: [f64; 2],
    /// Readout lattice dimensions: the developed source is resampled onto
    /// this lattice before the grain is read out of it.
    pub width: u32,
    pub height: u32,
    /// Pixel dimensions of the developed source the device reads, which the
    /// conservative resample needs as the source's extent. They are the
    /// image's own dimensions and are independent of the readout lattice.
    pub source_width: u32,
    pub source_height: u32,
    /// Readout footprint in bank texels, already divided by clamped `size` and
    /// multiplied by clamped `softness`; used directly by pitch and residual.
    pub footprint_um: [f64; 2],
    /// Residual sampling controls.
    pub seed: u64,
    pub amount: f64,
    pub size: f64,
    pub softness: f64,
    pub color: f64,
    pub layers: [f64; 3],
}

/// One record's device formation coefficients.
///
/// The coating is a list of sublayers, exactly as the reference's
/// `Population::device_records` reports them: `light` walks every sublayer of
/// a record in order and accumulates its own deposits into one lattice, so
/// the device forms each sublayer with its own cells, mark spread, capacity,
/// thresholds and developing fractions.
#[derive(Debug, Clone)]
pub struct FotufilmRecordGpuParams {
    /// Per-sublayer formation coefficients, in sublayer order. A record with
    /// no sublayers forms no bank levels, exactly as `build_bank` skips it.
    pub sublayers: Vec<FotufilmSublayerGpuParams>,
    /// Bank density floor and ceiling of this record.
    pub d_min: f64,
    pub d_max: f64,
}

/// One sublayer's device formation coefficients.
#[derive(Debug, Clone)]
pub struct FotufilmSublayerGpuParams {
    /// Coating grid divisions per axis of one bank formation pass.
    pub cells: u32,
    /// Whether the sublayer develops silver (uncovered-grain) or dye clouds.
    pub silver: bool,
    /// Mark spread in millimetres, before the source's texel scaling.
    pub sigma_mm: f64,
    /// Local dye capacity in density units.
    pub capacity: f64,
    /// Unit-crystal mark scale.
    pub edge: f64,
    /// Developing fraction against normalized density, 129 taps.
    pub forming: Vec<f64>,
    /// Cumulative Poisson count thresholds, 48 values.
    pub thresholds: Vec<f32>,
    /// Largest texel distance from a crystal centre to any texel it can
    /// deposit into, as an upper bound of the law's support.
    pub reach_texels: f64,
    /// The largest crystal occupied area this sublayer can draw, as the
    /// reference's `occupied_area` resolves it. The device uses it to bound
    /// the uncovered square's side.
    pub occupied_area: f64,
}

/// f32 boundary representation shared by the core mapper and the WGPU
/// Exposure Grain shader. Tables are prepared from `seed` and the effective
/// size pair resolved from these controls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposureGrainGpuParams {
    pub strength: f32,
    pub size: f32,
    pub auto_size: bool,
    /// Automatic-size short-edge format constant in millimetres.
    pub format_mm: f32,
    pub absolute_size_px: f32,
    pub roughness: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub push: f32,
    pub color_variation: f32,
    /// Already reduced to the reference's nonnegative 31-bit domain.
    pub seed: u32,
    pub render_scale: f32,
}
