//! Direct unit coverage for `agent::context`'s tool-call carriage.
//!
//! Task 9a turned this module from a dead seam into the live representation
//! of a tool round trip: a `ContextMessage` can now carry the structured
//! calls an assistant turn requested (`tool_calls`) and the call a tool
//! result answers (`responds_to`), because no provider wire format can
//! reconstruct that association from flat text. OpenAI keys results by
//! `tool_call_id`, Anthropic by `tool_use_id`, and Google by function
//! *name* — so both the id and the name have to survive the provider-neutral
//! layer, or an adapter cannot emit a valid follow-up request at all.

use crate::agent::context::{
    ContextBuilder, ContextError, ContextMessage, ContextRole, ContextToolCall,
};

#[test]
fn assistant_tool_calls_are_carried_structurally_not_as_text() {
    let message = ContextMessage::assistant_tool_calls(
        "",
        vec![ContextToolCall::new(
            "call_1",
            "project_search",
            r#"{"query":"budget"}"#,
        )],
    );

    assert_eq!(message.role, ContextRole::Assistant);
    assert_eq!(message.tool_calls.len(), 1);
    assert_eq!(message.tool_calls[0].call_id, "call_1");
    assert_eq!(message.tool_calls[0].name, "project_search");
    assert_eq!(message.tool_calls[0].arguments, r#"{"query":"budget"}"#);
    assert!(message.responds_to.is_none());
}

#[test]
fn tool_result_records_the_call_it_answers() {
    let message = ContextMessage::tool_result("call_1", "project_search", "3 matches");

    assert_eq!(message.role, ContextRole::Tool);
    let responds_to = message
        .responds_to
        .as_ref()
        .expect("a tool result must name the call it answers");
    assert_eq!(responds_to.call_id, "call_1");
    assert_eq!(
        responds_to.name, "project_search",
        "Google's functionResponse is keyed by name, not id — the name must survive"
    );
}

#[test]
fn byte_len_counts_tool_call_arguments_against_the_context_budget() {
    let arguments = r#"{"query":"budget"}"#;
    let message = ContextMessage::assistant_tool_calls(
        "",
        vec![ContextToolCall::new("call_1", "project_search", arguments)],
    );

    assert!(
        message.byte_len() >= arguments.len(),
        "a tool-call payload is real context the model pays for; excluding it from \
         byte_len would let a tool-heavy round silently blow the run's byte budget"
    );
}

#[test]
fn builder_accepts_a_pure_tool_call_turn_with_no_text() {
    // A model that answers only with tool calls emits an assistant turn whose
    // text content is empty. Rejecting it as `EmptyContent` would make the
    // single most common tool round trip unrepresentable.
    let mut context = ContextBuilder::new(8, 4_096);

    context
        .push(ContextMessage::assistant_tool_calls(
            "",
            vec![ContextToolCall::new("call_1", "project_search", "{}")],
        ))
        .expect("a pure tool-call assistant turn must be representable");

    assert_eq!(context.len(), 1);
}

#[test]
fn builder_still_rejects_a_message_that_is_empty_in_every_dimension() {
    let mut context = ContextBuilder::new(8, 4_096);

    assert_eq!(
        context.push(ContextMessage::new(ContextRole::User, "")),
        Err(ContextError::EmptyContent),
        "relaxing the empty check for tool calls must not relax it for plain text"
    );
}

#[test]
fn tool_result_does_not_apply_a_second_hidden_prefix_limit() {
    let input = format!("{}€tail", "a".repeat(40 * 1024));
    let message = ContextMessage::tool_result("call_1", "project_search", input.clone());

    assert_eq!(message.role, ContextRole::Tool);
    assert_eq!(message.content, input);
    assert!(!message.truncated);
}

/// Asserts the invariant every provider validates: no tool result names a call
/// that is absent, and no call goes unanswered.
///
/// Anthropic rejects the first with `unexpected tool_use_id`; OpenAI rejects a
/// `role: "tool"` message whose `tool_call_id` names nothing, and rejects an
/// assistant turn whose calls are not all answered. Either way the whole request
/// fails, so a context that violates this does not merely lose history — it ends
/// the run.
fn assert_tool_pairing_is_intact(messages: &[ContextMessage]) {
    let offered: Vec<&str> = messages
        .iter()
        .flat_map(|message| message.tool_calls.iter())
        .map(|call| call.call_id.as_str())
        .collect();
    let answered: Vec<&str> = messages
        .iter()
        .filter_map(|message| message.responds_to.as_ref())
        .map(|answered| answered.call_id.as_str())
        .collect();

    for call_id in &offered {
        assert!(
            answered.contains(call_id),
            "call {call_id} was offered to the model and left unanswered; \
             the provider rejects the request. messages: {messages:#?}"
        );
    }
    for call_id in &answered {
        assert!(
            offered.contains(call_id),
            "tool result answers {call_id}, which no surviving assistant turn \
             requested; the provider rejects the request. messages: {messages:#?}"
        );
    }
}

/// One round: an assistant turn asking for `calls` parallel tools, then a result
/// for each, every result `result_bytes` long.
fn push_tool_round(context: &mut ContextBuilder, round: usize, calls: usize, result_bytes: usize) {
    let ids: Vec<String> = (0..calls)
        .map(|call| format!("call_{round}_{call}"))
        .collect();
    context
        .push(ContextMessage::assistant_tool_calls(
            "",
            ids.iter()
                .map(|id| ContextToolCall::new(id.clone(), "project_search", r#"{"q":"x"}"#))
                .collect(),
        ))
        .expect("an assistant tool-call turn always fits a budget sized for one");
    for id in &ids {
        context
            .push(ContextMessage::tool_result(
                id.clone(),
                "project_search",
                "r".repeat(result_bytes),
            ))
            .expect("a bounded tool result always fits a budget sized for one");
    }
}

#[test]
fn byte_pressure_rejects_without_mutating_exact_history() {
    // Sized so that freeing the assistant turn *alone* is enough to seat what
    // comes next: it costs 93 bytes against three 512-byte results, making it
    // the cheapest message in the round and the first one a drop-oldest policy
    // reaches for. Freeing it leaves three results answering a turn that is no
    // longer there, which is the request every provider rejects.
    //
    // The margin matters as much as the pressure. Sustained pressure hides the
    // bug: later evictions carry off the orphans that earlier ones created, so
    // the final state looks healthy. Stopping one small message past the budget
    // is what leaves the damage visible.
    let mut context = ContextBuilder::new(64, 1_700);
    push_tool_round(&mut context, 0, 3, 512);
    let before = context.clone().build();
    let result = context.push(ContextMessage::new(ContextRole::Assistant, "x".repeat(100)));

    assert_eq!(result, Err(ContextError::ByteLimitExceeded));
    assert_eq!(context.build(), before);
}

#[test]
fn message_pressure_rejects_without_mutating_exact_history() {
    // The same stop-just-past-the-budget shape against the count budget, which
    // clearing cannot relieve. Two rounds of three calls fill it exactly; the
    // next message forces exactly one eviction.
    let mut context = ContextBuilder::new(9, 1024 * 1024);
    context
        .push(ContextMessage::new(ContextRole::System, "you are xfalcon"))
        .expect("system");
    push_tool_round(&mut context, 0, 3, 16);
    push_tool_round(&mut context, 1, 3, 16);
    let before = context.clone().build();
    let result = context.push(ContextMessage::new(ContextRole::User, "and last quarter?"));

    assert_eq!(result, Err(ContextError::MessageLimitExceeded));
    assert_eq!(context.build(), before);
    assert_tool_pairing_is_intact(&before);
}

#[test]
fn the_run_status_turn_is_last_so_the_model_reads_it_before_deciding() {
    let mut context = ContextBuilder::new(64, 8_192);
    context
        .push(ContextMessage::new(
            ContextRole::User,
            "how much did we spend?",
        ))
        .expect("user");
    push_tool_round(&mut context, 0, 1, 64);
    context
        .set_run_status("<run-status>Tool rounds used: 1 of 8.</run-status>")
        .expect("a one-sentence status always fits");

    let messages = context.build();
    assert_eq!(
        messages.last().expect("a last message").content,
        "<run-status>Tool rounds used: 1 of 8.</run-status>"
    );
}

#[test]
fn only_the_newest_run_status_survives() {
    // A stale "3 rounds remaining" is worse than none, so the turn is replaced
    // rather than appended.
    let mut context = ContextBuilder::new(64, 8_192);
    context
        .set_run_status("<run-status>Tool rounds used: 1 of 8.</run-status>")
        .expect("first status");
    context
        .set_run_status("<run-status>Tool rounds used: 2 of 8.</run-status>")
        .expect("second status");

    let messages = context.build();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].content.contains("2 of 8"));
}

#[test]
fn the_run_status_turn_occupies_no_message_slot_and_cannot_be_evicted() {
    // It reports on the rounds; letting eviction drop it to seat one of those
    // rounds — or letting it consume a slot a round needed — would be backwards.
    let mut context = ContextBuilder::new(3, 1024 * 1024);
    context
        .set_run_status("<run-status>Tool rounds used: 1 of 8.</run-status>")
        .expect("status");
    push_tool_round(&mut context, 0, 2, 32);

    let messages = context.build();
    assert_eq!(messages.len(), 4, "3 round messages plus the status");
    assert_tool_pairing_is_intact(&messages);
    assert!(messages.last().expect("last").content.contains("1 of 8"));
}

#[test]
fn the_run_status_turn_is_charged_against_the_byte_budget() {
    let mut context = ContextBuilder::new(64, 1024 * 1024);
    let before = context.total_bytes();
    context
        .set_run_status("<run-status>Tool rounds used: 1 of 8.</run-status>")
        .expect("status");

    assert!(
        context.total_bytes() > before,
        "a status the model pays for must count against what it may read"
    );
}

#[test]
fn clearing_the_run_status_releases_its_bytes() {
    let mut context = ContextBuilder::new(64, 1024 * 1024);
    let before = context.total_bytes();
    context
        .set_run_status("<run-status>x</run-status>")
        .expect("set");
    context.set_run_status("").expect("clear");

    assert_eq!(context.total_bytes(), before);
    assert!(context.build().is_empty());
}

/// Images ride a budget of their own (spec: context-image-budget).
///
/// The failure these exist to prevent: `ContextBuilder::push` refuses any single
/// message larger than a budget, because evicting *other* messages cannot shrink
/// one oversized message. So the moment image bytes were charged to the prose
/// budget, an ordinary photo stopped degrading the context and started failing
/// the run.
mod image_budget {
    use crate::agent::context::{
        ContextAttachment, ContextBuilder, ContextError, ContextMessage, ContextRole,
        ImageMediaType,
    };

    const PROSE_BUDGET: usize = 2_048;
    const IMAGE_BUDGET: usize = 4 * 1024 * 1024;
    const PER_ATTACHMENT: usize = 2 * 1024 * 1024;

    fn builder() -> ContextBuilder {
        ContextBuilder::with_image_budget(64, PROSE_BUDGET, IMAGE_BUDGET, PER_ATTACHMENT)
    }

    fn image(bytes: usize) -> ContextAttachment {
        ContextAttachment::Image {
            media_type: ImageMediaType::Png,
            bytes: vec![0u8; bytes],
        }
    }

    #[test]
    fn an_image_far_larger_than_the_prose_budget_is_still_accepted() {
        // THE test this change exists for. One mebibyte against a 2 KiB prose
        // budget: before the split this returned ByteLimitExceeded and took the
        // run with it.
        let mut context = builder();

        context
            .push(ContextMessage::user_with_attachments(
                "what is in this picture?",
                vec![image(1024 * 1024)],
            ))
            .expect("an image is not weighed as prose");

        assert_eq!(context.len(), 1);
        assert_eq!(context.total_image_bytes(), 1024 * 1024);
        // And it did not consume the prose budget it does not belong to.
        assert!(context.total_bytes() < PROSE_BUDGET);
    }

    #[test]
    fn separating_the_budgets_did_not_loosen_the_prose_one() {
        let mut context = builder();

        let outcome = context.push(ContextMessage::new(
            ContextRole::User,
            "x".repeat(PROSE_BUDGET + 1),
        ));

        assert_eq!(outcome, Err(ContextError::ByteLimitExceeded));
    }

    #[test]
    fn a_picture_with_no_words_is_representable() {
        let mut context = builder();

        context
            .push(ContextMessage::user_with_attachments("", vec![image(64)]))
            .expect("handing over an image without commentary is the ordinary case");

        assert_eq!(context.len(), 1);
    }

    #[test]
    fn a_message_empty_in_every_dimension_is_still_refused() {
        let mut context = builder();

        let outcome = context.push(ContextMessage::user_with_attachments("", Vec::new()));

        assert_eq!(outcome, Err(ContextError::EmptyContent));
    }

    #[test]
    fn one_oversized_attachment_is_refused_rather_than_trimmed() {
        // Half an image is not an image, so this is reported against the file
        // instead of being silently corrupted.
        let mut context = builder();

        let outcome = context.push(ContextMessage::user_with_attachments(
            "look",
            vec![image(PER_ATTACHMENT + 1)],
        ));

        assert_eq!(outcome, Err(ContextError::AttachmentTooLarge));
        assert!(context.is_empty());
    }

    #[test]
    fn image_pressure_rejects_without_releasing_any_bytes() {
        let mut context = builder();
        for index in 0..2 {
            context
                .push(ContextMessage::user_with_attachments(
                    format!("photo {index}"),
                    vec![image(PER_ATTACHMENT)],
                ))
                .expect("each image fits on its own");
        }
        let before = context.clone().build();

        let result = context.push(ContextMessage::user_with_attachments(
            "photo 2",
            vec![image(PER_ATTACHMENT)],
        ));

        assert_eq!(result, Err(ContextError::AttachmentTooLarge));
        assert_eq!(context.build(), before);
    }

    #[test]
    fn a_builder_with_no_image_budget_admits_no_image() {
        // `ContextBuilder::new` is the pre-attachment constructor; every existing
        // caller keeps its exact behavior, which means refusing images outright
        // rather than silently carrying unbudgeted bytes.
        let mut context = ContextBuilder::new(64, PROSE_BUDGET);

        let outcome = context.push(ContextMessage::user_with_attachments(
            "look",
            vec![image(1)],
        ));

        assert_eq!(outcome, Err(ContextError::AttachmentTooLarge));
    }

    #[test]
    fn a_recognized_media_type_round_trips_and_an_unknown_one_is_refused() {
        assert_eq!(
            ImageMediaType::parse("image/png"),
            Some(ImageMediaType::Png)
        );
        // Providers name `image/jpeg`; `image/jpg` is what file extensions say.
        assert_eq!(
            ImageMediaType::parse("IMAGE/JPG"),
            Some(ImageMediaType::Jpeg)
        );
        assert_eq!(ImageMediaType::Webp.as_str(), "image/webp");
        assert_eq!(ImageMediaType::parse("application/pdf"), None);
        assert_eq!(ImageMediaType::parse("image/svg+xml"), None);
    }
}
