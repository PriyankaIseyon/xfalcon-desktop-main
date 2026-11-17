## 1. Reasoning across the shipped vendors

- [x] 1.1 Replace `REASONING_MODEL_PREFIX` with the shipped namespace set, and
      rewrite the doc comment: the old one justifies a premise that no longer
      holds, and a stale reason is worse than none.
- [x] 1.2 Update `requests_reasoning_only_for_anthropic_models` to cover the new
      set, keeping its "a prefix is not a substring" case.
- [x] 1.3 Keep the `the_reasoning_request_names_no_token_budget` pin. It is what
      stops a future edit from "completing" the object into an outage.

## 2. Routing that will not drop the tools

- [x] 2.1 Send `provider: { require_parameters: true }` when `tools` is
      non-empty; omit the whole `provider` key otherwise.
- [x] 2.2 Tests: present with tools, absent without, and a comment stating why
      reasoning does NOT trigger it.

## 3. Anthropic prompt caching

- [x] 3.1 Mark the system message with `cache_control: {type: "ephemeral"}` for
      `anthropic/` models above a size floor, by rewriting that message's content
      into the content-part array form OpenRouter documents.
- [x] 3.2 State the floor's reason: Anthropic ignores a breakpoint below its own
      minimum, so the floor exists to avoid pointless markup, not to avoid an
      error.
- [x] 3.3 Tests: marked for a large Anthropic prompt, absent for other vendors,
      absent below the floor, absent with no system message, and never on a
      second message.

## 4. Verify

- [x] 4.1 `cargo test --workspace --all-features`, `cargo clippy` with
      `-D warnings`, `cargo fmt --check`.
- [x] 4.2 Live: send one turn to an OpenAI model and one to a Google model
      through OpenRouter and confirm reasoning reaches the panel. This is the
      only check that proves the widened namespace set is right, and it needs a
      machine with a provider key.
