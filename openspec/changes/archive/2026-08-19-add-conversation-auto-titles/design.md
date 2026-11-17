## Context

Titles are owned by the data plane: `ChatService::conversation_rename` calls
`da.update_conversation`, and `conversation_create` optionally sets one the same
way. The desktop holds no title authority of its own today — it holds a sealed
projection of what DA said.

Two stages of automatic titling need a notion of _how_ a title was decided, so a
background model call can replace a placeholder but never a person's choice.

## Goals / Non-Goals

- Goal: a conversation is named the moment it exists.
- Goal: a user's own title is never overwritten by automation.
- Goal: no new outbound destination and no new stored credential.
- Non-Goal: an auxiliary model tier or any new model setting.
- Non-Goal: re-titling an existing conversation as it goes on. One name, decided
  at the start, from the message that started it.
- Non-Goal: propagating title provenance to DA or to xfalcon-web.

## Decisions

- **Decision: title provenance is device-local.** Recording `derived|llm|user`
  in DA would need a new DA field, and `project.md` states new DA fields require
  a DA merge and deploy first. It is also unnecessary: the automatic titler runs
  only on this device, so the only race the ladder has to settle — a background
  call landing after the user typed a name — is entirely local. Provenance goes
  in a small sealed table in the local projection, keyed by account and
  conversation.
  - Consequence, stated rather than mitigated: a title set from xfalcon-web has
    no local provenance row. It is therefore treated as `user` authority, which
    is the safe direction — automation declines rather than overwrites.
  - Alternative considered: a DA field. Rejected as a cross-repo dependency for
    a purely local race.

- **Decision: the derived title is written on the same DA call that creates the
  conversation.** `conversation_create` already accepts an optional title. The
  instant stage costs no extra round trip and cannot fail independently of
  creation.

- **Decision: the upgrade uses the conversation's own provider and model.** A
  title is a handful of tokens; a separate cheap tier would mean a new setting,
  a new credential path, and a second provider to keep working, to save a cost
  that is already negligible. Reusing the conversation's provider also means the
  message being titled goes only where it was already going.
  - Alternative considered: a configurable auxiliary model, as Hermes has.
    Rejected under the simplest-sufficient rule; Hermes needs it because it runs
    unattended against many providers.

- **Decision: the model is constrained to JSON, thinking disabled, 64 tokens.**
  Without a schema the observed failure is a model answering the message instead
  of naming it. With one there is nothing to parse out of prose.

- **Decision: titling reads the user's opening message only.** Not the
  assistant's reply — that is what made it slow — and not later turns.

- **Decision: an untitleable opener produces no title rather than a bad one.** A
  message that is entirely machine-authored scaffolding names the scaffolding.

## Risks / Trade-offs

- The opening message is sent to the provider a second time, for titling → it is
  the same provider and the same credential the turn itself uses, so this adds
  no destination and no new exposure. Worth stating explicitly at review.
- A background failure leaves the derived title in place → that is the point of
  writing it first; the failure mode is a slightly worse name, never a nameless
  chat.
- Provenance is device-local, so two devices can disagree about whether a title
  was automatic → both resolve toward `user`, which declines to overwrite. The
  disagreement cannot produce data loss.

## Migration Plan

One additive migration creating the provenance table. Existing conversations
have no row, so they read as `user` authority and are left alone — automation
never touches a chat that already existed. No backfill, nothing to roll back
beyond dropping the table.

## Open Questions

None blocking.
