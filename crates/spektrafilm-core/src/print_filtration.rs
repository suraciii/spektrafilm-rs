//! Print filtration ownership: the neutral baseline a dichroic axis sits at,
//! and the preset-owned shifts that offset it.
//!
//! The effective CC value of an axis is `neutral + shift`, and the two live in
//! different places. A **shift** is a preset control: it is fitted with the look
//! and travels with it. A **neutral baseline** is built in: it belongs to the
//! resolved paper, `data/filters/neutral_print_filters.json` is keyed by
//! (print stock, illuminant, film stock), and each entry is the filtration that
//! prints a neutral ramp neutral on that paper, so a value fitted on one paper
//! is not portable to another.
//!
//! No preset of either kind may carry one. A look excludes the three neutral
//! axes from its enlarger group, and an enlarger section does not own them, so a
//! document that places one there is rejected instead of silently reusing one
//! paper's filtration on the next. The neutral axes are also not editable in the
//! GUI, so a preset may not change what the user cannot.
//!
//! Per axis the resolution order is an explicit parameter source (a `--set`
//! leaf, which stays protected through preparation), the database entry for the
//! resolved (print stock, illuminant, film stock) triple, then the runtime
//! default that preparation retains when no entry exists.
//!
//! See the [print filtration contract](../../../specs/presets/spec.md#print-filtration).

use serde::Serialize;
use serde_json::Value;

use crate::neutral_filters::NeutralFilters;
use crate::params::{EnlargerParams, RuntimeParams};

/// The three neutral baseline paths, in cyan, magenta, yellow order. Axis order
/// for the protected flags, the resolution and both dry-run reports.
pub const NEUTRAL_BASELINE_PATHS: [&str; 3] = [
    "enlarger.c_filter_neutral",
    "enlarger.m_filter_neutral",
    "enlarger.y_filter_neutral",
];

/// The leaf name a neutral baseline path uses inside the `enlarger` group.
pub fn leaf_of(path: &str) -> &str {
    path.strip_prefix("enlarger.").unwrap_or(path)
}

/// The three baselines an enlarger group holds, in axis order.
pub fn baselines(enlarger: &EnlargerParams) -> [(&'static str, f32); 3] {
    let values = [
        enlarger.c_filter_neutral,
        enlarger.m_filter_neutral,
        enlarger.y_filter_neutral,
    ];
    std::array::from_fn(|axis| (leaf_of(NEUTRAL_BASELINE_PATHS[axis]), values[axis]))
}

/// Whether a canonical parameter path names a neutral filter baseline.
pub fn is_neutral_baseline(path: &str) -> bool {
    NEUTRAL_BASELINE_PATHS.contains(&path)
}

/// Which source supplied an axis's effective neutral baseline.
///
/// Reported by both dry-run formats so a caller can see that a paper change
/// moved the baseline instead of guessing from the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NeutralBaselineSource {
    /// A `--set` leaf pinned the axis.
    Set,
    /// The resolved (print stock, illuminant, film stock) entry supplied it.
    Database,
    /// The runtime default applied because no other source had a value.
    Default,
}

impl NeutralBaselineSource {
    /// The stable wire name both dry-run formats report.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Database => "database",
            Self::Default => "default",
        }
    }
}

/// One axis's effective filtration: baseline, shift, their sum, and the step
/// that supplied the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AxisFiltration {
    pub axis: &'static str,
    pub baseline: f32,
    pub shift: f32,
    pub effective: f32,
    pub source: NeutralBaselineSource,
}

/// Resolve the effective print filtration of the three dichroic axes.
///
/// Both dry-run formats report this, so a caller sees before a render which
/// step produced each baseline: an explicit parameter source, the database
/// entry for the resolved (print stock, illuminant, film stock) triple, or the
/// runtime default. The values are the ones preparation left in `params`; this
/// function names their source.
pub fn resolve(
    params: &RuntimeParams,
    film_stock: &str,
    print_stock: &str,
    database: Option<&NeutralFilters>,
) -> [AxisFiltration; 3] {
    let entry = database
        .filter(|_| params.settings.neutral_print_filters_from_database)
        .and_then(|database| database.lookup(print_stock, &params.enlarger.illuminant, film_stock));
    let axes = [
        (params.enlarger.c_filter_neutral, 0.0),
        (
            params.enlarger.m_filter_neutral,
            params.enlarger.m_filter_shift,
        ),
        (
            params.enlarger.y_filter_neutral,
            params.enlarger.y_filter_shift,
        ),
    ];
    std::array::from_fn(|axis| {
        let (baseline, shift) = axes[axis];
        let source = if params.neutral_print_filters_protected[axis] {
            NeutralBaselineSource::Set
        } else if entry.is_some() {
            NeutralBaselineSource::Database
        } else {
            NeutralBaselineSource::Default
        };
        AxisFiltration {
            axis: NEUTRAL_BASELINE_PATHS[axis],
            baseline,
            shift,
            effective: baseline + shift,
            source,
        }
    })
}

/// Remove a legacy look's carried enlarger neutrals.
///
/// Such a value is a paper's baseline that a preset may not carry: the resolved
/// paper supplies it from the built-in database, and the user's own parameter
/// source supplies it otherwise. Dropping the leaves lets an older whole look
/// keep loading; the axes then resolve for whatever paper it prints on.
pub(crate) fn strip_enlarger_baseline(parameters: &mut serde_json::Map<String, Value>) {
    let Some(enlarger) = parameters
        .get_mut("enlarger")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    for path in NEUTRAL_BASELINE_PATHS {
        enlarger.remove(leaf_of(path));
    }
}

/// Reject a neutral baseline carried by a preset of either kind.
///
/// The leaf is not preset content: it is the resolved paper's baseline, or the
/// user's own parameter source. Accepting one would let a document fitted on
/// one paper change the filtration of every other, so the whole document is
/// rejected and the message names the path and the authority that does supply
/// the value.
pub(crate) fn reject_neutral_axes(parameters: &Value) -> Result<(), String> {
    let enlarger = parameters.get("enlarger").and_then(Value::as_object);
    let Some(enlarger) = enlarger else {
        return Ok(());
    };
    for path in NEUTRAL_BASELINE_PATHS {
        if enlarger.contains_key(leaf_of(path)) {
            return Err(format!(
                "parameters.{path} is not a preset control; the resolved paper supplies the neutral filter baseline"
            ));
        }
    }
    Ok(())
}
