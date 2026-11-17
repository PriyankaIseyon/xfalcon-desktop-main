## ADDED Requirements

### Requirement: A project file previews from disk

The system MUST register an HTML or SVG project file for preview by reading it in the
backend, through the project's containment and sensitivity checks, so the document never
crosses IPC. A project preview MAY be up to 32 MiB; a document the renderer supplies (a
chat card) SHALL stay bounded at 2 MiB.

#### Scenario: A 20 MiB dashboard in a project

- **GIVEN** a 20 MiB HTML file in a trusted project
- **WHEN** it is opened in the file view
- **THEN** it previews from the preview origin, and no HTML was sent from the renderer

#### Scenario: A sensitive path

- **WHEN** a preview is requested for a path classified as sensitive
- **THEN** it is refused and nothing is read
