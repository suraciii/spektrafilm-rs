//! Dye grain: the device side of the Poisson count decision.
//!
//! `specs/film-grain/v3/design.md#device-count-law` owns this contract. The
//! device runs the reference's count law -- Knuth below 30, Hörmann's PTRS
//! above, never a normal approximation -- in single precision and from its own
//! coordinate-addressed words. The law itself lives in `dye_count.wgsl`, which
//! every count-drawing kernel concatenates ahead of its entry point: this
//! module's decision kernel and the production G4 shader run the same source.
//! Dye's appearance is a function of the realized count statistics rather than
//! of the reference's individual draws, so the device is not asked to reproduce
//! those draws, and the CPU reference is a reference for *statistics* instead
//! of for words.
//!
//! Acceptance is therefore statistical rather than sample-for-sample: the
//! realized counts must be Poisson in mean, variance and shape, across both
//! branches and across the branch switch, and a control build that replaces the
//! law must fail the same checks. The bands are declared here before any run,
//! from the law's own moments and the sample count, at one fixed screening
//! level ([`SCREENING_SIGMA`]); the five replicates additionally run pooled --
//! 1.5M cases per rate -- through the same bands at their own sample size, so
//! a systematic bias that hides inside every per-replicate band still fails.
//! Skewness bands are asymptotic; this is not an exact family-wise
//! significance test.
//!
//! The rejection test's log-probability is evaluated in a rearranged form of
//! the same factorial equation (`ptrs_rhs` in `dye_count.wgsl`): the factorial
//! form cancels two five-digit f32 words against a six-digit one at the top of
//! the rate range, and that erratic error is wider than the rejection boundary
//! itself. Two GPU tests cover the equation directly: a placed probe that
//! compares the device's trial states, verdicts and rejection values at frozen
//! boundaries against an independent f64 factorial oracle, and a control law
//! that keeps the replaced equation and must be exposed at [`DYE_MAX_RATE`]:
//! by the pooled counts wherever the adapter's arithmetic rounds the cancelled
//! arrangement's threshold error into them, by its word consumption otherwise
//! (see `the_original_cancellation_is_exposed_at_the_top_rate`).
//!
//! The supported rate range is a measured claim, not an integer-domain one:
//! [`DYE_MAX_RATE`] is where the f32 realization of this law still passes those
//! bands. Rates above it fail their case rather than returning a count with
//! silently different statistics, and [`WgpuBackend::certify_dye_counts`] runs
//! the same law on the actual device before any Dye field is prepared.

use super::*;

/// One decision to place: a rate, a stream address, and the seed it mixes in.
///
/// The layout mirrors `dye_decisions.wgsl`'s `Case`; the tag and the cell are
/// carried rather than derived so the stream-tag convention stays in one place
/// per side.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DyeDecisionCase {
    lam: f32,
    seed_lo: u32,
    seed_hi: u32,
    tag: u32,
    i: i32,
    j: i32,
    pad0: u32,
    pad1: u32,
}

/// The sentinel the count law writes when a case cannot be realized exactly: a
/// rate outside the supported range, a proposal outside the exact f32 integer
/// range, or a word budget exhausted after `MAX_DRAWS` words. A truncated loop
/// is a wrong count, not a slow one.
///
/// Must match `DECISION_FAILED` in `dye_count.wgsl`.
pub const DYE_DECISION_FAILED: u32 = 0xFFFF_FFFF;

/// The largest rate the device count law realizes: the top of the supported
/// range, not of an integer domain. Must match `MAX_RATE` in `dye_count.wgsl`.
///
/// A conservative envelope selected by an offline f32 PTRS probe: at 300k
/// cases, five independent samples at 65536 held their per-replicate bands;
/// 131072 showed a systematic variance bias of about +2%, and 196608 about
/// -3%. Those probes predate the rejection-equation repair; stabilizing that
/// equation does not extend the envelope, which stays frozen at the certified
/// top and must clear the actual adapter's five-replicate pooled screen before
/// any field is prepared. The exact-integer domain alone does not establish
/// count-law accuracy.
pub const DYE_MAX_RATE: f64 = 65536.0;

/// The decision kernel: the shared count law plus the decision entry. G4's
/// production shader concatenates the same `dye_count.wgsl` ahead of its own
/// tables and accumulation entry, so a fix to the law lands in both at once.
const DECISION_SHADER: &str = concat!(
    include_str!("dye_count.wgsl"),
    include_str!("dye_decisions.wgsl")
);

/// The acceptance control: the shared words and the same entry, with the count
/// law replaced. Only tests compile it.
#[cfg(test)]
const CONTROL_SHADER: &str = concat!(
    include_str!("dye_count.wgsl"),
    include_str!("dye_decisions_control.wgsl")
);

/// The rejection-equation control: the shared words and trial body, the same
/// count law, and the pre-repair rejection equation in place of `ptrs_rhs`.
/// At [`DYE_MAX_RATE`] it must be exposed by the pooled counts or by its word
/// consumption, whichever the adapter's arithmetic permits; the control test
/// owns that requirement. Only tests compile it.
#[cfg(test)]
const CANCELLED_SHADER: &str = concat!(
    include_str!("dye_count.wgsl"),
    include_str!("dye_count_control_cancelled.wgsl"),
    include_str!("dye_decisions_control_cancelled.wgsl")
);

/// The placed numerical-rejection probe: the shared words and trial body, the
/// production rejection equation, and the pre-repair one beside it, over
/// placed `(lam, u, v)` trials. Only tests compile it.
#[cfg(test)]
const REJECTION_SHADER: &str = concat!(
    include_str!("dye_count.wgsl"),
    include_str!("dye_count_control_cancelled.wgsl"),
    include_str!("dye_decisions_rejection.wgsl")
);

/// One placed trial for [`REJECTION_SHADER`]: the rate, the pair's raw uniform
/// (`u01`; the law's centred `u` is `u01 - 0.5`) and the pair's second
/// uniform. Must match `dye_decisions_rejection.wgsl`'s `Place`.
#[cfg(test)]
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct DyeRejectionPlace {
    lam: f32,
    u01: f32,
    v: f32,
    pad: u32,
}

impl DyeDecisionCase {
    /// A case for the stream `(seed, tag)` of the cell `(i, j)` at rate
    /// `lambda`.
    ///
    /// The rate is narrowed to f32 here because f32 is the device's arithmetic:
    /// narrowing it in the shader would only hide where it happens. A rate the
    /// device does not support -- negative, NaN, above [`DYE_MAX_RATE`] -- is
    /// still a representable case; the law reports it as
    /// [`DYE_DECISION_FAILED`] instead of returning a count.
    pub fn new(lambda: f64, seed: u64, tag: u64, cell: (i32, i32)) -> Self {
        Self {
            lam: lambda as f32,
            seed_lo: seed as u32,
            seed_hi: (seed >> 32) as u32,
            tag: tag as u32,
            i: cell.0,
            j: cell.1,
            pad0: 0,
            pad1: 0,
        }
    }

    /// The rate this case realizes.
    pub fn lambda(&self) -> f32 {
        self.lam
    }
}

/// One violated property of a realized count sample, with the rate it was
/// checked at, so a failing control names what it broke instead of just
/// failing.
struct Violation {
    rate: f64,
    kind: &'static str,
    detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rate {}: {}: {}", self.rate, self.kind, self.detail)
    }
}

#[cfg(test)]
fn render(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n  ")
}

/// One conservative screening width for every rate and statistic, frozen
/// before device acceptance. Known-good CPU sampling calibrates false alarms;
/// the altered-law control independently proves detection power on both
/// branches. Correlated checks and heterogeneous rates do not support a pooled
/// binomial confidence claim or an exact family-wise significance level.
const SCREENING_SIGMA: f64 = 7.0;

/// The statistical checks of one rate's realized counts against the exact
/// law's predeclared bands. Shared by the acceptance tests, the known-good
/// CPU generator control, and [`WgpuBackend::certify_dye_counts`], so the
/// bands mean one thing everywhere.
///
/// Every band is [`SCREENING_SIGMA`] standard errors of the checking
/// statistic under the exact Poisson law at this rate and sample count --
/// derived from the law's own moments, declared here before any run rather
/// than fitted to one:
///
/// * mean: `SE = sqrt(lambda / n)`, exact for any count law with this
///   variance.
/// * variance: exact for the unbiased sample variance `s^2 = n*m2/(n-1)`:
///   `Var(s^2) = lambda/n + 2 lambda^2/(n - 1)` (Poisson kurtosis).
/// * skewness: `b1 = m3 / m2^(3/2)` about the sample mean, delta-method
///   asymptotic `SE = sqrt((6 + 9/(2 lambda) + 1/(4 lambda^2)) / n)`. The
///   closed form comes from the influence function `Y^3 - 3Y - gamma1 -
///   (3/2) gamma1 (Y^2 - 1)` with `Y = (x - mu)/sqrt(lambda)`; the full
///   central-moment algebra and a second independent derivation are the source
///   comment above, and the calibration of this exact estimator is reproduced
///   by `skew_band_calibration.py`, retained with the Dye GPU acceptance
///   evidence (it reports the measured replicate spread against these closed
///   forms, the screen's false-rejection count, and how far the zero-skew
///   shortcut sits from this variance).
/// * empty cell: `P(count = 0) = exp(-lambda)` is a probability, so its
///   standard error is exactly binomial, `sqrt(p (1 - p) / n)`; the check runs
///   only where its expectation exceeds five cases.
///
/// Mean and variance alone cannot see a normal approximation -- it is built to
/// match them -- so the shape is carried by the skewness and empty-cell checks:
/// Poisson's skewness is `1 / sqrt(lambda)`, a rounded normal's is zero, and
/// its empty-cell probability is not `exp(-lambda)`.
fn distribution_violations(rate: f64, counts: &[u32]) -> Vec<Violation> {
    let mut out = Vec::new();
    if counts.is_empty() {
        out.push(Violation {
            rate,
            kind: "empty-sample",
            detail: "no cases to check".to_owned(),
        });
        return out;
    }
    // A zero rate has no distribution to compare against; the contract is
    // that it produces no events at all.
    if rate <= 0.0 {
        let nonzero = counts.iter().filter(|&&c| c != 0).count();
        if nonzero != 0 {
            out.push(Violation {
                rate,
                kind: "zero-rate",
                detail: format!("{nonzero} cases produced a nonzero count"),
            });
        }
        return out;
    }
    // A drop or a collapse cannot hide behind plausible moments: a kernel
    // that loses part of its dispatch or returns one value everywhere is
    // caught here even where the bands have little power.
    if counts.iter().min() == counts.iter().max() {
        out.push(Violation {
            rate,
            kind: "collapse",
            detail: format!("every case returned {}", counts[0]),
        });
    }
    let n = counts.len() as f64;
    let mean = counts.iter().map(|&c| f64::from(c)).sum::<f64>() / n;
    let centered = |k: i32| {
        counts
            .iter()
            .map(|&c| (f64::from(c) - mean).powi(k))
            .sum::<f64>()
            / n
    };
    let m2 = centered(2);
    let variance = m2 * n / (n - 1.0);
    let skewness = centered(3) / m2.powf(1.5);

    let mean_band = SCREENING_SIGMA * (rate / n).sqrt();
    let variance_band = SCREENING_SIGMA * (rate / n + 2.0 * rate * rate / (n - 1.0)).sqrt();
    let skew_band =
        SCREENING_SIGMA * ((6.0 + 9.0 / (2.0 * rate) + 1.0 / (4.0 * rate * rate)) / n).sqrt();

    if (mean - rate).abs() > mean_band {
        out.push(Violation {
            rate,
            kind: "mean",
            detail: format!("{mean:.4} is outside +-{mean_band:.4}"),
        });
    }
    if (variance - rate).abs() > variance_band {
        out.push(Violation {
            rate,
            kind: "variance",
            detail: format!("{variance:.4} is outside +-{variance_band:.4} of {rate}"),
        });
    }
    let expected_skew = 1.0 / rate.sqrt();
    if (skewness - expected_skew).abs() > skew_band {
        out.push(Violation {
            rate,
            kind: "skewness",
            detail: format!("{skewness:.4} is outside +-{skew_band:.4} of {expected_skew:.4}"),
        });
    }
    // The exact law's probability of an empty cell. It is the sharpest check
    // where the rate is small, and it needs its own guard: at the top of the
    // range the expectation is far below one case.
    let zero = (-rate).exp();
    if zero * n > 5.0 {
        let observed = counts.iter().filter(|&&c| c == 0).count() as f64 / n;
        let zero_band = SCREENING_SIGMA * (zero * (1.0 - zero) / n).sqrt();
        if (observed - zero).abs() > zero_band {
            out.push(Violation {
                rate,
                kind: "empty-cell",
                detail: format!(
                    "P(count = 0) {observed:.5} is outside +-{zero_band:.5} of {zero:.5}"
                ),
            });
        }
    }
    out
}

/// Device-side checks of how a rate's counts were realized: no case failed,
/// and every case drew the words its rate implies. A decision must consume
/// words; zero means the kernel never ran.
fn realization_violations(rate: f64, counts: &[u32], words: &[u32]) -> Vec<Violation> {
    let mut out = Vec::new();
    if let Some(bad) = counts.iter().position(|&c| c == DYE_DECISION_FAILED) {
        out.push(Violation {
            rate,
            kind: "failed-case",
            detail: format!(
                "case {bad} reported the failure sentinel (unsupported rate, unrepresentable \
                 proposal, or exhausted word budget)"
            ),
        });
    }
    if rate > 0.0 {
        if let Some(bad) = words.iter().position(|&w| w == 0) {
            out.push(Violation {
                rate,
                kind: "no-draw",
                detail: format!("case {bad} produced its count without drawing a word"),
            });
        }
    } else if let Some(bad) = words.iter().position(|&w| w != 0) {
        out.push(Violation {
            rate,
            kind: "zero-rate-drew",
            detail: format!("case {bad} drew words at a zero rate"),
        });
    }
    out
}

/// The case set for one rate: `count` streams, each with its own cell and one
/// seed per rate, so every case gets its own stream and no two rates share
/// one.
fn rate_cases(count: usize, rate_index: usize, rate: f64) -> Vec<DyeDecisionCase> {
    (0..count)
        .map(|n| {
            let cell = ((n % 64) as i32, (n / 64) as i32);
            DyeDecisionCase::new(rate, 0x5EED_0000_0000_0000 + rate_index as u64, 0, cell)
        })
        .collect()
}

impl WgpuBackend {
    /// Count decisions for `cases`, with the number of words each one consumed.
    ///
    /// A case the device cannot realize -- an unsupported rate, or one that ran
    /// out of budget -- comes back as [`DYE_DECISION_FAILED`] and is the
    /// caller's to reject.
    pub fn dye_decision_counts(
        &self,
        cases: &[DyeDecisionCase],
    ) -> Result<(Vec<u32>, Vec<u32>), String> {
        self.dye_decision_counts_with(cases, DECISION_SHADER)
    }

    /// [`Self::dye_decision_counts`] against an arbitrary module. Private
    /// because the acceptance procedure needs a control build: a kernel with a
    /// different count law must *fail* the statistical checks, or a passing run
    /// against the real module would prove nothing.
    fn dye_decision_counts_with(
        &self,
        cases: &[DyeDecisionCase],
        source: &'static str,
    ) -> Result<(Vec<u32>, Vec<u32>), String> {
        let n = cases.len();
        if n == 0 {
            return Ok((Vec::new(), Vec::new()));
        }
        let bytes = (n * std::mem::size_of::<u32>()) as u64;
        let inputs = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dye_decision_cases"),
                contents: bytemuck::cast_slice(cases),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let alloc = |label| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let counts = alloc("dye_decision_counts");
        let words = alloc("dye_decision_words");
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_decision_readback"),
            size: 2 * bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        let writable = wgpu::BufferBindingType::Storage { read_only: false };
        let pipeline = self.cached_pipeline(source, &[storage, writable, writable]);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_decision_pass"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: inputs.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: words.as_entire_binding(),
                },
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dye_decision_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            dispatch_linear(&mut pass, n as u32);
        }
        encoder.copy_buffer_to_buffer(&counts, 0, &readback, 0, bytes);
        encoder.copy_buffer_to_buffer(&words, 0, &readback, bytes, bytes);
        self.submit_commands(Some(encoder.finish()))?;
        let out = self.read_mapped_u32(&readback, 2 * bytes);
        Ok((out[..n].to_vec(), out[n..2 * n].to_vec()))
    }

    /// The placed numerical-rejection probe: one run of [`REJECTION_SHADER`]
    /// over `places`, returning its eight words per place (the trial's state
    /// and the two verdicts as integers, then `proposal`, `lhs`, `rhs_stable`
    /// and `rhs_control` as raw f32 bits, then padding). Test-only.
    #[cfg(test)]
    fn dye_rejection_probe(&self, places: &[DyeRejectionPlace]) -> Result<Vec<u32>, String> {
        let n = places.len();
        if n == 0 {
            return Ok(Vec::new());
        }
        let bytes = (n * 8 * std::mem::size_of::<u32>()) as u64;
        let inputs = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dye_rejection_places"),
                contents: bytemuck::cast_slice(places),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let results = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_rejection_results"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_rejection_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        let writable = wgpu::BufferBindingType::Storage { read_only: false };
        let pipeline = self.cached_pipeline(REJECTION_SHADER, &[storage, writable]);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_rejection_pass"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: inputs.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: results.as_entire_binding(),
                },
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dye_rejection_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            dispatch_linear(&mut pass, n as u32);
        }
        encoder.copy_buffer_to_buffer(&results, 0, &readback, 0, bytes);
        self.queue.submit(Some(encoder.finish()));
        Ok(self.read_mapped_u32(&readback, bytes))
    }

    /// A bounded, image-independent device self-check of the shared count law,
    /// run against this backend before any Dye field is prepared. The caller
    /// caches the result per backend.
    ///
    /// It runs the production law on a fixed modest suite -- both branches,
    /// both sides of the switch, zero and the small-rate end, the highest rate
    /// production conditioning has produced, rates beyond it, and the top of
    /// the supported range -- and checks the same predeclared statistical bands
    /// as the acceptance tests, plus the properties a modest sample can still
    /// see: no case failed, no case collapsed or dropped, the switch falls at
    /// thirty, unsupported rates fail instead of returning a count, and the
    /// same case in two different dispatch shapes reads back identically. It
    /// never compares against a CPU draw: the device draws its own words.
    ///
    /// Everything it touches is fixed-size and parameter-free. A device whose
    /// counts move with the dispatch shape, lose a branch, collapse, or leave
    /// the law's moments gets a named error here instead of a wrong grain
    /// field later.
    pub(super) fn certify_dye_counts(&self) -> Result<(), String> {
        const PER_RATE: usize = 8192;
        const RATES: [f64; 11] = [
            0.0,
            0.03,
            1.0,
            5.0,
            29.0,
            30.0,
            31.0,
            160.0,
            1000.0,
            10000.0,
            DYE_MAX_RATE,
        ];
        let suite: Vec<DyeDecisionCase> = RATES
            .iter()
            .enumerate()
            .flat_map(|(index, rate)| rate_cases(PER_RATE, index, *rate))
            .collect();
        // Streams that must be realized, at the edges of the address domain
        // and on both sides of the switch...
        let mut probes: Vec<DyeDecisionCase> = [
            DyeDecisionCase::new(0.03, 0x0000_0000_0000_0001, 0, (0, 0)),
            DyeDecisionCase::new(7.5, 0x1234_5678_9ABC_DEF0, 3, (-5, 9)),
            DyeDecisionCase::new(29.999, 0xFFFF_FFFF_FFFF_FFFF, 8, (1_000_000, -1_000_000)),
            DyeDecisionCase::new(30.0, 0x9E37_79B9_7F4A_7C15, 5, (i32::MIN, i32::MAX)),
            DyeDecisionCase::new(160.0, 0xDEAD_BEEF_CAFE_F00D, 11, (-3, 2)),
            DyeDecisionCase::new(DYE_MAX_RATE, 0x5EED_0000_0000_0000, 1, (7, -7)),
        ]
        .to_vec();
        let probes_len = probes.len();
        // ...and rates the device must refuse: just above the supported range,
        // at the integer-domain half-range, and non-finite or negative.
        probes.extend([
            DyeDecisionCase::new(DYE_MAX_RATE + 1.0, 1, 0, (0, 0)),
            DyeDecisionCase::new(8388608.0, 2, 0, (0, 0)),
            DyeDecisionCase::new(f64::INFINITY, 3, 0, (0, 0)),
            DyeDecisionCase::new(f64::NAN, 4, 0, (0, 0)),
            DyeDecisionCase::new(-1.0, 5, 0, (0, 0)),
        ]);
        let suite_offset = probes.len();
        // The same cases in two dispatch shapes: the probes first and the
        // suite second, then reversed. A count that moves with the shape would
        // make a crop or a tile reseed a cell.
        let mut first = probes.clone();
        first.extend(suite.iter().copied());
        let mut second = suite.clone();
        second.extend(probes.iter().copied());
        let run = |cases: &[DyeDecisionCase]| {
            self.dye_decision_counts(cases)
                .map_err(|e| format!("the device could not run the count law: {e}"))
        };
        let (first_counts, first_words) = run(&first)?;
        let (second_counts, second_words) = run(&second)?;
        let suite_len = suite.len();

        let mut broken: Vec<String> = Vec::new();
        for (index, rate) in RATES.iter().enumerate() {
            let lo = suite_offset + index * PER_RATE;
            let counts = &first_counts[lo..lo + PER_RATE];
            let words = &first_words[lo..lo + PER_RATE];
            let mut problems = distribution_violations(*rate, counts);
            problems.extend(realization_violations(*rate, counts, words));
            broken.extend(problems.iter().map(|v| v.to_string()));
        }
        for (index, probe) in probes.iter().enumerate() {
            let refused = first_counts[index] == DYE_DECISION_FAILED;
            if index < probes_len && refused {
                broken.push(format!(
                    "rate {}: a supported probe stream reported the failure sentinel",
                    probe.lambda()
                ));
            }
            if index >= probes_len && !refused {
                broken.push(format!(
                    "rate {}: an unsupported rate returned the count {} instead of failing",
                    probe.lambda(),
                    first_counts[index]
                ));
            }
        }
        // The word count is the branch witness: Knuth multiplies one uniform
        // per count, so a rate just below the switch consumes about thirty
        // words, while PTRS accepts most proposals within its first pair. A
        // kernel that lost one branch, or moved the switch, cannot fake both.
        for (rate, above) in [(29.0, false), (30.0, true)] {
            let index = RATES.iter().position(|&r| r == rate).unwrap();
            let lo = suite_offset + index * PER_RATE;
            let used = first_words[lo..lo + PER_RATE]
                .iter()
                .map(|&w| f64::from(w))
                .sum::<f64>()
                / PER_RATE as f64;
            let (bound, branch) = if above {
                (8.0, "PTRS, at or above the switch")
            } else {
                (15.0, "Knuth, below the switch")
            };
            if (above && used >= bound) || (!above && used <= bound) {
                broken.push(format!(
                    "rate {rate}: {used:.2} mean words is not the {branch} the reference takes"
                ));
            }
        }
        for offset in 0..suite_len {
            if first_counts[suite_offset + offset] != second_counts[offset]
                || first_words[suite_offset + offset] != second_words[offset]
            {
                broken.push(format!(
                    "suite case {offset} moved between two dispatch shapes: count {} -> {}, \
                     words {} -> {}",
                    first_counts[suite_offset + offset],
                    second_counts[offset],
                    first_words[suite_offset + offset],
                    second_words[offset]
                ));
                break;
            }
        }
        for offset in 0..probes.len() {
            if first_counts[offset] != second_counts[suite_len + offset]
                || first_words[offset] != second_words[suite_len + offset]
            {
                broken.push(format!(
                    "probe case {offset} moved between two dispatch shapes: count {} -> {}, \
                     words {} -> {}",
                    first_counts[offset],
                    second_counts[suite_len + offset],
                    first_words[offset],
                    second_words[suite_len + offset]
                ));
                break;
            }
        }
        if broken.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "the device failed the Dye count-law self-check:\n  {}",
                broken.join("\n  ")
            ))
        }
    }
}

#[cfg(test)]
mod tests;
