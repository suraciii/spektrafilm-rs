//! Finite, conservative attribution for compute passes. Labels are diagnostic
//! metadata only; unknown work stays `Other` rather than acquiring a guessed kind.

use spektrafilm_backend::telemetry::{GpuPassKind, GpuStage};

pub(super) fn classify_pass(hint: GpuStage, label: Option<&str>) -> (GpuStage, GpuPassKind) {
    use GpuPassKind as K;
    use GpuStage as S;

    let label = label.unwrap_or("");
    let (inferred_stage, kind) = match label {
        "gaussian_gather" => (S::Other, K::Gather),
        "meter_vertical_gather" | "meter_horizontal_gather" => (S::Metering, K::Gather),
        "gaussian_scatter" => (S::Other, K::Scatter),
        "gaussian_bounded_line" | "grain_blur_fir" | "grain_blur_iir" | "grain_blur_box" => {
            (S::Other, K::Filter)
        }
        "gaussian_raw_transport" | "grain_blur_copy" => (S::Other, K::Copy),
        "fotufilm_form" => (S::FotufilmFormation, K::Formation),
        "fotufilm_form_reduce" => (S::FotufilmFormation, K::Reduction),
        "fotufilm_transmittance"
        | "fotufilm_cloud_accumulate"
        | "fotufilm_cloud_prepare_kernel" => (S::FotufilmFormation, K::Pointwise),
        "fotufilm_cloud_tent" | "fotufilm_cloud_filter" | "fotufilm_cloud_upsample" => {
            (S::FotufilmFormation, K::Filter)
        }
        "fotufilm_downsample" => (S::FotufilmStatistics, K::Filter),
        "fotufilm_running_sums" => (S::FotufilmStatistics, K::PrefixSum),
        "fotufilm_pitch" => (S::FotufilmStatistics, K::Gather),
        "fotufilm_correlate" => (S::FotufilmStatistics, K::Reduction),
        "fotufilm_resample_gather" => (S::FotufilmReadout, K::Gather),
        "fotufilm_resample_finalize" | "fotufilm_residual" | "fotufilm_compose" => {
            (S::FotufilmReadout, K::Readout)
        }
        "dye_law_readout" | "dye_readout" | "dye_response_readout" => (S::DyeReadout, K::Readout),
        "dye_law_blur" | "dye_law_highpass" | "dye_form_h" | "dye_form_v" => {
            (S::DyeFormation, K::Filter)
        }
        "dye_law_combine" | "dye_law_subtract" | "dye_law_apply" | "dye_remap" => {
            (S::DyeFormation, K::Pointwise)
        }
        "spectral_prepare_reference" | "spectral_prepare_contract" => {
            (S::SpectralPrepare, K::Spectral)
        }
        "spectral_prepare_gather_neutral" => (S::SpectralPrepare, K::Gather),
        "spectral_prepare_normalize" | "spectral_prepare_finish_neutral" => {
            (S::SpectralPrepare, K::Reduction)
        }
        "gamut_prepare"
        | "input_gamut_reach"
        | "input_gamut_hull"
        | "input_gamut_cmax"
        | "input_gamut_coordinates"
        | "input_gamut_oklch_coordinates"
        | "input_gamut_interpolate" => (S::GamutPrepare, K::Pointwise),
        "input_gamut_fft" | "input_gamut_frequency_filter" => (S::GamutPrepare, K::Filter),
        "procedural_input_transform" | "procedural_compose" | "procedural_canvas" => {
            (S::GrainProcedural, K::Pointwise)
        }
        "procedural_mean" | "procedural_reduce" => (S::GrainProcedural, K::Reduction),
        "procedural_resolution_gather" => (S::GrainProcedural, K::Gather),
        "procedural_resolution_filter" => (S::GrainProcedural, K::Filter),
        "spektra_seed_formation" | "spektra_particle_formation" | "spektra_clump_formation" => {
            (S::GrainSpektra, K::Formation)
        }
        "spektra_compose" => (S::GrainSpektra, K::Pointwise),
        "input_gamut_gather_cmax" | "input_gamut_gather_rgb" | "input_gamut_neutral_gather" => {
            (S::GamutPrepare, K::Gather)
        }
        "input_gamut_hull_scale" | "input_gamut_neutral_reduce" => (S::GamutPrepare, K::Reduction),
        "meter_vertical_accumulate" | "meter_horizontal_accumulate" => (S::Metering, K::Filter),
        "meter_reduce" => (S::Metering, K::Reduction),
        "meter_luminance" => (S::Metering, K::Pointwise),
        "output_samples" => (S::Saving, K::Pointwise),
        "front_transform"
        | "print_spectral"
        | "scan_device"
        | "spectral_lut_interpolate"
        | "enlarger_lut_interpolate" => (S::Other, K::Spectral),
        "input_decode" => (S::Input, K::Pointwise),
        "output_gamut" => (S::Gamut, K::Pointwise),
        "output_transfer" | "cmyk" | "finishing" => (S::Output, K::Pointwise),
        "geometry_gaussian_line" | "geometry_spline_line" => (S::Geometry, K::Filter),
        "optical_diffusion" => (S::Other, K::Filter),
        "highlight_merge" | "diffusion_print_domain" => (S::Other, K::Pointwise),
        "film_density"
        | "print_density"
        | "log_exposure"
        | "dir_pointwise"
        | "halation_scatter_init"
        | "halation_scatter_tail_acc"
        | "halation_scatter_finish"
        | "halation_source_response"
        | "halation_final_add"
        | "halation_renormalize"
        | "glare_apply"
        | "unsharp_combine"
        | "retained_scale"
        | "retained_mix"
        | "retained_transfer"
        | "retained_cmyk"
        | "retained_matrix" => (S::Other, K::Pointwise),
        "geometry_triangle" | "geometry_spline_sample" | "lut_gather" => (S::Other, K::Gather),
        "geometry_range" | "highlight_reduce" => (S::Other, K::Reduction),
        _ => (S::Other, K::Other),
    };
    (
        if hint == S::Other {
            inferred_stage
        } else {
            hint
        },
        kind,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_explicit_stage_and_reports_unknown_work() {
        assert_eq!(
            classify_pass(GpuStage::Halation, Some("gaussian_gather")),
            (GpuStage::Halation, GpuPassKind::Gather)
        );
        assert_eq!(
            classify_pass(GpuStage::Other, Some("unrecognized")),
            (GpuStage::Other, GpuPassKind::Other)
        );
        assert_eq!(
            classify_pass(GpuStage::Scan, None),
            (GpuStage::Scan, GpuPassKind::Other)
        );
        assert_eq!(
            classify_pass(GpuStage::Scan, Some("output_samples")),
            (GpuStage::Scan, GpuPassKind::Pointwise)
        );
    }

    #[test]
    fn distinguishes_mixed_grain_and_gaussian_work() {
        for (label, expected) in [
            (
                "fotufilm_form",
                (GpuStage::FotufilmFormation, GpuPassKind::Formation),
            ),
            (
                "fotufilm_running_sums",
                (GpuStage::FotufilmStatistics, GpuPassKind::PrefixSum),
            ),
            (
                "fotufilm_residual",
                (GpuStage::FotufilmReadout, GpuPassKind::Readout),
            ),
            (
                "dye_law_blur",
                (GpuStage::DyeFormation, GpuPassKind::Filter),
            ),
            (
                "dye_law_readout",
                (GpuStage::DyeReadout, GpuPassKind::Readout),
            ),
            ("gaussian_gather", (GpuStage::Other, GpuPassKind::Gather)),
            ("gaussian_scatter", (GpuStage::Other, GpuPassKind::Scatter)),
            (
                "gaussian_bounded_line",
                (GpuStage::Other, GpuPassKind::Filter),
            ),
        ] {
            assert_eq!(classify_pass(GpuStage::Other, Some(label)), expected);
        }
    }
}
