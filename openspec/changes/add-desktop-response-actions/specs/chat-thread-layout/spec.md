## MODIFIED Requirements

### Requirement: Turn actions appear on hover in a reserved slot

A turn's actions MUST occupy a reserved slot so interaction does not move the transcript. Copy and regenerate MUST remain visible without hover on all pointer types. Edit MAY retain its existing hover reveal. Every action MUST be keyboard reachable and have an accessible name and tooltip. Copy MUST confirm success with a temporary checkmark and show a retry on failure.

#### Scenario: Permanent response controls

- **WHEN** a durable assistant response is displayed without pointer hover
- **THEN** copy and the eligible regenerate action are visible in the reserved slot

#### Scenario: Every action is reachable without a pointer

- **WHEN** the user moves focus through the actions
- **THEN** each action receives focus and announces its purpose

#### Scenario: The transcript does not move when actions appear

- **WHEN** the pointer enters a turn
- **THEN** no text in the transcript changes position

#### Scenario: A touch device shows them outright

- **GIVEN** a coarse pointer
- **THEN** copy and eligible regenerate actions are visible without hover

## ADDED Requirements

### Requirement: Persisted response feedback

The desktop SHALL place icon-only thumbs up and thumbs down beside regenerate for durable assistant answers. It SHALL load the current user's votes and write changes through DA's existing message-feedback endpoints. Selecting the active rating SHALL clear it. Failures SHALL retain the last confirmed vote and permit retry. Results SHALL remain scoped to the owning account and conversation.

#### Scenario: Load, switch, and clear

- **WHEN** the user opens a conversation with an upvote, selects downvote, and selects downvote again
- **THEN** the UI loads the saved upvote, saves the downvote, and deletes the feedback through DA

#### Scenario: Failed persistence

- **WHEN** DA rejects a vote change
- **THEN** the prior confirmed vote remains and a visible error allows retry

#### Scenario: Immediate selection while saving

- **WHEN** the user selects a response rating
- **THEN** the selection updates immediately, with green for upvotes and dark orange for downvotes
- **AND** the existing endpoint saves in the background, with failure restoring the prior rating

### Requirement: Contextual next prompts

The desktop SHALL show up to three Try next prompts after the latest durable assistant answer settles. It SHALL generate them using bounded answer and previous-user context through the existing native provider transport. Provider failure or empty results SHALL use a deterministic BI-focused local fallback. Suggestions SHALL focus on ETL, data quality, analysis, reports and dashboards; small talk without analytics context SHALL use BI starter prompts instead of language or etiquette tasks. A selected prompt SHALL enter the normal chat submission path verbatim. Suggestions SHALL be suppressed during submission, streaming, revision, offline views, and after a new user turn. Stale results SHALL not render after navigation.

#### Scenario: Select a generated prompt

- **WHEN** a settled answer receives contextual suggestions and the user selects one
- **THEN** the normal chat send handler receives the full prompt

#### Scenario: Optional generation fails

- **WHEN** the provider fails or exceeds the bounded generation deadline
- **THEN** contextual local suggestions render once without a late replacement

#### Scenario: Navigation during generation

- **WHEN** the user leaves the conversation before generation completes
- **THEN** the previous conversation's results do not appear in the current conversation
