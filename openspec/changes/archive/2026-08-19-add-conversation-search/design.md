## Context

The audit that prompted this change pointed at Hermes Agent's search layer:
SQLite FTS5, a trigram index beside it, CJK triggers, and storage-version keys
that tell the index when to rebuild. That is the right design for Hermes, whose
session store is plaintext SQLite.

It cannot be ported here as-is. `EncryptedChatProjection` seals every row:
`chat_conversations` and `chat_messages` hold `nonce` and `ciphertext` and no
plaintext column at all (`0002_chat_projection.sql`). FTS5 indexes text. An FTS5
index over this store would have to hold conversation titles and message bodies
in the clear on disk, next to the ciphertext that exists so they are not.

## Goals / Non-Goals

- Goal: the palette finds a conversation by name.
- Goal: no plaintext conversation content is written to disk.
- Non-Goal: searching message bodies. That is the change that would force the
  index question, and it should be proposed on its own with the security
  trade-off stated, not smuggled in behind a palette fix.
- Non-Goal: fuzzy or ranked matching. There is no evidence yet that substring
  matching over titles is insufficient.

## Decisions

- **Decision: search titles in memory, not through a new index.** The shell
  already holds every conversation summary — `ChatWorkspaceBootstrap` returns
  the full list and the sidebar renders from it. Titles are short and the list
  is a user's own chats, not a corpus. Filtering it in the renderer costs
  nothing measurable and adds no storage, no migration, and no plaintext.
  - Alternatives considered: an FTS5 table over decrypted titles — rejected, it
    puts titles in the clear on disk for a list small enough to scan. A Rust
    search command over the sealed store — rejected, it would decrypt every row
    per keystroke to answer a question the renderer can already answer from data
    it holds.

- **Decision: normalise both sides before comparing.** Lowercase and strip
  diacritics via `NFD` + combining-mark removal. Auto-generated titles carry the
  user's own words, and a title typed with an accent must be findable without
  one.
  - Alternative considered: `localeCompare`-based collation — rejected, it
    answers ordering questions, not substring ones.

- **Decision: conversations are a labelled group, not merged into commands.**
  A palette that mixes "Settings" and "Q3 margin review" in one undifferentiated
  list makes the user read every row to find out what kind of thing it is.

- **Decision: cap the conversation group.** A query matching two hundred chats
  should not render two hundred rows into a palette whose purpose is to be
  faster than scrolling.

## Risks / Trade-offs

- Title-only search will miss a chat the user remembers by its content →
  accepted for now, and stated in the non-goals rather than hidden. The
  follow-up change has a real security decision to make and deserves its own
  proposal.
- Matching runs on every keystroke over the whole list → the list is one
  account's conversations and each comparison is a substring test on a short
  string; if this ever needs memoising, the measurement will say so.

## Migration Plan

None. No schema, no stored state, no contract change. The palette's props change
and its single call site is updated in the same change.

## Open Questions

None blocking. Whether message-body search is wanted at all is the user's call,
and is deliberately left outside this change.
