//! Saved reports, as the Reports destination sees them.
//!
//! A saved report is a DA report kit: DA stores the dashboard's page together
//! with the dataset definitions that refresh it, and desktop only names it.
//! A report's HTML is deliberately absent from every view here, and the
//! document never crosses IPC either. Opening a report fetches the assembled
//! page from DA in the backend, registers it with the bounded preview-document
//! store, and returns only the content-addressed id
//! ([`ReportDocument::document_id`]), so the markup reaches the webview by
//! loading from the `xf-preview` origin under that scheme's own per-response
//! policy.
//!
//! Project saves are by identity for the same reason: the renderer names a
//! conversation, a message, and which HTML document inside it, and the backend
//! re-reads those bytes from the encrypted chat projection. Nothing here
//! carries markup, so there is nothing for a caller to substitute.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::projects::ProjectId;
use crate::{ContractVersion, ConversationId, CONTRACT_VERSION};

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct ReportId(String);

impl ReportId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ReportId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ReportId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Who a report is visible to. DA owns the answer; the surface only shows it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ReportScope {
    User,
    Global,
}

/// What a saved report is. A dashboard has HTML pages; a workbook or a
/// presentation is one Office file DA fills from the report's stored queries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ReportKind {
    Dashboard,
    Workbook,
    Presentation,
}

/// One saved report, without its document. Everything the list needs to name,
/// order, search, and open a report, and everything the surface needs to decide
/// which actions to offer.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: ReportId,
    pub kind: ReportKind,
    pub title: String,
    pub description: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// When DA stored the report's thumbnail image, or `None` when it holds none
    /// (the card then draws the report itself). A changed value is a new image,
    /// so it is the key a cached thumbnail is kept under.
    pub thumbnail_updated_at_ms: Option<i64>,
    /// The reader's own pin. Per viewer, so one user's pin never reorders
    /// another user's list.
    pub pinned: bool,
    /// The reader's own hide. A preference, not access control: a hidden report
    /// is still listed so it can be shown again, and still opens by id.
    pub hidden: bool,
    /// Total payload bytes DA holds for the report, shown as its size.
    pub byte_size: u64,
    pub scope: ReportScope,
    /// Whether the signed-in account created the report. Only the owner can
    /// rename or delete a user-scoped report.
    pub is_owner: bool,
    /// The creator's name when the report is shared, so a reader can tell whose
    /// report it is. Absent when DA does not expose one.
    pub owner_display_name: Option<String>,
    /// Whether the report can be re-run against its datasets. Set by DA when the
    /// report is saved, so a report that is not refreshable never has a refresh
    /// action.
    pub refreshable: bool,
    /// DA's answer for this viewer. The surface shows an action only when the
    /// matching flag is set, and DA still refuses it if the answer is stale.
    pub can_refresh: bool,
    pub can_manage: bool,
    pub can_delete: bool,
    pub page_count: u32,
    pub dataset_count: u32,
    /// The names of the data sources the report's datasets read, as DA lists
    /// those this reader can see. Empty for a report with none, or none visible.
    pub data_sources: Vec<String>,
    /// Absent once the conversation the report came from is deleted. The
    /// report itself outlives it.
    pub source_conversation_id: Option<ConversationId>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportListResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub reports: Vec<ReportView>,
    /// The project each linked report was created in, for the library's Project
    /// filter. Device-local: DA keeps no such fact, so a report made on another
    /// device, or before the link existed, is not listed here and reads as having
    /// no project. At most one entry per report.
    pub project_origins: Vec<ReportProjectOrigin>,
}

impl ReportListResult {
    pub fn new(reports: Vec<ReportView>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            reports,
            project_origins: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_project_origins(mut self, project_origins: Vec<ReportProjectOrigin>) -> Self {
        self.project_origins = project_origins;
        self
    }
}

/// The trusted project a saved report was created in. The name is the project's
/// current one, read when the list is, so renaming a project needs no write here.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportProjectOrigin {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub report_id: ReportId,
    pub project_id: String,
    pub project_name: String,
}

impl ReportProjectOrigin {
    pub fn new(report_id: ReportId, project_id: String, project_name: String) -> Self {
        Self {
            version: CONTRACT_VERSION,
            report_id,
            project_id,
            project_name,
        }
    }
}

/// When one dataset behind the opened page was last stored, and how many rows it
/// held. DA stamps both when it stores the data, so neither is the time the
/// report was saved or edited.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportDatasetFreshness {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub dataset_id: String,
    /// Absent when DA does not know: the dataset was never refreshed, so the page
    /// still shows the rows it was saved with.
    pub executed_at_ms: Option<i64>,
    pub row_count: Option<u64>,
}

/// An opened report: its identity, its name, and the preview-document id its
/// frame loads from. Never the markup — see the module doc.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportDocument {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: ReportId,
    pub title: String,
    pub description: Option<String>,
    /// The page the reader is viewing, by name. A multi-page report opens one
    /// page at a time.
    pub page: String,
    /// Every page name in the report, in order, so the surface can offer the
    /// rest without a second call.
    pub pages: Vec<String>,
    /// The id the `xf-preview` scheme serves this document under. In-memory
    /// and evictable, so it is resolved fresh on every open and never stored.
    pub document_id: String,
    /// Freshness of each dataset inlined into this page. Empty when the page
    /// has none (a static report) or DA sent none.
    pub datasets: Vec<ReportDatasetFreshness>,
}

/// The image format of a stored thumbnail. DA accepts only these two, and checks
/// that the bytes are one on upload.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ReportThumbnailFormat {
    Png,
    Webp,
}

/// A report's stored thumbnail: a still image of its first page, at most
/// 200 KiB, that DA captured when the report was saved or opened in a viewer
/// that takes one. The card shows it instead of loading the report itself.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportThumbnail {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: ReportId,
    pub format: ReportThumbnailFormat,
    /// The image bytes, standard base64 with padding.
    pub data_base64: String,
}

/// A report's first page opened to make its thumbnail: a preview id like
/// [`ReportDocument::document_id`], for a page that also carries DA's capture
/// script, and the nonce that script answers to.
///
/// The frame that loads it is never shown. The renderer connects to the script
/// with this nonce over a private `MessageChannel`, receives one PNG, and hands
/// it to `report_thumbnail_save`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportCaptureDocument {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: ReportId,
    pub document_id: String,
    pub nonce: String,
}

/// One dataset's outcome on refresh. A dataset that fails is reported here,
/// not as an operation failure, so the other datasets' rows still show.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportRefreshOutcome {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub dataset_id: String,
    pub ok: bool,
    /// Rows the dataset returned. Zero when it failed.
    pub rows: u64,
    /// How long DA spent on the dataset, in milliseconds.
    pub duration_ms: u64,
    /// The name of the data source DA ran it against, when DA says.
    pub data_source: Option<String>,
    /// What DA reported for a failed dataset, absent when `ok`.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportRefreshResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub report: ReportView,
    pub outcomes: Vec<ReportRefreshOutcome>,
}

/// What saving a dashboard into its conversation's project found. Every arm is
/// the outcome of a successful call.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProjectReportSaveStatus {
    Saved,
    /// A file in the project's `reports` folder already holds these bytes.
    /// [`ProjectReportSaveResult::file`] names it.
    Duplicate,
    TooLarge,
    /// The message does not exist, does not belong to the named conversation,
    /// or is not an assistant turn — one arm, so the difference is not an
    /// existence oracle.
    NotFound,
    /// The message holds no HTML document at that position.
    NotHtml,
    /// The conversation is not bound to a project on this device.
    NotInProject,
}

/// A report file inside a trusted project. Project-relative, like every path
/// the file view already shows: the folder's own location never crosses the
/// boundary.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectReportFile {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub project_id: ProjectId,
    pub relative_path: String,
}

impl ProjectReportFile {
    pub fn new(project_id: ProjectId, relative_path: String) -> Self {
        Self {
            version: CONTRACT_VERSION,
            project_id,
            relative_path,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectReportSaveResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub status: ProjectReportSaveStatus,
    /// The written file on `saved`, the existing one on `duplicate`, absent
    /// otherwise.
    pub file: Option<ProjectReportFile>,
    /// The byte ceiling on `too_large`; absent on every other status.
    pub limit: Option<u64>,
    /// Advisory. A project save writes the document verbatim, so a script or
    /// stylesheet it cannot load stays dead in the file.
    pub external_reference: bool,
}

impl ProjectReportSaveResult {
    pub fn saved(file: ProjectReportFile, external_reference: bool) -> Self {
        Self {
            version: CONTRACT_VERSION,
            status: ProjectReportSaveStatus::Saved,
            file: Some(file),
            limit: None,
            external_reference,
        }
    }

    pub fn duplicate(existing: ProjectReportFile) -> Self {
        Self {
            version: CONTRACT_VERSION,
            status: ProjectReportSaveStatus::Duplicate,
            file: Some(existing),
            limit: None,
            external_reference: false,
        }
    }

    pub fn too_large(limit: u64) -> Self {
        Self {
            version: CONTRACT_VERSION,
            status: ProjectReportSaveStatus::TooLarge,
            file: None,
            limit: Some(limit),
            external_reference: false,
        }
    }

    pub fn refused(status: ProjectReportSaveStatus) -> Self {
        Self {
            version: CONTRACT_VERSION,
            status,
            file: None,
            limit: None,
            external_reference: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ReportErrorCode {
    /// No report of that id under the active account. One code for a report
    /// that does not exist, one belonging to another account, and an id that is
    /// not well-formed — distinguishing them would let a caller learn which
    /// ids are real by asking.
    NotFound,
    /// The report library could not be reached or read: a network or server
    /// failure, an answer desktop could not use, or a sign-in the server no
    /// longer accepts.
    StorageUnavailable,
    /// The name or description was unusable: an empty name, a name with a control
    /// character or an invisible one (rename only: a project save turns those
    /// into spaces in the file name instead of refusing), or either past its
    /// bound. The caller sent something it can fix and resend.
    InvalidTitle,
    /// The conversation's project folder could not be reached, or refused the
    /// write. Nothing was written.
    ProjectUnavailable,
    /// The account lacks the grant or the ownership the action needs (refresh,
    /// rename, or delete). DA refused it.
    Forbidden,
    /// The report's assembled page is larger than the preview store will
    /// serve, so it cannot be opened. The page is never truncated.
    TooLargeToOpen,
    /// DA cannot refresh this report: it holds no stored queries, or DA refused
    /// the refresh as invalid. Nothing was changed.
    NotRefreshable,
    /// The report could not be written to the place the reader chose: the save
    /// dialog could not open, the destination could not be written, or the file
    /// was larger than an export accepts. Nothing was written there.
    ExportFailed,
    /// A thumbnail to store was not a PNG within DA's 200 KiB bound. Nothing
    /// was sent to DA.
    InvalidThumbnail,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReportOperationError {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub code: ReportErrorCode,
}

impl ReportOperationError {
    pub fn new(code: ReportErrorCode) -> Self {
        Self {
            version: CONTRACT_VERSION,
            code,
        }
    }
}
