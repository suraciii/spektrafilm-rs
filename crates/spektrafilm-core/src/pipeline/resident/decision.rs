use super::super::Pipeline;

#[derive(Debug, Clone, Copy)]
pub(super) enum ResidentFallbackReason {
    WorkflowRoute,
    /// Active Dye needs a film-coordinate readout the resident chain's
    /// fixed-size density buffer cannot express (a crop or upscale, or the
    /// env-gated certification readouts); the field resolves on the
    /// per-stage route instead.
    FilmGrainV3FieldCpu,
    UnsupportedOutputGamut,
    BlurRadiusExceedsBackendSupport,
    MissingResidentFrontPass,
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
        if !matches!(
            self.params.workflow.route.as_str(),
            "input > film > scan" | "input > film > print > scan"
        ) && !self.params.workflow.route_enum().is_convert_film() {
            reasons.push(ResidentFallbackReason::WorkflowRoute);
        }
        // Negative-to-positive interpretation no longer declines at all:
        // the density reading replaces the acquisition as the assembled
        // acquisition fence (pipe architecture, capability class — see
        // `scan_acquisition_cpu_stage` in `try_gpu_resident`).
        // Input transfer decoding no longer declines: the resident route
        // decodes the input before upload (pipe architecture, capability
        // class — see `try_gpu_resident`).
        // The enlarger LUT acquisition no longer declines: it is a device
        // pass. The scanner LUT and modeled-receiver acquisitions still run
        // as assembled CPU fences that replace the GPU spectral integration
        // (see `scan_acquisition_cpu_stage` in `try_gpu_resident`).
        // Optical diffusion no longer declines: the camera and enlarger
        // filters are device passes, so neither needs a fence.
        // Spektra grain no longer declines: it runs on the host as an
        // assembled fence on the film density buffer (pipe architecture,
        // capability class — see `density_cpu_stage` in `try_gpu_resident`).
        // An active Dye field resolves its film-coordinate readout from the
        // full developed film density. A request the chain can host — a plain
        // crop or upscale included — runs inside the resident chain: the
        // device pass, or the assembled density fence for a request the device
        // cannot realize (see `resident_device_params_for_dims` in
        // `stages::grain_v3`). Readouts that need a deferred crop tail or the
        // aperture fine-print lattice, and the env-gated certification
        // readouts, keep the whole-route fallback to the per-stage readout
        // (`resident_fence_supported`, backed by `readout_plan`). This
        // decision is the capability authority for every resident entry point.
        if crate::stages::grain_v3::active(&self.params)
            && !crate::stages::grain_v3::resident_fence_supported(&self.film, &self.params)
        {
            reasons.push(ResidentFallbackReason::FilmGrainV3FieldCpu);
        }
        if self.output_gamut.is_active() && self.output_gamut.gpu_params().is_none() {
            reasons.push(ResidentFallbackReason::UnsupportedOutputGamut);
        }
        if self.tc_lut.is_none() && self.mallett_core.is_none() && self.convert_converter.is_none() {
            reasons.push(ResidentFallbackReason::MissingResidentFrontPass);
        }
        ResidentDecision::reasons(reasons)
    }

}
