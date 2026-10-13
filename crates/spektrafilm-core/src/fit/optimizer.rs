//! Bounded Levenberg-Marquardt least squares with a finite-difference Jacobian.
//!
//! The solver owns no pipeline knowledge: it minimizes the sum of squares of a
//! residual vector under box bounds.

use nalgebra::{DMatrix, DVector};
use rayon::prelude::*;

/// The residual function a solve minimizes.
pub(crate) type Objective<'a> = dyn Fn(&[f64]) -> Result<Vec<f64>, String> + Sync + 'a;

pub(crate) struct Problem<'a> {
    pub initial: Vec<f64>,
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
    /// Finite-difference step of every parameter; its sign is chosen inward at a bound.
    pub steps: Vec<f64>,
    pub max_iters: usize,
    pub objective: &'a Objective<'a>,
}

#[derive(Debug)]
pub(crate) struct Solution {
    pub x: Vec<f64>,
    pub iterations: usize,
    pub evaluations: usize,
    pub cost: f64,
}

fn sum_squares(values: &[f64]) -> f64 {
    values.iter().map(|value| value * value).sum()
}

fn clamped(point: &[f64], lower: &[f64], upper: &[f64]) -> Vec<f64> {
    point
        .iter()
        .zip(lower.iter().zip(upper))
        .map(|(value, (low, high))| value.clamp(*low, *high))
        .collect()
}

/// Minimize `objective` from `initial` under the box bounds.
pub(crate) fn solve(problem: &Problem<'_>) -> Result<Solution, String> {
    let n = problem.initial.len();
    if n == 0 {
        return Err("fit has no free parameters".into());
    }
    if problem.max_iters == 0 {
        return Err("fit requires at least one iteration".into());
    }
    let mut x = clamped(&problem.initial, &problem.lower, &problem.upper);
    let mut residuals = (problem.objective)(&x)?;
    let mut evaluations = 1usize;
    let mut cost = sum_squares(&residuals);
    let mut mu = 1e-3f64;
    let mut iterations = 0usize;

    while iterations < problem.max_iters {
        let columns: Result<Vec<Vec<f64>>, String> = (0..n)
            .into_par_iter()
            .map(|i| {
                let step = problem.steps[i].abs().max(f64::MIN_POSITIVE);
                let delta = if x[i] + step <= problem.upper[i] {
                    step
                } else {
                    -step
                };
                let mut point = x.clone();
                point[i] = (x[i] + delta).clamp(problem.lower[i], problem.upper[i]);
                let moved = point[i] - x[i];
                if moved == 0.0 {
                    return Ok(vec![0.0; residuals.len()]);
                }
                let shifted = (problem.objective)(&point)?;
                Ok(shifted
                    .iter()
                    .zip(&residuals)
                    .map(|(value, base)| (value - base) / moved)
                    .collect())
            })
            .collect();
        let columns = columns?;
        evaluations += n;

        let jacobian = DMatrix::from_columns(
            &columns
                .iter()
                .map(|column| DVector::from_column_slice(column))
                .collect::<Vec<_>>(),
        );
        let residual_vector = DVector::from_column_slice(&residuals);
        let gradient = jacobian.transpose() * &residual_vector;
        let normal = jacobian.transpose() * &jacobian;
        let scale: Vec<f64> = (0..n).map(|i| normal[(i, i)].abs().max(1e-12)).collect();

        let mut accepted = false;
        for _ in 0..8 {
            let damped = &normal + DMatrix::from_diagonal(&DVector::from_column_slice(&scale)) * mu;
            let Some(cholesky) = damped.cholesky() else {
                mu = (mu * 10.0).min(1e12);
                continue;
            };
            let step = cholesky.solve(&(-&gradient));
            let candidate = clamped(
                &x.iter()
                    .zip(step.iter())
                    .map(|(value, step)| value + step)
                    .collect::<Vec<_>>(),
                &problem.lower,
                &problem.upper,
            );
            let movement = sum_squares(
                &candidate
                    .iter()
                    .zip(&x)
                    .map(|(value, base)| value - base)
                    .collect::<Vec<_>>(),
            )
            .sqrt();
            if movement <= 1e-12 * (1.0 + x.iter().map(|value| value * value).sum::<f64>().sqrt()) {
                return Ok(Solution {
                    x,
                    iterations,
                    evaluations,
                    cost,
                });
            }
            let candidate_residuals = (problem.objective)(&candidate)?;
            evaluations += 1;
            let candidate_cost = sum_squares(&candidate_residuals);
            if candidate_cost < cost {
                x = candidate;
                residuals = candidate_residuals;
                cost = candidate_cost;
                mu = (mu * 0.3).max(1e-12);
                accepted = true;
                break;
            }
            mu = (mu * 10.0).min(1e12);
        }
        if !accepted {
            break;
        }
        iterations += 1;
    }

    Ok(Solution {
        x,
        iterations,
        evaluations,
        cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_a_linear_system_within_budget() {
        let objective = |x: &[f64]| Ok(vec![x[0] - 2.0, x[1] + 1.0, 3.0 * x[2]]);
        let solution = solve(&Problem {
            initial: vec![0.0; 3],
            lower: vec![-10.0; 3],
            upper: vec![10.0; 3],
            steps: vec![0.05; 3],
            max_iters: 15,
            objective: &objective,
        })
        .expect("solves");
        assert!((solution.x[0] - 2.0).abs() < 1e-9, "{:?}", solution.x);
        assert!((solution.x[1] + 1.0).abs() < 1e-9, "{:?}", solution.x);
        assert!(solution.x[2].abs() < 1e-9, "{:?}", solution.x);
        assert!(solution.iterations <= 15);
    }

    #[test]
    fn converges_on_a_curved_objective() {
        let objective = |x: &[f64]| Ok(vec![x[0] * x[0] - 4.0, x[1] - x[0]]);
        let solution = solve(&Problem {
            initial: vec![3.0, 0.0],
            lower: vec![-10.0; 2],
            upper: vec![10.0; 2],
            steps: vec![0.05; 2],
            max_iters: 15,
            objective: &objective,
        })
        .expect("solves");
        assert!((solution.x[0] - 2.0).abs() < 1e-6, "{:?}", solution.x);
        assert!((solution.x[1] - 2.0).abs() < 1e-6, "{:?}", solution.x);
    }

    #[test]
    fn respects_bounds_when_the_minimum_lies_outside() {
        let objective = |x: &[f64]| Ok(vec![x[0] - 5.0]);
        let solution = solve(&Problem {
            initial: vec![0.0],
            lower: vec![-1.0],
            upper: vec![1.0],
            steps: vec![0.05],
            max_iters: 15,
            objective: &objective,
        })
        .expect("solves");
        assert!((solution.x[0] - 1.0).abs() < 1e-9, "{:?}", solution.x);
    }

    #[test]
    fn rejects_a_zero_iteration_cap() {
        let objective = |x: &[f64]| Ok(vec![x[0] - 2.0]);
        let error = solve(&Problem {
            initial: vec![0.0],
            lower: vec![-10.0],
            upper: vec![10.0],
            steps: vec![0.05],
            max_iters: 0,
            objective: &objective,
        })
        .expect_err("zero iterations are rejected");
        assert!(error.contains("at least one iteration"), "{error}");
    }

    #[test]
    fn counts_only_accepted_steps() {
        // The solver stops after the cap, even though the objective is not minimized yet.
        let objective = |x: &[f64]| Ok(vec![x[0] - 1.0]);
        let solution = solve(&Problem {
            initial: vec![0.0],
            lower: vec![-10.0],
            upper: vec![10.0],
            steps: vec![0.05],
            max_iters: 2,
            objective: &objective,
        })
        .expect("solves");
        assert_eq!(solution.iterations, 2, "{:?}", solution);
        assert!(
            solution.x[0] > 0.999 && solution.x[0] < 1.0,
            "{:?}",
            solution
        );
    }

    #[test]
    fn reports_objective_failures() {
        let objective = |_: &[f64]| Err("pipeline rejected the candidate".to_string());
        let error = solve(&Problem {
            initial: vec![0.0],
            lower: vec![-1.0],
            upper: vec![1.0],
            steps: vec![0.05],
            max_iters: 3,
            objective: &objective,
        })
        .expect_err("failure propagates");
        assert!(error.contains("pipeline rejected"));
    }
}
