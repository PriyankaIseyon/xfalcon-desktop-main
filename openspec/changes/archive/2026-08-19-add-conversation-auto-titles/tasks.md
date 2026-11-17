## 1. Derivation

- [x] 1.1 Pure `derive_title` — first meaningful line, word-boundary cut,
      ellipsis; `None` for an untitleable message
- [x] 1.2 Scaffolding stripper, so a wrapped request titles by its request
- [x] 1.3 Unit tests: long, multiline, multibyte, scaffolding-only, wrapped

## 2. Authority

- [x] 2.1 Migration creating the sealed per-conversation title-source table
- [x] 2.2 `TitleSource` ordering `derived < llm < user`
- [x] 2.3 Check-and-write in one transaction; a missing row reads as `user`
- [x] 2.4 `conversation_rename` records `user`

## 3. Instant stage

- [x] 3.1 Pass the derived title into `conversation_create` so it costs no extra
      round trip
- [x] 3.2 Record `derived` authority alongside it

## 4. Upgrade stage

- [x] 4.1 Title prompt with a strict JSON response shape, 64-token ceiling
- [x] 4.2 Issue it on the conversation's own provider and model, off the run's
      critical path
- [x] 4.3 Parse and clean the result; refuse an unusable one rather than write it
- [x] 4.4 Write at `llm` authority through the ladder
- [x] 4.5 Emit the rename so the sidebar updates without a refresh
- [x] 4.6 Audit entry for the titling call

## 5. Wiring

- [x] 5.1 `build.rs` command list and `security/capabilities.rs` allowlist if a
      command is added
- [x] 5.2 Regenerate `src/lib/generated/contracts.ts`
- [x] 5.3 Frontend applies the emitted title in place

## 6. Tests

- [x] 6.1 A new conversation is named without the model
- [x] 6.2 The upgrade replaces a derived title
- [x] 6.3 A user rename during flight survives the upgrade
- [x] 6.4 A failed upgrade leaves the derived title and does not fail the run
- [x] 6.5 An existing conversation with no authority row is never retitled
- [x] 6.6 Titling is not repeated on later turns
- [x] 6.7 Full suite, lint, `tsc -b`, cargo test
