# lean-report-export — Spec Delta

## ADDED Requirements

### Requirement: Streamlined report export to PDF and Markdown

The system MUST allow users to export the current conversation analysis or saved report into a clean, standalone PDF or Markdown file via an export dialog.

The generated output MUST format tables, headings, and code blocks using clean typography and MUST omit raw system prompts or internal tool JSON payloads unless explicitly checked by the user.

#### Scenario: Exporting a conversation summary to PDF

- **GIVEN** an active conversation containing analysis text and structured tables
- **WHEN** the user selects "Export Document" -> "PDF"
- **THEN** a styled document is rendered and saved to the user's chosen file location

#### Scenario: Exporting to Markdown

- **GIVEN** an active conversation or saved report
- **WHEN** the user selects "Export Document" -> "Markdown (.md)"
- **THEN** a formatted `.md` file with table syntax and metadata headers is written to disk
