//! Render-time array access and B&W development-time resolution.

use super::Profile;

impl Profile {
    /// Get density curves as [N][3] f64 array for calibration precision.
    pub fn density_curves_f64(&self) -> Vec<[f64; 3]> {
        self.data
            .density_curves
            .iter()
            .map(|row| {
                [
                    row.get(0).copied().unwrap_or(0.0),
                    row.get(1).copied().unwrap_or(0.0),
                    row.get(2).copied().unwrap_or(0.0),
                ]
            })
            .collect()
    }

    /// Get log_exposure as f64 slice.
    pub fn log_exposure_f64(&self) -> Vec<f64> {
        self.data.log_exposure.clone()
    }

    /// Get density curves as [N][3] f32 array for fast interpolation.
    pub fn density_curves_f32(&self) -> Vec<[f32; 3]> {
        self.data
            .density_curves
            .iter()
            .map(|row| {
                [
                    row.get(0).copied().unwrap_or(0.0) as f32,
                    row.get(1).copied().unwrap_or(0.0) as f32,
                    row.get(2).copied().unwrap_or(0.0) as f32,
                ]
            })
            .collect()
    }

    /// Get log_exposure as f32 slice.
    pub fn log_exposure_f32(&self) -> Vec<f32> {
        self.data.log_exposure.iter().map(|&v| v as f32).collect()
    }

    /// Get log_sensitivity as [81][3] f32 array.
    pub fn log_sensitivity_f32(&self) -> Vec<[f32; 3]> {
        self.data
            .log_sensitivity
            .iter()
            .map(|row| {
                [
                    row.get(0).copied().unwrap_or(0.0) as f32,
                    row.get(1).copied().unwrap_or(0.0) as f32,
                    row.get(2).copied().unwrap_or(0.0) as f32,
                ]
            })
            .collect()
    }

    /// Get log_sensitivity as [81][3] f64 array (precision-preserving).
    pub fn log_sensitivity_f64(&self) -> Vec<[f64; 3]> {
        self.data
            .log_sensitivity
            .iter()
            .map(|row| {
                [
                    row.get(0).copied().unwrap_or(0.0),
                    row.get(1).copied().unwrap_or(0.0),
                    row.get(2).copied().unwrap_or(0.0),
                ]
            })
            .collect()
    }

    /// Per-sublayer density curves in the Python layout
    /// `density_curves_layers[k, sublayer, channel]`, as
    /// `[exposure][3 sublayers][3 channels]` f64 (the layered-grain path
    /// reads the raw f64 profile values — `np.nanmax` and the interpolation
    /// both run at full precision upstream). Empty when the profile carries
    /// no layer curves; missing entries read as 0.0 like the deserializer.
    pub fn density_curves_layers_f64(&self) -> Vec<[[f64; 3]; 3]> {
        self.data
            .density_curves_layers
            .iter()
            .map(|row| {
                let mut out = [[0.0f64; 3]; 3];
                for (sl, layer) in row.iter().enumerate() {
                    if sl >= 3 {
                        break;
                    }
                    for (ch, v) in layer.iter().enumerate() {
                        if ch >= 3 {
                            break;
                        }
                        out[sl][ch] = *v;
                    }
                }
                out
            })
            .collect()
    }
}

/// Index into a development-time family: nearest entry to `requested` by
/// absolute difference, or the floor-middle entry when unset. Mirrors
/// Python `select_development_time` (`None` → `(N-1)//2`, else nearest).
/// Public so the GUI's picker resolves the selection identically.
pub fn development_time_index(times: &[f64], requested: Option<f64>) -> usize {
    if times.len() <= 1 {
        return 0;
    }
    match requested {
        Some(t) => times
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (*a - t)
                    .abs()
                    .partial_cmp(&(*b - t).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .unwrap_or(0),
        None => (times.len() - 1) / 2,
    }
}

/// Extract one column of the per-wavelength base-density rows (clamping the
/// index for single-column colour profiles).
pub(super) fn base_density_column(rows: &[Vec<f64>], idx: usize) -> Vec<f64> {
    rows.iter()
        .map(|row| {
            let i = idx.min(row.len().saturating_sub(1));
            row.get(i).copied().unwrap_or(f64::NAN)
        })
        .collect()
}

/// Resolve a profile for rendering: collapse a B&W development-time family
/// to the requested time and broadcast the single channel onto the engine's
/// 3-channel layout. Colour profiles pass through untouched.
///
/// Mirrors upstream's `select_development_time` (nearest entry; default the
/// floor-middle of the family) followed by its `n_channels == 1` semantics:
/// the engine runs the same per-channel math on identical replicated
/// channels, while `channel_density` becomes `[dye, 0, 0]` so every spectral
/// integration computes exactly the upstream single-channel
/// `density · dye_spectrum` (the G/B lanes carry no spectral weight).
pub fn resolve_for_render(mut profile: Profile, development_time: Option<f64>) -> Profile {
    if !profile.is_bw() {
        return profile;
    }
    let idx = development_time_index(&profile.data.development_time, development_time);
    let d = &mut profile.data;

    // 1. Collapse the development-time family (columns of density_curves,
    //    base_density, and rows of the curves model) to the chosen entry.
    for row in &mut d.density_curves {
        let i = idx.min(row.len().saturating_sub(1));
        *row = vec![row.get(i).copied().unwrap_or(0.0)];
    }
    d.base_density = base_density_column(&d.base_density_rows, idx);
    d.base_density_rows = d.base_density.iter().map(|&v| vec![v]).collect();
    if let Some(model) = &mut d.density_curves_model {
        let pick = |m: &[Vec<f64>]| -> Vec<Vec<f64>> {
            m.get(idx.min(m.len().saturating_sub(1)))
                .cloned()
                .map(|row| vec![row])
                .unwrap_or_default()
        };
        model.centers = pick(&model.centers);
        model.amplitudes = pick(&model.amplitudes);
        model.sigmas = pick(&model.sigmas);
        if let Some(alphas) = model.alphas.clone() {
            model.alphas = Some(pick(&alphas));
        }
    }
    // Layers are n_le × n_layers × n_times on B&W families (upstream slices
    // `[:, :, idx]`). Collapse the family, then broadcast the single
    // channel onto the engine's 3-channel layout (same convention as
    // `density_curves` below) so the layered-grain path — which indexes
    // `density_curves_layers[k, sublayer, channel]` — reads replicated
    // columns, mirroring how the composite curves are broadcast.
    for layer_row in &mut d.density_curves_layers {
        for layer in layer_row.iter_mut() {
            let i = idx.min(layer.len().saturating_sub(1));
            let v = layer.get(i).copied().unwrap_or(0.0);
            *layer = vec![v, v, v];
        }
    }
    if !d.development_time.is_empty() {
        let i = idx.min(d.development_time.len() - 1);
        d.development_time = vec![d.development_time[i]];
    }

    // 2. Broadcast the single channel to the 3-channel engine layout.
    for row in &mut d.log_sensitivity {
        let s = row.first().copied().unwrap_or(0.0);
        *row = vec![s, s, s];
    }
    for row in &mut d.density_curves {
        let v = row.first().copied().unwrap_or(0.0);
        *row = vec![v, v, v];
    }
    for row in &mut d.channel_density {
        let c = row.first().copied().unwrap_or(f64::NAN);
        *row = vec![c, 0.0, 0.0];
    }
    if let Some(model) = &mut d.density_curves_model {
        let bcast = |m: &mut Vec<Vec<f64>>| {
            if let Some(row) = m.first().cloned() {
                *m = vec![row.clone(), row.clone(), row];
            }
        };
        bcast(&mut model.centers);
        bcast(&mut model.amplitudes);
        bcast(&mut model.sigmas);
        if let Some(alphas) = &mut model.alphas {
            bcast(alphas);
        }
    }
    profile
}
