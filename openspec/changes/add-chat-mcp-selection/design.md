# Chat MCP isolation

The user authorized implementation and confirmed that a changed MCP selection
starts a fresh chat while preserving the old chat.

## Durable boundary

Store a versioned selection in conversation creation metadata under
`xfalcon_desktop_mcp`. All stores explicit null server ID and endpoint fields. A specific selection
contains the registered server ID (or the reserved bundled ID) and the normalized
endpoint resolved by native code. The renderer supplies only a server ID.

The binding is immutable for that conversation. Existing rows with no reserved
metadata are legacy All conversations. Malformed reserved metadata is an error,
never a reason to widen access. A dedicated typed command reads this binding;
an unavailable read prevents submission rather than displaying an assumed All.

## Execution boundary

Every submission path validates the requested selection against the conversation
binding before truncating or appending history. Native code composes a runtime
containing exactly the selected server. It checks the persisted endpoint against
the current registry and rejects a removed, disabled, denied, unauthenticated or
repointed server. It never substitutes the bundled server or All.
Selected chats also disable DA's legacy automatic attachment of all MCP servers.

The admitted run captures its runtime set. Tool schemas, prompt inventory and
dispatch use that same set across provider rounds, reconnects and approval
pauses. All preserves the existing runtime behavior.

Selected runtimes use a reserved namespace derived from the immutable server ID.
Registry display names and their mutable slugs cannot authorize old tool calls.
Admission also captures the account and session generation. A changed session
rejects the submission, and the bundled selected runtime cannot authenticate
with another account's bearer during a reconnect.

## History boundary

Changing a selection creates a fresh conversation; no prior messages, tool
results, exact-result handles, compaction summaries, attachments or drafts are
transferred. The previous conversation remains available. Opening an existing
chat restores its own selection. Attachment reservations receive the same
selection as a normal first send. Retry, edit and regenerate cannot change it.

Every authoritative context load checks tool-call and tool-result names against
the captured scope before summaries or pruning can hide them. Native compaction
fingerprints include the selected server ID and endpoint. An incompatible saved
compaction fails instead of being replayed as plain text. Client summary caches
also carry their MCP scope, so a summary from another scope cannot be reused.

User-supplied message text and attachments remain explicit user input. The
selector controls MCP tool access and conversation context; it does not switch
the configured workspace data plane.

## Platform boundary

React and native Rust enforcement are shared by Windows and macOS. Validate
native behavior with Windows tests available on this host. Record macOS checks
separately and never describe cross-compilation or source review as native app
execution. Linux app behavior is outside this change's validation scope.
