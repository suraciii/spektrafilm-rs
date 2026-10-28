use super::chain_chunks::PointwisePass;
use super::grain::RasterStorage;
use super::*;

mod prepare;

/// Pre-built output gamut compression over bounded scan-output windows,
/// between glare and unsharp, matching the CPU scanning order. The transport
/// raster is never bound in full; each window shares the retained `C_max`
/// coefficient table and completes before the next photographic stage.
///
/// CPU equivalent: `OutputGamutCompress::compress`.
pub(super) struct GamutState {
    dispatch: PointwisePass,
}

pub(super) fn build_gamut_state(
    device: &ObservedDevice,
    gp: &spektrafilm_backend::GamutGpuParams<'_>,
    buf_b: &(impl RasterStorage + ?Sized),
    n_pixels: u32,
    backend: &WgpuBackend,
) -> Result<GamutState, String> {
    // The JzAzBz mode carries its perceptual transform in binary64, so the
    // pass needs native shader f64 when it is selected. A device without it
    // is a named failure, not a host fallback (spec gpu-execution
    // §Numerical alignment: a capability limitation is not permission for a
    // CPU image-stage fallback).
    if gp.mode == 4 && !device.features().contains(wgpu::Features::SHADER_F64) {
        return Err(
            "output gamut compression in jzazbz mode requires device binary64 shader arithmetic"
                .into(),
        );
    }
    let shader = if device.features().contains(wgpu::Features::SHADER_F64) {
        GAMUT_SHADER_F64
    } else {
        GAMUT_SHADER_F32
    };
    let pipe = backend.cached_pointwise_pipeline(
        shader,
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
        &[(2, "img")],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct GamutParams {
        n_pixels: u32,
        mode: u32,
        n_l: u32,
        n_h: u32,
        knee: [f32; 4],
        lspace: [f32; 4],
        rgb_to_xyz: [[f32; 4]; 3],
        xyz_to_rgb: [[f32; 4]; 3],
        cam16: [f32; 4],
    }
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("gamut_params"),
        contents: bytemuck::bytes_of(&GamutParams {
            n_pixels,
            mode: gp.mode,
            n_l: gp.n_l,
            n_h: gp.n_h,
            knee: [gp.knee[0], gp.knee[1], gp.knee[2], 0.0],
            lspace: [gp.l_min, gp.l_max, 0.0, 0.0],
            // The shader's matrices are `mat3x3`, whose std140 layout is three
            // vec4 columns. Uploading the row-major matrix's columns keeps
            // `m * rgb` the standard matrix-vector product, so the f32
            // transform is the CPU's `mat_vec` with no index transposition
            // step to get wrong.
            rgb_to_xyz: super::pack_matrix_columns(&gp.rgb_to_xyz),
            xyz_to_rgb: super::pack_matrix_columns(&gp.xyz_to_rgb),
            cam16: gp.cam16,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
    });
    let limit = super::bounded::storage_binding_limit(backend)?;
    let cmax_buf = if let Some(prepared) = gp.prepared {
        let table = prepared
            .values
            .as_any()
            .downcast_ref::<prepare::GamutPreparationTable>()
            .ok_or_else(|| "output gamut boundary payload is not a GPU gamut table".to_owned())?;
        if table.backend_id != backend.backend_id {
            return Err("output gamut boundary belongs to a different GPU backend".into());
        }
        let expected = u64::from(gp.n_l) * u64::from(gp.n_h) * 4;
        if table.buffer.size() != expected {
            return Err("output gamut boundary dimensions do not match its retained table".into());
        }
        if expected > limit {
            return Err(format!(
                "output gamut boundary minimum coefficient binding needs {expected} bytes; device binding limit is {limit}"
            ));
        }
        table.buffer.clone()
    } else {
        // The CPU-reference constructor owns host coefficients. ACES RGC
        // needs only a nonempty placeholder binding, never table samples.
        let bytes = (gp.cmax.len().max(1) as u64) * 4;
        if bytes > limit {
            return Err(format!(
                "output gamut boundary minimum coefficient binding needs {bytes} bytes; device binding limit is {limit}"
            ));
        }
        let cmax_f32: Vec<f32> = if gp.cmax.is_empty() {
            vec![0.0]
        } else {
            gp.cmax.iter().map(|&v| v as f32).collect()
        };
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gamut_cmax"),
            contents: bytemuck::cast_slice(&cmax_f32),
            usage: wgpu::BufferUsages::STORAGE,
        })
    };
    let pages = buf_b.pages();
    let dispatch = backend.chain_pointwise(
        pipe,
        &[&params_buf, &cmax_buf, &pages],
        &[(2, super::page_views::Access::ReadWrite)],
        n_pixels,
        false,
        "output_gamut",
    )?;
    Ok(GamutState { dispatch })
}

/// JzAzBz requires explicit binary64 helpers: native f64 transcendentals
/// return wrong values on some devices.
const GAMUT_SHADER_F64: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("jzazbz.wgsl"),
    include_str!("gamut_common.wgsl"),
    include_str!("gamut_compress.wgsl"),
);
const GAMUT_SHADER_F32: &str = concat!(
    include_str!("gamut_common.wgsl"),
    include_str!("gamut_compress_f32.wgsl"),
);

impl GamutState {
    /// Submit each image window separately, retaining device-only samples.
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.dispatch.submit(backend)
    }
}

impl WgpuBackend {
    /// Run the output gamut compressor over `img` and return the compressed
    /// samples. One upload, bounded in-place device dispatches, one readback.
    ///
    /// This is the device twin of `OutputGamutCompress::compress` for callers
    /// that hold the compressor's `GamutGpuParams` rather than an assembled
    /// chain. Production runs the same kernel through the resident chain
    /// ([`build_gamut_state`]); the parameters, shader source, and uniform
    /// layout are shared, so a device/CPU comparison here covers production.
    ///
    /// Returns `Err` when the device cannot serve the requested mode — the
    /// JzAzBz mode needs native shader binary64 — so a caller reports the
    /// named failure instead of reading unprocessed samples.
    pub fn gamut_compress_gpu(
        &self,
        img: &ImageBuf,
        gp: &spektrafilm_backend::GamutGpuParams<'_>,
    ) -> Result<ImageBuf, String> {
        if img.width == 0 || img.height == 0 {
            return Ok(img.clone());
        }
        let n_pixels = img.pixel_count() as u32;
        let input = scalars_to_f32(&img.data);
        let buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gamut_input"),
                contents: bytemuck::cast_slice(input.as_ref()),
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            });
        let state = build_gamut_state(&self.device, gp, &buf, n_pixels, self)?;
        state.submit(self)?;
        let out = self
            .pipe
            .readback_buffer_cancellable(&buf, img.pixel_count() as u64 * 3 * 4)?;
        Ok(ImageBuf::from_data(
            img.width,
            img.height,
            f32_to_scalars(out),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::{from_f64, to_f64};

    // CPU OkLab reference with an identity destination and constant C_max.
    fn reference(rgb: [f64; 3]) -> [f64; 3] {
        fn mv(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
            m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
        }
        let lms = mv(
            [
                [0.8189330101, 0.3618667424, -0.1288597137],
                [0.0329845436, 0.9293118715, 0.0361456387],
                [0.0482003018, 0.2643662691, 0.633851707],
            ],
            rgb,
        )
        .map(f64::cbrt);
        let lab = mv(
            [
                [0.2104542553, 0.793617785, -0.0040720468],
                [1.9779984951, -2.428592205, 0.4505937099],
                [0.0259040371, 0.7827717662, -0.808675766],
            ],
            lms,
        );
        let c = lab[1].hypot(lab[2]);
        let h = lab[2].atan2(lab[1]);
        let d = c / 0.3;
        let compressed = d / (1.0 + d.powf(2.0)).powf(0.5) * 0.3;
        let lab = [lab[0], compressed * h.cos(), compressed * h.sin()];
        let lms = mv(
            [
                [1.0, 0.3963377922, 0.2158037581],
                [1.0, -0.1055613423, -0.0638541748],
                [1.0, -0.0894841821, -1.2914855379],
            ],
            lab,
        )
        .map(|v| v * v * v);
        mv(
            [
                [1.2270138511, -0.5577999807, 0.2812561490],
                [-0.0405801784, 1.1122568696, -0.0716766787],
                [-0.0763812845, -0.4214819784, 1.5861632204],
            ],
            lms,
        )
    }

    #[test]
    fn gamut_oklch_restricted_device_matches_cpu() {
        let gpu = crate::test_support::restricted_adapter();
        assert!(!gpu.device.features().contains(wgpu::Features::SHADER_F64));
        let pixels = [
            [0.4, 0.2, 0.1],
            [1.2, 0.4, 0.05],
            [0.1, 0.7, 1.3],
            [0.2, 0.5, 0.3],
        ];
        let image = ImageBuf::from_data(
            4,
            1,
            pixels.iter().flatten().copied().map(from_f64).collect(),
        );
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let table = [0.3; 4];
        let gp = spektrafilm_backend::GamutGpuParams {
            mode: 1,
            knee: [0.0, 1.0, 2.0],
            l_min: 0.0,
            l_max: 1.0,
            rgb_to_xyz: identity,
            xyz_to_rgb: identity,
            cam16: [0.0; 4],
            cmax: &table,
            prepared: None,
            n_l: 2,
            n_h: 2,
        };
        let out = gpu
            .gamut_compress_gpu(&image, &gp)
            .expect("restricted gamut pass");
        let mut worst = 0.0f64;
        for (i, rgb) in pixels.iter().enumerate() {
            let expected = reference(rgb.map(|v| to_f64(from_f64(v))));
            for (channel, value) in expected.iter().enumerate() {
                worst = worst.max((value - to_f64(out.data[i * 3 + channel])).abs());
            }
        }
        // Measured max error: 1.165e-6; retain over fourfold device margin.
        assert!(worst < 5e-6, "maximum error: {worst}");
    }
}
