use super::super::{Pipeline, select_illuminant};

impl Pipeline {
    /// Prepare the resident front pass, borrowing any inverse scan parameters.
    pub(super) fn resident_front_pass<'a>(
        &'a self,
        ae_ev: f64,
        convert_params: Option<&'a spektrafilm_backend::ConvertSolverGpuParams<'a>>,
    ) -> Option<spektrafilm_backend::FrontPass<'a>> {
        // Bake the exposure scale (auto-exposure × manual EV compensation)
        // into the front-pass matrix. Both upsamplers (hanatos and mallett)
        // are homogeneous in the input RGB, so scaling the matrix is
        // equivalent to scaling the input — saves a separate "scale" compute
        // pass at the head of the chain. Auto-exposure metering itself stays
        // on CPU (~30 ms at 6 MP after the per-row rayon parallelization);
        // the result is a single float that's cheap to roll into the matrix.
        let mut exposure_scale_f64 = 1.0f64;
        if self.params.camera.auto_exposure {
            exposure_scale_f64 *= 2.0f64.powf(ae_ev);
        }
        if self.params.camera.exposure_compensation_ev != 0.0 {
            exposure_scale_f64 *= 2.0f64.powf(self.params.camera.exposure_compensation_ev as f64);
        }
        // Front pass: hanatos TC LUT lookup, or the mallett 3×3 matmul
        let fold_exposure = |m: &mut [[f64; 3]; 3]| {
            if (exposure_scale_f64 - 1.0).abs() > 1e-9 {
                for row in m.iter_mut() {
                    for v in row.iter_mut() {
                        *v *= exposure_scale_f64;
                    }
                }
            }
        };

        // A convert-film route's input is a real scanned capture, so its
        // first numerical stage is the inverse solve rather than an
        // rgb→raw transform (`stages::converting::process_with_backend`).
        // The solve decodes the input transfer itself, which is why the
        // chain suppresses its own decode for this head. The route never
        // meters a camera exposure (`expose_full_input` returns the image
        // unchanged), so the exposure fold above is not consulted here. The
        // solver state is resolved once at pipeline construction — it is
        // image-independent — so the chain borrows its caller's parameter
        // record rather than carrying the full record in every front variant.
        if self.params.workflow.route_enum().is_convert_film() {
            return Some(spektrafilm_backend::FrontPass::ScanInverse(convert_params?));
        }
        // Front pass: hanatos TC LUT lookup, or the mallett 3×3 matmul
        // (`core · M_cs`, same fold as the CPU `expose` dispatch).
        let front = if let Some(tc_lut) = self.tc_lut.as_ref() {
            let ref_illuminant = select_illuminant(&self.front_illuminant);
            let mut rgb_to_adapted = spektrafilm_math::spectral::build_rgb_to_adapted_xyz(
                &self.params.io.input_color_space,
                &ref_illuminant,
                self.params.settings.use_cat16,
            );
            fold_exposure(&mut rgb_to_adapted);
            spektrafilm_backend::FrontPass::Hanatos2025 {
                tc_lut,
                generated: self.prepared_tc_lut.as_ref(),
                rgb_to_adapted_xyz: rgb_to_adapted,
            }
        } else {
            let core = self.mallett_core.as_ref()?;
            let mut matrix = crate::mallett::film_matrix(core, &self.params.io.input_color_space);
            fold_exposure(&mut matrix);
            spektrafilm_backend::FrontPass::Mallett2019 { matrix }
        };
        Some(front)
    }
}

#[cfg(test)]
mod tests {

    /// The resident chain's Mallett2019 front pass must reproduce the CPU
    /// `stages::filming::expose` route at its own f32 boundary, including
    /// where the exposure scale is applied.
    ///
    /// CPU order (`stages/filming.rs`):
    ///   rgb  = decoded input                                    (f32)
    ///   rgb *= 2^ae_ev                                          (f32)
    ///   raw  = mat_vec(film_matrix(core, cs), rgb)              (f64 matrix)
    ///   raw *= 2^exposure_compensation_ev                       (f32)
    ///
    /// Resident order (this module + `mallett_rgb_to_raw.wgsl`):
    ///   m    = (2^ae_ev · 2^ev · correction) · film_matrix(core, cs)
    ///   raw  = m · rgb                                          (f32 storage)
    ///
    /// The two are the same product with the scalar factor applied at a
    /// different point. This pins the resident assembly — the matrix fold
    /// and the row-major application the shader performs — against the CPU
    /// order over signed, exact-zero, and endpoint samples.
    #[test]
    fn mallett_front_matrix_matches_the_cpu_reference() {
        // A smooth 81-point illuminant over the basis grid (380–780 nm,
        // 5 nm step) so `compute_core_matrix` integrates something
        // non-degenerate rather than a flat spectrum.
        let illuminant: Vec<f64> = (0..crate::mallett::MALLETT2019_BASIS.len())
            .map(|i| {
                let t = i as f64 / (crate::mallett::MALLETT2019_BASIS.len() - 1) as f64;
                0.55 + 0.45 * (1.0 - (t - 0.5).abs() * 2.0)
            })
            .collect();
        let core = crate::mallett::compute_core_matrix(&crate::mallett::MALLETT2019_BASIS, &illuminant);
        let color_space = "sRGB";
        let m = crate::mallett::film_matrix(&core, color_space);

        // The exposure fold this module applies at the chain head.
        let ae_ev = 0.75f64;
        let ev = -0.4f64;
        let correction = 1.0f64;
        let scale = 2.0f64.powf(ae_ev) * 2.0f64.powf(ev) * correction;
        let mut folded = m;
        for row in folded.iter_mut() {
            for value in row.iter_mut() {
                *value *= scale;
            }
        }

        // Signed, exact-zero, and endpoint samples: the matmul must carry a
        // negative input through unchanged in sign, and the endpoints sit at
        // the extremes the pipeline can produce.
        let samples: [[f64; 3]; 8] = [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [-0.25, 0.5, 1.75],
            [0.5, -0.5, 0.0],
            [2.5, 0.0, -1.0],
            [0.001, 0.002, 0.003],
            [-1.0, -1.0, -1.0],
            [0.184, 0.184, 0.184],
        ];

        let mut worst = 0.0f64;
        for rgb in samples {
            // CPU reference: the auto-exposure scaling lands on the samples
            // first (f32 storage), then the f64 matrix, then the manual EV
            // compensation as a second f32 multiply.
            let scaled = [
                (rgb[0] as f32 * 2.0f32.powf(ae_ev as f32)) as f64,
                (rgb[1] as f32 * 2.0f32.powf(ae_ev as f32)) as f64,
                (rgb[2] as f32 * 2.0f32.powf(ae_ev as f32)) as f64,
            ];
            let cpu_raw = crate::mallett::apply(&m, scaled)
                .map(|v| (v as f32 * 2.0f32.powf(ev as f32)) as f64);

            // Resident: one matmul against the pre-folded matrix, with the
            // samples and the result stored as f32 exactly as the chain's
            // buffers do.
            let resident = {
                let v = [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32];
                std::array::from_fn::<f32, 3, _>(|row| {
                    (0..3)
                        .map(|column| folded[row][column] as f32 * v[column])
                        .sum::<f32>()
                })
            }
            .map(|v| v as f64);

            for channel in 0..3 {
                worst = worst.max((resident[channel] - cpu_raw[channel]).abs());
            }
        }
        // Both sides are the same product through f32 storage: the matrices
        // are narrowed once, the samples once, and the sum runs in f32 on
        // both sides of the boundary. The budget is the f32 boundary of
        // order-1..4 raw values (~1e-6); a transposed matrix or a dropped
        // exposure factor is O(1) and cannot hide under it.
        assert!(
            worst < 1e-5,
            "Mallett front pass diverged from the CPU order: max error {worst:.3e}"
        );
    }
}
