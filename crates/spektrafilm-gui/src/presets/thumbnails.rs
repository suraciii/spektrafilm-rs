//! Current-photograph preset thumbnails: identity, bounded session cache, and demand.
use super::thumbnail_worker::{
    Candidate, Outcome, ThumbnailContext, ThumbnailKey, ThumbnailResult, ThumbnailWorker,
};
use crate::App;
use eframe::egui;
use spektrafilm_core::presets::{LibraryEntry, Preset, PresetGroup, PresetKind};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Visible cards the worker may render. This is the explicit entry ceiling:
/// at most this many textures of `THUMBNAIL_SIZE`'s long edge (about 15 MiB)
/// are retained for the accepted source, and no offscreen card is requested.
/// It covers every supported browser viewport; a window tall enough to show
/// more cards keeps the remainder in the explicit pending state.
const MAX_VISIBLE_DEMAND: usize = 64;
/// Seconds between an accepted-context change and the first candidate request.
const SETTLE_SECONDS: f64 = 0.25;
/// Long edge of one candidate preview. Small enough for a two-column browser,
/// large enough that the accepted look is recognizable.
pub(super) const THUMBNAIL_SIZE: u32 = 240;

/// Display-profile content identity, rechecked at most once per second so a
/// same-metadata replacement is still detected without reading the ICC file
/// on every frame.
#[derive(Default)]
struct DisplayIdentity {
    cached: RefCell<Option<(PathBuf, Instant, u64)>>,
    recheck: Cell<Option<Instant>>,
}

impl DisplayIdentity {
    fn fingerprint(&self, path: &Path) -> u64 {
        let now = Instant::now();
        let mut cached = self.cached.borrow_mut();
        if let Some((cached_path, checked, identity)) = cached.as_ref()
            && cached_path == path
            && now.duration_since(*checked) < Duration::from_secs(1)
        {
            self.recheck.set(Some(*checked + Duration::from_secs(1)));
            return *identity;
        }
        let identity = std::fs::read(path)
            .map(|bytes| {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                bytes.hash(&mut hasher);
                hasher.finish()
            })
            .unwrap_or(0);
        *cached = Some((path.to_path_buf(), now, identity));
        // Keep a 1 Hz content check alive while the browser is idle, so an ICC
        // replacement is still detected after this fresh read.
        self.recheck.set(Some(now + Duration::from_secs(1)));
        identity
    }

    fn recheck_at(&self) -> Option<Instant> {
        self.recheck.get()
    }
}

/// Cheap content proxy for a file the candidates read: size and modification
/// time change with ordinary rewrites, without hashing the whole file per frame.
fn file_fingerprint(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::metadata(path) else {
        return 0;
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    metadata.len().hash(&mut hasher);
    metadata.modified().ok().hash(&mut hasher);
    hasher.finish()
}

fn content_fingerprint(preset: &Preset) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match preset.to_json() {
        Ok(document) => document.hash(&mut hasher),
        Err(_) => 0_u8.hash(&mut hasher),
    }
    hasher.finish()
}

/// Identity of the materials candidate preparation reads. File metadata
/// changes on ordinary edits; the profile reference pins the intended content.
fn material_fingerprint(data_dir: &Path, entries: &[LibraryEntry]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entry in entries {
        for reference in [&entry.film_profile, &entry.print_profile]
            .into_iter()
            .flatten()
        {
            reference.stock.hash(&mut hasher);
            reference.version.hash(&mut hasher);
            reference.content_identity.hash(&mut hasher);
            file_fingerprint(
                &data_dir
                    .join("profiles")
                    .join(format!("{}.json", reference.stock)),
            )
            .hash(&mut hasher);
        }
    }
    file_fingerprint(&data_dir.join("filters").join("neutral_print_filters.json"))
        .hash(&mut hasher);
    let mut styles: Vec<PathBuf> = std::fs::read_dir(data_dir.join("scan-styles"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default();
    styles.sort();
    for path in styles {
        path.file_name().hash(&mut hasher);
        file_fingerprint(&path).hash(&mut hasher);
    }
    hasher.finish()
}

/// The display profile the candidate display path consumes, including the
/// platform-discovered fallback that an unset selection uses.
fn effective_display_profile(app: &App) -> Option<PathBuf> {
    app.gui_state
        .display_profile()
        .map(PathBuf::from)
        .or_else(|| {
            app.viewer
                .settings
                .use_display_transform
                .then(crate::display::effective_display_profile)
                .flatten()
        })
}

/// Identity of everything the accepted candidates depend on. A changed
/// signature invalidates every cached thumbnail, including failures.
fn thumbnail_signature(app: &App, display: &DisplayIdentity) -> Option<u64> {
    app.image.as_ref()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app.input_epoch.hash(&mut hasher);
    app.parameter_revision.hash(&mut hasher);
    app.input_rotation.hash(&mut hasher);
    app.viewer.settings.use_display_transform.hash(&mut hasher);
    app.scan_for_print_snapshot.hash(&mut hasher);
    app.presets.refresh_epoch.hash(&mut hasher);
    material_fingerprint(&app.data_dir, &app.presets.entries).hash(&mut hasher);
    let display_profile = effective_display_profile(app);
    display_profile.hash(&mut hasher);
    display_profile
        .as_deref()
        .map(|path| display.fingerprint(path))
        .hash(&mut hasher);
    app.backend.preparation_device_id().hash(&mut hasher);
    THUMBNAIL_SIZE.hash(&mut hasher);
    Some(hasher.finish())
}

fn thumbnail_context(app: &App) -> ThumbnailContext {
    let mut params = app.params.clone();
    if let Some(glare) = app.scan_for_print_snapshot {
        params.print_render.glare.active = glare;
    }
    ThumbnailContext {
        source: app.image.clone().expect("signature requires a source"),
        params,
        data_dir: app.data_dir.clone(),
        special: app.gui_state.special().clone(),
        backend: app.backend.clone(),
        display_enabled: app.viewer.settings.use_display_transform,
        display_profile: effective_display_profile(app),
        rotation_quarter_turns: app.input_rotation.rem_euclid(4) as u32,
        cache_epoch: {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            app.presets.refresh_epoch.hash(&mut hasher);
            material_fingerprint(&app.data_dir, &app.presets.entries).hash(&mut hasher);
            hasher.finish()
        },
        size: THUMBNAIL_SIZE,
    }
}

pub(super) struct Thumbnails {
    worker: ThumbnailWorker,
    entries: BTreeMap<ThumbnailKey, Entry>,
    pending: BTreeSet<ThumbnailKey>,
    /// Validated document fingerprints, so a candidate follows its document
    /// contents rather than its name or position.
    content: BTreeMap<(PresetGroup, String), u64>,
    context: Option<ThumbnailContext>,
    signature: Option<u64>,
    display_identity: DisplayIdentity,
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
            content: BTreeMap::new(),
            context: None,
            signature: None,
            display_identity: DisplayIdentity::default(),
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

    /// Identity of the accepted dependencies right now. A change invalidates
    /// every cached candidate; the context itself is only cloned on reset.
    pub(super) fn accepted_signature(&self, app: &App) -> Option<u64> {
        thumbnail_signature(app, &self.display_identity)
    }

    /// Render context for an accepted signature. Callers only need this when
    /// `signature` changes from the cached one.
    pub(super) fn accepted_context(&self, app: &App) -> ThumbnailContext {
        thumbnail_context(app)
    }

    /// Deadline for the next display-ICC content check, if a profile is used.
    pub(super) fn display_recheck_at(&self) -> Option<Instant> {
        self.display_identity.recheck_at()
    }

    /// Rebuild candidate document fingerprints after a library scan.
    pub(super) fn update_content(&mut self, entries: &[LibraryEntry]) {
        self.content.clear();
        for entry in entries {
            if let Some(preset) = &entry.preset
                && preset.kind() == Ok(PresetKind::Look)
            {
                self.content.insert(
                    (entry.group, entry.name.clone()),
                    content_fingerprint(preset),
                );
            }
        }
    }

    /// Candidate cache key for the accepted `generation` and the entry's
    /// validated document contents.
    pub(super) fn candidate_key(&self, generation: u64, entry: &LibraryEntry) -> ThumbnailKey {
        ThumbnailKey::new(
            generation,
            entry.group,
            &entry.name,
            self.content
                .get(&(entry.group, entry.name.clone()))
                .copied()
                .unwrap_or(0),
        )
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
