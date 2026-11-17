//! A file the person picked but has not uploaded yet.
//!
//! Picking a dataset does not start a transfer: the person previews the file,
//! chooses a worksheet and a header row, optionally shapes the data, and then
//! confirms. Between those two moments the file is *staged* — its path held
//! here, keyed by an opaque id, with the webview holding only the id.
//!
//! This is the same split [`crate::uploads::service`] makes between its
//! `sources` map and its records, for the same reason: a command that took a
//! path would let whatever runs in the webview name any file on the disk and
//! have this process upload it to tenant storage.
//!
//! # The cached sample
//!
//! A staged entry keeps the rows read for the preview. Re-reading them on every
//! header-row click would re-open the file, and for a workbook that means
//! re-loading its whole shared-string table before the first cell resolves. The
//! cache holds one worksheet at a time: switching worksheets is rare, clicking
//! through header candidates is not.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use uuid::Uuid;
use xfalcon_contracts::datasets::{
    DatasetErrorCode, DatasetFormat, StagedDatasetId, StagedDatasetView,
};

use crate::datasets::preview::{self, Sample};
use crate::datasets::validate;
use crate::uploads::domain::sanitize_display_name;

/// Files staged at once.
///
/// A staged entry costs a path and one worksheet's rows, not a copy of the
/// file, so this bound exists to stop the map growing without limit from
/// repeated picks rather than to limit the product. It sits above
/// `MAX_CONCURRENT_UPLOADS` so a person can line a few up while three
/// transfer.
pub const MAX_STAGED_DATASETS: usize = 8;

/// One staged file.
struct StagedFile {
    /// Absolute path. Never projected — see the module doc.
    path: PathBuf,
    size: u64,
    format: DatasetFormat,
    /// The file's own name, sanitized. DA compares it against the declared
    /// content type, so it keeps its extension whatever the dataset is called.
    file_name: String,
    /// The rows last read, and which worksheet they came from.
    sample: Option<(u32, Sample)>,
}

/// Everything a command needs to read a staged file, without holding the lock.
#[derive(Clone, Debug)]
pub struct StagedSource {
    pub path: PathBuf,
    pub size: u64,
    pub format: DatasetFormat,
    pub file_name: String,
}

/// The staged files, and nothing else.
pub struct Staging {
    /// A `std::sync::Mutex` rather than an async one: every operation here is
    /// a map access, and the file reads that are not are deliberately done
    /// outside the lock.
    entries: Mutex<HashMap<String, StagedFile>>,
    account_generation: AtomicU64,
    admission: Mutex<()>,
}

impl Default for Staging {
    fn default() -> Self {
        Self::new()
    }
}

impl Staging {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            account_generation: AtomicU64::new(0),
            admission: Mutex::new(()),
        }
    }

    #[must_use]
    pub(crate) fn current_account_generation(&self) -> u64 {
        self.account_generation.load(Ordering::SeqCst)
    }

    /// Validates a picked path and stages it.
    ///
    /// Everything a file has to be is checked here, before the id exists: an
    /// entry in this map is a file that DA has a chance of accepting.
    pub fn stage(&self, path: PathBuf) -> Result<StagedDatasetView, DatasetErrorCode> {
        self.stage_for_generation(path, self.current_account_generation())
    }

    pub(crate) fn stage_for_generation(
        &self,
        path: PathBuf,
        account_generation: u64,
    ) -> Result<StagedDatasetView, DatasetErrorCode> {
        // Answered before the file is touched. The cap is checked again under
        // the lock below, which is the authoritative one; this one is here so a
        // dropped folder cannot make this process parse eight workbooks to
        // learn what one map access already knows.
        if self.len() >= MAX_STAGED_DATASETS {
            return Err(DatasetErrorCode::StageLimitReached);
        }
        let metadata = std::fs::metadata(&path).map_err(|_| DatasetErrorCode::Unreadable)?;
        if !metadata.is_file() {
            return Err(DatasetErrorCode::Unreadable);
        }
        let size = metadata.len();
        if size > crate::uploads::service::MAX_UPLOAD_BYTES {
            return Err(DatasetErrorCode::FileTooLarge);
        }
        let format = validate::classify(&path)?;
        validate::verify_content(&path, format, size)?;

        // Read outside the lock: opening a workbook to name its worksheets is
        // file work, and holding the map while it happens would block every
        // other staged file's preview.
        let worksheets = if format.has_worksheets() {
            preview::worksheet_names(&path, size)
        } else {
            Vec::new()
        };
        let file_name = sanitize_display_name(
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
                .as_str(),
        );

        let id = Uuid::new_v4().to_string();
        {
            let _admission = self.admission.lock().expect("staging admission lock");
            if self.current_account_generation() != account_generation {
                return Err(DatasetErrorCode::AuthRequired);
            }
            let mut entries = self.entries.lock().expect("staging lock");
            if entries.len() >= MAX_STAGED_DATASETS {
                return Err(DatasetErrorCode::StageLimitReached);
            }
            entries.insert(
                id.clone(),
                StagedFile {
                    path,
                    size,
                    format,
                    file_name: file_name.clone(),
                    sample: None,
                },
            );
        }
        Ok(StagedDatasetView::new(
            id, file_name, size, format, worksheets,
        ))
    }

    /// What a staged id names, without its cached rows.
    pub fn source(&self, id: &StagedDatasetId) -> Result<StagedSource, DatasetErrorCode> {
        let entries = self.entries.lock().expect("staging lock");
        let staged = entries
            .get(id.as_str())
            .ok_or(DatasetErrorCode::StageNotFound)?;
        Ok(StagedSource {
            path: staged.path.clone(),
            size: staged.size,
            format: staged.format,
            file_name: staged.file_name.clone(),
        })
    }

    /// The cached rows for one worksheet, if that is the worksheet last read.
    pub fn cached_sample(&self, id: &StagedDatasetId, worksheet_index: u32) -> Option<Sample> {
        let entries = self.entries.lock().expect("staging lock");
        let staged = entries.get(id.as_str())?;
        match &staged.sample {
            Some((cached_index, sample)) if *cached_index == worksheet_index => {
                Some(sample.clone())
            }
            _ => None,
        }
    }

    /// Keeps the rows just read, replacing any other worksheet's.
    ///
    /// Dropped silently when the id is gone: the person discarded the file
    /// while it was being read, which is an answer, not a failure.
    pub fn cache_sample(&self, id: &StagedDatasetId, worksheet_index: u32, sample: Sample) {
        let mut entries = self.entries.lock().expect("staging lock");
        if let Some(staged) = entries.get_mut(id.as_str()) {
            staged.sample = Some((worksheet_index, sample));
        }
    }

    /// Releases a staged file.
    pub fn discard(&self, id: &StagedDatasetId) {
        self.entries
            .lock()
            .expect("staging lock")
            .remove(id.as_str());
    }

    /// Releases every path and preview cached for the account being left.
    pub fn clear_account_scope(&self) {
        let _admission = self.admission.lock().expect("staging admission lock");
        self.account_generation.fetch_add(1, Ordering::SeqCst);
        self.entries.lock().expect("staging lock").clear();
    }

    /// How many files are staged. Used by tests and by the cap.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().expect("staging lock").len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Where a cleanup recipe writes the file it produces.
///
/// Under the app's own local data, not the system temp directory. A generated
/// file is deleted after success or cancellation; a failed transfer retains it
/// with its retry queue entry.
pub fn staging_directory(app_local_data: &Path) -> PathBuf {
    app_local_data.join("staging")
}
