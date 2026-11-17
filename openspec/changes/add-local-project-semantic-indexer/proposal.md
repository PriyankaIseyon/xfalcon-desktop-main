# Local Project Semantic Search & Structured Schema Indexer

## Why

When working with local project folders containing dozens of data sheets, notes, PDFs, or SQLite files, sending entire files into the prompt context window quickly exhausts token budgets, raises costs, and degrades inference quality.

A **Local Project Semantic Search & Structured Schema Indexer** indexes local project files on the client side using a fast Rust-powered engine. It extracts table schemas from structured files (CSV, Parquet, JSON, SQLite) and creates a local hybrid (BM25 + vector) index, allowing the assistant to query schemas and retrieve relevant snippets with high precision.

## What Changes

- **Rust Background Indexer:** Scans bound local project directories in the background, extracting chunked text from `.txt`, `.md`, `.csv`, `.json`, `.sql`, and `.pdf` files.
- **Local SQLite/BM25 Index:** Stores terms and inverted index locally under `<app_local_data>/indexes/<project_id>.sqlite3` with zero external cloud egress.
- **Schema Extractor Tool:** Automatic schema extraction for CSV/JSON/SQLite tables (column names, inferred data types, sample rows) injected into system context so the model knows the exact schema without reading all data rows.
- **Project Search Tool for Agents:** A native Rust tool `project_search(query, top_k)` allowing the assistant to retrieve relevant document chunks dynamically during reasoning loops.

## Capabilities Affected

- `local-project-semantic-indexer` (new capability)
