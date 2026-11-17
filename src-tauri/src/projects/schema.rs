//! Schema extraction for tabular and structured files (CSV, JSON, SQLite)
//! in trusted projects (spec: local-project-semantic-indexer).
//!
//! Produces structured table metadata (`ProjectSchemaSummary`) so the agent
//! loop can understand dataset schemas without exhausting token budgets on raw rows.

use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use xfalcon_contracts::projects::{ProjectColumnSchema, ProjectTableSchema};

#[derive(Debug, thiserror::Error)]
pub enum SchemaExtractError {
    #[error("project root unavailable or not found")]
    ProjectNotFound,
    #[error("invalid or unsafe identifier: contains null bytes")]
    InvalidIdentifier,
    #[error("file io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Safely truncate string to `max_chars` UTF-8 code points without panicking on multi-byte characters.
pub fn truncate_sample_cell(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}…")
    } else {
        s.to_string()
    }
}

/// Infer basic data type string from a slice of string values.
///
/// Returns "string" for all-empty input, "boolean" strictly for "true"/"false" (case-insensitive),
/// "integer" for integer literals, "float" for floating point numbers, and "string" otherwise.
pub fn infer_type(values: &[&str]) -> &'static str {
    let non_empty: Vec<&str> = values
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect();
    if non_empty.is_empty() {
        return "string";
    }

    let mut is_bool = true;
    let mut is_int = true;
    let mut is_float = true;

    for v in &non_empty {
        let lower = v.to_lowercase();
        if !matches!(lower.as_str(), "true" | "false") {
            is_bool = false;
        }
        if v.parse::<i64>().is_err() {
            is_int = false;
        }
        if v.parse::<f64>().is_err() {
            is_float = false;
        }
    }

    if is_bool {
        "boolean"
    } else if is_int {
        "integer"
    } else if is_float {
        "float"
    } else {
        "string"
    }
}

/// Robust RFC4180-compliant CSV parser handling quoted multiline fields,
/// escaped quotes (""), varied CRLF/LF line endings, and unquoted internal quotes (e.g. 6" pipe).
///
/// Preserves whitespace inside quoted fields while trimming unquoted fields.
/// Parses all rows in the content to provide an exact row count.
pub fn parse_csv_document(content: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_field = String::new();
    let mut in_quotes = false;
    let mut field_is_quoted = false;
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' if !field_is_quoted && current_field.trim().is_empty() => {
                // Opening quote at the start of a field
                in_quotes = true;
                field_is_quoted = true;
                current_field.clear();
            }
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    // Escaped quote inside quoted field
                    current_field.push('"');
                    chars.next();
                } else {
                    // Closing quote
                    in_quotes = false;
                }
            }
            ',' if !in_quotes => {
                let val = if field_is_quoted {
                    std::mem::take(&mut current_field)
                } else {
                    current_field.trim().to_string()
                };
                current_row.push(val);
                current_field.clear();
                field_is_quoted = false;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                if in_quotes {
                    current_field.push('\n');
                } else {
                    let val = if field_is_quoted {
                        std::mem::take(&mut current_field)
                    } else {
                        current_field.trim().to_string()
                    };
                    current_row.push(val);
                    current_field.clear();
                    field_is_quoted = false;
                    if !current_row.is_empty()
                        && (current_row.len() > 1 || !current_row[0].is_empty())
                    {
                        rows.push(std::mem::take(&mut current_row));
                    }
                }
            }
            '\n' => {
                if in_quotes {
                    current_field.push('\n');
                } else {
                    let val = if field_is_quoted {
                        std::mem::take(&mut current_field)
                    } else {
                        current_field.trim().to_string()
                    };
                    current_row.push(val);
                    current_field.clear();
                    field_is_quoted = false;
                    if !current_row.is_empty()
                        && (current_row.len() > 1 || !current_row[0].is_empty())
                    {
                        rows.push(std::mem::take(&mut current_row));
                    }
                }
            }
            _ => {
                current_field.push(c);
            }
        }
    }

    if in_quotes {
        tracing::warn!("unterminated quote detected in CSV document, remaining content treated as closed field");
    }

    if !current_field.is_empty() || !current_row.is_empty() || field_is_quoted {
        let val = if field_is_quoted {
            current_field
        } else {
            current_field.trim().to_string()
        };
        current_row.push(val);
        if !current_row.is_empty() && (current_row.len() > 1 || !current_row[0].is_empty()) {
            rows.push(current_row);
        }
    }

    if rows.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let headers = rows.remove(0);
    (headers, rows)
}

/// Extract schema from a CSV file content without artificial row limits.
pub fn extract_csv_schema(
    table_name: &str,
    relative_path: &str,
    content: &str,
) -> Option<ProjectTableSchema> {
    let (headers, rows) = parse_csv_document(content);
    if headers.is_empty() {
        return None;
    }

    let mut columns = Vec::with_capacity(headers.len());
    for (i, header) in headers.iter().enumerate() {
        let col_values: Vec<&str> = rows
            .iter()
            .filter_map(|r| r.get(i).map(|s| s.as_str()))
            .collect();
        let inferred = infer_type(&col_values);
        let sample_values = col_values
            .iter()
            .take(3)
            .map(|s| truncate_sample_cell(s, 60))
            .collect();

        columns.push(ProjectColumnSchema {
            name: header.clone(),
            inferred_type: inferred.to_string(),
            sample_values,
        });
    }

    let total_rows = rows.len() as u64;
    let sample_rows = rows
        .into_iter()
        .take(3)
        .map(|r| {
            r.into_iter()
                .map(|s| truncate_sample_cell(&s, 60))
                .collect()
        })
        .collect();

    Some(ProjectTableSchema {
        name: table_name.to_string(),
        relative_path: relative_path.to_string(),
        format: "csv".to_string(),
        columns,
        sample_rows,
        row_count_estimate: Some(total_rows),
    })
}

/// Extract schema from a JSON file content (array of objects or single object).
pub fn extract_json_schema(
    table_name: &str,
    relative_path: &str,
    content: &str,
) -> Option<ProjectTableSchema> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    match value {
        serde_json::Value::Array(items) if !items.is_empty() => {
            let mut keys: Vec<String> = Vec::new();
            let mut seen_keys = std::collections::HashSet::new();
            for item in &items {
                if let serde_json::Value::Object(map) = item {
                    for k in map.keys() {
                        if seen_keys.insert(k.clone()) {
                            keys.push(k.clone());
                        }
                    }
                }
            }
            if keys.is_empty() {
                return None;
            }

            let mut columns = Vec::with_capacity(keys.len());
            for key in &keys {
                let mut sample_values = Vec::with_capacity(3);
                let mut primary_type = "string";
                for item in &items {
                    if let serde_json::Value::Object(map) = item {
                        if let Some(v) = map.get(key) {
                            if sample_values.is_empty() {
                                primary_type = match v {
                                    serde_json::Value::Bool(_) => "boolean",
                                    serde_json::Value::Number(n) => {
                                        if n.is_i64() || n.is_u64() {
                                            "integer"
                                        } else {
                                            "float"
                                        }
                                    }
                                    serde_json::Value::String(_) => "string",
                                    serde_json::Value::Array(_) => "array",
                                    serde_json::Value::Object(_) => "object",
                                    serde_json::Value::Null => "null",
                                };
                            }
                            sample_values.push(truncate_sample_cell(&v.to_string(), 60));
                            if sample_values.len() >= 3 {
                                break;
                            }
                        }
                    }
                }
                columns.push(ProjectColumnSchema {
                    name: key.clone(),
                    inferred_type: primary_type.to_string(),
                    sample_values,
                });
            }

            let mut sample_rows = Vec::new();
            for item in items.iter().take(3) {
                if let serde_json::Value::Object(map) = item {
                    let row = keys
                        .iter()
                        .map(|k| {
                            let cell = map.get(k).map(|v| v.to_string()).unwrap_or_default();
                            truncate_sample_cell(&cell, 60)
                        })
                        .collect();
                    sample_rows.push(row);
                }
            }

            Some(ProjectTableSchema {
                name: table_name.to_string(),
                relative_path: relative_path.to_string(),
                format: "json".to_string(),
                columns,
                sample_rows,
                row_count_estimate: Some(items.len() as u64),
            })
        }
        serde_json::Value::Object(map) if !map.is_empty() => {
            let mut columns = Vec::new();
            let mut row = Vec::new();
            for (k, v) in &map {
                let type_str = match v {
                    serde_json::Value::Bool(_) => "boolean",
                    serde_json::Value::Number(n) => {
                        if n.is_i64() || n.is_u64() {
                            "integer"
                        } else {
                            "float"
                        }
                    }
                    serde_json::Value::String(_) => "string",
                    serde_json::Value::Array(_) => "array",
                    serde_json::Value::Object(_) => "object",
                    serde_json::Value::Null => "null",
                };
                let val_str = truncate_sample_cell(&v.to_string(), 60);
                columns.push(ProjectColumnSchema {
                    name: k.clone(),
                    inferred_type: type_str.to_string(),
                    sample_values: vec![val_str.clone()],
                });
                row.push(val_str);
            }
            Some(ProjectTableSchema {
                name: table_name.to_string(),
                relative_path: relative_path.to_string(),
                format: "json".to_string(),
                columns,
                sample_rows: vec![row],
                row_count_estimate: Some(1),
            })
        }
        _ => None,
    }
}

/// Quote-escape SQLite identifier.
pub fn escape_sqlite_identifier(ident: &str) -> Result<String, SchemaExtractError> {
    if ident.contains('\0') {
        return Err(SchemaExtractError::InvalidIdentifier);
    }
    Ok(format!("\"{}\"", ident.replace('"', "\"\"")))
}

/// Extract every schema from a local SQLite database file.
pub fn extract_sqlite_schemas(
    db_path: &Path,
    relative_path: &str,
) -> Result<Vec<ProjectTableSchema>, SchemaExtractError> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    )?;
    let table_names: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(Result::ok)
        .collect();

    let mut tables = Vec::new();
    for table in table_names {
        let escaped_table = escape_sqlite_identifier(&table)?;
        let mut pragma_stmt = conn.prepare(&format!("PRAGMA table_info({escaped_table})"))?;
        let columns_meta: Vec<(String, String)> = pragma_stmt
            .query_map([], |row| {
                let name: String = row.get(1)?;
                let col_type: String = row.get(2)?;
                Ok((name, col_type))
            })?
            .filter_map(Result::ok)
            .collect();

        let mut columns = Vec::new();
        for (col_name, declared_type) in &columns_meta {
            let escaped_col = escape_sqlite_identifier(col_name)?;
            let mut sample_stmt = conn.prepare(&format!(
                "SELECT {escaped_col} FROM {escaped_table} WHERE {escaped_col} IS NOT NULL LIMIT 3"
            ))?;
            let samples: Vec<String> = sample_stmt
                .query_map([], |row| {
                    let val: rusqlite::types::Value = row.get(0)?;
                    Ok(match val {
                        rusqlite::types::Value::Null => "null".to_string(),
                        rusqlite::types::Value::Integer(i) => i.to_string(),
                        rusqlite::types::Value::Real(f) => f.to_string(),
                        rusqlite::types::Value::Text(t) => truncate_sample_cell(&t, 60),
                        rusqlite::types::Value::Blob(b) => format!("<blob {} bytes>", b.len()),
                    })
                })?
                .filter_map(Result::ok)
                .collect();

            let inferred = if declared_type.trim().is_empty() {
                "string"
            } else {
                declared_type.as_str()
            };

            columns.push(ProjectColumnSchema {
                name: col_name.clone(),
                inferred_type: inferred.to_lowercase(),
                sample_values: samples,
            });
        }

        let mut row_sample_stmt =
            conn.prepare(&format!("SELECT * FROM {escaped_table} LIMIT 3"))?;
        let col_count = columns_meta.len();
        let sample_rows: Vec<Vec<String>> = row_sample_stmt
            .query_map([], |row| {
                let mut vals = Vec::with_capacity(col_count);
                for i in 0..col_count {
                    let val: rusqlite::types::Value = row.get(i)?;
                    let formatted = match val {
                        rusqlite::types::Value::Null => "null".to_string(),
                        rusqlite::types::Value::Integer(v) => v.to_string(),
                        rusqlite::types::Value::Real(v) => v.to_string(),
                        rusqlite::types::Value::Text(v) => truncate_sample_cell(&v, 60),
                        rusqlite::types::Value::Blob(b) => format!("<blob {} bytes>", b.len()),
                    };
                    vals.push(formatted);
                }
                Ok(vals)
            })?
            .filter_map(Result::ok)
            .collect();

        let count: u64 = conn
            .query_row(&format!("SELECT count(*) FROM {escaped_table}"), [], |r| {
                r.get(0)
            })
            .unwrap_or(0);

        tables.push(ProjectTableSchema {
            name: table,
            relative_path: relative_path.to_string(),
            format: "sqlite".to_string(),
            columns,
            sample_rows,
            row_count_estimate: Some(count),
        });
    }

    Ok(tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_csv_schema_extraction_large_rows_uncapped() {
        let mut csv = String::from("id,name,active,score\n");
        for i in 1..=500 {
            csv.push_str(&format!("{i},User{i},true,{}.5\n", i % 100));
        }
        let schema = extract_csv_schema("users", "data/users.csv", &csv).expect("valid csv schema");
        assert_eq!(schema.name, "users");
        assert_eq!(schema.columns.len(), 4);
        assert_eq!(schema.columns[0].name, "id");
        assert_eq!(schema.columns[0].inferred_type, "integer");
        assert_eq!(schema.columns[1].inferred_type, "string");
        assert_eq!(schema.columns[2].inferred_type, "boolean");
        assert_eq!(schema.columns[3].inferred_type, "float");
        assert_eq!(schema.sample_rows.len(), 3);
        // Fully accurate without 100-row cap
        assert_eq!(schema.row_count_estimate, Some(500));
    }

    #[test]
    fn test_non_ascii_multibyte_truncation_does_not_panic() {
        let long_multibyte = "🎉".repeat(40);
        let truncated = truncate_sample_cell(&long_multibyte, 20);
        assert!(truncated.ends_with('…'));
        assert_eq!(truncated.chars().count(), 20);

        let csv = format!("id,text\n1,\"{}\"", "é".repeat(100));
        let schema = extract_csv_schema("test", "test.csv", &csv).expect("must parse multibyte");
        assert_eq!(schema.columns[1].sample_values[0].chars().count(), 60);
    }

    #[test]
    fn test_unquoted_quotes_like_pipe_specification() {
        let csv = "id,spec,notes\n1,6\" pipe,standard\n2,12\" cable,heavy-duty";
        let schema =
            extract_csv_schema("hardware", "hardware.csv", csv).expect("valid hardware csv");
        assert_eq!(schema.columns.len(), 3);
        assert_eq!(schema.sample_rows.len(), 2);
        assert_eq!(schema.sample_rows[0][1], "6\" pipe");
        assert_eq!(schema.sample_rows[1][1], "12\" cable");
        assert_eq!(schema.row_count_estimate, Some(2));
    }

    #[test]
    fn test_empty_column_infer_type() {
        assert_eq!(infer_type(&["", "   ", "\n"]), "string");
        assert_eq!(infer_type(&["1", "0"]), "integer");
        assert_eq!(infer_type(&["true", "false"]), "boolean");
    }

    #[test]
    fn test_rfc4180_quotes_and_newlines_and_padding_csv() {
        let csv = "id,notes,padded\n1,\"Line 1\nLine 2\",\"  keep padding  \"\n2,\"He said \"\"hello\"\"\",unquoted  ";
        let schema = extract_csv_schema("notes", "notes.csv", csv).expect("multiline csv");
        assert_eq!(schema.columns.len(), 3);
        assert_eq!(schema.sample_rows.len(), 2);
        assert_eq!(schema.sample_rows[0][1], "Line 1\nLine 2");
        assert_eq!(schema.sample_rows[0][2], "  keep padding  ");
        assert_eq!(schema.sample_rows[1][1], "He said \"hello\"");
        assert_eq!(schema.sample_rows[1][2], "unquoted");
        assert_eq!(schema.row_count_estimate, Some(2));
    }

    #[test]
    fn test_unterminated_quote_csv_recovers_gracefully() {
        let csv = "id,notes\n1,\"unterminated string";
        let schema =
            extract_csv_schema("bad", "bad.csv", csv).expect("recovers without dropping file");
        assert_eq!(schema.columns.len(), 2);
        assert_eq!(schema.sample_rows.len(), 1);
        assert_eq!(schema.sample_rows[0][1], "unterminated string");
    }

    #[test]
    fn test_sqlite_identifier_escaping() {
        let escaped = escape_sqlite_identifier("table \"weird\"").unwrap();
        assert_eq!(escaped, "\"table \"\"weird\"\"\"");
        assert!(escape_sqlite_identifier("table\0bad").is_err());
    }

    #[test]
    fn test_real_sqlite_database_extraction() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("test_schema.db");

        {
            let conn = Connection::open(&db_path).expect("open db");
            conn.execute_batch(
                "CREATE TABLE customers (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    balance REAL,
                    avatar BLOB
                );
                INSERT INTO customers (id, name, balance, avatar) VALUES (1, 'Alice', 105.50, X'DEADBEEF');
                INSERT INTO customers (id, name, balance, avatar) VALUES (2, 'Bob', 25.00, NULL);"
            ).expect("seed db");
        }

        let schemas =
            extract_sqlite_schemas(&db_path, "data/test_schema.db").expect("extract sqlite");
        assert_eq!(schemas.len(), 1);
        let table = &schemas[0];
        assert_eq!(table.name, "customers");
        assert_eq!(table.format, "sqlite");
        assert_eq!(table.columns.len(), 4);
        assert_eq!(table.columns[0].name, "id");
        assert_eq!(table.columns[0].inferred_type, "integer");
        assert_eq!(table.columns[1].name, "name");
        assert_eq!(table.columns[1].inferred_type, "text");
        assert_eq!(table.columns[2].name, "balance");
        assert_eq!(table.columns[2].inferred_type, "real");
        assert_eq!(table.columns[3].name, "avatar");
        assert_eq!(table.columns[3].inferred_type, "blob");
        assert_eq!(table.row_count_estimate, Some(2));
        assert_eq!(table.sample_rows.len(), 2);
    }

    #[test]
    fn test_json_schema_extraction() {
        let json = r#"[
            {"product_id": 101, "title": "Keyboard", "price": 49.99, "in_stock": true},
            {"product_id": 102, "title": "Mouse", "price": 24.50, "in_stock": false}
        ]"#;
        let schema =
            extract_json_schema("products", "data/products.json", json).expect("valid json schema");
        assert_eq!(schema.name, "products");
        assert_eq!(schema.columns.len(), 4);
        assert_eq!(schema.columns[0].name, "product_id");
        assert_eq!(schema.columns[0].inferred_type, "integer");
        assert_eq!(schema.sample_rows.len(), 2);
        assert_eq!(schema.row_count_estimate, Some(2));
    }

    #[test]
    fn test_json_array_discovers_keys_after_item_fifty() {
        let mut items = Vec::new();
        for index in 0..50 {
            items.push(serde_json::json!({"id": index}));
        }
        items.push(serde_json::json!({"id": 50, "late_key": "visible"}));

        let schema = extract_json_schema(
            "events",
            "data/events.json",
            &serde_json::to_string(&items).expect("serialize fixtures"),
        )
        .expect("valid json schema");

        assert!(schema
            .columns
            .iter()
            .any(|column| column.name == "late_key"));
    }

    #[test]
    fn test_json_object_discovers_every_field() {
        let mut object = serde_json::Map::new();
        for index in 0..51 {
            object.insert(format!("field_{index:02}"), serde_json::json!(index));
        }

        let schema = extract_json_schema(
            "wide",
            "data/wide.json",
            &serde_json::Value::Object(object).to_string(),
        )
        .expect("valid json schema");

        assert_eq!(schema.columns.len(), 51);
        assert!(schema
            .columns
            .iter()
            .any(|column| column.name == "field_50"));
    }

    #[test]
    fn test_sqlite_discovers_every_table_and_column() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("wide.db");
        let conn = Connection::open(&db_path).expect("open db");
        for index in 0..20 {
            conn.execute(&format!("CREATE TABLE table_{index:02} (id INTEGER)"), [])
                .expect("create table");
        }
        let columns = (0..51)
            .map(|index| format!("column_{index:02} TEXT"))
            .collect::<Vec<_>>()
            .join(", ");
        conn.execute(&format!("CREATE TABLE table_20 ({columns})"), [])
            .expect("create wide table");
        drop(conn);

        let schemas = extract_sqlite_schemas(&db_path, "data/wide.db").expect("extract sqlite");
        assert_eq!(schemas.len(), 21);
        let wide = schemas
            .iter()
            .find(|table| table.name == "table_20")
            .expect("wide table");
        assert_eq!(wide.columns.len(), 51);
        assert!(wide.columns.iter().any(|column| column.name == "column_50"));
    }
}
