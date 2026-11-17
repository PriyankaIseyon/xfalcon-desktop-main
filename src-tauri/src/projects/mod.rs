//! Project identity, trust, and path containment.
//!
//! A *project* is a directory on disk the account has explicitly chosen to
//! trust (via [`trust::ProjectsService::create`], backed by a Tauri
//! directory picker at the command layer). Once trusted, all filesystem
//! access to that directory's contents goes through
//! [`domain::TrustedRoot::resolve_existing`]/[`domain::TrustedRoot::resolve_new`]
//! -- a capability-scoped (`cap-std`) containment API that revalidates the
//! root's stable platform identity before every decision and rejects `..`
//! traversal, absolute-path injection, symlink/junction/reparse-point
//! escapes, and platform-unsafe path forms. No other module reaches into a
//! trusted project's filesystem contents by any other route.
//!
//! See `docs/decisions.md` (Task 4)
//! and `docs/decisions.md`.
//!
//! Task 5 (`index`, `read`, `search`, `sensitive`) builds on top of that
//! containment API: `read`/`index` are the only modules that touch a
//! trusted project's file *contents* (through
//! [`domain::TrustedRoot::open_file`]/[`domain::TrustedRoot::file_metadata`]/
//! [`domain::TrustedRoot::list_entries`]), `search` only ever reads from
//! `index`'s encrypted local store, and `sensitive` is consulted by both
//! `read` and `search` before any content crosses the default-deny
//! boundary. See
//! `docs/decisions.md`.
//!
//! `commands` is the user-facing half of that same read surface: two Tauri
//! commands that project `read`'s outcomes onto
//! `xfalcon_contracts::projects` and add no policy of their own, so a person
//! browsing a project and an agent reading it resolve, contain, bound, and
//! classify identically (see that module's doc comment).

mod backup;
mod commands;
mod domain;
mod edit;
mod index;
mod office_convert;
mod office_preview;
mod patch;
mod path_policy;
mod platform;
mod read;
mod revision_diff;
pub mod schema;
mod search;
mod sensitive;
mod table;
mod trust;
mod viewer;
mod writer;

#[cfg(test)]
mod tests;

pub use backup::{
    BackupCapture, BackupRevision, LoadedBackup, ShadowBackupStore, ShadowBackupStoreError,
};
pub use commands::{
    project_document_read, project_file_read_bytes, project_file_stat, project_list_directory,
    project_preview_register, project_schema_summary, project_table_rows, ProjectFileError,
};
pub(crate) use commands::{read_scope_id, sensitive_class_projection};
pub use domain::{
    ContainmentError, ProjectId, ProjectIdParseError, ResolvedProjectPath, TrustedRoot,
    MAX_DIRECTORY_ENTRIES,
};
pub use edit::{EditError, EditOperation, TextEdit, MAX_ANCHOR_BYTES, MAX_EDITS_PER_OPERATION};
pub use index::{
    run_index_pass, IndexError, IndexPassOutcome, IndexPassRequest, IndexPolicy, IndexStartError,
    IndexStore, IndexWaiter, IndexedFileRecord, ProjectIndexer, INDEX_BATCH_SIZE,
    MAX_INDEXED_FILES_PER_PASS, MAX_INDEX_WORKER_THREADS,
};
pub(crate) use office_preview::OfficePreviewJobs;
pub use office_preview::{
    project_office_preview, project_office_preview_cancel, project_office_save_original,
    project_office_save_pdf, report_office_preview,
};
pub use patch::{
    hash_bytes, keep_line_endings, ContentHash, MutationPlan, MutationRiskTier, PatchError,
    PatchOperation, PlanId, PlanIdParseError, DEFAULT_PLAN_TTL, MAX_DIFF_BYTES,
    MAX_EDITABLE_FILE_BYTES, MAX_OPERATIONS_PER_PLAN, MAX_TOTAL_PLAN_BYTES,
    SECOND_CONFIRMATION_BYTE_THRESHOLD, SECOND_CONFIRMATION_PATH_THRESHOLD,
};
pub use path_policy::{PathPolicyError, RelativeProjectPath};
pub use platform::{PlatformError, RootIdentity};
pub use read::{
    list_directory, read_file, read_file_with_audit, ContentKind, DirectoryEntryView, EntryKind,
    LineRange, MetadataOnlyReason, ReadError, ReadFileOutcome, ReadFileRequest, TextContent,
    MAX_FILE_READ_BYTES, MAX_HASHED_FILE_BYTES, MAX_LINE_BYTES, MAX_LINE_RANGE_LINES, SNIFF_BYTES,
};
pub use revision_diff::{
    revision_diff, DiffTarget, RevisionDiff, RevisionDiffLimit, MAX_REVISION_DIFF_CHANGED_LINES,
    MAX_REVISION_DIFF_FILE_BYTES,
};
pub use search::{
    search_files, search_text, FileSearchMode, FileSearchOutcome, FileSearchResult, SearchError,
    SearchMode, TextMatch, TextSearchOutcome, MAX_LIVE_WALK_FILES, MAX_MATCHES_PER_FILE,
    MAX_MATCH_LINE_PREVIEW_BYTES, MAX_OFFICE_FILES_PER_SEARCH, MAX_OFFICE_SOURCE_BYTES_PER_SEARCH,
    MAX_PATTERN_PROGRAM_BYTES, MAX_SEARCH_BYTES_PER_FILE, MAX_SEARCH_FILES_SCANNED,
    MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_RESULTS,
};
pub use sensitive::{
    evaluate, SecretClass, SecretClassifier, SensitiveDecision, SensitiveError,
    SensitiveOverrideGrant, SensitiveOverrideRequest, SensitiveReadScope,
    MAX_CONFIGURED_SECRET_GLOBS,
};
pub use table::MAX_TABLE_WINDOW_ROWS;
pub use trust::{
    CancellationSignal, ProjectRecord, ProjectsService, ProjectsServiceError, TrustError,
    TrustStore, PROJECT_DATA_DIR,
};
pub use viewer::{MAX_PROJECT_PREVIEW_BYTES, MAX_READ_RANGE_BYTES, MAX_VIEWABLE_FILE_BYTES};
pub use writer::{ExecutionOutcome, WriterError, WriterExecutionFailure};
// `execute`/`ApprovalProof` are crate-internal only: the sole legitimate
// caller/constructor is `crate::approvals` (see `writer.rs`'s own doc
// comments on both) -- deliberately not part of this module's external
// public surface, matching `plan_already_applied` immediately below.
pub(crate) use writer::{execute, ApprovalProof};
// `plan_already_applied` is an internal reconciliation helper for
// `crate::approvals::service::ApprovalsService::resolve` (see its own doc
// comment) -- deliberately not part of this module's public surface.
pub(crate) use writer::plan_already_applied;
