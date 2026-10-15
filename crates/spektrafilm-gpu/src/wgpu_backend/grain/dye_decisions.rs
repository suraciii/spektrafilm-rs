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
        self.queue.submit(Some(encoder.finish()));
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
mod tests {
    use super::*;

    /// Cases per rate for the statistical checks.
    ///
    /// The count is what gives the checks their power. The weakest case is the
    /// highest rate with a shape signal, where the skewness estimate's standard
    /// error is `sqrt(6 + 9/(2 lambda) + 1/(4 lambda^2)) / sqrt(n)` while the
    /// signal is `1 / sqrt(lambda)`: at `lambda = 160` that puts the estimate
    /// about seventeen standard errors from a same-mean, same-variance
    /// substitute, roughly two and a half band-widths at the family's
    /// screening level. The checks would still see that substitute at a
    /// fraction of this count; the margin is here so that a reader never has to
    /// argue about whether they would. Each rate runs in its own dispatch, so
    /// no case buffer exceeds a 128 MiB storage binding.
    const CASES_PER_RATE: usize = 300_000;

    /// Covers zero and the small-rate end, both sides of the Knuth/PTRS
    /// switch, the highest rate production conditioning has produced (`160` --
    /// an empirical observation, not the supported limit), two rates beyond
    /// that range, and the top of the supported range.
    const RATES: [f64; 14] = [
        0.0,
        0.03,
        1.0,
        5.0,
        25.0,
        29.0,
        30.0,
        31.0,
        60.0,
        158.0,
        160.0,
        1000.0,
        10000.0,
        DYE_MAX_RATE,
    ];

    /// A device for one test, with the shared device lock held: the software
    /// Vulkan driver is not safe under concurrent device use in one process,
    /// which is the pattern every device test in this backend follows.
    fn locked_adapter() -> (
        std::sync::MutexGuard<'static, ()>,
        crate::wgpu_backend::WgpuBackend,
    ) {
        let guard = crate::wgpu_backend::observation::TEST_DEVICE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (guard, crate::test_support::adapter())
    }

    fn run_rate(
        backend: &WgpuBackend,
        source: &'static str,
        rate_index: usize,
        rate: f64,
    ) -> (Vec<u32>, Vec<u32>) {
        backend
            .dye_decision_counts_with(&rate_cases(CASES_PER_RATE, rate_index, rate), source)
            .expect("device run")
    }

    fn mean_words(words: &[u32]) -> f64 {
        words.iter().map(|&w| f64::from(w)).sum::<f64>() / words.len() as f64
    }

    /// The per-case word-consumption difference between a control run and the
    /// production law over the same case set: the mean, its standard error,
    /// and how many cases disagree. The rejection equation's verdict decides
    /// whether a case draws another pair, so an equation that is wrong about
    /// the boundary changes this even where its counts stay inside the bands.
    struct WordsDelta {
        mean: f64,
        se: f64,
        differing: usize,
    }

    fn words_delta(control: &[u32], law: &[u32]) -> WordsDelta {
        let n = control.len() as f64;
        let delta = |index: usize| control[index] as f64 - law[index] as f64;
        let mean = (0..control.len()).map(delta).sum::<f64>() / n;
        let variance = (0..control.len())
            .map(|index| (delta(index) - mean).powi(2))
            .sum::<f64>()
            / (n - 1.0);
        WordsDelta {
            mean,
            se: (variance / n).sqrt(),
            differing: (0..control.len())
                .filter(|&index| control[index] != law[index])
                .count(),
        }
    }

    /// Mean, the unbiased sample variance `s^2 = n m2/(n-1)`, the sample
    /// skewness `m3/m2^(3/2)` about the sample mean, and the empty-cell rate
    /// of one realized sample: the printed evidence, and the numbers the
    /// pooled screen checks.
    struct Moments {
        mean: f64,
        variance: f64,
        skewness: f64,
        p0: f64,
    }

    fn moments(counts: &[u32]) -> Moments {
        let n = counts.len() as f64;
        let mean = counts.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
        let m2 = counts
            .iter()
            .map(|&v| (f64::from(v) - mean).powi(2))
            .sum::<f64>()
            / n;
        let m3 = counts
            .iter()
            .map(|&v| (f64::from(v) - mean).powi(3))
            .sum::<f64>()
            / n;
        Moments {
            mean,
            variance: m2 * n / (n - 1.0),
            skewness: if m2 == 0.0 { 0.0 } else { m3 / m2.powf(1.5) },
            p0: counts.iter().filter(|&&v| v == 0).count() as f64 / n,
        }
    }

    #[test]
    fn device_counts_follow_the_reference_law() {
        let (_guard, backend) = locked_adapter();
        eprintln!("dye count law adapter: {}", backend.name());
        let mut broken = Vec::new();
        let mut pooled_counts: Vec<Vec<u32>> = RATES.iter().map(|_| Vec::new()).collect();
        let mut pooled_words: Vec<Vec<u32>> = RATES.iter().map(|_| Vec::new()).collect();
        for replicate in 0..5 {
            for (index, rate) in RATES.iter().enumerate() {
                let seed_index = replicate * RATES.len() + index;
                let (counts, words) = run_rate(&backend, DECISION_SHADER, seed_index, *rate);
                broken.extend(distribution_violations(*rate, &counts));
                broken.extend(realization_violations(*rate, &counts, &words));
                let m = moments(&counts);
                eprintln!(
                    "G1 replicate={replicate} lambda={rate} n={} mean={:.8} variance={:.8} skewness={:.8} p0={:.8} mean_words={:.4}",
                    counts.len(),
                    m.mean,
                    m.variance,
                    m.skewness,
                    m.p0,
                    mean_words(&words)
                );
                pooled_counts[index].extend_from_slice(&counts);
                pooled_words[index].extend_from_slice(&words);
            }
        }
        // The five replicates pooled: 1.5M cases per rate through the same
        // predeclared bands at their own sample size. A per-replicate screen
        // is wide enough at the top rate to hide a systematic variance bias of
        // nearly one percent -- the first hardware acceptance run's five
        // samples all held their own bands while pooling eight standard errors
        // high -- and this check is where that bias has to fail. The bands
        // stay at one screening width; pooling narrows them by taking more
        // cases, not by relaxing the screen.
        for (index, rate) in RATES.iter().enumerate() {
            let counts = &pooled_counts[index];
            let words = &pooled_words[index];
            broken.extend(distribution_violations(*rate, counts));
            broken.extend(realization_violations(*rate, counts, words));
            let m = moments(counts);
            eprintln!(
                "G1 pooled lambda={rate} n={} mean={:.8} variance={:.8} skewness={:.8} p0={:.8} mean_words={:.4}",
                counts.len(),
                m.mean,
                m.variance,
                m.skewness,
                m.p0,
                mean_words(words)
            );
        }
        assert!(
            broken.is_empty(),
            "device counts violate the reference law:\n  {}",
            render(&broken)
        );
    }

    /// The control module: the same words and entry, and the count law the
    /// specification forbids. Its mean and variance match the reference by
    /// construction, so what fails is the shape and the empty-cell probability.
    #[test]
    fn the_control_build_fails_the_same_checks() {
        let (_guard, backend) = locked_adapter();
        eprintln!("dye count law control adapter: {}", backend.name());
        let mut broken = Vec::new();
        for (index, rate) in RATES.iter().enumerate() {
            let (counts, _) = run_rate(&backend, CONTROL_SHADER, index, *rate);
            broken.extend(distribution_violations(*rate, &counts));
        }
        eprintln!("G1 altered-law control violations:\n  {}", render(&broken));
        assert!(
            !broken.is_empty(),
            "the normal-approximation control passed every check, so this test cannot \
             distinguish the reference law from a different one"
        );
        // The control differs from the reference in two places: the empty-cell
        // probability below the switch, and the shape above it. Both have to be
        // reported, or the check that covers one side is not shown to work.
        assert!(
            broken
                .iter()
                .any(|v| v.rate < 30.0 && v.kind == "empty-cell"),
            "the control was caught without the empty-cell check firing below the switch:\n  {}",
            render(&broken)
        );
        assert!(
            broken
                .iter()
                .any(|v| v.rate >= 30.0 && v.kind == "skewness"),
            "the control was caught without the shape check firing above the switch, so the \
             shape check's power against a same-mean, same-variance substitute is untested:\n  {}",
            render(&broken)
        );
    }

    /// One stream's count is a function of its address, not of how the dispatch
    /// was shaped: batching the case differently must not change it, which is
    /// what keeps a crop or a tile from reseeding a cell.
    #[test]
    fn a_streams_count_does_not_depend_on_its_position_in_the_dispatch() {
        let (_guard, backend) = locked_adapter();
        eprintln!("dye count address adapter: {}", backend.name());
        // Several streams, each of which has to survive the dispatch change on
        // its own: a single case would not show a stream that only moves when
        // the grid around it changes.
        let cases: Vec<DyeDecisionCase> = [
            DyeDecisionCase::new(7.5, 0x1234_5678_9ABC_DEF0, 3, (-5, 9)),
            DyeDecisionCase::new(0.03, 0x0000_0000_0000_0001, 0, (0, 0)),
            DyeDecisionCase::new(29.5, 0xFFFF_FFFF_FFFF_FFFF, 8, (1_000_000, -1_000_000)),
            DyeDecisionCase::new(160.0, 0x9E37_79B9_7F4A_7C15, 5, (i32::MIN, i32::MAX)),
            DyeDecisionCase::new(DYE_MAX_RATE, 0xDEAD_BEEF_CAFE_F00D, 11, (-3, 2)),
        ]
        .to_vec();
        let (alone, _) = backend
            .dye_decision_counts(&cases)
            .expect("single-case run");
        assert!(
            alone.iter().all(|&c| c != DYE_DECISION_FAILED),
            "a supported stream failed"
        );
        let padded: Vec<DyeDecisionCase> = cases
            .iter()
            .copied()
            .chain(rate_cases(CASES_PER_RATE, 3, 5.0))
            .collect();
        let (batched, _) = backend.dye_decision_counts(&padded).expect("batched run");
        assert_eq!(
            alone[..cases.len()],
            batched[..cases.len()],
            "a count moved with the dispatch shape"
        );
        let (repeat, _) = backend.dye_decision_counts(&cases).expect("repeat run");
        assert_eq!(alone, repeat, "the same case counted differently twice");
    }

    /// The switch falls at thirty, exactly where the reference's does: the
    /// word count is the branch witness, since Knuth multiplies one uniform per
    /// count while PTRS accepts most proposals within its first pair. The
    /// rates sit on both sides of the boundary, not just at it, and the
    /// smallest positive rate checks the other end: it takes the same Knuth
    /// branch without its word cost.
    #[test]
    fn the_branch_switch_falls_at_thirty() {
        const N: usize = 4096;
        let (_guard, backend) = locked_adapter();
        eprintln!("dye count branch adapter: {}", backend.name());
        for rate in [29.0, 29.999, 30.0, 30.001, 31.0] {
            let cases = rate_cases(N, 0, rate);
            let (counts, words) = backend.dye_decision_counts(&cases).expect("device run");
            assert!(
                counts.iter().all(|&c| c != DYE_DECISION_FAILED),
                "rate {rate} failed"
            );
            let used = mean_words(&words);
            if rate < 30.0 {
                assert!(used > 15.0, "rate {rate} spent {used:.2} words: not Knuth");
            } else {
                assert!(used < 8.0, "rate {rate} spent {used:.2} words: not PTRS");
            }
        }
        // Just above zero: still Knuth, but the product drops on the first
        // word, so the counts are the small integers the rate implies.
        let (counts, words) = backend
            .dye_decision_counts(&rate_cases(N, 0, 1e-6))
            .expect("device run");
        assert!(
            counts.iter().all(|&c| c != DYE_DECISION_FAILED),
            "rate 1e-6 failed"
        );
        assert!(
            counts.iter().all(|&c| c <= 2),
            "rate 1e-6 produced a count above two"
        );
        assert!(
            words.iter().all(|&w| w >= 1),
            "rate 1e-6 produced a count without drawing"
        );
    }

    /// Rates the device cannot realize fail their case; they are never
    /// rounded, saturated, or replaced by a fallback count. The boundaries are
    /// the top of the supported range (realized, and refused one step above)
    /// and the non-finite and negative rates no conditioning can produce.
    #[test]
    fn unsupported_rates_fail_the_case_instead_of_rounding_a_count() {
        let (_guard, backend) = locked_adapter();
        eprintln!("dye count guard adapter: {}", backend.name());
        let single = |rate: f64| {
            let (counts, words) = backend
                .dye_decision_counts(&[DyeDecisionCase::new(rate, 7, 0, (1, 2))])
                .expect("device run");
            (counts[0], words[0])
        };
        let (top, top_words) = single(DYE_MAX_RATE);
        assert_ne!(top, DYE_DECISION_FAILED, "the supported top rate failed");
        assert!(top_words >= 2, "the top rate drew no PTRS words");
        for rate in [
            DYE_MAX_RATE + 1.0,
            131072.0,
            8388608.0,
            1.0e9,
            f64::INFINITY,
            f64::NAN,
            -1.0,
        ] {
            let (count, _) = single(rate);
            assert_eq!(
                count, DYE_DECISION_FAILED,
                "rate {rate} returned the count {count} instead of failing"
            );
        }
        let (zero, zero_words) = single(0.0);
        assert_eq!(zero, 0, "a zero rate produced an event");
        assert_eq!(zero_words, 0, "a zero rate drew words");
    }

    /// The bands must accept a known-good law, not only reject a broken one.
    /// The reference's own exact-Poisson generator -- the CPU implementation of
    /// the same Knuth/PTRS law, drawing from its own streams -- has to pass the
    /// same checks, or the acceptance would prove only that the bands are
    /// tight.
    #[test]
    fn the_known_good_cpu_generator_passes_the_same_bands() {
        use spektrafilm_math::stats::{FastStatsRng, poisson_exact};
        let mut broken = Vec::new();
        for (index, rate) in RATES.iter().enumerate() {
            let counts: Vec<u32> = (0..CASES_PER_RATE)
                .map(|n| {
                    let mut rng =
                        FastStatsRng::stream(0x5EED_0000_0000_0000 + index as u64, 0, n as u64);
                    poisson_exact(&mut rng, *rate).expect("reference law") as u32
                })
                .collect();
            broken.extend(distribution_violations(*rate, &counts));
        }
        assert!(
            broken.is_empty(),
            "the bands reject the known-good reference generator:\n  {}",
            render(&broken)
        );
    }

    // ── the placed numerical-rejection probe ──
    //
    // Frozen from the CPU derivation `dye_rejection_derivation.py` /
    // `dye_rejection_placements.py` (in the acceptance workspace): the
    // placement set's stable-form rejection error is `2.3e-6` worst case under
    // a correctly rounded f32 `log` and stays far under the bound under
    // perturbed device `log` models, while the pre-repair factorial form's
    // error at the top rate reaches `8e-2`. The margins sit between the two
    // error scales with room for the device's own `log` on the stable side.

    /// Where the production rejection value must sit from the f64 factorial
    /// equation: three orders under the pre-repair form's error at the top
    /// rate, two orders over the measured stable-form error.
    const REJECTION_ERROR_BOUND: f64 = 1e-3;
    /// How near a placed boundary the oracle may be before its verdict is not
    /// evidence either way; every placement is outside this, and the
    /// cancellation placements sit just outside it at `+-0.006`.
    const REJECTION_MARGIN_MIN: f64 = 5e-3;
    /// The pre-repair equation's error must exceed this somewhere at the top
    /// rate, or the probe has not actually placed the cancellation it claims
    /// to reject.
    const REJECTION_CONTROL_ERROR_MIN: f64 = 2e-2;
    /// How many of the 50 placed top-rate boundaries the pre-repair equation
    /// must flip against the oracle. The CPU model flips 24; the flip count
    /// moves with the device's own rounding and `log` realization, so the
    /// floor is set well under any plausible one.
    const REJECTION_CONTROL_FLIP_MIN: usize = 10;

    /// Trial states of the shared trial body, matching `PTRS_*` in
    /// `dye_count.wgsl`.
    const TRIAL_RETRY: u32 = 0;
    const TRIAL_FAILED: u32 = 1;
    const TRIAL_SHORTCUT: u32 = 2;
    const TRIAL_LOG_TEST: u32 = 3;

    /// One oracle trial: the reference's own PTRS trial body in f64.
    #[derive(Debug)]
    struct Trial64 {
        state: u32,
        proposal: f64,
        lhs: f64,
        rhs: f64,
        margin: f64,
    }

    /// `ln(k!)` for the oracle: exact below the table's end by summed
    /// logarithms, the Stirling series with three corrections above it. Its
    /// remaining error is under `1e-11` at `k >= 13`, orders below
    /// [`REJECTION_ERROR_BOUND`].
    fn ln_fact_f64(k: f64) -> f64 {
        if k < 13.0 {
            return (2..=k as u64).map(|i| (i as f64).ln()).sum();
        }
        let z = k + 1.0;
        (z - 0.5) * z.ln() - z + 0.9189385175704956 + 1.0 / (12.0 * z) - 1.0 / (360.0 * z * z * z)
            + 1.0 / (1260.0 * z.powi(5))
    }

    /// The reference's own PTRS trial evaluated in f64 from one placed pair:
    /// the proposal and the squeeze in the reference's order, and the
    /// rejection test against the exact factorial equation
    /// `-lambda + k log(lambda) - ln(k!)`. An independent implementation of
    /// the equation the device's rearranged form must stay equivalent to; its
    /// constants are f64, so a placement's margin is what the exact math says
    /// and the device may only differ within its bounded f32 error.
    fn trial_f64(lam: f64, u01: f64, v: f64) -> Trial64 {
        let slam = lam.sqrt();
        let b = 0.931 + 2.53 * slam;
        let a = -0.059 + 0.02483 * b;
        let inv_alpha = 1.1239 + 1.1328 / (b - 3.4);
        let vr = 0.9277 - 3.6224 / (b - 2.0);
        let u = u01 - 0.5;
        let us = 0.5 - u.abs();
        let mut t = Trial64 {
            state: TRIAL_RETRY,
            proposal: 0.0,
            lhs: 0.0,
            rhs: 0.0,
            margin: 0.0,
        };
        if us == 0.0 {
            return t;
        }
        let proposal = ((2.0 * a / us + b) * u + lam + 0.43).floor();
        t.proposal = proposal;
        if proposal < 0.0 {
            return t;
        }
        if us < 0.013 && v > us {
            return t;
        }
        if proposal >= 16777216.0 {
            t.state = TRIAL_FAILED;
            return t;
        }
        if us >= 0.07 && v <= vr {
            t.state = TRIAL_SHORTCUT;
            return t;
        }
        let lhs = v.ln() + inv_alpha.ln() - (a / (us * us) + b).ln();
        let rhs = -lam + proposal * lam.ln() - ln_fact_f64(proposal);
        t.state = TRIAL_LOG_TEST;
        t.lhs = lhs;
        t.rhs = rhs;
        t.margin = lhs - rhs;
        t
    }

    /// The frozen placement set: 50 cancellation pairs at the top rate (five
    /// boundary locations either side of the peak, each at oracle margins
    /// `+-0.006`), per-rate equivalence placements at the decision bands of
    /// the whole supported range, the exact-table tail below the factorial
    /// table's end, and one placement per trial state.
    const REJECTION_PLACEMENTS: &[(f32, f32, f32)] = &[
        (65536.0, 0.629314244, 0.994368851), // cancel k=65631 margin=+0.006
        (65536.0, 0.629314244, 0.982507706), // cancel k=65631 margin=-0.006
        (65536.0, 0.65769875, 0.984759152),  // cancel k=65653 margin=+0.006
        (65536.0, 0.65769875, 0.973012686),  // cancel k=65653 margin=-0.006
        (65536.0, 0.685282886, 0.973310411), // cancel k=65675 margin=+0.006
        (65536.0, 0.685282886, 0.961700439), // cancel k=65675 margin=-0.006
        (65536.0, 0.711930156, 0.961210907), // cancel k=65697 margin=+0.006
        (65536.0, 0.711930156, 0.949745297), // cancel k=65697 margin=-0.006
        (65536.0, 0.737487793, 0.949742198), // cancel k=65719 margin=+0.006
        (65536.0, 0.737487793, 0.938413382), // cancel k=65719 margin=-0.006
        (65536.0, 0.761791468, 0.940207899), // cancel k=65741 margin=+0.006
        (65536.0, 0.761791468, 0.928992808), // cancel k=65741 margin=-0.006
        (65536.0, 0.847163379, 0.943502665), // cancel k=65834 margin=+0.006
        (65536.0, 0.847163379, 0.932248294), // cancel k=65834 margin=-0.006
        (65536.0, 0.862728357, 0.955444694), // cancel k=65856 margin=+0.006
        (65536.0, 0.862728357, 0.944047868), // cancel k=65856 margin=-0.006
        (65536.0, 0.876519263, 0.969532788), // cancel k=65878 margin=+0.006
        (65536.0, 0.876519263, 0.957967937), // cancel k=65878 margin=-0.006
        (65536.0, 0.888637185, 0.983600914), // cancel k=65900 margin=+0.006
        (65536.0, 0.888637185, 0.971868217), // cancel k=65900 margin=-0.006
        (65536.0, 0.899222374, 0.995172381), // cancel k=65922 margin=+0.006
        (65536.0, 0.899222374, 0.983301699), // cancel k=65922 margin=-0.006
        (65536.0, 0.927380025, 0.981029689), // cancel k=66002 margin=+0.006
        (65536.0, 0.927380025, 0.969327688), // cancel k=66002 margin=-0.006
        (65536.0, 0.93295753, 0.95567739),   // cancel k=66024 margin=+0.006
        (65536.0, 0.93295753, 0.944277823),  // cancel k=66024 margin=-0.006
        (65536.0, 0.354003727, 0.99034369),  // cancel k=65428 margin=+0.006
        (65536.0, 0.354003727, 0.978530586), // cancel k=65428 margin=-0.006
        (65536.0, 0.326071531, 0.979719162), // cancel k=65406 margin=+0.006
        (65536.0, 0.326071531, 0.968032777), // cancel k=65406 margin=-0.006
        (65536.0, 0.29901737, 0.967900753),  // cancel k=65384 margin=+0.006
        (65536.0, 0.29901737, 0.956355333),  // cancel k=65384 margin=-0.006
        (65536.0, 0.272987574, 0.956136346), // cancel k=65362 margin=+0.006
        (65536.0, 0.272987574, 0.944731236), // cancel k=65362 margin=-0.006
        (65536.0, 0.248142526, 0.94573015),  // cancel k=65340 margin=+0.006
        (65536.0, 0.248142526, 0.934449196), // cancel k=65340 margin=-0.006
        (65536.0, 0.224649727, 0.937951267), // cancel k=65318 margin=+0.006
        (65536.0, 0.224649727, 0.926763117), // cancel k=65318 margin=-0.006
        (65536.0, 0.15294145, 0.945191324),  // cancel k=65238 margin=+0.006
        (65536.0, 0.15294145, 0.933916807),  // cancel k=65238 margin=-0.006
        (65536.0, 0.137364984, 0.956944227), // cancel k=65216 margin=+0.006
        (65536.0, 0.137364984, 0.945529521), // cancel k=65216 margin=-0.006
        (65536.0, 0.123563074, 0.97078371),  // cancel k=65194 margin=+0.006
        (65536.0, 0.123563074, 0.959203899), // cancel k=65194 margin=-0.006
        (65536.0, 0.111434929, 0.98453474),  // cancel k=65172 margin=+0.006
        (65536.0, 0.111434929, 0.972790956), // cancel k=65172 margin=-0.006
        (65536.0, 0.100392751, 0.996118784), // cancel k=65149 margin=+0.006
        (65536.0, 0.100392751, 0.984236777), // cancel k=65149 margin=-0.006
        (65536.0, 0.0726579204, 0.97945708), // cancel k=65070 margin=+0.006
        (65536.0, 0.0726579204, 0.967773855), // cancel k=65070 margin=-0.006
        (30.0, 0.269810796, 0.92627424),     // equiv k=26 margin=+0.01
        (30.0, 0.269810796, 0.907932758),    // equiv k=26 margin=-0.01
        (30.0, 0.324428022, 0.958561242),    // equiv k=27 margin=+0.01
        (30.0, 0.324428022, 0.9395805),      // equiv k=27 margin=-0.01
        (30.0, 0.683441877, 0.88357389),     // equiv k=33 margin=+0.01
        (30.0, 0.683441877, 0.86607796),     // equiv k=33 margin=-0.01
        (30.0, 0.737503767, 0.840604782),    // equiv k=34 margin=+0.01
        (30.0, 0.737503767, 0.823959649),    // equiv k=34 margin=-0.01
        (30.0, 0.786777437, 0.80678463),     // equiv k=35 margin=+0.01
        (30.0, 0.786777437, 0.790809214),    // equiv k=35 margin=-0.01
        (31.0, 0.273049325, 0.929091215),    // equiv k=27 margin=+0.01
        (31.0, 0.273049325, 0.910693944),    // equiv k=27 margin=-0.01
        (31.0, 0.734188735, 0.845122874),    // equiv k=35 margin=+0.01
        (31.0, 0.734188735, 0.828388333),    // equiv k=35 margin=-0.01
        (60.0, 0.291303188, 0.95069313),     // equiv k=55 margin=+0.01
        (60.0, 0.291303188, 0.931868136),    // equiv k=55 margin=-0.01
        (60.0, 0.751313686, 0.866003215),    // equiv k=66 margin=+0.01
        (60.0, 0.751313686, 0.848855197),    // equiv k=66 margin=-0.01
        (160.0, 0.292025089, 0.959265113),   // equiv k=152 margin=+0.01
        (160.0, 0.292025089, 0.940270424),   // equiv k=152 margin=-0.01
        (160.0, 0.711313546, 0.923144221),   // equiv k=168 margin=+0.01
        (160.0, 0.711313546, 0.904864788),   // equiv k=168 margin=-0.01
        (160.0, 0.798334658, 0.878650606),   // equiv k=172 margin=+0.01
        (160.0, 0.798334658, 0.861252129),   // equiv k=172 margin=-0.01
        (1000.0, 0.186511695, 0.92881453),   // equiv k=968 margin=+0.01
        (1000.0, 0.186511695, 0.910422742),  // equiv k=968 margin=-0.01
        (1000.0, 0.814507842, 0.913543284),  // equiv k=1032 margin=+0.01
        (1000.0, 0.814507842, 0.89545393),   // equiv k=1032 margin=-0.01
        (10000.0, 0.0322550721, 0.240785107), // equiv k=9700 margin=+0.01
        (10000.0, 0.0322550721, 0.236017242), // equiv k=9700 margin=-0.01
        (10000.0, 0.967767298, 0.256019086), // equiv k=10300 margin=+0.01
        (10000.0, 0.967767298, 0.250949562), // equiv k=10300 margin=-0.01
        (30.0, 0.0167498421, 0.00837492105), // table k=5 margin=+6.186
        (30.0, 0.0199514795, 0.00997573975), // table k=8 margin=+2.318
        (30.0, 0.0266907252, 0.0133453626),  // table k=12 margin=-1.046
        (65536.0, 0.0, 0.5),                 // retry: us == 0
        (65536.0, 0.001, 0.5),               // retry: squeeze
        (65536.0, 0.25, 0.5),                // shortcut
        (30.0, 0.25, 0.3),                   // shortcut
        (65536.0, 0.9999995, 1e-7),          // failed: proposal past the limit
    ];

    /// The device's rejection computation has to be the original factorial
    /// equation to within a frozen f32 error, and the pre-repair arrangement
    /// of that same equation must be measurably -- and decisively -- worse on
    /// the same placed trials. The probe runs the production law's own trial
    /// body and rejection function (`ptrs_pair` / `ptrs_rhs`), so what is
    /// compared here is the equation production ships, on the actual device's
    /// arithmetic; the oracle is the reference's factorial form in f64.
    #[test]
    fn the_rejection_computation_matches_an_independent_f64_oracle() {
        let (_guard, backend) = locked_adapter();
        eprintln!("dye rejection probe adapter: {}", backend.name());
        let places: Vec<DyeRejectionPlace> = REJECTION_PLACEMENTS
            .iter()
            .map(|&(lam, u01, v)| DyeRejectionPlace {
                lam,
                u01,
                v,
                pad: 0,
            })
            .collect();
        let results = backend.dye_rejection_probe(&places).expect("probe run");
        let mut broken = Vec::new();
        let mut cancel_placements = 0usize;
        let mut control_flips = 0usize;
        let mut max_error_stable = 0.0f64;
        let mut max_error_control = 0.0f64;
        for (index, &(lam, u01, v)) in REJECTION_PLACEMENTS.iter().enumerate() {
            let row = &results[index * 8..index * 8 + 8];
            let trial = trial_f64(f64::from(lam), f64::from(u01), f64::from(v));
            if row[0] != trial.state {
                broken.push(format!(
                    "place {index} (lambda {lam}, u {u01}, v {v}): device trial state {} is not \
                     the oracle's {}",
                    row[0], trial.state
                ));
                continue;
            }
            let oracle_accept = trial.state == TRIAL_SHORTCUT
                || (trial.state == TRIAL_LOG_TEST && trial.margin <= 0.0);
            if trial.state == TRIAL_RETRY || trial.state == TRIAL_FAILED {
                if row[1] != 0 || row[2] != 0 {
                    broken.push(format!(
                        "place {index} (lambda {lam}, u {u01}, v {v}): a {} trial accepted a \
                         proposal",
                        if trial.state == TRIAL_RETRY {
                            "retried"
                        } else {
                            "failed"
                        }
                    ));
                }
                continue;
            }
            if trial.state == TRIAL_SHORTCUT {
                if row[1] != 1 || row[2] != 1 {
                    broken.push(format!(
                        "place {index} (lambda {lam}, u {u01}, v {v}): the squeeze shortcut did \
                         not accept"
                    ));
                }
                continue;
            }
            let proposal = f64::from(f32::from_bits(row[3]));
            let rhs_stable = f64::from(f32::from_bits(row[5]));
            let rhs_control = f64::from(f32::from_bits(row[6]));
            if (proposal - trial.proposal).abs() > 0.5 {
                broken.push(format!(
                    "place {index} (lambda {lam}, u {u01}, v {v}): device proposal {proposal} is \
                     not the oracle's {}",
                    trial.proposal
                ));
            }
            let error_stable = (rhs_stable - trial.rhs).abs();
            let error_control = (rhs_control - trial.rhs).abs();
            max_error_stable = max_error_stable.max(error_stable);
            max_error_control = max_error_control.max(error_control);
            if error_stable > REJECTION_ERROR_BOUND {
                broken.push(format!(
                    "place {index} (lambda {lam}, k {proposal}): the production rejection value \
                     {rhs_stable} is {error_stable} from the f64 factorial equation, above \
                     {REJECTION_ERROR_BOUND}"
                ));
            }
            if trial.margin.abs() >= REJECTION_MARGIN_MIN {
                let stable_accept = row[1] == 1;
                if stable_accept != oracle_accept {
                    broken.push(format!(
                        "place {index} (lambda {lam}, k {proposal}, margin {:.6}): the production \
                         equation {} where the oracle {}",
                        trial.margin,
                        if stable_accept {
                            "accepted"
                        } else {
                            "rejected"
                        },
                        if oracle_accept { "accepts" } else { "rejects" }
                    ));
                }
            }
            if lam == DYE_MAX_RATE as f32 && trial.margin.abs() <= 2.0 * REJECTION_MARGIN_MIN {
                cancel_placements += 1;
                if (row[2] == 1) != oracle_accept {
                    control_flips += 1;
                }
            }
        }
        assert!(
            broken.is_empty(),
            "the device rejection computation diverges from the f64 oracle:\n  {}",
            broken.join("\n  ")
        );
        eprintln!(
            "G1 rejection probe: {} places, stable max error {max_error_stable:.3e}, cancelled \
             max error {max_error_control:.3e}, cancelled verdicts flipped {control_flips} of \
             {cancel_placements} placed top-rate boundaries",
            REJECTION_PLACEMENTS.len()
        );
        assert_eq!(
            cancel_placements, 50,
            "the placement set no longer carries the 50 cancellation boundaries the probe is \
             calibrated for"
        );
        assert!(
            max_error_control >= REJECTION_CONTROL_ERROR_MIN,
            "the pre-repair equation measured only {max_error_control:.5} error at the top rate, \
             so this probe has not placed the cancellation it must reject"
        );
        assert!(
            control_flips >= REJECTION_CONTROL_FLIP_MIN,
            "the pre-repair equation's decisions diverged from the f64 oracle at only \
             {control_flips} of {cancel_placements} placed boundaries at lambda {DYE_MAX_RATE}; \
             the probe cannot reject the cancellation it is here to reject"
        );
    }

    /// The pre-repair control's pooled variance on the certified adapter
    /// (`RADV RAPHAEL_MENDOCINO`) at 3,000,000 draws, per
    /// `specs/film-grain/v3/design.md`. The gap between it and the pooled
    /// band's top is the screen's calibrated catch.
    const CERTIFIED_CANCELLED_VARIANCE: f64 = 66100.5493;

    /// The pooled screen has to be able to reject the equation this repair
    /// replaced, or the certification above proves only that the bands are
    /// wide. The control law keeps the pre-repair rejection equation and
    /// changes nothing else. What the cancelled arrangement costs depends on
    /// the adapter's arithmetic: its threshold error is the arrangement's own
    /// f32 rounding plus the adapter `log`'s per-value error multiplied by
    /// `k`, and the total is signed, so the same equation distorts different
    /// adapters differently.
    ///
    /// The certified adapter's error is positive: its counts are drawn
    /// systematically high and its ten pooled replicates reach the recorded
    /// variance `66100.5493`, above the `65910.6` top of the band at this
    /// sample size, so the screen rejects them. The software adapter
    /// (llvmpipe)'s error is negative: its count distortion stays inside the
    /// bands (`65553.0` measured) and the equation is exposed through what its
    /// verdicts cost -- 2.4% of the same cases draw a different number of
    /// words than the production law, `+0.0286` per case.
    ///
    /// The test requires the exposure the adapter permits. The pooled band
    /// must stay narrower than the certified distortion, and the control must
    /// either leave those bands or consume words differently from the
    /// production law over the same cases by more than one screening width.
    #[test]
    fn the_original_cancellation_is_exposed_at_the_top_rate() {
        const REPLICATES: usize = 10;
        let (_guard, backend) = locked_adapter();
        eprintln!("dye count cancellation-control adapter: {}", backend.name());
        let top = RATES.iter().position(|&r| r == DYE_MAX_RATE).unwrap();
        let mut pooled = Vec::new();
        let mut pooled_words = Vec::new();
        let mut law_words = Vec::new();
        for replicate in 0..REPLICATES {
            let seed_index = replicate * RATES.len() + top;
            let (counts, words) = run_rate(&backend, CANCELLED_SHADER, seed_index, DYE_MAX_RATE);
            // The production law on the same case set: the control's decision
            // consumption is compared against what the law spends, so the
            // comparison carries no case-selection noise. The production
            // equation's own error is bounded against the f64 oracle by the
            // placed probe.
            let (_, law) = run_rate(&backend, DECISION_SHADER, seed_index, DYE_MAX_RATE);
            pooled.extend_from_slice(&counts);
            pooled_words.extend_from_slice(&words);
            law_words.extend_from_slice(&law);
        }
        let n = pooled.len() as f64;
        let broken = distribution_violations(DYE_MAX_RATE, &pooled);
        let m = moments(&pooled);
        let variance_band = SCREENING_SIGMA
            * (DYE_MAX_RATE / n + 2.0 * DYE_MAX_RATE * DYE_MAX_RATE / (n - 1.0)).sqrt();
        let delta = words_delta(&pooled_words, &law_words);
        eprintln!(
            "G1 cancelled pooled lambda={DYE_MAX_RATE} n={} mean={:.8} variance={:.8} \
             skewness={:.8} p0={:.8} mean_words={:.4} variance_band=+-{variance_band:.2} \
             violations={}",
            pooled.len(),
            m.mean,
            m.variance,
            m.skewness,
            m.p0,
            mean_words(&pooled_words),
            broken.len()
        );
        eprintln!(
            "G1 cancelled decision consumption: {:+.6} words per case ({:+.2} screening \
             widths), {} of {} cases differ",
            delta.mean,
            delta.mean / delta.se,
            delta.differing,
            pooled.len()
        );
        assert!(
            DYE_MAX_RATE + variance_band < CERTIFIED_CANCELLED_VARIANCE,
            "the pooled variance band tops out at {:.2}, at or above the certified adapter's \
             recorded cancellation variance {CERTIFIED_CANCELLED_VARIANCE}: the screen is too \
             wide to reject the equation it exists for",
            DYE_MAX_RATE + variance_band
        );
        let rejected = broken.iter().any(|v| v.kind == "variance");
        let widths = delta.mean / delta.se;
        assert!(
            rejected || widths.abs() > SCREENING_SIGMA,
            "the cancelled expression left no top-rate band ({} violations) and its word \
             consumption matched the production law's ({:+.6} per case, {widths:+.2} screening \
             widths): the control proves nothing on this adapter",
            broken.len(),
            delta.mean
        );
    }

    /// The runtime self-check a fresh backend runs before `prepare_dye`.
    #[test]
    fn the_runtime_self_check_accepts_this_device() {
        let (_guard, backend) = locked_adapter();
        eprintln!("dye count self-check adapter: {}", backend.name());
        backend.certify_dye_counts().expect("device self-check");
    }
}
