//! Opt-in test export of the app-owned text that reaches model prompts.
//!
//! This is deliberately absent from production builds and has no filesystem
//! effect unless `XFALCON_PROMPT_SCAN_DIR` is set for the test run.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::agent::context::{ContextMessage, ContextRole, ContextToolCall};
use crate::agent::prompt::{build_system_prompt, PromptProject, PromptSkill, SystemPromptInputs};
use crate::agent::summary::{summarization_messages, transcript, ClientSummary};
use crate::providers::ProviderToolSchema;

const EXPECTED: &[&str] = &[
    "agent_active_skills_header.txt",
    "agent_assembled_system_prompt.txt",
    "agent_baseline_system_prompt.txt",
    "agent_capability_catalog.txt",
    "agent_default_system_prompt.txt",
    "agent_desktop_skill_guidance.txt",
    "agent_elided_tool_arguments.txt",
    "agent_omitted_history_note.txt",
    "agent_pruned_tool_result.txt",
    "agent_summary_replay.txt",
    "agent_summary_request.json",
    "agent_summary_system.txt",
    "agent_summary_transcript.txt",
    "agent_tool_inventory_prefix.txt",
    "agent_tool_inventory_suffix.txt",
    "agent_visualization_contract.txt",
    "builtin_tool_catalog.json",
    "chat_followup_request.json",
    "chat_title_request.json",
    "claude_runtime_assistant_marker.txt",
    "claude_runtime_attachment_placeholder.txt",
    "claude_runtime_continue_instruction.txt",
    "claude_runtime_default_system_text.txt",
    "claude_runtime_handled_text.txt",
    "claude_runtime_preamble.txt",
    "claude_runtime_reply_instruction.txt",
    "claude_runtime_tool_call_block.txt",
    "claude_runtime_tool_result_block.txt",
    "claude_runtime_tool_result_unknown_block.txt",
    "claude_runtime_truncated_line.txt",
    "claude_runtime_user_marker.txt",
    "custom_gateway_tool_response_guidance.txt",
    "custom_openai_guided_request.json",
    "custom_openai_probe_request.json",
    "turn_more_tool_calls_continuation.txt",
    "turn_malformed_arguments_answer.txt",
    "turn_plain_continuation.txt",
    "openrouter_compaction_instructions.txt",
    "openrouter_compaction_request.json",
    "openrouter_probe_request.json",
    "skill_files_note.txt",
    "turn_last_round_budget_wrapper.json",
    "turn_run_status_wrapper.json",
    "turn_single_tool_call_continuation.txt",
    "turn_stream_cut_continuation.txt",
    "turn_thinking_limit_continuation.txt",
    "turn_unknown_tool_answer.txt",
    "turn_unanswered_call_answer.txt",
    "turn_unfinished_plan_notice.txt",
    "turn_unrunnable_calls_continuation.txt",
    "user_attachment_message.json",
];

struct Artifact {
    file: &'static str,
    source: &'static str,
    content: String,
}

#[test]
fn export_complete_app_prompt_inventory_when_configured() {
    let Some(directory) = std::env::var_os("XFALCON_PROMPT_SCAN_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let artifacts = collect_artifacts();
    let names: BTreeSet<_> = artifacts.iter().map(|artifact| artifact.file).collect();
    assert_eq!(
        names.len(),
        artifacts.len(),
        "prompt inventory has duplicate files"
    );
    let expected: BTreeSet<_> = EXPECTED.iter().copied().collect();
    assert_eq!(
        names, expected,
        "prompt inventory differs from its expected catalog"
    );

    fs::create_dir_all(&directory).expect("create requested prompt scan directory");
    let mut records = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        assert!(
            !artifact.content.trim().is_empty(),
            "{} is empty",
            artifact.file
        );
        fs::write(directory.join(artifact.file), artifact.content.as_bytes())
            .unwrap_or_else(|error| panic!("write {}: {error}", artifact.file));
        let hash = Sha256::digest(artifact.content.as_bytes());
        records.push(json!({
            "file": artifact.file,
            "variant": artifact.file.rsplit_once('.').map_or(artifact.file, |(stem, _)| stem),
            "source": artifact.source,
            "bytes": artifact.content.len(),
            "sha256": format!("{hash:x}"),
        }));
    }
    let manifest = json!({
        "format_version": 1,
        "expected_files": EXPECTED,
        "artifacts": records,
        "scope_limit": "App-owned prompts and synthetic assembled variants only. Arbitrary runtime user, project, skill, MCP, database, and provider content is not scanned.",
    });
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("prompt scan manifest serializes"),
    )
    .expect("write prompt scan manifest");

    for file in EXPECTED {
        let bytes = fs::read(directory.join(file)).expect("expected prompt artifact exists");
        assert!(
            !bytes.is_empty(),
            "expected prompt artifact {file} is nonempty"
        );
    }
}

fn collect_artifacts() -> Vec<Artifact> {
    let mut artifacts = Vec::new();
    for (name, content) in crate::agent::prompt::prompt_scan_fragments() {
        let (file, source) = match name {
            "default_system_prompt" => (
                "agent_default_system_prompt.txt",
                "src-tauri/src/agent/prompts/default_system_prompt.md",
            ),
            "visualization_contract" => (
                "agent_visualization_contract.txt",
                "src-tauri/src/agent/prompts/visualization_contract.md",
            ),
            "tool_inventory_prefix" => (
                "agent_tool_inventory_prefix.txt",
                "src-tauri/src/agent/prompts/tool_inventory_prefix.md",
            ),
            "tool_inventory_suffix" => (
                "agent_tool_inventory_suffix.txt",
                "src-tauri/src/agent/prompts/tool_inventory_suffix.md",
            ),
            "active_skills_header" => (
                "agent_active_skills_header.txt",
                "src-tauri/src/agent/prompt.rs::ACTIVE_SKILLS_HEADER",
            ),
            "desktop_skill_guidance" => (
                "agent_desktop_skill_guidance.txt",
                "src-tauri/src/agent/prompt.rs::DESKTOP_SKILL_GUIDANCE",
            ),
            "capability_catalog" => (
                "agent_capability_catalog.txt",
                "src-tauri/src/agent/prompt.rs::CAPABILITY_CATALOG",
            ),
            _ => unreachable!("unknown prompt fragment"),
        };
        artifacts.push(Artifact {
            file,
            source,
            content: content.to_owned(),
        });
    }

    let baseline = build_system_prompt(&SystemPromptInputs::default());
    artifacts.push(Artifact {
        file: "agent_baseline_system_prompt.txt",
        source: "src-tauri/src/agent/prompt.rs::build_system_prompt; empty inputs",
        content: baseline,
    });
    let skills = [PromptSkill {
        name: "synthetic-reporting".to_owned(),
        description: "Guidance for summarizing synthetic business metrics.".to_owned(),
        source: "synthetic-test".to_owned(),
    }];
    let tools = [ProviderToolSchema {
        name: "synthetic.query_metrics".to_owned(),
        description: "Query synthetic metrics from an approved data source.".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": { "metric": { "type": "string" } },
            "required": ["metric"]
        }),
    }];
    let project = PromptProject {
        display_name: "Synthetic project".to_owned(),
    };
    let assembled = build_system_prompt(&SystemPromptInputs {
        conversation_system_prompt: Some(
            "Use the synthetic conversation's stated reporting preferences.",
        ),
        project: Some(&project),
        skills: &skills,
        tools: &tools,
        tool_ids: None,
        unavailable_servers: &["synthetic-server".to_owned()],
    });
    artifacts.push(Artifact {
        file: "agent_assembled_system_prompt.txt",
        source: "src-tauri/src/agent/prompt.rs::build_system_prompt; synthetic project, skill, tool, conversation guidance and unavailable server",
        content: assembled,
    });

    let source_messages = vec![
        ContextMessage::new(ContextRole::User, "Summarize synthetic sales totals."),
        ContextMessage::assistant_tool_calls(
            "Checking the synthetic sales metric.",
            vec![ContextToolCall::new(
                "synthetic-call",
                "synthetic.query_metrics",
                "{\"metric\":\"sales\"}",
            )],
        ),
        ContextMessage::tool_result(
            "synthetic-call",
            "synthetic.query_metrics",
            "{\"status\":\"completed\",\"rows\":1}",
        ),
        ContextMessage::new(ContextRole::Assistant, "The synthetic total is 42."),
    ];
    let transcript_text = transcript(&source_messages, Some("Previous synthetic summary."));
    let summary_request = summarization_messages(transcript_text.clone());
    let summary_fragments = crate::agent::summary::prompt_scan_fragments();
    let summary_system = summary_fragments
        .iter()
        .find(|(name, _)| *name == "summary_system_prompt")
        .expect("summary system prompt fragment exists")
        .1
        .clone();
    artifacts.push(Artifact {
        file: "agent_summary_system.txt",
        source: "src-tauri/src/agent/summary.rs::SUMMARY_SYSTEM_PROMPT",
        content: summary_system,
    });
    artifacts.push(Artifact {
        file: "agent_summary_transcript.txt",
        source: "src-tauri/src/agent/summary.rs::transcript; synthetic messages",
        content: transcript_text,
    });
    artifacts.push(Artifact {
        file: "agent_summary_request.json",
        source: "src-tauri/src/agent/summary.rs::summarization_messages; exact constructed message strings",
        content: context_messages_json(&summary_request),
    });
    artifacts.push(Artifact {
        file: "agent_summary_replay.txt",
        source: "src-tauri/src/agent/summary.rs::ClientSummary::message; synthetic summary",
        content: ClientSummary {
            covered: Vec::new(),
            text: "Synthetic request and completed query facts.".to_owned(),
        }
        .message()
        .content,
    });
    for (name, content) in summary_fragments {
        let file = match name {
            "summary_system_prompt" => continue,
            "elided_tool_arguments" => "agent_elided_tool_arguments.txt",
            "omitted_history_note" => "agent_omitted_history_note.txt",
            "pruned_tool_result" => "agent_pruned_tool_result.txt",
            _ => unreachable!("unknown summary prompt fragment"),
        };
        artifacts.push(Artifact {
            file,
            source: "src-tauri/src/agent/summary.rs; exact app-authored context text",
            content,
        });
    }

    let title = crate::chat::prompt_scan_title_request(
        "synthetic-model",
        "Summarize the synthetic quarterly sales totals.",
    );
    artifacts.push(Artifact {
        file: "chat_title_request.json",
        source: "src-tauri/src/chat/title.rs::model_title_request; synthetic opening message",
        content: context_messages_json(&title.messages),
    });
    let follow_up = crate::chat::prompt_scan_follow_up_request();
    artifacts.push(Artifact {
        file: "chat_followup_request.json",
        source: "src-tauri/src/chat/follow_ups.rs::suggestion_request; synthetic excerpts",
        content: context_messages_json(&follow_up.messages),
    });
    artifacts.push(Artifact {
        file: "user_attachment_message.json",
        source: "src-tauri/src/chat/attachments.rs::attach_to_turn; synthetic text attachment",
        content: context_messages_json(&[ContextMessage::new(
            ContextRole::User,
            crate::chat::prompt_scan_attachment_text(),
        )]),
    });
    artifacts.push(Artifact {
        file: "skill_files_note.txt",
        source: "src-tauri/src/skills/dispatch.rs::FILES_NOTE",
        content: crate::skills::dispatch::prompt_scan_files_note().to_owned(),
    });

    for (name, content) in crate::agent::turn_driver::prompt_scan_fragments() {
        let file = match name {
            "run_status_wrapper" => "turn_run_status_wrapper.json",
            "last_round_budget_wrapper" => "turn_last_round_budget_wrapper.json",
            "unfinished_plan_notice" => "turn_unfinished_plan_notice.txt",
            "stream_cut_continuation" => "turn_stream_cut_continuation.txt",
            "more_tool_calls_continuation" => "turn_more_tool_calls_continuation.txt",
            "unrunnable_calls_continuation" => "turn_unrunnable_calls_continuation.txt",
            "thinking_limit_continuation" => "turn_thinking_limit_continuation.txt",
            "single_tool_call_continuation" => "turn_single_tool_call_continuation.txt",
            "plain_continuation" => "turn_plain_continuation.txt",
            "malformed_arguments_answer" => "turn_malformed_arguments_answer.txt",
            "unknown_tool_answer" => "turn_unknown_tool_answer.txt",
            "unanswered_call_answer" => "turn_unanswered_call_answer.txt",
            _ => unreachable!("unknown turn-driver prompt fragment"),
        };
        artifacts.push(Artifact {
            file,
            source:
                "src-tauri/src/agent/turn_driver.rs; test-only accessor calls production formatter",
            content: if name == "run_status_wrapper" || name == "last_round_budget_wrapper" {
                context_messages_json(&[ContextMessage::new(ContextRole::User, content)])
            } else {
                content
            },
        });
    }
    for (name, content) in crate::providers::openrouter_prompt_scan_fragments() {
        let file = match name {
            "openrouter_compaction_instructions" => "openrouter_compaction_instructions.txt",
            "openrouter_probe_request" => "openrouter_probe_request.json",
            "openrouter_compaction_request" => "openrouter_compaction_request.json",
            _ => unreachable!("unknown OpenRouter prompt fragment"),
        };
        artifacts.push(Artifact {
            file,
            source: "src-tauri/src/providers/openrouter.rs; production compaction and probe constructors",
            content,
        });
    }
    for (name, content) in crate::providers::custom_openai_prompt_scan_fragments() {
        let file = match name {
            "custom_gateway_tool_response_guidance" => "custom_gateway_tool_response_guidance.txt",
            "custom_openai_probe_request" => "custom_openai_probe_request.json",
            "custom_openai_tool_guidance_request" => "custom_openai_guided_request.json",
            _ => unreachable!("unknown custom OpenAI prompt fragment"),
        };
        artifacts.push(Artifact {
            file,
            source: "src-tauri/src/providers/custom_openai.rs; production request-body constructor",
            content,
        });
    }
    for (name, content) in crate::providers::claude_runtime::prompt::prompt_scan_fragments() {
        let file = match name {
            "default_system_text" => "claude_runtime_default_system_text.txt",
            "preamble" => "claude_runtime_preamble.txt",
            "reply_instruction" => "claude_runtime_reply_instruction.txt",
            "continue_instruction" => "claude_runtime_continue_instruction.txt",
            "user_marker" => "claude_runtime_user_marker.txt",
            "assistant_marker" => "claude_runtime_assistant_marker.txt",
            "truncated_line" => "claude_runtime_truncated_line.txt",
            "attachment_placeholder" => "claude_runtime_attachment_placeholder.txt",
            "tool_call_block" => "claude_runtime_tool_call_block.txt",
            "tool_result_block" => "claude_runtime_tool_result_block.txt",
            "tool_result_unknown_block" => "claude_runtime_tool_result_unknown_block.txt",
            "handled_text" => "claude_runtime_handled_text.txt",
            _ => unreachable!("unknown Claude runtime prompt fragment"),
        };
        artifacts.push(Artifact {
            file,
            source: "src-tauri/src/providers/claude_runtime/prompt.rs and declare_server.rs; production rendering and fixed text",
            content,
        });
    }
    artifacts.push(Artifact {
        file: "builtin_tool_catalog.json",
        source: "src-tauri/src/tools/registry.rs; production built-in ToolDescriptor constructors",
        content: crate::tools::registry::prompt_scan_builtin_catalog()
            .expect("built-in prompt-facing tool catalog is valid"),
    });
    artifacts
}

fn context_messages_json(messages: &[ContextMessage]) -> String {
    let messages: Vec<Value> = messages
        .iter()
        .map(|message| {
            let role = match message.role {
                ContextRole::System => "system",
                ContextRole::User => "user",
                ContextRole::Assistant => "assistant",
                ContextRole::Tool => "tool",
            };
            json!({ "role": role, "content": message.content })
        })
        .collect();
    serde_json::to_string_pretty(&messages).expect("constructed prompt messages serialize")
}
