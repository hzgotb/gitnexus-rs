use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::Result;
use serde_json::{Value, json};

use crate::commands::{delegate::run_ts_cli, local_mcp::call_tool_json};
use crate::storage::repo_manager::{find_repo, list_registered_repos};

const TS_FALLBACK_ENV: &str = "GITNEXUS_USE_TS_AUGMENT";

pub fn run(pattern: &str) -> Result<()> {
    if should_use_ts_fallback(TS_FALLBACK_ENV) {
        let args = vec![pattern.to_string()];
        return run_ts_cli("augment", &args);
    }

    let pattern = pattern.trim();
    if pattern.len() < 3 {
        return Ok(());
    }

    if let Some(text) = build_augmentation(pattern)? {
        // Keep stderr contract for hook compatibility.
        eprintln!("{text}");
    }

    Ok(())
}

fn build_augmentation(pattern: &str) -> Result<Option<String>> {
    let mut args = json!({
        "query": pattern,
        "limit": 3
    });

    if let Some(repo_name) = resolve_repo_name_for_cwd()? {
        args["repo"] = Value::String(repo_name);
    }

    let result = match call_tool_json("query", &args) {
        Ok(result) => result,
        Err(_) => return Ok(None),
    };

    Ok(format_augmentation(&result))
}

fn format_augmentation(result: &Value) -> Option<String> {
    let mut process_labels = HashMap::<String, String>::new();
    if let Some(processes) = result.get("processes").and_then(Value::as_array) {
        for process in processes {
            let Some(id) = process.get("id").and_then(Value::as_str) else {
                continue;
            };

            let label = process
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_string();
            process_labels.insert(id.to_string(), label);
        }
    }

    let mut symbols = BTreeMap::<String, (String, String, String, Vec<String>)>::new();
    if let Some(rows) = result.get("process_symbols").and_then(Value::as_array) {
        for row in rows.iter().take(20) {
            let Some(id) = row.get("id").and_then(Value::as_str) else {
                continue;
            };

            let name = row
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let file_path = row
                .get("filePath")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let sym_type = row
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("Symbol")
                .to_string();

            let entry =
                symbols
                    .entry(id.to_string())
                    .or_insert((name, sym_type, file_path, Vec::new()));

            if let Some(process_id) = row.get("process_id").and_then(Value::as_str)
                && let Some(label) = process_labels.get(process_id)
                && !entry.3.contains(label)
            {
                entry.3.push(label.clone());
            }
        }
    }

    if symbols.is_empty()
        && let Some(defs) = result.get("definitions").and_then(Value::as_array)
    {
        for row in defs.iter().take(5) {
            let Some(id) = row.get("id").and_then(Value::as_str) else {
                continue;
            };
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let file_path = row
                .get("filePath")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let sym_type = row
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("Symbol")
                .to_string();

            symbols.insert(id.to_string(), (name, sym_type, file_path, Vec::new()));
        }
    }

    if symbols.is_empty() {
        return None;
    }

    let mut rows = symbols
        .into_values()
        .map(|(name, sym_type, file_path, flows)| {
            let flow_count = flows.len();
            (name, sym_type, file_path, flows, flow_count)
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| b.4.cmp(&a.4));
    rows.truncate(5);

    let mut lines = Vec::<String>::new();
    lines.push(format!("[GitNexus] {} related symbols found:", rows.len()));
    lines.push(String::new());

    for (name, sym_type, file_path, mut flows, _) in rows {
        lines.push(format!("{sym_type} {name} ({file_path})"));
        if !flows.is_empty() {
            flows.truncate(3);
            lines.push(format!("  Flows: {}", flows.join(", ")));
        }
        lines.push(String::new());
    }

    Some(lines.join("\n").trim_end().to_string())
}

fn resolve_repo_name_for_cwd() -> Result<Option<String>> {
    let cwd = std::env::current_dir()?;
    let Some(repo) = find_repo(&cwd)? else {
        return Ok(None);
    };

    let entries = list_registered_repos(true)?;
    let current_repo_path = repo.repo_path;

    for entry in entries {
        if path_matches(Path::new(&entry.path), &current_repo_path) {
            return Ok(Some(entry.name));
        }
    }

    Ok(None)
}

fn path_matches(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    let left = left.canonicalize().ok();
    let right = right.canonicalize().ok();
    matches!((left, right), (Some(a), Some(b)) if a == b)
}

fn should_use_ts_fallback(var_name: &str) -> bool {
    let Some(raw) = std::env::var_os(var_name) else {
        return false;
    };

    let text = raw.to_string_lossy().trim().to_ascii_lowercase();
    !text.is_empty() && text != "0" && text != "false" && text != "no"
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::format_augmentation;

    #[test]
    fn format_augmentation_from_process_symbols() {
        let payload = json!({
            "processes": [
                { "id": "p1", "summary": "Auth Login Flow" }
            ],
            "process_symbols": [
                {
                    "id": "sym-1",
                    "name": "validate_user",
                    "type": "Function",
                    "filePath": "src/auth.rs",
                    "process_id": "p1"
                },
                {
                    "id": "sym-2",
                    "name": "AuthService",
                    "type": "Struct",
                    "filePath": "src/service.rs",
                    "process_id": "p1"
                }
            ]
        });

        let text = format_augmentation(&payload).expect("expected augmentation output");
        assert!(text.contains("[GitNexus] 2 related symbols found:"));
        assert!(text.contains("Function validate_user (src/auth.rs)"));
        assert!(text.contains("Struct AuthService (src/service.rs)"));
        assert!(text.contains("Flows: Auth Login Flow"));
    }

    #[test]
    fn format_augmentation_falls_back_to_definitions() {
        let payload = json!({
            "definitions": [
                {
                    "id": "def-1",
                    "name": "GraphNode",
                    "type": "Struct",
                    "filePath": "src/graph.rs"
                }
            ]
        });

        let text = format_augmentation(&payload).expect("expected definition fallback output");
        assert!(text.contains("[GitNexus] 1 related symbols found:"));
        assert!(text.contains("Struct GraphNode (src/graph.rs)"));
    }

    #[test]
    fn format_augmentation_returns_none_for_empty_results() {
        let payload = json!({
            "processes": [],
            "process_symbols": [],
            "definitions": []
        });

        assert!(format_augmentation(&payload).is_none());
    }
}
