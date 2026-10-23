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

pub(crate) fn stage_label_localized(ctx: &egui::Context, unit: &str) -> String {
    let key = match unit {
        "worker_wait" => "operations.stage.queued",
        "input_prepare" => "operations.stage.input_prepare",
        "resize" => "operations.stage.resize",
        "metering" => "operations.stage.metering",
        "geometry" => "operations.stage.geometry",
        "converting" => "operations.stage.converting",
        "input_transfer" => "operations.stage.input_transfer",
        "rgb_to_raw" => "operations.stage.rgb_to_raw",
        "highlight_boost" => "operations.stage.highlight_boost",
        "optical_diffusion" => "operations.stage.optical_diffusion",
        "filming_expose" => "operations.stage.filming_expose",
        "filming_develop" => "operations.stage.filming_develop",
        "grain_v1" | "grain_v2" | "grain_v2_cpu" | "grain_v3_field" | "grain_fotufilm" => {
            "operations.stage.grain"
        }
        "enlarger_lut_interpolate" => "operations.stage.enlarger_lut",
        "printing" => "operations.stage.printing",
        "printing_develop" => "operations.stage.printing_develop",
        "glare" => "operations.stage.glare",
        "scanning" => "operations.stage.scanning",
        "negative_to_positive" => "operations.stage.negative_to_positive",
        "scanner_lut" => "operations.stage.scanner_lut",
        "post_scan" => "operations.stage.post_scan",
        "output_gamut" => "operations.stage.output_gamut",
        "output_transfer" => "operations.stage.output_transfer",
        "output_mix" => "operations.stage.output_mix",
        "cmyk" => "operations.stage.cmyk",
        "film_chain" | "simulation" => "operations.stage.rendering",
        "display_prepare" => "operations.stage.display_prepare",
        other => return other.to_owned(),
    };
    crate::i18n::text(ctx, key).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sink_retains_the_innermost_unit() {
        let progress = RenderProgress::default();
        ProgressSink::current(&progress, Some("printing"));
        assert_eq!(progress.unit(), Some("printing"));
        ProgressSink::current(&progress, None);
        assert_eq!(progress.unit(), None);
    }
}
