# Change: Run private custom models in Desktop

## Why

DA can store a signed-in user's private OpenAI-compatible model and its encrypted
key, but Desktop currently offers only four OpenRouter models. Desktop must run
the private model locally without sending its key or conversation to another
origin.

## What Changes

- Sync the deployed DA staging OpenAPI contract and accept the caller's eligible
  `custom_openai` rows alongside the existing product models. Sonnet remains the
  default.
- Add a separate guarded OpenAI-compatible transport. Resolve the exact key
  linked by the model, validate and pin a public DNS address on every request,
  and keep TLS hostname verification and redirect refusal.
- Add Settings controls to create, edit, and remove private models through DA's
  interactive-session routes. Refresh the model catalog after each mutation.
  Show one default-model dropdown across OpenRouter and private models. Group
  model and connection controls under one Settings heading.
- Mark private models in the picker and give distinct, actionable errors for
  unsafe addresses, rejected keys, unreachable endpoints, and removed models.

## Impact

- Affected capability: `desktop-private-custom-models`.
- Affected code: DA contract snapshot and generated DTOs; model selection and
  catalog cache; credential resolution; provider dispatch and HTTP transport;
  Settings, model picker, typed IPC contracts, and capability allowlist.
- Desktop only. DA, Web, existing provider origins, and the webview CSP stay
  unchanged.
- Dependency: DA's `feat/add-custom-models` OpenAPI must be deployed to the
  staging endpoint before the pinned `sync-da-contract.ps1` can consume it. The
  script does not accept a local OpenAPI file.
