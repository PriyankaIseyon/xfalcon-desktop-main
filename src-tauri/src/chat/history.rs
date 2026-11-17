//! The durable conversation, projected into the model's context.
//!
//! # Why this reads `xf_chat.chats` and nothing else
//!
//! A tool round is durable as two message shapes: the assistant turn carrying
//! `tool_calls`, and one `role="tool"` row answering each call. Both this app
//! and xfalcon-web write exactly that (`create_assistant_tool_call_message` /
//! `create_tool_result_message` here, `persistAssistantToolCallMessage` /
//! `persistToolResultMessage` there), so the conversation is the one record
//! either product needs to continue the other's work.
//!
//! `xf_chat.tool_calls` is the execution log — arguments, output, timing,
//! approval, which device ran it. It is the right source for showing a user what
//! ran, and the wrong source for rebuilding a conversation: its rows attach to
//! the assistant message a *run* finalized, not to the turn that requested them,
//! so a multi-round run replays with every round hung off its last turn.
//!
//! # A provider validates the pairing, so this does too
//!
//! Every provider rejects a request in which an advertised call has no answer,
//! or an answer names no call. These rows arrive over the network and a write
//! can fail between the two halves, so the pairing is re-established here rather
//! than assumed: a call keeps its place only when an answer for it exists, and
//! an answer is emitted only beside the call it belongs to.

use std::collections::{HashMap, HashSet};

use crate::agent::context::{ContextMessage, ContextRole, ContextToolCall, DurableMessageIdentity};
use crate::agent::turn_driver::UNANSWERED_CALL_ANSWER;
#[cfg(test)]
use crate::clients::da::generated::ChatMessage;
use crate::clients::da::generated::{
    ChatMessageToolCalls, ContextMessage as DaContextMessage, ContextMessageRole, PayloadStatus,
};
use crate::tools::exact_result::AdmittedToolResultPayload;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ContextHistoryError {
    #[error("context snapshot sequence is not strictly increasing")]
    InvalidSequence,
    #[error("context snapshot contains a duplicate tool result")]
    DuplicateToolResult,
    #[error("context snapshot contains an incomplete tool exchange")]
    IncompleteToolExchange,
    #[error("context snapshot contains an empty provider message")]
    EmptyMessage,
    #[error("context snapshot contains an invalid tool-result payload descriptor")]
    InvalidPayloadDescriptor,
}

/// Inference projection for the authoritative DA context endpoint.
///
/// A model continuation must see closed tool groups: every call the assistant
/// made is followed by an answer. DA's rows are taken as they are, with one
/// repair for a round that was cut off between writing the assistant's calls
/// and writing their results (a dropped connection, a rejected write, the app
/// closing). Such a call is answered in place with "no result was produced",
/// which is what happened, so the conversation stays usable instead of failing
/// every later message before it reaches the provider. The added answer has no
/// durable identity, like the system prompt, so compaction never counts it.
/// Results with no call to answer, and rows with no call id, are left out.
pub(crate) fn context_from_snapshot(
    rows: &[DaContextMessage],
) -> Result<Vec<ContextMessage>, ContextHistoryError> {
    let mut previous_sequence = None;
    let mut answers: HashMap<&str, &DaContextMessage> = HashMap::new();
    for row in rows {
        if previous_sequence.is_some_and(|previous| row.sequence_no <= previous) {
            return Err(ContextHistoryError::InvalidSequence);
        }
        previous_sequence = Some(row.sequence_no);
        if row.role == ContextMessageRole::Tool {
            let Some(call_id) = row.tool_call_id.as_deref() else {
                continue;
            };
            if answers.insert(call_id, row).is_some() {
                return Err(ContextHistoryError::DuplicateToolResult);
            }
        }
    }

    let mut consumed = HashSet::new();
    let mut context = Vec::with_capacity(rows.len());
    for row in rows {
        match row.role {
            ContextMessageRole::User => {
                push_snapshot_text(&mut context, ContextRole::User, row)?;
            }
            ContextMessageRole::System => {
                push_snapshot_text(&mut context, ContextRole::System, row)?;
            }
            ContextMessageRole::Assistant => {
                let calls = requested_context_calls(row.tool_calls.as_ref());
                if calls.is_empty() {
                    push_snapshot_text(&mut context, ContextRole::Assistant, row)?;
                    continue;
                }
                if calls.iter().any(|call| {
                    answers.contains_key(call.call_id.as_str())
                        && !consumed.insert(call.call_id.clone())
                }) {
                    return Err(ContextHistoryError::IncompleteToolExchange);
                }
                context.push(
                    ContextMessage::assistant_tool_calls(row.content.clone(), calls.clone())
                        .with_durable_identity(durable_identity(row)),
                );
                for call in calls {
                    let Some(answer) = answers.get(call.call_id.as_str()) else {
                        tracing::warn!(
                            tool = %call.name,
                            "context snapshot: a tool call has no recorded result; answering it as unfinished"
                        );
                        context.push(ContextMessage::tool_result(
                            call.call_id,
                            call.name,
                            UNANSWERED_CALL_ANSWER.to_owned(),
                        ));
                        continue;
                    };
                    context.push(
                        ContextMessage::tool_result(
                            call.call_id,
                            answer.name.clone().unwrap_or(call.name),
                            answer.content.clone(),
                        )
                        .with_durable_identity(durable_identity(answer))
                        .with_durable_tool_result_payloads(ready_payloads(answer)?),
                    );
                }
            }
            ContextMessageRole::Tool => {}
        }
    }
    Ok(context)
}

fn push_snapshot_text(
    context: &mut Vec<ContextMessage>,
    role: ContextRole,
    row: &DaContextMessage,
) -> Result<(), ContextHistoryError> {
    if row.content.trim().is_empty() {
        return Err(ContextHistoryError::EmptyMessage);
    }
    context.push(
        ContextMessage::new(role, row.content.clone()).with_durable_identity(durable_identity(row)),
    );
    Ok(())
}

fn durable_identity(row: &DaContextMessage) -> DurableMessageIdentity {
    DurableMessageIdentity {
        id: row.id,
        role: row.role.to_string(),
        sequence_no: row.sequence_no,
    }
}

fn ready_payloads(
    row: &DaContextMessage,
) -> Result<Vec<AdmittedToolResultPayload>, ContextHistoryError> {
    row.tool_result_payloads
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|payload| {
            matches!(payload.status, PayloadStatus::Ready) && payload.retrieval_available
        })
        .map(|payload| {
            let byte_length = usize::try_from(payload.byte_length)
                .ok()
                .filter(|length| *length > 0)
                .ok_or(ContextHistoryError::InvalidPayloadDescriptor)?;
            if payload.sha256.len() != 64
                || !payload.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                || payload.media_type.to_string() != "application/json"
                || payload.serialization_format.to_string() != "json"
                || payload.serialization_version != 1
                || payload.content_encoding != "identity"
            {
                return Err(ContextHistoryError::InvalidPayloadDescriptor);
            }
            Ok(AdmittedToolResultPayload {
                id: payload.id,
                byte_length,
                sha256: payload.sha256.clone(),
                media_type: payload.media_type.to_string(),
                serialization_format: payload.serialization_format.to_string(),
                serialization_version: payload.serialization_version,
                content_encoding: payload.content_encoding.clone(),
            })
        })
        .collect()
}

fn requested_context_calls(tool_calls: Option<&ChatMessageToolCalls>) -> Vec<ContextToolCall> {
    let Some(ChatMessageToolCalls::Array(calls)) = tool_calls else {
        return Vec::new();
    };
    calls
        .iter()
        .filter_map(|call| {
            let call_id = call.get("id")?.as_str()?;
            let function = call.get("function")?;
            let name = function.get("name")?.as_str()?;
            Some(ContextToolCall::new(
                call_id,
                name,
                arguments_text(function.get("arguments")),
            ))
        })
        .collect()
}

/// Projects durable conversation rows into the model's context, restoring each
/// turn's tool round.
///
/// Rows may arrive in any order and are ordered here by `sequence_no` — the
/// column DA allocates server-side under an advisory lock, and the only total
/// order over a conversation (`created_at` ties within a round).
///
/// Superseded rows are dropped: a conversation rewound to a checkpoint must
/// replay as the user sees it, not as it once was.
#[cfg(test)]
pub(crate) fn context_from_history(rows: &[ChatMessage]) -> Vec<ContextMessage> {
    let mut ordered: Vec<&ChatMessage> = rows
        .iter()
        .filter(|row| row.superseded_by_checkpoint_id.is_none())
        .collect();
    ordered.sort_by_key(|row| row.sequence_no);

    let mut answers: HashMap<&str, &ChatMessage> = HashMap::new();
    for row in &ordered {
        if !row.role.eq_ignore_ascii_case("tool") {
            continue;
        }
        if let Some(call_id) = row.tool_call_id.as_deref() {
            // First answer wins. A second row for one call id is a duplicate
            // write, and emitting both would advertise one call answered twice —
            // the same shape a provider refuses as an unmatched answer.
            answers.entry(call_id).or_insert(row);
        }
    }

    let mut consumed: HashSet<String> = HashSet::new();
    let mut context = Vec::with_capacity(ordered.len());
    for row in ordered {
        match role_of(&row.role) {
            Some(ContextRole::User) => push_text(&mut context, ContextRole::User, &row.content),
            Some(ContextRole::System) => push_text(&mut context, ContextRole::System, &row.content),
            Some(ContextRole::Assistant) => {
                // One answer answers one call: a call id already spent by an
                // earlier turn is not advertised again, or the request carries
                // two turns claiming the same answer.
                let answered = requested_calls(row)
                    .into_iter()
                    .filter(|call| {
                        answers.contains_key(call.call_id.as_str())
                            && consumed.insert(call.call_id.clone())
                    })
                    .collect::<Vec<_>>();
                if answered.is_empty() {
                    // Either a plain answer, or a round whose answers never
                    // landed. Both replay as prose; a call with no answer cannot
                    // be sent at all.
                    push_text(&mut context, ContextRole::Assistant, &row.content);
                    continue;
                }
                context.push(ContextMessage::assistant_tool_calls(
                    row.content.clone(),
                    answered.clone(),
                ));
                for call in answered {
                    let answer = answers
                        .get(call.call_id.as_str())
                        .expect("only answered calls reach here");
                    context.push(ContextMessage::tool_result(
                        call.call_id,
                        answer.name.clone().unwrap_or(call.name),
                        answer.content.clone(),
                    ));
                }
            }
            // Emitted above, beside the call it answers. On its own it names a
            // call the request never made.
            Some(ContextRole::Tool) | None => {}
        }
    }
    context
}

#[cfg(test)]
fn role_of(role: &str) -> Option<ContextRole> {
    match role.to_ascii_lowercase().as_str() {
        "user" => Some(ContextRole::User),
        "assistant" => Some(ContextRole::Assistant),
        "system" => Some(ContextRole::System),
        "tool" => Some(ContextRole::Tool),
        _ => None,
    }
}

/// An empty turn is dropped rather than sent: some providers reject a message
/// with no content, and none can do anything with one.
#[cfg(test)]
fn push_text(context: &mut Vec<ContextMessage>, role: ContextRole, content: &str) {
    if content.trim().is_empty() {
        return;
    }
    context.push(ContextMessage::new(role, content.to_owned()));
}

/// The calls one assistant row advertises, in the order it advertised them.
///
/// Only the array form is understood. DA's column also admits an object, which
/// carries no order over its entries — a round is a sequence, so a row shaped
/// that way is replayed as prose instead of guessed at.
#[cfg(test)]
fn requested_calls(row: &ChatMessage) -> Vec<ContextToolCall> {
    let Some(ChatMessageToolCalls::Array(calls)) = row.tool_calls.as_ref() else {
        return Vec::new();
    };
    calls
        .iter()
        .filter_map(|call| {
            let call_id = call.get("id")?.as_str()?;
            let function = call.get("function")?;
            let name = function.get("name")?.as_str()?;
            Some(ContextToolCall::new(
                call_id.to_owned(),
                name.to_owned(),
                arguments_text(function.get("arguments")),
            ))
        })
        .collect()
}

/// Provider arguments are JSON *text*. A row holding an object instead is
/// re-serialized rather than dropped: the call was made, and the shape a
/// provider wants is recoverable from either.
fn arguments_text(arguments: Option<&serde_json::Value>) -> String {
    match arguments {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned()),
        None => "{}".to_owned(),
    }
}
