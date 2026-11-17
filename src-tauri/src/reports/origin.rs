//! Which project a saved report was created in (spec: reports, "A report made in
//! a project chat is linked to its project").
//!
//! DA stores no such fact for a kit an assistant saves, so the device records it
//! when the save call returns: [`saved_kit_id`] reads the new kit's id out of
//! DA's answer, and [`ReportOriginRecorder`] writes the link beside the
//! conversation's project binding.
//!
//! The link is a convenience for the library's Project filter. It never gates
//! anything and never reaches DA, so every doubt resolves to "no link": a report
//! that is not linked shows under *No project*, and nothing about it changes.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;
use xfalcon_contracts::mcp::BUNDLED_SERVER_ID;
use xfalcon_contracts::reports::{ReportId, ReportProjectOrigin, ReportView};
use xfalcon_contracts::ConversationId;

use crate::mcp::runtime::mcp_tool_route;
use crate::projects::{ProjectId, ProjectsService};
use crate::security::account_cleanup::ValidatedAccountId;
use crate::tools::invocation::ToolResult;

/// The bare tool name DA publishes for saving a dashboard, workbook or deck.
const SAVE_TOOL: &str = "ida_save_dashboard";

/// The longest kit id accepted. DA's ids are short opaque tokens; the bound keeps
/// a hostile or garbled answer out of the database.
const MAX_KIT_ID_BYTES: usize = 128;

/// The kit a successful `ida_save_dashboard` call CREATED, if this was one.
///
/// `arguments` are the call's own, `result` the structured body the MCP
/// dispatcher built (`is_error`, `text`, `structured`, `truncated`). The id is
/// read from DA's structured answer when it sent one, else from its text.
/// Returns
/// `None` for every other outcome: a refused or failed save, a part of a
/// chunked save that is not the last, a redefine (a call that names a `kit_id`
/// edits a kit that already exists, possibly in another chat, and must not move
/// it), and any answer whose shape this does not recognise.
pub(crate) fn saved_kit_id(arguments: &Value, result: &Value) -> Option<String> {
    if has_kit_id_argument(arguments) {
        return None;
    }
    if result.get("is_error").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    // A clipped answer may have lost the id or the heading.
    if result.get("truncated").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    // DA also returns a dashboard save's id as data (`{kit_id, page, result}`),
    // and then that is the answer: no text in the report's name can pose as it.
    // A workbook or deck save, and an older DA, name the kit only in the text.
    if let Some(id) = result
        .get("structured")
        .and_then(|structured| structured.get("kit_id"))
    {
        return id.as_str().filter(|id| valid_kit_id(id)).map(str::to_owned);
    }
    let text = result.get("text")?.as_array()?;
    let first = text.first()?.as_str()?;
    let heading = first.lines().next()?.trim();
    // Both live shapes: `# ✅ Dashboard Saved`, and a document save's
    // `# ✅ <Workbook|Deck> Saved`. A part receipt starts `# 📦`, a refusal `# ❌`.
    if !(heading.starts_with("# ✅") && heading.ends_with("Saved")) {
        return None;
    }
    // Exactly one kit: an answer naming two is not one this understands.
    let [id] = named_kit_ids(first)[..] else {
        return None;
    };
    if !valid_kit_id(id) {
        return None;
    }
    // The answer's own follow-up hints repeat the id (`kit_id='…'`). One that
    // disagrees means text in the report's name posed as the id line.
    if quoted_kit_ids(first).any(|hint| hint != id) {
        return None;
    }
    Some(id.to_owned())
}

fn has_kit_id_argument(arguments: &Value) -> bool {
    match arguments.get("kit_id") {
        None | Some(Value::Null) => false,
        Some(Value::String(id)) => !id.trim().is_empty(),
        Some(_) => true,
    }
}

/// The text that opens DA's id line for a dashboard or document save.
const KIT_ID_MARKER: &str = "— kit id `";

/// The kit id each line of the answer names, in order.
///
/// DA words two lines this way: `**<name>** — kit id `<id>`` (a document adds
/// ` (<format>)`), and a chunked save's last part `Kit `<id>` — assembled from N
/// parts.`. The name is the model's text and may hold backticks, newlines, or
/// the marker itself, so a line is read from its END: the last marker on a line
/// is the real one, and what follows the id must be exactly what DA writes
/// there. A line that does not fit names nothing.
fn named_kit_ids(text: &str) -> Vec<&str> {
    text.lines().filter_map(kit_on_line).collect()
}

fn kit_on_line(line: &str) -> Option<&str> {
    if let Some(at) = line.rfind(KIT_ID_MARKER) {
        let rest = &line[at + KIT_ID_MARKER.len()..];
        let end = rest.find('`')?;
        let tail = rest[end + 1..].trim();
        let formatted = matches!(tail, "(dashboard)" | "(workbook)" | "(presentation)");
        return (tail.is_empty() || formatted).then(|| &rest[..end]);
    }
    let rest = line.strip_prefix("Kit `")?;
    let end = rest.find('`')?;
    rest[end + 1..]
        .starts_with(" — assembled from ")
        .then(|| &rest[..end])
}

/// Every id written as `kit_id='<id>'` in the answer (its recall and refresh hints).
fn quoted_kit_ids(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices("kit_id='").filter_map(|(at, marker)| {
        let rest = &text[at + marker.len()..];
        rest.find('\'').map(|end| &rest[..end])
    })
}

fn valid_kit_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_KIT_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

/// The links to show for the reports DA listed.
///
/// A link to a kit DA did not list (deleted since, or not visible to this
/// reader) is dropped: the library names only reports it shows. `linked` is
/// `(kit id, project, project name)`.
pub(crate) fn origins_for_listed(
    listed: &[ReportView],
    linked: Vec<(String, ProjectId, String)>,
) -> Vec<ReportProjectOrigin> {
    let shown: HashSet<&str> = listed.iter().map(|report| report.id.as_str()).collect();
    linked
        .into_iter()
        .filter(|(kit_id, _, _)| shown.contains(kit_id.as_str()))
        .map(|(kit_id, project_id, name)| {
            ReportProjectOrigin::new(ReportId::from(kit_id), project_id.to_string(), name)
        })
        .collect()
}

/// Writes the link for a run whose conversation is bound to a project.
///
/// Built once per run, only when there is a project, so a regular chat carries no
/// recorder and a save in it can never be linked by accident.
pub(crate) struct ReportOriginRecorder {
    projects: Arc<ProjectsService>,
    account: ValidatedAccountId,
    project_id: ProjectId,
    conversation_id: ConversationId,
}

impl ReportOriginRecorder {
    pub(crate) fn new(
        projects: Arc<ProjectsService>,
        account: ValidatedAccountId,
        project_id: ProjectId,
        conversation_id: ConversationId,
    ) -> Self {
        Self {
            projects,
            account,
            project_id,
            conversation_id,
        }
    }

    /// Looks at a finished MCP tool call and, when it was a save that created a
    /// kit, links the kit. Never fails the call: the save already happened, and
    /// the link is only a way to find it again.
    pub(crate) fn observe(&self, tool_id: &str, arguments: &Value, result: &ToolResult) {
        // Only xFalcon's own server: the library reads that server's reports, and a
        // save answered by any other server names a kit that is not in it.
        if !mcp_tool_route(tool_id)
            .is_some_and(|(slug, name)| slug == BUNDLED_SERVER_ID && name == SAVE_TOOL)
        {
            return;
        }
        let Some(body) = result.structured() else {
            return;
        };
        self.observe_save(arguments, body);
    }

    fn observe_save(&self, arguments: &Value, result: &Value) {
        let Some(kit_id) = saved_kit_id(arguments, result) else {
            return;
        };
        match self.projects.record_report_origin(
            &self.account,
            &kit_id,
            self.project_id,
            &self.conversation_id,
        ) {
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "reports: could not link a saved report to its project");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn answer(text: &str) -> Value {
        json!({ "is_error": false, "text": [text], "structured": null, "truncated": false })
    }

    const DASHBOARD_SAVED: &str = "# ✅ Dashboard Saved\n\n**Weekly stock** — kit id `kit-123`\n- Scope: user\n- Pages: 1\n- Datasets stored: 2\n\nRecall it later with `ida_recall_dashboard(kit_id='kit-123')`.";

    #[test]
    fn a_new_dashboard_save_names_its_kit() {
        assert_eq!(
            saved_kit_id(&json!({ "name": "Weekly stock" }), &answer(DASHBOARD_SAVED)),
            Some("kit-123".to_owned())
        );
    }

    #[test]
    fn the_last_part_of_a_chunked_save_names_its_kit() {
        let text = "# ✅ Dashboard Saved\n\nKit `kit-9` — assembled from 3 parts.\n\n1 page(s), 2 dataset(s) stored.";
        assert_eq!(
            saved_kit_id(
                &json!({ "transfer_id": "t", "part_index": 2 }),
                &answer(text)
            ),
            Some("kit-9".to_owned())
        );
    }

    #[test]
    fn a_document_save_names_its_kit() {
        let text = "# ✅ Workbook Saved\n\n**Margins** — kit id `kit-wb` (workbook)\n- Scope: user";
        assert_eq!(
            saved_kit_id(&json!({ "format": "workbook" }), &answer(text)),
            Some("kit-wb".to_owned())
        );
    }

    fn structured_answer(text: &str, structured: Value) -> Value {
        json!({ "is_error": false, "text": [text], "structured": structured, "truncated": false })
    }

    #[test]
    fn a_structured_answer_names_its_kit_whatever_the_text_says() {
        let posing = "# ✅ Dashboard Saved\n\n**x — kit id `kit-evil`** — kit id `kit-evil`";
        assert_eq!(
            saved_kit_id(
                &json!({ "name": "x" }),
                &structured_answer(
                    posing,
                    json!({ "kit_id": "kit-123", "page": null, "result": posing })
                )
            ),
            Some("kit-123".to_owned())
        );
        // A structured answer with no usable id is not read again as text.
        for id in [json!(null), json!(""), json!(7), json!("kit 1")] {
            assert_eq!(
                saved_kit_id(
                    &json!({ "name": "x" }),
                    &structured_answer(DASHBOARD_SAVED, json!({ "kit_id": id }))
                ),
                None,
                "{id}"
            );
        }
        // Still never a redefine, an error, or a clipped answer.
        let saved = structured_answer(DASHBOARD_SAVED, json!({ "kit_id": "kit-123" }));
        assert_eq!(saved_kit_id(&json!({ "kit_id": "kit-123" }), &saved), None);
        let mut failed = saved.clone();
        failed["is_error"] = json!(true);
        assert_eq!(saved_kit_id(&json!({}), &failed), None);
        let mut clipped = saved;
        clipped["truncated"] = json!(true);
        assert_eq!(saved_kit_id(&json!({}), &clipped), None);
    }

    #[test]
    fn a_redefine_of_an_existing_kit_is_not_linked() {
        assert_eq!(
            saved_kit_id(&json!({ "kit_id": "kit-123" }), &answer(DASHBOARD_SAVED)),
            None
        );
    }

    #[test]
    fn a_blank_or_null_kit_id_argument_still_counts_as_a_new_kit() {
        for argument in [json!(null), json!(""), json!("   ")] {
            assert_eq!(
                saved_kit_id(&json!({ "kit_id": argument }), &answer(DASHBOARD_SAVED)),
                Some("kit-123".to_owned()),
                "{argument}"
            );
        }
    }

    #[test]
    fn a_refusal_a_part_receipt_and_an_error_are_not_linked() {
        let refused = "# ❌ Dashboard Not Saved\n\nkit id `kit-123` was refused";
        let receipt = "# 📦 Part 1/3 received\n\nTransfer `t-1` — 1 of 3 parts stored.";
        for text in [refused, receipt, "Error: name is required"] {
            assert_eq!(saved_kit_id(&json!({}), &answer(text)), None, "{text}");
        }
        let mut errored = answer(DASHBOARD_SAVED);
        errored["is_error"] = json!(true);
        assert_eq!(saved_kit_id(&json!({}), &errored), None);
    }

    #[test]
    fn a_clipped_or_malformed_result_is_not_linked() {
        let mut clipped = answer(DASHBOARD_SAVED);
        clipped["truncated"] = json!(true);
        assert_eq!(saved_kit_id(&json!({}), &clipped), None);
        for body in [
            json!(null),
            json!({}),
            json!({ "is_error": false }),
            json!({ "is_error": false, "text": [] }),
            json!({ "is_error": false, "text": "not a list" }),
            json!({ "is_error": false, "text": [7] }),
        ] {
            assert_eq!(saved_kit_id(&json!({}), &body), None, "{body}");
        }
    }

    #[test]
    fn an_answer_naming_two_kits_is_not_linked() {
        let text = "# ✅ Dashboard Saved\n\n**a** — kit id `kit-1`\n**b** — kit id `kit-2`";
        assert_eq!(saved_kit_id(&json!({}), &answer(text)), None);
    }

    #[test]
    fn a_malformed_or_oversized_id_is_not_linked() {
        let long = "k".repeat(MAX_KIT_ID_BYTES + 1);
        for id in [
            "",
            "has space",
            "semi;colon",
            "quo'te",
            "new\nline",
            long.as_str(),
        ] {
            let text = format!("# ✅ Dashboard Saved\n\n**x** — kit id `{id}`");
            assert_eq!(saved_kit_id(&json!({}), &answer(&text)), None, "{id:?}");
        }
        let longest = "k".repeat(MAX_KIT_ID_BYTES);
        let text = format!("# ✅ Dashboard Saved\n\n**x** — kit id `{longest}`");
        assert_eq!(saved_kit_id(&json!({}), &answer(&text)), Some(longest));
    }

    #[test]
    fn only_the_first_text_block_is_read() {
        let body = json!({
            "is_error": false,
            "text": ["unrelated", DASHBOARD_SAVED],
            "truncated": false,
        });
        assert_eq!(saved_kit_id(&json!({}), &body), None);
    }

    fn listed_report(id: &str) -> ReportView {
        use xfalcon_contracts::reports::{ReportKind, ReportScope};
        ReportView {
            version: xfalcon_contracts::CONTRACT_VERSION,
            id: ReportId::from(id),
            kind: ReportKind::Dashboard,
            title: "Report".to_owned(),
            description: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            thumbnail_updated_at_ms: None,
            pinned: false,
            hidden: false,
            byte_size: 0,
            scope: ReportScope::User,
            is_owner: true,
            owner_display_name: None,
            refreshable: true,
            can_refresh: true,
            can_manage: true,
            can_delete: true,
            page_count: 1,
            dataset_count: 1,
            data_sources: Vec::new(),
            source_conversation_id: None,
        }
    }

    #[test]
    fn only_links_to_listed_reports_are_returned_with_the_projects_name() {
        let project = ProjectId::new();
        let origins = origins_for_listed(
            &[listed_report("kit-1"), listed_report("kit-2")],
            vec![
                ("kit-1".to_owned(), project, "Sales".to_owned()),
                // Deleted since, or not visible to this reader: not shown.
                ("kit-gone".to_owned(), project, "Sales".to_owned()),
            ],
        );
        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0].report_id.as_str(), "kit-1");
        assert_eq!(origins[0].project_id, project.to_string());
        assert_eq!(origins[0].project_name, "Sales");
    }

    #[test]
    fn no_listed_reports_or_no_links_give_no_origins() {
        let project = ProjectId::new();
        assert!(
            origins_for_listed(&[], vec![("kit-1".to_owned(), project, "S".to_owned())]).is_empty()
        );
        assert!(origins_for_listed(&[listed_report("kit-1")], Vec::new()).is_empty());
    }

    mod recorder {
        use std::str::FromStr;
        use std::sync::Arc;

        use tempfile::tempdir;
        use xfalcon_contracts::tools::ToolSource;

        use super::*;
        use crate::persistence::{InMemorySecretStore, SecretStore};
        use crate::security::account_cleanup::CurrentAccountScope;

        const ACCOUNT: &str = "account-1";
        const SAVE: &str = "mcp.tenant.ida_save_dashboard";

        struct Fixture {
            _app_root: tempfile::TempDir,
            _project_root: tempfile::TempDir,
            projects: Arc<ProjectsService>,
            account: ValidatedAccountId,
            project_id: ProjectId,
            recorder: ReportOriginRecorder,
        }

        async fn fixture() -> Fixture {
            let app_root = tempdir().expect("app root");
            let project_root = tempdir().expect("project root");
            let scope = Arc::new(CurrentAccountScope::default());
            scope.bind_native(ACCOUNT).expect("bind account");
            let secrets = Arc::new(InMemorySecretStore::default());
            secrets.put_key(ACCOUNT, [0x11; 32]).expect("account key");
            let projects = Arc::new(ProjectsService::new(
                app_root.path().to_path_buf(),
                scope,
                secrets as Arc<dyn SecretStore>,
            ));
            let created = projects
                .create(
                    &project_root.path().to_string_lossy(),
                    Some("Sales".to_owned()),
                )
                .await
                .expect("create project");
            let project_id = ProjectId::from_str(created.id.as_str()).expect("project id");
            let account = ValidatedAccountId::parse(ACCOUNT).expect("account");
            let recorder = ReportOriginRecorder::new(
                projects.clone(),
                account.clone(),
                project_id,
                ConversationId::from("conversation-1"),
            );
            Fixture {
                _app_root: app_root,
                _project_root: project_root,
                projects,
                account,
                project_id,
                recorder,
            }
        }

        fn result(text: &str) -> ToolResult {
            ToolResult::new(vec![], Some(answer(text)), ToolSource::Mcp).expect("result")
        }

        fn linked(fixture: &Fixture) -> Vec<(String, ProjectId, String)> {
            fixture
                .projects
                .report_origins(&fixture.account)
                .expect("origins")
        }

        #[tokio::test]
        async fn a_save_in_a_project_run_links_the_kit_with_the_projects_name() {
            let fixture = fixture().await;
            fixture
                .recorder
                .observe(SAVE, &json!({ "name": "Weekly" }), &result(DASHBOARD_SAVED));
            assert_eq!(
                linked(&fixture),
                vec![("kit-123".to_owned(), fixture.project_id, "Sales".to_owned())]
            );
        }

        #[tokio::test]
        async fn only_the_tenant_servers_save_tool_counts() {
            let fixture = fixture().await;
            // Another tool of the tenant server, another server's save tool, and
            // an id that is not an MCP id at all: none of them links anything.
            for id in [
                "mcp.tenant.ida_list_dashboards",
                "mcp.other-server.ida_save_dashboard",
                "not-an-mcp-id",
            ] {
                fixture
                    .recorder
                    .observe(id, &json!({}), &result(DASHBOARD_SAVED));
            }
            assert!(linked(&fixture).is_empty());
            fixture
                .recorder
                .observe(SAVE, &json!({}), &result(DASHBOARD_SAVED));
            assert_eq!(linked(&fixture).len(), 1);
        }

        #[tokio::test]
        async fn a_redefine_a_refusal_and_a_result_with_no_body_link_nothing() {
            let fixture = fixture().await;
            fixture.recorder.observe(
                SAVE,
                &json!({ "kit_id": "kit-123" }),
                &result(DASHBOARD_SAVED),
            );
            fixture.recorder.observe(
                SAVE,
                &json!({}),
                &result("# ❌ Dashboard Not Saved\n\nkit id `kit-123`"),
            );
            let bodyless = ToolResult::new(vec![], None, ToolSource::Mcp).expect("result");
            fixture.recorder.observe(SAVE, &json!({}), &bodyless);
            assert!(linked(&fixture).is_empty());
        }

        #[tokio::test]
        async fn a_second_save_of_the_same_kit_keeps_the_first_link() {
            let fixture = fixture().await;
            fixture
                .recorder
                .observe(SAVE, &json!({}), &result(DASHBOARD_SAVED));
            // The same kit again, from a conversation of ANOTHER trusted project.
            let other_root = tempdir().expect("other project root");
            let other = fixture
                .projects
                .create(&other_root.path().to_string_lossy(), Some("Ops".to_owned()))
                .await
                .expect("second project");
            let other_id = ProjectId::from_str(other.id.as_str()).expect("project id");
            let recorder = ReportOriginRecorder::new(
                fixture.projects.clone(),
                fixture.account.clone(),
                other_id,
                ConversationId::from("conversation-2"),
            );
            recorder.observe(SAVE, &json!({}), &result(DASHBOARD_SAVED));

            let links = linked(&fixture);
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].1, fixture.project_id);
            assert_eq!(links[0].2, "Sales");
        }

        #[tokio::test]
        async fn a_project_the_account_does_not_hold_links_nothing_and_does_not_fail() {
            let fixture = fixture().await;
            let recorder = ReportOriginRecorder::new(
                fixture.projects.clone(),
                fixture.account.clone(),
                ProjectId::new(),
                ConversationId::from("conversation-2"),
            );
            recorder.observe(SAVE, &json!({}), &result(DASHBOARD_SAVED));
            assert!(linked(&fixture).is_empty());
        }

        #[tokio::test]
        async fn a_forgotten_kit_leaves_the_list() {
            let fixture = fixture().await;
            fixture
                .recorder
                .observe(SAVE, &json!({}), &result(DASHBOARD_SAVED));
            fixture
                .projects
                .forget_report_origin(&fixture.account, "kit-123")
                .expect("forget");
            assert!(linked(&fixture).is_empty());
        }
    }

    #[test]
    fn a_name_that_poses_as_the_id_line_cannot_name_the_kit() {
        let hint = "\n\nRecall it later with `ida_recall_dashboard(kit_id='kit-123')`.";
        // The marker inside the name, on the one line: the line's own end wins.
        let inline =
            format!("# ✅ Dashboard Saved\n\n**x — kit id `FORGED`** — kit id `kit-123`{hint}");
        assert_eq!(
            saved_kit_id(&json!({}), &answer(&inline)),
            Some("kit-123".to_owned())
        );
        // A backtick pairing shifted by an odd number of backticks in the name.
        let shifted = format!("# ✅ Dashboard Saved\n\n**a kit `** — kit id `kit-123`{hint}");
        assert_eq!(
            saved_kit_id(&json!({}), &answer(&shifted)),
            Some("kit-123".to_owned())
        );
        // A name with a newline that starts a second, forged line: two ids, refused.
        let split = format!(
            "# ✅ Dashboard Saved\n\n**x\nKit `FORGED` — assembled from 2 parts.\ny** — kit id `kit-123`{hint}"
        );
        assert_eq!(saved_kit_id(&json!({}), &answer(&split)), None);
        // A forged id with no real line behind it, and a hint that disagrees.
        let lone = "# ✅ Dashboard Saved\n\n**x — kit id `FORGED`**";
        assert_eq!(saved_kit_id(&json!({}), &answer(lone)), None);
        let disagreeing = "# ✅ Dashboard Saved\n\n**x** — kit id `FORGED`\n\n`ida_recall_dashboard(kit_id='kit-123')`";
        assert_eq!(saved_kit_id(&json!({}), &answer(disagreeing)), None);
    }

    #[test]
    fn text_after_the_id_on_its_line_must_be_what_xfalcon_writes() {
        let extra = "# ✅ Dashboard Saved\n\n**x** — kit id `kit-1` and more";
        assert_eq!(saved_kit_id(&json!({}), &answer(extra)), None);
        let word = "# ✅ Dashboard Saved\n\nA toolkit `kit-1` was made";
        assert_eq!(saved_kit_id(&json!({}), &answer(word)), None);
    }

    #[test]
    fn an_id_that_is_not_named_as_a_kit_is_ignored() {
        let text = "# ✅ Dashboard Saved\n\nTransfer `t-1` stored; the dataset `sales` too.";
        assert_eq!(saved_kit_id(&json!({}), &answer(text)), None);
    }
}
