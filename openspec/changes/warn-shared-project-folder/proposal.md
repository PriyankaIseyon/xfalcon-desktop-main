# Change: Warn when projects share a folder

## Why

Users can add the same folder to multiple projects without knowing that file edits affect every project using it. The user requested an advisory warning and an option to open an existing project while continuing to allow shared folders.

## What Changes

- Check selected folder identity against current-account projects through a read-only native command.
- Name matching projects and provide controls to open them.
- Explain shared edits and offer a different folder without blocking duplicate project creation.
- Use consistent "New project" labels and a content-sized dialog with a cyan primary folder action.
- Preserve explicit trust confirmation and display lookup or activation failures.

## Impact

- Affected spec: local-project-execution.
- Affected code: native project lookup, command registration, project creation dialog, and shell navigation.
- No persistence migration or change to project creation semantics.

The user's request authorizes implementation of this behavior.
