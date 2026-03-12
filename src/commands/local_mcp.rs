use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, GetPromptRequestParams, GetPromptResult,
        Implementation, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
        ListToolsResult, PaginatedRequestParams, Prompt, ReadResourceRequestParams,
        ReadResourceResult, Resource, ResourceTemplate, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    transport::stdio,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::ingestion::IngestionResult;
use crate::storage::repo_manager::{RegistryEntry, list_registered_repos};

#[derive(Debug, Clone, Default)]
pub(crate) struct GitNexusMcpServer;

pub fn run_server() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to initialize tokio runtime for MCP server")?;

    runtime.block_on(async {
        let service = GitNexusMcpServer
            .serve(stdio())
            .await
            .map_err(|err| anyhow::anyhow!("failed to start rmcp stdio transport: {err}"))?;

        service
            .waiting()
            .await
            .map_err(|err| anyhow::anyhow!("rmcp service exited with error: {err}"))?;

        Ok(())
    })
}

impl ServerHandler for GitNexusMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
            server_info: Implementation {
                name: "gitnexus-rs".to_string(),
                title: Some("GitNexus Rust".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: Some("Rust-native GitNexus MCP server (rmcp transport)".to_string()),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use query/context/impact before edits and detect_changes before committing."
                    .to_string(),
            ),
            ..Default::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ListToolsResult, McpError>> + Send + '_ {
        async move {
            Ok(ListToolsResult::with_all_items(
                decode_list_from_key::<Tool>(tools_list_result(), "tools")?,
            ))
        }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        decode_list_from_key::<Tool>(tools_list_result(), "tools")
            .ok()
            .and_then(|tools| tools.into_iter().find(|tool| tool.name.as_ref() == name))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<CallToolResult, McpError>> + Send + '_ {
        async move {
            let params = json!({
                "name": request.name.as_ref(),
                "arguments": Value::Object(request.arguments.unwrap_or_default()),
            });

            let payload = tools_call_result(&params)
                .map_err(|err| McpError::internal_error(err.to_string(), None))?;
            decode_from_value::<CallToolResult>(payload, "invalid tools/call result payload")
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ListResourcesResult, McpError>> + Send + '_ {
        async move {
            Ok(ListResourcesResult::with_all_items(decode_list_from_key::<
                Resource,
            >(
                resources_list_result(),
                "resources",
            )?))
        }
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ListResourceTemplatesResult, McpError>> + Send + '_
    {
        async move {
            Ok(ListResourceTemplatesResult::with_all_items(
                decode_list_from_key::<ResourceTemplate>(
                    resource_templates_result(),
                    "resourceTemplates",
                )?,
            ))
        }
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ReadResourceResult, McpError>> + Send + '_ {
        async move {
            let payload = resources_read_result(&json!({ "uri": request.uri }))
                .map_err(|err| McpError::resource_not_found(err.to_string(), None))?;
            decode_from_value::<ReadResourceResult>(payload, "invalid resources/read payload")
        }
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ListPromptsResult, McpError>> + Send + '_ {
        async move {
            Ok(ListPromptsResult::with_all_items(decode_list_from_key::<
                Prompt,
            >(
                prompts_list_result(),
                "prompts",
            )?))
        }
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<GetPromptResult, McpError>> + Send + '_ {
        async move {
            let payload = prompts_get_result(&json!({
                "name": request.name,
                "arguments": Value::Object(request.arguments.unwrap_or_default()),
            }))
            .map_err(|err| McpError::invalid_params(err.to_string(), None))?;

            decode_from_value::<GetPromptResult>(payload, "invalid prompts/get payload")
        }
    }
}

fn decode_from_value<T: DeserializeOwned>(
    payload: Value,
    context: &str,
) -> std::result::Result<T, McpError> {
    serde_json::from_value::<T>(payload)
        .map_err(|err| McpError::internal_error(format!("{context}: {err}"), None))
}

fn decode_list_from_key<T: DeserializeOwned>(
    payload: Value,
    key: &str,
) -> std::result::Result<Vec<T>, McpError> {
    let items = payload
        .get(key)
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));
    decode_from_value::<Vec<T>>(items, &format!("invalid field '{key}'"))
}

fn tools_list_result() -> Value {
    let tools = vec![
        json!({
            "name": "list_repos",
            "description": "List all indexed repositories available to GitNexus.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "query",
            "description": "Query the code knowledge graph for execution flows related to a concept.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "task_context": {"type": "string"},
                    "goal": {"type": "string"},
                    "limit": {"type": "number"},
                    "max_symbols": {"type": "number"},
                    "include_content": {"type": "boolean"},
                    "repo": {"type": "string"}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "context",
            "description": "360-degree view of a single code symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "uid": {"type": "string"},
                    "file_path": {"type": "string"},
                    "include_content": {"type": "boolean"},
                    "repo": {"type": "string"}
                },
                "required": []
            }
        }),
        json!({
            "name": "impact",
            "description": "Analyze blast radius of changing a code symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {"type": "string"},
                    "direction": {"type": "string"},
                    "maxDepth": {"type": "number"},
                    "relationTypes": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "includeTests": {"type": "boolean"},
                    "minConfidence": {"type": "number"},
                    "repo": {"type": "string"}
                },
                "required": ["target", "direction"]
            }
        }),
        json!({
            "name": "cypher",
            "description": "Execute Cypher query against the knowledge graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "repo": {"type": "string"}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "detect_changes",
            "description": "Analyze uncommitted git changes and affected execution flows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": {"type": "string"},
                    "base_ref": {"type": "string"},
                    "repo": {"type": "string"}
                },
                "required": []
            }
        }),
        json!({
            "name": "rename",
            "description": "Multi-file coordinated rename using graph + text search.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol_name": {"type": "string"},
                    "symbol_uid": {"type": "string"},
                    "new_name": {"type": "string"},
                    "file_path": {"type": "string"},
                    "dry_run": {"type": "boolean"},
                    "repo": {"type": "string"}
                },
                "required": ["new_name"]
            }
        }),
    ];

    json!({ "tools": tools })
}

fn tools_call_result(params: &Value) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("tools/call requires params.name"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    let result = call_tool(name, &arguments).unwrap_or_else(|err| {
        json!({
            "error": err.to_string()
        })
    });

    let text = serde_json::to_string_pretty(&result)?;
    let hint = next_step_hint(name, &arguments);

    let mut response = json!({
        "content": [
            {
                "type": "text",
                "text": format!("{}{}", text, hint)
            }
        ]
    });

    if result.get("error").is_some() {
        response
            .as_object_mut()
            .map(|obj| obj.insert("isError".to_string(), Value::Bool(true)));
    }

    Ok(response)
}

fn call_tool(name: &str, args: &Value) -> Result<Value> {
    match name {
        "list_repos" => {
            let entries = list_registered_repos(true)?;
            let repos = entries
                .iter()
                .map(registry_entry_to_json)
                .collect::<Vec<_>>();
            Ok(Value::Array(repos))
        }
        "query" => run_cli_json("query", build_query_args(args)?),
        "context" => run_cli_json("context", build_context_args(args)?),
        "impact" => run_cli_json("impact", build_impact_args(args)?),
        "cypher" => run_cli_json("cypher", build_cypher_args(args)?),
        "detect_changes" => run_cli_json("detect-changes", build_detect_changes_args(args)?),
        "rename" => run_cli_json("rename", build_rename_args(args)?),
        // backward aliases
        "search" => run_cli_json("query", build_query_args(args)?),
        "explore" => run_cli_json("context", build_context_args(args)?),
        other => bail!("Unknown tool: {other}"),
    }
}

fn run_cli_json(subcommand: &str, mut args: Vec<String>) -> Result<Value> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let mut cli_args = vec![subcommand.to_string()];
    cli_args.append(&mut args);

    let output = Command::new(exe)
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
        bail!("subcommand failed: {detail}");
    }

    if stdout_text.is_empty() {
        return Ok(json!({}));
    }

    match serde_json::from_str::<Value>(&stdout_text) {
        Ok(v) => Ok(v),
        Err(_) => Ok(json!({ "output": stdout_text })),
    }
}

fn build_query_args(args: &Value) -> Result<Vec<String>> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("query tool requires 'query'"))?;

    let mut out = vec![query.to_string()];
    push_string_flag(args, "repo", "--repo", &mut out);
    push_string_flag(args, "task_context", "--context", &mut out);
    push_string_flag(args, "goal", "--goal", &mut out);
    push_u64_flag(args, "limit", "--limit", &mut out);
    push_bool_switch(args, "include_content", "--content", &mut out);
    Ok(out)
}

fn build_context_args(args: &Value) -> Result<Vec<String>> {
    let mut out = Vec::<String>::new();

    if let Some(name) = args.get("name").and_then(Value::as_str) {
        out.push(name.to_string());
    }

    push_string_flag(args, "repo", "--repo", &mut out);
    push_string_flag(args, "uid", "--uid", &mut out);
    push_string_flag(args, "file_path", "--file", &mut out);
    push_bool_switch(args, "include_content", "--content", &mut out);

    if out.is_empty() || (out.len() == 1 && out[0].starts_with("--")) {
        bail!("context tool requires name or uid");
    }

    Ok(out)
}

fn build_impact_args(args: &Value) -> Result<Vec<String>> {
    let target = args
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("impact tool requires 'target'"))?;
    let direction = args
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("upstream");

    let mut out = vec![
        target.to_string(),
        "--direction".to_string(),
        direction.to_string(),
    ];
    push_string_flag(args, "repo", "--repo", &mut out);
    push_u64_flag(args, "maxDepth", "--depth", &mut out);
    if !push_string_array_csv_flag(args, "relationTypes", "--relation-types", &mut out) {
        let _ = push_string_array_csv_flag(args, "relation_types", "--relation-types", &mut out);
    }
    push_bool_switch(args, "includeTests", "--include-tests", &mut out);
    if !push_f64_flag(args, "minConfidence", "--min-confidence", &mut out) {
        let _ = push_f64_flag(args, "min_confidence", "--min-confidence", &mut out);
    }
    Ok(out)
}

fn build_cypher_args(args: &Value) -> Result<Vec<String>> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("cypher tool requires 'query'"))?;
    let mut out = vec![query.to_string()];
    push_string_flag(args, "repo", "--repo", &mut out);
    Ok(out)
}

fn build_detect_changes_args(args: &Value) -> Result<Vec<String>> {
    let mut out = Vec::<String>::new();
    push_string_flag(args, "scope", "--scope", &mut out);
    push_string_flag(args, "base_ref", "--base-ref", &mut out);
    push_string_flag(args, "repo", "--repo", &mut out);
    Ok(out)
}

fn build_rename_args(args: &Value) -> Result<Vec<String>> {
    let new_name = args
        .get("new_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("rename tool requires 'new_name'"))?;

    let mut out = Vec::<String>::new();
    push_string_flag(args, "symbol_name", "--symbol-name", &mut out);
    push_string_flag(args, "symbol_uid", "--symbol-uid", &mut out);
    out.push("--new-name".to_string());
    out.push(new_name.to_string());
    push_string_flag(args, "file_path", "--file-path", &mut out);
    push_string_flag(args, "repo", "--repo", &mut out);

    if let Some(dry_run) = args.get("dry_run").and_then(Value::as_bool) {
        if !dry_run {
            out.push("--apply".to_string());
        }
    }

    Ok(out)
}

fn push_string_flag(args: &Value, key: &str, flag: &str, out: &mut Vec<String>) {
    if let Some(value) = args.get(key).and_then(Value::as_str) {
        out.push(flag.to_string());
        out.push(value.to_string());
    }
}

fn push_u64_flag(args: &Value, key: &str, flag: &str, out: &mut Vec<String>) {
    if let Some(value) = args.get(key).and_then(Value::as_u64) {
        out.push(flag.to_string());
        out.push(value.to_string());
    }
}

fn push_f64_flag(args: &Value, key: &str, flag: &str, out: &mut Vec<String>) -> bool {
    if let Some(value) = args.get(key).and_then(Value::as_f64) {
        out.push(flag.to_string());
        out.push(value.to_string());
        true
    } else {
        false
    }
}

fn push_string_array_csv_flag(args: &Value, key: &str, flag: &str, out: &mut Vec<String>) -> bool {
    let Some(value) = args.get(key) else {
        return false;
    };

    let values = if let Some(items) = value.as_array() {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
    } else if let Some(raw) = value.as_str() {
        raw.split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    if values.is_empty() {
        return false;
    }

    out.push(flag.to_string());
    out.push(values.join(","));
    true
}

fn push_bool_switch(args: &Value, key: &str, flag: &str, out: &mut Vec<String>) {
    if args.get(key).and_then(Value::as_bool).unwrap_or(false) {
        out.push(flag.to_string());
    }
}

fn registry_entry_to_json(entry: &RegistryEntry) -> Value {
    json!({
        "name": entry.name,
        "path": entry.path,
        "indexedAt": entry.indexed_at,
        "lastCommit": entry.last_commit,
        "stats": entry.stats,
    })
}

fn resources_list_result() -> Value {
    json!({
        "resources": [
            {
                "uri": "gitnexus://repos",
                "name": "All Indexed Repositories",
                "description": "List of all indexed repos with stats.",
                "mimeType": "text/yaml"
            },
            {
                "uri": "gitnexus://setup",
                "name": "GitNexus Setup Content",
                "description": "AGENTS.md setup hint content.",
                "mimeType": "text/markdown"
            }
        ]
    })
}

fn resource_templates_result() -> Value {
    json!({
        "resourceTemplates": [
            {
                "uriTemplate": "gitnexus://repo/{name}/context",
                "name": "Repo Overview",
                "description": "Codebase stats and tool summary.",
                "mimeType": "text/yaml"
            },
            {
                "uriTemplate": "gitnexus://repo/{name}/clusters",
                "name": "Repo Modules",
                "description": "Heuristic communities.",
                "mimeType": "text/yaml"
            },
            {
                "uriTemplate": "gitnexus://repo/{name}/processes",
                "name": "Repo Processes",
                "description": "Execution process summaries.",
                "mimeType": "text/yaml"
            },
            {
                "uriTemplate": "gitnexus://repo/{name}/schema",
                "name": "Graph Schema",
                "description": "Graph schema for cypher reference.",
                "mimeType": "text/yaml"
            },
            {
                "uriTemplate": "gitnexus://repo/{name}/cluster/{clusterName}",
                "name": "Module Detail",
                "description": "Specific community detail.",
                "mimeType": "text/yaml"
            },
            {
                "uriTemplate": "gitnexus://repo/{name}/process/{processName}",
                "name": "Process Trace",
                "description": "Specific process detail.",
                "mimeType": "text/yaml"
            }
        ]
    })
}

fn resources_read_result(params: &Value) -> Result<Value> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("resources/read requires params.uri"))?;

    let text = read_resource_text(uri)?;
    Ok(json!({
        "contents": [
            {
                "uri": uri,
                "mimeType": if uri.contains("setup") { "text/markdown" } else { "text/yaml" },
                "text": text
            }
        ]
    }))
}

fn read_resource_text(uri: &str) -> Result<String> {
    if uri == "gitnexus://repos" {
        return repos_resource_text();
    }
    if uri == "gitnexus://setup" {
        return Ok("Run `gitnexus analyze` in your repo, then use MCP tools query/context/impact before edits.".to_string());
    }

    let Some(rest) = uri.strip_prefix("gitnexus://repo/") else {
        bail!("Unknown resource URI: {uri}");
    };
    let mut parts = rest.split('/').collect::<Vec<_>>();
    if parts.len() < 2 {
        bail!("Invalid repo resource URI: {uri}");
    }

    let repo_name = parts.remove(0);
    let resource_kind = parts.remove(0);

    let entry = resolve_repo_entry(repo_name)?;
    match resource_kind {
        "context" => repo_context_resource_text(&entry),
        "schema" => Ok(schema_resource_text()),
        "clusters" => clusters_resource_text(&entry, None),
        "processes" => processes_resource_text(&entry, None),
        "cluster" => {
            let cluster_name = parts.join("/");
            clusters_resource_text(&entry, Some(&cluster_name))
        }
        "process" => {
            let process_name = parts.join("/");
            processes_resource_text(&entry, Some(&process_name))
        }
        _ => bail!("Unknown resource kind: {resource_kind}"),
    }
}

fn repos_resource_text() -> Result<String> {
    let repos = list_registered_repos(true)?;
    if repos.is_empty() {
        return Ok("repos: []\n# No repositories indexed. Run: gitnexus analyze".to_string());
    }

    let mut lines = Vec::<String>::new();
    lines.push("repos:".to_string());
    for repo in &repos {
        lines.push(format!("  - name: \"{}\"", repo.name));
        lines.push(format!("    path: \"{}\"", repo.path));
        lines.push(format!("    indexed: \"{}\"", repo.indexed_at));
        lines.push(format!(
            "    commit: \"{}\"",
            short_commit(&repo.last_commit)
        ));
        if let Some(stats) = &repo.stats {
            lines.push(format!("    files: {}", stats.files.unwrap_or(0)));
            lines.push(format!("    symbols: {}", stats.nodes.unwrap_or(0)));
            lines.push(format!("    processes: {}", stats.processes.unwrap_or(0)));
        }
    }

    if repos.len() > 1 {
        lines.push("".to_string());
        lines.push("# Multiple repos indexed. Use repo parameter in tool calls.".to_string());
    }

    Ok(lines.join("\n"))
}

fn resolve_repo_entry(repo_name: &str) -> Result<RegistryEntry> {
    let repos = list_registered_repos(true)?;
    let mut matches = repos
        .into_iter()
        .filter(|r| {
            r.name.eq_ignore_ascii_case(repo_name)
                || r.path == repo_name
                || r.name
                    .to_ascii_lowercase()
                    .contains(&repo_name.to_ascii_lowercase())
        })
        .collect::<Vec<_>>();

    matches.sort_by(|a, b| a.name.cmp(&b.name));
    matches.dedup_by(|a, b| a.path == b.path);

    match matches.len() {
        0 => bail!("Repository '{repo_name}' not found"),
        1 => Ok(matches.remove(0)),
        _ => {
            let names = matches.into_iter().map(|m| m.name).collect::<Vec<_>>();
            bail!(
                "Repository '{repo_name}' is ambiguous: {}",
                names.join(", ")
            )
        }
    }
}

fn repo_context_resource_text(entry: &RegistryEntry) -> Result<String> {
    let mut lines = Vec::<String>::new();
    lines.push(format!("project: {}", entry.name));
    lines.push(format!("path: {}", entry.path));
    lines.push(format!("indexed: {}", entry.indexed_at));
    lines.push("".to_string());
    lines.push("stats:".to_string());
    if let Some(stats) = &entry.stats {
        lines.push(format!("  files: {}", stats.files.unwrap_or(0)));
        lines.push(format!("  symbols: {}", stats.nodes.unwrap_or(0)));
        lines.push(format!("  processes: {}", stats.processes.unwrap_or(0)));
    } else {
        lines.push("  files: 0".to_string());
        lines.push("  symbols: 0".to_string());
        lines.push("  processes: 0".to_string());
    }
    lines.push("".to_string());
    lines.push("tools_available:".to_string());
    lines.push("  - list_repos".to_string());
    lines.push("  - query".to_string());
    lines.push("  - context".to_string());
    lines.push("  - impact".to_string());
    lines.push("  - detect_changes".to_string());
    lines.push("  - rename".to_string());
    lines.push("  - cypher".to_string());

    Ok(lines.join("\n"))
}

fn schema_resource_text() -> String {
    [
        "# GitNexus Graph Schema",
        "",
        "nodes:",
        "  - File",
        "  - Folder",
        "  - Function",
        "  - Class",
        "  - Interface",
        "  - Struct",
        "  - Enum",
        "  - Trait",
        "  - Community",
        "  - Process",
        "",
        "edges:",
        "  - CONTAINS",
        "  - DEFINES",
        "  - IMPORTS",
        "  - CALLS",
        "  - EXTENDS",
        "  - IMPLEMENTS",
        "",
        "query tips:",
        "  - use cypher for custom graph queries",
        "  - use context/impact for safer high-level analysis",
    ]
    .join("\n")
}

fn clusters_resource_text(entry: &RegistryEntry, only: Option<&str>) -> Result<String> {
    let graph = load_graph_from_storage(entry)?;
    let mut lines = Vec::<String>::new();
    lines.push("modules:".to_string());

    let mut communities = graph.communities.clone();
    communities.sort_by(|a, b| b.file_count.cmp(&a.file_count));

    for community in communities {
        if let Some(only) = only
            && community.id != only
        {
            continue;
        }
        lines.push(format!("  - name: \"{}\"", community.id));
        lines.push(format!("    files: {}", community.file_count));
        let preview = community.files.into_iter().take(8).collect::<Vec<_>>();
        lines.push("    sample_files:".to_string());
        for file in preview {
            lines.push(format!("      - {}", file));
        }
    }

    Ok(lines.join("\n"))
}

fn processes_resource_text(entry: &RegistryEntry, only: Option<&str>) -> Result<String> {
    let graph = load_graph_from_storage(entry)?;
    let mut lines = Vec::<String>::new();
    lines.push("processes:".to_string());

    let mut processes = graph.processes.clone();
    processes.sort_by(|a, b| b.step_count.cmp(&a.step_count));

    for process in processes {
        if let Some(only) = only
            && process.id != only
        {
            continue;
        }
        lines.push(format!("  - name: \"{}\"", process.id));
        lines.push(format!("    entry_symbol_id: {}", process.entry_symbol_id));
        lines.push(format!("    symbols: {}", process.symbol_count));
        lines.push(format!("    steps: {}", process.step_count));
        let preview = process.symbols.into_iter().take(12).collect::<Vec<_>>();
        lines.push("    preview_symbols:".to_string());
        for sym in preview {
            lines.push(format!("      - {}", sym));
        }
    }

    Ok(lines.join("\n"))
}

fn load_graph_from_storage(entry: &RegistryEntry) -> Result<IngestionResult> {
    let storage_path = Path::new(&entry.path).join(".gitnexus");
    let graph_path = storage_path.join("graph.json");
    let raw = std::fs::read_to_string(&graph_path)
        .with_context(|| format!("failed to read {}", graph_path.to_string_lossy()))?;
    serde_json::from_str::<IngestionResult>(&raw)
        .with_context(|| format!("failed to parse {}", graph_path.to_string_lossy()))
}

fn prompts_list_result() -> Value {
    json!({
        "prompts": [
            {
                "name": "detect_impact",
                "description": "Analyze impact of current changes before committing.",
                "arguments": [
                    {"name": "scope", "description": "unstaged|staged|all|compare", "required": false},
                    {"name": "base_ref", "description": "base branch for compare scope", "required": false}
                ]
            },
            {
                "name": "generate_map",
                "description": "Generate architecture documentation from graph resources.",
                "arguments": [
                    {"name": "repo", "description": "repo name", "required": false}
                ]
            }
        ]
    })
}

fn prompts_get_result(params: &Value) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("prompts/get requires params.name"))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    let text = match name {
        "detect_impact" => {
            let scope = args.get("scope").and_then(Value::as_str).unwrap_or("all");
            let base_ref = args.get("base_ref").and_then(Value::as_str).unwrap_or("");
            format!(
                "Analyze current changes: run detect_changes({{scope: \"{scope}\"{}}}), then context() and impact() on high-risk symbols.",
                if base_ref.is_empty() {
                    "".to_string()
                } else {
                    format!(", base_ref: \"{base_ref}\"")
                }
            )
        }
        "generate_map" => {
            let repo = args.get("repo").and_then(Value::as_str).unwrap_or("{name}");
            format!(
                "Generate architecture map: READ gitnexus://repo/{repo}/context, /clusters, /processes, then produce ARCHITECTURE.md with diagrams."
            )
        }
        _ => bail!("Unknown prompt: {name}"),
    };

    Ok(json!({
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": text
                }
            }
        ]
    }))
}

fn short_commit(commit: &str) -> String {
    let short = commit.chars().take(7).collect::<String>();
    if short.is_empty() {
        "unknown".to_string()
    } else {
        short
    }
}

fn next_step_hint(tool_name: &str, args: &Value) -> String {
    let repo = args.get("repo").and_then(Value::as_str).unwrap_or("{name}");
    let mut hints = BTreeMap::<&str, String>::new();
    hints.insert(
        "list_repos",
        "\n\n---\n**Next:** READ gitnexus://repo/{name}/context for a repo above to inspect stats and tools.".to_string(),
    );
    hints.insert(
        "query",
        format!(
            "\n\n---\n**Next:** run context({{name: \"<symbol>\", repo: \"{}\"}}) on a symbol from process_symbols.",
            repo
        ),
    );
    hints.insert(
        "context",
        format!(
            "\n\n---\n**Next:** run impact({{target: \"<symbol>\", direction: \"upstream\", repo: \"{}\"}}) before editing.",
            repo
        ),
    );
    hints.insert(
        "impact",
        "\n\n---\n**Next:** review d=1 first (WILL BREAK), then inspect affected process traces."
            .to_string(),
    );
    hints.insert(
        "detect_changes",
        "\n\n---\n**Next:** run context() on high-risk changed symbols and verify caller chains."
            .to_string(),
    );
    hints.insert(
        "rename",
        "\n\n---\n**Next:** run detect_changes() to verify rename side effects.".to_string(),
    );
    hints.insert(
        "cypher",
        "\n\n---\n**Next:** drill into returned symbols with context().".to_string(),
    );

    hints.get(tool_name).cloned().unwrap_or_default()
}
