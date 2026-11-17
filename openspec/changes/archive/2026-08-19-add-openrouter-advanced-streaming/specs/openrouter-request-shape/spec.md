## ADDED Requirements

### Requirement: Reasoning Is Requested From Every Vendor That Supports It

The OpenRouter adapter SHALL request a reasoning trace for the vendor namespaces
the product ships against, not for one of them, whenever the turn allows
reasoning.

#### Scenario: The shipped vendors are asked to reason

- **WHEN** the model belongs to the `anthropic/`, `openai/`, `google/`, or
  `x-ai/` namespace and the turn allows reasoning
- **THEN** the request carries a `reasoning` key

#### Scenario: A vendor outside the shipped set is not asked

- **WHEN** the model belongs to any other namespace
- **THEN** the request carries no `reasoning` key

#### Scenario: A namespace is a prefix, not a substring

- **WHEN** a model id merely contains a shipped namespace somewhere inside it
- **THEN** the request carries no `reasoning` key

#### Scenario: A turn that forbids reasoning is never asked

- **WHEN** the turn does not allow reasoning
- **THEN** the request carries no `reasoning` key, whatever the model

#### Scenario: No token budget is named

- **WHEN** the request carries a `reasoning` key
- **THEN** that key names neither `max_tokens` nor `effort`

### Requirement: A Turn With Tools Refuses A Route That Would Drop Them

The adapter SHALL require parameter support from the routed provider when the
turn carries tools AND the model is one the product ships against, so a provider
that would ignore those tools is not selected.

The model condition is not caution for its own sake. Narrowing the routing when
no endpoint qualifies makes OpenRouter answer 404, which the adapter reports as a
response it could not parse — the wrong account of the failure. Restricting the
key to models known to call tools means that case cannot arise.

#### Scenario: Tools demand a capable route

- **WHEN** the turn carries at least one tool and the model is a shipped vendor's
- **THEN** the request sets `provider.require_parameters` to true

#### Scenario: A tool-free turn does not narrow its routing

- **WHEN** the turn carries no tools
- **THEN** the request carries no `provider` key

#### Scenario: A model outside the shipped set keeps the permissive default

- **WHEN** the turn carries tools and the model is not a shipped vendor's
- **THEN** the request carries no `provider` key

### Requirement: The Anthropic System Prompt Is Cached

The adapter SHALL mark the system message with a cache breakpoint for
`anthropic/` models, whose provider does not cache without being asked. Vendors
that cache automatically MUST be left unmarked.

#### Scenario: A large Anthropic system prompt is marked

- **WHEN** the model belongs to the `anthropic/` namespace and the system message
  is above the size floor
- **THEN** that message's content carries a `cache_control` breakpoint
- **AND** no other message carries one

#### Scenario: A vendor that caches automatically is left alone

- **WHEN** the model belongs to any other namespace
- **THEN** no message carries a `cache_control` breakpoint

#### Scenario: A short system prompt is not marked

- **WHEN** the system message is below the size floor
- **THEN** no message carries a `cache_control` breakpoint

#### Scenario: A turn with no system message is unaffected

- **WHEN** the turn carries no system message
- **THEN** the request is unchanged and no message carries a breakpoint
