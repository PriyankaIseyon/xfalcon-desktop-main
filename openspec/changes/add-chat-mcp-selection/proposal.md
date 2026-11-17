# Change: Select MCP servers per chat

## Why

Every usable MCP server is currently available to every chat run. A user needs to
choose one environment without exposing another environment's tools or history.

## What Changes

- Add an MCP selector beside the composer model name, with All servers as the default.
- Bind a new conversation to All or one server and its normalized endpoint.
- Start a fresh chat when the user changes the selection. Preserve the previous chat.
- Enforce the binding before history mutations, tool declaration and dispatch.
- Preserve the binding for attachments, retries, edits, regeneration and reopen.
- Keep client ID and client secret onboarding unchanged.

## Impact

- Affected specs: chat-composer, run-execution, chat-mcp-selection.
- Affected code: chat contracts, composer and chat state, conversation creation,
  uploads, native chat admission, MCP runtime composition and security tests.
- Existing persisted conversations without a binding retain All servers.
- No DA schema, authentication, dependency or release-channel changes.
