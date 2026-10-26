//! Device-independent Dye parameter records.
//!
//! These records cross the crate boundary in the `ComputeBackend::dye_field`
//! signature and in `FilmChainParams::grain_dye`, so they live with the trait
//! rather than with the device passes that consume them.

/// One channel's device-side tables.
///
/// The layout matches `Table` in `dye_tables.wgsl`, where the three `vec4`
/// members force sixteen-byte alignment, so the padding is explicit rather
/// than implied: a `#[repr(C)]` struct of four-byte members would otherwise
/// put `cap` eight bytes earlier than the shader reads it. The final
/// four-byte slot carries `lookup_sign`, the sign the bracket applies to its
/// query, so the device reads the same sign the model derived.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DyeChannelTableGpu {
    pub min: f32,
    pub cap_total: f32,
    pub pad_a: [u32; 2],
    pub cap: [f32; 4],
    pub frac: [f32; 4],
    pub area: [f32; 4],
    pub uniformity: f32,
    pub knot_count: u32,
    pub offset: u32,
    pub lookup_sign: f32,
}

/// All three channels: the fixed tables plus the packed knot and curve data.
///
/// `curves` holds each channel's block in the shader's order: `[knots K]
/// [inv_dx K-1][layer 0 K][layer 1 K][layer 2 K]`. `K` comes from the film
/// profile's curve resolution, so the offsets are data rather than a compiled
/// maximum.
#[derive(Clone, Debug, Default)]
pub struct DyeChannelTablesGpu {
    pub tables: [DyeChannelTableGpu; 3],
    pub curves: Vec<f32>,
}

/// One channel's tables as plain slices.
///
/// The device side has no model dependency, so the owner of the model hands
/// its tables over in this shape and this crate decides where they land in the
/// curve buffer.
#[derive(Clone, Debug)]
pub struct DyeChannelParts<'a> {
    pub min: f64,
    pub cap_total: f64,
    pub cap: [f64; 3],
    pub frac: [f64; 3],
    pub area: [f64; 3],
    pub uniformity: f64,
    /// Sign the bracket applies to its query (`-1` for positive films).
    pub lookup_sign: f64,
    pub knots: &'a [f64],
    pub inv_dx: &'a [f64],
    pub layers: [&'a [f64]; 3],
}

impl DyeChannelTablesGpu {
    /// Pack three channels into the device layout.
    ///
    /// The curve block layout `[knots K][inv_dx K-1][layer 0 K][layer 1 K]
    /// [layer 2 K]` and the resulting offsets are this crate's rule, so callers
    /// hand over values and not addresses. `inv_dx` carries one spacing per
    /// bracket and a bracket is chosen with `low <= K-2`, so its block is one
    /// shorter than the knot block. Every channel must come from one model,
    /// whose tables are already length-checked.
    pub fn pack(channels: [DyeChannelParts<'_>; 3]) -> Self {
        let size = channels.iter().map(|c| c.knots.len() * 5 - 1).sum();
        let mut curves = Vec::with_capacity(size);
        let tables = std::array::from_fn(|c| {
            let channel = &channels[c];
            debug_assert_eq!(channel.inv_dx.len() + 1, channel.knots.len());
            debug_assert!(channel.knots.len() >= 2);
            debug_assert!(
                channel
                    .layers
                    .iter()
                    .all(|l| l.len() == channel.knots.len())
            );
            let offset = curves.len() as u32;
            curves.extend(channel.knots.iter().map(|&v| v as f32));
            curves.extend(channel.inv_dx.iter().map(|&v| v as f32));
            for layer in channel.layers {
                curves.extend(layer.iter().map(|&v| v as f32));
            }
            DyeChannelTableGpu {
                min: channel.min as f32,
                cap_total: channel.cap_total as f32,
                pad_a: [0; 2],
                cap: [
                    channel.cap[0] as f32,
                    channel.cap[1] as f32,
                    channel.cap[2] as f32,
                    0.0,
                ],
                frac: [
                    channel.frac[0] as f32,
                    channel.frac[1] as f32,
                    channel.frac[2] as f32,
                    0.0,
                ],
                area: [
                    channel.area[0] as f32,
                    channel.area[1] as f32,
                    channel.area[2] as f32,
                    0.0,
                ],
                uniformity: channel.uniformity as f32,
                knot_count: channel.knots.len() as u32,
                offset,
                lookup_sign: channel.lookup_sign as f32,
            }
        });
        Self { tables, curves }
    }
}

#[derive(Clone, Debug)]
pub struct DyeReadoutGrid {
    pub origin_um: [f64; 2],
    pub pitch_um: [f64; 2],
    pub width: u32,
    pub height: u32,
}

/// Resolved field law. Scalar records carry no legacy capacity tables.
#[derive(Clone, Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "One law per render; retain the existing inline capacity tables without another heap allocation"
)]
pub enum DyeFieldLaw {
    Capacity(DyeChannelTablesGpu),
    ScalarRecord { event_contribution_d_um2: f64 },
}

/// Packed representation of the model owner's resolved field parameters.
#[derive(Clone, Debug)]
pub struct DyeGpuParams {
    pub law: DyeFieldLaw,
    pub cell_um: f64,
    pub taps: Vec<f64>,
    pub source_pitch_um: f64,
    pub grid: DyeReadoutGrid,
    pub seed: u64,
    pub channel_correlation: f32,
    pub expectation: bool,
    /// Profile-derived single-record layout, carried in three equal lanes.
    pub monochrome: bool,
}
