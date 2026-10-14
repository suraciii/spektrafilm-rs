use super::super::Pipeline;

#[derive(Debug, Clone, Copy)]
pub(super) enum ResidentFallbackReason {
    WorkflowRoute,
    /// Active Dye resolves its film-coordinate field on the CPU; the
    /// resident chain has no equivalent readout.
    FilmGrainV3FieldCpu,
    FilmGrainFotufilmCpu,
    UnsupportedOutputGamut,
    BlurRadiusExceedsBackendSupport,
    MissingResidentFrontPass,
    MallettExecutionParity,
}

#[derive(Debug)]
pub(super) enum ResidentDecision {
    UseResident,
    PerStage {
        reasons: Vec<ResidentFallbackReason>,
    },
}

impl ResidentDecision {
    fn reasons(reasons: Vec<ResidentFallbackReason>) -> Self {
        if reasons.is_empty() {
            Self::UseResident
        } else {
            Self::PerStage { reasons }
        }
    }
}

impl Pipeline {
    pub(super) fn resident_decision(&self) -> ResidentDecision {
        let mut reasons = Vec::new();
        // The Mallett shader has no numerically equivalent route through
        // CPU display-domain grain and optics; retain the per-stage path.
        if self.params.settings.rgb_to_raw_method == "mallett2019" {
            reasons.push(ResidentFallbackReason::MallettExecutionParity);
        }
        if !matches!(
            self.params.workflow.route.as_str(),
            "input > film > scan" | "input > film > print > scan"
        ) {
            reasons.push(ResidentFallbackReason::WorkflowRoute);
        }
        // Negative-to-positive interpretation no longer declines at all:
        // the capture methods (separable/coupled) run as an assembled CPU
        // fence on the scanner readout, and the density reading and Digital
        // Reference replace the acquisition as the assembled acquisition
        // fence (pipe architecture, capability class — see
        // `scan_tail_cpu_stage` and `scan_acquisition_cpu_stage` in
        // `try_gpu_resident`).
        // A modeled scanner receiver (LabScanApprox) no longer declines
        // either: the receiver capture replaces the GPU spectral
        // integration as the same assembled acquisition fence.
        // Input transfer decoding no longer declines: the resident route
        // decodes the input before upload (pipe architecture, capability
        // class — see `try_gpu_resident`).
        // The scanner and enlarger LUT acquisitions no longer decline: they
        // run as assembled CPU fences that replace the GPU spectral
        // integration (see `scan_acquisition_cpu_stage` and
        // `print_acquisition_cpu_stage` in `try_gpu_resident`).
        // Optical diffusion no longer declines: the enlarger filter runs in
        // the print acquisition fence and the camera filter runs as the
        // chain-head fence on the raw exposure buffer (see
        // `camera_diffusion_cpu_stage` in `try_gpu_resident`).
        // Spektra grain no longer declines: it runs on the host as an
        // assembled fence on the film density buffer (pipe architecture,
        // capability class — see `density_cpu_stage` in `try_gpu_resident`).
        // An active Dye field is resolved on the CPU from the full developed
        // film density. The identity-geometry case runs as an assembled fence
        // on the film density buffer; the readout geometries the pipe cannot
        // express and the env-gated certification readouts keep the
        // whole-route CPU fallback (see `resident_fence_supported` in
        // `stages::grain_v3`). This decision is the capability authority for
        // every resident entry point.
        if crate::stages::grain_v3::active(&self.params)
            && !crate::stages::grain_v3::resident_fence_supported(&self.film, &self.params)
        {
            reasons.push(ResidentFallbackReason::FilmGrainV3FieldCpu);
        }
        if crate::stages::grain_fotufilm::active(&self.params) {
            reasons.push(ResidentFallbackReason::FilmGrainFotufilmCpu);
        }
        if self.output_gamut.is_active() && self.output_gamut.gpu_params().is_none() {
            reasons.push(ResidentFallbackReason::UnsupportedOutputGamut);
        }
        if self.tc_lut.is_none() && self.mallett_core.is_none() {
            reasons.push(ResidentFallbackReason::MissingResidentFrontPass);
        }
        ResidentDecision::reasons(reasons)
    }
}
