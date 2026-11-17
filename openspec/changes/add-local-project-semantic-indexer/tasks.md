# Tasks for Local Project Semantic Indexer

- [x] 1. Rust Indexer Core & Schema Extractor
  - [x] 1.1 Implement file scanner and chunker for Markdown, text, and structured files in `src-tauri/src/projects/index.rs`.
  - [x] 1.2 Implement CSV, JSON, and SQLite table schema extractor (extracting headers, types, and top-3 sample rows with UTF-8 safety and RFC4180 compliance).
  - [x] 1.3 Lexical search is delivered by the encrypted `IndexStore` plus `project.search_files` / `project.search_text` (incremental fingerprint skip, mark-and-sweep). A separate FTS5 virtual table is not the shipped engine.
  - [x] 1.4 Honor `.gitignore` inside a real git repository (`require_git(true)`). Never walk or index a path component whose name starts with `.`.

- [x] 2. Agent Tool & Native Search Command
  - [x] 2.1 Register `project.search_files` and `project.search_text` in the agent tool registry and dispatch loop, with path references and ranked results.
  - [x] 2.2 Add Tauri command `project_schema_summary(project_id)` to power UI context chips and dataset schemas.

- [x] 3. UI Index Status & Search Bar
  - [x] 3.1 Display project file tree and readiness status dot in the Project Explorer header.
  - [x] 3.2 Add quick search box in Project Explorer to let users search their project files locally.

- [x] 4. Verification & Tests
  - [x] 4.1 Rust unit tests for schema extraction across malformed/diverse CSV, JSON, and SQLite datasets.
  - [ ] 4.2 Benchmark tests for search retrieval latency (< 10ms for 1,000 files).
