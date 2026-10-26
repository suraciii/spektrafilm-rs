//! Conservative additional buffer capacities for the fixed photographic stages.
//! Chain-owned rasters and shared Gaussian image pages are counted by passes.rs.
use super::super::{
    WgpuBackend, bounded::storage_binding_limit, page_views::ADAPTER_WORKSPACE_BYTES,
};
use spektrafilm_backend::FilmChainParams;

fn add(a: u64, b: u64) -> Result<u64, String> {
    a.checked_add(b)
        .ok_or_else(|| "GPU stage resource plan overflows".into())
}
fn mul(a: u64, b: u64) -> Result<u64, String> {
    a.checked_mul(b)
        .ok_or_else(|| "GPU stage resource plan overflows".into())
}

impl WgpuBackend {
    pub(super) fn chain_effect_workspace_bytes(
        &self,
        name: &str,
        p: &FilmChainParams<'_>,
        width: u32,
        height: u32,
    ) -> Result<u64, String> {
        let pixels = mul(u64::from(width), u64::from(height))?;
        let image = mul(pixels, 12)?;
        let limit = storage_binding_limit(self)?;
        let window = image.min(limit / 12 * 12);
        if pixels == 0 || limit < 12 {
            return Err("GPU stage requires a nonempty RGB working set".into());
        }
        let gaussian =
            super::super::grain::v1::blur::bounded::workspace_bytes(width, height, limit)?;
        let pointwise = ADAPTER_WORKSPACE_BYTES;
        // At most sixteen static bindings in these kernels. The static preflight
        // supplies the largest active coefficient/uniform allocation, not a sum.
        let coefficients = mul(super::required_chain_buffers(p).static_bytes, 16)?;
        let base = add(pointwise, coefficients)?;
        let extra = match name {
            "dir_couplers" => {
                let dp = p.dir_couplers.as_ref().ok_or("DIR parameters missing")?;
                let diffuse = dp.diffusion_size_px > 0.0 || dp.diffusion_tail_px > 0.0;
                // Curve pages overlap one row. Even two-row pages fit twice the
                // original tables; they are constructed after spatial fields drop.
                let curves = mul(
                    add(
                        mul(dp.log_exposure.len() as u64, 4)?,
                        mul(dp.density_curves_0.len() as u64, 12)?,
                    )?,
                    2,
                )?;
                add(
                    mul(image, if diffuse { 3 } else { 1 })?,
                    add(gaussian, curves)?,
                )?
            }
            "camera_lens_blur" | "scanner_lens_blur" | "unsharp_mask" => gaussian,
            "halation" => {
                let hp = p.halation.as_ref().ok_or("halation parameters missing")?;
                let compressed = hp.source_compression > 0.0
                    && hp.halation_n_bounces > 0
                    && hp.halation_a_tot.iter().any(|&a| a > 0.0);
                let bounce = hp.halation_n_bounces >= 1
                    && (0..3)
                        .any(|c| hp.halation_a_tot[c] > 0.0 && hp.halation_first_sigma_px[c] > 0.0);
                let jobs = add(
                    if hp.scatter_amount > 0.0 { 4 } else { 0 },
                    if bounce {
                        u64::from(hp.halation_n_bounces)
                    } else {
                        0
                    },
                )?;
                // Each retained filter has bounded line buffers. Pointwise plans
                // are retained with their filter jobs, including alias adapters.
                let pointwise_jobs = add(jobs, 6)?;
                add(
                    mul(image, 1 + u64::from(compressed))?,
                    add(mul(jobs, gaussian)?, mul(pointwise_jobs, pointwise)?)?,
                )?
            }
            "glare" => add(
                image,
                if p.glare.as_ref().is_some_and(|g| g.blur_px > 0.0) {
                    gaussian
                } else {
                    0
                },
            )?,
            "highlight_boost" => {
                // Maximum reduction retains only one bounded scalar input/tree;
                // chunk uniforms/groups are constructed during execution.
                add(65_536 * 4 + 33 * 4, 128 * 3)?
            }
            "scanner_density" => {
                // Prepared exposure uses three binary64 samples per pixel. The
                // RGB adapter and frame-setup reductions remain separate owners.
                add(mul(image, 2)?, add(window, add(image, gaussian)?)?)?
            }
            "camera_diffusion" | "enlarger_diffusion" => {
                let print = name == "enlarger_diffusion";
                let params = if print {
                    p.enlarger_diffusion.as_ref()
                } else {
                    p.camera_diffusion.as_ref()
                }
                .ok_or("optical diffusion parameters missing")?;
                self.diffusion_workspace_bytes(width, height, params, print)?
            }
            "enlarger_lut" | "scanner_lut" => {
                // Seven brick buffers, plus up to two submitted bricks. Each is
                // bounded independently of hardware binding size by its owner.
                mul(super::super::resources::WORKSPACE_BINDING_BYTES / 8, 21)?
            }
            "dye_print_response_log" | "dye_aperture_collapse" => image,
            "grain_spektra_v1" => {
                let gp = p
                    .grain_v1
                    .as_ref()
                    .ok_or("Spektra grain parameters missing")?;
                let window_limit = super::super::grain::v1::transport_window_limit(limit);
                let window = image.min(window_limit / 12 * 12);
                let layers = if gp.sublayers_active {
                    3
                } else {
                    gp.n_sub_layers
                };
                if layers == 0 {
                    return Err("Spektra grain requires a nonzero layer count".into());
                }
                let layers = u64::from(layers);
                let layer_image = mul(image, layers)?;
                let chunk = (window_limit / mul(12, layers)?).min(pixels);
                if chunk == 0 {
                    return Err("Spektra layer window cannot hold one pixel".into());
                }
                let mut bytes = add(layer_image, image)?;
                bytes = add(bytes, mul(mul(chunk, 12)?, add(layers, 2)?)?)?;
                if gp.sublayers_active && gp.blur_dye_clouds > 0.0 {
                    bytes = add(bytes, add(layer_image, mul(add(image, gaussian)?, 3)?)?)?;
                }
                if gp.sublayers_active && gp.micro_sigma > 0.05 && gp.micro_blur_px > 0.4 {
                    bytes = add(bytes, add(mul(image, 2)?, gaussian)?)?;
                }
                // Finishing retains its windows, filtered/sharpened fields,
                // filter intermediates and full-source mass-reduction pyramid.
                bytes = add(bytes, mul(window, 4)?)?;
                let blur = gp.grain_blur > if gp.sublayers_active { 0.0 } else { 0.4 };
                if blur {
                    bytes = add(bytes, add(mul(image, 2)?, gaussian)?)?;
                }
                if gp.sublayers_active && gp.usm_sigma > 0.0 && gp.usm_amount > 0.0 {
                    bytes = add(bytes, add(mul(image, 3)?, gaussian)?)?;
                    bytes = add(bytes, mul(pixels.div_ceil(8192), 256 * 48 * 2)?)?;
                    bytes = add(bytes, mul((window / 12 / 32).max(1), 48)?)?;
                }
                // Every retained chunk uniform is below 2 KiB; include all
                // formation/compose/finish/reduction command partitions.
                bytes = add(
                    bytes,
                    mul(
                        add(pixels.div_ceil(chunk), pixels.div_ceil(8192))?,
                        16 * 2048,
                    )?,
                )?;
                if gp.mt19937_regime {
                    let planes = mul(layers, 3)?;
                    bytes = add(bytes, mul(planes, add(mul(pixels, 8)?, 2512 + 1024)?)?)?;
                    let mt_chunk = (limit / 12)
                        .min(u64::from(super::super::grain::mt19937::CHUNK_ELEMENTS))
                        .max(1);
                    bytes = add(bytes, mul(mt_chunk, 32)?)?;
                }
                bytes
            }
            "grain_v2" => {
                // The procedural owner validates its resolution support. Four
                // full rasters and eight capped sample/filter windows coexist.
                // Its retained command metadata is budgeted by the actual
                // bounded-resolution planner before construction.
                self.procedural_workspace_bytes(
                    width,
                    height,
                    p.grain_v2
                        .as_ref()
                        .ok_or("procedural grain parameters missing")?,
                )?
            }
            "input_decode"
            | "convert_inverse_solver"
            | "front_transform"
            | "log_exposure"
            | "film_density"
            | "print_lut_roundtrip"
            | "print_spectral"
            | "print_density"
            | "scan_spectral"
            | "output_gamut"
            | "output_transfer"
            | "cmyk" => 0,
            _ => return Err(format!("unplanned GPU stage resource owner: {name}")),
        };
        add(base, extra)
    }
}
