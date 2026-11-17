## RENAMED Requirements

- FROM: `### Requirement: A saved report is sealed under the account and cleared with it`
- TO: `### Requirement: A saved report is cached locally, sealed under the account, and cleared with it`
- FROM: `### Requirement: A row that cannot be decrypted is skipped and recorded`
- TO: `### Requirement: A cache row that cannot be decrypted is skipped, recorded, and repaired by the next reconcile`

## MODIFIED Requirements

### Requirement: A report is captured from the stored message, never from the renderer

The system MUST capture a report from a source identity — the conversation, the
message, and the ordinal of the HTML document within that message — together with a
name the reader chose. The renderer MUST NOT send the document's markup, and the
backend MUST NOT accept it if sent.

The backend SHALL load the named message from the encrypted local projection,
re-scan its text for fenced HTML documents, and take the document at the given
ordinal. Re-reading from what was persisted is what makes the capture path unusable
for turning renderer-supplied markup into a stored report. Once resolved, the
document SHALL be submitted to the DA data plane as the durable copy of record; the
renderer's inputs remain limited to identity and name regardless of where the bytes
are ultimately stored.

Three conditions MUST refuse the capture, reported as outcomes of a successful call
rather than as operation failures: the message does not belong to the named
conversation, the message is not an assistant turn, the message holds no document at
that ordinal.

#### Scenario: A dashboard is saved

- **GIVEN** an assistant message in the open conversation holding one fenced HTML
  document
- **WHEN** the reader names it and saves it
- **THEN** a report is stored on the data plane whose bytes are the document's bytes
  as persisted, and it is readable from this device immediately

#### Scenario: A message from another conversation

- **GIVEN** a capture naming a message that belongs to a different conversation
- **THEN** the capture is refused as not found, telling the caller nothing about
  whether that message exists

#### Scenario: A message with no such document

- **GIVEN** a capture naming an ordinal past the last HTML document in the message
- **THEN** the capture is refused with that reason, and no row is written

#### Scenario: The renderer cannot supply the bytes

- **GIVEN** the capture command's inputs
- **THEN** none of them carries document markup

### Requirement: A saved report is cached locally, sealed under the account, and cleared with it

The DA data plane is the copy of record. A local encrypted cache MAY hold a
report's rows, including its document bytes once fetched, so that reads work
offline — the same local-first shape `chat-conversation-open`'s cache already
has. Only what ordering and dedup need SHALL be plaintext in the cache;
title, description, and document SHALL live in one AEAD ciphertext under its
own authenticated-data family, following the encrypted chat projection.

Clearing an account's local data MUST remove the cache. This clears what this
device holds; it does not delete the account's reports from the data plane —
"clear local data" and "delete a report" remain two different actions with
two different scopes, and clearing local data MUST NOT be capable of deleting
anything the data plane would otherwise still answer for after the next sign-in.

#### Scenario: Clearing local data

- **GIVEN** an account with cached reports
- **WHEN** the reader clears local data
- **THEN** the cache is gone, and the clear is not partial

#### Scenario: Reports survive a local clear

- **GIVEN** an account that cleared its local data
- **WHEN** the reader signs in again and opens Reports
- **THEN** every report the data plane holds for that account is listed again

#### Scenario: Nothing readable is left on disk

- **GIVEN** a cached report
- **THEN** its title and document are absent from the plaintext columns

### Requirement: A cache row that cannot be decrypted is skipped, recorded, and repaired by the next reconcile

The system MUST skip a cache row it cannot decrypt when listing, rather than
failing the list, and MUST record that this happened. Unlike the store this
capability used before this change, an undecryptable row is not a lost report
— the data plane still holds it, and the next successful reconcile
overwrites the row with a freshly sealed one.

#### Scenario: One unreadable cache row

- **GIVEN** three cached reports, one of whose rows cannot be decrypted
- **THEN** the list returns two immediately and the third is repaired on the
  next successful reconcile with the data plane

#### Scenario: What the reader is told about an unreadable row

- **WHEN** the list reports rows it could not decrypt
- **THEN** the surface says they return on the next sync, and MUST NOT
  describe them as lost or their saved copies as unavailable
- **AND** where no row could be read at all, the surface offers the sync that
  repairs them

### Requirement: A report that could not be opened is never saved

The system MUST refuse to store a document larger than the preview store will serve —
the per-document ceiling SHALL equal the preview ceiling, so a report that stores is a
report that opens. This ceiling is enforced locally regardless of what the data
plane's own limit is, because a document under the data plane's ceiling but over
this one would store there and then be permanently unopenable here.

The per-account count and total-bytes limits are the data plane's to enforce; the
system MUST surface its answer as a capture outcome stating the limit reached, not
attempt an independent local count.

#### Scenario: A document over the ceiling

- **GIVEN** a document larger than the preview store will serve
- **THEN** the capture is refused as too large, stating the maximum, before any
  network call is made

#### Scenario: The data plane's cap is reached

- **GIVEN** an account already at the data plane's maximum number of reports
- **WHEN** one more is captured
- **THEN** the capture is refused with the stated limit, existing reports untouched

## ADDED Requirements

### Requirement: Reports read locally first and reconcile against the data plane

The system MUST serve a report list from the local cache immediately and
reconcile it against the data plane in the same call, following the pattern
`chat-conversation-open` already established: a successful reconcile replaces
the cache and reports the list as current; a failed reconcile against a cache
that has synced before serves the cached rows and reports the list as an
offline copy; a failed reconcile with no prior successful sync has nothing to
fall back to and fails outright rather than reporting an empty list as
current. An unauthorized answer from the data plane MUST always be reported
as a failure, never folded into the offline-copy case.

#### Scenario: A normal list

- **WHEN** the reader opens Reports with a working connection
- **THEN** the list reflects the data plane and is not marked as an offline copy

#### Scenario: Listing while offline, previously synced

- **GIVEN** a device that has successfully listed reports before
- **WHEN** the reader opens Reports without a connection
- **THEN** the previously cached list is shown, marked as an offline copy

#### Scenario: Listing while offline, never synced

- **GIVEN** a device that has never successfully listed reports
- **WHEN** the reader opens Reports without a connection
- **THEN** the list fails to load rather than showing as empty and current

#### Scenario: The session is rejected

- **WHEN** the data plane answers a list request as unauthorized
- **THEN** the list fails to load rather than falling back to a cached copy

### Requirement: Writing a report requires an active session

Capturing, renaming, pinning, unpinning, and deleting a report all MUST
require a usable data-plane session, refused clearly when one is not
available — the same precedent chat's own message-send already set: reads
work offline, writes do not.

Where a list is served as an offline copy, that is already known before the
reader acts, so the surface MUST present those writes as unavailable rather
than offer them and fail. The reason MUST be stated once where the list is
read, in a form a screen reader receives. A write that was already begun when
the list became an offline copy MUST be refused with what was kept intact,
not retried into the same failure.

#### Scenario: Acting on an offline copy

- **GIVEN** a list served as an offline copy
- **WHEN** the reader looks at a report's actions
- **THEN** renaming, pinning and deleting are unavailable, and the list states
  why in a form a screen reader receives

#### Scenario: A write already open when the list goes stale

- **GIVEN** a rename or delete begun while the list was current
- **WHEN** the list becomes an offline copy before the write is sent
- **THEN** the write is refused, stating what was kept, and nothing is sent

#### Scenario: Capturing while signed out

- **GIVEN** no active account is signed in
- **WHEN** the reader tries to save a report
- **THEN** the save is refused with a message naming that a sign-in is
  needed, not a generic storage error

#### Scenario: Renaming with an expired session

- **GIVEN** a session that has expired
- **WHEN** the reader renames a report
- **THEN** the rename is refused, stating the session expired

### Requirement: Reports saved before this capability existed migrate to the data plane once

The system MUST, on the first list after upgrading an account that has
reports saved under the earlier local-only storage, submit each one it can
still read to the data plane, and MUST NOT repeat this migration on a later
list.

The earlier store holds the only copy of every row in it, so the system MUST
NOT remove it except when every row was confirmed accepted by the data plane.
A row that failed for a reason a retry could resolve MUST leave the migration
unrecorded, so the next list attempts it again; a resubmission MUST NOT create
a second copy of a report that already crossed. A row that no retry could ever
resolve MUST complete the migration without removing the store — its bytes
must not be destroyed in order to record that nothing could be done with them.

#### Scenario: An account with pre-existing local reports

- **GIVEN** an account with reports saved before this capability existed
- **WHEN** the reader opens Reports for the first time after upgrading
- **THEN** each readable report is submitted to the data plane, and it is
  visible there afterward
- **AND** the earlier store is removed only once every row has crossed

#### Scenario: The data plane is unreachable mid-migration

- **GIVEN** an account whose migration cannot reach the data plane
- **WHEN** the reader opens Reports
- **THEN** the earlier store is kept and the migration stays unrecorded
- **AND** the next list retries it without duplicating anything already sent

#### Scenario: A row nothing can resolve

- **GIVEN** a stored row that names no conversation the data plane knows
- **WHEN** the migration runs
- **THEN** the migration completes and does not run again
- **AND** the earlier store is kept, because it holds the only copy

#### Scenario: Migration runs exactly once

- **GIVEN** an account whose migration has already completed
- **WHEN** Reports is opened again
- **THEN** nothing already migrated is submitted a second time
