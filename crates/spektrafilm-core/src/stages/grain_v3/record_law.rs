//! Opt-in scalar-record resolution shared by CPU and GPU execution.
use super::*;
use crate::telemetry::{RecordConfiguration, RecordLaw, RecordOpticalOrder};

pub(super) const Q_ENV: &str = "SPEKTRAFILM_GRAIN_RECORD_Q";

pub(super) fn contribution() -> Result<Option<f64>, String> {
    positive_env(Q_ENV)
}

fn positive_env(name: &str) -> Result<Option<f64>, String> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| {
            if name == Q_ENV {
                format!("{Q_ENV} must be a finite positive density-area contribution in D*um^2")
            } else {
                format!("{name} must be finite and positive")
            }
        })?;
    Ok(Some(value))
}

pub(super) fn resolve(
    film: &Profile,
    params: &RuntimeParams,
) -> Result<Option<RecordConfiguration>, String> {
    let Some(q) = contribution()? else {
        return Ok(None);
    };
    if !active(params)
        || !film.is_film()
        || !film.is_filming()
        || !film.is_bw()
        || !(film.is_negative() || film.is_positive())
        || params.workflow.route_enum() != WorkflowRoute::FilmScan
        || !params.io.scan_film
        || params.scanner.scan_output != "direct_scan"
    {
        return Err(format!(
            "{Q_ENV} requires active Dye, a monochrome filming stock, and input > film > scan / direct_scan"
        ));
    }
    if std::env::var_os(APERTURE_FACTOR_ENV).is_some()
        || std::env::var_os("SPEKTRAFILM_GRAIN_V3_MASS_LAW").is_some()
    {
        return Err(format!(
            "{Q_ENV} cannot be combined with aperture-factor or legacy mass-law experiments"
        ));
    }
    // This existing validator owns exact-cell LUT, route, and spatial-optics limits.
    validate_cell_spectral(params)?;
    let support_um = params.film_render.grain.v3_dye_support_um as f64;
    if !support_um.is_finite() || support_um <= 0.0 {
        return Err(format!(
            "{Q_ENV} requires finite positive Dye support in um"
        ));
    }
    let cell_um = positive_env(CELL_UM_ENV)?.unwrap_or(support_um / 2.0);
    if cell_um > support_um / 2.0 {
        return Err(format!("{CELL_UM_ENV} must not exceed Dye support / 2"));
    }
    let expectation = match std::env::var_os(EXPECTATION_ENV) {
        None => false,
        Some(value) if value == "0" => false,
        Some(value) if value == "1" => true,
        Some(_) => return Err(format!("{EXPECTATION_ENV} must be exactly 0 or 1 when set")),
    };
    Ok(Some(RecordConfiguration {
        law: RecordLaw::ScalarFixedContribution,
        event_contribution_d_um2: q,
        support_um,
        cell_um,
        seed: params.random_seed,
        expectation,
        optical_order: RecordOpticalOrder::FormedDensityThenCellResponseThenFractionalIntegration,
    }))
}

fn field_params(resolved: RecordConfiguration) -> v3::RecordFieldParams {
    v3::RecordFieldParams {
        event_contribution_d_um2: resolved.event_contribution_d_um2,
        support_um: resolved.support_um,
        cell_um: resolved.cell_um,
        seed: resolved.seed,
        expectation: resolved.expectation,
    }
}

pub(super) fn device_params(
    resolved: RecordConfiguration,
    pitch: f64,
    grid: ReadoutGrid,
) -> Result<spektrafilm_backend::dye_params::DyeGpuParams, String> {
    let field = field_params(resolved);
    Ok(spektrafilm_backend::dye_params::DyeGpuParams {
        law: spektrafilm_backend::dye_params::DyeFieldLaw::ScalarRecord {
            event_contribution_d_um2: field.event_contribution_d_um2,
        },
        cell_um: field.cell_um,
        taps: field.formation_taps()?,
        source_pitch_um: pitch,
        grid: spektrafilm_backend::dye_params::DyeReadoutGrid {
            origin_um: grid.origin_um,
            pitch_um: grid.pitch_um,
            width: grid.width,
            height: grid.height,
        },
        seed: field.seed,
        channel_correlation: 0.0,
        expectation: field.expectation,
        monochrome: true,
    })
}

pub(super) fn readout(
    source: &ImageBuf,
    resolved: RecordConfiguration,
    pitch: f64,
    grid: ReadoutGrid,
    backend: &dyn spektrafilm_backend::ComputeBackend,
    device_response: &spektrafilm_backend::DyeResponseGpuParams<'_>,
    response: impl Fn(&ImageBuf, &dyn spektrafilm_backend::ComputeBackend) -> Result<ImageBuf, String>
    + Sync,
) -> Result<ImageBuf, String> {
    if backend.is_gpu() {
        let device = device_params(resolved, pitch, grid)?;
        let observation = super::super::StageObservation::new(backend, "grain_v3_field");
        return observation
            .backend(backend)
            .dye_field_with_response(source, &device, device_response)
            .ok_or_else(|| "selected GPU has no scalar-record field implementation".to_owned())?;
    }
    let field = field_params(resolved);
    let observation = super::super::StageObservation::cpu(
        backend,
        "grain_v3_field",
        spektrafilm_backend::telemetry::CpuReason::GrainV3FieldCpu,
    );
    let backend = observation.backend(backend);
    // Developed BW lanes contain image-forming density above deterministic base.
    // No legacy random floor is added. Negative targets are rejected by the law.
    v3::apply_record_field_with_response(source, pitch, grid, &field, |cells| {
        response(cells, backend)
    })
}
