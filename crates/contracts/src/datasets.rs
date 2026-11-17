//! Datasets, as the Datasets destination sees them
//! (spec: dataset-uploads).
//!
//! A dataset is a file a person uploaded into tenant storage, where it becomes
//! queryable data that outlives every conversation. A composer attachment is
//! not one: it is 10 MiB, belongs to one message, and is fetched back through a
//! signed GET. The two share the transfer runtime in
//! `src-tauri/src/uploads` and nothing else, which is why the vocabulary here
//! is separate from [`crate::uploads`].
//!
//! # What deliberately never crosses this boundary
//!
//! [`crate::uploads`]'s module doc names the three things the transfer keeps to
//! itself — the local path, the presigned URL, the storage key. This module
//! adds two of its own:
//!
//! - **The staged file's path.** Picking a file does not start an upload here;
//!   a person previews it and confirms first. Between those two moments the
//!   path lives in the backend's staging map and the webview holds
//!   [`StagedDatasetId`], which is an opaque id and nothing else. There is
//!   deliberately no command that starts an upload from a path.
//! - **The presigned download URL.** DA returns one for a dataset file, and
//!   xfalcon-web assigns it to an anchor's `href`. That URL is a bearer
//!   credential: anyone holding it can read tenant object storage without an
//!   identity. So the backend fetches it and streams the body to a destination
//!   the person chose in the OS save dialog, and the reply is the same
//!   [`crate::export::DocumentExportResult`] every other "save this as a file"
//!   action answers with. `crate::export`'s doc argues for one export command
//!   rather than one per surface; that argument holds where the renderer
//!   composes the bytes, and this is the one case where it cannot — the bytes
//!   are in object storage and can be 2 GiB.
//!
//! # Preview cells are strings, and are not reformatted
//!
//! [`DatasetPreview::rows`] carries each cell in the file's own lexical form.
//! Reformatting a number for display would be a locale decision, and the
//! locale question is open elsewhere; showing `1,5` as `15` in a confirmation
//! step would be the worst possible place to get it wrong. A cast in a
//! [`DatasetRecipe`] still parses properly — it runs in the backend against
//! the file's values, never against these display strings.
//!
//! Each column carries an inferred type beside it so the surface can label
//! what it is showing without deriving the type from a truncated sample.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::uploads::{UploadId, UploadQueueV1};
use crate::{ContractVersion, CONTRACT_VERSION};

/// A dataset in tenant storage. DA's `dataset_id`.
#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct DatasetId(String);

impl DatasetId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for DatasetId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for DatasetId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// A file the person picked but has not uploaded yet.
///
/// Opaque by construction: it names an entry in the backend's staging map, so
/// holding one grants nothing but the right to preview, discard, or upload the
/// file the person already chose.
#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct StagedDatasetId(String);

impl StagedDatasetId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for StagedDatasetId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for StagedDatasetId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Where a dataset is in DA's ingest.
///
/// The catalog shows only [`Self::Ready`]. An `uploading` row whose transfer
/// died is not a dataset a person can use, and a `failed` one never will be —
/// listing either as a permanent entry would make the catalog a record of what
/// was attempted rather than of what exists.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DatasetStatus {
    Uploading,
    Ready,
    Failed,
}

/// The file kinds this destination accepts.
///
/// Exactly the two xfalcon-web's picker accepts, because DA's presign enforces
/// `content_type == _EXT_TO_MIME[ext]` and a third kind would be refused
/// server-side after the transfer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DatasetFormat {
    Csv,
    Xlsx,
}

impl DatasetFormat {
    /// The extension this format is always recognised by.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
        }
    }

    /// The MIME type DA's presign requires for this extension.
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Csv => "text/csv",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        }
    }

    /// Whether the format can hold more than one worksheet.
    #[must_use]
    pub const fn has_worksheets(self) -> bool {
        matches!(self, Self::Xlsx)
    }
}

/// One column of a dataset file, or of a preview.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetColumnView {
    pub name: String,
    pub inferred_type: String,
}

/// One file inside a dataset.
///
/// Every count is optional because DA returns null for rows written before its
/// audit-column migration. Absence renders as unknown, which is not zero: a
/// table of zero rows and a table whose count was never recorded are different
/// facts, and collapsing them would tell a person their data is empty.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetFileView {
    pub file_id: String,
    pub table_name: String,
    #[ts(type = "number | null")]
    pub row_count: Option<u64>,
    pub column_count: Option<u32>,
    #[ts(type = "number | null")]
    pub size_bytes: Option<u64>,
    pub original_filename: Option<String>,
    /// The server's inferred schema, in the order it returned. A map would let
    /// the renderer's iteration order decide the column order.
    pub columns: Vec<DatasetColumnView>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// One dataset, without its files. Everything the catalog needs to name,
/// order, search, and open one.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: DatasetId,
    pub display_name: String,
    pub status: DatasetStatus,
    /// Epoch milliseconds. DA reports an ISO-8601 string; it is parsed in the
    /// backend so every surface reads the same instant the same way, rather
    /// than each one carrying its own string parser.
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetListResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub datasets: Vec<DatasetView>,
    /// False only when an administrator deactivated the account's upload
    /// source. DA omits the field on older builds, and absence there is read
    /// as enabled — so a server that never learned to report it is not
    /// mistaken for one that turned uploads off.
    pub uploads_enabled: bool,
    /// The page cap stopped the walk before DA's reported total. Reported so
    /// the surface can say the list is partial; a truncated list presented as
    /// complete is how a person concludes their dataset was lost.
    pub truncated: bool,
}

impl DatasetListResult {
    #[must_use]
    pub fn new(datasets: Vec<DatasetView>, uploads_enabled: bool, truncated: bool) -> Self {
        Self {
            version: CONTRACT_VERSION,
            datasets,
            uploads_enabled,
            truncated,
        }
    }
}

/// One dataset with its files.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetDetail {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub dataset: DatasetView,
    pub files: Vec<DatasetFileView>,
    /// Same default-true semantics as [`DatasetListResult::uploads_enabled`].
    /// Present here too because the drawer's download action is gated by it.
    pub uploads_enabled: bool,
}

impl DatasetDetail {
    #[must_use]
    pub fn new(dataset: DatasetView, files: Vec<DatasetFileView>, uploads_enabled: bool) -> Self {
        Self {
            version: CONTRACT_VERSION,
            dataset,
            files,
            uploads_enabled,
        }
    }
}

/// A picked file, before it is uploaded.
///
/// Carries no path — see the module doc. `worksheets` is empty for every
/// format that cannot hold more than one.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StagedDatasetView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: StagedDatasetId,
    /// The suggested dataset name, already sanitized for display. The person
    /// can rename it before uploading.
    pub display_name: String,
    #[ts(type = "number")]
    pub size: u64,
    pub format: DatasetFormat,
    pub worksheets: Vec<String>,
}

impl StagedDatasetView {
    #[must_use]
    pub fn new(
        id: impl Into<StagedDatasetId>,
        display_name: impl Into<String>,
        size: u64,
        format: DatasetFormat,
        worksheets: Vec<String>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id: id.into(),
            display_name: display_name.into(),
            size,
            format,
            worksheets,
        }
    }
}

/// Why a staged file has no preview.
///
/// An outcome of a successful call, not a failure: the app looked and can say
/// what it found. The file still uploads — DA applies its own defaults for the
/// worksheet and the header row.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DatasetPreviewUnavailable {
    /// Above the parse ceiling. A workbook must be parsed whole to be read at
    /// all, so the ceiling is the parse, not the transfer.
    FileTooLarge,
    /// No line-delimited structure in the leading bytes, or a cell so long the
    /// parser would buffer the file into one value.
    NotDelimited,
    /// The worksheet or the file could not be read as tabular data.
    Unreadable,
}

/// A staged file's first rows, after the recipe.
///
/// The recipe is applied by the same backend function that produces the
/// uploaded file, so this is not an approximation of the result: it is the
/// result, computed over a sample.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetPreview {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub staged_id: StagedDatasetId,
    /// Which worksheet these rows came from. Zero for a format with none.
    pub worksheet_index: u32,
    /// Which sampled row was read as the header row.
    pub header_row_index: u32,
    /// The row the backend suggests holds the headers, so a file with a title
    /// or banner row above its columns opens on the right one.
    pub suggested_header_row: u32,
    pub columns: Vec<DatasetColumnView>,
    /// A cell absent from the file is `null`, distinct from an empty string:
    /// the difference decides whether a remove-nulls step drops the row.
    pub rows: Vec<Vec<Option<String>>>,
    /// The leading raw rows, before a header row is chosen, so the picker can
    /// show what each candidate would name the columns.
    pub header_candidates: Vec<Vec<Option<String>>>,
    /// Rows read from the file before the recipe ran.
    pub sampled_rows: u32,
    pub truncated_rows: bool,
    pub truncated_columns: bool,
    /// At least one cell was cut to its display ceiling.
    pub truncated_cell: bool,
    /// Set when there are no rows to show and why. `columns` and `rows` are
    /// empty whenever it is present.
    pub unavailable: Option<DatasetPreviewUnavailable>,
    /// False when the file is above the recipe's parse ceiling. The surface
    /// disables the recipe controls rather than accepting steps it cannot
    /// apply to the uploaded file.
    pub recipe_available: bool,
}

/// What a cast step parses a column into.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DatasetCastType {
    Text,
    Number,
    Integer,
    Boolean,
    Date,
}

/// How a filter step compares.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DatasetFilterOp {
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    AtLeast,
    AtMost,
    Contains,
    IsNull,
    IsNotNull,
}

impl DatasetFilterOp {
    /// Whether the comparison reads a value. The null tests do not, and a
    /// value supplied with one is a caller mistake rather than a no-op.
    #[must_use]
    pub const fn takes_value(self) -> bool {
        !matches!(self, Self::IsNull | Self::IsNotNull)
    }
}

/// One step of a recipe.
///
/// Named for what a person is doing to the data, not for a SQL clause: this is
/// a fixup for a spreadsheet that is nearly right, not a query language.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DatasetRecipeStep {
    Filter {
        column: String,
        op: DatasetFilterOp,
        /// Absent for the null tests, required for every other comparison.
        value: Option<String>,
    },
    Sort {
        column: String,
        descending: bool,
    },
    Rename {
        column: String,
        new_name: String,
    },
    Cast {
        column: String,
        cast_to: DatasetCastType,
    },
    Trim {
        column: String,
    },
    Upper {
        column: String,
    },
    Lower {
        column: String,
    },
    Replace {
        column: String,
        find: String,
        replace_with: String,
    },
    Split {
        column: String,
        delimiter: String,
        parts: u32,
    },
    /// Drops rows that repeat an earlier row in full.
    Dedup,
    RemoveNulls {
        column: String,
    },
    FillNulls {
        column: String,
        value: String,
    },
}

/// An ordered recipe. Applied in order, because two steps that both touch one
/// column do not commute.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetRecipe {
    pub steps: Vec<DatasetRecipeStep>,
}

impl DatasetRecipe {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Everything the backend needs to turn a staged file into a dataset.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetUploadRequest {
    pub staged_id: StagedDatasetId,
    pub display_name: String,
    /// Omitted when the person did not choose, so DA applies its own default
    /// rather than this app asserting one on their behalf.
    pub worksheet_index: Option<u32>,
    pub header_row_index: Option<u32>,
    pub recipe: Option<DatasetRecipe>,
}

/** The exact transfer admitted by one confirmed dataset upload. */
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetUploadStartResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub upload_id: UploadId,
    pub queue: UploadQueueV1,
}

impl DatasetUploadStartResult {
    #[must_use]
    pub fn new(upload_id: UploadId, queue: UploadQueueV1) -> Self {
        Self {
            version: CONTRACT_VERSION,
            upload_id,
            queue,
        }
    }
}

/// What a dropped file became.
///
/// Emitted rather than returned, because a drop is not a call: the person drops
/// a file on the window and the backend decides what it is. Exactly one field
/// is set.
///
/// The surface shows this only while the datasets destination is the one on
/// screen. That is a rendering decision, not a security one — the backend
/// stages and validates a dropped path whatever the renderer believes is
/// active, so nothing the webview reports can widen what a drop may do.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetDropOutcome {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub staged: Option<StagedDatasetView>,
    pub error: Option<DatasetOperationError>,
}

impl DatasetDropOutcome {
    #[must_use]
    pub fn staged(staged: StagedDatasetView) -> Self {
        Self {
            version: CONTRACT_VERSION,
            staged: Some(staged),
            error: None,
        }
    }

    #[must_use]
    pub fn refused(error: DatasetOperationError) -> Self {
        Self {
            version: CONTRACT_VERSION,
            staged: None,
            error: Some(error),
        }
    }
}

/// Why a datasets operation failed.
///
/// DA's own message text is never forwarded, here or anywhere: it can carry a
/// presigned URL, a storage key, or a bucket name. The surface writes the copy
/// from the code, and an unrecognised server code arrives as
/// [`Self::ServerRefused`] rather than as a raw string a person cannot act on.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DatasetErrorCode {
    /// No session, or one that could not be renewed.
    AuthRequired,
    /// The tenant is not configured on this device.
    NotConfigured,
    /// An administrator deactivated the account's upload source.
    UploadsDisabled,
    /// A signed application update has closed work admission until restart.
    UpdatePending,
    /// No staged file of that id. One code for discarded, already uploaded,
    /// and never staged — telling them apart would let a caller learn which
    /// ids are real by asking.
    StageNotFound,
    /// The staging cap is reached. Discard something first.
    StageLimitReached,
    /// The person dismissed the file dialog. A successful call with nothing to
    /// show, reported as an error only because there is no view to return.
    NoFileChosen,
    /// The extension is outside the accepted set.
    UnsupportedFormat,
    /// The extension and the file's content disagree.
    ContentMismatch,
    /// A password-protected workbook. Nothing local can read it, so it is
    /// refused before the transfer rather than after DA rejects it.
    EncryptedFile,
    EmptyFile,
    /// Above the 2 GiB dataset ceiling.
    FileTooLarge,
    /// The file cannot be read from disk.
    Unreadable,
    /// The recipe is longer than the cap.
    RecipeTooLong,
    /// A step names a column the file does not have, or carries a parameter
    /// that cannot be read. [`DatasetOperationError::step_index`] says which.
    RecipeInvalidStep,
    /// The file is above the recipe's parse ceiling, so a recipe cannot be
    /// applied to it. It still uploads unshaped.
    RecipeFileTooLarge,
    /// No dataset of that id under the active account.
    NotFound,
    /// DA refused the operation. Covers an unrecognised server code, so a
    /// build that has not learned a new one still says something true.
    ServerRefused,
    /// The request did not complete.
    Network,
    /// A local write failed — the save destination, or the staging directory.
    Storage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DatasetOperationError {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub code: DatasetErrorCode,
    /// Which recipe step failed, counting from zero. Present only with
    /// [`DatasetErrorCode::RecipeInvalidStep`]: a step that is silently
    /// skipped uploads data that differs from the reviewed preview, so the
    /// position travels with the refusal.
    pub step_index: Option<u32>,
}

impl DatasetOperationError {
    #[must_use]
    pub fn new(code: DatasetErrorCode) -> Self {
        Self {
            version: CONTRACT_VERSION,
            code,
            step_index: None,
        }
    }

    #[must_use]
    pub fn invalid_step(step_index: u32) -> Self {
        Self {
            version: CONTRACT_VERSION,
            code: DatasetErrorCode::RecipeInvalidStep,
            step_index: Some(step_index),
        }
    }
}
