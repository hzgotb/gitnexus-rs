use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request as AxumRequest,
    response::Response as AxumResponse,
    routing::any,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::{Value, json};

use crate::commands::local_mcp::GitNexusMcpServer;
use crate::ingestion::{HeuristicProcess, IngestionResult, StructureNode, run_ingestion_pipeline};
use crate::storage::repo_manager::{
    RegistryEntry, get_storage_paths, list_registered_repos, load_meta,
};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 4747;
const MAX_REQUEST_BYTES: usize = 10 * 1024 * 1024;
const GRAPH_CACHE_FILENAME: &str = "graph.json";

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    content_type: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

pub fn run_server(host: Option<&str>, port: Option<u16>) -> Result<()> {
    let host = host.unwrap_or(DEFAULT_HOST);
    let port = port.unwrap_or(DEFAULT_PORT);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to initialize tokio runtime for HTTP server")?;

    runtime.block_on(run_server_async(host.to_string(), port))
}

async fn run_server_async(host: String, port: u16) -> Result<()> {
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .with_context(|| format!("failed to bind HTTP server on {host}:{port}"))?;

    println!("GitNexus server running on http://{host}:{port}");

    let mcp_service: StreamableHttpService<GitNexusMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(GitNexusMcpServer::default()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );

    let app = Router::new()
        .nest_service("/api/mcp", mcp_service)
        .fallback(any(legacy_http_handler));

    axum::serve(listener, app)
        .await
        .context("axum server terminated unexpectedly")?;
    Ok(())
}

async fn legacy_http_handler(request: AxumRequest<Body>) -> AxumResponse {
    let request = match to_legacy_request(request).await {
        Ok(request) => request,
        Err(err) => return to_axum_response(json_error_response(400, err.to_string())),
    };

    match route_request(request) {
        Ok(response) => to_axum_response(response),
        Err(err) => {
            let status = map_error_status(&err);
            to_axum_response(json_error_response(status, err.to_string()))
        }
    }
}

async fn to_legacy_request(request: AxumRequest<Body>) -> Result<HttpRequest> {
    let (parts, body) = request.into_parts();
    let target = match parts.uri.query() {
        Some(query) if !query.is_empty() => format!("{}?{query}", parts.uri.path()),
        _ => parts.uri.path().to_string(),
    };
    let (path, query) = split_target(&target);

    let body = to_bytes(body, MAX_REQUEST_BYTES).await.map_err(|err| {
        anyhow::anyhow!("failed to read HTTP body: {err}").context("invalid HTTP request body")
    })?;

    Ok(HttpRequest {
        method: parts.method.as_str().to_string(),
        path,
        query,
        body: body.to_vec(),
    })
}

fn to_axum_response(response: HttpResponse) -> AxumResponse {
    let mut builder = axum::http::Response::builder()
        .status(response.status)
        .header("Content-Type", response.content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header(
            "Access-Control-Allow-Headers",
            "Content-Type, MCP-Session-Id",
        )
        .header("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS");

    for (key, value) in response.headers {
        builder = builder.header(key, value);
    }

    builder
        .body(Body::from(response.body))
        .unwrap_or_else(|_| axum::http::Response::new(Body::from("{\"error\":\"internal\"}")))
}

fn split_target(target: &str) -> (String, HashMap<String, String>) {
    let (path_raw, query_raw) = target.split_once('?').unwrap_or((target, ""));
    let mut query = HashMap::<String, String>::new();

    for pair in query_raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(url_decode(k), url_decode(v));
    }

    (url_decode(path_raw), query)
}

fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::<u8>::with_capacity(bytes.len());
    let mut idx = 0usize;

    while idx < bytes.len() {
        match bytes[idx] {
            b'+' => {
                out.push(b' ');
                idx += 1;
            }
            b'%' if idx + 2 < bytes.len() => {
                let hi = bytes[idx + 1] as char;
                let lo = bytes[idx + 2] as char;
                if let (Some(a), Some(b)) = (hi.to_digit(16), lo.to_digit(16)) {
                    out.push(((a << 4) + b) as u8);
                    idx += 3;
                } else {
                    out.push(bytes[idx]);
                    idx += 1;
                }
            }
            other => {
                out.push(other);
                idx += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).to_string()
}

fn route_request(request: HttpRequest) -> Result<HttpResponse> {
    if request.method.eq_ignore_ascii_case("OPTIONS") {
        return Ok(HttpResponse {
            status: 204,
            content_type: "application/json; charset=utf-8",
            headers: Vec::new(),
            body: Vec::new(),
        });
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/health") => Ok(json_response(json!({ "status": "ok" }))),
        ("GET", "/api/repos") => handle_list_repos(),
        ("GET", "/api/repo") => handle_repo_info(&request),
        ("GET", "/api/graph") => handle_graph(&request),
        ("POST", "/api/query") => handle_query(&request),
        ("POST", "/api/search") => handle_search(&request),
        ("GET", "/api/file") => handle_file(&request),
        ("GET", "/api/processes") => handle_processes(&request),
        ("GET", "/api/process") => handle_process_detail(&request),
        ("GET", "/api/clusters") => handle_clusters(&request),
        ("GET", "/api/cluster") => handle_cluster_detail(&request),
        _ => Ok(json_error_response(404, "Not found")),
    }
}

fn handle_list_repos() -> Result<HttpResponse> {
    let entries = list_registered_repos(true)?;
    let repos = entries
        .into_iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "path": entry.path,
                "indexedAt": entry.indexed_at,
                "lastCommit": entry.last_commit,
                "stats": entry.stats
            })
        })
        .collect::<Vec<_>>();
    Ok(json_response(Value::Array(repos)))
}

fn handle_repo_info(request: &HttpRequest) -> Result<HttpResponse> {
    let repo_hint = requested_repo(request, None);
    let entry = resolve_repo_entry(repo_hint.as_deref())?;
    let meta = load_meta(Path::new(&entry.storage_path))?;

    let indexed_at = meta
        .as_ref()
        .map(|m| m.indexed_at.clone())
        .unwrap_or_else(|| entry.indexed_at.clone());
    let stats = meta
        .as_ref()
        .and_then(|m| m.stats.clone())
        .or(entry.stats.clone());

    Ok(json_response(json!({
        "name": entry.name,
        "repoPath": entry.path,
        "indexedAt": indexed_at,
        "stats": stats
    })))
}

fn handle_graph(request: &HttpRequest) -> Result<HttpResponse> {
    let repo_hint = requested_repo(request, None);
    let entry = resolve_repo_entry(repo_hint.as_deref())?;
    let graph = load_or_build_graph(&entry)?;

    let nodes = graph
        .graph
        .nodes
        .iter()
        .map(|node| {
            json!({
                "id": node.id,
                "label": node.label,
                "properties": {
                    "name": node.name,
                    "filePath": node.file_path,
                    "startLine": node.start_line,
                    "language": node.language
                }
            })
        })
        .collect::<Vec<_>>();

    let relationships = graph
        .graph
        .relationships
        .iter()
        .map(|rel| {
            json!({
                "id": rel.id,
                "type": rel.rel_type,
                "sourceId": rel.source_id,
                "targetId": rel.target_id,
                "confidence": rel.confidence,
                "reason": rel.reason
            })
        })
        .collect::<Vec<_>>();

    Ok(json_response(json!({
        "nodes": nodes,
        "relationships": relationships
    })))
}

fn handle_query(request: &HttpRequest) -> Result<HttpResponse> {
    let body = parse_json_body(&request.body)?;
    let cypher = body
        .get("cypher")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing \"cypher\" in request body"))?;

    let repo_hint = requested_repo(request, Some(&body));
    let mut args = vec![cypher.to_string()];
    if let Some(repo) = repo_hint {
        args.push("--repo".to_string());
        args.push(repo);
    }

    let result = run_cli_json("cypher", args)?;
    Ok(json_response(json!({ "result": result })))
}

fn handle_search(request: &HttpRequest) -> Result<HttpResponse> {
    let body = parse_json_body(&request.body)?;
    let query = body
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing \"query\" in request body"))?;

    let repo_hint = requested_repo(request, Some(&body));
    let limit = body
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n.clamp(1, 100))
        .unwrap_or(10);

    let mut args = vec![query.to_string(), "--limit".to_string(), limit.to_string()];
    if let Some(repo) = repo_hint {
        args.push("--repo".to_string());
        args.push(repo);
    }

    let results = run_cli_json("query", args)?;
    Ok(json_response(json!({ "results": results })))
}

fn handle_file(request: &HttpRequest) -> Result<HttpResponse> {
    let repo_hint = requested_repo(request, None);
    let entry = resolve_repo_entry(repo_hint.as_deref())?;

    let file_path = request
        .query
        .get("path")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing path"))?;

    let repo_root = PathBuf::from(&entry.path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&entry.path));
    let joined = repo_root.join(file_path);
    let full_path = joined.canonicalize().unwrap_or(joined);

    if !full_path.starts_with(&repo_root) {
        bail!("Path traversal denied");
    }

    let content = std::fs::read_to_string(&full_path).with_context(|| {
        format!(
            "failed to read file {}",
            full_path.as_path().to_string_lossy()
        )
    })?;

    Ok(json_response(json!({ "content": content })))
}

fn handle_processes(request: &HttpRequest) -> Result<HttpResponse> {
    let repo_hint = requested_repo(request, None);
    let entry = resolve_repo_entry(repo_hint.as_deref())?;
    let graph = load_or_build_graph(&entry)?;
    let nodes_by_id = build_node_map(&graph.graph.nodes);

    let processes = graph
        .processes
        .iter()
        .map(|process| {
            json!({
                "id": process.id,
                "name": process_summary(process, &nodes_by_id),
                "entrySymbolId": process.entry_symbol_id,
                "symbolCount": process.symbol_count,
                "stepCount": process.step_count
            })
        })
        .collect::<Vec<_>>();

    Ok(json_response(json!({ "processes": processes })))
}

fn handle_process_detail(request: &HttpRequest) -> Result<HttpResponse> {
    let repo_hint = requested_repo(request, None);
    let entry = resolve_repo_entry(repo_hint.as_deref())?;
    let graph = load_or_build_graph(&entry)?;
    let nodes_by_id = build_node_map(&graph.graph.nodes);

    let name = request
        .query
        .get("name")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing \"name\" query parameter"))?;

    let process = graph
        .processes
        .iter()
        .find(|p| p.id == name || process_summary(p, &nodes_by_id) == name)
        .ok_or_else(|| anyhow::anyhow!("Process not found"))?;

    let steps = process
        .symbols
        .iter()
        .enumerate()
        .map(|(idx, symbol_id)| {
            let node = nodes_by_id.get(symbol_id);
            json!({
                "step": idx + 1,
                "symbolId": symbol_id,
                "name": node.map(|n| n.name.clone()).unwrap_or_else(|| symbol_id.clone()),
                "filePath": node.map(|n| n.file_path.clone()),
                "startLine": node.and_then(|n| n.start_line)
            })
        })
        .collect::<Vec<_>>();

    Ok(json_response(json!({
        "id": process.id,
        "name": process_summary(process, &nodes_by_id),
        "entrySymbolId": process.entry_symbol_id,
        "symbolCount": process.symbol_count,
        "stepCount": process.step_count,
        "steps": steps
    })))
}

fn handle_clusters(request: &HttpRequest) -> Result<HttpResponse> {
    let repo_hint = requested_repo(request, None);
    let entry = resolve_repo_entry(repo_hint.as_deref())?;
    let graph = load_or_build_graph(&entry)?;

    let clusters = graph
        .communities
        .iter()
        .map(|community| {
            json!({
                "id": community.id,
                "fileCount": community.file_count
            })
        })
        .collect::<Vec<_>>();

    Ok(json_response(json!({ "clusters": clusters })))
}

fn handle_cluster_detail(request: &HttpRequest) -> Result<HttpResponse> {
    let repo_hint = requested_repo(request, None);
    let entry = resolve_repo_entry(repo_hint.as_deref())?;
    let graph = load_or_build_graph(&entry)?;

    let name = request
        .query
        .get("name")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing \"name\" query parameter"))?;

    let community = graph
        .communities
        .iter()
        .find(|c| c.id == name)
        .ok_or_else(|| anyhow::anyhow!("Cluster not found"))?;

    Ok(json_response(json!({
        "id": community.id,
        "fileCount": community.file_count,
        "files": community.files
    })))
}

fn requested_repo(request: &HttpRequest, body: Option<&Value>) -> Option<String> {
    if let Some(repo) = request.query.get("repo").cloned() {
        return Some(repo);
    }
    body.and_then(|value| {
        value
            .get("repo")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn parse_json_body(body: &[u8]) -> Result<Value> {
    if body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice::<Value>(body).context("request body is not valid JSON")
}

fn resolve_repo_entry(repo_hint: Option<&str>) -> Result<RegistryEntry> {
    let entries = list_registered_repos(true)?;
    if entries.is_empty() {
        bail!("No indexed repositories found. Run: gitnexus analyze");
    }

    if let Some(hint) = repo_hint {
        let hint_lower = hint.to_ascii_lowercase();
        let hint_path = Path::new(hint);

        let mut matches = entries
            .iter()
            .filter(|entry| {
                entry.name.eq_ignore_ascii_case(hint)
                    || entry.path == hint
                    || Path::new(&entry.path) == hint_path
                    || entry.name.to_ascii_lowercase().contains(&hint_lower)
                    || entry.path.to_ascii_lowercase().contains(&hint_lower)
            })
            .cloned()
            .collect::<Vec<_>>();

        matches.sort_by(|a, b| a.name.cmp(&b.name));
        matches.dedup_by(|a, b| a.path == b.path);

        return match matches.len() {
            0 => bail!("Repository '{hint}' not found. Run `gitnexus list` for available repos."),
            1 => Ok(matches.remove(0)),
            _ => {
                let names = matches.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
                bail!(
                    "Repository '{hint}' is ambiguous. Candidates: {}",
                    names.join(", ")
                )
            }
        };
    }

    Ok(entries[0].clone())
}

fn load_or_build_graph(entry: &RegistryEntry) -> Result<IngestionResult> {
    let repo_path = Path::new(&entry.path);
    let storage = get_storage_paths(repo_path);
    let graph_path = storage.storage_path.join(GRAPH_CACHE_FILENAME);

    if graph_path.exists() {
        let raw = std::fs::read_to_string(&graph_path)
            .with_context(|| format!("failed to read {}", graph_path.to_string_lossy()))?;
        return serde_json::from_str::<IngestionResult>(&raw)
            .with_context(|| format!("failed to parse {}", graph_path.to_string_lossy()));
    }

    let result = run_ingestion_pipeline(repo_path)?;
    std::fs::create_dir_all(&storage.storage_path).with_context(|| {
        format!(
            "failed to create storage directory {}",
            storage.storage_path.to_string_lossy()
        )
    })?;
    let content = serde_json::to_string_pretty(&result)?;
    std::fs::write(&graph_path, content)
        .with_context(|| format!("failed to write {}", graph_path.to_string_lossy()))?;
    Ok(result)
}

fn run_cli_json(subcommand: &str, mut args: Vec<String>) -> Result<Value> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let mut cli_args = vec![subcommand.to_string()];
    cli_args.append(&mut args);

    let output = std::process::Command::new(exe)
        .args(&cli_args)
        .output()
        .with_context(|| format!("failed to run subcommand: gitnexus {}", cli_args.join(" ")))?;

    let stdout_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr_text = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let detail = if stderr_text.is_empty() {
            stdout_text
        } else {
            stderr_text
        };
        bail!("{detail}");
    }

    if stdout_text.is_empty() {
        return Ok(json!({}));
    }

    match serde_json::from_str::<Value>(&stdout_text) {
        Ok(v) => Ok(v),
        Err(_) => Ok(json!({ "output": stdout_text })),
    }
}

fn build_node_map(nodes: &[StructureNode]) -> HashMap<String, StructureNode> {
    nodes
        .iter()
        .cloned()
        .map(|node| (node.id.clone(), node))
        .collect::<HashMap<_, _>>()
}

fn process_summary(
    process: &HeuristicProcess,
    nodes_by_id: &HashMap<String, StructureNode>,
) -> String {
    let entry = nodes_by_id
        .get(&process.entry_symbol_id)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| process.entry_symbol_id.clone());
    format!("{} -> {}", process.id, entry)
}

fn map_error_status(err: &anyhow::Error) -> u16 {
    let msg = err.to_string().to_ascii_lowercase();
    if msg.contains("missing")
        || msg.contains("invalid")
        || msg.contains("ambiguous")
        || msg.contains("request body is not valid json")
    {
        return 400;
    }
    if msg.contains("not found") || msg.contains("no indexed repositories") {
        return 404;
    }
    if msg.contains("path traversal denied") {
        return 403;
    }
    500
}

fn json_response(value: Value) -> HttpResponse {
    let body =
        serde_json::to_vec(&value).unwrap_or_else(|_| b"{\"error\":\"serialization\"}".to_vec());
    HttpResponse {
        status: 200,
        content_type: "application/json; charset=utf-8",
        headers: Vec::new(),
        body,
    }
}

fn json_error_response(status: u16, message: impl Into<String>) -> HttpResponse {
    let body = serde_json::to_vec(&json!({ "error": message.into() }))
        .unwrap_or_else(|_| b"{\"error\":\"internal\"}".to_vec());
    HttpResponse {
        status,
        content_type: "application/json; charset=utf-8",
        headers: Vec::new(),
        body,
    }
}
