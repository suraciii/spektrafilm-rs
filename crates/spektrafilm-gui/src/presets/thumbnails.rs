//! Current-photograph preset thumbnails: bounded session cache and demand.
use super::thumbnail_worker::{
    Candidate, Outcome, ThumbnailContext, ThumbnailKey, ThumbnailResult, ThumbnailWorker,
};
use eframe::egui;
use spektrafilm_core::presets::Preset;
use std::collections::{BTreeMap, BTreeSet};

/// Visible cards the worker may render. This is the explicit entry ceiling:
/// at most this many textures of `THUMBNAIL_SIZE`'s long edge (about 15 MiB)
/// are retained for the accepted source, and no offscreen card is requested.
/// It covers every supported browser viewport; a window tall enough to show
/// more cards keeps the remainder in the explicit pending state.
const MAX_VISIBLE_DEMAND: usize = 64;
/// Seconds between an accepted-context change and the first candidate request.
const SETTLE_SECONDS: f64 = 0.25;

pub(super) struct Thumbnails {
    worker: ThumbnailWorker,
    entries: BTreeMap<ThumbnailKey, Entry>,
    pending: BTreeSet<ThumbnailKey>,
    context: Option<ThumbnailContext>,
    signature: Option<u64>,
    /// Bumped whenever the accepted context changes, so results rendered
    /// against the previous context cannot enter the cleared cache.
    request: u64,
    ready_at: f64,
    /// Results injected by state-transition tests instead of a real render.
    #[cfg(test)]
    test_results: Vec<ThumbnailResult>,
}

enum Entry {
    Ready(egui::TextureHandle),
    Failed(crate::app::Message),
}

pub(super) enum Status<'a> {
    Ready(&'a egui::TextureHandle),
    Pending,
    Failed(&'a crate::app::Message),
}

impl Default for Thumbnails {
    fn default() -> Self {
        Self {
            worker: ThumbnailWorker::new(),
            entries: BTreeMap::new(),
            pending: BTreeSet::new(),
            context: None,
            signature: None,
            request: 0,
            ready_at: 0.0,
            #[cfg(test)]
            test_results: Vec::new(),
        }
    }
}

impl Thumbnails {
    /// Identity of the accepted context the cached candidates were rendered
    /// under. A change discards every candidate, including failures.
    pub(super) fn signature(&self) -> Option<u64> {
        self.signature
    }

    /// Adopt the accepted context for `signature`. A changed signature drops
    /// all cached candidates and defers new requests until the context settles.
    pub(super) fn reset(
        &mut self,
        signature: Option<u64>,
        context: Option<ThumbnailContext>,
        now: f64,
    ) {
        if self.signature == signature && self.context.is_some() == context.is_some() {
            self.context = context;
            return;
        }
        self.signature = signature;
        self.context = context;
        self.request = self.request.wrapping_add(1);
        self.entries.clear();
        self.pending.clear();
        self.worker.cancel();
        self.ready_at = now + SETTLE_SECONDS;
    }

    /// Drain completed candidates, then request the visible demand.
    pub(super) fn prepare(
        &mut self,
        ctx: &egui::Context,
        visible: &[(ThumbnailKey, Preset)],
        foreground_busy: bool,
    ) {
        while let Some(result) = self.worker.try_recv() {
            self.accept(ctx, visible, result);
        }
        #[cfg(test)]
        for result in std::mem::take(&mut self.test_results) {
            self.accept(ctx, visible, result);
        }
        let now = ctx.input(|input| input.time);
        let Some(context) = self.context.as_ref() else {
            self.worker.set_foreground(true);
            self.worker.cancel();
            self.entries.clear();
            self.pending.clear();
            return;
        };
        let wanted: BTreeSet<ThumbnailKey> = visible
            .iter()
            .take(MAX_VISIBLE_DEMAND)
            .map(|(key, _)| key.clone())
            .collect();
        self.entries.retain(|key, _| wanted.contains(key));
        if foreground_busy {
            self.worker.set_foreground(true);
            self.worker.cancel();
            self.pending.clear();
            return;
        }
        self.worker.set_foreground(false);
        if now < self.ready_at {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                (self.ready_at - now).max(0.0) + 0.001,
            ));
            return;
        }
        let candidates: Vec<Candidate> = visible
            .iter()
            .take(MAX_VISIBLE_DEMAND)
            .filter(|(key, _)| !self.entries.contains_key(key))
            .map(|(key, preset)| Candidate {
                key: key.clone(),
                preset: preset.clone(),
            })
            .collect();
        let wanted_pending: BTreeSet<ThumbnailKey> = candidates
            .iter()
            .map(|candidate| candidate.key.clone())
            .collect();
        if self.pending.is_empty() {
            if candidates.is_empty() {
                return;
            }
            self.submit(context.clone(), candidates);
        } else if !self.pending.is_subset(&wanted_pending) {
            // Demand no longer includes work already queued: replace it so
            // offscreen candidates stop instead of delaying visible ones.
            self.worker.cancel();
            self.pending.clear();
            if candidates.is_empty() {
                return;
            }
            self.submit(context.clone(), candidates);
        } else {
            // The active batch is still part of visible demand; let it drain.
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        if !self.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn accept(
        &mut self,
        ctx: &egui::Context,
        visible: &[(ThumbnailKey, Preset)],
        result: ThumbnailResult,
    ) {
        if result.request != self.request {
            return;
        }
        self.pending.remove(&result.key);
        if !visible.iter().any(|(key, _)| key == &result.key) {
            return;
        }
        match result.outcome {
            Outcome::Ready(image) if image.size[0] > 0 && image.size[1] > 0 => {
                let texture = ctx.load_texture(
                    format!(
                        "preset-thumbnail:{}:{}",
                        result.key.generation, result.key.identity
                    ),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                self.entries.insert(result.key, Entry::Ready(texture));
            }
            Outcome::Ready(_) => {
                self.entries.insert(
                    result.key,
                    Entry::Failed(crate::app::Message::new(
                        "surfaces.presets.thumbnail_unavailable",
                    )),
                );
            }
            Outcome::Failed(message) => {
                self.entries.insert(result.key, Entry::Failed(message));
            }
        }
    }

    #[cfg(test)]
    fn queue_result(&mut self, request: u64, key: ThumbnailKey, outcome: Outcome) {
        self.test_results.push(ThumbnailResult {
            request,
            key,
            outcome,
        });
    }

    /// Stop candidate work while the panel is collapsed or unavailable.
    pub(super) fn suspend(&mut self) {
        self.worker.set_foreground(true);
        self.worker.cancel();
        self.pending.clear();
    }

    fn submit(&mut self, context: ThumbnailContext, candidates: Vec<Candidate>) {
        self.pending
            .extend(candidates.iter().map(|candidate| candidate.key.clone()));
        self.worker.submit(context, candidates, self.request);
    }

    pub(super) fn status(&self, key: &ThumbnailKey) -> Status<'_> {
        match self.entries.get(key) {
            Some(Entry::Ready(texture)) => Status::Ready(texture),
            Some(Entry::Failed(message)) => Status::Failed(message),
            None => Status::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use spektrafilm_core::presets::{PresetGroup, builtin_presets};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn fixture() -> (Thumbnails, egui::Context, ThumbnailKey, Preset) {
        let preset = builtin_presets().remove(0);
        let key = ThumbnailKey::new(7, PresetGroup::BuiltIn, &preset.name, 0);
        let context = ThumbnailContext {
            source: crate::render::InputImage::Host(Arc::new(
                spektrafilm_math::image::ImageBuf::from_data(1, 1, vec![0.0, 0.0, 0.0]),
            )),
            params: spektrafilm_core::params::RuntimeParams::default(),
            data_dir: PathBuf::from("."),
            special: serde_json::json!({}),
            backend: Arc::new(CpuBackend),
            display_enabled: false,
            display_profile: None,
            rotation_quarter_turns: 0,
            cache_epoch: 1,
            size: 1,
        };
        let mut thumbnails = Thumbnails::default();
        thumbnails.reset(Some(11), Some(context), 0.0);
        (thumbnails, egui::Context::default(), key, preset)
    }

    #[test]
    fn results_from_an_invalidated_context_never_enter_the_cache() {
        let (mut thumbnails, ctx, key, preset) = fixture();
        let current = thumbnails.request;
        let visible = [(key.clone(), preset)];
        thumbnails.queue_result(
            current.wrapping_sub(1),
            key.clone(),
            Outcome::Failed(crate::app::Message::new("stale result")),
        );
        thumbnails.prepare(&ctx, &visible, true);
        assert!(
            matches!(thumbnails.status(&key), Status::Pending),
            "a result rendered for the previous context must not be published"
        );
        thumbnails.queue_result(
            current,
            key.clone(),
            Outcome::Failed(crate::app::Message::new("current result")),
        );
        thumbnails.prepare(&ctx, &visible, true);
        assert!(
            matches!(thumbnails.status(&key), Status::Failed(_)),
            "a current result remains an unavailable state"
        );
    }
}
