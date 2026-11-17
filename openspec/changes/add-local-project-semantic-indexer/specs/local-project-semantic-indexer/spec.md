# local-project-semantic-indexer — Spec Delta

## ADDED Requirements

### Requirement: Local background file indexing and schema extraction

The system MUST index bound local project files in the background using a local SQLite FTS5 / BM25 engine without transmitting file content to external servers.

For tabular files (CSV, JSON, SQLite), the system MUST automatically extract column names, inferred data types, and brief summary statistics to serve as structured schema context.

#### Scenario: Indexing a newly bound local project

- **GIVEN** a user binds a local project folder containing CSV files and Markdown docs
- **WHEN** the project is opened in xFalcon Desktop
- **THEN** the background indexer processes the files, updates the index status to "Ready", and extracts tabular schemas

### Requirement: Index honors gitignore and never walks dot directories

The indexer SHALL honor `.gitignore` inside a real git repository boundary (`require_git(true)`). It SHALL NOT index, search, or even enter a directory whose name starts with `.`, including `.git`, `.github`, `.venv`, and `.xfalcon`. Dotfiles at any depth SHALL not be added to the index.

A later pass MUST prune rows for those paths if an older index still holds them.

#### Scenario: A gitignored file is not indexed

- **GIVEN** a trusted project that is a git repository whose `.gitignore` lists `*.log`
- **WHEN** an index pass completes
- **THEN** no `*.log` path is in the index

#### Scenario: Dot directories are never entered

- **GIVEN** a trusted project containing `.github/workflows/ci.yml` and `src/main.rs`
- **WHEN** an index pass completes
- **THEN** the index holds `src/main.rs`
- **AND** it does not hold any path under `.github`

### Requirement: Dynamic search retrieval tool for agent workflows

The system MUST provide a `project_search` tool to the agent runtime that queries the local index, returning the top matching snippets with file paths and line offsets.

The agent MUST use search results to answer queries without requiring the user to manually paste large documents into the chat prompt.

#### Scenario: Querying across multiple project documents

- **GIVEN** a question about historical financial data across multiple sheets
- **WHEN** the assistant processes the query
- **THEN** it executes `project_search`, retrieves relevant chunks, and cites the source files in its final answer
