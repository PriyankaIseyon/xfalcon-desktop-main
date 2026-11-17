## Implementation and verification

- [x] Luna's product id is `~openai/gpt-luna-latest`, and Sol
      (`~openai/gpt-sol-latest`) is offered after it on the bounded-context
      path. Evidence:
  - `clients::da::tests::model_selection::selects_sol_on_the_bounded_context_path`
  - `clients::da::tests::model_selection::projects_only_the_four_product_models_in_stable_order`
- [x] The retired `openai/gpt-5.6-luna` is never offered or defaulted, and still
      runs a conversation locked to it. Evidence:
  - `clients::da::tests::model_selection::the_retired_luna_id_is_never_offered_or_defaulted`
  - `clients::da::tests::model_selection::a_conversation_locked_to_the_retired_luna_id_still_runs`
- [x] A saved preference or new default resolves through the offered set, with
      the retired id read as its successor. Evidence:
  - `clients::da::tests::model_selection::a_saved_retired_luna_preference_resolves_to_its_successor`
  - `clients::da::tests::model_selection::an_offered_model_must_be_a_product_model`
- [x] A locked conversation's catalog view lists its own model, and new chats
      list only product models. Evidence:
  - `chat::tests::service::a_conversation_locked_to_the_retired_luna_id_lists_its_own_model`
- [ ] DA seeds `~openai/gpt-luna-latest` and `~openai/gpt-sol-latest` into
      `CHAT_MODEL_CATALOG`. Not in this repository. Until it lands, the desktop
      offers Sonnet and Grok only.

Verification on 2026-09-23, on the Windows mirror:

- `cargo test --workspace --locked` passed: 1,756 native, 28 contract,
  9 release.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` was clean.
- `cargo fmt --check` was clean.

OpenRouter's model API was checked on the same date. Both aliases have a
1,050,000-token context window and a 128,000-token maximum output, with tools
and reasoning supported. Pricing per million tokens: Luna $0.10 in and $0.50
out; Sol $2.00 in and $10.00 out.
