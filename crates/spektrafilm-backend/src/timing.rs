use std::sync::OnceLock;
use std::time::Instant;

/// Runtime-injected switch, so `OnceLock` rather than `LazyLock`: the value
/// comes from the process entry point, not a declaration-time initializer.
static STAGE_TIMINGS: OnceLock<bool> = OnceLock::new();

/// Install the per-stage timing switch once, from the process entry point.
///
/// Prefer this over setting `SPEKTRAFILM_STAGE_TIMINGS` at runtime: the value
/// is read from stage functions that may run on rayon workers, so a process
/// environment mutation there would be unsound. This is idempotent and safe
/// to call from any thread.
///
/// The flag ORs with the environment variable rather than replacing it, so a
/// script that exports `SPEKTRAFILM_STAGE_TIMINGS=1` keeps its behaviour when
/// the same process also passes `--timings`.
pub fn set_stage_timings(enabled: bool) {
    let _ = STAGE_TIMINGS.set(enabled || std::env::var_os("SPEKTRAFILM_STAGE_TIMINGS").is_some());
}

/// Whether per-stage timings are printed to stderr.
///
/// Reads the installed value, falling back to the environment variable for
/// scripts that set it before the process starts.
pub fn stage_timings_enabled() -> bool {
    *STAGE_TIMINGS.get_or_init(|| std::env::var_os("SPEKTRAFILM_STAGE_TIMINGS").is_some())
}

pub fn print_stage_timing(enabled: bool, stage: &str, start: Instant) {
    if enabled {
        eprintln!("stage {stage}: {} ms", start.elapsed().as_millis());
    }
}
