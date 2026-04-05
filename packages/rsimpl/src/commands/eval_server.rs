use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};

use crate::commands::{delegate::run_ts_cli, local_mcp::call_tool_json};

const DEFAULT_PORT: u16 = 4848;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const TS_FALLBACK_ENV: &str = "GITNEXUS_USE_TS_EVAL_SERVER";

#[derive(Debug, Clone, Default)]
pub struct EvalServerOptions {
    pub port: Option<u16>,
    pub idle_timeout: Option<u32>,
}

#[derive(Clone)]
struct EvalServerState {
    repo_names: Arc<Vec<String>>,
    last_activity_unix: Arc<AtomicU64>,
    shutdown_requested: Arc<AtomicBool>,
}

pub fn run(options: EvalServerOptions) -> Result<()> {
    if should_use_ts_fallback(TS_FALLBACK_ENV) {
        let mut args = Vec::<String>::new();
        if let Some(port) = options.port {
            args.push("--port".to_string());
            args.push(port.to_string());
        }
        if let Some(idle_timeout) = options.idle_timeout {
            args.push("--idle-timeout".to_string());
            args.push(idle_timeout.to_string());
        }
        return run_ts_cli("eval-server", &args);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_server(options))
}

async fn run_server(options: EvalServerOptions) -> Result<()> {
    let port = options.port.unwrap_or(DEFAULT_PORT);
    let idle_timeout = options.idle_timeout.unwrap_or(0);
    let repos = load_repo_names()?;
    if repos.is_empty() {
        return Err(anyhow!(
            "GitNexus eval-server: No indexed repositories found. Run: gitnexus analyze"
        ));
    }

    eprintln!(
        "GitNexus eval-server: {} repo(s) loaded: {}",
        repos.len(),
        repos.join(", ")
    );

    let state = EvalServerState {
        repo_names: Arc::new(repos),
        last_activity_unix: Arc::new(AtomicU64::new(now_unix())),
        shutdown_requested: Arc::new(AtomicBool::new(false)),
    };

    if idle_timeout > 0 {
        tokio::spawn(run_idle_watchdog(state.clone(), idle_timeout));
    }

    let app = Router::new()
        .route("/health", get(handle_health))
        .route("/shutdown", post(handle_shutdown))
        .route("/tool/{name}", post(handle_tool))
        .fallback(not_found)
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;

    eprintln!("GitNexus eval-server: listening on http://127.0.0.1:{port}");
    eprintln!("  POST /tool/query    — search execution flows");
    eprintln!("  POST /tool/context  — 360-degree symbol view");
    eprintln!("  POST /tool/impact   — blast radius analysis");
    eprintln!("  POST /tool/cypher   — raw Cypher query");
    eprintln!("  GET  /health        — health check");
    eprintln!("  POST /shutdown      — graceful shutdown");
    if idle_timeout > 0 {
        eprintln!("  Auto-shutdown after {idle_timeout}s idle");
    }

    // Used by eval harnesses to detect readiness.
    println!("GITNEXUS_EVAL_SERVER_READY:{port}");

    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(state.shutdown_requested.clone()))
        .await?;

    Ok(())
}

async fn run_idle_watchdog(state: EvalServerState, timeout_secs: u32) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;

        if state.shutdown_requested.load(Ordering::SeqCst) {
            break;
        }

        let last = state.last_activity_unix.load(Ordering::SeqCst);
        if now_unix().saturating_sub(last) >= u64::from(timeout_secs) {
            eprintln!("GitNexus eval-server: Idle timeout reached, shutting down");
            state.shutdown_requested.store(true, Ordering::SeqCst);
            break;
        }
    }
}

async fn wait_for_shutdown(flag: Arc<AtomicBool>) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn handle_health(State(state): State<EvalServerState>) -> impl IntoResponse {
    mark_activity(&state);
    let payload = json!({
        "status": "ok",
        "repos": state.repo_names.as_ref()
    });
    (StatusCode::OK, axum::Json(payload))
}

async fn handle_shutdown(State(state): State<EvalServerState>) -> impl IntoResponse {
    mark_activity(&state);
    state.shutdown_requested.store(true, Ordering::SeqCst);
    (
        StatusCode::OK,
        axum::Json(json!({ "status": "shutting_down" })),
    )
}

async fn handle_tool(
    State(state): State<EvalServerState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    mark_activity(&state);

    if body.len() > MAX_BODY_BYTES {
        return text_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Error: Request body too large (max 1MB)",
        );
    }

    let args = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(value) if value.is_object() => value,
            Ok(_) => {
                return text_response(
                    StatusCode::BAD_REQUEST,
                    "Error: JSON body must be an object",
                );
            }
            Err(_) => return text_response(StatusCode::BAD_REQUEST, "Error: Invalid JSON body"),
        }
    };

    let result = match call_tool_json(&name, &args) {
        Ok(result) => result,
        Err(err) => {
            return text_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error: {err}"));
        }
    };

    let text = format!(
        "{}{}",
        format_tool_result(&name, &result),
        next_step_hint(&name)
    );
    text_response(StatusCode::OK, &text)
}

async fn not_found() -> impl IntoResponse {
    text_response(
        StatusCode::NOT_FOUND,
        "Not found. Use POST /tool/:name or GET /health",
    )
}

fn load_repo_names() -> Result<Vec<String>> {
    let repos = call_tool_json("list_repos", &json!({}))?;
    let names = repos
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    Ok(names)
}

fn format_tool_result(tool_name: &str, result: &Value) -> String {
    match tool_name {
        "query" => format_query_result(result),
        "context" => format_context_result(result),
        "impact" => format_impact_result(result),
        "cypher" => format_cypher_result(result),
        "detect_changes" => format_detect_changes_result(result),
        "list_repos" => format_list_repos_result(result),
        _ => serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string()),
    }
}

fn format_query_result(result: &Value) -> String {
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        return format!("Error: {err}");
    }

    let processes = result
        .get("processes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let symbols = result
        .get("process_symbols")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let defs = result
        .get("definitions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if processes.is_empty() && defs.is_empty() {
        return "No matching execution flows found. Try a different search term or use grep."
            .to_string();
    }

    let mut lines = Vec::<String>::new();
    lines.push(format!("Found {} execution flow(s):", processes.len()));
    lines.push(String::new());

    for (idx, process) in processes.iter().enumerate() {
        let summary = as_str(process, "summary").unwrap_or("?");
        let step_count = as_u64(process, "step_count").unwrap_or(0);
        let symbol_count = as_u64(process, "symbol_count").unwrap_or(0);
        lines.push(format!(
            "{}. {} ({} steps, {} symbols)",
            idx + 1,
            summary,
            step_count,
            symbol_count
        ));

        let process_id = as_str(process, "id").unwrap_or_default();
        let mut process_symbols = symbols
            .iter()
            .filter(|sym| as_str(sym, "process_id").unwrap_or_default() == process_id)
            .collect::<Vec<_>>();
        process_symbols.truncate(6);

        for sym in process_symbols {
            let kind = as_str(sym, "type").unwrap_or("Symbol");
            let name = as_str(sym, "name").unwrap_or("?");
            let file = as_str(sym, "filePath").unwrap_or("?");
            let loc = as_u64(sym, "startLine")
                .map(|line| format!(":{line}"))
                .unwrap_or_default();
            lines.push(format!("   {} {} -> {}{}", kind, name, file, loc));
        }
        lines.push(String::new());
    }

    if !defs.is_empty() {
        lines.push("Standalone definitions:".to_string());
        for def in defs.iter().take(8) {
            let kind = as_str(def, "type").unwrap_or("Symbol");
            let name = as_str(def, "name").unwrap_or("?");
            let file = as_str(def, "filePath").unwrap_or("?");
            lines.push(format!("  {} {} -> {}", kind, name, file));
        }
    }

    lines.join("\n").trim().to_string()
}

fn format_context_result(result: &Value) -> String {
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        return format!("Error: {err}");
    }

    if result.get("status").and_then(Value::as_str) == Some("ambiguous") {
        let candidates = result
            .get("candidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let name = candidates
            .first()
            .and_then(|item| as_str(item, "name"))
            .unwrap_or("?");
        let mut lines = vec![format!(
            "Multiple symbols named '{name}'. Disambiguate with file path:\n"
        )];
        for candidate in candidates {
            let kind = as_str(&candidate, "kind").unwrap_or("Symbol");
            let item_name = as_str(&candidate, "name").unwrap_or("?");
            let file = as_str(&candidate, "filePath").unwrap_or("?");
            let line = as_u64(&candidate, "line")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".to_string());
            let uid = as_str(&candidate, "uid").unwrap_or("?");
            lines.push(format!(
                "  {} {} -> {}:{}  (uid: {})",
                kind, item_name, file, line, uid
            ));
        }
        return lines.join("\n");
    }

    let Some(symbol) = result.get("symbol") else {
        return "Symbol not found.".to_string();
    };

    let mut lines = Vec::<String>::new();
    let kind = as_str(symbol, "kind").unwrap_or("Symbol");
    let name = as_str(symbol, "name").unwrap_or("?");
    let file = as_str(symbol, "filePath").unwrap_or("?");
    let loc = match (as_u64(symbol, "startLine"), as_u64(symbol, "endLine")) {
        (Some(start), Some(end)) => format!(":{start}-{end}"),
        _ => String::new(),
    };
    lines.push(format!("{kind} {name} -> {file}{loc}"));
    lines.push(String::new());

    if let Some(incoming) = result.get("incoming").and_then(Value::as_object) {
        let incoming_count = incoming
            .values()
            .filter_map(Value::as_array)
            .map(Vec::len)
            .sum::<usize>();
        if incoming_count > 0 {
            lines.push(format!("Called/imported by ({}):", incoming_count));
            for (rel_type, refs) in incoming {
                for item in refs.as_array().into_iter().flatten().take(10) {
                    let ref_kind = as_str(item, "kind").unwrap_or("Symbol");
                    let ref_name = as_str(item, "name").unwrap_or("?");
                    let ref_file = as_str(item, "filePath").unwrap_or("?");
                    lines.push(format!(
                        "  <- [{}] {} {} -> {}",
                        rel_type, ref_kind, ref_name, ref_file
                    ));
                }
            }
            lines.push(String::new());
        }
    }

    if let Some(outgoing) = result.get("outgoing").and_then(Value::as_object) {
        let outgoing_count = outgoing
            .values()
            .filter_map(Value::as_array)
            .map(Vec::len)
            .sum::<usize>();
        if outgoing_count > 0 {
            lines.push(format!("Calls/imports ({}):", outgoing_count));
            for (rel_type, refs) in outgoing {
                for item in refs.as_array().into_iter().flatten().take(10) {
                    let ref_kind = as_str(item, "kind").unwrap_or("Symbol");
                    let ref_name = as_str(item, "name").unwrap_or("?");
                    let ref_file = as_str(item, "filePath").unwrap_or("?");
                    lines.push(format!(
                        "  -> [{}] {} {} -> {}",
                        rel_type, ref_kind, ref_name, ref_file
                    ));
                }
            }
            lines.push(String::new());
        }
    }

    if let Some(processes) = result.get("processes").and_then(Value::as_array)
        && !processes.is_empty()
    {
        lines.push(format!(
            "Participates in {} execution flow(s):",
            processes.len()
        ));
        for process in processes {
            let proc_name = as_str(process, "name").unwrap_or("?");
            let step_idx = as_u64(process, "step_index").unwrap_or(0);
            let step_count = as_u64(process, "step_count").unwrap_or(0);
            lines.push(format!(
                "  * {} (step {}/{})",
                proc_name, step_idx, step_count
            ));
        }
    }

    if let Some(content) = symbol.get("content").and_then(Value::as_str)
        && !content.trim().is_empty()
    {
        lines.push(String::new());
        lines.push("Source:".to_string());
        lines.push(content.to_string());
    }

    lines.join("\n").trim().to_string()
}

fn format_impact_result(result: &Value) -> String {
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        return format!("Error: {err}");
    }

    let target = result.get("target").unwrap_or(&Value::Null);
    let target_name = as_str(target, "name").unwrap_or("?");
    let target_kind = as_str(target, "kind")
        .or_else(|| as_str(target, "type"))
        .unwrap_or("");
    let direction = result
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("upstream");
    let total = result
        .get("impactedCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    if total == 0 {
        return format!(
            "{}: No {} dependencies found. This symbol appears isolated.",
            target_name, direction
        );
    }

    let dir_label = if direction == "upstream" {
        "depends on this (will break if changed)"
    } else {
        "this depends on"
    };

    let mut lines = vec![format!(
        "Blast radius for {} {} ({}): {} symbol(s) {}",
        target_kind, target_name, direction, total, dir_label
    )];
    lines.push(String::new());

    let depth_labels = [
        (1u64, "WILL BREAK (direct)"),
        (2u64, "LIKELY AFFECTED (indirect)"),
        (3u64, "MAY NEED TESTING (transitive)"),
    ];

    for (depth, label) in depth_labels {
        let Some(items) = result
            .get("byDepth")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get(&depth.to_string()))
            .and_then(Value::as_array)
        else {
            continue;
        };

        if items.is_empty() {
            continue;
        }

        lines.push(format!("d={}: {} ({})", depth, label, items.len()));
        for item in items.iter().take(12) {
            let kind = as_str(item, "type")
                .or_else(|| as_str(item, "node_type"))
                .unwrap_or("Symbol");
            let name = as_str(item, "name").unwrap_or("?");
            let file = as_str(item, "filePath")
                .or_else(|| as_str(item, "file_path"))
                .unwrap_or("?");
            let relation = as_str(item, "relationType")
                .or_else(|| as_str(item, "relation_type"))
                .unwrap_or("?");
            let confidence = item
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            let conf_suffix = if confidence < 1.0 {
                format!(" (conf: {})", round3(confidence))
            } else {
                String::new()
            };
            lines.push(format!(
                "  {} {} -> {} [{}]{}",
                kind, name, file, relation, conf_suffix
            ));
        }
        lines.push(String::new());
    }

    lines.join("\n").trim().to_string()
}

fn format_cypher_result(result: &Value) -> String {
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        return format!("Error: {err}");
    }

    if let Some(markdown) = result.get("markdown").and_then(Value::as_str) {
        return markdown.to_string();
    }

    serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string())
}

fn format_detect_changes_result(result: &Value) -> String {
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        return format!("Error: {err}");
    }

    let summary = result.get("summary").unwrap_or(&Value::Null);
    if summary
        .get("changed_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        == 0
    {
        return "No changes detected.".to_string();
    }

    let changed_files = summary
        .get("changed_files")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let changed_count = summary
        .get("changed_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let affected_count = summary
        .get("affected_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let risk = summary
        .get("risk_level")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    let mut lines = vec![
        format!(
            "Changes: {} files, {} symbols",
            changed_files, changed_count
        ),
        format!("Affected processes: {}", affected_count),
        format!("Risk level: {}", risk),
        String::new(),
    ];

    if let Some(changed_symbols) = result.get("changed_symbols").and_then(Value::as_array)
        && !changed_symbols.is_empty()
    {
        lines.push("Changed symbols:".to_string());
        for item in changed_symbols.iter().take(15) {
            let name = as_str(item, "name").unwrap_or("?");
            let file = as_str(item, "filePath").unwrap_or("?");
            let change_type = as_str(item, "change_type").unwrap_or("changed");
            lines.push(format!("  {} ({}) -> {}", name, change_type, file));
        }
        lines.push(String::new());
    }

    if let Some(affected) = result.get("affected_processes").and_then(Value::as_array)
        && !affected.is_empty()
    {
        lines.push("Affected execution flows:".to_string());
        for process in affected.iter().take(10) {
            let name = as_str(process, "name").unwrap_or("?");
            let step_count = as_u64(process, "step_count").unwrap_or(0);
            let steps = process
                .get("changed_steps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|step| as_str(step, "symbol"))
                .collect::<Vec<_>>();
            lines.push(format!(
                "  * {} ({} steps) — changed: {}",
                name,
                step_count,
                if steps.is_empty() {
                    "none".to_string()
                } else {
                    steps.join(", ")
                }
            ));
        }
    }

    lines.join("\n").trim().to_string()
}

fn format_list_repos_result(result: &Value) -> String {
    let Some(repos) = result.as_array() else {
        return "No indexed repositories.".to_string();
    };
    if repos.is_empty() {
        return "No indexed repositories.".to_string();
    }

    let mut lines = vec!["Indexed repositories:".to_string(), String::new()];
    for repo in repos {
        let name = as_str(repo, "name").unwrap_or("?");
        let path = as_str(repo, "path").unwrap_or("?");
        let indexed = as_str(repo, "indexedAt").unwrap_or("?");
        let stats = repo.get("stats").unwrap_or(&Value::Null);
        let nodes = as_u64(stats, "nodes")
            .or_else(|| as_u64(stats, "files"))
            .unwrap_or(0);
        let edges = as_u64(stats, "edges").unwrap_or(0);
        let processes = as_u64(stats, "processes").unwrap_or(0);
        lines.push(format!(
            "  {} — {} symbols, {} relationships, {} flows",
            name, nodes, edges, processes
        ));
        lines.push(format!("    Path: {}", path));
        lines.push(format!("    Indexed: {}", indexed));
    }
    lines.join("\n")
}

fn next_step_hint(tool_name: &str) -> &'static str {
    match tool_name {
        "query" => {
            "\n---\nNext: Pick a symbol above and run gitnexus context \"<name>\" to inspect callers, callees, and flows."
        }
        "context" => {
            "\n---\nNext: To check what breaks if you change this, run gitnexus impact \"<name>\" --direction upstream."
        }
        "impact" => {
            "\n---\nNext: Review d=1 items first (WILL BREAK), then inspect source and apply fixes."
        }
        "cypher" => {
            "\n---\nNext: To inspect any returned symbol in depth, run gitnexus context \"<name>\"."
        }
        "detect_changes" => {
            "\n---\nNext: Run gitnexus context on high-risk changed symbols to review caller impact."
        }
        _ => "",
    }
}

fn text_response(status: StatusCode, body: &str) -> Response {
    (status, [("Content-Type", "text/plain")], body.to_string()).into_response()
}

fn mark_activity(state: &EvalServerState) {
    state.last_activity_unix.store(now_unix(), Ordering::SeqCst);
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn as_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn as_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
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

    use super::{format_context_result, format_impact_result, format_query_result, next_step_hint};

    #[test]
    fn format_query_result_with_process_symbols() {
        let payload = json!({
            "processes": [
                {
                    "id": "proc-1",
                    "summary": "User Login Flow",
                    "step_count": 4,
                    "symbol_count": 2
                }
            ],
            "process_symbols": [
                {
                    "process_id": "proc-1",
                    "type": "Function",
                    "name": "validateUser",
                    "filePath": "src/auth.rs",
                    "startLine": 42
                }
            ],
            "definitions": []
        });

        let text = format_query_result(&payload);
        assert!(text.contains("Found 1 execution flow(s):"));
        assert!(text.contains("1. User Login Flow (4 steps, 2 symbols)"));
        assert!(text.contains("Function validateUser -> src/auth.rs:42"));
    }

    #[test]
    fn format_query_result_without_hits() {
        let payload = json!({
            "processes": [],
            "definitions": []
        });

        let text = format_query_result(&payload);
        assert!(text.contains("No matching execution flows found"));
    }

    #[test]
    fn format_context_result_for_ambiguous_symbol() {
        let payload = json!({
            "status": "ambiguous",
            "candidates": [
                {
                    "kind": "Function",
                    "name": "run",
                    "filePath": "src/commands/a.rs",
                    "line": 12,
                    "uid": "uid-a"
                },
                {
                    "kind": "Function",
                    "name": "run",
                    "filePath": "src/commands/b.rs",
                    "line": 24,
                    "uid": "uid-b"
                }
            ]
        });

        let text = format_context_result(&payload);
        assert!(text.contains("Multiple symbols named 'run'"));
        assert!(text.contains("Function run -> src/commands/a.rs:12"));
        assert!(text.contains("uid: uid-b"));
    }

    #[test]
    fn format_impact_result_with_no_dependencies() {
        let payload = json!({
            "target": { "name": "run", "kind": "Function" },
            "direction": "upstream",
            "impactedCount": 0
        });

        let text = format_impact_result(&payload);
        assert!(text.contains("No upstream dependencies found"));
        assert!(text.contains("run"));
    }

    #[test]
    fn next_step_hint_matches_tool_name() {
        assert!(next_step_hint("query").contains("gitnexus context"));
        assert!(next_step_hint("impact").contains("d=1"));
        assert_eq!(next_step_hint("unknown"), "");
    }
}
