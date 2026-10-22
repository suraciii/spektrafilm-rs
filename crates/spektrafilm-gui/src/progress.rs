//! Live operation progress for the window header.
//!
//! The render worker binds [`RenderProgress`] to its thread; the pipeline
//! reports work units through [`spektrafilm_core::progress`]. The header is
//! the only surface that shows the operation status, so the vocabulary here
//! is the one product-language name of each work unit.

use parking_lot::Mutex;
use spektrafilm_core::progress::ProgressSink;
use std::sync::OnceLock;

/// The innermost work unit an in-flight render is executing.
#[derive(Default)]
pub(crate) struct RenderProgress {
    unit: Mutex<Option<&'static str>>,
    context: OnceLock<egui::Context>,
}

impl RenderProgress {
    pub(crate) fn unit(&self) -> Option<&'static str> {
        *self.unit.lock()
    }

    /// Bind the UI context so a work-unit change repaints the header. Without
    /// it the status would advance only on unrelated repaints.
    pub(crate) fn bind_context(&self, ctx: &egui::Context) {
        let _ = self.context.set(ctx.clone());
    }
}

impl ProgressSink for RenderProgress {
    fn current(&self, unit: Option<&'static str>) {
        {
            let mut slot = self.unit.lock();
            if *slot == unit {
                return;
            }
            *slot = unit;
        }
        if let Some(ctx) = self.context.get() {
            ctx.request_repaint();
        }
    }
}

/// Product-language name of a work unit.
///
/// A unit names the physical operation it executes. An unknown unit keeps its
/// identifier rather than guessing a physical meaning.
pub(crate) fn stage_label(unit: &str) -> &str {
    match unit {
        "worker_wait" => "queued",
        "input_prepare" => "preparing input",
        "resize" => "resizing",
        "metering" => "metering",
        "geometry" => "geometry",
        "color_reference" => "color reference",
        "converting" => "negative conversion",
        "input_transfer" => "input transfer",
        "rgb_to_raw" => "raw conversion",
        "highlight_boost" => "highlight boost",
        "optical_diffusion" => "optical diffusion",
        "filming_expose" => "film exposure",
        "filming_develop" => "film development",
        "grain_v1" | "grain_v2" | "grain_v2_cpu" | "grain_v3_field" | "grain_fotufilm" => "grain",
        "enlarger_lut_interpolate" => "enlarger LUT",
        "printing" => "printing",
        "printing_develop" => "print development",
        "glare" => "print viewing glare",
        "scanning" => "scanning",
        "negative_to_positive" => "negative to positive",
        "scanner_film_points" => "scanner film points",
        "scanner_finish" => "scanner finish",
        "scanner_lut" => "scanner LUT",
        "post_scan" => "post-scan",
        "output_gamut" => "output gamut",
        "output_transfer" => "output transfer",
        "output_mix" => "output mix",
        "cmyk" => "CMYK ink style",
        "film_chain" | "simulation" => "rendering",
        "display_prepare" => "display transform",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_keep_their_physical_names() {
        assert_eq!(stage_label("filming_develop"), "film development");
        assert_eq!(stage_label("grain_v3_field"), "grain");
        assert_eq!(stage_label("scanner_lut"), "scanner LUT");
        // An unknown unit keeps its identifier instead of inventing a stage.
        assert_eq!(stage_label("not_a_stage"), "not_a_stage");
    }

    #[test]
    fn the_sink_retains_the_innermost_unit() {
        let progress = RenderProgress::default();
        ProgressSink::current(&progress, Some("printing"));
        assert_eq!(progress.unit(), Some("printing"));
        ProgressSink::current(&progress, None);
        assert_eq!(progress.unit(), None);
    }
}
