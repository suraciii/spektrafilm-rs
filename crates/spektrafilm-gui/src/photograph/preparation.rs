//! Captured photograph settings and material metadata; history stays on the UI.
use super::{PhotoContext, record::PhotographSettings};
use crate::input_worker::{InputSource, InputSpec, InputView, PreparedInput};
use crate::*;

pub(crate) struct PhotoPreparation {
    pub(super) path: PathBuf,
    pub(super) settings: PhotographSettings,
    pub(super) film: profile::Profile,
    pub(super) print: profile::Profile,
}
impl PhotoPreparation {
    pub(crate) fn input(&self, max_size: Option<u32>) -> InputSpec {
        InputSpec {
            source: InputSource::File {
                path: self.path.clone(),
                raw: self.settings.raw.clone(),
            },
            rotation: self.settings.rotation,
            space: self.settings.params.io.input_color_space.clone(),
            decode: self.settings.params.io.input_cctf_decoding,
            max_size,
        }
    }
    pub(crate) fn finish(self, loaded: PreparedInput) -> (PhotoContext, Option<InputView>) {
        let history = crate::app::history::AdjustmentHistory::from_photograph(
            &self.settings,
            &self.film,
            &self.print,
        );
        let baseline = Some(self.settings.clone());
        let raw_draft = self.settings.raw.clone();
        let context = PhotoContext {
            settings: self.settings,
            path: self.path,
            source_digest: loaded
                .digest
                .expect("file preparation supplies its accepted digest"),
            raw_draft,
            numeric_drafts: Default::default(),
            image: loaded.source,
            metadata: loaded.metadata,
            lens_info: loaded.lens_info,
            lens_model_identity: loaded.lens_model_identity,
            history,
            baseline,
            record_path: None,
            film_reference_illuminant: self.film.info.reference_illuminant.clone(),
            film_medium: scan_medium(&self.film),
            film_dev_times: self.film.data.development_time,
            print_dev_times: self.print.data.development_time,
        };
        (context, loaded.view)
    }
}
