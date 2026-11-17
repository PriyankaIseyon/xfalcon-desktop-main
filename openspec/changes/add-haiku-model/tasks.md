## Implementation and verification

- [x] Haiku leads the order, is the default with no saved preference, and runs
      on the bounded-context path. Evidence:
  - `clients::da::tests::model_selection::haiku_is_the_default_when_every_product_model_is_eligible`
  - `clients::da::tests::model_selection::the_default_falls_through_to_sonnet_when_haiku_is_not_served`
  - `clients::da::tests::model_selection::selects_haiku_on_the_bounded_context_path`
  - `clients::da::tests::model_selection::projects_only_the_five_product_models_in_stable_order`
- [x] The pinned `anthropic/claude-haiku-4.5` is still not a product model.
      Evidence: `clients::da::tests::model_selection::a_pinned_haiku_id_is_not_the_product_model`
- [x] Docs name the five-model order.
- [ ] DA seeds `~anthropic/claude-haiku-latest` into `CHAT_MODEL_CATALOG`. Not
      in this repository.
- [ ] Live check: with the row seeded, Haiku appears in Settings → Models and
      answers a chat turn, including a tool call.
