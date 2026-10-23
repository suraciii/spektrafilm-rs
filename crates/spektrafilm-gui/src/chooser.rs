//! Native file chooser boundary.
//!
//! Production opens the platform chooser through `rfd` and hands the
//! confirmed path — or `None` when the user cancels — to the caller's
//! existing operation. Acceptance scripts that result instead of opening a
//! native dialog, so the same production operation still runs against a
//! real file or a real cancellation.

use std::path::PathBuf;

/// Chooser outcome for one action. `Scripted` carries the path the chooser
/// would have returned; `Canceled` is the user dismissing the dialog.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChooserResult {
    Scripted(PathBuf),
    Canceled,
}

/// Opens the platform chooser. Acceptance replaces the platform result so
/// the same production operation runs without a native dialog.
#[derive(Default)]
pub(super) struct Chooser {
    #[cfg(test)]
    scripted: std::collections::VecDeque<ChooserResult>,
    /// A scripted action without a pre-seeded result cancels instead of
    /// opening a native dialog on a machine that may have no display.
    #[cfg(test)]
    strict: bool,
}

#[cfg(test)]
impl Chooser {
    pub(super) fn scripted(&mut self, result: ChooserResult) {
        self.scripted.push_back(result);
    }

    pub(super) fn set_strict(&mut self, strict: bool) {
        self.strict = strict;
    }

    /// Path of an action inside a scripted chooser sequence, for test setup.
    pub(super) fn chooser_path(path: &std::path::Path) -> ChooserResult {
        ChooserResult::Scripted(path.to_path_buf())
    }
}

impl Chooser {
    /// Pick an existing file. Production runs the real chooser; a scripted
    /// session returns the seeded result.
    pub(super) fn pick_file(&mut self, dialog: rfd::FileDialog) -> Option<PathBuf> {
        #[cfg(test)]
        if self.strict {
            return match self.scripted.pop_front() {
                Some(ChooserResult::Scripted(path)) => Some(path),
                Some(ChooserResult::Canceled) | None => None,
            };
        }
        dialog.pick_file()
    }

    pub(super) fn pick_files(&mut self, dialog: rfd::FileDialog) -> Option<Vec<PathBuf>> {
        #[cfg(test)]
        if self.strict {
            return match self.scripted.pop_front() {
                Some(ChooserResult::Scripted(path)) => Some(vec![path]),
                Some(ChooserResult::Canceled) | None => None,
            };
        }
        dialog.pick_files()
    }

    pub(super) fn pick_folder(&mut self, dialog: rfd::FileDialog) -> Option<PathBuf> {
        #[cfg(test)]
        if self.strict {
            return match self.scripted.pop_front() {
                Some(ChooserResult::Scripted(path)) => Some(path),
                Some(ChooserResult::Canceled) | None => None,
            };
        }
        dialog.pick_folder()
    }

    /// Choose a destination file. Same contract as [`Self::pick_file`].
    pub(super) fn save_file(&mut self, dialog: rfd::FileDialog) -> Option<PathBuf> {
        #[cfg(test)]
        if self.strict {
            return match self.scripted.pop_front() {
                Some(ChooserResult::Scripted(path)) => Some(path),
                Some(ChooserResult::Canceled) | None => None,
            };
        }
        dialog.save_file()
    }
}
