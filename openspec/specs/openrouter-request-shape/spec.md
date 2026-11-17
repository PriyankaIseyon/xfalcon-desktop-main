# openrouter-request-shape Specification

## Purpose

Three things the OpenRouter adapter must get right that a generic passthrough would
not: asking every shipped vendor for reasoning, refusing a route that would silently
drop tools, and marking the one vendor that does not cache its own system prompt.

## Scope

| File                                    | What it owns here                                               |
| --------------------------------------- | --------------------------------------------------------------- |
| `src-tauri/src/providers/openrouter.rs` | the adapter: reasoning request, provider routing, cache marking |

## Boundaries

- **Coalescing and forwarding the resulting stream** belongs to
  `chat-streaming-performance`.
- **What the reasoning disclosure does with the fragments** belongs to
  `chat-reasoning-panel`.
- **Retry and classification of the response** belong to
  `provider-failure-recovery`.

## Requirements

### Requirement: Reasoning is requested from every vendor that supports it

The adapter SHALL request a reasoning trace for the vendor namespaces the product
ships against, not for one of them, whenever the turn allows reasoning.

#### Scenario: The shipped vendors are asked to reason

- **WHEN** the model belongs to a shipped vendor namespace and the turn allows
  reasoning
- **THEN** the request carries a reasoning key

#### Scenario: A vendor outside the shipped set is not asked

- **WHEN** the model belongs to any other namespace
- **THEN** the request carries no reasoning key

#### Scenario: A namespace is a prefix, not a substring

- **WHEN** a model id merely contains a shipped namespace somewhere inside it
- **THEN** the request carries no reasoning key

#### Scenario: A turn that forbids reasoning is never asked

- **WHEN** the turn does not allow reasoning
- **THEN** the request carries no reasoning key, whatever the model

#### Scenario: No token budget is named

- **WHEN** the request carries a reasoning key
- **THEN** that key names neither a max-tokens nor an effort field

### Requirement: A turn with tools refuses a route that would drop them

The adapter SHALL require parameter support from the routed provider when the turn
carries tools AND the model is one the product ships against, so a provider that
would ignore those tools is not selected.

Narrowing the routing when no endpoint qualifies makes the upstream answer with a
not-found response, which the adapter reports as a response it could not parse — the
wrong account of the failure. Restricting the requirement to models known to call
tools means that case cannot arise.

#### Scenario: Tools demand a capable route

- **WHEN** the turn carries at least one tool and the model is a shipped vendor's
- **THEN** the request requires provider parameter support

#### Scenario: A tool-free turn does not narrow its routing

- **WHEN** the turn carries no tools
- **THEN** the request carries no provider routing key

#### Scenario: A model outside the shipped set keeps the permissive default

- **WHEN** the turn carries tools and the model is not a shipped vendor's
- **THEN** the request carries no provider routing key

### Requirement: The Anthropic system prompt is cached

The adapter SHALL mark the system message with a cache breakpoint for Anthropic
models, whose provider does not cache without being asked. Vendors that cache
automatically MUST be left unmarked.

#### Scenario: A large Anthropic system prompt is marked

- **WHEN** the model belongs to the Anthropic namespace and the system message is
  above the size floor
- **THEN** that message's content carries a cache breakpoint, and no other message
  carries one

#### Scenario: A vendor that caches automatically is left alone

- **WHEN** the model belongs to any other namespace
- **THEN** no message carries a cache breakpoint

#### Scenario: A short system prompt is not marked

- **WHEN** the system message is below the size floor
- **THEN** no message carries a cache breakpoint

#### Scenario: A turn with no system message is unaffected

- **WHEN** the turn carries no system message
- **THEN** no message carries a breakpoint

## Provenance

From the delta of `add-openrouter-advanced-streaming`.
