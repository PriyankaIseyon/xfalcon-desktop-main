## ADDED Requirements

### Requirement: The estimate is anchored on the provider

The system MUST estimate text at three bytes per token, charge each image the
documented ceiling, and, once a provider has reported its prompt size, estimate
a later request as that size plus the growth since.

#### Scenario: Text is not charged a token per byte

- **WHEN** 30 KB of text is estimated
- **THEN** it scores between 9,000 and 12,000 tokens

#### Scenario: An image's file size does not matter

- **WHEN** a 3 KB image and a 15 MB image are estimated
- **THEN** they score the same

### Requirement: A provider's "prompt too long" is answered by compacting harder

The system MUST recognise an overflow answer on every adapter, reduce the input
it allows by a quarter, compact, and ask again, at most twice in a row.

#### Scenario: One rejection

- **GIVEN** the provider rejects a request as too long
- **WHEN** the run asks again under the lower limit
- **THEN** the turn completes

#### Scenario: Endless rejection

- **WHEN** the provider rejects three requests in a row
- **THEN** the run fails with the provider's message

### Requirement: A model without native compaction condenses its own history

The system MUST, above 75% of the input a model may use, stub tool results older
than the newest 30% of the window except skill instruction and reference results,
and if that is not enough summarize the oldest
history with the run's own model, keeping the newest 25% and the latest user turn
word for word. It MUST NOT send a request it estimates above the limit.

#### Scenario: Old tool results

- **GIVEN** three large tool results and a window they nearly fill
- **WHEN** the next request is built
- **THEN** the two oldest results are stubs, every call still has an answer, and no extra model request is made

#### Scenario: Loaded skills survive routine pruning

- **GIVEN** an older skill fetch with instruction, continuation, or reference pages
- **WHEN** older large file-read results are replaced with stubs
- **THEN** the skill pages remain exact and every call keeps its result
- **AND** full summarization and emergency shedding remain subject to the hard input limit

#### Scenario: Long history

- **GIVEN** a history above the threshold with no large tool results
- **WHEN** the next request is built
- **THEN** one request asks the model for a summary, and the answer request carries the summary and the newest turns but not the oldest

#### Scenario: A summary that comes back incomplete

- **WHEN** the summarizer returns empty text, no completion, or a stop reason other than a natural end
- **THEN** the result SHALL NOT replace history or enter the summary cache
- **AND** the run MAY continue with its existing context if it still fits
- **AND** an oversized request SHALL use the existing bounded capacity-recovery path or fail naming the capacity

### Requirement: A summary is reused only for the conversation it describes

The system MUST reuse a summary only while its covered durable history still
matches. Rewriting that history MUST invalidate the cached summary.

#### Scenario: The next message

- **GIVEN** a conversation already summarized
- **WHEN** the next message starts
- **THEN** no second summary is requested

#### Scenario: A rewritten conversation

- **GIVEN** an early message was edited
- **WHEN** the next message starts
- **THEN** the remembered summary is dropped and a new one is written

### Requirement: A compaction that cannot be sent as a block is read as text

The system MUST replay a valid but incompatible compaction as summary text.
An integrity failure MUST stop the run before a provider request.

#### Scenario: A different model or changed system prompt

- **GIVEN** an active compaction whose model or fingerprint no longer matches the run
- **WHEN** a request is built
- **THEN** the compaction's summary text is shown as a message, no block is sent, and the run continues

#### Scenario: A damaged record

- **GIVEN** an active compaction whose digest does not match
- **WHEN** the run starts
- **THEN** it fails before any provider request

### Requirement: What the record omits is put back

The system MUST restore run-local thinking and attachments across snapshot
reloads. A prefix-bound model MUST receive the exact temporary messages that
preceded valid thinking. A new native compaction MUST retire invalidated
thinking from the wire without rewriting the durable history.

#### Scenario: Thinking between rounds

- **GIVEN** a turn that thought and then called a tool without a subsequent prefix replacement
- **WHEN** the next round's context is rebuilt from the saved record
- **THEN** the tool call carries its thinking blocks
- **AND** a prefix-bound model retains every temporary run-status reminder that preceded those blocks
- **AND** a newly active native compaction retires only thinking and reminders invalidated by the replaced prefix

### Requirement: Shortening the history is announced while it happens

The system MUST tell the interface when a run starts and stops shortening its
history, on both the native-compaction and the client-summary paths, and MUST
send the stop even when the step fails. While announced, the run's activity
label MUST read "Summarizing earlier conversation…" in place of the rotating
phrases, and only for the run that announced it.

#### Scenario: A summary is being written

- **WHEN** the run asks its model to summarize older history
- **THEN** a start notice goes out before the request and a stop notice after it
- **AND** the label reads "Summarizing earlier conversation…" between them

#### Scenario: The summary fails

- **WHEN** the summary request fails or comes back unusable
- **THEN** the stop notice is still sent

#### Scenario: Another run announces

- **WHEN** a notice arrives for a run other than the one in view
- **THEN** the label does not change
