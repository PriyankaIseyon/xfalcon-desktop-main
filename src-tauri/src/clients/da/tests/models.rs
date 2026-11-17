use indexmap::IndexMap;
use uuid::Uuid;
use xfalcon_contracts::credentials::{ModelId, ProviderId};

use super::super::{
    generated,
    models::{project_model, ModelRecord},
};

fn model(provider: &str) -> generated::ModelEntity {
    generated::ModelEntity {
        id: Uuid::nil(),
        model_id: "model-safe-id".to_owned(),
        provider: provider.to_owned(),
        provider_parent: provider.to_owned(),
        provider_sub: None,
        display_name: "Safe model".to_owned(),
        context_length: 128_000,
        max_output_tokens: Some(16_384),
        input_price_per_m: 1.0,
        output_price_per_m: 2.0,
        released_at: None,
        is_chat: true,
        is_enabled: true,
        supports_streaming: true,
        supports_tools: true,
        supports_vision: true,
        metadata: Some(IndexMap::new()),
        created_at: None,
        updated_at: None,
        is_active: true,
        tenant_id: "tenant-must-not-project".to_owned(),
        supports_reasoning: true,
        supports_extended_thinking: true,
        reasoning_param_style: "raw-chain-of-thought".to_owned(),
        default_thinking_budget_tokens: Some(8_192),
        owner_user_id: None,
        base_url: None,
        upstream_model_id: None,
        provider_key_id: None,
    }
}

#[test]
fn supported_model_projects_only_redacted_picker_fields() {
    let raw = ModelRecord::from_generated(model("openai"));
    let view = project_model(&raw).expect("supported active chat model");
    let wire = serde_json::to_string(&view)
        .expect("model projection serializes")
        .to_ascii_lowercase();

    assert_eq!(view.provider, ProviderId::OpenAi);
    assert!(wire.contains("model-safe-id"));
    for forbidden in [
        "tenant-must-not-project",
        "input_price",
        "output_price",
        "metadata",
        "reasoning_param_style",
        "thinking_budget",
        "raw-chain-of-thought",
    ] {
        assert!(
            !wire.contains(forbidden),
            "model projection leaked {forbidden}"
        );
    }
}

#[test]
fn unknown_or_noncanonical_providers_fail_closed() {
    for provider in ["openai-compatible", "OpenAI", "azure-openai", "", "other"] {
        let raw = ModelRecord::from_generated(model(provider));
        assert!(
            project_model(&raw).is_err(),
            "accepted unsupported provider {provider:?}"
        );
    }
}

/// An aggregator row names the vendor inside the slug, so the slug must survive
/// projection untouched — routing depends on it.
#[test]
fn an_aggregator_row_projects_with_its_vendor_qualified_slug() {
    let mut dto = model("openrouter");
    dto.model_id = "anthropic/claude-sonnet-latest".to_owned();
    let view = project_model(&ModelRecord::from_generated(dto)).expect("supported model");

    assert_eq!(view.provider, ProviderId::OpenRouter);
    assert_eq!(view.id, ModelId::from("anthropic/claude-sonnet-latest"));
}

#[test]
fn unavailable_models_fail_closed() {
    let mut disabled = model("anthropic");
    disabled.is_enabled = false;
    let mut inactive = model("anthropic");
    inactive.is_active = false;
    let mut non_chat = model("anthropic");
    non_chat.is_chat = false;
    let mut non_streaming = model("anthropic");
    non_streaming.supports_streaming = false;

    for dto in [disabled, inactive, non_chat, non_streaming] {
        let raw = ModelRecord::from_generated(dto);
        assert!(project_model(&raw).is_err());
    }
}

#[test]
fn capabilities_are_mapped_conservatively() {
    let raw = ModelRecord::from_generated(model("google"));
    let view = project_model(&raw).expect("supported model");
    assert!(view.capabilities.tools);
    assert!(view.capabilities.image_input);
    assert!(
        !view.capabilities.reasoning_summary,
        "raw reasoning flags do not prove a user-safe reasoning summary"
    );

    let mut limited = model("google");
    limited.supports_tools = false;
    limited.supports_vision = false;
    limited.supports_reasoning = true;
    let limited = project_model(&ModelRecord::from_generated(limited)).expect("supported model");
    assert!(!limited.capabilities.tools);
    assert!(!limited.capabilities.image_input);
    assert!(!limited.capabilities.reasoning_summary);
}
