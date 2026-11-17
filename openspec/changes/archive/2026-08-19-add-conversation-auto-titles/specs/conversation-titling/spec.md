## ADDED Requirements

### Requirement: A conversation is named the moment it exists

The system MUST derive a title from the user's opening message and apply it when
the conversation is created, without calling a model.

The deterministic stage exists because the model stage cannot be fast enough to
matter. A first turn can run for minutes, and a chat that is nameless for that
long has already failed the person trying to find it. Deriving a title from text
the system already holds costs nothing, cannot fail on its own, and puts a name
in the sidebar before the first token streams.

#### Scenario: The opening message names the chat

- **WHEN** a user sends the first message of a new conversation
- **THEN** the conversation is created carrying a title derived from that
  message

#### Scenario: Derivation does not depend on the model

- **GIVEN** the provider is unreachable
- **WHEN** a user sends the first message of a new conversation
- **THEN** the conversation is still named

#### Scenario: A long opener is cut at a word boundary

- **GIVEN** an opening message longer than the derived-title budget
- **WHEN** the title is derived
- **THEN** it ends at a word boundary and is marked as shortened, not cut
  mid-word

### Requirement: The model improves the derived title once

The system MUST make exactly one background model call per conversation to
replace the derived title with a better one, and MUST NOT block the turn on it.

One call, at the start, on the message that states the intent. Re-titling as a
conversation develops would rename rows out from under a user who has already
learned where things are.

#### Scenario: The upgrade replaces the derived title

- **GIVEN** a conversation carrying a derived title
- **WHEN** the background titling call returns a usable title
- **THEN** the conversation carries the model's title

#### Scenario: The turn does not wait

- **WHEN** titling is in flight
- **THEN** the run proceeds and streams independently of it

#### Scenario: A failed upgrade leaves the chat named

- **WHEN** the background titling call fails
- **THEN** the derived title remains
- **AND** the failure is logged rather than surfaced as a run error

#### Scenario: Titling does not repeat

- **GIVEN** a conversation that has already been titled by the model
- **WHEN** further messages are sent
- **THEN** no further titling call is made

### Requirement: A title the user chose is final

The system MUST record how each title was decided, ordered `derived` below
`llm` below `user`, and MUST refuse any write whose authority is lower than the
authority already recorded.

The race this settles is real and unavoidable: the background call is in flight
while the user can already rename the chat. Without an ordering, whichever
finishes last wins, and the user watches their own title get replaced by a
machine's. The check and the write must be one transaction, or the ordering is
merely advisory.

#### Scenario: Automation cannot overwrite a user's title

- **GIVEN** the user renamed a conversation while titling was in flight
- **WHEN** the titling call returns
- **THEN** the user's title is kept

#### Scenario: The model may replace a derived title

- **GIVEN** a conversation whose title authority is `derived`
- **WHEN** the model's title is written
- **THEN** it is applied, and the authority becomes `llm`

#### Scenario: A conversation with no recorded authority is left alone

- **GIVEN** a conversation created before this capability existed, or renamed on
  another client
- **WHEN** automatic titling considers it
- **THEN** it is treated as user-authored and not retitled

### Requirement: Titling introduces no new destination for user content

The titling call MUST use the same provider, credential, and model as the
conversation it titles.

The message being titled is one the user is already sending to that provider on
that turn. Reusing the same route means automatic titling widens no egress path,
stores no additional secret, and adds no destination to review.

#### Scenario: No second provider is contacted

- **WHEN** a conversation is titled
- **THEN** the only provider contacted is the one the conversation runs on

#### Scenario: Titling is recorded in the audit trail

- **WHEN** a titling call is made
- **THEN** it is recorded as a model call attributable to that conversation

### Requirement: Machine-authored openers are not titled

The system MUST NOT derive or request a title from a message that carries no
user intent, and SHALL leave such a conversation untitled rather than name it
after scaffolding.

An opener can be entirely machine-authored — a resumed-session marker, a
recovery notice, a command envelope. Naming a chat after that text tells the
user nothing and looks like a bug, which is worse than a chat that is plainly
unnamed.

#### Scenario: A scaffolding-only opener yields no title

- **GIVEN** an opening message consisting only of machine-authored scaffolding
- **WHEN** the conversation is created
- **THEN** no title is derived and no titling call is made

#### Scenario: A real request wrapped in scaffolding is titled by its request

- **GIVEN** an opening message whose scaffolding surrounds the user's own words
- **WHEN** the title is derived
- **THEN** it comes from the user's words, not the scaffolding
