use crate::agent::context::{
    ContextAttachment, ContextMessage, ContextRole, ContextToolCall, DurableMessageIdentity,
    ImageMediaType,
};
use crate::agent::context_budget::{
    largest_bootstrap_prefix, request_input_tokens, text_tokens, ContextCapacity,
    ContextCapacityError, UsageAnchor, IMAGE_ESTIMATED_TOKENS, MAX_PROVIDER_OUTPUT_TOKENS_PER_TURN,
    OPENROUTER_COMPACTION_TRIGGER_TOKENS, PROTOCOL_SAFETY_TOKENS, UNMEASURED_BLOCK_OVERHEAD_TOKENS,
};
use crate::providers::ProviderToolSchema;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn durable(role: ContextRole, content: String, sequence_no: i64) -> ContextMessage {
    ContextMessage::new(role, content).with_durable_identity(DurableMessageIdentity {
        id: Uuid::from_u128(sequence_no as u128),
        role: match role {
            ContextRole::System => "system",
            ContextRole::User => "user",
            ContextRole::Assistant => "assistant",
            ContextRole::Tool => "tool",
        }
        .to_owned(),
        sequence_no,
    })
}

#[test]
fn million_token_capacity_reserves_output_and_protocol_space() {
    let capacity = ContextCapacity::new(1_000_000, 128_000).expect("valid capacity");

    assert_eq!(capacity.context_tokens(), 1_000_000);
    assert_eq!(capacity.max_output_tokens(), 128_000);
    assert_eq!(
        capacity.hard_input_tokens(),
        1_000_000 - 128_000 - PROTOCOL_SAFETY_TOKENS
    );
    assert!(capacity.admits(capacity.hard_input_tokens()));
    assert!(!capacity.admits(capacity.hard_input_tokens() + 1));
    assert_eq!(
        capacity.hard_input_tokens() - OPENROUTER_COMPACTION_TRIGGER_TOKENS,
        155_616,
        "automatic compaction must leave room for a large final tool round"
    );
}

#[test]
fn capacity_rejects_unsupported_or_inconsistent_catalog_values() {
    let grok = ContextCapacity::new(500_000, 450_000).expect("Grok catalog capacity");
    assert_eq!(
        grok.max_output_tokens(),
        MAX_PROVIDER_OUTPUT_TOKENS_PER_TURN
    );
    assert_eq!(grok.hard_input_tokens(), 355_616);
    let smaller_output = ContextCapacity::new(500_000, 64_000).expect("smaller DA ceiling");
    assert_eq!(smaller_output.max_output_tokens(), 64_000);
    let high_ceiling = ContextCapacity::new(500_000, 499_999).expect("high DA ceiling");
    assert_eq!(
        high_ceiling.max_output_tokens(),
        MAX_PROVIDER_OUTPUT_TOKENS_PER_TURN
    );
    assert_eq!(high_ceiling.hard_input_tokens(), 355_616);
    assert_eq!(
        ContextCapacity::new(PROTOCOL_SAFETY_TOKENS as i64, 1),
        Err(ContextCapacityError::ContextTooSmall)
    );
    assert_eq!(
        ContextCapacity::new(1_000_000, 0),
        Err(ContextCapacityError::OutputLimitMissing)
    );
    assert_eq!(
        ContextCapacity::new(1_000_000, 1_000_000),
        Err(ContextCapacityError::NoInputCapacity)
    );
}

#[test]
fn unmeasured_bytes_are_charged_at_the_estimate_ratio_plus_block_framing() {
    let capacity = ContextCapacity::new(1_000_000, 128_000).expect("valid capacity");

    assert_eq!(
        capacity
            .unmeasured_input_tokens(64 * 1024, 3)
            .expect("estimate fits"),
        text_tokens(64 * 1024) + 3 * UNMEASURED_BLOCK_OVERHEAD_TOKENS
    );
}

#[test]
fn text_is_not_charged_a_whole_token_per_byte() {
    // One token per byte overstated a typical request three to four times, so
    // the guard fired long before the provider's own trigger.
    let message = ContextMessage::new(ContextRole::User, "x".repeat(30_000));
    let tokens = request_input_tokens(&[message], &[], &[]).expect("estimate");

    assert!(tokens < 12_000, "30 KB of text scored {tokens} tokens");
    assert!(tokens > 9_000, "but it must still be charged: {tokens}");
}

#[test]
fn an_image_costs_the_same_whatever_its_file_size() {
    let image = |bytes: usize| {
        ContextMessage::user_with_attachments(
            "look",
            vec![ContextAttachment::Image {
                media_type: ImageMediaType::Png,
                bytes: vec![0; bytes],
            }],
        )
    };
    let small = request_input_tokens(&[image(3_000)], &[], &[]).expect("small");
    let large = request_input_tokens(&[image(15_000_000)], &[], &[]).expect("large");

    assert_eq!(
        small, large,
        "a provider prices an image by pixels, not bytes"
    );
    assert!(small >= IMAGE_ESTIMATED_TOKENS);
}

#[test]
fn request_admission_counts_tools_reasoning_and_the_active_barrier() {
    let message = ContextMessage::new(ContextRole::User, "question")
        .with_reasoning_details(vec![json!({"opaque": "reasoning"})]);
    let tool = ProviderToolSchema {
        name: "query".to_owned(),
        description: "query data".to_owned(),
        input_schema: json!({"type": "object"}),
    };
    let bare = request_input_tokens(std::slice::from_ref(&message), &[], &[]).expect("bare");
    let with_tools = request_input_tokens(
        std::slice::from_ref(&message),
        std::slice::from_ref(&tool),
        &[],
    )
    .expect("tools");
    let complete = request_input_tokens(
        &[message],
        &[tool],
        &[json!({"type": "compaction", "content": "opaque"})],
    )
    .expect("complete");

    assert!(with_tools > bare, "tool schemas must be charged");
    assert!(complete > with_tools, "active barriers must be charged");
}

#[test]
fn an_anchor_adds_the_growth_since_the_provider_counted() {
    let anchor = UsageAnchor::new(300_000, 100_000).expect("anchor");

    assert_eq!(anchor.estimate(100_000), 300_000);
    assert_eq!(
        anchor.estimate(110_000),
        310_000,
        "new messages are added on top of the provider's own count"
    );
}

#[test]
fn an_anchor_scales_down_when_the_context_shrinks() {
    let anchor = UsageAnchor::new(300_000, 100_000).expect("anchor");

    // A context a fifth the size keeps the provider's ratio rather than
    // subtracting, which would understate a request made mostly of what stayed.
    assert_eq!(anchor.estimate(20_000), 60_000);
}

#[test]
fn a_count_far_below_the_estimate_is_not_trusted() {
    // A server that reports only the tokens it newly read (it reused a cached
    // prefix) would make a full context look nearly empty.
    assert_eq!(UsageAnchor::new(400, 100_000), None);
    assert_eq!(UsageAnchor::new(39_999, 100_000), None);
    // Real text is four or five bytes per token against the estimate's three,
    // so a count a bit over half the estimate is ordinary.
    let anchor = UsageAnchor::new(60_000, 100_000).expect("a plausible count anchors");
    assert_eq!(anchor.estimate(100_000), 60_000);
    assert!(UsageAnchor::new(40_000, 100_000).is_some());
}

#[test]
fn nothing_anchors_on_a_zero_count() {
    assert_eq!(UsageAnchor::new(0, 100), None);
    assert_eq!(UsageAnchor::new(100, 0), None);
}

#[test]
fn largest_prefix_ends_after_a_complete_tool_group_and_has_the_da_digest() {
    let first_id = Uuid::from_u128(1);
    // 700 KB of text is about 233,000 estimated tokens: more than a provider
    // needs before it will compact, well under the limit below.
    let user = durable(ContextRole::User, "u".repeat(700_000), 1);
    let assistant =
        ContextMessage::assistant_tool_calls("", vec![ContextToolCall::new("call", "query", "{}")])
            .with_durable_identity(DurableMessageIdentity {
                id: Uuid::from_u128(2),
                role: "assistant".to_owned(),
                sequence_no: 2,
            });
    let result = ContextMessage::tool_result("call", "query", "ok").with_durable_identity(
        DurableMessageIdentity {
            id: Uuid::from_u128(3),
            role: "tool".to_owned(),
            sequence_no: 3,
        },
    );
    let too_large = durable(ContextRole::User, "z".repeat(600_000), 4);
    let prefix = largest_bootstrap_prefix(
        400_000,
        &[],
        &[user, assistant, result, too_large],
        &[],
        &[],
        0,
    )
    .expect("valid history")
    .expect("compactable prefix");

    assert_eq!(prefix.message_count, 3);
    assert_eq!(prefix.through_sequence_no, 3);
    let manifest = json!([
        {"active": true, "id": first_id, "role": "user", "sequence_no": 1},
        {"active": true, "id": Uuid::from_u128(2), "role": "assistant", "sequence_no": 2},
        {"active": true, "id": Uuid::from_u128(3), "role": "tool", "sequence_no": 3}
    ]);
    let expected = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&manifest).expect("canonical manifest"))
    );
    assert_eq!(prefix.source_message_ids_sha256, expected);
}

#[test]
fn a_prefix_too_small_for_a_provider_to_compact_fails_as_indivisible_overhead() {
    let history = vec![durable(ContextRole::User, "small".to_owned(), 1)];
    assert_eq!(
        largest_bootstrap_prefix(200_000, &[], &history, &[], &[], 0),
        Err(ContextCapacityError::IndivisibleOverhead)
    );
}

#[test]
fn bootstrap_never_covers_replay_owned_by_a_later_tool_turn() {
    let history = vec![
        durable(ContextRole::User, "u".repeat(270_000), 1),
        ContextMessage::new(ContextRole::Assistant, "Continue reading the next file."),
        ContextMessage::new(ContextRole::User, "<run-status>retry</run-status>"),
        ContextMessage::assistant_tool_calls("", vec![ContextToolCall::new("call", "query", "{}")])
            .with_durable_identity(DurableMessageIdentity {
                id: Uuid::from_u128(2),
                role: "assistant".to_owned(),
                sequence_no: 2,
            }),
        ContextMessage::tool_result("call", "query", "r".repeat(600_000)).with_durable_identity(
            DurableMessageIdentity {
                id: Uuid::from_u128(3),
                role: "tool".to_owned(),
                sequence_no: 3,
            },
        ),
    ];
    let prefix = largest_bootstrap_prefix(200_000, &[], &history, &[], &[], 0)
        .expect("valid saved prefix")
        .expect("first user message fits");
    assert_eq!(prefix.message_count, 1);
    assert_eq!(prefix.source_message_count, 1);
    assert_eq!(prefix.through_sequence_no, 1);
}
