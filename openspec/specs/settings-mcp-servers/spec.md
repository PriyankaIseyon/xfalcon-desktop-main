# settings-mcp-servers Specification

## Purpose

Which MCP connections the reader can configure, and which are not theirs to configure.

## Scope

| File                                          | What it owns here                      |
| --------------------------------------------- | -------------------------------------- |
| `src/features/settings/McpServersSection.tsx` | the section and which servers it lists |
| `src/features/settings/McpServerSheet.tsx`    | adding and editing a server            |

## Boundaries

- **Server registration, transport, and tool discovery** belong to
  `mcp-server-registry`.
- **Re-authorizing an expired grant** belongs to `mcp-reauth-prompt`.
- **Tool approval policy** belongs to `tool-approval`.
- **The Settings grouping this section sits in** belongs to `settings-appearance`.

## Requirements

### Requirement: The bundled tenant server has no Settings card

The MCP servers section SHALL NOT render a card for the bundled tenant server. The
tenant connection has nothing to configure, and its former card carried approval copy
that auto-approve made false.

Servers the reader registered MUST keep their cards unchanged.

#### Scenario: Settings page without the tenant card

- **WHEN** the reader opens the MCP servers section
- **THEN** no card for the workspace-provided server is present
- **AND** reader-registered server cards render as before

## Provenance

From the delta of `refine-run-feedback-ux`. This capability is deliberately narrow:
everything else about MCP belongs to `mcp-server-registry` and `mcp-reauth-prompt`.
