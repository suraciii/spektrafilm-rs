//! Forward evaluation of one candidate parameter vector through the calibrated
//! pipeline.

use super::chart::{self, ChartLayout, PatchTable};
use super::color::{self, PatchDifferences};
use crate::params::RuntimeParams;
use crate::profile::Profile;
use crate::runtime::Runtime;
use spektrafilm_gpu::cpu_backend::CpuBackend;
use spektrafilm_math::image::ImageBuf;
use std::path::PathBuf;

/// Scalar count of the free tone parameters: `positive_density_ev`, then the
/// red and blue balance channels. The green channel is pinned, see
/// [`PINNED_BALANCE_CHANNEL`].
pub(crate) const TONE_PARAMETERS: usize = 3;
/// The balance channel a tone fit pins.
///
/// A common offset added to `positive_density_ev` and subtracted from every
/// `positive_balance_ev` channel leaves the positive scan unchanged, so the
/// four values are redundant by one degree of freedom. Pinning one channel
/// makes the fitted tone unique.
pub(crate) const PINNED_BALANCE_CHANNEL: usize = 1;
/// Largest supported leading surface term count (the polynomial stores 15).
pub(crate) const MAX_SURFACE_TERMS: usize = 14;
pub(crate) const SURFACE_LOWER: f64 = -40.0;
pub(crate) const SURFACE_UPPER: f64 = 40.0;
pub(crate) const DENSITY_LOWER: f64 = -4.0;
pub(crate) const DENSITY_UPPER: f64 = 4.0;
pub(crate) const BALANCE_LOWER: f64 = -2.0;
pub(crate) const BALANCE_UPPER: f64 = 2.0;
pub(crate) const SURFACE_STEP: f64 = 0.05;
pub(crate) const TONE_STEP: f64 = 0.01;
/// The scan output color space of every evaluation: the reference table has
/// sRGB primaries and the patch comparison converts from linear sRGB.
pub(crate) const FIT_OUTPUT_COLOR_SPACE: &str = "sRGB";

/// One forward evaluation: the patch colors, the residual vector, and the two
/// reported medians.
pub(crate) struct Evaluation {
    pub labs: Vec<[f64; 3]>,
    pub residual: Vec<f64>,
    /// Median difference over the scored color patches.
    pub color_de: f64,
    /// Median difference over the scored gray patches.
    pub gray_de: f64,
}

/// The chain data and options of one fit. Cloning it rescores the same chart
/// with a different patch set, which is what cross validation needs.
#[derive(Clone)]
pub(crate) struct Evaluator {
    film: Profile,
    print: Profile,
    params: RuntimeParams,
    data_dir: PathBuf,
    layout: ChartLayout,
    chart: ImageBuf,
    reference_labs: Vec<[f64; 3]>,
    /// The scored patches: the color set followed by the fit-gray set, sorted.
    fit_indices: Vec<usize>,
    /// The scored color patches, in layout order.
    color: Vec<usize>,
    /// The scored gray patches, in layout order.
    fit_gray: Vec<usize>,
    surface: bool,
    tone: bool,
    surface_terms: usize,
    lambda: f64,
    /// The resolved value the tone fit pins in [`PINNED_BALANCE_CHANNEL`].
    tone_pin: f64,
}

impl Evaluator {
    /// Validate the fit inputs and build the chart image once.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        film: Profile,
        print: Profile,
        params: RuntimeParams,
        data_dir: PathBuf,
        layout: ChartLayout,
        scene: &PatchTable,
        reference: &PatchTable,
        surface: bool,
        tone: bool,
        surface_terms: usize,
        lambda: f64,
    ) -> Result<Self, String> {
        layout.validate()?;
        if !(lambda.is_finite() && lambda >= 0.0) {
            return Err(format!(
                "lambda must be a finite value of at least 0, got {lambda}"
            ));
        }
        if surface {
            if !(1..=MAX_SURFACE_TERMS).contains(&surface_terms) {
                return Err(format!(
                    "surface_terms must be between 1 and {MAX_SURFACE_TERMS}, got {surface_terms}"
                ));
            }
            let stored = &film.data.hanatos2025_adaptation_surface_params;
            let width = stored.first().map_or(0, Vec::len);
            if stored.len() < 3 || width < 1 + surface_terms {
                return Err(format!(
                    "film profile {:?} carries no usable hanatos2025_adaptation_surface_params \
                     for {surface_terms} terms: found {} rows of {} values",
                    film.info.name,
                    stored.len(),
                    width
                ));
            }
        }
        if params.settings.preview_mode {
            return Err("fit requires preview mode off (settings.preview_mode = false)".into());
        }
        if params.scanner.scan_output != "positive_scan" {
            return Err(format!(
                "fit requires the finished positive scan (scanner.scan_output = positive_scan), \
                 got {:?}",
                params.scanner.scan_output
            ));
        }
        let count = layout.patch_count();
        if scene.patches.len() != count || reference.patches.len() != count {
            return Err(format!(
                "chart layout expects {count} patches, scene table has {}, reference table has {}",
                scene.patches.len(),
                reference.patches.len()
            ));
        }
        for (name, table) in [("scene", scene), ("reference", reference)] {
            if table.source.trim().is_empty() {
                return Err(format!(
                    "{name} patch table needs a non-empty source naming the instrument or \
                     computation that produced its values"
                ));
            }
            if let Some((index, patch)) = table
                .patches
                .iter()
                .enumerate()
                .find(|(_, patch)| patch.iter().any(|value| !value.is_finite()))
            {
                return Err(format!("{name} patch {index} is not finite: {patch:?}"));
            }
        }
        let chart = chart::assemble(&layout, &scene.patches)?;
        let reference_labs = reference
            .patches
            .iter()
            .map(|patch| color::linear_srgb_to_lab(*patch))
            .collect();
        let color: Vec<usize> = layout.color.indices().collect();
        let fit_gray: Vec<usize> = layout.fit_gray.indices().collect();
        let mut fit_indices: Vec<usize> = color
            .iter()
            .copied()
            .chain(fit_gray.iter().copied())
            .collect();
        fit_indices.sort_unstable();
        let tone_pin = params.scanner.positive_balance_ev[PINNED_BALANCE_CHANNEL];
        Ok(Self {
            film,
            print,
            params,
            data_dir,
            layout,
            chart,
            reference_labs,
            fit_indices,
            color,
            fit_gray,
            surface,
            tone,
            surface_terms,
            lambda,
            tone_pin,
        })
    }

    /// The scored color patches, in layout order.
    pub(crate) fn color_indices(&self) -> &[usize] {
        &self.color
    }

    /// The same chain data, scoring a different color patch set.
    pub(crate) fn scoring_color(&self, color: Vec<usize>) -> Self {
        let mut fit_indices: Vec<usize> = color
            .iter()
            .copied()
            .chain(self.fit_gray.iter().copied())
            .collect();
        fit_indices.sort_unstable();
        Self {
            color,
            fit_indices,
            ..self.clone()
        }
    }

    /// Median CIEDE2000 over `indices` of an evaluated patch set.
    pub(crate) fn delta_median(&self, labs: &[[f64; 3]], indices: &[usize]) -> f64 {
        color::median(
            &indices
                .iter()
                .map(|&index| color::delta_e(labs[index], self.reference_labs[index]))
                .collect::<Vec<_>>(),
        )
    }

    pub(crate) fn dimension(&self) -> usize {
        self.surface_parameters() + if self.tone { TONE_PARAMETERS } else { 0 }
    }

    fn surface_parameters(&self) -> usize {
        if self.surface {
            3 * self.surface_terms
        } else {
            0
        }
    }

    /// Starting vector: a zero surface and the configured tone values.
    pub(crate) fn initial(&self) -> Vec<f64> {
        let mut x = vec![0.0; self.dimension()];
        if self.tone {
            let offset = self.surface_parameters();
            x[offset] = self.params.scanner.positive_density_ev;
            x[offset + 1] = self.params.scanner.positive_balance_ev[0];
            x[offset + 2] = self.params.scanner.positive_balance_ev[2];
        }
        x
    }

    pub(crate) fn lower(&self) -> Vec<f64> {
        let mut bounds = vec![SURFACE_LOWER; self.surface_parameters()];
        if self.tone {
            bounds.push(DENSITY_LOWER);
            bounds.extend([BALANCE_LOWER; TONE_PARAMETERS - 1]);
        }
        bounds
    }

    pub(crate) fn upper(&self) -> Vec<f64> {
        let mut bounds = vec![SURFACE_UPPER; self.surface_parameters()];
        if self.tone {
            bounds.push(DENSITY_UPPER);
            bounds.extend([BALANCE_UPPER; TONE_PARAMETERS - 1]);
        }
        bounds
    }

    pub(crate) fn steps(&self) -> Vec<f64> {
        let mut steps = vec![SURFACE_STEP; self.surface_parameters()];
        if self.tone {
            steps.extend([TONE_STEP; TONE_PARAMETERS]);
        }
        steps
    }

    /// The tone values a candidate vector denotes, including the pinned balance.
    pub(crate) fn tone_values(&self, x: &[f64]) -> (f64, [f64; 3]) {
        let offset = self.surface_parameters();
        let mut balance = [0.0; 3];
        balance[0] = x[offset + 1];
        balance[PINNED_BALANCE_CHANNEL] = self.tone_pin;
        balance[2] = x[offset + 2];
        (x[offset], balance)
    }

    /// The balance channel a tone fit pins, and the value it pins.
    pub(crate) fn tone_pin(&self) -> (usize, f64) {
        (PINNED_BALANCE_CHANNEL, self.tone_pin)
    }

    /// Fit a candidate vector and measure the rendered patches.
    pub(crate) fn evaluate(&self, x: &[f64]) -> Result<Evaluation, String> {
        let mut film = self.film.clone();
        let mut params = self.params.clone();
        let surface_values = &x[..self.surface_parameters()];
        if self.surface {
            film.data.hanatos2025_adaptation_surface_params = self.surface_matrix(surface_values);
            params.settings.apply_hanatos2025_adaptation_surface = true;
        }
        if self.tone {
            let (density, balance) = self.tone_values(x);
            params.scanner.positive_density_ev = density;
            params.scanner.positive_balance_ev = balance;
        }
        params.film_render.halation.active = false;
        params.film_render.grain.active = false;
        params.io.output_cctf_encoding = false;
        params.io.output_color_space = FIT_OUTPUT_COLOR_SPACE.into();

        let runtime = Runtime::new(film, self.print.clone(), params, &self.data_dir)?;
        let rendered = runtime.process(self.chart.clone(), &CpuBackend)?;
        let patches = chart::sample(&self.layout, &rendered)?;
        let labs: Vec<[f64; 3]> = patches
            .iter()
            .map(|patch| color::linear_srgb_to_lab(*patch))
            .collect();

        let mut residual = Vec::with_capacity(self.fit_indices.len() * 3 + surface_values.len());
        for &index in &self.fit_indices {
            for (value, reference) in labs[index].iter().zip(&self.reference_labs[index]) {
                residual.push(value - reference);
            }
        }
        if self.surface {
            let weight = self.lambda.sqrt();
            residual.extend(surface_values.iter().map(|value| weight * value));
        }
        let color_de = self.delta_median(&labs, &self.color);
        let gray_de = self.delta_median(&labs, &self.fit_gray);
        Ok(Evaluation {
            labs,
            residual,
            color_de,
            gray_de,
        })
    }

    /// The full 3x15 surface matrix a profile stores, from fitted terms.
    pub(crate) fn surface_matrix(&self, values: &[f64]) -> Vec<Vec<f64>> {
        let mut matrix = vec![vec![0.0; 15]; 3];
        for layer in 0..3 {
            for term in 0..self.surface_terms {
                matrix[layer][1 + term] = values[layer * self.surface_terms + term];
            }
        }
        matrix
    }

    pub(crate) fn differences(&self, labs: &[[f64; 3]]) -> PatchDifferences {
        color::differences(labs, &self.reference_labs)
    }
}
