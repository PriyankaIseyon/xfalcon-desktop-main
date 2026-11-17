//! Saving a dashboard into the project its conversation is bound to (spec:
//! reports, "A project conversation saves reports into its project folder").
//!
//! The renderer names a conversation, a message, and a document ordinal, and
//! [`resolve_document`] re-reads the bytes from the encrypted chat projection.
//! The bytes land in the project folder. The project is the conversation's own
//! binding, never a value the renderer supplies, so no caller can steer a report
//! into a folder the conversation does not belong to. The file name is derived
//! here from the report name, so no path crosses the boundary either.
//!
//! The write is one `Create` through [`ApprovalsService::save_for_account`] — the
//! path a person's own file-viewer save takes — so it is planned against the
//! current trusted root, journaled, audited, containment-checked again before
//! anything is touched, and atomic. `Create` refuses a target that exists when it
//! is applied. The writer checks that and then renames into place, and does not
//! hold the two together, so a file that another process creates in that gap can
//! still be replaced; the writer documents the same window. A free name is chosen
//! just before, so reaching it takes a concurrent writer to the same folder.

use std::collections::HashSet;
use std::io::Read;

use xfalcon_contracts::reports::{
    ProjectReportFile, ProjectReportSaveResult, ProjectReportSaveStatus, ReportErrorCode,
    ReportOperationError,
};
use xfalcon_contracts::ConversationId;

use crate::approvals::{ApprovalOutcome, ApprovalsService};
use crate::chat::repository::LocalChatProjection;
use crate::projects::{
    list_directory, ContainmentError, DirectoryEntryView, EditOperation, EntryKind, ProjectId,
    ProjectsService, ReadError, RelativeProjectPath, SecretClassifier, TrustedRoot,
    MAX_EDITABLE_FILE_BYTES,
};
use crate::security::account_cleanup::ValidatedAccountId;

use super::capture::{has_external_reference, resolve_document, ResolvedDocument};

/// The folder every project report is written into, at the project root.
pub const REPORTS_FOLDER: &str = "reports";

/// The largest document a project report may hold: the most the project write
/// path accepts in one file. Every report within it opens in the file view,
/// which previews HTML up to `crate::projects::MAX_PROJECT_PREVIEW_BYTES` and
/// shows a larger document as source.
pub const PROJECT_REPORT_MAX_BYTES: usize = MAX_EDITABLE_FILE_BYTES as usize;

/// A file-name stem this long is already past what a file tree shows; the cap
/// keeps `stem (999).html` comfortably inside the 255-byte component limit.
const MAX_STEM_BYTES: usize = 120;
/// How many numbered siblings a name may have before the save gives up.
const MAX_NAME_SUFFIX: u32 = 999;
/// How much of a same-size candidate is read at a time when comparing it.
const COMPARE_CHUNK_BYTES: usize = 64 * 1024;
/// The naming dialog's bound on a report name, in characters. DA bounds its own
/// report names; a file name has no server to do it, so this is the only check.
const MAX_TITLE_CHARS: usize = 200;
/// The stem used when the report name leaves nothing a platform accepts.
const FALLBACK_STEM: &str = "Report";
const HTML_EXTENSION: &str = ".html";

pub struct ProjectReportRequest<'a> {
    pub conversation_id: ConversationId,
    pub message_id: &'a str,
    pub html_index: u32,
    pub title: &'a str,
    pub allow_duplicate: bool,
}

/// Saves the `html_index`-th HTML document of one assistant message into the
/// project `request.conversation_id` is bound to, under `account`.
pub async fn save_to_project(
    chat: &dyn LocalChatProjection,
    projects: &ProjectsService,
    approvals: &ApprovalsService,
    account: &ValidatedAccountId,
    request: ProjectReportRequest<'_>,
) -> Result<ProjectReportSaveResult, ReportOperationError> {
    let title = project_report_title(request.title)?;
    let Some(project_id) = projects
        .project_for_conversation(account, &request.conversation_id)
        .map_err(|_| error(ReportErrorCode::StorageUnavailable))?
    else {
        return Ok(ProjectReportSaveResult::refused(
            ProjectReportSaveStatus::NotInProject,
        ));
    };

    let html = match resolve_document(
        chat,
        request.conversation_id,
        request.message_id,
        request.html_index,
    )
    .await
    .map_err(|_| error(ReportErrorCode::StorageUnavailable))?
    {
        ResolvedDocument::Found(html) => html,
        ResolvedDocument::NotFound => {
            return Ok(ProjectReportSaveResult::refused(
                ProjectReportSaveStatus::NotFound,
            ))
        }
        ResolvedDocument::NotHtml => {
            return Ok(ProjectReportSaveResult::refused(
                ProjectReportSaveStatus::NotHtml,
            ))
        }
    };
    write_project_report(
        projects,
        approvals,
        account,
        project_id,
        title,
        html,
        request.allow_duplicate,
    )
    .await
}

/// The name a project report is saved under, judged: trimmed, required, bounded.
pub(crate) fn project_report_title(title: &str) -> Result<&str, ReportOperationError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return Err(error(ReportErrorCode::InvalidTitle));
    }
    Ok(title)
}

/// Writes one report document into `project_id`'s `reports/` folder.
///
/// The one function both ways into a project go through: the chat card's save
/// (the document re-read from the stored message) and the library's save (the
/// assembled page DA holds). They differ only in where the bytes come from, so
/// the name derivation, duplicate rule, size ceiling, and approvals write cannot
/// drift apart. `title` is already judged by [`project_report_title`].
pub(crate) async fn write_project_report(
    projects: &ProjectsService,
    approvals: &ApprovalsService,
    account: &ValidatedAccountId,
    project_id: ProjectId,
    title: &str,
    html: String,
    allow_duplicate: bool,
) -> Result<ProjectReportSaveResult, ReportOperationError> {
    if let Some(refused) = size_refusal(html.len()) {
        return Ok(refused);
    }

    let root = projects
        .trusted_root(account, project_id)
        .map_err(|_| error(ReportErrorCode::ProjectUnavailable))?;
    // Listing the folder and comparing candidates can read megabytes, so it
    // runs on the blocking pool, as a viewer range read does.
    let title = title.to_owned();
    let (html, placement) = tokio::task::spawn_blocking(move || {
        let placement = place_report(&root, &title, html.as_bytes(), allow_duplicate);
        (html, placement)
    })
    .await
    .map_err(|_| error(ReportErrorCode::ProjectUnavailable))?;
    let relative = match placement? {
        Placement::Duplicate(existing) => {
            return Ok(ProjectReportSaveResult::duplicate(report_file(
                project_id, existing,
            )))
        }
        Placement::New(relative) => relative,
    };

    let external_reference = has_external_reference(&html);
    let path = RelativeProjectPath::new(&relative)
        .map_err(|_| error(ReportErrorCode::ProjectUnavailable))?;
    let outcome = approvals
        .save_for_account(
            account,
            project_id,
            vec![EditOperation::Create {
                path,
                content: html.into_bytes(),
            }],
        )
        .await
        .map_err(|failure| {
            // The Display strings of `approvals` and `projects` are already
            // redacted: no absolute path and no file content.
            tracing::warn!(error = %failure, "reports: project report write refused");
            error(ReportErrorCode::ProjectUnavailable)
        })?;
    // As `project_save_file`: a denial is unreachable for a plan this path
    // approves itself, and a partial run of one operation is a contradiction.
    // Neither is reduced to success.
    match outcome {
        ApprovalOutcome::Executed(execution)
            if execution.operations_completed == execution.operations_total => {}
        ApprovalOutcome::Executed(_) | ApprovalOutcome::Denied => {
            return Err(error(ReportErrorCode::ProjectUnavailable));
        }
    }

    Ok(ProjectReportSaveResult::saved(
        report_file(project_id, relative),
        external_reference,
    ))
}

/// The answer for a document too large for the project write path, if it is.
fn size_refusal(byte_length: usize) -> Option<ProjectReportSaveResult> {
    (byte_length > PROJECT_REPORT_MAX_BYTES)
        .then(|| ProjectReportSaveResult::too_large(PROJECT_REPORT_MAX_BYTES as u64))
}

fn error(code: ReportErrorCode) -> ReportOperationError {
    ReportOperationError::new(code)
}

fn report_file(project_id: ProjectId, relative_path: String) -> ProjectReportFile {
    ProjectReportFile::new(project_id.into(), relative_path)
}

/// Where a document goes in the `reports` folder.
enum Placement {
    /// A report file already holds exactly these bytes.
    Duplicate(String),
    /// A project-relative path no file answers to yet.
    New(String),
}

/// Looks in the `reports` folder once: for a file already holding `bytes`,
/// unless a copy was asked for, and otherwise for the name the new file takes.
fn place_report(
    root: &TrustedRoot,
    title: &str,
    bytes: &[u8],
    allow_duplicate: bool,
) -> Result<Placement, ReportOperationError> {
    let entries = reports_folder_entries(root)?;
    if !allow_duplicate {
        if let Some(existing) = identical_report(root, &entries, bytes)? {
            return Ok(Placement::Duplicate(existing));
        }
    }
    unused_report_path(root, title, &entries).map(Placement::New)
}

/// The `reports` folder's immediate children. A folder that does not exist yet
/// has none: the first save creates it under containment. A `reports` that is
/// not a readable folder fails the save rather than being written around.
fn reports_folder_entries(
    root: &TrustedRoot,
) -> Result<Vec<DirectoryEntryView>, ReportOperationError> {
    let folder = RelativeProjectPath::new(REPORTS_FOLDER)
        .map_err(|_| error(ReportErrorCode::ProjectUnavailable))?;
    match list_directory(root, &SecretClassifier::built_in(), Some(folder)) {
        Ok(entries) => Ok(entries),
        Err(ReadError::Containment(ContainmentError::NotFound)) => Ok(Vec::new()),
        Err(_) => Err(error(ReportErrorCode::ProjectUnavailable)),
    }
}

/// The project-relative path of a report file holding exactly `bytes`, if any.
///
/// Only a regular, non-sensitive `.html` file of the same size is opened, and
/// it is compared a chunk at a time, stopping at the first difference, so a
/// crowded folder costs one listing and no whole-file buffer. A candidate that
/// vanished or cannot be read is not proven a duplicate and is passed over; a
/// containment failure still fails the save.
fn identical_report(
    root: &TrustedRoot,
    entries: &[DirectoryEntryView],
    bytes: &[u8],
) -> Result<Option<String>, ReportOperationError> {
    let mut chunk = Vec::new();
    for entry in entries {
        if entry.kind != EntryKind::File
            || entry.is_sensitive
            || entry.size_bytes != bytes.len() as u64
            || !has_html_extension(&entry.name)
        {
            continue;
        }
        let relative = format!("{REPORTS_FOLDER}/{}", entry.name);
        let Ok(path) = RelativeProjectPath::new(&relative) else {
            continue;
        };
        let file = match root
            .resolve_existing(path)
            .and_then(|resolved| root.open_file(&resolved))
        {
            Ok(file) => file,
            Err(ContainmentError::NotFound | ContainmentError::Io(_)) => continue,
            Err(_) => return Err(error(ReportErrorCode::ProjectUnavailable)),
        };
        if chunk.is_empty() {
            chunk = vec![0; COMPARE_CHUNK_BYTES];
        }
        if holds_exactly(file, bytes, &mut chunk).unwrap_or(false) {
            return Ok(Some(relative));
        }
    }
    Ok(None)
}

/// Whether `reader` yields exactly `expected` and then ends.
fn holds_exactly(
    mut reader: impl Read,
    expected: &[u8],
    chunk: &mut [u8],
) -> std::io::Result<bool> {
    let mut remaining = expected;
    loop {
        let read = match reader.read(chunk) {
            Ok(0) => return Ok(remaining.is_empty()),
            Ok(read) => read,
            Err(failure) if failure.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(failure) => return Err(failure),
        };
        if read > remaining.len() || chunk[..read] != remaining[..read] {
            return Ok(false);
        }
        remaining = &remaining[read..];
    }
}

fn has_html_extension(name: &str) -> bool {
    name.len() > HTML_EXTENSION.len()
        && name.is_char_boundary(name.len() - HTML_EXTENSION.len())
        && name[name.len() - HTML_EXTENSION.len()..].eq_ignore_ascii_case(HTML_EXTENSION)
}

/// `reports/<stem>.html`, or the first numbered sibling no file answers to.
///
/// The listing is compared without regard to case, because the folders this
/// app runs on (NTFS, APFS) are case-insensitive: `pipeline.html` beside
/// `Pipeline.html` would be the same file there, and a folder synced from a
/// case-sensitive disk must not end up holding both. A name the listing leaves
/// free is then asked of the folder itself, which applies its own case and
/// Unicode-normalization rules, so `Create` meets an existing file only when
/// one appears in the moment between this check and the write.
fn unused_report_path(
    root: &TrustedRoot,
    title: &str,
    entries: &[DirectoryEntryView],
) -> Result<String, ReportOperationError> {
    let taken: HashSet<String> = entries
        .iter()
        .map(|entry| entry.name.to_lowercase())
        .collect();
    let stem = file_stem(title);
    for suffix in 1..=MAX_NAME_SUFFIX {
        let name = if suffix == 1 {
            format!("{stem}{HTML_EXTENSION}")
        } else {
            format!("{stem} ({suffix}){HTML_EXTENSION}")
        };
        if taken.contains(&name.to_lowercase()) {
            continue;
        }
        let relative = format!("{REPORTS_FOLDER}/{name}");
        if is_free(root, &relative)? {
            return Ok(relative);
        }
    }
    Err(error(ReportErrorCode::ProjectUnavailable))
}

/// Whether nothing in the project answers to `relative` right now. A link
/// answering to it occupies the name like any file.
fn is_free(root: &TrustedRoot, relative: &str) -> Result<bool, ReportOperationError> {
    let path = RelativeProjectPath::new(relative)
        .map_err(|_| error(ReportErrorCode::ProjectUnavailable))?;
    match root
        .resolve_new(path)
        .and_then(|resolved| root.file_metadata(&resolved))
    {
        Ok(_) | Err(ContainmentError::LinkOrReparse) => Ok(false),
        Err(ContainmentError::NotFound) => Ok(true),
        Err(_) => Err(error(ReportErrorCode::ProjectUnavailable)),
    }
}

/// A file-name stem every supported platform accepts, derived from a report
/// name.
///
/// Characters Windows refuses in a name (`<>:"/\|?*`) and control characters
/// become `-`; runs of whitespace become one space. Leading dots would hide the
/// file on Unix, and trailing dots or spaces are refused on Windows, so both
/// go, with any space the leading dots leave exposed. The stem is capped at
/// [`MAX_STEM_BYTES`] on a character boundary. A stem the project path policy
/// still refuses — a reserved device name such as `CON` — falls back to
/// [`FALLBACK_STEM`].
pub(crate) fn file_stem(title: &str) -> String {
    let mut stem = String::new();
    let mut pending_space = false;
    for character in title.chars() {
        if character.is_whitespace() {
            pending_space = !stem.is_empty();
            continue;
        }
        if pending_space {
            stem.push(' ');
            pending_space = false;
        }
        let safe = if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
            '-'
        } else {
            character
        };
        if stem.len() + safe.len_utf8() > MAX_STEM_BYTES {
            break;
        }
        stem.push(safe);
    }
    let trimmed = stem
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' '])
        .to_owned();
    let candidate = format!("{REPORTS_FOLDER}/{trimmed}{HTML_EXTENSION}");
    if trimmed.is_empty() || RelativeProjectPath::new(&candidate).is_err() {
        FALLBACK_STEM.to_owned()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::str::FromStr;
    use std::sync::Arc;

    use tempfile::{tempdir, TempDir};
    use uuid::Uuid;
    use xfalcon_contracts::reports::{ProjectReportSaveStatus, ReportErrorCode};
    use xfalcon_contracts::ConversationId;

    use super::{
        file_stem, holds_exactly, project_report_title, save_to_project, size_refusal,
        write_project_report, ProjectReportRequest, PROJECT_REPORT_MAX_BYTES, REPORTS_FOLDER,
    };
    use crate::approvals::ApprovalsService;
    use crate::chat::domain::{
        ChatProjectionEvent, ChatSnapshot, Conversation, Message, MessageContent, MessageId,
        MessageRole, SyncCursor,
    };
    use crate::chat::local_projection::EncryptedChatProjection;
    use crate::chat::repository::LocalChatProjection;
    use crate::persistence::{InMemorySecretStore, SecretStore};
    use crate::projects::{ProjectId, ProjectsService};
    use crate::security::account_cleanup::{CurrentAccountScope, ValidatedAccountId};

    const ACCOUNT: &str = "account-1";

    /// A document over the 100-byte span floor the scanner requires.
    fn document(marker: &str) -> String {
        format!(
            "<!doctype html><html><head><title>{marker}</title></head><body>\
             <div id=\"{marker}\">Bookings by region for the third quarter, charted</div>\
             </body></html>"
        )
    }

    fn answer(documents: &[&str]) -> String {
        documents
            .iter()
            .map(|document| format!("Here is the dashboard.\n\n```html\n{document}\n```\n"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A real chat projection, project registry, and approvals service over one
    /// temporary app root and one bound account — the same three the command
    /// composes at runtime.
    struct Fixture {
        _app: TempDir,
        project_root: TempDir,
        chat: EncryptedChatProjection,
        projects: ProjectsService,
        approvals: ApprovalsService,
        account: ValidatedAccountId,
        project_id: ProjectId,
        conversation_id: ConversationId,
        message_id: String,
    }

    impl Fixture {
        async fn new(text: &str, bound: bool) -> Self {
            let app = tempdir().expect("app root");
            let project_root = tempdir().expect("project root");
            let secrets = Arc::new(InMemorySecretStore::default());
            secrets.put_key(ACCOUNT, [0x41; 32]).expect("account key");
            let secrets: Arc<dyn SecretStore> = secrets;
            let scope = Arc::new(CurrentAccountScope::default());
            scope.bind_native(ACCOUNT).expect("bind account");
            let account = ValidatedAccountId::parse(ACCOUNT).expect("account id");

            let projects =
                ProjectsService::new(app.path().to_path_buf(), scope.clone(), secrets.clone());
            let approvals =
                ApprovalsService::new(app.path().to_path_buf(), scope.clone(), secrets.clone());
            let project_id = ProjectId::from_str(
                projects
                    .create(&project_root.path().to_string_lossy(), None)
                    .await
                    .expect("trust project")
                    .id
                    .as_str(),
            )
            .expect("project id");

            let conversation_id = ConversationId::from(Uuid::new_v4().to_string());
            if bound {
                projects
                    .bind_conversation(&account, &conversation_id, project_id)
                    .expect("bind conversation");
            }
            let message_id = MessageId::new();
            let message = Message::try_new(
                message_id,
                conversation_id.clone(),
                1,
                MessageRole::Assistant,
                vec![MessageContent::text(text)],
                vec![],
            )
            .expect("message")
            .into_completed();
            let chat = EncryptedChatProjection::open(
                app.path().join("chat.sqlite3"),
                secrets.clone(),
                account.clone(),
            )
            .expect("open chat projection");
            chat.apply(ChatProjectionEvent::ReplaceSnapshot(ChatSnapshot::new(
                Conversation::new(conversation_id.clone(), "Dashboards"),
                vec![message],
                None,
                vec![],
                SyncCursor::new(1),
            )))
            .await
            .expect("persist snapshot");

            Self {
                _app: app,
                project_root,
                chat,
                projects,
                approvals,
                account,
                project_id,
                conversation_id,
                message_id: message_id.as_uuid().to_string(),
            }
        }

        async fn save(
            &self,
            title: &str,
            html_index: u32,
            allow_duplicate: bool,
        ) -> Result<xfalcon_contracts::reports::ProjectReportSaveResult, ReportErrorCode> {
            save_to_project(
                &self.chat,
                &self.projects,
                &self.approvals,
                &self.account,
                ProjectReportRequest {
                    conversation_id: self.conversation_id.clone(),
                    message_id: &self.message_id,
                    html_index,
                    title,
                    allow_duplicate,
                },
            )
            .await
            .map_err(|error| error.code)
        }

        /// What the library's "Save to project" does: the same writer, fed the
        /// page DA assembled instead of a document re-read from a message.
        async fn write_library(
            &self,
            project_id: ProjectId,
            title: &str,
            html: &str,
            allow_duplicate: bool,
        ) -> Result<xfalcon_contracts::reports::ProjectReportSaveResult, ReportErrorCode> {
            write_project_report(
                &self.projects,
                &self.approvals,
                &self.account,
                project_id,
                title,
                html.to_owned(),
                allow_duplicate,
            )
            .await
            .map_err(|error| error.code)
        }

        fn reports_folder(&self) -> std::path::PathBuf {
            self.project_root.path().join(REPORTS_FOLDER)
        }

        fn report_names(&self) -> Vec<String> {
            let Ok(entries) = fs::read_dir(self.reports_folder()) else {
                return Vec::new();
            };
            let mut names: Vec<String> = entries
                .map(|entry| {
                    entry
                        .expect("entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.sort();
            names
        }
    }

    #[tokio::test]
    async fn the_persisted_document_is_written_into_the_projects_reports_folder() {
        let html = document("alpha");
        let fixture = Fixture::new(&answer(&[&html]), true).await;

        let result = fixture.save("  Q3 revenue ", 0, false).await.expect("save");

        assert_eq!(result.status, ProjectReportSaveStatus::Saved);
        let file = result.file.expect("the written file");
        assert_eq!(file.relative_path, "reports/Q3 revenue.html");
        assert_eq!(file.project_id.as_str(), fixture.project_id.to_string());
        assert!(!result.external_reference);
        // The bytes are the document as persisted, created with the folder.
        let written =
            fs::read_to_string(fixture.reports_folder().join("Q3 revenue.html")).expect("read");
        assert_eq!(written, html);
    }

    #[tokio::test]
    async fn the_ordinal_selects_which_document_of_the_message_is_saved() {
        let (first, second) = (document("first"), document("second"));
        let fixture = Fixture::new(&answer(&[&first, &second]), true).await;

        fixture.save("Second", 1, false).await.expect("save");

        let written =
            fs::read_to_string(fixture.reports_folder().join("Second.html")).expect("read");
        assert_eq!(written, second);
    }

    #[tokio::test]
    async fn an_unbound_conversation_is_refused_and_nothing_is_written() {
        let fixture = Fixture::new(&answer(&[&document("alpha")]), false).await;

        let result = fixture.save("Q3 revenue", 0, false).await.expect("save");

        assert_eq!(result.status, ProjectReportSaveStatus::NotInProject);
        assert_eq!(result.file, None);
        assert!(!fixture.reports_folder().exists());
    }

    #[tokio::test]
    async fn a_missing_message_or_document_is_an_outcome_and_writes_nothing() {
        let mut fixture = Fixture::new(&answer(&[&document("alpha")]), true).await;

        let past_the_end = fixture.save("Q3", 1, false).await.expect("save");
        assert_eq!(past_the_end.status, ProjectReportSaveStatus::NotHtml);

        fixture.message_id = Uuid::new_v4().to_string();
        let unknown = fixture.save("Q3", 0, false).await.expect("save");
        assert_eq!(unknown.status, ProjectReportSaveStatus::NotFound);

        assert!(fixture.report_names().is_empty());
    }

    #[tokio::test]
    async fn a_name_in_use_gets_a_numbered_sibling_and_the_first_file_is_untouched() {
        let html = document("alpha");
        let fixture = Fixture::new(&answer(&[&html]), true).await;
        fs::create_dir(fixture.reports_folder()).expect("reports folder");
        fs::write(
            fixture.reports_folder().join("Pipeline.html"),
            "someone else's",
        )
        .expect("seed");

        let result = fixture.save("pipeline", 0, false).await.expect("save");

        assert_eq!(
            result.file.expect("file").relative_path,
            "reports/pipeline (2).html",
            "a case-only difference is the same file on NTFS and APFS"
        );
        assert_eq!(
            fs::read_to_string(fixture.reports_folder().join("Pipeline.html")).expect("read"),
            "someone else's"
        );
    }

    #[tokio::test]
    async fn identical_bytes_are_a_duplicate_and_a_copy_is_a_second_file() {
        let fixture = Fixture::new(&answer(&[&document("alpha")]), true).await;
        fixture
            .save("Q3 revenue", 0, false)
            .await
            .expect("first save");

        let again = fixture.save("Another name", 0, false).await.expect("save");
        assert_eq!(again.status, ProjectReportSaveStatus::Duplicate);
        assert_eq!(
            again.file.expect("existing file").relative_path,
            "reports/Q3 revenue.html"
        );
        assert_eq!(fixture.report_names(), vec!["Q3 revenue.html"]);

        let copy = fixture.save("Q3 revenue", 0, true).await.expect("copy");
        assert_eq!(copy.status, ProjectReportSaveStatus::Saved);
        assert_eq!(
            copy.file.expect("copy file").relative_path,
            "reports/Q3 revenue (2).html"
        );
        assert_eq!(
            fixture.report_names(),
            vec!["Q3 revenue (2).html", "Q3 revenue.html"]
        );
    }

    #[tokio::test]
    async fn a_same_size_file_with_other_bytes_is_not_a_duplicate() {
        let html = document("alpha");
        let fixture = Fixture::new(&answer(&[&html]), true).await;
        fs::create_dir(fixture.reports_folder()).expect("reports folder");
        let lookalike = html.replacen("alpha", "gamma", 1);
        assert_eq!(lookalike.len(), html.len());
        fs::write(fixture.reports_folder().join("Lookalike.html"), &lookalike).expect("seed");

        let result = fixture.save("Q3", 0, false).await.expect("save");

        assert_eq!(result.status, ProjectReportSaveStatus::Saved);
        assert_eq!(result.file.expect("file").relative_path, "reports/Q3.html");
    }

    #[test]
    fn a_comparison_reads_in_chunks_and_needs_the_exact_length() {
        // A chunk that does not divide the length, so the last read is short.
        fn compare(candidate: &[u8]) -> bool {
            holds_exactly(candidate, b"0123456789abcdef", &mut [0; 5]).expect("in-memory read")
        }
        assert!(compare(b"0123456789abcdef"));
        assert!(!compare(b"0123456789abcdeX"));
        assert!(!compare(b"X123456789abcdef"));
        assert!(!compare(b"0123456789abcde"));
        assert!(!compare(b"0123456789abcdef0"));
    }

    #[test]
    fn the_project_ceiling_is_the_write_path_bound_and_refuses_one_byte_past_it() {
        assert_eq!(
            PROJECT_REPORT_MAX_BYTES,
            crate::projects::MAX_EDITABLE_FILE_BYTES as usize
        );
        assert_eq!(size_refusal(PROJECT_REPORT_MAX_BYTES), None);
        let refused = size_refusal(PROJECT_REPORT_MAX_BYTES + 1).expect("refused");
        assert_eq!(refused.status, ProjectReportSaveStatus::TooLarge);
        assert_eq!(refused.limit, Some(PROJECT_REPORT_MAX_BYTES as u64));
        assert_eq!(refused.file, None);
    }

    #[tokio::test]
    async fn an_unusable_name_is_refused_before_anything_is_read() {
        let fixture = Fixture::new(&answer(&[&document("alpha")]), true).await;

        assert_eq!(
            fixture.save("   ", 0, false).await.err(),
            Some(ReportErrorCode::InvalidTitle)
        );
        assert_eq!(
            fixture.save(&"n".repeat(201), 0, false).await.err(),
            Some(ReportErrorCode::InvalidTitle)
        );
        assert!(fixture.report_names().is_empty());
    }

    #[tokio::test]
    async fn a_reports_path_that_is_not_a_folder_fails_the_save_and_is_left_alone() {
        let fixture = Fixture::new(&answer(&[&document("alpha")]), true).await;
        fs::write(fixture.reports_folder(), "notes").expect("a file named reports");

        assert_eq!(
            fixture.save("Q3", 0, false).await.err(),
            Some(ReportErrorCode::ProjectUnavailable)
        );
        assert_eq!(
            fs::read_to_string(fixture.reports_folder()).expect("read"),
            "notes"
        );
    }

    #[test]
    fn a_report_name_becomes_a_file_name_every_platform_accepts() {
        assert_eq!(file_stem("Revenue: EMEA/APAC"), "Revenue- EMEA-APAC");
        assert_eq!(file_stem("a\tb\n\nc"), "a b c");
        assert_eq!(file_stem("  ..hidden report.  "), "hidden report");
        assert_eq!(file_stem(". . notes"), "notes");
        assert_eq!(file_stem("<>|?*\"\\"), "-------");
        assert_eq!(file_stem("bell\u{7}"), "bell-");
        // Reserved device names, with or without an extension of their own.
        assert_eq!(file_stem("CON"), "Report");
        assert_eq!(file_stem("nul.summary"), "Report");
        assert_eq!(file_stem("..."), "Report");
        // Capped on a character boundary, never inside a multibyte character.
        let long = file_stem(&"é".repeat(200));
        assert!(long.len() <= 120);
        assert_eq!(long, "é".repeat(60));
    }

    #[tokio::test]
    async fn a_library_page_is_written_like_a_chat_save() {
        // No conversation is involved: the project is the one the reader picked.
        let fixture = Fixture::new(&answer(&[&document("unused")]), false).await;
        let page = document("assembled");

        let result = fixture
            .write_library(fixture.project_id, "Weekly review", &page, false)
            .await
            .expect("save");

        assert_eq!(result.status, ProjectReportSaveStatus::Saved);
        assert_eq!(
            result.file.expect("file").relative_path,
            "reports/Weekly review.html"
        );
        assert_eq!(
            fs::read_to_string(fixture.reports_folder().join("Weekly review.html")).expect("read"),
            page
        );
    }

    #[tokio::test]
    async fn a_library_copy_follows_the_duplicate_and_numbering_rules() {
        let fixture = Fixture::new(&answer(&[&document("unused")]), false).await;
        let page = document("assembled");
        fixture
            .write_library(fixture.project_id, "Weekly review", &page, false)
            .await
            .expect("first");

        // The same bytes again is a duplicate, and nothing new is written.
        let again = fixture
            .write_library(fixture.project_id, "Weekly review", &page, false)
            .await
            .expect("second");
        assert_eq!(again.status, ProjectReportSaveStatus::Duplicate);
        assert_eq!(fixture.report_names(), ["Weekly review.html"]);

        // A copy on purpose, and a refreshed page of the same name, each get a
        // numbered sibling; the first file is never replaced.
        fixture
            .write_library(fixture.project_id, "Weekly review", &page, true)
            .await
            .expect("copy");
        fixture
            .write_library(
                fixture.project_id,
                "Weekly review",
                &document("refreshed"),
                false,
            )
            .await
            .expect("refreshed");
        assert_eq!(
            fixture.report_names(),
            [
                "Weekly review (2).html",
                "Weekly review (3).html",
                "Weekly review.html"
            ]
        );
        assert_eq!(
            fs::read_to_string(fixture.reports_folder().join("Weekly review.html")).expect("read"),
            page
        );
    }

    #[tokio::test]
    async fn a_project_the_account_does_not_hold_is_unavailable_and_nothing_is_written() {
        let fixture = Fixture::new(&answer(&[&document("unused")]), false).await;
        let stranger = ProjectId::from_str(&Uuid::new_v4().to_string()).expect("project id");

        let error = fixture
            .write_library(stranger, "Weekly review", &document("assembled"), false)
            .await
            .expect_err("a project this account never trusted");

        assert_eq!(error, ReportErrorCode::ProjectUnavailable);
        assert!(fixture.report_names().is_empty());
    }

    #[tokio::test]
    async fn a_library_page_over_the_project_ceiling_is_refused_with_the_ceiling() {
        let fixture = Fixture::new(&answer(&[&document("unused")]), false).await;
        let oversized = "x".repeat(PROJECT_REPORT_MAX_BYTES + 1);

        let result = fixture
            .write_library(fixture.project_id, "Huge", &oversized, false)
            .await
            .expect("an outcome, not an error");

        assert_eq!(result.status, ProjectReportSaveStatus::TooLarge);
        assert_eq!(result.limit, Some(PROJECT_REPORT_MAX_BYTES as u64));
        assert!(fixture.report_names().is_empty());
    }

    #[test]
    fn a_project_report_title_is_trimmed_required_and_bounded() {
        assert_eq!(project_report_title("  Q3  ").expect("trimmed"), "Q3");
        for bad in ["", "   ", &"t".repeat(201)] {
            assert_eq!(
                project_report_title(bad).expect_err("refused").code,
                ReportErrorCode::InvalidTitle,
                "{bad:?}"
            );
        }
        assert!(project_report_title(&"t".repeat(200)).is_ok());
    }
}
