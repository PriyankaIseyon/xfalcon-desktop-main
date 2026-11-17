## ADDED Requirements

### Requirement: OpenRouter uses the protocol selected by the conversation model

The OpenRouter adapter SHALL send native-compaction Sonnet turns to the fixed Anthropic Messages endpoint and SHALL send Grok and Luna turns to the fixed Chat Completions endpoint. Both paths MUST use the same borrowed OpenRouter credential and fixed attribution headers.

#### Scenario: A native-compaction request is dispatched

- **WHEN** the run carries validated native-compaction context
- **THEN** the adapter uses `/api/v1/messages` and the compaction beta contract

#### Scenario: A generic request is dispatched

- **WHEN** the run carries no native-compaction context
- **THEN** the adapter uses `/api/v1/chat/completions`
- **AND** it preserves streaming tools, reasoning, usage, cancellation, and error classification
