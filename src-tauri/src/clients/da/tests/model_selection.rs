use indexmap::IndexMap;
use uuid::Uuid;

use super::super::{
    chat_api::{
        pick_default_model, select_model, select_model_by_id, select_offered_model,
        supported_models, CustomModelRoute, DaChatApiError, ModelExecutionMode, GROK_MODEL_ID,
        HAIKU_MODEL_ID, LUNA_MODEL_ID, MIN_NATIVE_CONTEXT_TOKENS, RETIRED_LUNA_MODEL_ID,
        SOL_MODEL_ID, SONNET_MODEL_ID,
    },
    generated,
};

fn model(model_id: &str, provider: &str) -> generated::ModelEntity {
    generated::ModelEntity {
        id: Uuid::nil(),
        model_id: model_id.to_owned(),
        provider: provider.to_owned(),
        provider_parent: provider.to_owned(),
        provider_sub: None,
        display_name: "Tenant model".to_owned(),
        context_length: MIN_NATIVE_CONTEXT_TOKENS,
        max_output_tokens: Some(64_000),
        input_price_per_m: 1.0,
        output_price_per_m: 2.0,
        released_at: None,
        is_chat: true,
        is_enabled: true,
        supports_streaming: true,
        supports_tools: true,
        supports_vision: false,
        metadata: Some(IndexMap::new()),
        created_at: None,
        updated_at: None,
        is_active: true,
        tenant_id: "tenant".to_owned(),
        supports_reasoning: false,
        supports_extended_thinking: false,
        reasoning_param_style: "none".to_owned(),
        default_thinking_budget_tokens: None,
        owner_user_id: None,
        base_url: None,
        upstream_model_id: None,
        provider_key_id: None,
    }
}

fn custom_model() -> generated::ModelEntity {
    let id = Uuid::parse_str("b575f456-1c79-4bc1-a80a-7892a44c4d2a").expect("fixture UUID");
    let key_id = Uuid::parse_str("8175d447-cfe7-4bbb-92b7-339b2ded2ab8").expect("fixture UUID");
    let mut model = model(&format!("custom/{id}"), "custom_openai");
    model.id = id;
    model.provider_parent = "custom_openai".to_owned();
    model.owner_user_id = Some("caller-subject".to_owned());
    model.base_url = Some("https://models.example.com/v1".to_owned());
    model.upstream_model_id = Some("organization/private-model".to_owned());
    model.provider_key_id = Some(key_id);
    model.context_length = 128_000;
    model.max_output_tokens = Some(16_000);
    model
}

#[test]
fn selects_catalog_backed_openrouter_sonnet_and_carries_both_limits() {
    let selection = select_model(&[model(SONNET_MODEL_ID, "openrouter")]).expect("eligible Sonnet");

    assert_eq!(selection.model_id, SONNET_MODEL_ID);
    assert_eq!(selection.provider, "openrouter");
    assert_eq!(selection.context_length, 1_000_000);
    assert_eq!(selection.max_output_tokens, 64_000);
    assert_eq!(selection.custom, None);
    assert_eq!(
        selection.execution_mode,
        ModelExecutionMode::NativeCompaction
    );
}

#[test]
fn accepts_openrouter_latest_family_alias_without_rewriting_it() {
    let selection = select_model(&[model(SONNET_MODEL_ID, "openrouter")])
        .expect("eligible OpenRouter latest-family alias");

    assert_eq!(selection.model_id, SONNET_MODEL_ID);
    assert_eq!(selection.context_length, 1_000_000);
    assert_eq!(selection.max_output_tokens, 64_000);
}

#[test]
fn selects_exact_grok_and_luna_on_the_bounded_context_path() {
    let mut grok = model(GROK_MODEL_ID, "openrouter");
    grok.context_length = 500_000;
    grok.max_output_tokens = Some(450_000);
    let luna = model(LUNA_MODEL_ID, "openrouter");
    let rows = [grok, luna];

    let grok_selection = select_model_by_id(&rows, Some(GROK_MODEL_ID)).expect("Grok");
    let luna_selection = select_model_by_id(&rows, Some(LUNA_MODEL_ID)).expect("Luna");

    assert_eq!(
        grok_selection.execution_mode,
        ModelExecutionMode::BoundedContext
    );
    assert_eq!(grok_selection.max_output_tokens, 450_000);
    assert_eq!(
        luna_selection.execution_mode,
        ModelExecutionMode::BoundedContext
    );
    assert_eq!(luna_selection.max_output_tokens, 64_000);
}

#[test]
fn selects_sol_on_the_bounded_context_path() {
    let rows = [model(SOL_MODEL_ID, "openrouter")];

    let sol = select_model_by_id(&rows, Some(SOL_MODEL_ID)).expect("Sol");

    assert_eq!(sol.model_id, SOL_MODEL_ID);
    assert_eq!(sol.execution_mode, ModelExecutionMode::BoundedContext);
}

#[test]
fn selects_haiku_on_the_bounded_context_path() {
    let rows = [model(HAIKU_MODEL_ID, "openrouter")];

    let haiku = select_model_by_id(&rows, Some(HAIKU_MODEL_ID)).expect("Haiku");

    assert_eq!(haiku.model_id, HAIKU_MODEL_ID);
    assert_eq!(haiku.execution_mode, ModelExecutionMode::BoundedContext);
    assert_eq!(
        select_offered_model(&rows, HAIKU_MODEL_ID).map(|model| model.model_id),
        Ok(HAIKU_MODEL_ID.to_owned())
    );
}

#[test]
fn haiku_is_the_default_when_every_product_model_is_eligible() {
    let rows = [
        model(SOL_MODEL_ID, "openrouter"),
        model(SONNET_MODEL_ID, "openrouter"),
        model(HAIKU_MODEL_ID, "openrouter"),
    ];

    assert_eq!(
        select_model(&rows).expect("default").model_id,
        HAIKU_MODEL_ID
    );
    assert_eq!(
        pick_default_model(&rows).map(|row| row.model_id.as_str()),
        Some(HAIKU_MODEL_ID)
    );
}

#[test]
fn the_default_falls_through_to_sonnet_when_haiku_is_not_served() {
    let rows = [
        model(SOL_MODEL_ID, "openrouter"),
        model(SONNET_MODEL_ID, "openrouter"),
    ];

    assert_eq!(
        select_model(&rows).expect("default").model_id,
        SONNET_MODEL_ID
    );
}

#[test]
fn a_pinned_haiku_id_is_not_the_product_model() {
    let rows = [model("anthropic/claude-haiku-4.5", "openrouter")];

    assert_eq!(
        select_offered_model(&rows, "anthropic/claude-haiku-4.5"),
        Err(DaChatApiError::NoModel)
    );
}

#[test]
fn projects_only_the_five_product_models_in_stable_order() {
    let rows = vec![
        model(HAIKU_MODEL_ID, "openrouter"),
        model(SOL_MODEL_ID, "openrouter"),
        model(RETIRED_LUNA_MODEL_ID, "openrouter"),
        model(LUNA_MODEL_ID, "openrouter"),
        model("anthropic/claude-opus-4.6", "openrouter"),
        model(GROK_MODEL_ID, "openrouter"),
        model(SONNET_MODEL_ID, "openrouter"),
    ];

    let selected: Vec<_> = supported_models(&rows)
        .into_iter()
        .map(|row| row.model_id.as_str())
        .collect();

    assert_eq!(
        selected,
        vec![
            HAIKU_MODEL_ID,
            SONNET_MODEL_ID,
            GROK_MODEL_ID,
            LUNA_MODEL_ID,
            SOL_MODEL_ID
        ]
    );
}

#[test]
fn offers_caller_owned_custom_models_after_the_product_models() {
    let custom = custom_model();
    let custom_id = custom.model_id.clone();
    let custom_uuid = custom.id;
    let key_id = custom.provider_key_id.expect("fixture key");
    let rows = vec![
        custom,
        model(SOL_MODEL_ID, "openrouter"),
        model(SONNET_MODEL_ID, "openrouter"),
    ];

    let offered: Vec<_> = supported_models(&rows)
        .into_iter()
        .map(|row| row.model_id.as_str())
        .collect();
    let selected = select_offered_model(&rows, &custom_id).expect("eligible custom model");

    assert_eq!(
        offered,
        vec![SONNET_MODEL_ID, SOL_MODEL_ID, custom_id.as_str()]
    );
    assert_eq!(selected.model_id, custom_id);
    assert_eq!(selected.provider, "custom_openai");
    assert_eq!(selected.context_length, 128_000);
    assert_eq!(selected.max_output_tokens, 16_000);
    assert_eq!(selected.execution_mode, ModelExecutionMode::BoundedContext);
    assert_eq!(
        selected.custom,
        Some(CustomModelRoute {
            model_id: custom_uuid,
            base_url: "https://models.example.com/v1".to_owned(),
            upstream_model_id: "organization/private-model".to_owned(),
            provider_key_id: key_id,
        })
    );
}

#[test]
fn custom_models_never_replace_the_product_default() {
    let custom = custom_model();
    let rows = [custom, model(GROK_MODEL_ID, "openrouter")];

    let selected = select_model(&rows).expect("product fallback");

    assert_eq!(selected.model_id, GROK_MODEL_ID);
    assert_eq!(selected.custom, None);
    assert_eq!(
        select_model(&[custom_model()]),
        Err(DaChatApiError::NoModel)
    );
}

#[test]
fn a_removed_or_disabled_custom_model_has_a_distinct_selection_error() {
    let mut custom = custom_model();
    let model_id = custom.model_id.clone();
    custom.is_active = false;

    assert_eq!(
        select_offered_model(&[], &model_id),
        Err(DaChatApiError::CustomModelUnavailable)
    );
    assert_eq!(
        select_model_by_id(&[custom], Some(&model_id)),
        Err(DaChatApiError::CustomModelUnavailable)
    );
    assert_eq!(
        select_offered_model(&[], "custom/not-a-uuid"),
        Err(DaChatApiError::NoModel)
    );
}

#[test]
fn rejects_custom_models_without_complete_caller_owned_routing_metadata() {
    let mut missing_owner = custom_model();
    missing_owner.owner_user_id = None;
    let mut blank_owner = custom_model();
    blank_owner.owner_user_id = Some("   ".to_owned());
    let mut wrong_provider = custom_model();
    wrong_provider.provider = "openrouter".to_owned();
    let mut wrong_parent = custom_model();
    wrong_parent.provider_parent = "openrouter".to_owned();
    let mut missing_tenant = custom_model();
    missing_tenant.tenant_id = "   ".to_owned();
    let mut mismatched_id = custom_model();
    mismatched_id.id = Uuid::new_v4();
    let mut missing_url = custom_model();
    missing_url.base_url = None;
    let mut missing_upstream = custom_model();
    missing_upstream.upstream_model_id = None;
    let mut missing_key = custom_model();
    missing_key.provider_key_id = None;
    let mut nil_key = custom_model();
    nil_key.provider_key_id = Some(Uuid::nil());

    for row in [
        missing_owner,
        blank_owner,
        wrong_provider,
        wrong_parent,
        missing_tenant,
        mismatched_id,
        missing_url,
        missing_upstream,
        missing_key,
        nil_key,
    ] {
        let model_id = row.model_id.clone();
        assert_eq!(
            select_offered_model(&[row], &model_id),
            Err(DaChatApiError::CustomModelUnavailable),
            "invalid custom route was admitted for {model_id}"
        );
    }
}

#[test]
fn rejects_custom_models_that_cannot_execute_the_chat_contract() {
    let mut inactive = custom_model();
    inactive.is_active = false;
    let mut disabled = custom_model();
    disabled.is_enabled = false;
    let mut not_chat = custom_model();
    not_chat.is_chat = false;
    let mut no_streaming = custom_model();
    no_streaming.supports_streaming = false;
    let mut no_tools = custom_model();
    no_tools.supports_tools = false;
    let mut missing_output = custom_model();
    missing_output.max_output_tokens = None;
    let mut no_input_window = custom_model();
    no_input_window.max_output_tokens = Some(no_input_window.context_length);

    for row in [
        inactive,
        disabled,
        not_chat,
        no_streaming,
        no_tools,
        missing_output,
        no_input_window,
    ] {
        let model_id = row.model_id.clone();
        assert_eq!(
            select_offered_model(&[row], &model_id),
            Err(DaChatApiError::CustomModelUnavailable),
            "non-executable custom model was admitted for {model_id}"
        );
    }
}

#[test]
fn rejects_structurally_invalid_custom_endpoint_and_upstream_metadata() {
    let invalid_urls = [
        "ftp://models.example.com/v1",
        "https://user@models.example.com/v1",
        "https://models.example.com:0/v1",
        "https://models.example.com/v1?key=value",
        "https://models.example.com/v1#fragment",
        " https://models.example.com/v1",
    ];
    for base_url in invalid_urls {
        let mut row = custom_model();
        let model_id = row.model_id.clone();
        row.base_url = Some(base_url.to_owned());
        assert_eq!(
            select_offered_model(&[row], &model_id),
            Err(DaChatApiError::CustomModelUnavailable),
            "invalid endpoint was admitted: {base_url}"
        );
    }

    for upstream_model_id in ["", " upstream", "upstream\nmodel"] {
        let mut row = custom_model();
        let model_id = row.model_id.clone();
        row.upstream_model_id = Some(upstream_model_id.to_owned());
        assert_eq!(
            select_offered_model(&[row], &model_id),
            Err(DaChatApiError::CustomModelUnavailable),
            "invalid upstream id was admitted: {upstream_model_id:?}"
        );
    }

    let mut oversized = custom_model();
    let model_id = oversized.model_id.clone();
    oversized.upstream_model_id = Some("x".repeat(257));
    assert_eq!(
        select_offered_model(&[oversized], &model_id),
        Err(DaChatApiError::CustomModelUnavailable)
    );
}

#[test]
fn admits_http_ip_and_local_custom_endpoints() {
    let valid_urls = [
        "http://models.example.com/v1",
        "http://localhost:8000/v1",
        "http://127.0.0.1:11434/v1",
        "http://192.168.1.20:8000/v1",
        "http://[::1]:8080/v1",
        "http://ollama:11434/v1",
        "https://models.internal/v1",
    ];
    for base_url in valid_urls {
        let mut row = custom_model();
        let model_id = row.model_id.clone();
        row.base_url = Some(base_url.to_owned());
        assert!(
            select_offered_model(&[row], &model_id).is_ok(),
            "valid endpoint was refused: {base_url}"
        );
    }
}

/// A conversation keeps the model it was created with. One locked to the
/// pinned GPT-5.6 Luna id must keep running while the catalog serves the row.
#[test]
fn a_conversation_locked_to_the_retired_luna_id_still_runs() {
    let rows = [
        model(RETIRED_LUNA_MODEL_ID, "openrouter"),
        model(LUNA_MODEL_ID, "openrouter"),
    ];

    let locked = select_model_by_id(&rows, Some(RETIRED_LUNA_MODEL_ID)).expect("locked Luna");

    assert_eq!(locked.model_id, RETIRED_LUNA_MODEL_ID);
    assert_eq!(locked.execution_mode, ModelExecutionMode::BoundedContext);
}

/// The retired id is never a default and never offered, even when it is the
/// only Luna row in the catalog.
#[test]
fn the_retired_luna_id_is_never_offered_or_defaulted() {
    let rows = [model(RETIRED_LUNA_MODEL_ID, "openrouter")];

    assert!(supported_models(&rows).is_empty());
    assert_eq!(select_model(&rows), Err(DaChatApiError::NoModel));
    assert_eq!(
        select_offered_model(&rows, RETIRED_LUNA_MODEL_ID),
        Err(DaChatApiError::NoModel)
    );
}

/// A person who chose Luna before the move keeps Luna, not a silent switch to
/// the Sonnet default.
#[test]
fn a_saved_retired_luna_preference_resolves_to_its_successor() {
    let rows = [
        model(SONNET_MODEL_ID, "openrouter"),
        model(RETIRED_LUNA_MODEL_ID, "openrouter"),
        model(LUNA_MODEL_ID, "openrouter"),
    ];

    let selected = select_offered_model(&rows, RETIRED_LUNA_MODEL_ID).expect("successor");

    assert_eq!(selected.model_id, LUNA_MODEL_ID);
}

#[test]
fn an_offered_non_custom_model_must_be_a_product_model() {
    let rows = [
        model(SOL_MODEL_ID, "openrouter"),
        model("anthropic/claude-opus-4.6", "openrouter"),
    ];

    assert_eq!(
        select_offered_model(&rows, SOL_MODEL_ID).map(|model| model.model_id),
        Ok(SOL_MODEL_ID.to_owned())
    );
    assert_eq!(
        select_offered_model(&rows, "anthropic/claude-opus-4.6"),
        Err(DaChatApiError::NoModel)
    );
}

#[test]
fn empty_catalog_fails_closed_without_a_built_in_model() {
    assert_eq!(select_model(&[]), Err(DaChatApiError::NoModel));
}

#[test]
fn rejects_every_ineligible_contract_dimension() {
    let mut inactive = model(SONNET_MODEL_ID, "openrouter");
    inactive.is_active = false;
    let mut disabled = model(SONNET_MODEL_ID, "openrouter");
    disabled.is_enabled = false;
    let mut not_chat = model(SONNET_MODEL_ID, "openrouter");
    not_chat.is_chat = false;
    let mut unary = model(SONNET_MODEL_ID, "openrouter");
    unary.supports_streaming = false;
    let wrong_provider = model(SONNET_MODEL_ID, "anthropic");
    let pinned_haiku = model("anthropic/claude-haiku-4.5", "openrouter");
    let unrelated = model("openai/gpt-5", "openrouter");
    let mut short_context = model(SONNET_MODEL_ID, "openrouter");
    short_context.context_length = MIN_NATIVE_CONTEXT_TOKENS - 1;
    let mut missing_output = model(SONNET_MODEL_ID, "openrouter");
    missing_output.max_output_tokens = None;
    let mut zero_output = model(SONNET_MODEL_ID, "openrouter");
    zero_output.max_output_tokens = Some(0);
    let mut negative_output = model(SONNET_MODEL_ID, "openrouter");
    negative_output.max_output_tokens = Some(-1);
    let mut missing_display_name = model(SONNET_MODEL_ID, "openrouter");
    missing_display_name.display_name = "   ".to_owned();
    let mut oversized_display_name = model(SONNET_MODEL_ID, "openrouter");
    oversized_display_name.display_name = "x".repeat(129);

    let rows = [
        inactive,
        disabled,
        not_chat,
        unary,
        wrong_provider,
        pinned_haiku,
        unrelated,
        short_context,
        missing_output,
        zero_output,
        negative_output,
        missing_display_name,
        oversized_display_name,
    ];
    assert_eq!(select_model(&rows), Err(DaChatApiError::NoModel));
}

#[test]
fn rejects_missing_tools_wrong_parent_and_output_that_consumes_the_window() {
    let mut no_tools = model(LUNA_MODEL_ID, "openrouter");
    no_tools.supports_tools = false;
    let mut wrong_parent = model(GROK_MODEL_ID, "openrouter");
    wrong_parent.provider_parent = "x-ai".to_owned();
    let mut no_input = model(GROK_MODEL_ID, "openrouter");
    no_input.max_output_tokens = Some(no_input.context_length);

    for row in [no_tools, wrong_parent, no_input] {
        let model_id = row.model_id.clone();
        assert_eq!(
            select_model_by_id(&[row], Some(model_id.as_str())),
            Err(DaChatApiError::NoModel)
        );
    }
}
