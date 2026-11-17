//! The catalog rows for the plan providers' models.
//!
//! DA does not know these models, so their rows are built here from what the
//! device knows and merged into the catalog at read time. A row is a picker
//! entry only: chat admission applies the same contract it applies to a DA row.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};
use std::time::Instant;

use indexmap::IndexMap;
use uuid::Uuid;

use super::service::ROWS_STALE_AFTER;
use crate::clients::da::generated::ModelEntity;
use crate::clients::da::model_catalog::LocalModelRows;
use crate::providers::chatgpt_plan::models::OfferedModel;
use crate::providers::claude_runtime::adapter::{
    CLAUDE_CONTEXT_TOKENS, CLAUDE_MAX_OUTPUT_TOKENS, CLAUDE_MODELS,
};

/// Fixed for the life of the product. Each row's id is derived from it, so
/// changing it would give every plan model a new id on the next launch.
const PLAN_MODEL_NAMESPACE: Uuid = Uuid::from_u128(0x64587701_b8e2_4ee0_89c0_ab99d085a553);
const MAX_SLUG_LEN: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlanModelRow {
    pub(crate) provider: &'static str,
    pub(crate) slug: String,
    pub(crate) display_name: String,
    pub(crate) context_length: i64,
    pub(crate) max_output_tokens: i64,
    pub(crate) supports_vision: bool,
}

/// The catalog row for one plan model. A plan model has no DA price, so both
/// prices are zero, and DA's own bookkeeping fields stay empty.
pub(crate) fn model_entity(row: &PlanModelRow, tenant_id: &str) -> ModelEntity {
    let model_id = format!("{}/{}", row.provider, row.slug);
    ModelEntity {
        id: Uuid::new_v5(&PLAN_MODEL_NAMESPACE, model_id.as_bytes()),
        model_id,
        provider: row.provider.to_owned(),
        provider_parent: row.provider.to_owned(),
        provider_sub: None,
        display_name: row.display_name.clone(),
        context_length: row.context_length,
        max_output_tokens: Some(row.max_output_tokens),
        input_price_per_m: 0.0,
        output_price_per_m: 0.0,
        released_at: None,
        is_chat: true,
        is_enabled: true,
        supports_streaming: true,
        supports_tools: true,
        supports_vision: row.supports_vision,
        metadata: Some(IndexMap::new()),
        created_at: None,
        updated_at: None,
        is_active: true,
        tenant_id: tenant_id.to_owned(),
        supports_reasoning: true,
        supports_extended_thinking: false,
        reasoning_param_style: "none".to_owned(),
        default_thinking_budget_tokens: None,
        owner_user_id: None,
        base_url: None,
        upstream_model_id: None,
        provider_key_id: None,
    }
}

pub(crate) fn claude_rows() -> Vec<PlanModelRow> {
    CLAUDE_MODELS
        .into_iter()
        .map(|(slug, display_name)| PlanModelRow {
            provider: "claude_code",
            slug: slug.to_owned(),
            display_name: display_name.to_owned(),
            context_length: CLAUDE_CONTEXT_TOKENS,
            max_output_tokens: CLAUDE_MAX_OUTPUT_TOKENS,
            supports_vision: false,
        })
        .collect()
}

/// The rows for the ChatGPT models one account may use. A model whose limits do
/// not fit the catalog's integers is dropped, as a bad slug is.
pub(crate) fn chatgpt_rows(offered: &[OfferedModel]) -> Vec<PlanModelRow> {
    offered
        .iter()
        .filter_map(|model| {
            Some(PlanModelRow {
                provider: "chatgpt_plan",
                slug: model.slug.clone(),
                display_name: model.display_name.clone(),
                context_length: i64::try_from(model.context_length).ok()?,
                max_output_tokens: i64::try_from(model.max_output_tokens).ok()?,
                supports_vision: true,
            })
        })
        .collect()
}

/// The Claude rows and the moment the probe that vouched for them finished.
struct ClaudeRows {
    probed_at: Instant,
    rows: Vec<PlanModelRow>,
}

impl ClaudeRows {
    /// Whether the rows are still offered at `now`.
    fn offered_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.probed_at) < ROWS_STALE_AFTER
    }
}

/// The plan models this device can run. Claude's rows belong to the device;
/// ChatGPT's belong to one account, so another account on the device sees none.
pub(crate) struct PlanModelSource {
    claude: RwLock<Option<ClaudeRows>>,
    chatgpt: RwLock<HashMap<String, Vec<PlanModelRow>>>,
}

impl PlanModelSource {
    pub(crate) fn new() -> Self {
        Self {
            claude: RwLock::new(None),
            chatgpt: RwLock::new(HashMap::new()),
        }
    }

    /// `probed_at` is when the probe that answered `Ready` finished. The rows stop
    /// being offered `ROWS_STALE_AFTER` after it, on every read, so a later read
    /// cannot offer rows that a newer probe would have removed.
    pub(crate) fn set_claude_at(&self, rows: Vec<PlanModelRow>, probed_at: Instant) {
        *self.claude.write().unwrap_or_else(PoisonError::into_inner) = Some(ClaudeRows {
            probed_at,
            rows: valid_rows(rows),
        });
    }

    /// Rows from a probe that finished now.
    #[cfg(test)]
    pub(crate) fn set_claude(&self, rows: Vec<PlanModelRow>) {
        self.set_claude_at(rows, Instant::now());
    }

    pub(crate) fn clear_claude(&self) {
        *self.claude.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Whether Claude's rows are offered now. A read that has them goes on with them,
    /// and renews their probe in the background.
    pub(crate) fn has_claude_rows(&self) -> bool {
        let now = Instant::now();
        self.claude
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|claude| !claude.rows.is_empty() && claude.offered_at(now))
    }

    pub(crate) fn set_chatgpt(&self, account: &str, rows: Vec<PlanModelRow>) {
        self.chatgpt
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account.to_owned(), valid_rows(rows));
    }

    pub(crate) fn clear_chatgpt(&self, account: &str) {
        self.chatgpt
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account);
    }

    /// How many rows the account has. Settings shows this count.
    pub(crate) fn chatgpt_count(&self, account: &str) -> usize {
        self.chatgpt
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .map_or(0, Vec::len)
    }

    pub(crate) fn clear_all_chatgpt(&self) {
        self.chatgpt
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    #[cfg(test)]
    pub(crate) fn clear_all(&self) {
        self.clear_claude();
        self.clear_all_chatgpt();
    }
}

impl PlanModelSource {
    /// The rows offered at `now`. Claude's rows are left out once their probe is older
    /// than `ROWS_STALE_AFTER`, which is the read the catalog publishes from.
    fn rows_at(&self, account: &str, now: Instant) -> Vec<ModelEntity> {
        let tenant_id = account.split_once(':').map_or("", |(tenant, _)| tenant);
        let claude = self.claude.read().unwrap_or_else(PoisonError::into_inner);
        let chatgpt = self.chatgpt.read().unwrap_or_else(PoisonError::into_inner);
        let claude_rows = claude
            .iter()
            .filter(|claude| claude.offered_at(now))
            .flat_map(|claude| claude.rows.iter());
        claude_rows
            .chain(chatgpt.get(account).into_iter().flatten())
            .map(|row| model_entity(row, tenant_id))
            .collect()
    }
}

impl LocalModelRows for PlanModelSource {
    fn rows(&self, account: &str) -> Vec<ModelEntity> {
        self.rows_at(account, Instant::now())
    }
}

/// A slug becomes part of a model id that the chat path parses, so only a
/// conservative character set gets through. A bad row is dropped, not repaired.
fn valid_rows(mut rows: Vec<PlanModelRow>) -> Vec<PlanModelRow> {
    rows.retain(|row| valid_slug(&row.slug));
    rows
}

fn valid_slug(slug: &str) -> bool {
    (1..=MAX_SLUG_LEN).contains(&slug.len())
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::clients::da::chat_api::{
        select_offered_model, supported_models, ModelExecutionMode, HAIKU_MODEL_ID,
    };
    use crate::plan_providers::service::PROBE_TTL;

    fn claude_row(slug: &str) -> PlanModelRow {
        PlanModelRow {
            provider: "claude_code",
            slug: slug.to_owned(),
            display_name: "Test model".to_owned(),
            context_length: 1_000_000,
            max_output_tokens: 128_000,
            supports_vision: false,
        }
    }

    fn chatgpt_row(slug: &str) -> PlanModelRow {
        PlanModelRow {
            provider: "chatgpt_plan",
            ..claude_row(slug)
        }
    }

    fn ids(models: &[ModelEntity]) -> Vec<String> {
        models.iter().map(|model| model.model_id.clone()).collect()
    }

    /// A DA product row. The fields a plan row sets are spelled out, so the
    /// test does not depend on which of them a plan row happens to share.
    fn product_model(model_id: &str) -> ModelEntity {
        ModelEntity {
            id: Uuid::nil(),
            model_id: model_id.to_owned(),
            provider: "openrouter".to_owned(),
            provider_parent: "openrouter".to_owned(),
            provider_sub: None,
            display_name: "Claude Haiku".to_owned(),
            context_length: 1_000_000,
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

    #[test]
    fn every_claude_row_is_offered_as_a_bounded_claude_code_model() {
        for row in claude_rows() {
            let model_id = format!("claude_code/{}", row.slug);
            let entity = model_entity(&row, "tenant");

            let selection =
                select_offered_model(std::slice::from_ref(&entity), &model_id).expect("offered");

            assert_eq!(selection.model_id, model_id);
            assert_eq!(selection.provider, "claude_code");
            assert_eq!(selection.context_length, 1_000_000);
            assert_eq!(selection.max_output_tokens, 128_000);
            assert_eq!(selection.execution_mode, ModelExecutionMode::BoundedContext);
            assert_eq!(selection.custom, None);
        }
    }

    #[test]
    fn the_claude_rows_are_the_three_pinned_models_in_order() {
        let slugs: Vec<String> = claude_rows().into_iter().map(|row| row.slug).collect();

        assert_eq!(
            slugs,
            vec!["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"]
        );
    }

    #[test]
    fn supported_models_lists_plan_rows_after_the_product_rows() {
        let source = PlanModelSource::new();
        source.set_claude(claude_rows());
        let mut models = source.rows("tenant:alice");
        models.push(product_model(HAIKU_MODEL_ID));

        let supported: Vec<String> = supported_models(&models)
            .into_iter()
            .map(|model| model.model_id.clone())
            .collect();

        assert_eq!(
            supported,
            vec![
                HAIKU_MODEL_ID,
                "claude_code/claude-opus-5-5",
                "claude_code/claude-sonnet-5-5",
                "claude_code/claude-haiku-5-5",
            ]
        );
    }

    #[test]
    fn a_row_with_an_invalid_slug_is_dropped_and_the_others_keep_their_order() {
        let source = PlanModelSource::new();

        source.set_claude(vec![
            claude_row("zeta-first"),
            claude_row(""),
            claude_row("has space"),
            claude_row("slash/in-slug"),
            claude_row(&"a".repeat(101)),
            claude_row(&"b".repeat(100)),
            claude_row("dots.under_score:colon-ok"),
            claude_row("alpha-last"),
        ]);

        assert_eq!(
            ids(&source.rows("tenant:alice")),
            vec![
                "claude_code/zeta-first".to_owned(),
                format!("claude_code/{}", "b".repeat(100)),
                "claude_code/dots.under_score:colon-ok".to_owned(),
                "claude_code/alpha-last".to_owned(),
            ]
        );
    }

    #[test]
    fn chatgpt_rows_are_visible_only_to_the_account_that_set_them() {
        let source = PlanModelSource::new();
        source.set_claude(claude_rows());
        source.set_chatgpt("tenant:alice", vec![chatgpt_row("gpt-plan")]);

        let alice = source.rows("tenant:alice");
        let bob = source.rows("tenant:bob");

        assert_eq!(
            ids(&alice).last().map(String::as_str),
            Some("chatgpt_plan/gpt-plan")
        );
        assert_eq!(alice.len(), 4);
        assert_eq!(bob.len(), 3);
        assert!(bob.iter().all(|model| model.provider == "claude_code"));
    }

    #[test]
    fn the_tenant_is_the_part_of_the_account_before_the_colon() {
        let source = PlanModelSource::new();
        source.set_claude(vec![claude_row("claude-sonnet-5-5")]);

        assert_eq!(source.rows("tenant:alice")[0].tenant_id, "tenant");
        assert_eq!(source.rows("alice")[0].tenant_id, "");
    }

    #[test]
    fn clearing_removes_only_what_it_names() {
        let source = PlanModelSource::new();
        source.set_claude(claude_rows());
        source.set_chatgpt("tenant:alice", vec![chatgpt_row("gpt-plan")]);
        source.set_chatgpt("tenant:bob", vec![chatgpt_row("gpt-other")]);

        source.clear_chatgpt("tenant:alice");
        assert_eq!(source.rows("tenant:alice").len(), 3);
        assert_eq!(source.rows("tenant:bob").len(), 4);

        source.clear_claude();
        assert_eq!(
            ids(&source.rows("tenant:bob")),
            vec!["chatgpt_plan/gpt-other"]
        );

        source.clear_all();
        assert!(source.rows("tenant:bob").is_empty());
    }

    fn offered(slug: &str, context_length: u64, max_output_tokens: u64) -> OfferedModel {
        OfferedModel {
            slug: slug.to_owned(),
            display_name: format!("Display {slug}"),
            context_length,
            max_output_tokens,
        }
    }

    #[test]
    fn chatgpt_rows_carry_the_offered_limits_and_accept_images() {
        let rows = chatgpt_rows(&[offered("gpt-6-luna", 1_050_000, 128_000)]);

        assert_eq!(
            rows,
            vec![PlanModelRow {
                provider: "chatgpt_plan",
                slug: "gpt-6-luna".to_owned(),
                display_name: "Display gpt-6-luna".to_owned(),
                context_length: 1_050_000,
                max_output_tokens: 128_000,
                supports_vision: true,
            }]
        );
    }

    #[test]
    fn a_chatgpt_model_whose_limits_do_not_fit_is_dropped_and_the_rest_keep_their_order() {
        let rows = chatgpt_rows(&[
            offered("gpt-6-luna", 1_050_000, 128_000),
            offered("gpt-too-large", u64::MAX, 128_000),
            offered("gpt-6-nova", 1_050_000, u64::MAX),
            offered("gpt-6.1-sol", 1_050_000, 128_000),
        ]);

        let slugs: Vec<&str> = rows.iter().map(|row| row.slug.as_str()).collect();
        assert_eq!(slugs, vec!["gpt-6-luna", "gpt-6.1-sol"]);
    }

    #[test]
    fn the_chatgpt_count_belongs_to_one_account() {
        let source = PlanModelSource::new();
        source.set_chatgpt(
            "tenant:alice",
            vec![chatgpt_row("gpt-plan"), chatgpt_row("gpt-two")],
        );

        assert_eq!(source.chatgpt_count("tenant:alice"), 2);
        assert_eq!(source.chatgpt_count("tenant:bob"), 0);
    }

    /// The rows outlive their probe's TTL, so a read a little after it still offers
    /// them. They stop only at the staleness bound.
    #[test]
    fn claude_rows_stop_being_offered_only_at_the_staleness_bound() {
        let source = PlanModelSource::new();
        let probed_at = Instant::now();
        source.set_claude_at(claude_rows(), probed_at);
        source.set_chatgpt("tenant:alice", vec![chatgpt_row("gpt-plan")]);

        let past_probe_ttl = source.rows_at("tenant:alice", probed_at + PROBE_TTL);
        let before_bound = source.rows_at(
            "tenant:alice",
            probed_at + ROWS_STALE_AFTER - Duration::from_secs(1),
        );
        let at_bound = source.rows_at("tenant:alice", probed_at + ROWS_STALE_AFTER);

        assert_eq!(past_probe_ttl.len(), 4);
        assert_eq!(before_bound.len(), 4);
        assert_eq!(ids(&at_bound), vec!["chatgpt_plan/gpt-plan"]);
    }

    #[test]
    fn has_claude_rows_is_true_only_while_the_rows_are_offered() {
        let source = PlanModelSource::new();
        assert!(!source.has_claude_rows());

        source.set_claude(claude_rows());
        assert!(source.has_claude_rows());

        source.set_claude_at(claude_rows(), Instant::now() - ROWS_STALE_AFTER);
        assert!(!source.has_claude_rows());

        source.set_claude(Vec::new());
        assert!(!source.has_claude_rows());

        source.set_claude(claude_rows());
        source.clear_claude();
        assert!(!source.has_claude_rows());
    }

    #[test]
    fn clearing_the_chatgpt_rows_keeps_the_claude_rows() {
        let source = PlanModelSource::new();
        source.set_claude(claude_rows());
        source.set_chatgpt("tenant:alice", vec![chatgpt_row("gpt-plan")]);
        source.set_chatgpt("tenant:bob", vec![chatgpt_row("gpt-other")]);

        source.clear_all_chatgpt();

        assert_eq!(source.rows("tenant:alice").len(), 3);
        assert_eq!(source.chatgpt_count("tenant:alice"), 0);
        assert_eq!(source.chatgpt_count("tenant:bob"), 0);
    }
}
